use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use regex::Regex;
use serde::Deserialize;

use crate::depot::ensure_depotdownloader;
use crate::env::{
    APP_NAME, DD_API_URL, DD_ASSET, EXE_NAME, GAME_RUNNING, HM_API_URL, HM_ASSET, IS_WINDOWS,
    NOT_INSTALLED_HM, PACKAGED, Paths, RELAUNCH_ARG, SEVENZ_API_URL, SEVENZ_ASSET, TL_API_URL,
    TL_ASSET, VERSION, parse_version,
};
use crate::github::{
    Progress, RateLimitError, download_error, fetch_to, github_asset, github_json,
    invalidate_api_cache,
};
use crate::heatedmetal::{
    HM_LATEST, apply_hm, clear_release_cache, ensure_7z, hm_installed_version,
};
use crate::log::fail;
use crate::manifest::{Download, hm_folder_name, installed_downloads, is_installed};
use crate::self_update::{replace_program, staging_file, write_outcome};
use crate::steam::is_game_running;
use crate::throwbackloader::{
    apply_tl, ensure_tl, tl_folder_version, tl_present, tl_version, write_launcher,
};
use crate::winspawn::hidden;

const UPDATE_API_URL: &str =
    "https://api.github.com/repos/xeralin/ThrowbackLauncher/releases/latest";

const APPIMAGE_ASSET: &str = "ThrowbackLauncher.AppImage";

const VERSION_TIMEOUT: Duration = Duration::from_secs(30);
const PROGRAM_ASSET: &str = if IS_WINDOWS { EXE_NAME } else { APPIMAGE_ASSET };

static DD_VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"v(\d+(?:\.\d+)+)").expect("pattern"));

static SEVENZ_VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"7-Zip(?:\s+\([arz]\))?\s+(\d+(?:\.\d+)+)").expect("pattern"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Component {
    Launcher,
    DepotDownloader,
    SevenZip,
    ThrowbackLoader,
    HeatedMetal,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(from = "Published")]
pub struct ReleaseNotes {
    pub body: String,
    pub url: String,
    pub date: String,
}

#[derive(Deserialize)]
struct Published {
    body: Option<String>,
    html_url: Option<String>,
    published_at: Option<String>,
}

impl From<Published> for ReleaseNotes {
    fn from(release: Published) -> Self {
        Self {
            body: release.body.unwrap_or_default(),
            url: release.html_url.unwrap_or_default(),
            date: release.published_at.unwrap_or_default(),
        }
    }
}

#[derive(Deserialize)]
struct Asset {
    name: String,
}

#[derive(Deserialize)]
struct Listed {
    tag_name: String,
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
    #[serde(flatten)]
    notes: ReleaseNotes,
}

type Probe = (Component, bool, Result<(Vec<Listed>, Option<String>)>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub component: Component,
    pub target: String,
    pub notes: ReleaseNotes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub notes: ReleaseNotes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    RateLimit(String),
    Error,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Check {
    pub pending: Vec<Pending>,
    pub failure: Option<Failure>,
    pub history: Vec<(Component, Vec<Release>)>,
}

pub struct Install {
    pub paths: Paths,
    pub libraries: Vec<PathBuf>,
    pub downloads: Vec<Download>,
    pub username: String,
}

fn newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => latest != current,
    }
}

fn oldest(versions: impl Iterator<Item = Option<String>>) -> Option<String> {
    let versions: Vec<String> = versions.collect::<Option<_>>()?;
    if versions
        .iter()
        .all(|version| parse_version(version).is_some())
    {
        versions
            .into_iter()
            .min_by_key(|version| parse_version(version))
    } else {
        versions.into_iter().min()
    }
}

fn tool_output(binary: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut command = Command::new(binary);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = hidden(&mut command).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let (sender, output) = mpsc::channel();
    thread::spawn(move || {
        let mut text = Vec::new();
        let _ = stdout.read_to_end(&mut text);
        let _ = sender.send(text);
    });
    let output = output.recv_timeout(timeout);
    if output.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    Some(String::from_utf8_lossy(&output.ok()?).into_owned())
}

fn tool_version(binary: &Path, args: &[&str], pattern: &Regex) -> Option<String> {
    let output = tool_output(binary, args, VERSION_TIMEOUT)?;
    Some(pattern.captures(&output)?[1].to_owned())
}

fn numbered(tag: &str) -> Result<String> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    ensure!(
        parse_version(version).is_some(),
        "unexpected release tag {tag}"
    );
    Ok(version.to_owned())
}

fn hm_installs(install: &Install) -> Vec<PathBuf> {
    let latest = install
        .downloads
        .iter()
        .filter(|download| download.hm_version.as_deref() == Some(HM_LATEST));
    latest
        .flat_map(|download| {
            let folder = hm_folder_name(&download.key);
            install.libraries.iter().map(move |root| root.join(&folder))
        })
        .filter(|folder| is_installed(folder))
        .collect()
}

fn game_stopped(install: &Install) -> Result<()> {
    ensure!(!is_game_running(&install.libraries), GAME_RUNNING);
    Ok(())
}

fn program(paths: &Paths) -> Option<PathBuf> {
    if IS_WINDOWS {
        return Some(paths.asset_root.join(EXE_NAME));
    }
    let appimage = std::env::var_os("APPIMAGE").filter(|path| !path.is_empty());
    appimage.map(PathBuf::from)
}

fn self_updatable(paths: &Paths) -> bool {
    PACKAGED && program(paths).is_some()
}

