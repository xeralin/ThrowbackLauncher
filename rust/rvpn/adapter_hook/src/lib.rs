#![no_std]

#[path = "../../common/mem.rs"]
mod mem;
#[path = "../../common/private_v4.rs"]
mod private_v4;

use core::arch::{asm, naked_asm};
use core::ffi::{CStr, c_void};
use core::iter::from_fn;
use core::mem::transmute;
use core::net::Ipv4Addr;
use core::panic::PanicInfo;
use core::ptr::{copy_nonoverlapping, null, null_mut};
use core::slice::from_raw_parts;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, Ordering};

use private_v4::is_private_v4;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS, EXCEPTION_ACCESS_VIOLATION, FARPROC, HANDLE,
    HMODULE, TRUE,
};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GAA_FLAG_INCLUDE_GATEWAYS, GetAdaptersAddresses, GetAdaptersInfo, GetBestInterface,
    IP_ADAPTER_ADDRESSES_LH, IP_ADAPTER_INFO, IP_ADAPTER_UNICAST_ADDRESS_LH,
};
use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows_sys::Win32::Networking::WinSock::{
    ADDRINFOA, AF_INET, HOSTENT, NI_NAMEREQD, NI_NUMERICHOST, NI_NUMERICSERV, SOCKADDR,
    SOCKADDR_IN, getaddrinfo, gethostbyname, gethostname, getnameinfo,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    AddVectoredExceptionHandler, EXCEPTION_CONTINUE_EXECUTION, EXCEPTION_CONTINUE_SEARCH,
    EXCEPTION_POINTERS, FlushInstructionCache, IMAGE_DIRECTORY_ENTRY_IMPORT, IMAGE_NT_HEADERS32,
};
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleHandleW, GetProcAddress, LoadLibraryA, LoadLibraryExW, LoadLibraryW,
};
use windows_sys::Win32::System::Memory::{
    GetProcessHeap, HeapAlloc, HeapFree, PAGE_EXECUTE_READWRITE, PAGE_READWRITE, VirtualProtect,
};
use windows_sys::Win32::System::SystemServices::{
    DLL_PROCESS_ATTACH, IMAGE_DOS_HEADER, IMAGE_IMPORT_DESCRIPTOR, IMAGE_ORDINAL_FLAG32,
};
use windows_sys::Win32::System::Threading::{
    CreateThread, GetCurrentProcess, Sleep, TlsAlloc, TlsGetValue, TlsSetValue,
};
use windows_sys::core::{BOOL, PCSTR, PCWSTR, PSTR};
use windows_sys::w;

const BUILDS: [(u32, usize, usize, usize, usize); 2] = [
    (0x68A8905C, 0x636A0, 0xC9001, 0x13F1D8, 0x5BA70),
    (0x6A69EE38, 0x788C0, 0xE5807, 0x15C778, 0x67D00),
];
const ALLOWED_MAX: usize = 32;
const ADAPTERS_BUF: u32 = 15 * 1024;
const ROL_WATCH_TICKS: u32 = 600;
const ROL_WATCH_MS: u32 = 100;

#[repr(C, packed)]
struct Unaligned<T>(T);

type Adapter = Unaligned<IP_ADAPTER_ADDRESSES_LH>;
type AdapterInfo = Unaligned<IP_ADAPTER_INFO>;
type Unicast = Unaligned<IP_ADAPTER_UNICAST_ADDRESS_LH>;

static FRAME_TLS: AtomicU32 = AtomicU32::new(0);
static FREE_FN: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static COUNTER: AtomicPtr<i32> = AtomicPtr::new(null_mut());
static TAP_IFINDEX: AtomicU32 = AtomicU32::new(0);
static ROL_PATCHED: AtomicBool = AtomicBool::new(false);

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    unsafe { asm!("ud2", options(noreturn)) }
}

#[unsafe(naked)]
unsafe extern "C" fn guard_landing() {
    naked_asm!(
        "cld",
        "pop dword ptr fs:[0]",
        "pop edi",
        "pop esi",
        "pop ebx",
        "pop ebp",
        "ret",
    )
}

