#define _GNU_SOURCE

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <signal.h>
#include <stdint.h>
#include <time.h>
#include <pthread.h>
#include <sys/ioctl.h>
#include <sys/select.h>
#include <net/if.h>
#include <linux/if_tun.h>
#include <sys/stat.h>
#include <sys/socket.h>
#include <arpa/inet.h>

#define ARP_CACHE_SIZE 256
#define ARP_CACHE_TTL_SEC 600

typedef struct {
    time_t   last_seen;
    int      valid;
    uint8_t  ip[4];
    uint8_t  mac[6];
} arp_cache_entry_t;

static arp_cache_entry_t g_arp_cache[ARP_CACHE_SIZE];
static int               g_arp_cache_count = 0;
static uint8_t           g_local_ip[4] = {0};
static int               g_have_local_ip = 0;

static pthread_mutex_t g_arp_lock = PTHREAD_MUTEX_INITIALIZER;

#define TAP_DEV_NAME    "radminvpn0"
#define FIFO_B2D        "/tmp/rvpn_b2d"
#define FIFO_D2B_HIGH   "/tmp/rvpn_d2b_high"
#define FIFO_D2B_LOW    "/tmp/rvpn_d2b_low"
#define MTU             1500
#define FRAME_MAX       (MTU + 14 + 4)

#define RELAY_BUF_MAX   2048

#define LOW_DRAIN_INTERVAL_US   50000

#define RETRY_RING_SIZE 8192
#define RETRY_RING_MASK (RETRY_RING_SIZE - 1)

struct retry_entry {
    uint16_t total_len;
    uint8_t  data[2 + RELAY_BUF_MAX];
};

static struct retry_entry g_retry_ring[RETRY_RING_SIZE];
static volatile uint32_t  g_retry_head = 0;
static volatile uint32_t  g_retry_tail = 0;

static volatile int running = 1;

static int g_tap_fd = -1;
static int g_b2d_fd = -1;
static int g_d2b_high_fd = -1;
static int g_d2b_low_fd = -1;

static void sig_handler(int sig)
{
    (void)sig;
    __atomic_store_n(&running, 0, __ATOMIC_SEQ_CST);
}

static int write_exact(int fd, const void *buf, size_t n);

static void fix_ip_checksum(uint8_t *frame)
{
    uint8_t ihl = (frame[14] & 0x0F) * 4;
    frame[24] = 0; frame[25] = 0;
    uint32_t sum = 0;
    for (int i = 0; i < ihl; i += 2)
        sum += ((uint32_t)frame[14 + i] << 8) | frame[15 + i];
    while (sum >> 16) sum = (sum & 0xFFFF) + (sum >> 16);
    uint16_t csum = ~((uint16_t)sum);
    frame[24] = (csum >> 8) & 0xFF;
    frame[25] = csum & 0xFF;
}

static int write_frame(int fd, const uint8_t *data, uint16_t len, int is_fifo)
{
    if (is_fifo) {
        uint8_t combined[2 + FRAME_MAX];
        if (len > FRAME_MAX)
            return 0;
        combined[0] = (uint8_t)(len & 0xFF);
        combined[1] = (uint8_t)((len >> 8) & 0xFF);
        memcpy(combined + 2, data, len);
        if (write_exact(fd, combined, (size_t)(2 + len)) < 0)
            return 0;
    } else {
        if (write_exact(fd, data, len) < 0)
            return 0;
    }
    return 1;
}

static int replicate_mcast_to_bcast(int write_fd, const uint8_t *frame, uint16_t frame_len, int is_fifo)
{
    if (frame_len < 42) return 0;

    if (frame[12] != 0x08 || frame[13] != 0x00) return 0;

    if (frame[30] != 224 || frame[31] != 0 || frame[32] != 2 || frame[33] != 60)
        return 0;

    if (frame[23] != 17) return 0;

    uint8_t ihl = (frame[14] & 0x0F) * 4;
    int udp_csum_off = 14 + ihl + 6;

    int count = 0;
    uint8_t copy[FRAME_MAX];
    if ((uint16_t)sizeof(copy) < frame_len) return 0;

    static const uint8_t dst_first_octet[] = { 26, 255 };
    for (size_t i = 0; i < sizeof(dst_first_octet); i++) {
        memcpy(copy, frame, frame_len);
        memset(copy, 0xFF, 6);
        copy[30] = dst_first_octet[i]; copy[31] = 255; copy[32] = 255; copy[33] = 255;
        fix_ip_checksum(copy);
        if (frame_len > (uint16_t)(udp_csum_off + 1)) { copy[udp_csum_off] = 0; copy[udp_csum_off + 1] = 0; }
        if (write_frame(write_fd, copy, frame_len, is_fifo)) {
            count++;
        }
    }

    return count;
}

