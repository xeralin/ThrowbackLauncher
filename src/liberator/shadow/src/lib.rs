#![no_std]

use core::ffi::c_void;
use core::panic::PanicInfo;

const FILE_MAP_READ: u32 = 0x0004;
const FILE_MAP_WRITE: u32 = 0x0002;
const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;
const EXCEPTION_CONTINUE_EXECUTION: i32 = -1;
const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
const CTX_RIP: usize = 0xF8;
const AV_EXECUTE: usize = 8;
const CALL_FIRST: u32 = 1;
const MEM_COMMIT: u32 = 0x1000;
const DLL_PROCESS_ATTACH: u32 = 1;
const LAYOUT_VERSION: i32 = 3;

#[repr(C)]
struct MemoryBasicInformation {
    base_address: *mut c_void,
    allocation_base: *mut c_void,
    allocation_protect: u32,
    alignment1: u32,
    region_size: usize,
    state: u32,
    protect: u32,
    kind: u32,
    alignment2: u32,
}

#[repr(C)]
struct ExceptionRecord {
    exception_code: u32,
    exception_flags: u32,
    exception_record: *mut c_void,
    exception_address: *mut c_void,
    number_parameters: u32,
    reserved: u32,
    exception_information: [usize; 15],
}

#[repr(C)]
struct ExceptionPointers {
    exception_record: *mut ExceptionRecord,
    context_record: *mut u8,
}

#[link(name = "kernel32")]
extern "system" {
    fn OpenFileMappingW(access: u32, inherit: i32, name: *const u16) -> *mut c_void;
    fn MapViewOfFile(map: *mut c_void, access: u32, hi: u32, lo: u32, len: usize) -> *mut u8;
    fn AddVectoredExceptionHandler(first: u32, handler: *mut c_void) -> *mut c_void;
    fn VirtualQuery(addr: *const c_void, info: *mut MemoryBasicInformation, len: usize) -> usize;
}

static mut TEXT_BASE: usize = 0;
static mut TEXT_SIZE: usize = 0;
static mut SHADOW_BASE: usize = 0;

unsafe extern "system" fn handler(ep: *mut ExceptionPointers) -> i32 {
    let rec = &*(*ep).exception_record;
    if rec.exception_code != STATUS_ACCESS_VIOLATION {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    if rec.exception_information[0] != AV_EXECUTE {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    let rip_slot = (*ep).context_record.add(CTX_RIP) as *mut usize;
    let rip = *rip_slot;
    let text_base = TEXT_BASE;
    let text_size = TEXT_SIZE;
    let shadow_base = SHADOW_BASE;
    if rip >= text_base && rip < text_base + text_size {
        let shadow_rip = shadow_base + (rip - text_base);
        let mut mbi: MemoryBasicInformation = core::mem::zeroed();
        let size = core::mem::size_of::<MemoryBasicInformation>();
        if VirtualQuery(shadow_rip as *const c_void, &mut mbi, size) != 0 && mbi.state == MEM_COMMIT
        {
            *rip_slot = shadow_rip;
            return EXCEPTION_CONTINUE_EXECUTION;
        }
        return EXCEPTION_CONTINUE_SEARCH;
    }
    if rip >= shadow_base && rip < shadow_base + text_size {
        *rip_slot = text_base + (rip - shadow_base);
        return EXCEPTION_CONTINUE_EXECUTION;
    }
    EXCEPTION_CONTINUE_SEARCH
}

static MAP_NAME: [u16; 14] = [
    b'S' as u16,
    b'h' as u16,
    b'a' as u16,
    b'd' as u16,
    b'o' as u16,
    b'w' as u16,
    b'R' as u16,
    b'e' as u16,
    b'g' as u16,
    b'i' as u16,
    b'o' as u16,
    b'n' as u16,
    b's' as u16,
    0,
];

unsafe fn install() {
    let access = FILE_MAP_READ | FILE_MAP_WRITE;
    let map = OpenFileMappingW(access, 0, MAP_NAME.as_ptr());
    if map.is_null() {
        return;
    }
    let view = MapViewOfFile(map, access, 0, 0, 0);
    if view.is_null() {
        return;
    }
    if (view.add(4) as *const i32).read_unaligned() != LAYOUT_VERSION {
        return;
    }
    TEXT_BASE = (view.add(8) as *const usize).read_unaligned();
    TEXT_SIZE = (view.add(16) as *const usize).read_unaligned();
    SHADOW_BASE = (view.add(24) as *const usize).read_unaligned();
    if TEXT_BASE == 0 || TEXT_SIZE == 0 || SHADOW_BASE == 0 {
        return;
    }
    AddVectoredExceptionHandler(CALL_FIRST, handler as *mut c_void);
}

#[no_mangle]
pub unsafe extern "system" fn DllMain(
    _instance: *mut c_void,
    reason: u32,
    _reserved: *mut c_void,
) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        install();
    }
    1
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
