#![no_std]

mod ffi;
mod frame;
mod queue;

use core::arch::asm;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::mem::zeroed;
use core::panic::PanicInfo;
use core::ptr::{null, null_mut};
use core::slice::from_raw_parts_mut;
use core::sync::atomic::Ordering;

use ffi::*;
use frame::{RX_FRAME_MAX, classify_high_priority, tlv_encode, tlv_frames};
use queue::{IrpQueue, Queued, RxRing};

const fn wide<const N: usize>(text: &[u8; N]) -> [u16; N] {
    let mut out = [0; N];
    let mut i = 0;
    while i < N {
        out[i] = text[i] as u16;
        i += 1;
    }
    out
}

const DEVICE_NAME: &[u16] = &wide(b"\\Device\\RVPNNETMP\0");
const DOSDEVICE_NAME: &[u16] = &wide(b"\\DosDevices\\RVPNNETMP\0");
const FIFO_B2D: &[u16] = &wide(b"\\??\\Z:\\tmp\\rvpn_b2d\0");
const FIFO_D2B_HIGH: &[u16] = &wide(b"\\??\\Z:\\tmp\\rvpn_d2b_high\0");
const FIFO_D2B_LOW: &[u16] = &wide(b"\\??\\Z:\\tmp\\rvpn_d2b_low\0");
const MAC_FILE: &[u16] = &wide(b"\\??\\Z:\\tmp\\rvpn_mac\0");

const IOCTL_RVPN_VERSION: u32 = 0x0022C004;
const IOCTL_RVPN_STATUS: u32 = 0x00224018;
const IOCTL_RVPN_CONNECT: u32 = 0x00224020;
const IOCTL_RVPN_SETLINK: u32 = 0x00224010;
const IOCTL_RVPN_SETUP: u32 = 0x0022801C;
const IOCTL_RVPN_PEERMAC: u32 = 0x00228014;
const IOCTL_RVPN_SETMAC: u32 = 0x00228024;

const DEFAULT_MAC: [u8; 6] = [0x02, 0x50, 0xDE, 0xAD, 0xBE, 0xEF];
const PEER_ROUTES_INITIAL: usize = 16;
const PEER_ROUTES_MAX: usize = 4096;
const PEER_ROUTES_TAG: u32 = u32::from_be_bytes(*b"rPvR");

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    unsafe { asm!("ud2", options(noreturn)) }
}

#[derive(Clone, Copy)]
struct PeerRoute {
    file_object: *mut c_void,
    mac: [u8; 6],
}

#[derive(Clone, Copy)]
struct Fifos {
    b2d: Handle,
    d2b_high: Handle,
    d2b_low: Handle,
}

struct State {
    mode: u32,
    fifos: Fifos,
    mac: Option<[u8; 6]>,
    queue: IrpQueue,
    picked: IrpQueue,
    ring: RxRing,
    routes: Option<&'static mut [PeerRoute]>,
}

impl State {
    fn routes(&mut self) -> &mut [PeerRoute] {
        self.routes.as_deref_mut().unwrap_or_default()
    }
}

struct SpinLock {
    lock: UnsafeCell<usize>,
    state: UnsafeCell<State>,
}

unsafe impl Sync for SpinLock {}

static STATE: SpinLock = unsafe { zeroed() };

const _: () = {
    let state: State = unsafe { zeroed() };
    assert!(state.mac.is_none() && state.routes.is_none());
};

fn locked<R>(body: impl FnOnce(&mut State) -> R) -> R {
    unsafe {
        let irql = KeAcquireSpinLockRaiseToDpc(STATE.lock.get());
        let result = body(&mut *STATE.state.get());
        KeReleaseSpinLock(STATE.lock.get(), irql);
        result
    }
}

fn unicode(text: &[u16]) -> UnicodeString {
    let size = size_of_val(text) as u16;
    UnicodeString {
        length: size - 2,
        maximum_length: size,
        buffer: text.as_ptr(),
    }
}