fn replace_launcher(paths: &Paths, program: &Path, progress: Progress) -> Result<()> {
    let (_, url) = github_asset(paths, UPDATE_API_URL, PROGRAM_ASSET)?;
    let staged = staging_file(program);
    let swap = || -> Result<()> {
        fetch_to(&url, &staged, progress, None)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))?;
        }
        Ok(replace_program(program, &staged)?)
    };
    swap().inspect_err(|_| {
        let _ = fs::remove_file(&staged);
    })
}

fn update_launcher(paths: &Paths, program: Option<PathBuf>, progress: Progress) -> Result<()> {
    program
        .context("APPIMAGE is not set")
        .and_then(|program| replace_launcher(paths, &program, progress))
        .map_err(|error| download_error(&format!("{APP_NAME} update failed"), error))?;
    write_outcome(paths, true);
    Ok(())
}

#[cfg(windows)]
fn start(program: &Path) -> Result<()> {
    use std::ffi::OsStr;

    crate::winspawn::spawn_detached(&[program.as_os_str(), OsStr::new(RELAUNCH_ARG)])
}

#[cfg(unix)]
fn start(program: &Path) -> Result<()> {
    use std::os::unix::process::CommandExt;

    Command::new(program)
        .arg(RELAUNCH_ARG)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    Ok(())
}

pub fn relaunch(paths: &Paths) {
    if let Some(program) = program(paths)
        && let Err(error) = start(&program)
    {
        fail(&format!("{APP_NAME} restart failed"), format!("{error:#}"));
    }
}

fn in_parallel<T: Send>(
    components: &[Component],
    work: impl Fn(Component) -> Result<T> + Sync,
) -> Vec<Result<T>> {
    let work = &work;
    thread::scope(|scope| {
        let workers: Vec<_> = components
            .iter()
            .map(|&component| scope.spawn(move || work(component)))
            .collect();
        workers
            .into_iter()
            .map(|worker| {
                let outcome = worker.join();
                outcome.unwrap_or_else(|_| Err(anyhow!("worker panicked")))
            })
            .collect()
    })
}

impl Component {
    pub const ALL: [Self; 5] = [
        Self::Launcher,
        Self::DepotDownloader,
        Self::SevenZip,
        Self::ThrowbackLoader,
        Self::HeatedMetal,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Launcher => APP_NAME,
            Self::DepotDownloader => "DepotDownloader",
            Self::SevenZip => "7z",
            Self::ThrowbackLoader => "ThrowbackLoader",
            Self::HeatedMetal => "Heated Metal",
        }
    }

    pub fn restarts(self) -> bool {
        self == Self::Launcher
    }

    pub fn repository(self) -> &'static str {
        let url = self.release_url();
        let path = url
            .strip_prefix("https://api.github.com/repos/")
            .unwrap_or(url);
        path.strip_suffix("/releases/latest").unwrap_or(path)
    }

    fn release_url(self) -> &'static str {
        match self {
            Self::Launcher => UPDATE_API_URL,
            Self::DepotDownloader => DD_API_URL,
            Self::SevenZip => SEVENZ_API_URL,
            Self::ThrowbackLoader => TL_API_URL,
            Self::HeatedMetal => HM_API_URL,
        }
    }

    fn present(self, install: &Install) -> bool {
        let paths = &install.paths;
        match self {
            Self::Launcher => self_updatable(paths),
            Self::DepotDownloader => paths.dd_bin().exists(),
            Self::SevenZip => paths.sevenz_bin().exists(),
            Self::ThrowbackLoader => tl_present(&paths.tl_dir()),
            Self::HeatedMetal => !hm_installs(install).is_empty(),
        }
    }

    fn current(self, install: &Install) -> Option<String> {
        let paths = &install.paths;
        match self {
            Self::Launcher => Some(VERSION.to_owned()),
            Self::DepotDownloader => tool_version(&paths.dd_bin(), &["--version"], &DD_VERSION),
            Self::SevenZip => tool_version(&paths.sevenz_bin(), &[], &SEVENZ_VERSION),
            Self::ThrowbackLoader => {
                let seasons = installed_downloads(&install.libraries);
                let copies = seasons.iter().map(|folder| tl_folder_version(folder));
                oldest(std::iter::once(tl_version(paths)).chain(copies))
            }
            Self::HeatedMetal => {
                let installs = hm_installs(install);
                oldest(installs.iter().map(|folder| hm_installed_version(folder)))
            }
        }
    }

    fn releases(self, paths: &Paths) -> Result<Vec<Listed>> {
        let url = self.release_url().trim_end_matches("/latest");
        github_json(paths, url, false)
    }

    fn asset(self) -> &'static str {
        match self {
            Self::Launcher => PROGRAM_ASSET,
            Self::DepotDownloader => DD_ASSET,
            Self::SevenZip => SEVENZ_ASSET,
            Self::ThrowbackLoader => TL_ASSET,
            Self::HeatedMetal => HM_ASSET,
        }
    }

    fn latest(self, listed: &[Listed]) -> Result<(String, ReleaseNotes)> {
        let newest = listed
            .iter()
            .find(|release| !release.prerelease)
            .context("no published release")?;
        let suffix = self.asset();
        let mut assets = newest.assets.iter();
        ensure!(
            assets.any(|asset| asset.name.ends_with(suffix)),
            "no release asset matching *{suffix}"
        );
        let tag = newest.tag_name.as_str();
        let version = match self {
            Self::Launcher | Self::ThrowbackLoader => numbered(tag)?,
            Self::DepotDownloader => tag
                .strip_prefix("DepotDownloader_")
                .unwrap_or(tag)
                .to_owned(),
            Self::SevenZip | Self::HeatedMetal => tag.to_owned(),
        };
        Ok((version, newest.notes.clone()))
    }

    fn offer(self, listed: &[Listed], current: Option<&str>) -> Result<Option<Pending>> {
        let (target, notes) = self.latest(listed)?;
        let newer = current.is_none_or(|current| newer(&target, current));
        Ok(newer.then_some(Pending {
            component: self,
            target,
            notes,
        }))
    }

    pub fn apply(self, install: &Install, progress: Progress) -> Result<()> {
        let paths = &install.paths;
        match self {
            Self::Launcher => {
                game_stopped(install)?;
                update_launcher(paths, program(paths), progress)
            }
            Self::DepotDownloader => ensure_depotdownloader(paths, progress, true).map(drop),
            Self::SevenZip => ensure_7z(paths, progress, true).map(drop),
            Self::ThrowbackLoader => {
                game_stopped(install)?;
                ensure_tl(paths, progress, true)?;
                let mut outcome = Ok(());
                for folder in installed_downloads(&install.libraries) {
                    let applied = apply_tl(paths, &folder, &install.username)
                        .and_then(|()| write_launcher(paths, &folder));
                    if let Err(error) = applied {
                        let message = fail("ThrowbackLoader setup failed", format!("{error:#}"));
                        outcome = Err(anyhow!(message));
                    }
                }
                outcome
            }
            Self::HeatedMetal => {
                let installs = hm_installs(install);
                ensure!(!installs.is_empty(), NOT_INSTALLED_HM);
                game_stopped(install)?;
                let mut outcome = Ok(());
                for folder in installs {
                    let applied =
                        apply_hm(paths, &folder, &install.username, HM_LATEST, progress, None);
                    match applied {
                        Err(error) if error.is::<RateLimitError>() => return Err(error),
                        Err(error) => outcome = Err(error),
                        Ok(()) => {}
                    }
                }
                outcome
            }
        }
    }
}

