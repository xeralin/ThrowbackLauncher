#[cfg(windows)]
use std::ffi::OsStr;
#[cfg(windows)]
use std::io;
use std::process::Command;
#[cfg(windows)]
use std::process::{Child, Stdio};

const NOWINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const BREAKAWAY: u32 = 0x0100_0000;

#[cfg(windows)]
fn creation_flags(command: &mut Command, flags: u32) -> &mut Command {
    use std::os::windows::process::CommandExt;
    command.creation_flags(flags)
}

#[cfg(not(windows))]
fn creation_flags(command: &mut Command, _flags: u32) -> &mut Command {
    command
}

pub fn hidden(command: &mut Command) -> &mut Command {
    creation_flags(command, NOWINDOW)
}

#[cfg(windows)]
pub fn spawn_detached<S: AsRef<OsStr>>(argv: &[S]) -> anyhow::Result<()> {
    let workdir = std::env::var_os("SYSTEMROOT").unwrap_or_else(|| r"C:\Windows".into());
    let child =
        spawn(argv, &workdir, NOWINDOW | BREAKAWAY).or_else(|_| spawn(argv, &workdir, NOWINDOW))?;
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(child.id()) };
    Ok(())
}

#[cfg(windows)]
fn spawn<S: AsRef<OsStr>>(argv: &[S], workdir: &OsStr, flags: u32) -> io::Result<Child> {
    let (program, args) = argv.split_first().ok_or(io::ErrorKind::InvalidInput)?;
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    creation_flags(&mut command, flags).spawn()
}
