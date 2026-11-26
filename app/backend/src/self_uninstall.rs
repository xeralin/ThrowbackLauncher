#[cfg(any(windows, test))]
use std::path::Path;
#[cfg(any(windows, test))]
use std::time::Duration;
#[cfg(any(windows, test))]
use std::{fs, thread};

#[cfg(any(windows, test))]
use crate::env::{EXE_NAME, Paths};

#[cfg(any(windows, test))]
const ATTEMPTS: u32 = 10;
#[cfg(windows)]
const PAUSE: Duration = Duration::from_secs(1);

#[cfg(windows)]
pub fn run(paths: &Paths) {
    use crate::env::{PACKAGED, UNINSTALL_KEY, unlink};
    use crate::self_install::start_menu_shortcut;

    if !PACKAGED {
        return;
    }
    if let Some(shortcut) = start_menu_shortcut() {
        let _ = unlink(&shortcut);
    }
    let _ = unlink(&desktop_shortcut());
    let _ = winreg::HKCU.delete_subkey(UNINSTALL_KEY);
    let _ = crate::depot::wipe_depot_token();
    if let Err(error) = system::hand_off(&paths.data_root) {
        crate::log::fail("Uninstall helper failed to start", format!("{error:#}"));
    }
}

#[cfg(windows)]
pub fn remove_installation(args: &[String]) -> bool {
    use crate::env::REMOVE_ARG;

    let Some(index) = args.iter().position(|arg| arg == REMOVE_ARG) else {
        return false;
    };
    let pid = args.get(index + 1).and_then(|pid| pid.parse().ok());
    if let (Some(pid), Some(root)) = (pid, args.get(index + 2).map(Path::new))
        && root.is_absolute()
        && root.join(EXE_NAME).is_file()
    {
        system::wait_for(pid);
        system::stop_programs_in(root);
        remove(&Paths::new(root.to_path_buf(), root.to_path_buf()), PAUSE);
    }
    system::release_self();
    true
}

#[cfg(any(windows, test))]
fn inside(root: &Path, program: &Path) -> bool {
    let root = format!(r"{}\", root.display()).to_lowercase();
    program.to_string_lossy().to_lowercase().starts_with(&root)
}

#[cfg(any(windows, test))]
fn remove(paths: &Paths, pause: Duration) -> bool {
    let root = &paths.data_root;
    let keep = paths.default_downloads_dir();
    let kept = keep.file_name().unwrap_or_default();
    let program = root.join(EXE_NAME);
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            thread::sleep(pause);
        }
        if fs::remove_file(&program).is_err() && program.exists() {
            continue;
        }
        for entry in fs::read_dir(root).into_iter().flatten().flatten() {
            if entry.file_name().eq_ignore_ascii_case(kept) {
                continue;
            }
            let path = entry.path();
            let _ = if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
        }
        let _ = fs::remove_dir(&keep);
        let _ = fs::remove_dir(root);
        let mut rest = fs::read_dir(root).into_iter().flatten().flatten();
        if !rest.any(|entry| !entry.file_name().eq_ignore_ascii_case(kept)) {
            return true;
        }
    }
    false
}

#[cfg(windows)]
pub(crate) fn desktop_shortcut() -> std::path::PathBuf {
    let folder = winreg::HKCU
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders")
        .and_then(|key| key.get_value("Desktop"))
        .ok();
    plan::desktop_shortcut(folder, &crate::env::home())
}

#[cfg(any(windows, test))]
mod plan {
    use std::path::{Path, PathBuf};

    use crate::env::APP_NAME;

    pub(super) fn desktop_shortcut(folder: Option<String>, home: &Path) -> PathBuf {
        folder
            .filter(|folder| !folder.is_empty())
            .map_or_else(|| home.join("Desktop"), PathBuf::from)
            .join(format!("{APP_NAME}.lnk"))
    }
}

