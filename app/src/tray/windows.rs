use std::{io, mem, ptr};

use async_channel::Sender;
use backend::env::{APP_NAME, DIR_NAME};
use backend::log;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GWLP_USERDATA, GetCursorPos, GetSystemMetrics, GetWindowLongPtrW, IMAGE_ICON, LR_SHARED,
    LoadImageW, MF_STRING, RegisterClassW, SM_CXSMICON, SM_CYSMICON, SetForegroundWindow,
    SetWindowLongPtrW, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_LBUTTONUP,
    WM_MBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
};

use super::{ENTRIES, Event};

const NOTICE: u32 = WM_APP;
const ICON_RESOURCE: usize = 1;

pub async fn available() -> bool {
    true
}

pub struct Icon {
    window: HWND,
    _events: Box<Sender<Event>>,
}

impl Drop for Icon {
    fn drop(&mut self) {
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &notice(self.window));
            DestroyWindow(self.window);
        }
    }
}

pub async fn show(events: Sender<Event>) -> Option<Icon> {
    let class = wide(DIR_NAME);
    let events = Box::new(events);
    let window = unsafe {
        let module = GetModuleHandleW(ptr::null());
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(handle),
            hInstance: module,
            lpszClassName: class.as_ptr(),
            ..mem::zeroed()
        });
        let parent = ptr::null_mut();
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            ptr::null(),
            0,
            0,
            0,
            0,
            0,
            parent,
            ptr::null_mut(),
            module,
            ptr::null(),
        );
        SetWindowLongPtrW(window, GWLP_USERDATA, &raw const *events as isize);
        window
    };
    let icon = Icon {
        window,
        _events: events,
    };
    if unsafe { Shell_NotifyIconW(NIM_ADD, &notice(window)) } == 0 {
        log::fail("Tray icon failed", io::Error::last_os_error());
        return None;
    }
    Some(icon)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn notice(window: HWND) -> NOTIFYICONDATAW {
    let mut notice: NOTIFYICONDATAW = unsafe { mem::zeroed() };
    notice.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    notice.hWnd = window;
    notice.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    notice.uCallbackMessage = NOTICE;
    notice.hIcon = unsafe {
        LoadImageW(
            GetModuleHandleW(ptr::null()),
            ICON_RESOURCE as *const u16,
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_SHARED,
        )
    };
    for (slot, unit) in notice.szTip.iter_mut().zip(APP_NAME.encode_utf16()) {
        *slot = unit;
    }
    notice
}

unsafe extern "system" fn handle(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message != NOTICE {
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }
    let event = match lparam as u32 {
        WM_LBUTTONUP | WM_MBUTTONUP => Some(Event::Open),
        WM_RBUTTONUP => picked(window),
        _ => None,
    };
    if let Some(event) = event {
        let events = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *const Sender<Event>;
        let _ = unsafe { &*events }.try_send(event);
    }
    0
}

fn picked(window: HWND) -> Option<Event> {
    let choice = unsafe {
        let menu = CreatePopupMenu();
        for (index, (label, _)) in ENTRIES.iter().enumerate() {
            AppendMenuW(menu, MF_STRING, index + 1, wide(label).as_ptr());
        }
        let mut pointer = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pointer);
        SetForegroundWindow(window);
        let flags = TPM_RETURNCMD | TPM_RIGHTBUTTON;
        let choice = TrackPopupMenu(menu, flags, pointer.x, pointer.y, 0, window, ptr::null());
        DestroyMenu(menu);
        choice
    };
    let index = usize::try_from(choice).ok()?.checked_sub(1)?;
    Some(ENTRIES.get(index)?.1)
}