#[unsafe(naked)]
unsafe extern "C" fn guarded_call(f: *mut c_void, arg: *mut c_void, frame: *mut u32) {
    naked_asm!(
        "push ebp",
        "push ebx",
        "push esi",
        "push edi",
        "push dword ptr fs:[0]",
        "mov ebp, esp",
        "mov eax, [ebp + 32]",
        "mov [eax], ebp",
        "push dword ptr [ebp + 28]",
        "call dword ptr [ebp + 24]",
        "mov esp, ebp",
        "jmp {landing}",
        landing = sym guard_landing,
    )
}

unsafe extern "system" fn crash_handler(ep: *mut EXCEPTION_POINTERS) -> i32 {
    unsafe {
        if (*(*ep).ExceptionRecord).ExceptionCode == EXCEPTION_ACCESS_VIOLATION {
            let frame = TlsGetValue(FRAME_TLS.load(Ordering::Relaxed)).cast::<u32>();
            if !frame.is_null() {
                (*(*ep).ContextRecord).Esp = *frame;
                (*(*ep).ContextRecord).Eip = guard_landing as *const () as u32;
                return EXCEPTION_CONTINUE_EXECUTION;
            }
        }
    }
    EXCEPTION_CONTINUE_SEARCH
}

unsafe extern "system" fn hook_dispatch(block: *mut *mut c_void) -> u32 {
    if block.is_null() {
        return 0;
    }
    unsafe {
        let (f, arg) = (*block, *block.add(1));
        let free: unsafe extern "C" fn(*mut c_void, u32) =
            transmute(FREE_FN.load(Ordering::Relaxed));
        free(block.cast(), 8);
        if !f.is_null() {
            let slot = FRAME_TLS.load(Ordering::Relaxed);
            let prev = TlsGetValue(slot);
            let mut frame = 0;
            TlsSetValue(slot, (&raw mut frame).cast());
            guarded_call(f, arg, &raw mut frame);
            TlsSetValue(slot, prev);
        }
        AtomicI32::from_ptr(COUNTER.load(Ordering::Relaxed)).fetch_sub(1, Ordering::SeqCst);
    }
    0
}

unsafe fn nt_headers(base: *const u8) -> *const IMAGE_NT_HEADERS32 {
    unsafe {
        let lfanew = (*base.cast::<IMAGE_DOS_HEADER>()).e_lfanew;
        base.add(lfanew as usize).cast()
    }
}

unsafe fn patch(at: *mut u8, expect: &[u8], code: &[u8]) {
    let mut old = 0;
    unsafe {
        if from_raw_parts(at, expect.len()) == expect
            && VirtualProtect(at.cast(), code.len(), PAGE_EXECUTE_READWRITE, &mut old) != 0
        {
            copy_nonoverlapping(code.as_ptr(), at, code.len());
            VirtualProtect(at.cast(), code.len(), old, &mut old);
            FlushInstructionCache(GetCurrentProcess(), at.cast(), code.len());
        }
    }
}

unsafe fn install_release_guard(base: *mut u8) {
    unsafe {
        let stamp = (*nt_headers(base)).FileHeader.TimeDateStamp;
        let Some(&(_, dispatch, free_fn, counter, get_inetwork)) =
            BUILDS.iter().find(|build| build.0 == stamp)
        else {
            return;
        };
        FREE_FN.store(base.add(free_fn).cast(), Ordering::Relaxed);
        COUNTER.store(base.add(counter).cast(), Ordering::Relaxed);
        patch(
            base.add(get_inetwork),
            &[0x56, 0x57, 0x8B],
            &[0x30, 0xC0, 0xC3],
        );
        let [a, b, c, d] = (hook_dispatch as *const () as u32).to_le_bytes();
        let stub = [0x68, a, b, c, d, 0xC3];
        patch(base.add(dispatch), &[0x55, 0x8B, 0xEC, 0x6A, 0xFF], &stub);
    }
}

fn is_cgnat(host: u32) -> bool {
    host & 0xFFC00000 == 0x64400000
}