#[cfg(windows)]
mod system {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, HandleOrInvalid, HandleOrNull, OwnedHandle};
    use std::path::Path;
    use std::{io, ptr};

    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, DELETE, FILE_FLAG_DELETE_ON_CLOSE, FILE_SHARE_DELETE, FILE_SHARE_READ,
        OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
    };

    use crate::env::{DIR_NAME, REMOVE_ARG};
    use crate::winspawn::spawn_detached;

    const WAIT_MS: u32 = 30_000;

    pub(super) fn hand_off(root: &Path) -> anyhow::Result<()> {
        let pid = std::process::id();
        let helper = std::env::temp_dir().join(format!("{DIR_NAME}.uninstall.{pid}.exe"));
        std::fs::copy(std::env::current_exe()?, &helper)?;
        let _doomed = delete_on_close(&helper).inspect_err(|_| {
            let _ = std::fs::remove_file(&helper);
        })?;
        let pid = pid.to_string();
        spawn_detached(&[
            helper.as_os_str(),
            OsStr::new(REMOVE_ARG),
            OsStr::new(&pid),
            root.as_os_str(),
        ])
    }

    fn delete_on_close(path: &Path) -> io::Result<OwnedHandle> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let inherited = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: ptr::null_mut(),
            bInheritHandle: 1,
        };
        let raw = unsafe {
            CreateFileW(
                wide.as_ptr(),
                DELETE,
                FILE_SHARE_READ | FILE_SHARE_DELETE,
                &inherited,
                OPEN_EXISTING,
                FILE_FLAG_DELETE_ON_CLOSE,
                ptr::null_mut(),
            )
        };
        OwnedHandle::try_from(unsafe { HandleOrInvalid::from_raw_handle(raw) })
            .map_err(|_| io::Error::last_os_error())
    }

    pub(super) fn wait_for(pid: u32) {
        if let Some(handle) = open(PROCESS_SYNCHRONIZE, pid) {
            unsafe { WaitForSingleObject(handle.as_raw_handle(), WAIT_MS) };
        }
    }

    fn open(access: u32, pid: u32) -> Option<OwnedHandle> {
        let opened = unsafe { OpenProcess(access, 0, pid) };
        OwnedHandle::try_from(unsafe { HandleOrNull::from_raw_handle(opened) }).ok()
    }

    pub(super) fn stop_programs_in(root: &Path) {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

        let Ok(mut system) = System::new() else {
            return;
        };
        let exe = ProcessRefreshKind::nothing().with_exe(UpdateKind::Always);
        system.refresh_processes_specifics(ProcessesToUpdate::All, true, exe);
        for process in system.processes().values() {
            if process
                .exe()
                .is_some_and(|program| super::inside(root, program))
                && let Some(handle) = open(PROCESS_TERMINATE, process.pid().as_u32())
            {
                unsafe { TerminateProcess(handle.as_raw_handle(), 1) };
            }
        }
    }

    pub(super) fn release_self() {
        let system = std::env::var_os("SYSTEMROOT").unwrap_or_else(|| r"C:\Windows".into());
        let shell = Path::new(&system).join(r"System32\cmd.exe");
        let _ = spawn_detached(&[shell.as_os_str(), OsStr::new("/c"), OsStr::new("exit")]);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::plan::desktop_shortcut;
    use super::*;
    use crate::throwbackloader::testing::names;

    #[test]
    fn the_installation_is_removed_but_downloaded_games_stay() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ThrowbackLauncher");
        let paths = Paths::new(root.clone(), root.clone());
        let season = paths.default_downloads_dir().join("Y1S0");
        fs::create_dir_all(paths.bin_dir().join("rvpn")).unwrap();
        fs::create_dir_all(&season).unwrap();
        fs::create_dir_all(root.join("logs")).unwrap();
        fs::write(season.join("RainbowSix.exe"), "").unwrap();
        for file in [EXE_NAME, "settings.toml", "ThrowbackLauncher.old"] {
            fs::write(root.join(file), "").unwrap();
        }
        assert!(remove(&paths, Duration::ZERO));
        assert_eq!(names(&root), ["downloads"]);
        assert_eq!(names(&season), ["RainbowSix.exe"]);

        fs::remove_dir_all(&season).unwrap();
        fs::write(root.join(EXE_NAME), "").unwrap();
        assert!(remove(&paths, Duration::ZERO));
        assert!(!root.exists());
    }

    #[test]
    fn only_programs_inside_the_installation_are_stopped() {
        let root = Path::new(r"D:\Jürgen's Ordner\ThrowbackLauncher");
        let inside = |program: &str| inside(root, Path::new(program));
        assert!(inside(
            r"D:\Jürgen's Ordner\ThrowbackLauncher\ThrowbackLauncher.exe"
        ));
        assert!(inside(
            r"d:\jürgen's ordner\throwbacklauncher\BIN\Liberator.exe"
        ));
        assert!(inside(
            r"D:\Jürgen's Ordner\ThrowbackLauncher\downloads\Y1S0\RainbowSix.exe"
        ));
        assert!(!inside(
            r"D:\Jürgen's Ordner\ThrowbackLauncher2\ThrowbackLauncher.exe"
        ));
        assert!(!inside(
            r"C:\Users\Docker\AppData\Local\Temp\ThrowbackLauncher.uninstall.4.exe"
        ));
    }

    #[test]
    fn desktop_shortcut_prefers_the_registered_desktop_folder() {
        let home = Path::new("/home/test");
        let fallback = home.join("Desktop").join("Throwback Launcher.lnk");
        assert_eq!(desktop_shortcut(None, home), fallback);
        assert_eq!(desktop_shortcut(Some(String::new()), home), fallback);
        assert_eq!(
            desktop_shortcut(Some("/mnt/OneDrive/Desktop".to_string()), home),
            Path::new("/mnt/OneDrive/Desktop/Throwback Launcher.lnk")
        );
    }
}
