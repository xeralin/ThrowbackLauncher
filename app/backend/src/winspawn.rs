#[cfg(any(windows, test))]
use std::ffi::OsStr;
#[cfg(any(windows, test))]
use std::io;
use std::process::Command;
#[cfg(any(windows, test))]
use std::process::{Child, Stdio};

const NOWINDOW: u32 = 0x0800_0000;
#[cfg(any(windows, test))]
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
pub fn powershell(script: &str) {
    let mut command = Command::new("powershell");
    command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    let _ = hidden(&mut command).status();
}

#[cfg(windows)]
pub fn spawn_detached<S: AsRef<OsStr>>(argv: &[S]) -> anyhow::Result<()> {
    let workdir = std::env::var_os("SYSTEMROOT").unwrap_or_else(|| r"C:\Windows".into());
    let child =
        spawn(argv, &workdir, NOWINDOW | BREAKAWAY).or_else(|_| spawn(argv, &workdir, NOWINDOW))?;
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(child.id()) };
    Ok(())
}

#[cfg(any(windows, test))]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_argv_is_refused() {
        let error = spawn(&[""; 0], OsStr::new("."), NOWINDOW).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn child_gets_the_workdir_null_streams_and_the_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let script = r#"streams=$(readlink /proc/$$/fd/0 /proc/$$/fd/1 /proc/$$/fd/2); printf '%s\n' "$streams" > streams; printf '%s\n' "$@" > arguments"#;
        let argv = ["sh", "-c", script, "sh", "--relaunch", "two words", ""];
        let mut child = spawn(&argv, dir.path().as_os_str(), NOWINDOW | BREAKAWAY).unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("streams")).unwrap(),
            "/dev/null\n/dev/null\n/dev/null\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("arguments")).unwrap(),
            "--relaunch\ntwo words\n\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn missing_workdir_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path().join("missing");
        let error = spawn(&["true"], workdir.as_os_str(), NOWINDOW).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
