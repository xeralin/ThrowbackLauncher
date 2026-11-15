#![no_std]
#![no_main]
#![windows_subsystem = "console"]

#[path = "../../common/mem.rs"]
mod mem;

use core::ffi::CStr;
use core::mem::transmute;
use core::panic::PanicInfo;
use core::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, TRUE};
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::Environment::GetCommandLineA;
use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameA, LoadLibraryA};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAllocEx, VirtualFreeEx,
};
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CreateProcessA, CreateRemoteThread, ExitProcess, GetExitCodeProcess,
    INFINITE, LPTHREAD_START_ROUTINE, PROCESS_INFORMATION, ResumeThread, STARTUPINFOA,
    WaitForSingleObject,
};

const MAX_PATH: usize = 260;
const CMDLINE_MAX: usize = 1024;
const INJECT_WAIT_MS: u32 = 5000;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    unsafe { ExitProcess(1) }
}

fn concat<const N: usize>(parts: &[&[u8]]) -> Option<[u8; N]> {
    let mut out = [0; N];
    let mut len = 0;
    for part in parts {
        out.get_mut(len..len + part.len())?.copy_from_slice(part);
        len += part.len();
    }
    (len < N).then_some(out)
}

fn skip_argv0(line: &[u8]) -> &[u8] {
    let end = match line {
        [b'"', rest @ ..] => rest
            .iter()
            .position(|&byte| byte == b'"')
            .map(|quote| quote + 2),
        _ => line.iter().position(|&byte| matches!(byte, b' ' | b'\t')),
    };
    &line[end.unwrap_or(line.len())..]
}

fn run() -> u32 {
    let mut exe = [0; MAX_PATH];
    unsafe { GetModuleFileNameA(null_mut(), exe.as_mut_ptr(), MAX_PATH as u32) };
    let cut = exe.iter().rposition(|&byte| byte == b'\\').unwrap_or(0);
    exe[cut] = 0;
    let dir = &exe[..cut];
    let args = skip_argv0(unsafe { CStr::from_ptr(GetCommandLineA().cast()) }.to_bytes());
    let Some(mut line) = concat::<CMDLINE_MAX>(&[b"\"", dir, b"\\RvControlSvc.exe\"", args]) else {
        return 1;
    };
    let si = STARTUPINFOA {
        cb: size_of::<STARTUPINFOA>() as u32,
        ..Default::default()
    };
    let mut pi = PROCESS_INFORMATION::default();
    let created = unsafe {
        CreateProcessA(
            null(),
            line.as_mut_ptr(),
            null(),
            null(),
            TRUE,
            CREATE_SUSPENDED,
            null(),
            exe.as_ptr(),
            &si,
            &mut pi,
        )
    };
    if created == 0 {
        return 1;
    }
    let process = pi.hProcess;
    let mut code = 1;
    unsafe {
        if let Some(dll) = concat::<MAX_PATH>(&[dir, b"\\adapter_hook.dll"]) {
            let commit = MEM_COMMIT | MEM_RESERVE;
            let remote = VirtualAllocEx(process, null(), dll.len(), commit, PAGE_READWRITE);
            WriteProcessMemory(process, remote, dll.as_ptr().cast(), dll.len(), null_mut());
            let entry: LPTHREAD_START_ROUTINE = transmute(LoadLibraryA as *const ());
            let thread = CreateRemoteThread(process, null(), 0, entry, remote, 0, null_mut());
            WaitForSingleObject(thread, INJECT_WAIT_MS);
            CloseHandle(thread);
            VirtualFreeEx(process, remote, 0, MEM_RELEASE);
        }
        ResumeThread(pi.hThread);
        WaitForSingleObject(process, INFINITE);
        GetExitCodeProcess(process, &mut code);
        CloseHandle(pi.hThread);
        CloseHandle(process);
    }
    code
}

#[unsafe(no_mangle)]
extern "C" fn mainCRTStartup() -> ! {
    unsafe { ExitProcess(run()) }
}
