#![no_std]
#![no_main]
#![windows_subsystem = "console"]

#[path = "../../common/mem.rs"]
mod mem;

use core::net::{Ipv4Addr, Ipv6Addr};
use core::panic::PanicInfo;
use core::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileA, FILE_APPEND_DATA, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_ALWAYS, WriteFile,
};
use windows_sys::Win32::System::Environment::GetCommandLineW;
use windows_sys::Win32::System::Threading::{ExitProcess, Sleep};

const CMDLINE_MAX: usize = 2048;
const CMD_DELAY_MS: u32 = 500;
const LINE_V4: Line = (b"ip addr add ", b"/8 dev radminvpn0\n");
const LINE_V6: Line = (b"ip -6 addr add ", b"/64 dev radminvpn0\n");
const ADDRESS_MAX: usize = 45;

type Line = (&'static [u8], &'static [u8]);

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    unsafe { ExitProcess(0) }
}

fn after<'a>(cmd: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let at = cmd.windows(key.len()).position(|window| window == key)?;
    Some(&cmd[at + key.len()..])
}

fn address(cmd: &[u8]) -> Option<(&[u8], Line)> {
    after(cmd, b"interface")?;
    let value = if after(cmd, b"set address").is_some() {
        after(cmd, b"address=")?
    } else {
        after(cmd, b"add address")?;
        after(cmd, b"addr=").or_else(|| after(cmd, b"address="))?
    };
    let value = value.split(|&byte| byte == b' ').next()?;
    let text = str::from_utf8(value).ok()?;
    if text.parse::<Ipv4Addr>().is_ok() {
        return Some((value, LINE_V4));
    }
    let v6 = text.parse::<Ipv6Addr>().ok()?;
    let listed = after(cmd, b"ipv6").is_some() && !v6.is_unicast_link_local();
    listed.then_some((value, LINE_V6))
}

#[unsafe(no_mangle)]
extern "C" fn mainCRTStartup() -> u32 {
    let args = unsafe { GetCommandLineW() };
    let mut cmd = [0; CMDLINE_MAX];
    let mut len = 0;
    for unit in (0..)
        .map(|at| unsafe { *args.add(at) })
        .take_while(|&unit| unit != 0)
    {
        let byte = match unit {
            0x22 => continue,
            0x09 | 0x20 if len > 0 && cmd[len - 1] == b' ' => continue,
            0x09 | 0x20 => b' ',
            0..0x80 => unit as u8,
            _ => b'?',
        };
        if len == CMDLINE_MAX {
            break;
        }
        cmd[len] = byte;
        len += 1;
    }
    if let Some((value, (prefix, suffix))) = address(&cmd[..len]) {
        let mut line = [0; LINE_V6.0.len() + ADDRESS_MAX + LINE_V6.1.len()];
        let end = prefix.len() + value.len();
        line[..prefix.len()].copy_from_slice(prefix);
        line[prefix.len()..end].copy_from_slice(value);
        line[end..end + suffix.len()].copy_from_slice(suffix);
        let written = (end + suffix.len()) as u32;
        unsafe {
            let file = CreateFileA(
                c"Z:\\tmp\\rvpn_netsh_cmd".as_ptr().cast(),
                FILE_APPEND_DATA,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_ALWAYS,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            );
            if file != INVALID_HANDLE_VALUE {
                WriteFile(file, line.as_ptr(), written, &mut 0, null_mut());
                CloseHandle(file);
                Sleep(CMD_DELAY_MS);
            }
        }
    }
    0
}
