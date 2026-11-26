use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use blake2::Blake2sVar;
use blake2::digest::{Update, VariableOutput};

pub const VERSION: &str = include_str!("../../VERSION").trim_ascii();

pub const APP_NAME: &str = "Throwback Launcher";
pub const DIR_NAME: &str = "ThrowbackLauncher";
pub const EXE_NAME: &str = "ThrowbackLauncher.exe";
pub const LIBERATOR_NAME: &str = "Liberator.exe";
pub const APP_SUBDIR: &str = "app";
pub const PENDING_SUBDIR: &str = "app.pending";
pub const PREVIOUS_SUBDIR: &str = "app.previous";
pub const UNINSTALL_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\ThrowbackLauncher";
pub const RELAUNCH_ARG: &str = "--relaunch";
pub const UNINSTALL_ARG: &str = "--uninstall";
pub const REMOVE_ARG: &str = "--remove-installation";

pub const IS_WINDOWS: bool = cfg!(windows);
pub const PACKAGED: bool = cfg!(feature = "packaged");
pub const DD_MEMBER: &str = if IS_WINDOWS {
    "DepotDownloader.exe"
} else {
    "DepotDownloader"
};
pub const DD_API_URL: &str = "https://api.github.com/repos/SteamRE/DepotDownloader/releases/latest";
pub const DD_ASSET: &str = if IS_WINDOWS {
    "DepotDownloader-windows-x64.zip"
} else if cfg!(target_arch = "aarch64") {
    "DepotDownloader-linux-arm64.zip"
} else {
    "DepotDownloader-linux-x64.zip"
};
pub const TL_API_URL: &str = "https://api.github.com/repos/xeralin/ThrowbackLoader/releases/latest";
pub const TL_ASSET: &str = ".zip";
pub const SEVENZ_API_URL: &str = "https://api.github.com/repos/ip7z/7zip/releases/latest";
pub const SEVENZ_ASSET: &str = if IS_WINDOWS {
    "7zr.exe"
} else if cfg!(target_arch = "aarch64") {
    "linux-arm64.tar.xz"
} else {
    "linux-x64.tar.xz"
};
pub const HM_API_URL: &str =
    "https://api.github.com/repos/DataCluster0/HeatedMetal/releases/latest";
pub const HM_ASSET: &str = ".7z";

pub const DEFAULTARGS_DLL: &str = "defaultargs.dll";
pub const TL_DLLS_COMMON: [&str; 2] = [DEFAULTARGS_DLL, "steam_api64.dll"];
pub const TL_LOADERS: [&str; 4] = [
    "uplay_r1_loader64.dll",
    "upc_r1_loader64.dll",
    "upc_r2_loader64.dll",
    "uplay_r2_loader64.dll",
];
pub const TL_TOML: &str = "Config.toml";
pub const TL_LAUNCHER: &str = "LaunchR6.exe";
pub const HM_FOLDER_SUFFIX: &str = "_HeatedMetal";
pub const SEVENZ_MEMBER: &str = if IS_WINDOWS { "7zr.exe" } else { "7zz" };

pub const DOWNLOADS_MIN: u32 = 1;
pub const DOWNLOADS_MAX: u32 = 100;
pub const SCALE_LEVELS: [u32; 6] = [75, 80, 90, 100, 110, 125];
pub const GIB: u64 = 1 << 30;

pub const BUSY_MESSAGE: &str = "A download is running for this season";
pub const DOWNLOAD_RUNNING: &str = "A download is running";
pub const UPDATE_RUNNING: &str = "An update is running";
pub const UNINSTALL_RUNNING: &str = "An uninstall is running";
pub const CACHE_CLEARING: &str = "The cache is being cleared";
pub const NOT_INSTALLED: &str = "Not installed";
pub const NOT_INSTALLED_HM: &str = "Not installed as Heated Metal";
pub const FOLDER_NOT_FOUND: &str = "Library folder not found";
pub const NO_PROTON: &str = "No Proton found";
pub const GAME_RUNNING: &str = "Stop R6S first";
pub const SEASON_RUNNING: &str = "Stop this season first";

pub const MAX_USERNAME_LENGTH: usize = 16;

pub fn parse_version(version: &str) -> Option<Vec<u64>> {
    version.split('.').map(|part| part.parse().ok()).collect()
}

pub fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_default()
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn user_data_base() -> PathBuf {
    if IS_WINDOWS {
        env_path("LOCALAPPDATA").unwrap_or_else(|| home().join("AppData").join("Local"))
    } else {
        env_path("XDG_DATA_HOME").unwrap_or_else(|| home().join(".local").join("share"))
    }
}

#[cfg(unix)]
const LINK_HOPS: usize = 40;

#[cfg(unix)]
pub fn resolve(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    follow(PathBuf::new(), &absolute, &mut Vec::new())
}