fn published(listed: Vec<Listed>, current: Option<&str>) -> Vec<Release> {
    let releases = listed
        .into_iter()
        .filter(|release| !release.prerelease)
        .map(|release| Release {
            version: release
                .tag_name
                .trim_start_matches(|letter: char| !letter.is_ascii_digit())
                .to_owned(),
            notes: release.notes,
        });
    releases
        .filter(|release| current.is_none_or(|current| !newer(&release.version, current)))
        .collect()
}

impl Check {
    fn failed(&mut self, component: Component, error: &anyhow::Error, present: bool) {
        let limit = error.downcast_ref::<RateLimitError>();
        if limit.is_none() {
            fail(
                &format!("{} check failed", component.name()),
                format!("{error:#}"),
            );
        }
        if present && !matches!(self.failure, Some(Failure::RateLimit(_))) {
            let failure = limit.map_or(Failure::Error, |limit| {
                Failure::RateLimit(limit.to_string())
            });
            self.failure = Some(failure);
        }
    }
}

fn assess(probes: impl Iterator<Item = Probe>) -> Check {
    let mut check = Check::default();
    for (component, present, found) in probes {
        let releases = match found {
            Ok((listed, current)) => {
                if present {
                    match component.offer(&listed, current.as_deref()) {
                        Ok(offered) => check.pending.extend(offered),
                        Err(error) => check.failed(component, &error, true),
                    }
                }
                published(listed, current.as_deref())
            }
            Err(error) => {
                check.failed(component, &error, present);
                Vec::new()
            }
        };
        check.history.push((component, releases));
    }
    check
}

fn check(install: &Install, force: bool) -> Check {
    let paths = &install.paths;
    if force && let Err(error) = invalidate_api_cache(paths) {
        fail("Update check failed", format!("{error:#}"));
        return Check {
            failure: Some(Failure::Error),
            ..Check::default()
        };
    }
    let present = Component::ALL.map(|component| component.present(install));
    let found = in_parallel(&Component::ALL, |component| {
        let listed = component.releases(paths)?;
        Ok((listed, component.current(install)))
    });
    let probes = Component::ALL.into_iter().zip(present).zip(found);
    assess(probes.map(|((component, present), found)| (component, present, found)))
}

pub fn available(install: &Install, force: bool) -> Check {
    clear_release_cache();
    check(install, force)
}

