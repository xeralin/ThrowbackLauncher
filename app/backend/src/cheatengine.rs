use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::LazyLock;

use anyhow::{Result, anyhow, bail};
use regex::Regex;

use crate::env::{NO_PROTON, NOT_INSTALLED, Paths};
use crate::log::{exit_text, fail};
use crate::manifest::installed_path;
use crate::steam::{Proton, proton_env};
use crate::throwbackloader::{read_tl_autorun, write_tl_autorun};

const CE_EXE: &str = "Cheat Engine.exe";

static WINE_NOISE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[0-9a-f]+:)*(?:fixme|warn|trace):").expect("pattern"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheatEngineStatus {
    pub enabled: bool,
    pub present: bool,
}

fn find_ce_exe(paths: &Paths, key: &str) -> Option<String> {
    let drive_c = paths.prefix_dir().join(key).join("pfx").join("drive_c");
    let folders = ["Program Files", "Program Files (x86)"]
        .into_iter()
        .flat_map(|base| {
            let entries = fs::read_dir(drive_c.join(base)).into_iter().flatten();
            entries.flatten().map(move |entry| (base, entry))
        });
    let installed = folders.filter(|(_, entry)| {
        let name = entry.file_name();
        name.to_string_lossy().starts_with("Cheat Engine") && entry.path().join(CE_EXE).exists()
    });
    let newest = installed.max_by_key(|(_, entry)| {
        let folder = entry.metadata();
        folder.and_then(|folder| folder.modified()).ok()
    });
    newest.map(|(base, entry)| {
        let name = entry.file_name();
        format!(r"C:\{base}\{}\{CE_EXE}", name.to_string_lossy())
    })
}

fn game_folder(libraries: &[PathBuf], key: &str) -> Result<PathBuf> {
    installed_path(libraries, key, false).ok_or_else(|| anyhow!(NOT_INSTALLED))
}

fn is_ce(entry: &str) -> bool {
    entry.to_lowercase().contains("cheat engine")
}

fn set_autorun(folder: &Path, exe: Option<String>, message: &str) -> Result<()> {
    let mut autorun = read_tl_autorun(folder);
    autorun.retain(|entry| !is_ce(entry));
    autorun.extend(exe);
    write_tl_autorun(folder, &autorun).map_err(|error| anyhow!(fail(message, format!("{error:#}"))))
}

pub fn cheat_engine_status(paths: &Paths, libraries: &[PathBuf], key: &str) -> CheatEngineStatus {
    let folder = installed_path(libraries, key, false);
    CheatEngineStatus {
        enabled: folder
            .is_some_and(|folder| read_tl_autorun(&folder).iter().any(|entry| is_ce(entry))),
        present: find_ce_exe(paths, key).is_some(),
    }
}

pub fn remove_cheat_engine(libraries: &[PathBuf], key: &str) -> Result<()> {
    let folder = game_folder(libraries, key)?;
    set_autorun(&folder, None, "Cheat Engine removal failed")
}

pub fn add_cheat_engine(paths: &Paths, libraries: &[PathBuf], key: &str) -> Result<()> {
    let folder = game_folder(libraries, key)?;
    let Some(exe) = find_ce_exe(paths, key) else {
        bail!("Cheat Engine not found");
    };
    set_autorun(&folder, Some(exe), "Cheat Engine setup failed")
}

fn installer_detail(output: &Output) -> String {
    let exit = exit_text(output.status);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut lines = stderr.lines().map(str::trim);
    match lines.find(|line| !line.is_empty() && !WINE_NOISE.is_match(line)) {
        Some(line) => format!("installer {exit}: {line}"),
        None => format!("installer {exit}"),
    }
}

pub fn install_cheat_engine(
    paths: &Paths,
    libraries: &[PathBuf],
    key: &str,
    proton: Option<&Proton>,
    installer: &Path,
) -> Result<()> {
    let folder = game_folder(libraries, key)?;
    let Some(proton) = proton else {
        bail!(NO_PROTON);
    };
    let prefix = paths.prefix_dir().join(key);
    let unstarted = |error: io::Error, path: &Path| {
        let detail = format!("{error}: '{}'", path.display());
        anyhow!(fail("Cheat Engine installer failed to start", detail))
    };
    let env = proton_env(&prefix).map_err(|error| unstarted(error, &prefix))?;
    let mut command = Command::new(&proton.binary);
    command.arg("run").arg(installer).envs(env.vars());
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    let run = command.output();
    let output = run.map_err(|error| unstarted(error, &proton.binary))?;
    let Some(exe) = find_ce_exe(paths, key) else {
        let detail = installer_detail(&output);
        bail!(fail(
            "Cheat Engine was not found after the installer closed",
            detail
        ));
    };
    set_autorun(&folder, Some(exe), "Cheat Engine setup failed")
}