fn open_file(path: &[u16], access: u32) -> Handle {
    let name = unicode(path);
    let attributes = ObjectAttributes {
        length: size_of::<ObjectAttributes>() as u32,
        root_directory: null_mut(),
        object_name: &name,
        attributes: OBJ_CASE_INSENSITIVE,
        security_descriptor: null_mut(),
        security_quality_of_service: null_mut(),
    };
    let mut handle = null_mut();
    let status = unsafe {
        ZwCreateFile(
            &mut handle,
            access | SYNCHRONIZE,
            &attributes,
            &mut IoStatusBlock::default(),
            null_mut(),
            FILE_ATTRIBUTE_NORMAL,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_OPEN,
            FILE_SYNCHRONOUS_IO_NONALERT,
            null_mut(),
            0,
        )
    };
    if status < 0 { null_mut() } else { handle }
}

fn read_file(file: Handle, buffer: &mut [u8]) -> usize {
    let mut iosb = IoStatusBlock::default();
    let status = unsafe {
        ZwReadFile(
            file,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut iosb,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
            null_mut(),
            null_mut(),
        )
    };
    if status < 0 { 0 } else { iosb.information }
}

fn read_exact(fifo: Handle, buffer: &mut [u8]) -> bool {
    let mut got = 0;
    while got < buffer.len() {
        let count = read_file(fifo, &mut buffer[got..]);
        if count == 0 {
            return false;
        }
        got += count;
    }
    true
}

unsafe fn complete(irp: *mut Irp, status: NtStatus, information: usize) -> NtStatus {
    unsafe {
        (*irp).io_status.status = status;
        (*irp).io_status.information = information;
        IofCompleteRequest(irp, IO_NO_INCREMENT);
    }
    status
}

unsafe fn claim(irp: *mut Irp) -> bool {
    !unsafe { (*irp).cancel_routine.swap(null_mut(), Ordering::SeqCst) }.is_null()
}

unsafe fn direct_buffer<'a>(irp: *mut Irp) -> &'a mut [u8] {
    unsafe {
        let buffer = (*(*irp).mdl_address).mapped_system_va.cast();
        from_raw_parts_mut(buffer, (*(*irp).current_stack_location).length as usize)
    }
}

fn deliver(frame: &[u8]) {
    let (count, mode) = locked(|state| {
        if state.queue.len == 0 {
            state.ring.push(frame);
            return (0, 0);
        }
        let group = frame[0] & 1 != 0;
        let mac = frame.first_chunk::<6>();
        let route = state
            .routes()
            .iter()
            .find(|route| !route.file_object.is_null() && Some(&route.mac) == mac)
            .map(|route| route.file_object);
        state.queue.take_first_per_file(&mut state.picked, |entry| {
            (group || Some(entry.file_object) == route) && unsafe { claim(entry.irp) }
        });
        (state.picked.len, state.mode)
    });
    for index in 0..count {
        let irp = locked(|state| state.picked.entries[index].irp);
        unsafe {
            let written = tlv_encode(direct_buffer(irp), frame, mode);
            complete(irp, STATUS_SUCCESS, written);
        }
    }
}

extern "system" fn rx_thread(fifo: Handle) {
    let mut prefix = [0; 2];
    let mut frame = [0; RX_FRAME_MAX];
    'frames: while read_exact(fifo, &mut prefix) {
        let len = usize::from(u16::from_le_bytes(prefix));
        let mut rem = len;
        while rem > 0 {
            let chunk = rem.min(RX_FRAME_MAX);
            if !read_exact(fifo, &mut frame[..chunk]) {
                break 'frames;
            }
            rem -= chunk;
        }
        if (1..=RX_FRAME_MAX).contains(&len) {
            deliver(&frame[..len]);
        }
    }
    unsafe { PsTerminateSystemThread(STATUS_SUCCESS) };
}

fn load_adapter_mac() {
    if locked(|state| state.mac.is_some()) {
        return;
    }
    let file = open_file(MAC_FILE, FILE_READ_DATA);
    if file.is_null() {
        return;
    }
    let mut mac = [0; 6];
    if read_file(file, &mut mac) == 6 {
        locked(|state| state.mac = Some(mac));
    }
    unsafe { ZwClose(file) };
}