fn is_uplink_v4(net: u32) -> bool {
    let host = u32::from_be(net);
    let ip = Ipv4Addr::from_bits(host);
    !ip.is_unspecified() && !ip.is_loopback() && !ip.is_link_local() && !is_cgnat(host)
}

unsafe fn s_addr(sa: *const SOCKADDR) -> u32 {
    unsafe {
        sa.cast::<SOCKADDR_IN>()
            .read_unaligned()
            .sin_addr
            .S_un
            .S_addr
    }
}

unsafe fn unicast_v4(adapter: *const Adapter) -> impl Iterator<Item = u32> {
    let mut cur: *const Unicast = unsafe { (*adapter).0.FirstUnicastAddress.cast() };
    from_fn(move || unsafe {
        while !cur.is_null() {
            let sa = (*cur).0.Address.lpSockaddr;
            cur = (*cur).0.Next.cast();
            if !sa.is_null() && (*sa).sa_family == AF_INET {
                return Some(s_addr(sa));
            }
        }
        None
    })
}

fn query_adapters(flags: u32) -> *mut Adapter {
    let mut size = ADAPTERS_BUF;
    unsafe {
        let heap = GetProcessHeap();
        let mut list = HeapAlloc(heap, 0, size as usize);
        let mut rc = GetAdaptersAddresses(AF_INET as u32, flags, null(), list.cast(), &mut size);
        if rc == ERROR_BUFFER_OVERFLOW {
            HeapFree(heap, 0, list);
            list = HeapAlloc(heap, 0, size as usize);
            rc = GetAdaptersAddresses(AF_INET as u32, flags, null(), list.cast(), &mut size);
        }
        if rc != ERROR_SUCCESS {
            HeapFree(heap, 0, list);
            return null_mut();
        }
        list.cast()
    }
}

fn default_route_ifindex() -> u32 {
    let mut best = 0;
    if unsafe { GetBestInterface(0x08080808, &mut best) } == ERROR_SUCCESS && best != 0 {
        return best;
    }
    let list = query_adapters(GAA_FLAG_INCLUDE_GATEWAYS);
    let (mut found, mut count) = (0, 0);
    let mut cur = list;
    unsafe {
        while !cur.is_null() {
            if (*cur).0.OperStatus == IfOperStatusUp
                && !(*cur).0.FirstGatewayAddress.is_null()
                && unicast_v4(cur).any(is_uplink_v4)
            {
                count += 1;
                found = (*cur).0.Anonymous1.Anonymous.IfIndex;
            }
            cur = (*cur).0.Next.cast();
        }
        HeapFree(GetProcessHeap(), 0, list.cast());
    }
    if count == 1 { found } else { 0 }
}

unsafe extern "system" fn hook_get_adapters_addresses(
    family: u32,
    flags: u32,
    reserved: *const c_void,
    addrs: *mut Adapter,
    size: *mut u32,
) -> u32 {
    unsafe {
        let ret = GetAdaptersAddresses(family, flags, reserved, addrs.cast(), size);
        if ret != ERROR_SUCCESS || addrs.is_null() {
            return ret;
        }
        let best = default_route_ifindex();
        let mut best_ok = false;
        let mut cur = addrs;
        while !cur.is_null() {
            let index = (*cur).0.Anonymous1.Anonymous.IfIndex;
            let desc = (*cur).0.Description;
            let mut name = b"radminvpn0\0".iter().enumerate();
            if !desc.is_null() && name.all(|(i, &byte)| *desc.add(i) == u16::from(byte)) {
                (*cur).0.Description = w!("Famatech Radmin VPN Ethernet Adapter").cast_mut();
                (*cur).0.FriendlyName = w!("Radmin VPN").cast_mut();
                TAP_IFINDEX.store(index, Ordering::Relaxed);
            } else if index == best {
                best_ok = true;
            }
            cur = (*cur).0.Next.cast();
        }
        if best_ok {
            let tap = TAP_IFINDEX.load(Ordering::Relaxed);
            let mut cur = addrs;
            while !cur.is_null() {
                let index = (*cur).0.Anonymous1.Anonymous.IfIndex;
                if index != best && index != tap {
                    (*cur).0.FirstUnicastAddress = null_mut();
                }
                cur = (*cur).0.Next.cast();
            }
        }
        ret
    }
}

