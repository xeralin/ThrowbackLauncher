use std::path::Path;
#[cfg(any(windows, test))]
use std::path::PathBuf;
use std::time::Instant;
use std::{fs, thread};

use crate::env::{APP_SUBDIR, EXE_NAME, PENDING_SUBDIR, PREVIOUS_SUBDIR, Paths};
use crate::self_update::{POLL_INTERVAL, clear};

#[cfg(any(windows, test))]
const PENDING_FILE: &str = ".pending";
#[cfg(windows)]
const PINNED: &str = r"Microsoft\Internet Explorer\Quick Launch\User Pinned";

#[cfg(any(windows, test))]
pub fn installation(program: &Path, root: &Path, registered: bool) -> Option<PathBuf> {
    use crate::env::same_path;

    let folder = program.parent()?;
    let home = folder.parent()?;
    let staged = folder.file_name()? == PENDING_SUBDIR && folder.join(PENDING_FILE).is_file();
    let used = || {
        let paths = Paths::new(home.to_path_buf(), home.to_path_buf());
        paths.settings_file().is_file()
    };
    let known = same_path(home, root) || (!registered && used());
    (staged && known).then(|| home.to_path_buf())
}

#[cfg(windows)]
pub fn move_in(program: &Path, root: &Path, args: &[String]) {
    use crate::env::RELAUNCH_ARG;
    use crate::self_install::{install, outdated, register, report, write_shortcut};
    use crate::self_update::write_outcome;
    use crate::winspawn::spawn_detached;

    let paths = Paths::new(root.to_path_buf(), root.to_path_buf());
    let old = root.join(APP_SUBDIR).join(EXE_NAME);
    let relaunch = args.iter().position(|arg| arg == RELAUNCH_ARG);
    let pid = relaunch.and_then(|index| args.get(index + 1)?.parse().ok());
    if pid.is_some_and(|pid| !system::wait_pid(pid)) {
        let _ = spawn_detached(&[&old]);
        return;
    }
    if !system::claim(&format!(r"Local\{}.update", paths.instance_key())) {
        return;
    }
    if let Some(folder) = program.parent() {
        let _ = crate::liberator::adopt(&paths, folder);
    }
    let target = root.join(EXE_NAME);
    let settle = || -> anyhow::Result<()> {
        if outdated() || !target.is_file() {
            install(program, root)?;
        }
        repoint_shortcuts(root);
        write_shortcut(root);
        register(root);
        spawn_detached(&[&target])
    };
    write_outcome(&paths, true);
    if let Err(error) = settle() {
        report(root, &error);
        write_outcome(&paths, false);
        let _ = fs::remove_file(root.join(PENDING_SUBDIR).join(PENDING_FILE));
        let _ = spawn_detached(&[&old]);
    }
}

#[cfg(any(windows, test))]
fn shortcuts_in(folder: &Path, nested: bool) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in fs::read_dir(folder).into_iter().flatten().flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            if nested {
                found.extend(shortcuts_in(&path, true));
            }
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
        {
            found.push(path);
        }
    }
    found
}

