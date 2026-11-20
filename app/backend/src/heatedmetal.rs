use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow, bail};

use crate::env::{
    DEFAULTARGS_DLL, HM_API_URL, HM_ASSET, Paths, SEVENZ_API_URL, SEVENZ_ASSET, copy_with_times,
    remove_if_present, resolve, same_path,
};
use crate::github::{Progress, download_error, fetch_to, github_asset};
use crate::log::fail;
use crate::throwbackloader::{apply_tl, ensure_tl, write_launcher};
use crate::winspawn::hidden;

pub const HM_LATEST: &str = "latest";
pub const HM_UNSTABLE: &str = "unstable";

const DEFAULT_ARGS_NAMES: [&str; 2] = ["DefaultArgs.dll", DEFAULTARGS_DLL];
const NOTICES_NAME: &str = "ThirdPartyLegalNotices.txt";
const HM_DIR: &str = "HeatedMetal";
const ARCHIVE_PREFIX: &str = "HeatedMetal-";
const PICKED_ARCHIVE: &str = "Unstable.7z";
const EXTRACTION_FAILED: &str = "Heated Metal extraction failed";

static LATEST_RELEASE: Mutex<Option<(String, String)>> = Mutex::new(None);

#[derive(Debug)]
pub struct HmFilesRemovedError;

impl fmt::Display for HmFilesRemovedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(EXTRACTION_FAILED)
    }
}

impl std::error::Error for HmFilesRemovedError {}

fn default_args(folder: &Path) -> Option<PathBuf> {
    DEFAULT_ARGS_NAMES
        .iter()
        .map(|name| folder.join(name))
        .find(|file| file.exists())
}

fn latest_release() -> MutexGuard<'static, Option<(String, String)>> {
    LATEST_RELEASE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

pub fn clear_release_cache() {
    *latest_release() = None;
}

pub fn resolve_hm_release(paths: &Paths, hm_version: &str) -> Result<(String, String)> {
    let latest = hm_version == HM_LATEST;
    if latest && let Some(release) = latest_release().clone() {
        return Ok(release);
    }
    let api_url = if latest {
        HM_API_URL.to_owned()
    } else {
        format!("https://api.github.com/repos/DataCluster0/HeatedMetal/releases/tags/{hm_version}")
    };
    let release = github_asset(paths, &api_url, HM_ASSET)
        .map_err(|error| download_error("Heated Metal release lookup failed", error))?;
    if latest {
        *latest_release() = Some(release.clone());
    }
    Ok(release)
}

fn download(url: &str, dest: &Path, progress: Progress) -> Result<()> {
    if let Some(folder) = dest.parent() {
        fs::create_dir_all(folder)?;
    }
    fetch_to(url, dest, progress, None)
}

pub fn ensure_7z(paths: &Paths, progress: Progress, force: bool) -> Result<PathBuf> {
    let bin = paths.sevenz_bin();
    if force || !bin.exists() {
        github_asset(paths, SEVENZ_API_URL, SEVENZ_ASSET)
            .and_then(|(_, asset_url)| install_7z(&asset_url, &bin, progress))
            .map_err(|error| download_error("7z download failed", error))?;
    }
    Ok(bin)
}

#[cfg(not(target_os = "linux"))]
fn install_7z(asset_url: &str, bin: &Path, progress: Progress) -> Result<()> {
    download(asset_url, bin, progress)
}

#[cfg(target_os = "linux")]
fn install_7z(asset_url: &str, bin: &Path, progress: Progress) -> Result<()> {
    let tarxz = bin.with_file_name("_7z.tar.xz");
    let tmp_dir = bin.with_file_name(".7z.tmp");
    let outcome =
        download(asset_url, &tarxz, progress).and_then(|()| unpack_7z(&tarxz, &tmp_dir, bin));
    let _ = fs::remove_file(&tarxz);
    let _ = fs::remove_dir_all(&tmp_dir);
    outcome
}

#[cfg(target_os = "linux")]
fn unpack_7z(tarxz: &Path, tmp_dir: &Path, bin: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    use crate::env::SEVENZ_MEMBER;

    let reader = lzma_rust2::XzReader::new(io::BufReader::new(File::open(tarxz)?), true);
    for entry in tar::Archive::new(reader).entries()? {
        let mut entry = entry?;
        if entry.path()? != Path::new(SEVENZ_MEMBER) {
            continue;
        }
        fs::create_dir_all(tmp_dir)?;
        let tmp_bin = tmp_dir.join(SEVENZ_MEMBER);
        let mut file = File::create(&tmp_bin)?;
        io::copy(&mut entry, &mut file)?;
        file.set_permissions(fs::Permissions::from_mode(0o755))?;
        return Ok(fs::rename(tmp_bin, bin)?);
    }
    bail!("{SEVENZ_MEMBER} is missing from the 7-Zip archive")
}

fn run_7z(sevenz: &Path, switches: &[&OsStr], archive: &Path) -> Result<Output> {
    let mut command = Command::new(sevenz);
    command.args(switches).arg(archive);
    hidden(&mut command)
        .output()
        .with_context(|| sevenz.display().to_string())
}

fn sevenz_error(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map_or_else(|| format!("7z {}", output.status), str::to_owned)
}

fn listing_holds_mod(listing: &str) -> bool {
    let entries: Vec<PathBuf> = listing
        .lines()
        .filter_map(|line| line.strip_prefix("Path = "))
        .map(|path| path.split(['\\', '/']).collect())
        .collect();
    entries.iter().any(|entry| {
        entry.parent().is_some_and(|root| {
            root.components().count() <= 1
                && DEFAULT_ARGS_NAMES.iter().any(|name| entry.ends_with(name))
                && entries
                    .iter()
                    .any(|other| other.starts_with(root.join(HM_DIR)))
        })
    })
}

