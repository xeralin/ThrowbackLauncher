use std::collections::VecDeque;
use std::ffi::{CString, c_char, c_int, c_short};
use std::fs::{self, File};
use std::io::{self, ErrorKind, Read, Write};
use std::mem::zeroed;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

const FIFO_B2D: &str = "/tmp/rvpn_b2d";
const FIFO_D2B_HIGH: &str = "/tmp/rvpn_d2b_high";
const FIFO_D2B_LOW: &str = "/tmp/rvpn_d2b_low";
const FIFOS: [&str; 3] = [FIFO_B2D, FIFO_D2B_HIGH, FIFO_D2B_LOW];

const ETH_HEADER: usize = 14;
const ETH_IPV4: [u8; 2] = [0x08, 0x00];
const ETH_ARP: [u8; 2] = [0x08, 0x06];
const FRAME_MAX: usize = 1500 + ETH_HEADER + 4;
const RELAY_BUF_MAX: usize = 2048;

const RETRY_RING_SIZE: usize = 8191;
const ARP_CACHE_SIZE: usize = 256;
const ARP_CACHE_TTL_SEC: i64 = 600;
const INCOMING_POLL_MS: i32 = 200;
const OUTGOING_POLL_MS: i32 = 1000;
const B2D_OPEN_POLL: Duration = Duration::from_millis(5);

static RUNNING: AtomicBool = AtomicBool::new(true);

fn stop() {
    RUNNING.store(false, Ordering::Relaxed);
}

fn running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

extern "C" fn on_stop_signal(_: c_int) {
    stop();
}

fn now() -> i64 {
    UNIX_EPOCH.elapsed().map_or(0, |d| d.as_secs() as i64)
}

fn pollfd(file: &File, events: c_short) -> libc::pollfd {
    libc::pollfd {
        fd: file.as_raw_fd(),
        events,
        revents: 0,
    }
}

fn poll(fds: &mut [libc::pollfd], timeout_ms: i32) {
    unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
}

fn fix_ip_checksum(frame: &mut [u8]) {
    let ihl = usize::from(frame[ETH_HEADER] & 0x0F) * 4;
    frame[24..26].fill(0);
    let (words, _) = frame[ETH_HEADER..ETH_HEADER + ihl].as_chunks();
    let mut sum: u32 = words
        .iter()
        .map(|word| u32::from(u16::from_be_bytes(*word)))
        .sum();
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    frame[24..26].copy_from_slice(&(!(sum as u16)).to_be_bytes());
}

fn replicate_mcast_to_bcast(mut out: &File, frame: &[u8], is_fifo: bool) {
    let len = frame.len();
    if !(42..=FRAME_MAX).contains(&len)
        || frame[12..14] != ETH_IPV4
        || frame[30..34] != [224, 0, 2, 60]
        || frame[23] != 17
    {
        return;
    }
    let udp_csum = ETH_HEADER + usize::from(frame[ETH_HEADER] & 0x0F) * 4 + 6;
    let mut wire = [0; 2 + FRAME_MAX];
    wire[..2].copy_from_slice(&(len as u16).to_le_bytes());
    let start = if is_fifo { 0 } else { 2 };
    for first_octet in [26, 255] {
        let copy = &mut wire[2..];
        copy[..len].copy_from_slice(frame);
        copy[..6].fill(0xFF);
        copy[30..34].copy_from_slice(&[first_octet, 255, 255, 255]);
        fix_ip_checksum(copy);
        if len > udp_csum + 1 {
            copy[udp_csum..udp_csum + 2].fill(0);
        }
        let _ = out.write_all(&wire[start..2 + len]);
    }
}

fn arp_reply(
    mut tap: &File,
    dst: &[u8],
    sender_mac: &[u8],
    sender_ip: &[u8],
    target_mac: &[u8],
    target_ip: &[u8],
) {
    let mut reply = [0; 60];
    reply[..6].copy_from_slice(dst);
    reply[6..12].copy_from_slice(sender_mac);
    reply[12..14].copy_from_slice(&ETH_ARP);
    reply[14..22].copy_from_slice(&[0, 1, 8, 0, 6, 4, 0, 2]);
    reply[22..28].copy_from_slice(sender_mac);
    reply[28..32].copy_from_slice(sender_ip);
    reply[32..38].copy_from_slice(target_mac);
    reply[38..42].copy_from_slice(target_ip);
    let _ = tap.write_all(&reply);
}

