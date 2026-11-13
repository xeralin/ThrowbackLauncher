#![no_std]

#[path = "../../common/private_v4.rs"]
mod private_v4;

use core::ffi::{c_char, c_int, c_void};
use core::mem::transmute;
use core::net::Ipv6Addr;
use core::panic::PanicInfo;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicPtr, Ordering};

use libc::{sockaddr, sockaddr_in, sockaddr_in6, socklen_t};
use private_v4::is_private_v4;

static REAL_GETNAMEINFO: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static REAL_SOCKET: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    unsafe { libc::abort() }
}

fn is_private_v6(ip: Ipv6Addr) -> bool {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_private_v4(mapped.to_bits());
    }
    ip.is_loopback()
        || ip.is_unicast_link_local()
        || ip.segments()[0] & 0xFFC0 == 0xFEC0
        || ip.is_unique_local()
}

unsafe fn is_private_sockaddr(sa: *const sockaddr, len: socklen_t) -> bool {
    if sa.is_null() {
        return false;
    }
    match c_int::from(unsafe { (*sa).sa_family }) {
        libc::AF_INET if len as usize >= size_of::<sockaddr_in>() => {
            let addr = unsafe { sa.cast::<sockaddr_in>().read_unaligned() };
            is_private_v4(u32::from_be(addr.sin_addr.s_addr))
        }
        libc::AF_INET6 if len as usize >= size_of::<sockaddr_in6>() => {
            let addr = unsafe { sa.cast::<sockaddr_in6>().read_unaligned() };
            is_private_v6(addr.sin6_addr.s6_addr.into())
        }
        _ => false,
    }
}

fn is_tcp(domain: c_int, kind: c_int, protocol: c_int) -> bool {
    matches!(domain, libc::AF_INET | libc::AF_INET6)
        && kind & !(libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC) == libc::SOCK_STREAM
        && matches!(protocol, 0 | libc::IPPROTO_TCP)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn getnameinfo(
    sa: *const sockaddr,
    salen: socklen_t,
    host: *mut c_char,
    hostlen: socklen_t,
    serv: *mut c_char,
    servlen: socklen_t,
    mut flags: c_int,
) -> c_int {
    let mut real = REAL_GETNAMEINFO.load(Ordering::Relaxed);
    if real.is_null() {
        real = unsafe { libc::dlsym(libc::RTLD_NEXT, c"getnameinfo".as_ptr()) };
        REAL_GETNAMEINFO.store(real, Ordering::Relaxed);
    }
    if real.is_null() {
        return libc::EAI_FAIL;
    }
    let real: unsafe extern "C" fn(
        *const sockaddr,
        socklen_t,
        *mut c_char,
        socklen_t,
        *mut c_char,
        socklen_t,
        c_int,
    ) -> c_int = unsafe { transmute(real) };
    if unsafe { is_private_sockaddr(sa, salen) } {
        flags = (flags | libc::NI_NUMERICHOST) & !libc::NI_NAMEREQD;
    }
    unsafe { real(sa, salen, host, hostlen, serv, servlen, flags) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn socket(domain: c_int, kind: c_int, protocol: c_int) -> c_int {
    let mut real = REAL_SOCKET.load(Ordering::Relaxed);
    if real.is_null() {
        real = unsafe { libc::dlsym(libc::RTLD_NEXT, c"socket".as_ptr()) };
        REAL_SOCKET.store(real, Ordering::Relaxed);
    }
    if real.is_null() {
        unsafe { *libc::__errno_location() = libc::ENOSYS };
        return -1;
    }
    let real: unsafe extern "C" fn(c_int, c_int, c_int) -> c_int = unsafe { transmute(real) };
    let fd = unsafe { real(domain, kind, protocol) };
    if fd >= 0 && is_tcp(domain, kind, protocol) {
        let on: c_int = 1;
        let size = size_of_val(&on) as socklen_t;
        unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_REUSEPORT,
                (&raw const on).cast(),
                size,
            )
        };
    }
    fd
}