fn archive_has_mod(sevenz: &Path, archive: &Path) -> Result<bool> {
    let output = run_7z(sevenz, &["l", "-ba", "-slt"].map(OsStr::new), archive)?;
    if !output.status.success() {
        bail!(sevenz_error(&output));
    }
    Ok(listing_holds_mod(&String::from_utf8_lossy(&output.stdout)))
}

fn remove_other_archives(bin_dir: &Path, keep: &Path, released: bool) -> io::Result<()> {
    for entry in fs::read_dir(bin_dir)? {
        let path = entry?.path();
        let is_archive = path
            .extension()
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case("7z"));
        let is_release = path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(ARCHIVE_PREFIX));
        if is_archive && is_release == released && !same_path(&path, keep) {
            remove_if_present(&path)?;
        }
    }
    Ok(())
}

pub fn cache_hm_archive(paths: &Paths, archive: &Path) -> Result<PathBuf> {
    let sevenz = ensure_7z(paths, None, false)?;
    let has_mod = archive_has_mod(&sevenz, archive).map_err(|error| {
        anyhow!(fail(
            "Heated Metal archive check failed",
            format!("{error:#}")
        ))
    })?;
    let name = archive.file_name().unwrap_or_default();
    if !has_mod {
        bail!(
            "{} does not contain the Heated Metal files",
            name.to_string_lossy()
        );
    }
    let bin_dir = paths.bin_dir();
    let cached = bin_dir.join(PICKED_ARCHIVE);
    if same_path(&resolve(archive), &resolve(&cached)) {
        return Ok(cached);
    }
    let part = cached.with_added_extension("part");
    copy_with_times(archive, &part)
        .and_then(|()| fs::rename(&part, &cached))
        .and_then(|()| remove_other_archives(&bin_dir, &cached, false))
        .map_err(|error| {
            let _ = fs::remove_file(&part);
            anyhow!(fail("Heated Metal archive copy failed", error))
        })?;
    Ok(cached)
}

fn release_archive(paths: &Paths, release: &str, progress: Progress) -> Result<(String, PathBuf)> {
    let (tag, asset_url) = resolve_hm_release(paths, release)?;
    let bin_dir = paths.bin_dir();
    let archive = bin_dir.join(format!("{ARCHIVE_PREFIX}{tag}.7z"));
    if !archive.exists() {
        download(&asset_url, &archive, progress)
            .map_err(|error| download_error("Heated Metal download failed", error))?;
        let _ = remove_other_archives(&bin_dir, &archive, true);
    }
    Ok((tag, archive))
}

fn unpack(sevenz: &Path, archive: &Path, tmp_dir: &Path) -> Result<Output> {
    let _ = fs::remove_dir_all(tmp_dir);
    fs::create_dir(tmp_dir).with_context(|| tmp_dir.display().to_string())?;
    let mut target = OsString::from("-o");
    target.push(tmp_dir);
    run_7z(
        sevenz,
        &[OsStr::new("x"), OsStr::new("-y"), &target],
        archive,
    )
}

fn unpacked_mod(tmp_dir: &Path) -> Option<PathBuf> {
    let subfolders = fs::read_dir(tmp_dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path());
    std::iter::once(tmp_dir.to_path_buf())
        .chain(subfolders)
        .find(|root| default_args(root).is_some() && root.join(HM_DIR).is_dir())
}

fn extract(paths: &Paths, archive: &Path, tmp_dir: &Path, progress: Progress) -> Result<PathBuf> {
    let sevenz = ensure_7z(paths, progress, false)?;
    let output = unpack(&sevenz, archive, tmp_dir)
        .map_err(|error| anyhow!(fail(EXTRACTION_FAILED, format!("{error:#}"))))?;
    let unpacked = output.status.success();
    if unpacked && let Some(mod_dir) = unpacked_mod(tmp_dir) {
        return Ok(mod_dir);
    }
    let intact = archive_has_mod(&sevenz, archive).unwrap_or(false);
    let detail = if !unpacked {
        sevenz_error(&output)
    } else if intact {
        "files missing after extraction".to_owned()
    } else {
        "the archive does not contain the Heated Metal files".to_owned()
    };
    let message = fail(EXTRACTION_FAILED, detail);
    if intact {
        return Err(HmFilesRemovedError.into());
    }
    let _ = fs::remove_file(archive);
    bail!(message)
}

fn cpu_variant() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    if std::is_x86_feature_detected!("avx") && !std::is_x86_feature_detected!("avx2") {
        return "AVX";
    }
    ""
}

fn hm_dll(target_dir: &Path, variant: &str) -> PathBuf {
    target_dir
        .join(HM_DIR)
        .join(format!("HeatedMetal{variant}.dll"))
}

pub fn hm_files_present(target_dir: &Path) -> bool {
    default_args(target_dir).is_some() && hm_dll(target_dir, cpu_variant()).is_file()
}

pub fn hm_build_time(target_dir: &Path) -> Option<u32> {
    let read = || -> io::Result<u32> {
        let mut dll = File::open(hm_dll(target_dir, ""))?;
        let mut field = [0; 4];
        dll.seek(SeekFrom::Start(0x3C))?;
        dll.read_exact(&mut field)?;
        dll.seek(SeekFrom::Start(u64::from(u32::from_le_bytes(field)) + 8))?;
        dll.read_exact(&mut field)?;
        Ok(u32::from_le_bytes(field))
    };
    read().ok()
}