static int write_exact(int fd, const void *buf, size_t n)
{
    size_t done = 0;
    while (done < n) {
        ssize_t w = write(fd, (const char *)buf + done, n - done);
        if (w < 0) {
            if (errno == EINTR)
                continue;
            return -1;
        }
        if (w == 0) return -1;
        done += w;
    }
    return 0;
}

static int retry_ring_push(const uint8_t *combined, uint16_t total_len)
{
    uint32_t head = __atomic_load_n(&g_retry_head, __ATOMIC_RELAXED);
    uint32_t next = (head + 1) & RETRY_RING_MASK;
    if (next == __atomic_load_n(&g_retry_tail, __ATOMIC_RELAXED))
        return -1;

    g_retry_ring[head].total_len = total_len;
    memcpy(g_retry_ring[head].data, combined, total_len);
    __atomic_store_n(&g_retry_head, next, __ATOMIC_RELEASE);
    return 0;
}

static int retry_ring_flush_one(void)
{
    uint32_t tail = __atomic_load_n(&g_retry_tail, __ATOMIC_ACQUIRE);
    if (tail == __atomic_load_n(&g_retry_head, __ATOMIC_RELAXED))
        return 0;

    struct retry_entry *e = &g_retry_ring[tail];
    if (write_exact(g_b2d_fd, e->data, e->total_len) < 0) {
        if (errno == EAGAIN || errno == EWOULDBLOCK)
            return 0;
        return -1;
    }

    __atomic_store_n(&g_retry_tail, (tail + 1) & RETRY_RING_MASK, __ATOMIC_RELEASE);
    return 1;
}

static int retry_ring_flush(void)
{
    while (__atomic_load_n(&g_retry_head, __ATOMIC_RELAXED) !=
           __atomic_load_n(&g_retry_tail, __ATOMIC_RELAXED)) {
        int rc = retry_ring_flush_one();
        if (rc < 0) return -1;
        if (rc == 0) break;
    }
    return 0;
}

static int open_tap(const char *dev_name)
{
    struct ifreq ifr;
    int fd = open("/dev/net/tun", O_RDWR);
    if (fd < 0) return -1;

    memset(&ifr, 0, sizeof(ifr));
    ifr.ifr_flags = IFF_TAP | IFF_NO_PI;
    strncpy(ifr.ifr_name, dev_name, IFNAMSIZ - 1);

    if (ioctl(fd, TUNSETIFF, &ifr) < 0) {
        close(fd);
        return -1;
    }
    return fd;
}

static void create_fifos(void)
{
    unlink(FIFO_B2D);
    unlink(FIFO_D2B_HIGH);
    unlink(FIFO_D2B_LOW);
    mkfifo(FIFO_B2D, 0666);
    mkfifo(FIFO_D2B_HIGH, 0666);
    mkfifo(FIFO_D2B_LOW, 0666);
}

static int read_exact(int fd, void *buf, size_t n)
{
    size_t done = 0;
    while (done < n) {
        ssize_t r = read(fd, (char *)buf + done, n - done);
        if (r < 0) {
            if (errno == EINTR)
                continue;
            return -1;
        }
        if (r == 0) return -1;
        done += r;
    }
    return 0;
}

static void detect_local_ip(const char *ifname)
{
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    if (fd < 0) return;
    struct ifreq ifr;
    memset(&ifr, 0, sizeof(ifr));
    strncpy(ifr.ifr_name, ifname, IFNAMSIZ - 1);
    if (ioctl(fd, SIOCGIFADDR, &ifr) == 0) {
        struct sockaddr_in *sin = (struct sockaddr_in *)&ifr.ifr_addr;
        memcpy(g_local_ip, &sin->sin_addr.s_addr, 4);
        g_have_local_ip = 1;
    }
    close(fd);
}