unsafe extern "system" fn dispatch_create(_: *mut DeviceObject, irp: *mut Irp) -> NtStatus {
    load_adapter_mac();
    let mut fifos = locked(|state| state.fifos);
    if fifos.b2d.is_null() {
        fifos.b2d = open_file(FIFO_B2D, FILE_READ_DATA);
        if !fifos.b2d.is_null() {
            let mut thread = null_mut();
            let status = unsafe {
                PsCreateSystemThread(
                    &mut thread,
                    THREAD_ALL_ACCESS,
                    null(),
                    null_mut(),
                    null_mut(),
                    rx_thread,
                    fifos.b2d,
                )
            };
            if status >= 0 {
                unsafe { ZwClose(thread) };
            }
        }
    }
    if fifos.d2b_high.is_null() {
        fifos.d2b_high = open_file(FIFO_D2B_HIGH, FILE_WRITE_DATA);
    }
    if fifos.d2b_low.is_null() {
        fifos.d2b_low = open_file(FIFO_D2B_LOW, FILE_WRITE_DATA);
    }
    locked(|state| state.fifos = fifos);
    unsafe { complete(irp, STATUS_SUCCESS, 0) }
}

unsafe extern "system" fn dispatch_close(_: *mut DeviceObject, irp: *mut Irp) -> NtStatus {
    let file_object = unsafe { (*(*irp).current_stack_location).file_object };
    locked(|state| {
        for route in state
            .routes()
            .iter_mut()
            .filter(|r| r.file_object == file_object)
        {
            route.file_object = null_mut();
        }
    });
    unsafe { complete(irp, STATUS_SUCCESS, 0) }
}

fn grow_routes(state: &mut State) {
    let capacity = (state.routes().len() * 2).max(PEER_ROUTES_INITIAL);
    if capacity > PEER_ROUTES_MAX {
        return;
    }
    let size = capacity * size_of::<PeerRoute>();
    let block = unsafe { ExAllocatePoolWithTag(NON_PAGED_POOL, size, PEER_ROUTES_TAG) };
    if block.is_null() {
        return;
    }
    let grown = unsafe {
        block.write_bytes(0, size);
        from_raw_parts_mut(block.cast::<PeerRoute>(), capacity)
    };
    let old = state.routes();
    grown[..old.len()].copy_from_slice(old);
    if let Some(old) = state.routes.replace(grown) {
        unsafe { ExFreePoolWithTag(old.as_mut_ptr().cast(), PEER_ROUTES_TAG) };
    }
}

fn add_peer_route(file_object: *mut c_void, mac: &[u8]) {
    locked(|state| {
        if state
            .routes()
            .iter()
            .all(|route| !route.file_object.is_null())
        {
            grow_routes(state);
        }
        if let Some(route) = state.routes().iter_mut().find(|r| r.file_object.is_null()) {
            route.file_object = file_object;
            route.mac.copy_from_slice(mac);
        }
    });
}

fn device_control(stack: &IoStackLocation, buffer: &mut [u8]) -> usize {
    let (input, output) = (stack.input_length as usize, stack.length as usize);
    let word = buffer
        .first_chunk()
        .map_or(0, |bytes| u32::from_le_bytes(*bytes));
    match stack.io_control_code {
        IOCTL_RVPN_VERSION if output >= 12 => {
            buffer[..12].fill(0);
            if input >= 4 && word == 4 {
                let mac = locked(|state| state.mac);
                buffer[4..10].copy_from_slice(&mac.unwrap_or(DEFAULT_MAC));
            } else {
                buffer[0] = 1;
            }
            12
        }
        IOCTL_RVPN_STATUS if output >= 4 => {
            buffer[..4].copy_from_slice(&1u32.to_le_bytes());
            4
        }
        IOCTL_RVPN_CONNECT if output >= 1 => {
            buffer[0] = 0x20;
            1
        }
        IOCTL_RVPN_SETLINK if output >= 0xB8 => {
            buffer[..0xB8].fill(0);
            0xB8
        }
        IOCTL_RVPN_SETMAC if output >= 1 => {
            buffer[0] = 0;
            1
        }
        IOCTL_RVPN_SETUP if input >= 4 => {
            locked(|state| state.mode = word);
            0
        }
        IOCTL_RVPN_PEERMAC if input >= 6 => {
            add_peer_route(stack.file_object, &buffer[..6]);
            0
        }
        _ => 0,
    }
}

