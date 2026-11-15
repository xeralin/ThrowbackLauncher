#[cfg(any(windows, test))]
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;

#[cfg(any(windows, test))]
use crate::env::{EXE_NAME, VERSION, parse_version};

#[cfg(windows)]
pub fn hand_over(args: &[String]) -> Result<bool, String> {
    use crate::env::{DIR_NAME, PACKAGED, resolve, same_path, user_data_base};
    use crate::migrate;

    if !PACKAGED {
        return Ok(false);
    }
    let program = std::env::current_exe().map_or_else(|_| PathBuf::new(), |exe| resolve(&exe));
    let registered = registered_root();
    let listed = registered.is_some();
    let root = registered.unwrap_or_else(|| user_data_base().join(DIR_NAME));
    if let Some(root) = migrate::installation(&program, &root, listed) {
        migrate::move_in(&program, &root, args);
        return Ok(true);
    }
    if program
        .parent()
        .is_some_and(|folder| same_path(folder, &root))
    {
        return Ok(false);
    }
    set_up(&program, &root)
        .map(|()| true)
        .map_err(|error| report(&root, &error))
}

#[cfg(windows)]
pub(crate) fn report(root: &Path, error: &anyhow::Error) -> String {
    use crate::env::Paths;

    crate::log::init(Paths::new(root.to_path_buf(), root.to_path_buf()).errors_file());
    crate::log::fail("Throwback Launcher setup failed", format!("{error:#}"))
}

#[cfg(windows)]
fn set_up(program: &Path, root: &Path) -> anyhow::Result<()> {
    use crate::winspawn::spawn_detached;

    let target = root.join(EXE_NAME);
    if outdated() || spawn_detached(&[&target]).is_err() {
        install(program, root)?;
        crate::migrate::repoint_shortcuts(root);
        write_shortcut(root);
        register(root);
        spawn_detached(&[&target])?;
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn outdated() -> bool {
    registered_version().is_some_and(|version| older(&version))
}

#[cfg(any(windows, test))]
fn older(installed: &str) -> bool {
    parse_version(installed).is_some_and(|installed| Some(installed) < parse_version(VERSION))
}

#[cfg(any(windows, test))]
pub(crate) fn install(program: &Path, root: &Path) -> std::io::Result<()> {
    use std::fs::{self, File};

    use crate::self_update::{replace_program, staging_file};

    let target = root.join(EXE_NAME);
    let staged = staging_file(&target);
    let place = || -> std::io::Result<()> {
        fs::create_dir_all(root)?;
        std::io::copy(&mut File::open(program)?, &mut File::create(&staged)?)?;
        replace_program(&target, &staged)
    };
    place().inspect_err(|_| {
        let _ = fs::remove_file(&staged);
    })
}

#[cfg(windows)]
pub(crate) fn write_shortcut(root: &Path) {
    let Some(shortcut) = start_menu_shortcut() else {
        return;
    };
    let quoted = |path: &Path| path.to_string_lossy().replace('\'', "''");
    let target = root.join(EXE_NAME);
    let script = format!(
        "$s=(New-Object -COM WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';\
         $s.WorkingDirectory='{}';$s.IconLocation='{}';$s.Save()",
        quoted(&shortcut),
        quoted(&target),
        quoted(root),
        quoted(&target),
    );
    crate::winspawn::powershell(&script);
}

#[cfg(windows)]
pub fn start_menu_shortcut() -> Option<PathBuf> {
    use crate::env::APP_NAME;

    let appdata = std::env::var_os("APPDATA").filter(|value| !value.is_empty())?;
    Some(
        Path::new(&appdata)
            .join(r"Microsoft\Windows\Start Menu\Programs")
            .join(format!("{APP_NAME}.lnk")),
    )
}

#[cfg(windows)]
fn registered_root() -> Option<PathBuf> {
    use crate::env::{UNINSTALL_KEY, resolve};

    let key = winreg::HKCU.open_subkey(UNINSTALL_KEY).ok()?;
    let location: String = key.get_value("InstallLocation").ok()?;
    let root = resolve(Path::new(&location));
    (!location.is_empty() && root.is_dir()).then_some(root)
}

#[cfg(windows)]
fn registered_version() -> Option<String> {
    let key = winreg::HKCU.open_subkey(crate::env::UNINSTALL_KEY).ok()?;
    key.get_value("DisplayVersion").ok()
}

#[cfg(windows)]
pub fn owns_entry(root: &Path) -> bool {
    registered_root().is_none_or(|registered| crate::env::same_path(&registered, root))
}

#[cfg(windows)]
pub fn register(root: &Path) {
    use crate::env::{APP_NAME, PACKAGED, UNINSTALL_ARG, UNINSTALL_KEY};

    if !PACKAGED || !owns_entry(root) {
        return;
    }
    let Ok((key, _)) = winreg::HKCU.create_subkey(UNINSTALL_KEY) else {
        return;
    };
    let file = root.join(EXE_NAME);
    let program = file.display().to_string();
    let stored = |name: &str| key.get_value::<String, _>(name).ok();
    if stored("DisplayVersion").as_deref() == Some(VERSION)
        && stored("DisplayIcon").as_deref() == Some(program.as_str())
    {
        return;
    }
    let size = std::fs::metadata(&file).map_or(0, |file| file.len() / 1024);
    let numbers = [
        ("NoModify", 1),
        ("NoRepair", 1),
        ("EstimatedSize", u32::try_from(size).unwrap_or(u32::MAX)),
    ];
    let texts = [
        ("DisplayName", APP_NAME.to_owned()),
        ("DisplayIcon", program.clone()),
        ("InstallLocation", root.display().to_string()),
        ("UninstallString", format!("\"{program}\" {UNINSTALL_ARG}")),
        ("DisplayVersion", VERSION.to_owned()),
    ];
    for (name, value) in numbers {
        let _ = key.set_value(name, &value);
    }
    for (name, value) in texts {
        let _ = key.set_value(name, &value);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn only_an_older_installation_is_replaced() {
        assert!(older("0.2.2"));
        assert!(older("0.0.9"));
        assert!(!older(VERSION));
        assert!(!older("99.0.0"));
        assert!(!older("unknown"));
    }

    fn names(folder: &Path) -> Vec<String> {
        let entries = fs::read_dir(folder).unwrap();
        let mut names: Vec<String> = entries
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_program_is_placed_into_a_new_broken_or_older_installation() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("Downloads").join(EXE_NAME);
        fs::create_dir(dir.path().join("Downloads")).unwrap();
        fs::write(&program, "program").unwrap();
        let root = dir.path().join("root");
        let target = root.join(EXE_NAME);
        install(&program, &root).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "program");

        fs::write(&target, "broken").unwrap();
        fs::write(root.join("settings.toml"), "kept").unwrap();
        install(&program, &root).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "program");
        #[cfg(windows)]
        assert_eq!(
            names(&root),
            [EXE_NAME, "ThrowbackLauncher.old", "settings.toml"]
        );
        #[cfg(not(windows))]
        assert_eq!(names(&root), [EXE_NAME, "settings.toml"]);
        assert!(program.is_file());
    }

    #[test]
    fn the_installation_stays_when_the_program_cannot_be_copied() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        fs::create_dir(&root).unwrap();
        fs::write(root.join(EXE_NAME), "old").unwrap();
        assert!(install(&dir.path().join("gone.exe"), &root).is_err());
        assert_eq!(names(&root), [EXE_NAME]);
        assert_eq!(fs::read_to_string(root.join(EXE_NAME)).unwrap(), "old");
    }
}