static void inject_gratuitous_arp(const uint8_t ip[4], const uint8_t mac[6])
{
    if (g_tap_fd < 0) return;

    uint8_t garp[60];
    memset(garp, 0, sizeof(garp));

    memset(garp, 0xFF, 6);
    memcpy(garp + 6, mac, 6);
    garp[12] = 0x08; garp[13] = 0x06;

    garp[14] = 0x00; garp[15] = 0x01;
    garp[16] = 0x08; garp[17] = 0x00;
    garp[18] = 6;
    garp[19] = 4;
    garp[20] = 0x00; garp[21] = 0x02;
    memcpy(garp + 22, mac, 6);
    memcpy(garp + 28, ip, 4);
    memcpy(garp + 32, mac, 6);
    memcpy(garp + 38, ip, 4);

    write(g_tap_fd, garp, 60);
}

static void arp_cache_update(const uint8_t ip[4], const uint8_t mac[6])
{
    if (g_have_local_ip && memcmp(ip, g_local_ip, 4) == 0) return;

    time_t now = time(NULL);

    for (int i = 0; i < g_arp_cache_count; i++) {
        if (g_arp_cache[i].valid && memcmp(g_arp_cache[i].ip, ip, 4) == 0) {
            memcpy(g_arp_cache[i].mac, mac, 6);
            g_arp_cache[i].last_seen = now;
            return;
        }
    }

    for (int i = 0; i < g_arp_cache_count; i++) {
        if (!g_arp_cache[i].valid ||
            (now - g_arp_cache[i].last_seen) > ARP_CACHE_TTL_SEC) {
            memcpy(g_arp_cache[i].ip, ip, 4);
            memcpy(g_arp_cache[i].mac, mac, 6);
            g_arp_cache[i].last_seen = now;
            g_arp_cache[i].valid = 1;
            inject_gratuitous_arp(ip, mac);
            return;
        }
    }

    if (g_arp_cache_count < ARP_CACHE_SIZE) {
        int i = g_arp_cache_count++;
        memcpy(g_arp_cache[i].ip, ip, 4);
        memcpy(g_arp_cache[i].mac, mac, 6);
        g_arp_cache[i].last_seen = now;
        g_arp_cache[i].valid = 1;
        inject_gratuitous_arp(ip, mac);
        return;
    }

    int oldest = 0;
    for (int i = 1; i < ARP_CACHE_SIZE; i++) {
        if (g_arp_cache[i].last_seen < g_arp_cache[oldest].last_seen)
            oldest = i;
    }
    memcpy(g_arp_cache[oldest].ip, ip, 4);
    memcpy(g_arp_cache[oldest].mac, mac, 6);
    g_arp_cache[oldest].last_seen = now;
    g_arp_cache[oldest].valid = 1;
    inject_gratuitous_arp(ip, mac);
}

static int arp_cache_lookup(const uint8_t ip[4], uint8_t out_mac[6])
{
    time_t now = time(NULL);
    for (int i = 0; i < g_arp_cache_count; i++) {
        if (g_arp_cache[i].valid && memcmp(g_arp_cache[i].ip, ip, 4) == 0) {
            if ((now - g_arp_cache[i].last_seen) <= ARP_CACHE_TTL_SEC) {
                memcpy(out_mac, g_arp_cache[i].mac, 6);
                return 1;
            }
            g_arp_cache[i].valid = 0;
        }
    }
    return 0;
}

static int try_arp_reply(int tap_fd, const uint8_t *frame, uint16_t len)
{
    if (len < 42) return 0;
    if (frame[12] != 0x08 || frame[13] != 0x06) return 0;
    if (frame[20] != 0x00 || frame[21] != 0x01) return 0;

    const uint8_t *target_ip = frame + 38;
    uint8_t cached_mac[6];
    if (!arp_cache_lookup(target_ip, cached_mac)) return 0;

    uint8_t reply[60];
    memset(reply, 0, sizeof(reply));

    memcpy(reply, frame + 6, 6);
    memcpy(reply + 6, cached_mac, 6);
    reply[12] = 0x08; reply[13] = 0x06;

    reply[14] = 0x00; reply[15] = 0x01;
    reply[16] = 0x08; reply[17] = 0x00;
    reply[18] = 6;
    reply[19] = 4;
    reply[20] = 0x00; reply[21] = 0x02;
    memcpy(reply + 22, cached_mac, 6);
    memcpy(reply + 28, target_ip, 4);
    memcpy(reply + 32, frame + 6, 6);
    memcpy(reply + 38, frame + 28, 4);

    if (write(tap_fd, reply, 60) == 60)
        return 1;
    return 0;
}