pub fn hm_crash_log(target_dir: &Path) -> Vec<PathBuf> {
    let logs: Vec<(SystemTime, PathBuf)> = fs::read_dir(target_dir.join(HM_DIR).join("CrashLogs"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|suffix| suffix == "dmp" || suffix == "txt")
        })
        .filter_map(|path| Some((fs::metadata(&path).ok()?.modified().ok()?, path)))
        .collect();
    let Some((_, newest)) = logs.iter().max_by_key(|(modified, _)| *modified) else {
        return Vec::new();
    };
    let mut pair: Vec<PathBuf> = logs
        .iter()
        .map(|(_, path)| path)
        .filter(|path| path.file_stem() == newest.file_stem())
        .cloned()
        .collect();
    pair.sort();
    pair
}

fn version_file(target_dir: &Path) -> PathBuf {
    target_dir.join(HM_DIR).join(".version")
}

pub fn hm_installed_version(target_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(version_file(target_dir)).ok()?;
    let version = text.trim();
    (!version.is_empty()).then(|| version.to_owned())
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    copy_with_times(from, to).with_context(|| format!("{} -> {}", from.display(), to.display()))
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    let entries = fs::read_dir(from).with_context(|| from.display().to_string())?;
    fs::create_dir(to).with_context(|| to.display().to_string())?;
    for entry in entries {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            copy(&entry.path(), &target)?;
        }
    }
    Ok(())
}

fn apply_mod(target_dir: &Path, mod_dir: &Path, variant: &str) -> Result<()> {
    let source = default_args(mod_dir).context("DefaultArgs.dll is missing from the mod files")?;
    remove_hm_files(target_dir)?;
    copy(&source, &target_dir.join(DEFAULTARGS_DLL))?;
    copy_tree(&mod_dir.join(HM_DIR), &target_dir.join(HM_DIR))?;
    let (default_dll, variant_dll) = (hm_dll(target_dir, ""), hm_dll(target_dir, variant));
    if default_dll.exists() && !variant_dll.exists() {
        copy(&default_dll, &variant_dll)?;
    }
    let notices = mod_dir.join(NOTICES_NAME);
    if notices.exists() {
        copy(&notices, &target_dir.join(NOTICES_NAME))?;
    }
    Ok(())
}

fn set_up(
    paths: &Paths,
    target_dir: &Path,
    username: &str,
    mod_dir: &Path,
    version: Option<&str>,
) -> Result<()> {
    apply_tl(paths, target_dir, username)?;
    apply_mod(target_dir, mod_dir, cpu_variant())?;
    if let Some(version) = version {
        let file = version_file(target_dir);
        fs::write(&file, version).with_context(|| file.display().to_string())?;
    }
    write_launcher(paths, target_dir)
}

pub fn apply_hm(
    paths: &Paths,
    target_dir: &Path,
    username: &str,
    release: &str,
    progress: Progress,
    archive: Option<&Path>,
) -> Result<()> {
    let tmp_dir = paths.bin_dir().join(".hm.tmp");
    let install = || -> Result<()> {
        let (version, archive) = if release == HM_UNSTABLE {
            let archive = archive.context("The Heated Metal archive is required")?;
            (None, archive.to_path_buf())
        } else {
            let (tag, archive) = release_archive(paths, release, progress)?;
            (Some(tag), archive)
        };
        let mod_dir = extract(paths, &archive, &tmp_dir, progress)?;
        ensure_tl(paths, progress, false)?;
        set_up(paths, target_dir, username, &mod_dir, version.as_deref())
            .map_err(|error| anyhow!(fail("Heated Metal setup failed", format!("{error:#}"))))
    };
    let outcome = install();
    let _ = fs::remove_dir_all(&tmp_dir);
    outcome
}

