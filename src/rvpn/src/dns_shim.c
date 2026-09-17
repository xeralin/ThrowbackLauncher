#define _GNU_SOURCE
#include <dlfcn.h>
#include <netdb.h>
#include <netinet/in.h>
#include <stdint.h>
#include <string.h>
#include <sys/socket.h>

static int private_v4(uint32_t host)
{
    return (host & 0xFF000000u) == 0x7F000000u ||
           (host & 0xFF000000u) == 0x0A000000u ||
           (host & 0xFFF00000u) == 0xAC100000u ||
           (host & 0xFFFF0000u) == 0xC0A80000u ||
           (host & 0xFFFF0000u) == 0xA9FE0000u ||
           (host & 0xFFC00000u) == 0x64400000u;
}

static int private_v6(const struct in6_addr *addr)
{
    return IN6_IS_ADDR_LOOPBACK(addr) || IN6_IS_ADDR_LINKLOCAL(addr) ||
           IN6_IS_ADDR_SITELOCAL(addr) || (addr->s6_addr[0] & 0xFE) == 0xFC;
}

static int private_sockaddr(const struct sockaddr *sa, socklen_t len)
{
    if (sa == NULL)
        return 0;
    if (sa->sa_family == AF_INET && len >= (socklen_t)sizeof(struct sockaddr_in))
        return private_v4(ntohl(((const struct sockaddr_in *)sa)->sin_addr.s_addr));
    if (sa->sa_family == AF_INET6 && len >= (socklen_t)sizeof(struct sockaddr_in6))
        return private_v6(&((const struct sockaddr_in6 *)sa)->sin6_addr);
    return 0;
}

int getnameinfo(const struct sockaddr *sa, socklen_t salen, char *host,
                socklen_t hostlen, char *serv, socklen_t servlen, int flags)
{
    static int (*real)(const struct sockaddr *, socklen_t, char *, socklen_t,
                       char *, socklen_t, int);

    if (real == NULL)
        real = dlsym(RTLD_NEXT, "getnameinfo");
    if (real == NULL)
        return EAI_FAIL;
    if (private_sockaddr(sa, salen))
        flags = (flags | NI_NUMERICHOST) & ~NI_NAMEREQD;
    return real(sa, salen, host, hostlen, serv, servlen, flags);
}

struct hostent *gethostbyaddr(const void *addr, socklen_t len, int type)
{
    static struct hostent *(*real)(const void *, socklen_t, int);
    uint32_t host;

    if (addr != NULL) {
        if (type == AF_INET && len == (socklen_t)sizeof(host)) {
            memcpy(&host, addr, sizeof(host));
            if (private_v4(ntohl(host))) {
                h_errno = HOST_NOT_FOUND;
                return NULL;
            }
        } else if (type == AF_INET6 &&
                   len == (socklen_t)sizeof(struct in6_addr) &&
                   private_v6((const struct in6_addr *)addr)) {
            h_errno = HOST_NOT_FOUND;
            return NULL;
        }
    }
    if (real == NULL)
        real = dlsym(RTLD_NEXT, "gethostbyaddr");
    if (real == NULL) {
        h_errno = NO_RECOVERY;
        return NULL;
    }
    return real(addr, len, type);
}