unsafe extern "system" fn hook_get_adapters_info(info: *mut AdapterInfo, size: *mut u32) -> u32 {
    unsafe {
        let ret = GetAdaptersInfo(info.cast(), size);
        if ret != ERROR_SUCCESS || info.is_null() {
            return ret;
        }
        let best = default_route_ifindex();
        let mut best_ok = false;
        let mut cur = info;
        while !cur.is_null() {
            best_ok |= (*cur).0.Index == best;
            cur = (*cur).0.Next.cast();
        }
        if best_ok {
            let tap = TAP_IFINDEX.load(Ordering::Relaxed);
            let mut cur = info;
            while !cur.is_null() {
                let index = (*cur).0.Index;
                if index != best && index != tap {
                    let ip = &raw mut (*cur).0.IpAddressList.IpAddress;
                    ip.cast::<[u8; 8]>().write(*b"0.0.0.0\0");
                    (*cur).0.IpAddressList.Next = null_mut();
                }
                cur = (*cur).0.Next.cast();
            }
        }
        ret
    }
}

fn build_allowed_v4(allowed: &mut [u32; ALLOWED_MAX]) -> Option<&[u32]> {
    let uplink = default_route_ifindex();
    if uplink == 0 {
        return None;
    }
    let tap = TAP_IFINDEX.load(Ordering::Relaxed);
    let list = query_adapters(0);
    let mut len = 0;
    let (mut uplink_has_v4, mut uplink_cgnat) = (false, false);
    let mut cur = list;
    unsafe {
        while !cur.is_null() {
            let index = (*cur).0.Anonymous1.Anonymous.IfIndex;
            if index == uplink || index == tap {
                for ip in unicast_v4(cur) {
                    if index == uplink {
                        uplink_has_v4 = true;
                        uplink_cgnat |= is_cgnat(u32::from_be(ip));
                    }
                    if len < ALLOWED_MAX {
                        allowed[len] = ip;
                        len += 1;
                    }
                }
            }
            cur = (*cur).0.Next.cast();
        }
        HeapFree(GetProcessHeap(), 0, list.cast());
    }
    (uplink_has_v4 && !uplink_cgnat).then_some(&allowed[..len])
}

fn host_label(name: &[u8]) -> &[u8] {
    let name = &name[..name.len().min(255)];
    name.split(|&byte| byte == b'.').next().unwrap_or_default()
}

unsafe fn is_local_hostname(name: PCSTR) -> bool {
    let mut host = [0; 256];
    unsafe {
        if name.is_null() || *name == 0 || gethostname(host.as_mut_ptr(), 256) != 0 {
            return false;
        }
        let name = CStr::from_ptr(name.cast()).to_bytes();
        let host = CStr::from_ptr(host.as_ptr().cast()).to_bytes();
        host_label(name).eq_ignore_ascii_case(host_label(host))
    }
}

unsafe extern "system" fn hook_gethostbyname(name: PCSTR) -> *mut HOSTENT {
    unsafe {
        let h = gethostbyname(name);
        let list = h.as_ref().map_or(null_mut(), |h| h.h_addr_list);
        if list.is_null() || (*h).h_addrtype != AF_INET as i16 || !is_local_hostname(name) {
            return h;
        }
        let mut allowed = [0; ALLOWED_MAX];
        let Some(allowed) = build_allowed_v4(&mut allowed) else {
            *list = null_mut();
            return h;
        };
        let (mut rd, mut wr) = (list, list);
        while !(*rd).is_null() {
            if allowed.contains(&(*rd).cast::<u32>().read_unaligned()) {
                *wr = *rd;
                wr = wr.add(1);
            }
            rd = rd.add(1);
        }
        *wr = null_mut();
        h
    }
}

