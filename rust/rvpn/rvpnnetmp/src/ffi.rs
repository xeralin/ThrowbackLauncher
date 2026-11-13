use core::ffi::c_void;
use core::mem::offset_of;
use core::sync::atomic::AtomicPtr;

pub type NtStatus = i32;
pub type Handle = *mut c_void;
type DriverDispatch = unsafe extern "system" fn(*mut DeviceObject, *mut Irp) -> NtStatus;

pub const STATUS_SUCCESS: NtStatus = 0;
pub const STATUS_PENDING: NtStatus = 0x103;
pub const STATUS_CANCELLED: NtStatus = 0xC0000120_u32 as i32;
pub const STATUS_DEVICE_NOT_CONNECTED: NtStatus = 0xC000009D_u32 as i32;

pub const IRP_MJ_CREATE: usize = 0;
pub const IRP_MJ_CLOSE: usize = 2;
pub const IRP_MJ_READ: usize = 3;
pub const IRP_MJ_WRITE: usize = 4;
pub const IRP_MJ_DEVICE_CONTROL: usize = 14;

pub const IO_NO_INCREMENT: u8 = 0;
pub const METHOD_BUFFERED: u32 = 0;
pub const NON_PAGED_POOL: i32 = 0;
pub const SL_PENDING_RETURNED: u8 = 1;
pub const FILE_DEVICE_UNKNOWN: u32 = 0x22;
pub const DO_DIRECT_IO: u32 = 0x10;
pub const DO_DEVICE_INITIALIZING: u32 = 0x80;
pub const OBJ_CASE_INSENSITIVE: u32 = 0x40;
pub const FILE_OPEN: u32 = 1;
pub const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x20;
pub const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
pub const FILE_SHARE_READ: u32 = 1;
pub const FILE_SHARE_WRITE: u32 = 2;
pub const FILE_READ_DATA: u32 = 1;
pub const FILE_WRITE_DATA: u32 = 2;
pub const SYNCHRONIZE: u32 = 0x100000;
pub const THREAD_ALL_ACCESS: u32 = 0x1FFFFF;

#[repr(C)]
pub struct UnicodeString {
    pub length: u16,
    pub maximum_length: u16,
    pub buffer: *const u16,
}

#[repr(C)]
pub struct ObjectAttributes {
    pub length: u32,
    pub root_directory: Handle,
    pub object_name: *const UnicodeString,
    pub attributes: u32,
    pub security_descriptor: *mut c_void,
    pub security_quality_of_service: *mut c_void,
}

#[repr(C)]
#[derive(Default)]
pub struct IoStatusBlock {
    pub status: NtStatus,
    pub information: usize,
}

#[repr(C)]
pub struct Mdl {
    _header: [u8; 24],
    pub mapped_system_va: *mut c_void,
}

#[repr(C)]
pub struct DeviceObject {
    _header: [u8; 48],
    pub flags: u32,
}

#[repr(C)]
pub struct DriverObject {
    _header: [u8; 112],
    pub major_function: [Option<DriverDispatch>; 28],
}

#[repr(C)]
pub struct IoStackLocation {
    _major_minor_flags: [u8; 3],
    pub control: u8,
    _pad4: u32,
    pub length: u32,
    _pad12: u32,
    pub input_length: u32,
    _pad20: u32,
    pub io_control_code: u32,
    _pad28: [u8; 20],
    pub file_object: *mut c_void,
}

#[repr(C)]
pub struct Irp {
    _header: [u8; 8],
    pub mdl_address: *mut Mdl,
    _flags: u64,
    pub system_buffer: *mut c_void,
    _thread_list_entry: [u64; 2],
    pub io_status: IoStatusBlock,
    _pad64: [u8; 5],
    pub cancel_irql: u8,
    _pad70: [u8; 34],
    pub cancel_routine: AtomicPtr<c_void>,
    _pad112: [u64; 9],
    pub current_stack_location: *mut IoStackLocation,
}