#[derive(Clone, Copy, Default)]
struct ArpEntry {
    last_seen: i64,
    valid: bool,
    ip: [u8; 4],
    mac: [u8; 6],
}

impl ArpEntry {
    fn is_fresh(&self, now: i64) -> bool {
        self.valid && now - self.last_seen <= ARP_CACHE_TTL_SEC
    }
}

struct ArpCache<'a> {
    tap: &'a File,
    entries: [ArpEntry; ARP_CACHE_SIZE],
    local_ip: Option<[u8; 4]>,
}

impl ArpCache<'_> {
    fn update(&mut self, ip: &[u8], mac: &[u8]) {
        if self.local_ip.is_some_and(|local| local == ip) {
            return;
        }
        let now = now();
        if let Some(entry) = self.entries.iter_mut().find(|e| e.valid && e.ip == ip) {
            entry.mac.copy_from_slice(mac);
            entry.last_seen = now;
            return;
        }
        let slot = self
            .entries
            .iter_mut()
            .min_by_key(|e| {
                if e.is_fresh(now) {
                    e.last_seen
                } else {
                    i64::MIN
                }
            })
            .unwrap();
        slot.ip.copy_from_slice(ip);
        slot.mac.copy_from_slice(mac);
        slot.last_seen = now;
        slot.valid = true;
        arp_reply(self.tap, &[0xFF; 6], mac, ip, mac, ip);
    }

    fn learn_incoming(&mut self, frame: &[u8]) {
        if frame.len() >= 34 && frame[12..14] == ETH_IPV4 {
            self.update(&frame[26..30], &frame[6..12]);
        }
        if frame.len() >= 42 && frame[12..14] == ETH_ARP {
            self.update(&frame[28..32], &frame[22..28]);
        }
    }

    fn learn_outgoing(&mut self, frame: &[u8]) {
        if self.local_ip.is_none() {
            self.local_ip = local_ipv4();
        }
        if frame.len() < 42 || frame[12..14] != ETH_ARP {
            return;
        }
        self.update(&frame[28..32], &frame[22..28]);
        if frame[20..22] != [0, 1] {
            return;
        }
        let target = &frame[38..42];
        let Some(entry) = self.entries.iter_mut().find(|e| e.valid && e.ip == target) else {
            return;
        };
        entry.valid = entry.is_fresh(now());
        if entry.valid {
            let (src, sender) = (&frame[6..12], &frame[28..32]);
            arp_reply(self.tap, src, &entry.mac, target, src, sender);
        }
    }
}

fn process_incoming_frame(
    mut fifo: &File,
    mut tap: &File,
    arp: &Mutex<ArpCache>,
    buf: &mut [u8; RELAY_BUF_MAX],
) -> io::Result<bool> {
    let mut prefix = [0; 2];
    match fifo.read_exact(&mut prefix) {
        Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(false),
        result => result?,
    }
    let len = usize::from(u16::from_le_bytes(prefix));
    if len > RELAY_BUF_MAX {
        return Err(ErrorKind::InvalidData.into());
    }
    let frame = &mut buf[..len];
    fifo.read_exact(frame)?;
    if len >= ETH_HEADER {
        arp.lock().unwrap().learn_incoming(frame);
        tap.write_all(frame)?;
        replicate_mcast_to_bcast(tap, frame, false);
    }
    Ok(true)
}

fn incoming_thread(high: &File, low: &File, tap: &File, arp: &Mutex<ArpCache>) -> io::Result<()> {
    let mut buf = [0; RELAY_BUF_MAX];
    let mut process = |fifo| process_incoming_frame(fifo, tap, arp, &mut buf);
    let mut fds = [pollfd(high, libc::POLLIN), pollfd(low, libc::POLLIN)];
    while running() {
        poll(&mut fds, INCOMING_POLL_MS);
        if fds[0].revents != 0 {
            while running() && process(high)? {}
        }
        if fds[1].revents != 0 {
            process(low)?;
        }
    }
    Ok(())
}

fn write_unless_full(mut b2d: &File, wire: &[u8]) -> io::Result<bool> {
    match b2d.write_all(wire) {
        Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(false),
        result => result.map(|()| true),
    }
}