#[cfg(any(windows, test))]
fn stale(root: &Path, target: &Path) -> bool {
    let old = format!(r"{}\{APP_SUBDIR}", root.display()).to_lowercase();
    let target = target.to_string_lossy().to_lowercase();
    [r"\", "."]
        .iter()
        .any(|next| target.starts_with(&format!("{old}{next}")))
}

#[cfg(windows)]
pub fn repoint_shortcuts(root: &Path) {
    let old = [APP_SUBDIR, PENDING_SUBDIR].map(|name| root.join(name));
    if !old.iter().any(|folder| folder.is_dir()) {
        return;
    }
    let desktop = crate::self_uninstall::desktop_shortcut();
    let mut files = desktop
        .parent()
        .map(|folder| shortcuts_in(folder, false))
        .unwrap_or_default();
    if let Some(appdata) = std::env::var_os("APPDATA").filter(|value| !value.is_empty()) {
        files.extend(shortcuts_in(&Path::new(&appdata).join(PINNED), true));
    }
    let program = root.join(EXE_NAME);
    crate::shortcut::repoint(&files, |target| stale(root, target), &program, root);
}

pub fn remove_program_folders(paths: &Paths, deadline: Instant) {
    let [app, pending, previous] =
        [APP_SUBDIR, PENDING_SUBDIR, PREVIOUS_SUBDIR].map(|name| paths.data_root.join(name));
    let free = clear(&previous, deadline);
    if free && app.join(EXE_NAME).is_file() && rename_until(&app, &previous, deadline) {
        clear(&previous, deadline);
    }
    clear(&pending, deadline);
}

fn rename_until(from: &Path, to: &Path, deadline: Instant) -> bool {
    loop {
        if fs::rename(from, to).is_ok() {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(windows)]
mod system {
    use std::ptr;

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Threading::{
        CreateMutexW, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    const WAIT_PID_MS: u32 = 30_000;

    pub(super) fn claim(name: &str) -> bool {
        let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let handle = unsafe { CreateMutexW(ptr::null(), 1, name.as_ptr()) };
        !handle.is_null() && unsafe { GetLastError() } != ERROR_ALREADY_EXISTS
    }

    pub(super) fn wait_pid(pid: u32) -> bool {
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            return true;
        }
        let result = unsafe { WaitForSingleObject(handle, WAIT_PID_MS) };
        unsafe { CloseHandle(handle) };
        result == WAIT_OBJECT_0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::throwbackloader::testing::names;

    #[test]
    fn only_the_update_folder_of_an_installation_counts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ThrowbackLauncher");
        let stage = |home: &Path, folder: &str| {
            fs::create_dir_all(home.join(folder)).unwrap();
            fs::write(home.join(folder).join(PENDING_FILE), "9.9.9").unwrap();
            home.join(folder).join(EXE_NAME)
        };
        let staged = stage(&root, PENDING_SUBDIR);
        assert_eq!(installation(&staged, &root, true), Some(root.clone()));
        assert_eq!(installation(&staged, &root, false), Some(root.clone()));
        for folder in [APP_SUBDIR, PREVIOUS_SUBDIR, "App.Pending", "Downloads"] {
            let program = stage(&root, folder);
            assert_eq!(installation(&program, &root, false), None, "{folder}");
        }
        assert_eq!(installation(&root.join(EXE_NAME), &root, false), None);
        assert_eq!(installation(Path::new(EXE_NAME), &root, false), None);
        fs::remove_file(root.join(PENDING_SUBDIR).join(PENDING_FILE)).unwrap();
        assert_eq!(installation(&staged, &root, true), None);

        let moved = dir.path().join("Tools");
        let staged = stage(&moved, PENDING_SUBDIR);
        assert_eq!(installation(&staged, &root, false), None);
        fs::write(moved.join("settings.toml"), "").unwrap();
        assert_eq!(installation(&staged, &root, false), Some(moved.clone()));
        assert_eq!(installation(&staged, &root, true), None);
    }

    #[test]
    fn the_program_folders_of_the_old_layout_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let paths = Paths::new(root.clone(), root.clone());
        for folder in [APP_SUBDIR, PENDING_SUBDIR, PREVIOUS_SUBDIR] {
            fs::create_dir_all(root.join(folder).join("lib")).unwrap();
            fs::write(root.join(folder).join(EXE_NAME), folder).unwrap();
            fs::write(root.join(folder).join("lib").join("python.dll"), "").unwrap();
        }
        fs::write(root.join(EXE_NAME), "program").unwrap();
        fs::create_dir(root.join("downloads")).unwrap();
        remove_program_folders(&paths, Instant::now());
        assert_eq!(names(&root), [EXE_NAME, "downloads"]);
    }

    #[test]
    fn a_folder_named_app_without_the_program_stays() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let paths = Paths::new(root.clone(), root.clone());
        fs::create_dir(root.join(APP_SUBDIR)).unwrap();
        fs::write(root.join(APP_SUBDIR).join("notes.txt"), "").unwrap();
        remove_program_folders(&paths, Instant::now());
        assert_eq!(names(&root), [APP_SUBDIR]);
        assert_eq!(names(&root.join(APP_SUBDIR)), ["notes.txt"]);
    }

    #[test]
    fn shortcuts_are_found_in_the_desktop_and_the_pinned_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("TaskBar").join("ImplicitAppShortcuts")).unwrap();
        for file in ["Throwback Launcher.lnk", "Other.LNK", "notes.txt"] {
            fs::write(root.join(file), "").unwrap();
        }
        fs::write(root.join("TaskBar").join("Pinned.lnk"), "").unwrap();
        fs::write(
            root.join("TaskBar")
                .join("ImplicitAppShortcuts")
                .join("Deep.lnk"),
            "",
        )
        .unwrap();
        let names = |nested| {
            let mut names: Vec<String> = shortcuts_in(root, nested)
                .iter()
                .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };
        assert_eq!(names(false), ["Other.LNK", "Throwback Launcher.lnk"]);
        assert_eq!(
            names(true),
            [
                "Deep.lnk",
                "Other.LNK",
                "Pinned.lnk",
                "Throwback Launcher.lnk"
            ]
        );
        assert!(shortcuts_in(&root.join("missing"), true).is_empty());
    }

    #[test]
    fn only_shortcuts_into_the_old_program_folders_are_stale() {
        let root = Path::new(r"D:\Jürgen's Ordner\ThrowbackLauncher");
        let stale = |target: &str| stale(root, Path::new(target));
        assert!(stale(
            r"D:\Jürgen's Ordner\ThrowbackLauncher\app\ThrowbackLauncher.exe"
        ));
        assert!(stale(
            r"d:\jürgen's ordner\throwbacklauncher\APP\ThrowbackLauncher.exe"
        ));
        assert!(stale(
            r"D:\Jürgen's Ordner\ThrowbackLauncher\app.pending\ThrowbackLauncher.exe"
        ));
        assert!(!stale(
            r"D:\Jürgen's Ordner\ThrowbackLauncher\ThrowbackLauncher.exe"
        ));
        assert!(!stale(
            r"D:\Jürgen's Ordner\ThrowbackLauncher\application\Tool.exe"
        ));
        assert!(!stale(r"D:\Jürgen's Ordner\ThrowbackLauncher\app"));
        assert!(!stale(r"C:\Windows\notepad.exe"));
    }
}