static int process_incoming_frame(int fd, uint8_t *buf, uint16_t *out_len)
{
    *out_len = 0;

    uint16_t len;
    if (read_exact(fd, &len, 2) < 0) {

        if (errno == EAGAIN || errno == EWOULDBLOCK)
            return 0;
        return -1;
    }
    if (len > RELAY_BUF_MAX)
        return -1;
    if (read_exact(fd, buf, len) < 0)
        return -1;

    *out_len = len;

    pthread_mutex_lock(&g_arp_lock);
    if (len >= 34 && buf[12] == 0x08 && buf[13] == 0x00)
        arp_cache_update(buf + 26, buf + 6);
    if (len >= 42 && buf[12] == 0x08 && buf[13] == 0x06)
        arp_cache_update(buf + 28, buf + 22);
    pthread_mutex_unlock(&g_arp_lock);

    if (write(g_tap_fd, buf, len) != (ssize_t)len) {
        if (errno == EAGAIN || errno == EWOULDBLOCK)
            return 0;
        __atomic_store_n(&running, 0, __ATOMIC_SEQ_CST);
        return -1;
    }

    replicate_mcast_to_bcast(g_tap_fd, buf, len, 0);
    return 0;
}

static void *incoming_thread(void *arg)
{
    (void)arg;
    uint8_t buf[RELAY_BUF_MAX];
    int maxfd = (g_d2b_high_fd > g_d2b_low_fd) ? g_d2b_high_fd : g_d2b_low_fd;

    struct timespec last_low_attempt = {0};
    int have_last_low_attempt = 0;

    int fatal = 0;
    while (__atomic_load_n(&running, __ATOMIC_RELAXED) && !fatal) {
        fd_set rfds;
        FD_ZERO(&rfds);
        FD_SET(g_d2b_high_fd, &rfds);
        FD_SET(g_d2b_low_fd, &rfds);

        struct timeval tv = { .tv_sec = 0, .tv_usec = 200000 };
        int ret = select(maxfd + 1, &rfds, NULL, NULL, &tv);
        if (ret < 0) {
            if (errno == EINTR) continue;
            break;
        }

        if (FD_ISSET(g_d2b_high_fd, &rfds)) {
            while (__atomic_load_n(&running, __ATOMIC_RELAXED)) {
                uint16_t len = 0;
                int rc = process_incoming_frame(g_d2b_high_fd, buf, &len);
                if (rc < 0) { fatal = 1; break; }
                if (len == 0)
                    break;
            }
        }
        if (fatal) break;

        int low_ready = FD_ISSET(g_d2b_low_fd, &rfds);
        int force_low = 0;
        if (!low_ready && have_last_low_attempt) {
            struct timespec now;
            clock_gettime(CLOCK_MONOTONIC, &now);
            long long us = (now.tv_sec - last_low_attempt.tv_sec) * 1000000LL
                         + (now.tv_nsec - last_low_attempt.tv_nsec) / 1000LL;
            if (us >= LOW_DRAIN_INTERVAL_US)
                force_low = 1;
        }

        if (low_ready || force_low) {
            clock_gettime(CLOCK_MONOTONIC, &last_low_attempt);
            have_last_low_attempt = 1;
            uint16_t len = 0;
            if (process_incoming_frame(g_d2b_low_fd, buf, &len) < 0) {
                fatal = 1;
                break;
            }
        }
    }

    __atomic_store_n(&running, 0, __ATOMIC_SEQ_CST);
    return NULL;
}