pub fn remove_hm_files(target_dir: &Path) -> Result<()> {
    let target_hm = target_dir.join(HM_DIR);
    if target_hm.exists() {
        fs::remove_dir_all(&target_hm).with_context(|| target_hm.display().to_string())?;
    }
    for name in DEFAULT_ARGS_NAMES.into_iter().chain([NOTICES_NAME]) {
        let file = target_dir.join(name);
        remove_if_present(&file).with_context(|| file.display().to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use tempfile::TempDir;

    use super::*;
    use crate::github::testing::seed;
    use crate::log::take_recorded;
    use crate::throwbackloader::testing::names;

    const TAG_URL: &str =
        "https://api.github.com/repos/DataCluster0/HeatedMetal/releases/tags/0.2.3";

    fn paths(dir: &TempDir) -> Paths {
        Paths::new(dir.path().join("asset"), dir.path().join("data"))
    }

    fn write(path: &Path, content: impl AsRef<[u8]>) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    fn stamp(path: &Path, seconds: u64) {
        let file = File::options().write(true).open(path).unwrap();
        file.set_modified(UNIX_EPOCH + Duration::from_secs(seconds))
            .unwrap();
    }

    fn mod_files(mod_dir: &Path) {
        write(&mod_dir.join("DefaultArgs.dll"), "mod args");
        write(&mod_dir.join(HM_DIR).join("HeatedMetal.dll"), "mod dll");
        write(
            &mod_dir.join(HM_DIR).join("Scripts").join("main.lua"),
            "lua",
        );
        write(&mod_dir.join(NOTICES_NAME), "notices");
        write(&mod_dir.join("readme.txt"), "readme");
    }

    #[test]
    fn the_mod_sits_in_the_archive_root_or_one_folder_below() {
        let cases: &[(&[&str], bool)] = &[
            (
                &["HeatedMetal", "HeatedMetal/x.dll", "DefaultArgs.dll"],
                true,
            ),
            (&["defaultargs.dll", "HeatedMetal/x.dll"], true),
            (
                &["Mod", "Mod/DefaultArgs.dll", "Mod/HeatedMetal/x.dll"],
                true,
            ),
            (&["a/Mod/DefaultArgs.dll", "a/Mod/HeatedMetal/x.dll"], false),
            (&["DefaultArgs.dll", "Mod/HeatedMetal/x.dll"], false),
            (&["Mod/DefaultArgs.dll", "HeatedMetal/x.dll"], false),
            (&["DefaultArgs.dll", "HeatedMetalX/x.dll"], false),
            (&["DefaultArgs.dll"], false),
            (&["HeatedMetal/x.dll"], false),
        ];
        for (entries, expected) in cases {
            let listing: String = entries
                .iter()
                .map(|path| format!("Path = {path}\nSize = 3\nMethod = LZMA2:48m\nBlock = 0\n\n"))
                .collect();
            assert_eq!(listing_holds_mod(&listing), *expected, "{entries:?}");
            let windows = listing.replace('/', "\\").replace('\n', "\r\n");
            assert_eq!(listing_holds_mod(&windows), *expected, "{entries:?}");
        }
    }

    #[test]
    fn build_time_is_the_pe_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(hm_build_time(dir.path()), None);
        let mut image = vec![0u8; 512];
        image[0x3C..0x40].copy_from_slice(&0x100u32.to_le_bytes());
        image[0x108..0x10C].copy_from_slice(&1_761_955_200u32.to_le_bytes());
        let dll = dir.path().join(HM_DIR).join("HeatedMetal.dll");
        write(&dll, &image);
        assert_eq!(hm_build_time(dir.path()), Some(1_761_955_200));
        write(&dll, "MZ");
        assert_eq!(hm_build_time(dir.path()), None);
    }

    #[test]
    fn crash_log_is_the_newest_pair() {
        let dir = tempfile::tempdir().unwrap();
        assert!(hm_crash_log(dir.path()).is_empty());
        let logs = dir.path().join(HM_DIR).join("CrashLogs");
        for (name, seconds) in [
            ("old.dmp", 1_000),
            ("old.txt", 1_001),
            ("new.txt", 2_000),
            ("new.dmp", 1_999),
            ("new.log", 3_000),
        ] {
            write(&logs.join(name), name);
            stamp(&logs.join(name), seconds);
        }
        assert_eq!(
            hm_crash_log(dir.path()),
            [logs.join("new.dmp"), logs.join("new.txt")]
        );
        stamp(&logs.join("old.dmp"), 5_000);
        assert_eq!(
            hm_crash_log(dir.path()),
            [logs.join("old.dmp"), logs.join("old.txt")]
        );
        fs::remove_file(logs.join("old.txt")).unwrap();
        assert_eq!(hm_crash_log(dir.path()), [logs.join("old.dmp")]);
    }

    #[test]
    fn installed_version_is_the_trimmed_version_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(hm_installed_version(dir.path()), None);
        let file = dir.path().join("HeatedMetal").join(".version");
        write(&file, "0.4.2.2\r\n");
        assert_eq!(hm_installed_version(dir.path()).as_deref(), Some("0.4.2.2"));
        write(&file, " \n");
        assert_eq!(hm_installed_version(dir.path()), None);
    }

    #[test]
    fn presence_needs_the_loader_file_and_the_dll_for_this_cpu() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path();
        write(&hm_dll(target, cpu_variant()), "dll");
        assert!(!hm_files_present(target));
        write(&target.join("defaultargs.dll"), "args");
        assert!(hm_files_present(target));
        let other = if cpu_variant().is_empty() { "AVX" } else { "" };
        fs::rename(hm_dll(target, cpu_variant()), hm_dll(target, other)).unwrap();
        assert!(!hm_files_present(target));
    }

    #[test]
    fn remove_deletes_the_mod_files_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path();
        remove_hm_files(target).unwrap();
        for name in [
            "DefaultArgs.dll",
            "defaultargs.dll",
            "ThirdPartyLegalNotices.txt",
            "RainbowSix.exe",
            "HeatedMetal/HeatedMetal.dll",
            "HeatedMetal/CrashLogs/a.dmp",
            "HeatedMetal/Saves/slot",
            "HeatedMetal/.version",
            "HeatedMetalX/keep",
        ] {
            write(&target.join(name), name);
        }
        remove_hm_files(target).unwrap();
        assert_eq!(names(target), ["HeatedMetalX", "RainbowSix.exe"]);
    }

    #[test]
    fn mod_files_replace_the_previous_install() {
        let dir = tempfile::tempdir().unwrap();
        let mod_dir = dir.path().join("mod");
        let target = dir.path().join("Y9S2_HeatedMetal");
        mod_files(&mod_dir);
        for name in [
            "DefaultArgs.dll",
            "RainbowSix.exe",
            "HeatedMetal/HeatedMetalAVX.dll",
            "HeatedMetal/CrashLogs/a.dmp",
            "HeatedMetal/Saves/slot",
        ] {
            write(&target.join(name), "previous");
        }
        stamp(&mod_dir.join("DefaultArgs.dll"), 1_600_000_000);
        apply_mod(&target, &mod_dir, "").unwrap();
        assert_eq!(
            names(&target),
            [
                "HeatedMetal",
                "RainbowSix.exe",
                "ThirdPartyLegalNotices.txt",
                "defaultargs.dll",
            ]
        );
        assert_eq!(names(&target.join(HM_DIR)), ["HeatedMetal.dll", "Scripts"]);
        assert_eq!(read(&target.join("defaultargs.dll")), "mod args");
        assert_eq!(
            fs::metadata(target.join("defaultargs.dll"))
                .unwrap()
                .modified()
                .unwrap(),
            UNIX_EPOCH + Duration::from_secs(1_600_000_000)
        );
        assert_eq!(read(&hm_dll(&target, "")), "mod dll");
        assert_eq!(
            read(&target.join(HM_DIR).join("Scripts").join("main.lua")),
            "lua"
        );
        assert_eq!(read(&target.join(NOTICES_NAME)), "notices");
        assert_eq!(read(&target.join("RainbowSix.exe")), "previous");

        fs::remove_file(mod_dir.join(NOTICES_NAME)).unwrap();
        apply_mod(&target, &mod_dir, "").unwrap();
        assert!(!target.join(NOTICES_NAME).exists());

        fs::remove_file(mod_dir.join("DefaultArgs.dll")).unwrap();
        assert!(apply_mod(&target, &mod_dir, "").is_err());
        assert_eq!(read(&target.join("defaultargs.dll")), "mod args");
        assert_eq!(read(&hm_dll(&target, "")), "mod dll");
    }

    #[test]
    fn the_variant_dll_is_a_copy_unless_the_mod_ships_it() {
        let dir = tempfile::tempdir().unwrap();
        let mod_dir = dir.path().join("mod");
        let target = dir.path().join("Y9S2_HeatedMetal");
        fs::create_dir_all(&target).unwrap();
        mod_files(&mod_dir);
        apply_mod(&target, &mod_dir, "AVX").unwrap();
        assert_eq!(
            names(&target.join(HM_DIR)),
            ["HeatedMetal.dll", "HeatedMetalAVX.dll", "Scripts"]
        );
        assert_eq!(read(&hm_dll(&target, "AVX")), "mod dll");
        assert_eq!(read(&hm_dll(&target, "")), "mod dll");

        write(&hm_dll(&mod_dir, "AVX"), "mod avx");
        apply_mod(&target, &mod_dir, "AVX").unwrap();
        assert_eq!(read(&hm_dll(&target, "AVX")), "mod avx");
        assert_eq!(read(&hm_dll(&target, "")), "mod dll");

        apply_mod(&target, &mod_dir, "").unwrap();
        assert_eq!(read(&hm_dll(&target, "AVX")), "mod avx");
        assert_eq!(read(&hm_dll(&target, "")), "mod dll");
    }

    #[test]
    fn the_latest_release_is_remembered_until_the_cache_is_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(&dir);
        let release = |url: &str| ("v1".to_owned(), url.to_owned());
        seed(&paths, HM_API_URL, &[("HeatedMetal.7z", "http://host/a")]);
        assert_eq!(
            resolve_hm_release(&paths, HM_LATEST).unwrap(),
            release("http://host/a")
        );
        seed(&paths, HM_API_URL, &[("HeatedMetal.7z", "http://host/b")]);
        assert_eq!(
            resolve_hm_release(&paths, HM_LATEST).unwrap(),
            release("http://host/a")
        );
        clear_release_cache();
        assert_eq!(
            resolve_hm_release(&paths, HM_LATEST).unwrap(),
            release("http://host/b")
        );

        seed(&paths, TAG_URL, &[("HeatedMetal.7z", "http://host/c")]);
        assert_eq!(
            resolve_hm_release(&paths, "0.2.3").unwrap(),
            release("http://host/c")
        );
        seed(&paths, TAG_URL, &[("HeatedMetal.7z", "http://host/d")]);
        assert_eq!(
            resolve_hm_release(&paths, "0.2.3").unwrap(),
            release("http://host/d")
        );
        assert!(take_recorded().is_empty());
    }

    #[cfg(unix)]
    mod with_7z {
        use std::io::ErrorKind;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::Arc;
        use std::thread;

        use super::*;
        use crate::env::{TL_API_URL, TL_DLLS_COMMON, TL_LAUNCHER, TL_LOADERS, TL_TOML};
        use crate::github::RateLimitError;
        use crate::github::testing::{Reply, Server};

        const FAKE_7Z: &str = r##"#!/bin/sh
[ "$#" -gt 0 ] && printf '%s\n' "$@" >> "$0.calls"
case "$1" in
l) mode=list archive="$4" ;;
x) mode=extract out="${3#-o}" archive="$4" ;;
*) exit 0 ;;
esac
. "$archive"
"##;
        const LIST_MOD: &str = concat!(
            r"printf 'Path = DefaultArgs.dll\r\nSize = 8\r\n\r\nPath = HeatedMetal\r\n",
            r"Path = HeatedMetal\\HeatedMetal.dll\r\n'",
        );
        const EXTRACT_MOD: &str = r##"mkdir -p "$out/HeatedMetal"