#[cfg(all(test, unix))]
mod tests {
    use std::time::{Duration, SystemTime};

    use tempfile::TempDir;

    use super::*;
    use crate::env::{TL_LAUNCHER, TL_TOML};
    use crate::log::take_recorded;

    const KEY: &str = "Y5S3_ShadowLegacy";
    const CONFIG: &str =
        "[Launch]\nargs = '/belaunch'\nautorun = []\n\n[User]\nusername = 'Tester'\n";
    const CE_75: &str = r"C:\Program Files\Cheat Engine 7.5\Cheat Engine.exe";

    struct Rig {
        _dir: TempDir,
        paths: Paths,
        libraries: Vec<PathBuf>,
    }

    impl Rig {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
            let libraries = vec![dir.path().join("lib")];
            Self {
                _dir: dir,
                paths,
                libraries,
            }
        }

        fn install(&self, folder: &str, config: &str) -> PathBuf {
            let folder = self.libraries[0].join(folder);
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join(TL_LAUNCHER), b"").unwrap();
            fs::write(folder.join(TL_TOML), config).unwrap();
            folder
        }

        fn cheat_engine(&self, base: &str, name: &str, age: u64) -> PathBuf {
            let prefix = self.paths.prefix_dir().join(KEY);
            let folder = prefix.join("pfx").join("drive_c").join(base).join(name);
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join(CE_EXE), b"").unwrap();
            let stamp = SystemTime::now() - Duration::from_secs(age);
            let handle = fs::File::open(&folder).unwrap();
            handle.set_modified(stamp).unwrap();
            folder
        }

        fn status(&self) -> (bool, bool) {
            let status = cheat_engine_status(&self.paths, &self.libraries, KEY);
            (status.enabled, status.present)
        }
    }

    fn message(result: Result<()>) -> String {
        result.unwrap_err().to_string()
    }

    #[test]
    fn the_newest_cheat_engine_folder_is_found_as_a_windows_path() {
        let rig = Rig::new();
        assert_eq!(find_ce_exe(&rig.paths, KEY), None);
        let lower = rig.cheat_engine("Program Files", "cheat engine 7.5", 0);
        let empty = rig.cheat_engine("Program Files", "Cheat Engine 7.6", 0);
        fs::remove_file(empty.join(CE_EXE)).unwrap();
        fs::write(lower.parent().unwrap().join(CE_EXE), b"").unwrap();
        assert_eq!(find_ce_exe(&rig.paths, KEY), None);
        rig.cheat_engine("Program Files", "Cheat Engine 7.4", 500);
        rig.cheat_engine("Program Files (x86)", "Cheat Engine", 100);
        rig.cheat_engine("Program Files", "Cheat Engine 7.5", 300);
        assert_eq!(
            find_ce_exe(&rig.paths, KEY).unwrap(),
            r"C:\Program Files (x86)\Cheat Engine\Cheat Engine.exe"
        );
        assert_eq!(find_ce_exe(&rig.paths, "Y1S0_Vanilla"), None);
    }

    #[test]
    fn the_status_tells_the_autorun_entry_from_the_installation() {
        let rig = Rig::new();
        assert_eq!(rig.status(), (false, false));
        rig.cheat_engine("Program Files", "Cheat Engine 7.5", 0);
        assert_eq!(rig.status(), (false, true));
        rig.install(
            KEY,
            "[Launch]\nautorun = ['Z:\\tools\\x.exe', 'C:\\Program Files\\CHEAT ENGINE 7.5\\Cheat Engine.exe']\n",
        );
        assert_eq!(rig.status(), (true, true));
    }

    #[test]
    fn adding_replaces_old_entries_and_keeps_other_tools() {
        let rig = Rig::new();
        let folder = rig.install(
            KEY,
            "[Launch]\nargs = '/belaunch'\nautorun = ['Z:\\tools\\x.exe', 'C:\\old\\Cheat Engine 6\\cheat engine.exe']\n",
        );
        rig.cheat_engine("Program Files", "Cheat Engine 7.5", 0);
        add_cheat_engine(&rig.paths, &rig.libraries, KEY).unwrap();
        assert_eq!(read_tl_autorun(&folder), [r"Z:\tools\x.exe", CE_75]);
        remove_cheat_engine(&rig.libraries, KEY).unwrap();
        remove_cheat_engine(&rig.libraries, KEY).unwrap();
        assert_eq!(read_tl_autorun(&folder), [r"Z:\tools\x.exe"]);
        assert!(take_recorded().is_empty());
    }

    #[test]
    fn adding_and_removing_report_what_is_missing() {
        let rig = Rig::new();
        rig.install("Y5S3_HeatedMetal", CONFIG);
        rig.cheat_engine("Program Files", "Cheat Engine 7.5", 0);
        let add = || message(add_cheat_engine(&rig.paths, &rig.libraries, KEY));
        let remove = || message(remove_cheat_engine(&rig.libraries, KEY));
        assert_eq!(
            (add(), remove()),
            ("Not installed".into(), "Not installed".into())
        );
        let folder = rig.install(KEY, CONFIG);
        fs::remove_dir_all(rig.paths.prefix_dir()).unwrap();
        assert_eq!(add(), "Cheat Engine not found");
        assert!(take_recorded().is_empty());
        rig.cheat_engine("Program Files", "Cheat Engine 7.5", 0);
        fs::write(folder.join(TL_TOML), "[User]\nusername = 'Tester'\n").unwrap();
        assert_eq!(add(), "Cheat Engine setup failed");
        assert_eq!(remove(), "Cheat Engine removal failed");
        assert_eq!(take_recorded().len(), 2);
    }

    mod installer {
        use std::os::unix::fs::PermissionsExt;

        use super::*;

        const SETUP: &str = "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$STEAM_COMPAT_DATA_PATH/arguments\"\nenv > \"$STEAM_COMPAT_DATA_PATH/environment\"\nfolder=\"$STEAM_COMPAT_DATA_PATH/pfx/drive_c/Program Files/Cheat Engine 7.5\"\nmkdir -p \"$folder\"\n: > \"$folder/Cheat Engine.exe\"\necho visible\nexit 4\n";
        const ABORT: &str = "#!/bin/sh\necho '0024:fixme:ntdll:stub' >&2\necho '0118:012c:warn:seh:x' >&2\necho '  setup aborted  ' >&2\necho 'second line' >&2\nexit 3\n";

        fn proton(rig: &Rig, script: &str, mode: u32) -> Proton {
            let binary = rig.paths.proton_dir().join("Probe").join("proton");
            fs::create_dir_all(binary.parent().unwrap()).unwrap();
            fs::write(&binary, script).unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(mode)).unwrap();
            Proton {
                display: "Probe".to_owned(),
                internal: "Probe".to_owned(),
                binary,
            }
        }

        fn install(rig: &Rig, proton: Option<&Proton>) -> Result<()> {
            let installer = rig.paths.data_root.join("Cheat Engine 75.exe");
            install_cheat_engine(&rig.paths, &rig.libraries, KEY, proton, &installer)
        }

        #[test]
        fn the_installer_runs_through_proton_in_the_season_prefix() {
            let rig = Rig::new();
            let proton = proton(&rig, SETUP, 0o755);
            assert_eq!(message(install(&rig, None)), "Not installed");
            let folder = rig.install(KEY, CONFIG);
            assert_eq!(message(install(&rig, None)), "No Proton found");
            install(&rig, Some(&proton)).unwrap();
            let prefix = rig.paths.prefix_dir().join(KEY);
            let installer = rig.paths.data_root.join("Cheat Engine 75.exe");
            assert_eq!(
                fs::read_to_string(prefix.join("arguments")).unwrap(),
                format!("run\n{}\n", installer.display())
            );
            let seen = fs::read_to_string(prefix.join("environment")).unwrap();
            let has = |start: &str| seen.lines().any(|line| line.starts_with(start));
            assert!(has("STEAM_COMPAT_CLIENT_INSTALL_PATH=") && has("PATH="));
            assert_eq!(read_tl_autorun(&folder), [CE_75]);
            assert!(take_recorded().is_empty());
        }

        #[test]
        fn a_closed_installer_without_cheat_engine_is_logged() {
            let rig = Rig::new();
            let folder = rig.install(KEY, CONFIG);
            assert_eq!(
                message(install(&rig, Some(&proton(&rig, ABORT, 0o755)))),
                "Cheat Engine was not found after the installer closed"
            );
            assert_eq!(
                take_recorded(),
                [
                    "Cheat Engine was not found after the installer closed - installer exit=3: setup aborted"
                ]
            );
            let locked = proton(&rig, ABORT, 0o644);
            assert_eq!(
                message(install(&rig, Some(&locked))),
                "Cheat Engine installer failed to start"
            );
            let logged = take_recorded();
            assert_eq!(logged.len(), 1, "{logged:?}");
            assert!(
                logged[0].contains(locked.binary.to_str().unwrap()),
                "{logged:?}"
            );
            assert!(read_tl_autorun(&folder).is_empty());
        }
    }
}