unsafe extern "system" fn hook_getaddrinfo(
    node: PCSTR,
    service: PCSTR,
    hints: *const ADDRINFOA,
    res: *mut *mut ADDRINFOA,
) -> i32 {
    unsafe {
        let rc = getaddrinfo(node, service, hints, res);
        if rc != 0 || res.is_null() || (*res).is_null() || !is_local_hostname(node) {
            return rc;
        }
        let mut allowed = [0; ALLOWED_MAX];
        let Some(allowed) = build_allowed_v4(&mut allowed) else {
            return rc;
        };
        let mut wr: *mut ADDRINFOA = null_mut();
        let mut rd = *res;
        while !rd.is_null() {
            let sa = (*rd).ai_addr;
            if (*rd).ai_family == AF_INET as i32 && !sa.is_null() && allowed.contains(&s_addr(sa)) {
                wr = if wr.is_null() { *res } else { (*wr).ai_next };
                (*wr).ai_family = (*rd).ai_family;
                (*wr).ai_socktype = (*rd).ai_socktype;
                (*wr).ai_protocol = (*rd).ai_protocol;
                (*wr).ai_addrlen = (*rd).ai_addrlen;
                (*wr).ai_addr = (*rd).ai_addr;
            }
            rd = (*rd).ai_next;
        }
        if !wr.is_null() {
            (*wr).ai_next = null_mut();
        }
        rc
    }
}

unsafe extern "system" fn hook_getnameinfo(
    sa: *const SOCKADDR,
    salen: i32,
    host: PSTR,
    hostlen: u32,
    serv: PSTR,
    servlen: u32,
    mut flags: i32,
) -> i32 {
    unsafe {
        if !sa.is_null()
            && (*sa).sa_family == AF_INET
            && salen >= size_of::<SOCKADDR_IN>() as i32
            && is_private_v4(u32::from_be(s_addr(sa)))
        {
            flags = (flags | NI_NUMERICHOST as i32 | NI_NUMERICSERV as i32) & !(NI_NAMEREQD as i32);
        }
        getnameinfo(sa, salen, host, hostlen, serv, servlen, flags)
    }
}

extern "system" fn hook_reg_set_key_security(_: usize, _: u32, _: usize) -> u32 {
    ERROR_SUCCESS
}

extern "system" fn perf_stub_ul(_: usize, _: usize, _: u32, _: u32) -> u32 {
    ERROR_SUCCESS
}

extern "system" fn perf_stub_ull(_: usize, _: usize, _: u32, _: u64) -> u32 {
    ERROR_SUCCESS
}

unsafe extern "system" fn hook_get_proc_address(module: HMODULE, name: PCSTR) -> FARPROC {
    unsafe {
        let found = GetProcAddress(module, name);
        if found.is_some() || (name as usize) >> 16 == 0 {
            return found;
        }
        let stub = match CStr::from_ptr(name.cast()).to_bytes() {
            b"PerfIncrementULongCounterValue" | b"PerfDecrementULongCounterValue" => {
                perf_stub_ul as *const ()
            }
            b"PerfIncrementULongLongCounterValue" | b"PerfDecrementULongLongCounterValue" => {
                perf_stub_ull as *const ()
            }
            _ => return None,
        };
        transmute(stub)
    }
}

unsafe fn install_import_hook(module: HMODULE, dll: &[u8], name: &[u8], ord: u16, new: *const ()) {
    let base = module.cast::<u8>();
    unsafe {
        let cstr = |rva: u32| CStr::from_ptr(base.add(rva as usize).cast()).to_bytes();
        let dirs = (*nt_headers(base)).OptionalHeader.DataDirectory;
        let imports = dirs[IMAGE_DIRECTORY_ENTRY_IMPORT as usize].VirtualAddress as usize;
        let mut desc = base.add(imports).cast::<IMAGE_IMPORT_DESCRIPTOR>();
        while (*desc).Name != 0 {
            if cstr((*desc).Name).eq_ignore_ascii_case(dll) {
                let first = (*desc).Anonymous.OriginalFirstThunk;
                let mut orig = base.add(first as usize).cast::<u32>();
                let mut thunk = base.add((*desc).FirstThunk as usize).cast::<u32>();
                while *orig != 0 {
                    let hit = if *orig & IMAGE_ORDINAL_FLAG32 != 0 {
                        *orig as u16 == ord
                    } else {
                        cstr(*orig + 2) == name
                    };
                    if hit {
                        let mut old = 0;
                        if VirtualProtect(thunk.cast(), 4, PAGE_READWRITE, &mut old) != 0 {
                            AtomicU32::from_ptr(thunk).store(new as u32, Ordering::Relaxed);
                            VirtualProtect(thunk.cast(), 4, old, &mut old);
                        }
                        return;
                    }
                    orig = orig.add(1);
                    thunk = thunk.add(1);
                }
            }
            desc = desc.add(1);
        }
    }
}