#[cfg(unix)]
fn follow(mut real: PathBuf, rest: &Path, open: &mut Vec<PathBuf>) -> PathBuf {
    for component in rest.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                real.pop();
            }
            Component::Normal(name) => {
                let next = real.join(name);
                real = match std::fs::read_link(&next) {
                    Ok(target) if open.len() < LINK_HOPS && !open.contains(&next) => {
                        open.push(next);
                        let target = follow(real, &target, open);
                        open.pop();
                        target
                    }
                    _ => next,
                };
            }
            anchor => real = PathBuf::from(anchor.as_os_str()),
        }
    }
    real
}

#[cfg(windows)]
pub fn resolve(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut normal = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                normal.pop();
            }
            Component::CurDir => {}
            other => normal.push(other),
        }
    }
    let mut missing = Vec::new();
    let mut existing = normal.as_path();
    let resolved = loop {
        if let Ok(resolved) = std::fs::canonicalize(existing) {
            break resolved;
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name.to_os_string());
                existing = parent;
            }
            _ => break existing.to_path_buf(),
        }
    };
    let mut result = plain(resolved);
    result.extend(missing.iter().rev());
    result
}

#[cfg(windows)]
pub fn unlink(path: &Path) -> io::Result<()> {
    use std::os::windows::fs::FileTypeExt;

    let meta = fs::symlink_metadata(path)?;
    if meta.permissions().readonly() {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    if meta.file_type().is_symlink_dir() {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(not(windows))]
pub fn unlink(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

pub(crate) fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_added_extension("tmp");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, path)
}

pub(crate) fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

pub(crate) fn copy_with_times(from: &Path, to: &Path) -> io::Result<()> {
    fs::copy(from, to)?;
    #[cfg(unix)]
    {
        let metadata = fs::metadata(from)?;
        let times = fs::FileTimes::new()
            .set_accessed(metadata.accessed()?)
            .set_modified(metadata.modified()?);
        fs::File::open(to)?.set_times(times)?;
    }
    Ok(())
}

pub fn same_path(left: &Path, right: &Path) -> bool {
    same_folded(left, right, IS_WINDOWS)
}

fn same_folded(left: &Path, right: &Path, ignore_case: bool) -> bool {
    if ignore_case {
        let folded = |path: &Path| path.to_string_lossy().to_lowercase();
        folded(left) == folded(right)
    } else {
        left == right
    }
}

#[cfg(windows)]
fn plain(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) => match rest.strip_prefix(r"UNC\") {
            Some(share) => PathBuf::from(format!(r"\\{share}")),
            None => PathBuf::from(rest),
        },
        None => path,
    }
}

#[derive(Clone, Debug)]
pub struct Paths {
    pub program_root: PathBuf,
    pub data_root: PathBuf,
}