unsafe extern "system" fn dispatch_device_control(_: *mut DeviceObject, irp: *mut Irp) -> NtStatus {
    let stack = unsafe { &*(*irp).current_stack_location };
    let mut info = 0;
    if stack.io_control_code & 3 == METHOD_BUFFERED {
        let length = stack.input_length.max(stack.length) as usize;
        let buffer = unsafe { (*irp).system_buffer }.cast();
        info = device_control(stack, unsafe { from_raw_parts_mut(buffer, length) });
    }
    unsafe { complete(irp, STATUS_SUCCESS, info) }
}

unsafe extern "system" fn cancel_routine(_: *mut DeviceObject, irp: *mut Irp) {
    unsafe { IoReleaseCancelSpinLock((*irp).cancel_irql) };
    locked(|state| state.queue.take(|entry| entry.irp == irp));
    unsafe { complete(irp, STATUS_CANCELLED, 0) };
}

unsafe extern "system" fn dispatch_read(_: *mut DeviceObject, irp: *mut Irp) -> NtStatus {
    let mut evicted = None;
    let done = locked(|state| {
        if state.fifos.b2d.is_null() {
            return Some((STATUS_DEVICE_NOT_CONNECTED, 0));
        }
        if state.ring.has_frames() {
            let out = unsafe { direct_buffer(irp) };
            return Some((STATUS_SUCCESS, state.ring.drain(out, state.mode)));
        }
        if state.queue.is_full() {
            evicted = state.queue.take(|entry| unsafe { claim(entry.irp) });
        }
        let file_object = unsafe {
            let stack = (*irp).current_stack_location;
            (*stack).control |= SL_PENDING_RETURNED;
            (*irp)
                .cancel_routine
                .store(cancel_routine as *mut c_void, Ordering::SeqCst);
            (*stack).file_object
        };
        state.queue.push(Queued { irp, file_object });
        None
    });
    if let Some((status, written)) = done {
        return unsafe { complete(irp, status, written) };
    }
    if let Some(oldest) = evicted {
        unsafe { complete(oldest, STATUS_CANCELLED, 0) };
    }
    STATUS_PENDING
}

unsafe extern "system" fn dispatch_write(_: *mut DeviceObject, irp: *mut Irp) -> NtStatus {
    let (mode, fifos) = locked(|state| (state.mode, state.fifos));
    let input = unsafe { direct_buffer(irp) };
    let mut wire = [0; 2 + RX_FRAME_MAX];
    for frame in tlv_frames(input, mode) {
        let fifo = if classify_high_priority(frame) {
            fifos.d2b_high
        } else {
            fifos.d2b_low
        };
        if !fifo.is_null() {
            let total = 2 + frame.len();
            wire[..2].copy_from_slice(&(frame.len() as u16).to_le_bytes());
            wire[2..total].copy_from_slice(frame);
            unsafe {
                ZwWriteFile(
                    fifo,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    &mut IoStatusBlock::default(),
                    wire.as_ptr().cast(),
                    total as u32,
                    null_mut(),
                    null_mut(),
                );
            }
        }
    }
    unsafe { complete(irp, STATUS_SUCCESS, input.len()) }
}

#[unsafe(no_mangle)]
unsafe extern "system" fn DriverEntry(
    driver: *mut DriverObject,
    _: *mut UnicodeString,
) -> NtStatus {
    let name = unicode(DEVICE_NAME);
    let link = unicode(DOSDEVICE_NAME);
    let mut device = null_mut();
    unsafe {
        let status = IoCreateDevice(driver, 0, &name, FILE_DEVICE_UNKNOWN, 0, 0, &mut device);
        if status < 0 {
            return status;
        }
        (*device).flags |= DO_DIRECT_IO;
        (*device).flags &= !DO_DEVICE_INITIALIZING;
        let major_function = &mut (*driver).major_function;
        major_function[IRP_MJ_CREATE] = Some(dispatch_create);
        major_function[IRP_MJ_CLOSE] = Some(dispatch_close);
        major_function[IRP_MJ_DEVICE_CONTROL] = Some(dispatch_device_control);
        major_function[IRP_MJ_READ] = Some(dispatch_read);
        major_function[IRP_MJ_WRITE] = Some(dispatch_write);
        let status = IoCreateSymbolicLink(&link, &name);
        if status < 0 {
            IoDeleteDevice(device);
            return status;
        }
    }
    STATUS_SUCCESS
}