const _: () = {
    assert!(offset_of!(Mdl, mapped_system_va) == 24);
    assert!(offset_of!(DeviceObject, flags) == 48);
    assert!(offset_of!(DriverObject, major_function) == 112);
    assert!(offset_of!(IoStackLocation, control) == 3);
    assert!(offset_of!(IoStackLocation, length) == 8);
    assert!(offset_of!(IoStackLocation, input_length) == 16);
    assert!(offset_of!(IoStackLocation, io_control_code) == 24);
    assert!(offset_of!(IoStackLocation, file_object) == 48);
    assert!(offset_of!(Irp, mdl_address) == 8);
    assert!(offset_of!(Irp, system_buffer) == 24);
    assert!(offset_of!(Irp, io_status) == 48);
    assert!(offset_of!(Irp, cancel_irql) == 69);
    assert!(offset_of!(Irp, cancel_routine) == 104);
    assert!(offset_of!(Irp, current_stack_location) == 184);
};

#[link(name = "ntoskrnl.exe", kind = "raw-dylib", modifiers = "+verbatim")]
unsafe extern "system" {
    pub fn ExAllocatePoolWithTag(pool: i32, size: usize, tag: u32) -> *mut c_void;
    pub fn ExFreePoolWithTag(block: *mut c_void, tag: u32);
    pub fn IoCreateDevice(
        driver: *mut DriverObject,
        extension_size: u32,
        name: *const UnicodeString,
        device_type: u32,
        characteristics: u32,
        exclusive: u8,
        device: *mut *mut DeviceObject,
    ) -> NtStatus;
    pub fn IoCreateSymbolicLink(
        link: *const UnicodeString,
        target: *const UnicodeString,
    ) -> NtStatus;
    pub fn IoDeleteDevice(device: *mut DeviceObject);
    pub fn IofCompleteRequest(irp: *mut Irp, boost: u8);
    pub fn IoReleaseCancelSpinLock(irql: u8);
    pub fn KeAcquireSpinLockRaiseToDpc(lock: *mut usize) -> u8;
    pub fn KeReleaseSpinLock(lock: *mut usize, irql: u8);
    pub fn PsCreateSystemThread(
        thread: *mut Handle,
        access: u32,
        attributes: *const ObjectAttributes,
        process: Handle,
        client: *mut c_void,
        start: extern "system" fn(Handle),
        context: Handle,
    ) -> NtStatus;
    pub fn PsTerminateSystemThread(status: NtStatus) -> NtStatus;
    pub fn ZwClose(handle: Handle) -> NtStatus;
    pub fn ZwCreateFile(
        handle: *mut Handle,
        access: u32,
        attributes: *const ObjectAttributes,
        iosb: *mut IoStatusBlock,
        allocation: *mut i64,
        file_attributes: u32,
        share: u32,
        disposition: u32,
        options: u32,
        ea: *mut c_void,
        ea_length: u32,
    ) -> NtStatus;
    pub fn ZwReadFile(
        handle: Handle,
        event: Handle,
        apc: *mut c_void,
        apc_context: *mut c_void,
        iosb: *mut IoStatusBlock,
        buffer: *mut c_void,
        length: u32,
        offset: *mut i64,
        key: *mut u32,
    ) -> NtStatus;
    pub fn ZwWriteFile(
        handle: Handle,
        event: Handle,
        apc: *mut c_void,
        apc_context: *mut c_void,
        iosb: *mut IoStatusBlock,
        buffer: *const c_void,
        length: u32,
        offset: *mut i64,
        key: *mut u32,
    ) -> NtStatus;
}

#[allow(dead_code)]
#[link(name = "ntoskrnl.exe", kind = "raw-dylib", modifiers = "+verbatim")]
unsafe extern "C" {
    fn memcpy(dest: *mut c_void, src: *const c_void, count: usize) -> *mut c_void;
    fn memmove(dest: *mut c_void, src: *const c_void, count: usize) -> *mut c_void;
    fn memset(dest: *mut c_void, value: i32, count: usize) -> *mut c_void;
}