pub fn installed(install: &Install) -> Vec<(Component, String)> {
    Component::ALL
        .into_iter()
        .filter_map(|component| Some((component, component.current(install)?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::{Cursor, Write};
    use std::sync::{Arc, Mutex};
    use std::time::SystemTime;

    use serde_json::{Value, json};
    use tempfile::TempDir;
    use zip::write::{SimpleFileOptions, ZipWriter};

    use super::*;
    use crate::env::{TL_DLLS_COMMON, TL_LAUNCHER, TL_LOADERS, TL_TOML};
    use crate::github::testing::{Reply, Server};
    use crate::log::take_recorded;
    use crate::manifest::downloads;
    use crate::pe::versioned_exe;
    use crate::self_update::take_outcome;
    use crate::throwbackloader::testing::names;

    const CONFIG: &str = "[User]\nusername = 'ThrowbackUser'\n\n[Launch]\nargs = ''\n";

    struct Bench {
        dir: TempDir,
        install: Install,
    }

    fn bench() -> Bench {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        fs::create_dir_all(paths.bin_dir()).unwrap();
        let install = Install {
            paths,
            libraries: vec![dir.path().join("one"), dir.path().join("two")],
            downloads: downloads(),
            username: "Ash".to_owned(),
        };
        Bench { dir, install }
    }

    fn seed(paths: &Paths, answers: &[(&str, Value)]) {
        let fetched = SystemTime::now();
        let cache: HashMap<&str, Value> = answers
            .iter()
            .map(|(url, data)| (*url, json!({"fetched": fetched, "data": data})))
            .collect();
        let text = serde_json::to_string(&cache).unwrap();
        fs::write(paths.api_cache_file(), text).unwrap();
    }

    fn release(tag: &str, assets: &[(&str, &str)]) -> Value {
        let assets: Vec<Value> = assets
            .iter()
            .map(|(name, url)| json!({"name": name, "url": url}))
            .collect();
        json!({"tag_name": tag, "assets": assets})
    }

    fn launcher_exe(version: [u32; 3]) -> Vec<u8> {
        let [major, minor, patch] = version;
        versioned_exe(&format!("{major}.{minor}.{patch}"))
    }

    fn loader_files(version: [u32; 3]) -> Vec<(&'static str, Vec<u8>)> {
        let dlls = TL_DLLS_COMMON.into_iter().chain(TL_LOADERS);
        dlls.map(|name| (name, name.as_bytes().to_vec()))
            .chain([
                (TL_TOML, CONFIG.as_bytes().to_vec()),
                (TL_LAUNCHER, launcher_exe(version)),
            ])
            .collect()
    }

    fn loader(paths: &Paths, version: [u32; 3]) {
        fs::create_dir_all(paths.tl_dir()).unwrap();
        for (name, content) in loader_files(version) {
            fs::write(paths.tl_dir().join(name), content).unwrap();
        }
    }

    fn loader_zip(version: [u32; 3]) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(&mut cursor);
        for (name, content) in loader_files(version) {
            writer
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(&content).unwrap();
        }
        drop(writer);
        cursor.into_inner()
    }

    fn season(library: &Path, name: &str, version: [u32; 3]) -> PathBuf {
        let folder = library.join(name);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(TL_LAUNCHER), launcher_exe(version)).unwrap();
        folder
    }

    fn hm_version(folder: &Path, version: &str) {
        fs::create_dir_all(folder.join("HeatedMetal")).unwrap();
        fs::write(folder.join("HeatedMetal").join(".version"), version).unwrap();
    }

    fn notes(body: &str, url: &str, date: &str) -> ReleaseNotes {
        ReleaseNotes {
            body: body.to_owned(),
            url: url.to_owned(),
            date: date.to_owned(),
        }
    }

    fn entry(tag: &str, assets: &[&str]) -> Value {
        let assets: Vec<Value> = assets.iter().map(|name| json!({"name": name})).collect();
        json!({"tag_name": tag, "prerelease": false, "assets": assets})
    }

    fn parsed(entries: Value) -> Vec<Listed> {
        serde_json::from_value(entries).unwrap()
    }

    fn lists(paths: &Paths, lists: [Value; 5]) {
        let urls = Component::ALL.map(|component| {
            let url = component.release_url();
            url.strip_suffix("/latest").unwrap()
        });
        let answers: Vec<(&str, Value)> = urls.into_iter().zip(lists).collect();
        seed(paths, &answers);
    }

    #[test]
    fn versions_compare_as_numbers_and_otherwise_by_inequality() {
        assert!(newer("0.2.10", "0.2.9"));
        assert!(newer("0.3", "0.2.1"));
        assert!(newer("0.2.1.0", "0.2.1"));
        assert!(!newer("0.2.1", "0.2.1"));
        assert!(!newer("0.2.0", "0.2.1"));
        assert!(newer("2026-11-01", "0.2.1"));
        assert!(newer("0.2.0", "2026-11-01"));
        assert!(!newer("2026-11-01", "2026-11-01"));
    }

    #[test]
    fn the_oldest_copy_counts_and_an_unknown_one_hides_the_version() {
        let oldest = |versions: &[Option<&str>]| {
            super::oldest(versions.iter().map(|version| version.map(str::to_owned)))
        };
        assert_eq!(
            oldest(&[Some("0.1.10"), Some("0.1.9"), Some("0.2.0")]).as_deref(),
            Some("0.1.9")
        );
        assert_eq!(oldest(&[Some("0.1.9"), None]), None);
        assert_eq!(oldest(&[]), None);
        assert_eq!(
            oldest(&[Some("nightly"), Some("0.3.10")]).as_deref(),
            Some("0.3.10")
        );
    }

    #[cfg(unix)]
    #[test]
    fn tool_versions_are_read_from_what_the_tool_prints() {
        let printed = |pattern: &Regex, output: &str| {
            let script = format!("printf '%s\\n' '{output}'");
            tool_version(Path::new("sh"), &["-c", &script], pattern)
        };
        assert_eq!(
            printed(&DD_VERSION, "DepotDownloader v3.4.0 - Runtime: .NET 9.0.9").as_deref(),
            Some("3.4.0")
        );
        assert_eq!(printed(&DD_VERSION, "DepotDownloader 3.4.0"), None);
        for (output, version) in [
            (
                "7-Zip (z) 26.03 (x64) : Copyright (c) 1999-2026 Igor Pavlov",
                "26.03",
            ),
            (
                "7-Zip (r) 25.01 (x86) : Igor Pavlov : Public domain",
                "25.01",
            ),
            ("7-Zip 24.09 (x64)", "24.09"),
        ] {
            assert_eq!(printed(&SEVENZ_VERSION, output).as_deref(), Some(version));
        }
        assert_eq!(printed(&SEVENZ_VERSION, "p7zip Version 16.02"), None);
        let missing = Path::new("/nonexistent/7zz");
        assert_eq!(tool_version(missing, &[], &SEVENZ_VERSION), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_tool_that_hangs_is_stopped_after_the_timeout() {
        let started = std::time::Instant::now();
        let output = tool_output(
            Path::new("sh"),
            &["-c", "echo 'v1.2'; exec sleep 30"],
            Duration::from_millis(200),
        );
        assert_eq!(output, None);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn components_keep_their_order_names_and_repositories() {
        let listed = Component::ALL.map(|component| {
            (
                component.name(),
                component.repository(),
                component.restarts(),
            )
        });
        assert_eq!(
            listed,
            [
                ("Throwback Launcher", "xeralin/ThrowbackLauncher", true),
                ("DepotDownloader", "SteamRE/DepotDownloader", false),
                ("7z", "ip7z/7zip", false),
                ("ThrowbackLoader", "xeralin/ThrowbackLoader", false),
                ("Heated Metal", "DataCluster0/HeatedMetal", false),
            ]
        );
    }

    #[test]
    fn release_notes_take_missing_and_empty_fields_as_empty() {
        let full = json!({
            "tag_name": "v0.3.0",
            "prerelease": false,
            "body": "## Fixed\n- Play",
            "html_url": "https://github.com/xeralin/ThrowbackLauncher/releases/tag/v0.3.0",
            "published_at": "2026-11-09T18:00:00Z",
            "assets": [],
        });
        assert_eq!(
            serde_json::from_value::<ReleaseNotes>(full).unwrap(),
            notes(
                "## Fixed\n- Play",
                "https://github.com/xeralin/ThrowbackLauncher/releases/tag/v0.3.0",
                "2026-11-09T18:00:00Z"
            )
        );
        let bare = json!({"tag_name": "v0.3.0", "body": null});
        assert_eq!(
            serde_json::from_value::<ReleaseNotes>(bare).unwrap(),
            ReleaseNotes::default()
        );
    }

    #[test]
    fn a_component_is_present_once_its_files_are() {
        let bench = bench();
        let (install, paths) = (&bench.install, &bench.install.paths);
        let present = || -> Vec<&str> {
            let present = Component::ALL
                .into_iter()
                .filter(|component| component.present(install));
            present.map(Component::name).collect()
        };
        assert!(present().is_empty());

        fs::write(paths.dd_bin(), "").unwrap();
        fs::write(paths.sevenz_bin(), "").unwrap();
        loader(paths, [0, 1, 8]);
        season(&install.libraries[0], "Y5S3_HeatedMetal", [0, 1, 8]);
        season(&install.libraries[0], "Y5S4_NeonDawn", [0, 1, 8]);
        fs::create_dir_all(install.libraries[1].join("Y5S4_HeatedMetal")).unwrap();
        assert_eq!(present(), ["DepotDownloader", "7z", "ThrowbackLoader"]);

        season(&install.libraries[1], "Y5S4_HeatedMetal", [0, 1, 8]);
        fs::remove_file(paths.tl_dir().join(TL_LOADERS[3])).unwrap();
        assert_eq!(present(), ["DepotDownloader", "7z", "Heated Metal"]);
    }

    #[test]
    fn the_loader_version_is_the_oldest_copy_in_any_installed_season() {
        let bench = bench();
        let install = &bench.install;
        let current = || Component::ThrowbackLoader.current(install);
        assert_eq!(current(), None);
        loader(&install.paths, [0, 1, 8]);
        assert_eq!(current().as_deref(), Some("0.1.8"));

        season(&install.libraries[0], "Y1S0_Vanilla", [0, 1, 10]);
        let old = season(&install.libraries[1], "Y5S4_HeatedMetal", [0, 1, 7]);
        fs::create_dir_all(install.libraries[1].join("Y2S1_VelvetShell")).unwrap();
        assert_eq!(current().as_deref(), Some("0.1.7"));

        fs::write(old.join(TL_LAUNCHER), "MZ").unwrap();
        assert_eq!(current(), None);
        assert_eq!(
            installed(install),
            [(Component::Launcher, VERSION.to_owned())]
        );
        fs::remove_dir_all(old).unwrap();
        assert_eq!(
            installed(install),
            [
                (Component::Launcher, VERSION.to_owned()),
                (Component::ThrowbackLoader, "0.1.8".to_owned())
            ]
        );
    }

    #[test]
    fn heated_metal_counts_the_installs_that_follow_the_latest_release() {
        let bench = bench();
        let install = &bench.install;
        let current = || Component::HeatedMetal.current(install);
        assert_eq!(current(), None);
        let failed = Component::HeatedMetal.apply(install, None);
        assert_eq!(
            failed.unwrap_err().to_string(),
            "Not installed as Heated Metal"
        );

        let pinned = season(&install.libraries[0], "Y5S3_HeatedMetal", [0, 1, 8]);
        hm_version(&pinned, "0.2.3");
        let first = season(&install.libraries[0], "Y5S4_HeatedMetal", [0, 1, 8]);
        let second = season(&install.libraries[1], "Y5S4_HeatedMetal", [0, 1, 8]);
        assert_eq!(hm_installs(install), [first.clone(), second.clone()]);
        assert_eq!(current(), None);

        hm_version(&first, "0.3.10\n");
        assert_eq!(current(), None);
        hm_version(&second, "0.3.9");
        assert_eq!(current().as_deref(), Some("0.3.9"));
        hm_version(&second, "nightly");
        assert_eq!(current().as_deref(), Some("0.3.10"));
    }

    #[test]
    fn the_newest_version_comes_from_the_newest_published_release() {
        let latest = |component: Component, entries: Value| {
            let found = component.latest(&parsed(entries));
            found.ok().map(|(version, _)| version)
        };
        let program = ["Runtime.zip", PROGRAM_ASSET];
        let sevenz = format!("7z2603-{SEVENZ_ASSET}");
        let early = json!({"tag_name": "0.4.3-1", "prerelease": true, "assets": [{"name": "HeatedMetal.7z"}]});
        assert_eq!(
            latest(Component::Launcher, json!([entry("v0.3.0", &program)])).as_deref(),
            Some("0.3.0")
        );
        assert_eq!(
            latest(
                Component::DepotDownloader,
                json!([entry("DepotDownloader_3.4.0", &[DD_ASSET])])
            )
            .as_deref(),
            Some("3.4.0")
        );
        assert_eq!(
            latest(Component::SevenZip, json!([entry("26.03", &[&sevenz])])).as_deref(),
            Some("26.03")
        );
        assert_eq!(
            latest(
                Component::ThrowbackLoader,
                json!([entry("v0.1.9", &["ThrowbackLoader-0.1.9.zip"])])
            )
            .as_deref(),
            Some("0.1.9")
        );
        assert_eq!(
            latest(
                Component::HeatedMetal,
                json!([early, entry("0.4.2.2", &["HeatedMetal.7z"])])
            )
            .as_deref(),
            Some("0.4.2.2")
        );

        let missing = [
            (Component::Launcher, entry("v0.3.0", &["Runtime.zip"])),
            (Component::Launcher, entry("v0.3.0-rc1", &program)),
            (
                Component::DepotDownloader,
                entry("DepotDownloader_3.4.0", &[]),
            ),
            (Component::SevenZip, entry("26.03", &["7z2603-mac.tar.xz"])),
            (
                Component::ThrowbackLoader,
                entry("nightly", &["ThrowbackLoader.zip"]),
            ),
            (Component::ThrowbackLoader, entry("v0.1.9", &[])),
            (Component::HeatedMetal, entry("0.4.2.2", &[])),
        ];
        for (component, release) in missing {
            assert_eq!(
                latest(component, json!([release])),
                None,
                "{}",
                component.name()
            );
        }
        assert_eq!(latest(Component::SevenZip, json!([])), None);
    }

    #[test]
    fn a_check_lists_what_is_newer_or_unknown_together_with_a_failure() {
        let limit = |minutes| Err(RateLimitError { minutes }.into());
        let found = |entries: Value, current: Option<&str>| {
            Ok((parsed(entries), current.map(str::to_owned)))
        };
        let mut loader_release = entry("v0.1.9", &["ThrowbackLoader-0.1.9.zip"]);
        loader_release["body"] = "## Changed".into();
        loader_release["html_url"] = "https://github.com/loader".into();
        loader_release["published_at"] = "2026-11-10T08:00:00Z".into();
        let mut depot_release = entry("DepotDownloader_3.4.0", &[DD_ASSET]);
        depot_release["body"] = "".into();
        depot_release["html_url"] = "https://github.com/depot".into();
        let installed = json!([{
            "tag_name": format!("v{VERSION}"),
            "prerelease": false,
            "assets": [{"name": PROGRAM_ASSET}],
        }]);
        let probes: [Probe; 5] = [
            (Component::Launcher, true, found(installed, Some(VERSION))),
            (
                Component::DepotDownloader,
                true,
                found(json!([depot_release]), None),
            ),
            (Component::SevenZip, true, Err(anyhow!("connection reset"))),
            (
                Component::ThrowbackLoader,
                true,
                found(json!([loader_release]), Some("0.1.8")),
            ),
            (Component::HeatedMetal, true, limit(2)),
        ];
        let check = assess(probes.into_iter());
        assert_eq!(
            check.pending,
            [
                Pending {
                    component: Component::DepotDownloader,
                    target: "3.4.0".to_owned(),
                    notes: notes("", "https://github.com/depot", ""),
                },
                Pending {
                    component: Component::ThrowbackLoader,
                    target: "0.1.9".to_owned(),
                    notes: notes(
                        "## Changed",
                        "https://github.com/loader",
                        "2026-11-10T08:00:00Z"
                    ),
                },
            ]
        );
        assert_eq!(
            check.failure,
            Some(Failure::RateLimit(
                "GitHub rate limit reached, try again in 2 minutes".to_owned()
            ))
        );
        assert_eq!(take_recorded(), ["7z check failed - connection reset"]);
        let versions: Vec<(&str, Vec<&str>)> = check
            .history
            .iter()
            .map(|(component, releases)| {
                let listed = releases.iter().map(|release| release.version.as_str());
                (component.name(), listed.collect())
            })
            .collect();
        assert_eq!(
            versions,
            [
                ("Throwback Launcher", vec![VERSION]),
                ("DepotDownloader", vec!["3.4.0"]),
                ("7z", vec![]),
                ("ThrowbackLoader", vec![]),
                ("Heated Metal", vec![]),
            ]
        );
    }

    #[test]
    fn the_first_rate_limit_outranks_other_failures_and_old_releases_are_skipped() {
        let limit = |minutes| Err(RateLimitError { minutes }.into());
        let failed = || Err(anyhow!("timed out"));
        let loader = |tag: &str| {
            let release = entry(tag, &["ThrowbackLoader.zip"]);
            Ok((parsed(json!([release])), Some("0.1.8".to_owned())))
        };
        let outcome = |probes: Vec<Probe>| {
            let check = assess(probes.into_iter());
            (check.pending, check.failure)
        };

        let probes = vec![
            (Component::Launcher, true, failed()),
            (Component::DepotDownloader, true, limit(3)),
            (Component::SevenZip, true, failed()),
            (Component::ThrowbackLoader, true, loader("v0.1.8")),
            (Component::HeatedMetal, true, limit(1)),
        ];
        assert_eq!(
            outcome(probes),
            (
                Vec::new(),
                Some(Failure::RateLimit(
                    "GitHub rate limit reached, try again in 3 minutes".to_owned()
                ))
            )
        );
        assert_eq!(
            take_recorded(),
            [
                "Throwback Launcher check failed - timed out",
                "7z check failed - timed out"
            ]
        );

        let probes = vec![
            (Component::Launcher, true, failed()),
            (Component::ThrowbackLoader, true, loader("v0.1.7")),
        ];
        assert_eq!(outcome(probes), (Vec::new(), Some(Failure::Error)));
        let probes = vec![(Component::ThrowbackLoader, true, loader("v0.1.8"))];
        assert_eq!(outcome(probes), (Vec::new(), None));
        assert_eq!(outcome(Vec::new()), (Vec::new(), None));
        let probes = vec![(Component::HeatedMetal, false, failed())];
        assert_eq!(outcome(probes), (Vec::new(), None));
        assert_eq!(
            take_recorded(),
            [
                "Throwback Launcher check failed - timed out",
                "Heated Metal check failed - timed out"
            ]
        );
    }

    #[test]
    fn a_check_asks_once_per_component_and_lists_the_updates_of_present_ones() {
        let bench = bench();
        let install = &bench.install;
        let paths = &install.paths;
        fs::write(paths.dd_bin(), "").unwrap();
        loader(paths, [0, 1, 8]);
        let offer = |loader_tag: &str| {
            let mut loader_release = entry(loader_tag, &["ThrowbackLoader.zip"]);
            loader_release["body"] = "## Changed".into();
            let depot = json!([entry("DepotDownloader_3.4.0", &[DD_ASSET])]);
            let none = || json!([]);
            lists(
                paths,
                [none(), depot, none(), json!([loader_release]), none()],
            );
        };
        let listed = |pending: Vec<Pending>| -> Vec<(&str, String, String)> {
            let updates = pending.into_iter();
            updates
                .map(|update| (update.component.name(), update.target, update.notes.body))
                .collect()
        };
        let depot = || ("DepotDownloader", "3.4.0".to_owned(), String::new());

        offer("v0.1.9");
        let checked = check(install, false);
        let loader = (
            "ThrowbackLoader",
            "0.1.9".to_owned(),
            "## Changed".to_owned(),
        );
        assert_eq!(listed(checked.pending), [depot(), loader]);
        assert_eq!(checked.failure, None);

        offer("v0.1.8");
        let checked = check(install, false);
        assert_eq!(listed(checked.pending), [depot()]);
        assert_eq!(checked.failure, None);
        assert!(take_recorded().is_empty());

        offer("nightly");
        let checked = check(install, false);
        assert_eq!(listed(checked.pending), [depot()]);
        assert_eq!(checked.failure, Some(Failure::Error));
        let recorded = take_recorded();
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert!(recorded[0].starts_with("ThrowbackLoader check failed - "));
    }

    #[test]
    fn a_forced_check_stops_when_the_cached_answers_cannot_be_dropped() {
        let bench = bench();
        let install = &bench.install;
        fs::create_dir(install.paths.api_cache_file()).unwrap();
        let failed = Check {
            failure: Some(Failure::Error),
            ..Check::default()
        };
        assert_eq!(check(install, true), failed);
        let recorded = take_recorded();
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert!(recorded[0].starts_with("Update check failed - "));
    }

    #[test]
    fn the_history_lists_published_releases_up_to_the_installed_version() {
        let bench = bench();
        let install = &bench.install;
        let listed = |tag: &str, prerelease: bool| {
            json!({
                "tag_name": tag,
                "prerelease": prerelease,
                "body": format!("notes {tag}"),
                "html_url": format!("https://github.com/release/{tag}"),
                "published_at": "2026-11-01T10:00:00Z",
            })
        };
        let installed = format!("v{VERSION}");
        lists(
            &install.paths,
            [
                json!([
                    listed("v99.0.0", false),
                    listed(&installed, false),
                    listed("v0.1.5", true),
                    listed("v0.1.0", false),
                ]),
                json!([listed("DepotDownloader_3.4.0", false)]),
                json!({"message": "Not Found"}),
                json!([]),
                json!([listed("0.3.1", false), {"tag_name": "0.3.0", "prerelease": false}]),
            ],
        );

        let kept = |tag: &str, version: &str| Release {
            version: version.to_owned(),
            notes: notes(
                &format!("notes {tag}"),
                &format!("https://github.com/release/{tag}"),
                "2026-11-01T10:00:00Z",
            ),
        };
        let bare = Release {
            version: "0.3.0".to_owned(),
            notes: ReleaseNotes::default(),
        };
        let checked = check(install, false);
        assert_eq!(
            checked.history,
            [
                (
                    Component::Launcher,
                    vec![kept(&installed, VERSION), kept("v0.1.0", "0.1.0")]
                ),
                (
                    Component::DepotDownloader,
                    vec![kept("DepotDownloader_3.4.0", "3.4.0")]
                ),
                (Component::SevenZip, Vec::new()),
                (Component::ThrowbackLoader, Vec::new()),
                (Component::HeatedMetal, vec![kept("0.3.1", "0.3.1"), bare]),
            ]
        );
        assert!(checked.pending.is_empty());
        assert_eq!(checked.failure, None);
        let recorded = take_recorded();
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert!(recorded[0].starts_with("7z check failed - "));
    }

    #[test]
    fn the_launcher_is_downloaded_next_to_the_program_and_swapped_in() {
        let bench = bench();
        let paths = &bench.install.paths;
        let server = Server::start(|request| match request.path.as_str() {
            "/program" => Reply::new(200, vec![7; 100_000]),
            "/limited" => Reply::rate_limit(),
            _ => Reply::new(404, []),
        });
        let offer = |path: &str| {
            let assets = [
                ("Runtime.zip", "unused"),
                (PROGRAM_ASSET, &*server.url(path)),
            ];
            seed(paths, &[(UPDATE_API_URL, release("v99.0.0", &assets))]);
        };
        let program = bench.dir.path().join(PROGRAM_ASSET);
        fs::write(&program, "old").unwrap();
        let progress = Arc::new(Mutex::new(Vec::new()));
        let sink = progress.clone();
        let report = move |fraction: f64| sink.lock().unwrap().push(fraction);
        let update = || update_launcher(paths, Some(program.clone()), Some(&report));
        let files = || names(bench.dir.path());

        offer("/gone");
        assert_eq!(
            update().unwrap_err().to_string(),
            "Throwback Launcher update failed"
        );
        offer("/limited");
        let limited = update().unwrap_err();
        assert!(limited.is::<RateLimitError>());
        assert_eq!(
            limited.to_string(),
            "GitHub rate limit reached, try again in 2 minutes"
        );
        assert_eq!(
            update_launcher(paths, None, Some(&report))
                .unwrap_err()
                .to_string(),
            "Throwback Launcher update failed"
        );
        assert_eq!(fs::read(&program).unwrap(), b"old");
        assert_eq!(files(), [PROGRAM_ASSET, "data"]);
        assert_eq!(take_outcome(paths), None);
        assert_eq!(take_recorded().len(), 2);

        offer("/program");
        update().unwrap();
        assert_eq!(fs::read(&program).unwrap(), vec![7; 100_000]);
        #[cfg(windows)]
        assert_eq!(files(), [PROGRAM_ASSET, "ThrowbackLauncher.old", "data"]);
        #[cfg(not(windows))]
        assert_eq!(files(), [PROGRAM_ASSET, "data"]);
        assert_eq!(take_outcome(paths), Some(true));
        assert_eq!(progress.lock().unwrap().last(), Some(&1.0));
        assert!(take_recorded().is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&program).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);

            fs::remove_file(&program).unwrap();
            fs::create_dir(&program).unwrap();
            fs::write(program.join("kept"), "").unwrap();
            assert!(update().is_err());
            assert_eq!(files(), [PROGRAM_ASSET, "data"]);
            assert_eq!(take_outcome(paths), None);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_restart_starts_the_program_with_the_relaunch_argument_alone() {
        use std::io::ErrorKind;
        use std::os::unix::fs::PermissionsExt;
        use std::time::Instant;

        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("launcher");
        let script = r#"#!/bin/sh
printf '%s\n' "$@" > "$0.part" && mv "$0.part" "$0.arguments"
"#;
        fs::write(&program, script).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        while let Err(error) = start(&program) {
            let kind = error.downcast_ref().map(std::io::Error::kind);
            assert_eq!(kind, Some(ErrorKind::ExecutableFileBusy), "{error:#}");
            thread::sleep(Duration::from_millis(5));
        }
        let arguments = dir.path().join("launcher.arguments");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !arguments.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(fs::read_to_string(arguments).unwrap(), "--relaunch\n");
    }

    #[test]
    fn a_loader_update_reaches_every_installed_season_and_reports_the_last_failure() {
        let bench = bench();
        let install = &bench.install;
        let paths = &install.paths;
        let archive = loader_zip([0, 1, 9]);
        let server = Server::start(move |_| Reply::new(200, archive.clone()));
        let asset = [("ThrowbackLoader.zip", &*server.url("/loader.zip"))];
        seed(paths, &[(TL_API_URL, release("v0.1.9", &asset))]);
        loader(paths, [0, 1, 8]);
        let broken = season(&install.libraries[0], "Y1S0_Vanilla", [0, 1, 8]);
        let intact = season(&install.libraries[1], "Y5S4_HeatedMetal", [0, 1, 8]);
        fs::create_dir(broken.join(TL_TOML)).unwrap();
        let apply = || Component::ThrowbackLoader.apply(install, None);

        assert_eq!(
            apply().unwrap_err().to_string(),
            "ThrowbackLoader setup failed"
        );
        let recorded = take_recorded();
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert!(recorded[0].starts_with("ThrowbackLoader setup failed - "));
        assert_eq!(tl_version(paths).as_deref(), Some("0.1.9"));
        assert_eq!(tl_folder_version(&intact).as_deref(), Some("0.1.9"));
        assert_eq!(tl_folder_version(&broken).as_deref(), Some("0.1.8"));
        let config = fs::read_to_string(intact.join(TL_TOML)).unwrap();
        assert!(config.contains("username = 'Ash'"), "{config}");

        fs::remove_dir(broken.join(TL_TOML)).unwrap();
        apply().unwrap();
        assert_eq!(
            Component::ThrowbackLoader.current(install).as_deref(),
            Some("0.1.9")
        );
        assert!(take_recorded().is_empty());
    }
}