unsafe fn install_iphlpapi_hooks(module: HMODULE) {
    let hook = |name, new| unsafe { install_import_hook(module, b"IPHLPAPI.DLL", name, 0, new) };
    hook(b"GetAdaptersAddresses", hook_get_adapters_addresses as _);
    hook(b"GetAdaptersInfo", hook_get_adapters_info as _);
}

unsafe fn install_ws2_32_hooks(module: HMODULE) {
    let hook =
        |name, ord, new| unsafe { install_import_hook(module, b"WS2_32.DLL", name, ord, new) };
    hook(b"", 52, hook_gethostbyname as _);
    hook(b"getaddrinfo", 0, hook_getaddrinfo as _);
    hook(b"getnameinfo", 0, hook_getnameinfo as _);
}

fn try_patch_rol() {
    if ROL_PATCHED.load(Ordering::Relaxed) {
        return;
    }
    let rol = unsafe { GetModuleHandleW(w!("RvROLClient.dll")) };
    if !rol.is_null() && !ROL_PATCHED.swap(true, Ordering::SeqCst) {
        unsafe {
            install_iphlpapi_hooks(rol);
            install_ws2_32_hooks(rol);
        }
    }
}

unsafe extern "system" fn hook_load_library_w(dll: PCWSTR) -> HMODULE {
    let module = unsafe { LoadLibraryW(dll) };
    try_patch_rol();
    module
}

unsafe extern "system" fn hook_load_library_a(dll: PCSTR) -> HMODULE {
    let module = unsafe { LoadLibraryA(dll) };
    try_patch_rol();
    module
}

unsafe extern "system" fn hook_load_library_ex_w(dll: PCWSTR, file: HANDLE, flags: u32) -> HMODULE {
    let module = unsafe { LoadLibraryExW(dll, file, flags) };
    try_patch_rol();
    module
}

extern "system" fn rol_watch_thread(_: *mut c_void) -> u32 {
    for _ in 0..ROL_WATCH_TICKS {
        if ROL_PATCHED.load(Ordering::Relaxed) {
            break;
        }
        try_patch_rol();
        unsafe { Sleep(ROL_WATCH_MS) };
    }
    0
}

#[unsafe(no_mangle)]
extern "system" fn _DllMainCRTStartup(_: HMODULE, reason: u32, _: *mut c_void) -> BOOL {
    if reason != DLL_PROCESS_ATTACH {
        return TRUE;
    }
    unsafe {
        FRAME_TLS.store(TlsAlloc(), Ordering::Relaxed);
        AddVectoredExceptionHandler(1, Some(crash_handler));
        let exe = GetModuleHandleW(null());
        install_release_guard(exe.cast());
        install_iphlpapi_hooks(exe);
        install_ws2_32_hooks(exe);
        install_import_hook(
            exe,
            b"ADVAPI32.DLL",
            b"RegSetKeySecurity",
            0,
            hook_reg_set_key_security as _,
        );
        let hook = |name, new| install_import_hook(exe, b"KERNEL32.dll", name, 0, new);
        hook(b"GetProcAddress", hook_get_proc_address as _);
        hook(b"LoadLibraryW", hook_load_library_w as _);
        hook(b"LoadLibraryA", hook_load_library_a as _);
        hook(b"LoadLibraryExW", hook_load_library_ex_w as _);
        try_patch_rol();
        let thread = CreateThread(null(), 0, Some(rol_watch_thread), null(), 0, null_mut());
        CloseHandle(thread);
    }
    TRUE
}