impl Paths {
    pub fn detect() -> Self {
        let program_root = if PACKAGED {
            std::env::current_exe()
                .ok()
                .map(|exe| resolve(&exe))
                .and_then(|exe| exe.parent().map(Path::to_path_buf))
                .unwrap_or_default()
        } else {
            resolve(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../build"))
        };
        let data_root = if PACKAGED && IS_WINDOWS {
            program_root.clone()
        } else {
            user_data_base().join(DIR_NAME)
        };
        Self::new(program_root, resolve(&data_root))
    }

    pub fn new(program_root: PathBuf, data_root: PathBuf) -> Self {
        Self {
            program_root,
            data_root,
        }
    }

    pub fn instance_key(&self) -> String {
        let mut hasher = Blake2sVar::new(4).expect("digest size");
        hasher.update(self.data_root.to_string_lossy().as_bytes());
        let mut digest = [0u8; 4];
        hasher.finalize_variable(&mut digest).expect("digest size");
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        format!("{DIR_NAME}-{hex}")
    }

    pub fn liberator_bin(&self) -> PathBuf {
        self.bin_dir().join(LIBERATOR_NAME)
    }

    pub fn rvpn_bin_dir(&self) -> PathBuf {
        self.program_root.join("rvpn")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.data_root.join("settings.toml")
    }

    pub fn playtime_file(&self) -> PathBuf {
        self.data_root.join("playtime.toml")
    }

    pub fn lock_file(&self) -> PathBuf {
        self.data_root.join(".lock")
    }

    pub fn bin_dir(&self) -> PathBuf {
        self.data_root.join("bin")
    }

    pub fn api_cache_file(&self) -> PathBuf {
        self.bin_dir().join("api_cache.json")
    }

    pub fn errors_file(&self) -> PathBuf {
        self.bin_dir().join("errors.txt")
    }

    pub fn default_downloads_dir(&self) -> PathBuf {
        self.data_root.join("downloads")
    }

    pub fn tl_dir(&self) -> PathBuf {
        self.bin_dir().join("tl")
    }

    pub fn dd_bin(&self) -> PathBuf {
        self.bin_dir().join(DD_MEMBER)
    }

    pub fn dd_zip(&self) -> PathBuf {
        self.bin_dir().join(DD_ASSET)
    }

    pub fn sevenz_bin(&self) -> PathBuf {
        self.bin_dir().join(SEVENZ_MEMBER)
    }

    pub fn rvpn_state_dir(&self) -> PathBuf {
        self.data_root.join("rvpn")
    }

    pub fn wine_dir(&self) -> PathBuf {
        self.bin_dir().join("wine")
    }

    pub fn prefix_dir(&self) -> PathBuf {
        self.data_root.join("prefixes")
    }

    pub fn proton_dir(&self) -> PathBuf {
        self.bin_dir().join("proton")
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn the_instance_key_is_the_one_of_the_python_launcher() {
        let key = |root: &str| Paths::new(PathBuf::new(), root.into()).instance_key();
        assert_eq!(
            key("/home/user/.local/share/ThrowbackLauncher"),
            "ThrowbackLauncher-3c353af8"
        );
        assert_eq!(
            key(r"C:\Users\Test\AppData\Local\ThrowbackLauncher"),
            "ThrowbackLauncher-84d35716"
        );
        assert_eq!(key("/tmp/x"), "ThrowbackLauncher-c8827849");
    }

    #[test]
    fn versions_order_like_integers() {
        assert!(parse_version("0.2.10") > parse_version("0.2.9"));
        assert_eq!(parse_version("0.2.1"), Some(vec![0, 2, 1]));
        assert_eq!(parse_version("0.3.0-1"), None);
        assert!(parse_version(VERSION).is_some());
    }

    #[test]
    fn resolve_keeps_missing_tails() {
        let dir = tempfile::tempdir().unwrap();
        let real = resolve(dir.path());
        assert_eq!(
            resolve(&dir.path().join("a").join("..").join("b").join("c")),
            real.join("b").join("c")
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_applies_parent_steps_to_the_link_target() {
        let dir = tempfile::tempdir().unwrap();
        let root = resolve(dir.path());
        std::fs::create_dir_all(root.join("real").join("deep")).unwrap();
        std::fs::create_dir_all(root.join("mnt")).unwrap();
        std::os::unix::fs::symlink(
            root.join("real").join("deep"),
            root.join("mnt").join("link"),
        )
        .unwrap();
        assert_eq!(
            resolve(&root.join("mnt").join("link").join("..").join("games")),
            root.join("real").join("games")
        );
        std::os::unix::fs::symlink("loop", root.join("loop")).unwrap();
        assert_eq!(
            resolve(&root.join("loop").join("x")),
            root.join("loop").join("x")
        );
    }

    #[test]
    fn write_atomic_creates_folders_and_replaces_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a").join("b").join("file.toml");
        write_atomic(&target, "one\n").unwrap();
        write_atomic(&target, "two\n").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "two\n");
        let names: Vec<_> = fs::read_dir(target.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["file.toml"]);
    }

    #[test]
    fn removal_ignores_missing_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        remove_if_present(&file).unwrap();
        fs::write(&file, "").unwrap();
        remove_if_present(&file).unwrap();
        assert!(!file.exists());
        fs::create_dir(&file).unwrap();
        assert!(remove_if_present(&file).is_err());
    }

    #[test]
    fn copies_keep_the_modification_time() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        let to = dir.path().join("to");
        fs::write(&from, "data").unwrap();
        let stamp = std::time::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        let source = fs::File::options().write(true).open(&from).unwrap();
        source.set_modified(stamp).unwrap();
        let mut permissions = source.metadata().unwrap().permissions();
        permissions.set_readonly(true);
        source.set_permissions(permissions).unwrap();
        drop(source);
        fs::write(&to, "older and longer").unwrap();
        copy_with_times(&from, &to).unwrap();
        let copied = fs::metadata(&to).unwrap();
        assert_eq!(fs::read_to_string(&to).unwrap(), "data");
        assert_eq!(copied.modified().unwrap(), stamp);
        assert!(copied.permissions().readonly());
    }

    #[test]
    fn paths_compare_without_case_only_on_windows() {
        let stored = Path::new("/mnt/Games/R6 Jäger");
        let typed = Path::new("/MNT/games/r6 JÄGER");
        assert!(same_folded(stored, typed, true));
        assert!(!same_folded(stored, typed, false));
        assert!(same_folded(stored, stored, false));
        assert!(!same_folded(stored, Path::new("/mnt/Games/R6"), true));
        assert_eq!(same_path(stored, typed), IS_WINDOWS);
    }
}