static void *outgoing_thread(void *arg)
{
    (void)arg;
    uint8_t buf[RELAY_BUF_MAX];

    while (__atomic_load_n(&running, __ATOMIC_RELAXED)) {
        if (retry_ring_flush() < 0) {
            __atomic_store_n(&running, 0, __ATOMIC_SEQ_CST);
            break;
        }

        fd_set rfds;
        FD_ZERO(&rfds);
        FD_SET(g_tap_fd, &rfds);

        struct timeval tv = { .tv_sec = 1, .tv_usec = 0 };
        int ret = select(g_tap_fd + 1, &rfds, NULL, NULL, &tv);
        if (ret < 0) {
            if (errno == EINTR) continue;
            __atomic_store_n(&running, 0, __ATOMIC_SEQ_CST);
            break;
        }
        if (ret == 0)
            continue;

        if (!FD_ISSET(g_tap_fd, &rfds))
            continue;

        ssize_t n = read(g_tap_fd, buf, sizeof(buf));
        if (n <= 0)
            continue;

        uint16_t len = (uint16_t)n;

        pthread_mutex_lock(&g_arp_lock);
        if (!g_have_local_ip) detect_local_ip(TAP_DEV_NAME);
        if (len >= 42 && buf[12] == 0x08 && buf[13] == 0x06)
            arp_cache_update(buf + 28, buf + 22);
        try_arp_reply(g_tap_fd, buf, len);
        pthread_mutex_unlock(&g_arp_lock);

        uint8_t combined[2 + RELAY_BUF_MAX];
        combined[0] = (uint8_t)(len & 0xFF);
        combined[1] = (uint8_t)((len >> 8) & 0xFF);
        memcpy(combined + 2, buf, len);
        uint16_t total_len = 2 + len;

        if (write_exact(g_b2d_fd, combined, total_len) < 0) {
            if (errno == EAGAIN || errno == EWOULDBLOCK) {
                retry_ring_push(combined, total_len);
            } else {
                __atomic_store_n(&running, 0, __ATOMIC_SEQ_CST);
                break;
            }
        }

        replicate_mcast_to_bcast(g_b2d_fd, buf, len, 1);
    }

    __atomic_store_n(&running, 0, __ATOMIC_SEQ_CST);
    return NULL;
}

int main(int argc, char *argv[])
{
    (void)argc; (void)argv;

    signal(SIGINT, sig_handler);
    signal(SIGTERM, sig_handler);
    signal(SIGPIPE, SIG_IGN);

    int tap_fd = open_tap(TAP_DEV_NAME);
    if (tap_fd < 0) return 1;
    g_tap_fd = tap_fd;

    create_fifos();

    int d2b_high_fd = open(FIFO_D2B_HIGH, O_RDONLY | O_NONBLOCK);
    if (d2b_high_fd < 0) { close(tap_fd); return 1; }
    fcntl(d2b_high_fd, F_SETPIPE_SZ, 256 * 1024);

    int d2b_low_fd = open(FIFO_D2B_LOW, O_RDONLY | O_NONBLOCK);
    if (d2b_low_fd < 0) { close(tap_fd); close(d2b_high_fd); return 1; }
    fcntl(d2b_low_fd, F_SETPIPE_SZ, 1024 * 1024);

    int b2d_fd = open(FIFO_B2D, O_WRONLY);
    if (b2d_fd < 0) { close(tap_fd); close(d2b_high_fd); close(d2b_low_fd); return 1; }
    int flags = fcntl(b2d_fd, F_GETFL, 0);
    if (flags >= 0) fcntl(b2d_fd, F_SETFL, flags | O_NONBLOCK);

    g_d2b_high_fd = d2b_high_fd;
    g_d2b_low_fd = d2b_low_fd;
    g_b2d_fd = b2d_fd;

    pthread_t in_thr, out_thr;
    if (pthread_create(&out_thr, NULL, outgoing_thread, NULL) != 0) {
        close(d2b_low_fd);
        close(d2b_high_fd);
        close(b2d_fd);
        close(tap_fd);
        return 1;
    }
    if (pthread_create(&in_thr, NULL, incoming_thread, NULL) != 0) {
        __atomic_store_n(&running, 0, __ATOMIC_SEQ_CST);
        pthread_join(out_thr, NULL);
        close(d2b_low_fd);
        close(d2b_high_fd);
        close(b2d_fd);
        close(tap_fd);
        return 1;
    }

    pthread_join(in_thr, NULL);
    pthread_join(out_thr, NULL);

    close(d2b_low_fd);
    close(d2b_high_fd);
    close(b2d_fd);
    close(tap_fd);
    unlink(FIFO_B2D);
    unlink(FIFO_D2B_HIGH);
    unlink(FIFO_D2B_LOW);
    return 0;
}