printf 'mod args' > "$out/DefaultArgs.dll"
printf 'mod dll' > "$out/HeatedMetal/HeatedMetal.dll"
printf 'notices' > "$out/ThirdPartyLegalNotices.txt""##;
        const LIST_NESTED: &str = concat!(
            r"printf 'Path = Release\nPath = Release/defaultargs.dll\n",
            r"Path = Release/HeatedMetal/HeatedMetal.dll\n'",
        );
        const EXTRACT_NESTED: &str = r##"mkdir -p "$out/Release/HeatedMetal"
printf 'nested args' > "$out/Release/defaultargs.dll"
printf 'nested dll' > "$out/Release/HeatedMetal/HeatedMetal.dll""##;
        const LIST_OTHER: &str = r"printf 'Path = readme.txt\n'";
        const EXTRACT_OTHER: &str = r##"printf readme > "$out/readme.txt""##;
        const BROKEN: &str = "printf 'ERROR: bad archive\\n' >&2\nexit 2\n";

        fn script(list: &str, extract: &str) -> String {
            format!("if [ \"$mode\" = list ]; then\n{list}\nelse\n{extract}\nfi\n")
        }

        fn bench() -> (TempDir, Paths) {
            let dir = tempfile::tempdir().unwrap();
            let paths = paths(&dir);
            let bin = paths.sevenz_bin();
            write(&bin, FAKE_7Z);
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
            while Command::new(&bin)
                .output()
                .is_err_and(|error| error.kind() == ErrorKind::ExecutableFileBusy)
            {
                thread::sleep(Duration::from_millis(5));
            }
            (dir, paths)
        }

        fn calls(paths: &Paths) -> Vec<String> {
            let file = paths.bin_dir().join("7zz.calls");
            let text = fs::read_to_string(&file).unwrap_or_default();
            let _ = fs::remove_file(file);
            text.lines().map(str::to_owned).collect()
        }

        fn loader(paths: &Paths) {
            for name in TL_DLLS_COMMON
                .into_iter()
                .chain(TL_LOADERS)
                .chain([TL_LAUNCHER])
            {
                write(&paths.tl_dir().join(name), format!("loader {name}"));
            }
            write(
                &paths.tl_dir().join(TL_TOML),
                "[User]\nusername = \"ThrowbackUser\"\n\n[Launch]\nargs = ''\nautorun = []\n",
            );
        }

        fn serve(status: u16, body: &str) -> Server {
            let body = body.as_bytes().to_vec();
            Server::start(move |_| Reply::new(status, body.clone()))
        }

        fn tmp_dir(paths: &Paths) -> PathBuf {
            paths.bin_dir().join(".hm.tmp")
        }

        fn extract_into_tmp(paths: &Paths, archive: &Path) -> Result<PathBuf> {
            extract(paths, archive, &tmp_dir(paths), None)
        }

        #[test]
        fn a_selected_archive_is_checked_and_copied_into_the_cache() {
            let (dir, paths) = bench();
            let bin_dir = paths.bin_dir();
            let picked = dir.path().join("Downloads").join("Un stable.7z");
            write(&picked, script(LIST_MOD, EXTRACT_MOD));
            for name in ["Old.7z", "HeatedMetal-0.2.3.7z", "notes.txt"] {
                write(&bin_dir.join(name), "old");
            }
            let cached = cache_hm_archive(&paths, &picked).unwrap();
            assert_eq!(cached, bin_dir.join("Unstable.7z"));
            assert_eq!(read(&cached), read(&picked));
            assert_eq!(
                calls(&paths),
                ["l", "-ba", "-slt", picked.to_str().unwrap()]
            );
            let kept = ["7zz", "HeatedMetal-0.2.3.7z", "Unstable.7z", "notes.txt"];
            assert_eq!(names(&bin_dir), kept);

            let named = dir.path().join("Downloads").join("HeatedMetal-next.7z");
            write(&named, script(LIST_MOD, EXTRACT_MOD));
            assert_eq!(cache_hm_archive(&paths, &named).unwrap(), cached);
            calls(&paths);
            assert_eq!(names(&bin_dir), kept);

            write(&bin_dir.join("Other.7z"), "other");
            assert_eq!(cache_hm_archive(&paths, &cached).unwrap(), cached);
            assert_eq!(read(&cached), read(&picked));
            assert!(bin_dir.join("Other.7z").exists());
            assert!(take_recorded().is_empty());
        }

        #[test]
        fn a_selected_archive_without_the_mod_is_refused() {
            let (dir, paths) = bench();
            let bin_dir = paths.bin_dir();
            let picked = dir.path().join("Wrong.7z");
            write(&picked, script(LIST_OTHER, EXTRACT_OTHER));
            write(&bin_dir.join("Unstable.7z"), "valid");
            let error = cache_hm_archive(&paths, &picked).unwrap_err();
            assert_eq!(
                error.to_string(),
                "Wrong.7z does not contain the Heated Metal files"
            );
            assert!(take_recorded().is_empty());
            assert!(picked.exists());
            assert_eq!(read(&bin_dir.join("Unstable.7z")), "valid");
            assert!(!bin_dir.join("Wrong.7z").exists());

            write(&picked, BROKEN);
            let error = cache_hm_archive(&paths, &picked).unwrap_err();
            assert_eq!(error.to_string(), "Heated Metal archive check failed");
            assert_eq!(take_recorded().len(), 1);

            write(&picked, script(LIST_MOD, EXTRACT_MOD));
            fs::create_dir(bin_dir.join("Unstable.7z.part")).unwrap();
            let error = cache_hm_archive(&paths, &picked).unwrap_err();
            assert_eq!(error.to_string(), "Heated Metal archive copy failed");
            assert_eq!(take_recorded().len(), 1);
            assert_eq!(read(&bin_dir.join("Unstable.7z")), "valid");
        }

        #[test]
        fn extraction_returns_the_folder_that_holds_the_mod() {
            let (dir, paths) = bench();
            let tmp_dir = tmp_dir(&paths);
            write(&tmp_dir.join("stale").join("file"), "stale");
            let archive = dir.path().join("Unstable.7z");
            write(&archive, script(LIST_MOD, EXTRACT_MOD));
            assert_eq!(extract_into_tmp(&paths, &archive).unwrap(), tmp_dir);
            assert_eq!(
                names(&tmp_dir),
                [
                    "DefaultArgs.dll",
                    "HeatedMetal",
                    "ThirdPartyLegalNotices.txt"
                ]
            );
            assert_eq!(
                calls(&paths),
                [
                    "x",
                    "-y",
                    &format!("-o{}", tmp_dir.display()),
                    archive.to_str().unwrap(),
                ]
            );
            write(&archive, script(LIST_NESTED, EXTRACT_NESTED));
            assert_eq!(
                extract_into_tmp(&paths, &archive).unwrap(),
                tmp_dir.join("Release")
            );
            assert_eq!(names(&tmp_dir), ["Release"]);
            assert!(archive.exists());
            assert!(take_recorded().is_empty());
        }

        #[test]
        fn missing_files_after_extraction_keep_the_archive() {
            let (dir, paths) = bench();
            let archive = dir.path().join("Unstable.7z");
            let disk_full = "printf 'ERROR: No space left on device\\n' >&2\nexit 2";
            for extract in [r##"mkdir -p "$out/HeatedMetal""##, disk_full] {
                write(&archive, script(LIST_MOD, extract));
                let error = extract_into_tmp(&paths, &archive).unwrap_err();
                assert!(error.is::<HmFilesRemovedError>());
                assert_eq!(error.to_string(), "Heated Metal extraction failed");
                assert!(archive.exists());
                assert_eq!(take_recorded().len(), 1);
            }
        }

        #[test]
        fn archives_without_the_mod_are_deleted() {
            let (dir, paths) = bench();
            let archive = dir.path().join("Unstable.7z");
            for content in [script(LIST_OTHER, EXTRACT_OTHER), BROKEN.to_owned()] {
                write(&archive, content);
                let error = extract_into_tmp(&paths, &archive).unwrap_err();
                assert!(!error.is::<HmFilesRemovedError>());
                assert_eq!(error.to_string(), "Heated Metal extraction failed");
                assert!(!archive.exists());
                assert_eq!(take_recorded().len(), 1);
            }
        }

        #[test]
        fn releases_are_downloaded_once_and_replace_older_ones() {
            let (_dir, paths) = bench();
            let bin_dir = paths.bin_dir();
            let server = serve(200, "archive");
            seed(&paths, TAG_URL, &[("HeatedMetal.7z", &server.url("/hm"))]);
            for name in [
                "HeatedMetal-0.1.0.7z",
                "HeatedMetal-notes.txt",
                "Unstable.7z",
            ] {
                write(&bin_dir.join(name), "old");
            }
            let seen = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&seen);
            let report = move |fraction: f64| sink.lock().unwrap().push(fraction);
            let (tag, archive) = release_archive(&paths, "0.2.3", Some(&report)).unwrap();
            assert_eq!(tag, "v1");
            assert_eq!(archive, bin_dir.join("HeatedMetal-v1.7z"));
            assert_eq!(read(&archive), "archive");
            assert_eq!(seen.lock().unwrap().last(), Some(&1.0));
            assert_eq!(
                names(&bin_dir),
                [
                    "7zz",
                    "HeatedMetal-notes.txt",
                    "HeatedMetal-v1.7z",
                    "Unstable.7z",
                    "api_cache.json",
                ]
            );
            release_archive(&paths, "0.2.3", Some(&report)).unwrap();
            assert_eq!(server.paths(), ["/hm"]);
            assert!(take_recorded().is_empty());
        }

        #[test]
        fn failed_release_downloads_keep_the_old_archive() {
            let (_dir, paths) = bench();
            let bin_dir = paths.bin_dir();
            write(&bin_dir.join("HeatedMetal-0.2.2.7z"), "old");
            let server = Server::start(|request| match request.path.as_str() {
                "/limited" => Reply::rate_limit(),
                _ => Reply::new(404, Vec::new()),
            });
            seed(&paths, TAG_URL, &[("HeatedMetal.7z", &server.url("/gone"))]);
            let error = release_archive(&paths, "0.2.3", None).unwrap_err();
            assert_eq!(error.to_string(), "Heated Metal download failed");
            assert_eq!(take_recorded().len(), 1);
            assert_eq!(
                names(&bin_dir),
                ["7zz", "HeatedMetal-0.2.2.7z", "api_cache.json"]
            );
            seed(
                &paths,
                TAG_URL,
                &[("HeatedMetal.7z", &server.url("/limited"))],
            );
            let error = release_archive(&paths, "0.2.3", None).unwrap_err();
            assert!(error.is::<RateLimitError>());
            assert!(take_recorded().is_empty());
        }

        #[test]
        fn an_unstable_archive_is_applied_without_a_version_file() {
            let (dir, paths) = bench();
            loader(&paths);
            let archive = paths.bin_dir().join("Unstable.7z");
            write(&archive, script(LIST_MOD, EXTRACT_MOD));
            let target = dir.path().join("library").join("Y9S2_HeatedMetal");
            write(&target.join("defaultargs.dll"), "depot original");
            write(&version_file(&target), "0.1");
            apply_hm(&paths, &target, "Ash", HM_UNSTABLE, None, Some(&archive)).unwrap();
            assert_eq!(read(&target.join("defaultargs.dll")), "mod args");
            assert_eq!(read(&hm_dll(&target, "")), "mod dll");
            assert_eq!(read(&target.join(NOTICES_NAME)), "notices");
            assert_eq!(hm_installed_version(&target), None);
            assert!(hm_files_present(&target));
            assert_eq!(read(&target.join(TL_LAUNCHER)), "loader LaunchR6.exe");
            assert_eq!(
                read(&target.join("steam_api64.dll")),
                "loader steam_api64.dll"
            );
            assert!(read(&target.join(TL_TOML)).contains("username = 'Ash'"));
            assert!(!tmp_dir(&paths).exists());
            assert!(archive.exists());
            assert!(take_recorded().is_empty());
        }

        #[test]
        fn a_release_is_applied_with_its_tag() {
            let (dir, paths) = bench();
            loader(&paths);
            let server = serve(200, &script(LIST_NESTED, EXTRACT_NESTED));
            seed(&paths, TAG_URL, &[("HeatedMetal.7z", &server.url("/hm"))]);
            let target = dir.path().join("library").join("Y5S3_HeatedMetal");
            fs::create_dir_all(&target).unwrap();
            apply_hm(&paths, &target, "Ash", "0.2.3", None, None).unwrap();
            assert_eq!(hm_installed_version(&target).as_deref(), Some("v1"));
            assert_eq!(read(&target.join("defaultargs.dll")), "nested args");
            assert_eq!(read(&hm_dll(&target, "")), "nested dll");
            assert!(target.join(TL_LAUNCHER).exists());
            assert!(paths.bin_dir().join("HeatedMetal-v1.7z").exists());
            assert!(!tmp_dir(&paths).exists());
            assert!(take_recorded().is_empty());
        }

        #[test]
        fn apply_failures_carry_their_message_and_clean_up() {
            let (dir, paths) = bench();
            let target = dir.path().join("library").join("Y9S2_HeatedMetal");
            fs::create_dir_all(&target).unwrap();
            let apply = |archive| apply_hm(&paths, &target, "Ash", HM_UNSTABLE, None, archive);

            write(&tmp_dir(&paths).join("stale"), "stale");
            let error = apply(None).unwrap_err();
            assert_eq!(error.to_string(), "The Heated Metal archive is required");
            assert!(take_recorded().is_empty());
            assert!(!tmp_dir(&paths).exists());

            let archive = paths.bin_dir().join("Unstable.7z");
            write(&archive, script(LIST_MOD, "true"));
            let error = apply(Some(&archive)).unwrap_err();
            assert!(error.is::<HmFilesRemovedError>());
            assert!(names(&target).is_empty());
            assert!(!tmp_dir(&paths).exists());
            assert_eq!(take_recorded().len(), 1);

            write(&archive, script(LIST_MOD, EXTRACT_MOD));
            let server = Server::start(|_| Reply::rate_limit());
            seed(
                &paths,
                TL_API_URL,
                &[("ThrowbackLoader.zip", &server.url("/tl"))],
            );
            let error = apply(Some(&archive)).unwrap_err();
            assert!(error.is::<RateLimitError>());
            assert!(names(&target).is_empty());
            assert!(!tmp_dir(&paths).exists());
            assert!(take_recorded().is_empty());

            loader(&paths);
            fs::create_dir(target.join(NOTICES_NAME)).unwrap();
            let error = apply(Some(&archive)).unwrap_err();
            assert_eq!(error.to_string(), "Heated Metal setup failed");
            assert!(target.join("steam_api64.dll").exists());
            assert!(!target.join(TL_LAUNCHER).exists());
            assert!(!tmp_dir(&paths).exists());
            assert_eq!(take_recorded().len(), 1);
        }

        #[cfg(target_os = "linux")]
        const SEVENZ_TAR_XZ: &str = concat!(
            "fd377a585a000004e6d6b446020021011c00000010cf58cce027ff00aa5d00241a4a676f9d775df3fe618228e423a9ae",
            "82e4479e5f8747f42d75ebd16e6b1b6160fdadb63f9b7939328abcf4b09a84b7edba02ffb0701b62b25a62a80551dd56",
            "09172ee58ca41f9b3b045d449629244b5fc2a46103446c081613e9079acf7e68842519d32addeab40b6520ab8171eedf",
            "b5310773a9dd8c6580731869183fc6bc9d3159de6e6396e4f79ac0c88e28a5f9d5dbae795f891cffc812ffd7e8c317ed",
            "560c9e8336c71b13800000003bb2e9bf95192cd60001c601805000004f9861b8b1c467fb020000000004595a",
        );

        #[cfg(target_os = "linux")]
        fn seed_7z(paths: &Paths, server: &Server) {
            let asset = format!("7z2600-{SEVENZ_ASSET}");
            seed(paths, SEVENZ_API_URL, &[(&asset, &server.url("/7z"))]);
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn sevenz_is_unpacked_from_the_release_archive() {
            let dir = tempfile::tempdir().unwrap();
            let paths = paths(&dir);
            let tarxz: Vec<u8> = (0..SEVENZ_TAR_XZ.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&SEVENZ_TAR_XZ[at..at + 2], 16).unwrap())
                .collect();
            let server = Server::start(move |_| Reply::new(200, tarxz.clone()));
            seed_7z(&paths, &server);
            let bin = ensure_7z(&paths, None, false).unwrap();
            assert_eq!(bin, paths.bin_dir().join("7zz"));
            assert_eq!(read(&bin), "#!/bin/sh\necho 7-Zip\n");
            let mode = fs::metadata(&bin).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
            assert_eq!(names(&paths.bin_dir()), ["7zz", "api_cache.json"]);

            write(&bin, "mine");
            assert_eq!(ensure_7z(&paths, None, false).unwrap(), bin);
            assert_eq!(read(&bin), "mine");
            assert_eq!(ensure_7z(&paths, None, true).unwrap(), bin);
            assert_eq!(read(&bin), "#!/bin/sh\necho 7-Zip\n");
            assert_eq!(server.requests().len(), 2);
            assert!(take_recorded().is_empty());
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn failed_sevenz_downloads_keep_the_old_binary() {
            let dir = tempfile::tempdir().unwrap();
            let paths = paths(&dir);
            write(&paths.sevenz_bin(), "old");
            for status in [200, 404] {
                seed_7z(&paths, &serve(status, "not an archive"));
                let error = ensure_7z(&paths, None, true).unwrap_err();
                assert_eq!(error.to_string(), "7z download failed");
                assert_eq!(read(&paths.sevenz_bin()), "old");
                assert_eq!(names(&paths.bin_dir()), ["7zz", "api_cache.json"]);
                assert_eq!(take_recorded().len(), 1);
            }

            fs::remove_file(paths.sevenz_bin()).unwrap();
            seed_7z(&paths, &Server::start(|_| Reply::rate_limit()));
            let archive = dir.path().join("Unstable.7z");
            write(&archive, script(LIST_MOD, EXTRACT_MOD));
            let error = cache_hm_archive(&paths, &archive).unwrap_err();
            assert!(error.is::<RateLimitError>());
            let error = extract_into_tmp(&paths, &archive).unwrap_err();
            assert!(error.is::<RateLimitError>());
            assert!(archive.exists());
            assert!(take_recorded().is_empty());
        }
    }
}
