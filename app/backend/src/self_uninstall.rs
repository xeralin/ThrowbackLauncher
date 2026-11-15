#[cfg(windows)]
pub fn run(paths: &crate::env::Paths) {
    use std::ffi::OsStr;

    use crate::env::{DIR_NAME, PACKAGED, UNINSTALL_KEY, unlink};
    use crate::self_install::{owns_entry, start_menu_shortcut};
    use crate::winspawn::spawn_detached;

    if !PACKAGED {
        return;
    }
    if owns_entry(&paths.data_root) {
        if let Some(shortcut) = start_menu_shortcut() {
            let _ = unlink(&shortcut);
        }
        let _ = unlink(&desktop_shortcut());
        let _ = winreg::HKCU.delete_subkey(UNINSTALL_KEY);
    }

    let _ = crate::depot::wipe_depot_token();
    let bat = std::env::temp_dir().join(format!("{DIR_NAME}.uninstall.bat"));
    let [cmd, quiet, run] = ["cmd", "/d", "/c"].map(OsStr::new);
    let started = std::fs::write(&bat, plan::batch(paths))
        .map_err(anyhow::Error::from)
        .and_then(|()| spawn_detached(&[cmd, quiet, run, bat.as_os_str()]));
    if let Err(error) = started {
        crate::log::fail("Uninstall helper failed to start", format!("{error:#}"));
    }
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

    use crate::env::{APP_NAME, EXE_NAME, Paths};

    pub(super) fn desktop_shortcut(folder: Option<String>, home: &Path) -> PathBuf {
        folder
            .filter(|folder| !folder.is_empty())
            .map_or_else(|| home.join("Desktop"), PathBuf::from)
            .join(format!("{APP_NAME}.lnk"))
    }

    pub(super) fn batch(paths: &Paths) -> String {
        let name = |path: PathBuf| {
            let last = path.file_name().unwrap_or_default();
            last.to_string_lossy().into_owned()
        };
        let keep = name(paths.default_downloads_dir());
        let bin = name(paths.bin_dir());
        let dir = paths.data_root.to_string_lossy().replace('%', "%%");
        let prefix = dir.replace('\'', "''");
        let exe = format!(r"{dir}\{EXE_NAME}");
        format!(
            r#"@echo off
chcp 65001>nul
ping 127.0.0.1 -n 3 >nul
powershell -NoProfile -NonInteractive -Command "Get-Process | Where-Object {{ $_.Path -and $_.Path.StartsWith('{prefix}\', [System.StringComparison]::OrdinalIgnoreCase) }} | Stop-Process -Force" >nul 2>&1
set n=0
:retry
del /f /q "{exe}" >nul 2>&1
if exist "{exe}" goto wait
for /d %%d in ("{dir}\*") do if /i not "%%~nxd"=="{keep}" rmdir /s /q "%%~fd" >nul 2>&1
del /f /q /a "{dir}\*" >nul 2>&1
rmdir "{dir}\{keep}" >nul 2>&1
rmdir "{dir}" >nul 2>&1
if not exist "{dir}\{bin}" goto done
:wait
set /a n+=1
if %n% lss 10 (ping 127.0.0.1 -n 2 >nul & goto retry)
:done
del "%~f0"
"#
        )
        .replace('\n', "\r\n")
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::plan::{batch, desktop_shortcut};
    use crate::env::Paths;

    const SPECIAL: &str = r#"@echo off
chcp 65001>nul
ping 127.0.0.1 -n 3 >nul
powershell -NoProfile -NonInteractive -Command "Get-Process | Where-Object { $_.Path -and $_.Path.StartsWith('D:\Spiele 100%% & mehr\Jürgen''s ^Ordner!\ThrowbackLauncher\', [System.StringComparison]::OrdinalIgnoreCase) } | Stop-Process -Force" >nul 2>&1
set n=0
:retry
del /f /q "D:\Spiele 100%% & mehr\Jürgen's ^Ordner!\ThrowbackLauncher\ThrowbackLauncher.exe" >nul 2>&1
if exist "D:\Spiele 100%% & mehr\Jürgen's ^Ordner!\ThrowbackLauncher\ThrowbackLauncher.exe" goto wait
for /d %%d in ("D:\Spiele 100%% & mehr\Jürgen's ^Ordner!\ThrowbackLauncher\*") do if /i not "%%~nxd"=="downloads" rmdir /s /q "%%~fd" >nul 2>&1
del /f /q /a "D:\Spiele 100%% & mehr\Jürgen's ^Ordner!\ThrowbackLauncher\*" >nul 2>&1
rmdir "D:\Spiele 100%% & mehr\Jürgen's ^Ordner!\ThrowbackLauncher\downloads" >nul 2>&1
rmdir "D:\Spiele 100%% & mehr\Jürgen's ^Ordner!\ThrowbackLauncher" >nul 2>&1
if not exist "D:\Spiele 100%% & mehr\Jürgen's ^Ordner!\ThrowbackLauncher\bin" goto done
:wait
set /a n+=1
if %n% lss 10 (ping 127.0.0.1 -n 2 >nul & goto retry)
:done
del "%~f0"
"#;

    fn script(install_dir: &str) -> String {
        let root = PathBuf::from(install_dir);
        batch(&Paths::new(root.clone(), root))
    }

    #[test]
    fn batch_removes_the_installation_and_quotes_the_path() {
        let text = script(r"D:\Spiele 100% & mehr\Jürgen's ^Ordner!\ThrowbackLauncher");
        assert_eq!(text.replace("\r\n", "\n"), SPECIAL);
        assert_eq!(text.matches("\r\n").count(), text.matches('\n').count());
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