fn outgoing_thread(mut tap: &File, b2d: &File, arp: &Mutex<ArpCache>) -> io::Result<()> {
    let mut ring: VecDeque<Vec<u8>> = VecDeque::new();
    let mut wire = [0; 2 + RELAY_BUF_MAX];
    let mut fds = [pollfd(tap, libc::POLLIN), pollfd(b2d, libc::POLLOUT)];
    while running() {
        while let Some(queued) = ring.front() {
            if !write_unless_full(b2d, queued)? {
                break;
            }
            ring.pop_front();
        }
        let watched = if ring.is_empty() { 1 } else { 2 };
        poll(&mut fds[..watched], OUTGOING_POLL_MS);
        if fds[0].revents == 0 {
            continue;
        }
        let len = tap.read(&mut wire[2..])?;
        wire[..2].copy_from_slice(&(len as u16).to_le_bytes());
        let (framed, frame) = (&wire[..2 + len], &wire[2..2 + len]);

        arp.lock().unwrap().learn_outgoing(frame);

        let sent = ring.is_empty() && write_unless_full(b2d, framed)?;
        if !sent && ring.len() < RETRY_RING_SIZE {
            ring.push_back(framed.to_vec());
        }
        replicate_mcast_to_bcast(b2d, frame, true);
    }
    Ok(())
}

fn tap_ifreq() -> libc::ifreq {
    let mut ifr: libc::ifreq = unsafe { zeroed() };
    for (dst, src) in ifr.ifr_name.iter_mut().zip(b"radminvpn0") {
        *dst = *src as c_char;
    }
    ifr
}

fn open_tap() -> io::Result<File> {
    let tap = File::options()
        .read(true)
        .write(true)
        .open("/dev/net/tun")?;
    let mut ifr = tap_ifreq();
    ifr.ifr_ifru.ifru_flags = (libc::IFF_TAP | libc::IFF_NO_PI) as c_short;
    match unsafe { libc::ioctl(tap.as_raw_fd(), libc::TUNSETIFF, &mut ifr) } {
        0 => Ok(tap),
        _ => Err(io::Error::last_os_error()),
    }
}

fn local_ipv4() -> Option<[u8; 4]> {
    let mut ifr = tap_ifreq();
    let rc = unsafe {
        let socket = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        let rc = libc::ioctl(socket, libc::SIOCGIFADDR, &mut ifr);
        libc::close(socket);
        rc
    };
    let addr = unsafe { (&raw const ifr.ifr_ifru).cast::<libc::sockaddr_in>().read() };
    (rc == 0).then_some(addr.sin_addr.s_addr.to_ne_bytes())
}

fn remove_fifos() {
    for path in FIFOS {
        let _ = fs::remove_file(path);
    }
}

fn create_fifos() {
    remove_fifos();
    for path in FIFOS {
        let path = CString::new(path).unwrap();
        unsafe { libc::mkfifo(path.as_ptr(), 0o666) };
    }
}

fn open_d2b(path: &str, pipe_size: c_int) -> io::Result<File> {
    let fifo = File::options()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    unsafe { libc::fcntl(fifo.as_raw_fd(), libc::F_SETPIPE_SZ, pipe_size) };
    Ok(fifo)
}

fn open_b2d() -> io::Result<File> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || sender.send(File::options().write(true).open(FIFO_B2D)));
    let b2d = loop {
        if let Ok(opened) = receiver.recv_timeout(B2D_OPEN_POLL) {
            break opened?;
        }
        if !running() {
            return Err(ErrorKind::Interrupted.into());
        }
    };
    let fd = b2d.as_raw_fd();
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
    Ok(b2d)
}

fn relay(tap: File, b2d: File, high: File, low: File) {
    let arp = Mutex::new(ArpCache {
        tap: &tap,
        entries: [ArpEntry::default(); ARP_CACHE_SIZE],
        local_ip: None,
    });
    thread::scope(|scope| {
        scope.spawn(|| {
            let _ = outgoing_thread(&tap, &b2d, &arp);
            stop();
        });
        scope.spawn(|| {
            let _ = incoming_thread(&high, &low, &tap, &arp);
            stop();
        });
    });
}

fn main() -> io::Result<()> {
    for signal in [libc::SIGINT, libc::SIGTERM] {
        unsafe { libc::signal(signal, on_stop_signal as *const () as libc::sighandler_t) };
    }
    let tap = open_tap()?;
    create_fifos();
    let high = open_d2b(FIFO_D2B_HIGH, 256 * 1024)?;
    let low = open_d2b(FIFO_D2B_LOW, 1024 * 1024)?;
    let b2d = open_b2d()?;
    relay(tap, b2d, high, low);
    remove_fifos();
    Ok(())
}
