use std::ffi::OsString;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError};

use anyhow::Result;
use regex::Regex;
use zip::ZipArchive;

use crate::env::{DD_API_URL, DD_ASSET, DD_MEMBER, GIB, Paths, user_data_base};
use crate::github::{Progress, RateLimitError, download_error, fetch_to, github_asset};
use crate::heatedmetal::HmFilesRemovedError;
use crate::log::fail;
use crate::manifest::Download;

pub const STAGING_HEADROOM: u64 = 2 * GIB;
pub const LOGIN_FAILED: &str = "Steam login failed";

const TAIL_LINES: usize = 8;

static FETCHING: Mutex<()> = Mutex::new(());

static PERCENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(\d+(?:\.\d+)?)\s*%").expect("pattern"));

#[derive(Clone, Debug, PartialEq)]
pub struct DepotCommand {
    pub args: Vec<OsString>,
    pub name: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginKind {
    Account,
    Password,
    Guard,
    GuardEmail,
    GuardMobile,
}

fn unpack(zip_path: &Path, part: &Path, bin: &Path) -> Result<()> {
    let mut archive = ZipArchive::new(File::open(zip_path)?)?;
    let mut source = archive.by_name(DD_MEMBER)?;
    io::copy(&mut source, &mut File::create(part)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(part)?.permissions().mode();
        fs::set_permissions(part, fs::Permissions::from_mode(mode | 0o111))?;
    }
    Ok(fs::rename(part, bin)?)
}

fn download(
    paths: &Paths,
    progress: Progress,
    zip_path: &Path,
    part: &Path,
    bin: &Path,
) -> Result<()> {
    let (_, asset_url) = github_asset(paths, DD_API_URL, DD_ASSET)?;
    fetch_to(&asset_url, zip_path, progress, None)?;
    unpack(zip_path, part, bin)
}

pub fn ensure_depotdownloader(paths: &Paths, progress: Progress, force: bool) -> Result<PathBuf> {
    let _fetching = FETCHING.lock().unwrap_or_else(PoisonError::into_inner);
    let bin = paths.dd_bin();
    if bin.exists() && !force {
        return Ok(bin);
    }
    let part = bin.with_added_extension("part");
    let zip_path = paths.dd_zip();
    let outcome = download(paths, progress, &zip_path, &part, &bin)
        .map_err(|error| download_error("DepotDownloader download failed", error));
    let _ = fs::remove_file(&part);
    let _ = fs::remove_file(&zip_path);
    outcome?;
    Ok(bin)
}

pub fn depot_commands(
    download: &Download,
    steam_account: &str,
    target: &Path,
    max_downloads: u32,
) -> [DepotCommand; 2] {
    let depots = [
        (download.depot_ww, &download.manifest_ww, "Worldwide"),
        (
            download.depot_content,
            &download.manifest_content,
            "Content",
        ),
    ];
    depots.map(|(depot_id, manifest_id, name)| DepotCommand {
        args: vec![
            "-depot".into(),
            depot_id.to_string().into(),
            "-manifest".into(),
            manifest_id.into(),
            "-app".into(),
            download.app.to_string().into(),
            "-username".into(),
            steam_account.into(),
            "-remember-password".into(),
            "-dir".into(),
            target.into(),
            "-validate".into(),
            "-max-downloads".into(),
            max_downloads.to_string().into(),
        ],
        name,
    })
}

pub fn qr_login(download: &Download, folder: &Path) -> Vec<OsString> {
    vec![
        "-depot".into(),
        download.depot_ww.to_string().into(),
        "-manifest".into(),
        (&download.manifest_ww).into(),
        "-app".into(),
        download.app.to_string().into(),
        "-qr".into(),
        "-remember-password".into(),
        "-manifest-only".into(),
        "-dir".into(),
        folder.into(),
    ]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QrCode {
    size: usize,
    dark: Vec<bool>,
}

impl QrCode {
    pub fn size(&self) -> usize {
        self.size
    }

    pub fn dark(&self, x: usize, y: usize) -> bool {
        self.dark[y * self.size + x]
    }
}

#[derive(Default)]
pub struct QrReader {
    rows: Option<Vec<Vec<bool>>>,
}

impl QrReader {
    pub fn read(&mut self, line: &str) -> Option<QrCode> {
        if line.contains("sign in with this QR code") {
            self.rows = Some(Vec::new());
            return None;
        }
        let rows = self.rows.as_mut()?;
        let cells: Vec<char> = line.chars().collect();
        let size = rows.first().map_or(cells.len() / 2, Vec::len);
        let Some(row) = modules(&cells, size) else {
            self.rows = None;
            return None;
        };
        rows.push(row);
        if rows.len() < size {
            return None;
        }
        let rows = self.rows.take()?;
        let quiet = rows.iter().take_while(|row| !row.contains(&true)).count();
        let size = size.checked_sub(2 * quiet).filter(|size| *size > 0)?;
        let kept = rows[quiet..quiet + size].iter();
        let dark = kept
            .flat_map(|row| &row[quiet..quiet + size])
            .copied()
            .collect();
        Some(QrCode { size, dark })
    }
}

fn modules(cells: &[char], size: usize) -> Option<Vec<bool>> {
    let blank = cells.iter().filter(|cell| **cell == ' ').count();
    let dark = size.checked_sub(blank / 2)?;
    let block = (cells.len() - blank).checked_div(2 * dark).unwrap_or(1);
    let mut row = Vec::with_capacity(size);
    let mut rest = cells;
    while let [first, ..] = rest {
        let lit = *first == ' ';
        let wide = if lit { 2 } else { 2 * block };
        let (module, tail) = rest.split_at_checked(wide)?;
        if wide == 0 || module.iter().any(|cell| (*cell == ' ') != lit) {
            return None;
        }
        row.push(!lit);
        rest = tail;
    }
    (size > 0 && row.len() == size).then_some(row)
}

pub fn qr_account(line: &str) -> Option<&str> {
    let (_, rest) = line.split_once("login with -username ")?;
    rest.split_once(' ').map(|(name, _)| name)
}

pub fn logged_on(line: &str) -> bool {
    line.trim() == "Done!"
}

#[derive(Debug, PartialEq, Eq)]
pub enum Fault {
    RateLimited(String),
    FilesMissing,
    Message(String),
}

impl From<anyhow::Error> for Fault {
    fn from(error: anyhow::Error) -> Self {
        if error.is::<RateLimitError>() {
            Self::RateLimited(error.to_string())
        } else if error.is::<HmFilesRemovedError>() {
            Self::FilesMissing
        } else {
            Self::Message(error.to_string())
        }
    }
}

pub fn take_lines(buffer: &mut Vec<u8>) -> Vec<String> {
    let Some(end) = buffer.iter().rposition(|byte| *byte == b'\n') else {
        return Vec::new();
    };
    let complete: Vec<u8> = buffer.drain(..=end).collect();
    complete[..end]
        .split(|byte| *byte == b'\n')
        .map(|line| {
            String::from_utf8_lossy(line)
                .trim_end_matches('\r')
                .to_owned()
        })
        .collect()
}

pub fn percent(line: &str) -> Option<f64> {
    let value: f64 = PERCENT.captures(line)?.get(1)?.as_str().parse().ok()?;
    Some(value.min(100.0))
}

pub fn output_error(line: &str) -> Option<&'static str> {
    let holds = |texts: &[&str]| texts.iter().any(|text| line.contains(text));
    let login = holds(&[
        "Failed to authenticate with Steam",
        "Unable to login to Steam3",
    ]);
    if line.contains("Could not connect to Steam")
        || (login && holds(&["ServiceUnavailable", "NoConnection", "Timeout"]))
    {
        return Some("Could not connect to Steam");
    }
    if login && holds(&["RateLimitExceeded", "AccountLoginDeniedThrottle"]) {
        return Some("Too many login attempts");
    }
    if holds(&[
        "InvalidPassword",
        "Access token was rejected",
        "Failed to poll status",
        "Failed to send steam guard code",
    ]) {
        return Some(LOGIN_FAILED);
    }
    if line.contains("being used by another process") && !line.starts_with("Encountered") {
        return Some("A file is already in use");
    }
    if line.ends_with("is not available from this account.") {
        return Some(if line.starts_with("App ") {
            "You do not own R6S on Steam"
        } else {
            "Your R6S license is not supported"
        });
    }
    None
}

pub fn mobile_confirmation(line: &str) -> bool {
    line.to_lowercase().contains("steam mobile app")
}

pub fn prompt(tail: &str) -> Option<LoginKind> {
    let low = tail.trim_end().to_lowercase();
    if !low.ends_with(':') {
        return None;
    }
    let holds = |text: &str| low.contains(text);
    if holds("auth code") || holds("authentication code") || holds("steam guard") {
        Some(if holds("email") {
            LoginKind::GuardEmail
        } else {
            LoginKind::Guard
        })
    } else if holds("password") {
        Some(LoginKind::Password)
    } else {
        None
    }
}

pub fn output_tail<'a>(lines: impl DoubleEndedIterator<Item = &'a str>) -> String {
    let frame = |line: &&str| {
        let line = line.trim_start();
        line.starts_with("at ") || line.starts_with("--- End of")
    };
    let mut tail: Vec<&str> = lines
        .rev()
        .filter(|line| !frame(line))
        .take(TAIL_LINES)
        .collect();
    if tail.is_empty() {
        return "no output".to_owned();
    }
    tail.reverse();
    tail.join("\n")
}

pub fn disk_shortfall(size_gb: f64, present: u64, free: u64) -> Option<f64> {
    let gib = GIB as f64;
    let missing = (size_gb * gib - present as f64).max(0.0);
    let short = missing + STAGING_HEADROOM as f64 - free as f64;
    (short > 0.0).then(|| (short / gib * 10.0).ceil() / 10.0)
}

#[cfg(unix)]
pub fn free_space(folder: &Path) -> io::Result<u64> {
    use std::os::unix::ffi::OsStrExt;

    let path = std::ffi::CString::new(folder.as_os_str().as_bytes())?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let stats = unsafe { stats.assume_init() };
    Ok(stats.f_bavail.saturating_mul(stats.f_frsize))
}

#[cfg(windows)]
pub fn free_space(folder: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let path: Vec<u16> = folder.as_os_str().encode_wide().chain([0]).collect();
    let mut free = 0;
    let (caller, total) = (std::ptr::null_mut(), std::ptr::null_mut());
    if unsafe { GetDiskFreeSpaceExW(path.as_ptr(), caller, total, &mut free) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(free)
}

fn find_stores(folder: &Path, stores: &mut Vec<PathBuf>) {
    let token = folder.join("AssemFiles").join("account.config");
    if token.symlink_metadata().is_ok() {
        return stores.push(folder.to_path_buf());
    }
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            find_stores(&entry.path(), stores);
        }
    }
}

fn wipe_stores(iso_dir: &Path) -> Result<(), String> {
    let mut stores = Vec::new();
    find_stores(iso_dir, &mut stores);
    let mut outcome = Ok(());
    for store in stores {
        if let Err(cause) = fs::remove_dir_all(store) {
            outcome = Err(fail("Steam token removal failed", cause));
        }
    }
    outcome
}

pub fn wipe_depot_token() -> Result<(), String> {
    wipe_stores(&user_data_base().join("IsolatedStorage"))
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use tempfile::TempDir;
    use zip::write::{SimpleFileOptions, ZipWriter};

    use super::*;
    use crate::github::testing::{Reply, Server, seed};
    use crate::log::take_recorded;
    use crate::throwbackloader::testing::names;

    fn paths(dir: &TempDir) -> Paths {
        Paths::new(dir.path().join("asset"), dir.path().join("data"))
    }

    fn archive(members: &[&str]) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(&mut cursor);
        for name in members {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer
                .write_all(format!("binary {name}").as_bytes())
                .unwrap();
        }
        drop(writer);
        cursor.into_inner()
    }

    fn seeded(paths: &Paths, asset_url: &str) {
        seed(paths, DD_API_URL, &[(DD_ASSET, asset_url)]);
    }

    fn part(paths: &Paths) -> PathBuf {
        paths.bin_dir().join(format!("{DD_MEMBER}.part"))
    }

    fn token(store: &Path) {
        fs::create_dir_all(store.join("AssemFiles")).unwrap();
        fs::write(store.join("AssemFiles").join("account.config"), "token").unwrap();
    }

    #[test]
    fn commands_cover_worldwide_then_content() {
        let download = Download {
            key: "Y1S0_Vanilla".to_string(),
            label: "Y1S0 Vanilla".to_string(),
            build: "8194013".to_string(),
            app: 359550,
            depot_content: 359551,
            depot_ww: 377237,
            manifest_content: "3893422760579204530".to_string(),
            manifest_ww: "8358812283631269928".to_string(),
            size_gb: 14.2,
            hm_version: None,
        };
        let target = Path::new("/games/R6 Library/Y1S0_Vanilla");
        let commands = depot_commands(&download, "steam user", target, 8);
        let args = |depot: &str, manifest: &str| -> Vec<OsString> {
            [
                "-depot",
                depot,
                "-manifest",
                manifest,
                "-app",
                "359550",
                "-username",
                "steam user",
                "-remember-password",
                "-dir",
                "/games/R6 Library/Y1S0_Vanilla",
                "-validate",
                "-max-downloads",
                "8",
            ]
            .map(OsString::from)
            .to_vec()
        };
        assert_eq!(
            commands,
            [
                DepotCommand {
                    args: args("377237", "8358812283631269928"),
                    name: "Worldwide",
                },
                DepotCommand {
                    args: args("359551", "3893422760579204530"),
                    name: "Content",
                },
            ]
        );
        let limit = depot_commands(&download, "", Path::new("x"), 100);
        assert_eq!(limit[1].args[13], "100");
        assert_eq!(limit[0].args[7], "");
    }

    #[test]
    fn errors_are_sorted_into_rate_limit_missing_files_and_text() {
        let limited = anyhow::Error::from(RateLimitError { minutes: 2 });
        assert_eq!(
            Fault::from(limited),
            Fault::RateLimited("GitHub rate limit reached, try again in 2 minutes".to_owned())
        );
        let later = download_error("ignored", RateLimitError { minutes: 0 }.into());
        assert_eq!(
            Fault::from(later),
            Fault::RateLimited("GitHub rate limit reached, try again later".to_owned())
        );
        assert_eq!(
            Fault::from(anyhow::Error::from(HmFilesRemovedError)),
            Fault::FilesMissing
        );
        let plain = anyhow::anyhow!("Heated Metal download failed").context("outer");
        assert_eq!(Fault::from(plain), Fault::Message("outer".to_owned()));
        assert!(take_recorded().is_empty());
    }

    #[test]
    fn complete_lines_leave_the_buffer_and_the_open_tail_stays() {
        let mut buffer = b" 12.5% a\r\n\r\nValidating\nEnter account pass".to_vec();
        assert_eq!(take_lines(&mut buffer), [" 12.5% a", "", "Validating"]);
        assert_eq!(buffer, b"Enter account pass");
        assert!(take_lines(&mut buffer).is_empty());
        buffer.extend(b"word for \"ash\": \xc3");
        assert!(take_lines(&mut buffer).is_empty());
        buffer.extend(b"\xa4\xff\r\r\n");
        assert_eq!(
            take_lines(&mut buffer),
            ["Enter account password for \"ash\": \u{e4}\u{fffd}"]
        );
        assert!(buffer.is_empty());
    }

    #[test]
    fn percent_is_read_from_the_line_start_and_capped() {
        for (line, value) in [
            (" 12.34% depots/377237/file", Some(12.34)),
            ("7%", Some(7.0)),
            ("\t 100.00 % done", Some(100.0)),
            ("250.5%", Some(100.0)),
            ("0.00% x", Some(0.0)),
            ("Got 12% of it", None),
            (".5%", None),
            ("12.%", None),
            ("12", None),
            ("", None),
        ] {
            assert_eq!(percent(line), value, "{line}");
        }
    }

    #[test]
    fn output_lines_are_mapped_to_the_six_messages() {
        for (line, message) in [
            (
                "Could not connect to Steam after 10 tries",
                Some("Could not connect to Steam"),
            ),
            (
                "Failed to authenticate with Steam: x with result ServiceUnavailable.",
                Some("Could not connect to Steam"),
            ),
            (
                "Unable to login to Steam3: NoConnection",
                Some("Could not connect to Steam"),
            ),
            (
                "Unable to login to Steam3: Timeout",
                Some("Could not connect to Steam"),
            ),
            (
                "Failed to authenticate with Steam: x with result RateLimitExceeded.",
                Some("Too many login attempts"),
            ),
            (
                "Unable to login to Steam3: AccountLoginDeniedThrottle",
                Some("Too many login attempts"),
            ),
            ("Got RateLimitExceeded from the content server", None),
            (
                "Failed to authenticate with Steam: x with result InvalidPassword.",
                Some(LOGIN_FAILED),
            ),
            ("Access token was rejected (Expired).", Some(LOGIN_FAILED)),
            (
                "Failed to authenticate with Steam: Failed to poll status with result Expired.",
                Some(LOGIN_FAILED),
            ),
            (
                "Failed to authenticate with Steam: Failed to send steam guard code with result x.",
                Some(LOGIN_FAILED),
            ),
            (
                "Failed to authenticate with Steam: x with result Fail.",
                None,
            ),
            (
                "The process cannot access the file 'a.forge' because it is being used by another process.",
                Some("A file is already in use"),
            ),
            (
                "Encountered error downloading chunk: it is being used by another process.",
                None,
            ),
            (
                "App 359550 is not available from this account.",
                Some("You do not own R6S on Steam"),
            ),
            (
                "Depot 377237 is not available from this account.",
                Some("Your R6S license is not supported"),
            ),
            ("Depot 377237 is not available from this account. x", None),
            (" 12.34% depots/377237/file", None),
        ] {
            assert_eq!(output_error(line), message, "{line}");
        }
    }

    #[test]
    fn prompts_are_told_apart_by_the_open_tail() {
        for (tail, kind) in [
            (
                "STEAM GUARD! Please enter your 2-factor auth code from your authenticator app: ",
                Some(LoginKind::Guard),
            ),
            (
                "STEAM GUARD! Please enter the auth code sent to the email at a@b.c: ",
                Some(LoginKind::GuardEmail),
            ),
            (
                "Please enter the authentication code:",
                Some(LoginKind::Guard),
            ),
            (
                "Enter account password for \"ash\": ",
                Some(LoginKind::Password),
            ),
            ("Enter account PASSWORD:\r\n", Some(LoginKind::Password)),
            ("Enter account password for \"ash\"", None),
            ("Connecting to Steam3:", None),
            ("Enter account password for \"ash\": x", None),
            ("", None),
        ] {
            assert_eq!(prompt(tail), kind, "{tail}");
        }
        assert!(mobile_confirmation(
            "STEAM GUARD! Use the Steam Mobile App to confirm your sign in"
        ));
        assert!(!mobile_confirmation("STEAM GUARD! Please enter the code"));
    }

    #[test]
    fn the_tail_drops_stack_frames_and_keeps_eight_lines() {
        assert_eq!(output_tail(std::iter::empty()), "no output");
        assert_eq!(
            output_tail(["   at DepotDownloader.Run()", "--- End of stack"].into_iter()),
            "no output"
        );
        let lines = [
            "one",
            "two",
            "Unhandled exception. System.IO.IOException: No space left on device",
            "   at System.IO.RandomAccess.WriteAtOffset()",
            "\tat DepotDownloader.ContentDownloader.Download()",
            "   --- End of inner exception stack trace ---",
            "three",
            "attic",
            "four",
            "five",
            "six",
            "seven",
        ];
        assert_eq!(
            output_tail(lines.into_iter()),
            "two\nUnhandled exception. System.IO.IOException: No space left on device\nthree\n\
             attic\nfour\nfive\nsix\nseven"
        );
    }

    #[test]
    fn the_shortfall_counts_what_is_missing_plus_the_headroom() {
        let gib = GIB;
        for (size_gb, present, free, shortfall) in [
            (14.2, 0, 100 * gib, None),
            (50.0, 0, 52 * gib, None),
            (50.0, 0, 52 * gib - 1, Some(0.1)),
            (50.0, 0, 0, Some(52.0)),
            (50.0, 40 * gib, 12 * gib, None),
            (50.0, 40 * gib, 11 * gib + gib / 2, Some(0.5)),
            (50.0, 60 * gib, 2 * gib, None),
            (50.0, 60 * gib, gib, Some(1.0)),
            (14.2, 0, 10 * gib, Some(6.2)),
            (14.2, gib / 4, 10 * gib, Some(6.0)),
            (0.05, 0, gib, Some(1.1)),
        ] {
            assert_eq!(
                disk_shortfall(size_gb, present, free),
                shortfall,
                "{size_gb} {present} {free}"
            );
        }
    }

    #[test]
    fn free_space_is_read_for_existing_folders_only() {
        let dir = tempfile::tempdir().unwrap();
        assert!(free_space(dir.path()).unwrap() > 0);
        assert!(free_space(&dir.path().join("missing")).is_err());
    }

    #[test]
    fn parallel_callers_share_one_download_of_the_binary() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(&dir);
        let server = Server::start(|_| Reply::new(200, archive(&[DD_MEMBER])));
        seeded(&paths, &server.url("/assets/1"));
        let bins: Vec<PathBuf> = std::thread::scope(|scope| {
            let calls: Vec<_> = (0..2)
                .map(|_| scope.spawn(|| ensure_depotdownloader(&paths, None, false).unwrap()))
                .collect();
            calls.into_iter().map(|call| call.join().unwrap()).collect()
        });
        assert_eq!(bins, [paths.dd_bin(), paths.dd_bin()]);
        assert_eq!(server.paths(), ["/assets/1"]);
    }

    #[test]
    fn ensure_downloads_the_binary_and_removes_the_archive() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(&dir);
        let server = Server::start(|_| Reply::new(200, archive(&["README.md", DD_MEMBER])));
        seeded(&paths, &server.url("/assets/1"));
        fs::write(part(&paths), "stale").unwrap();
        let bin = ensure_depotdownloader(&paths, None, false).unwrap();
        assert_eq!(bin, paths.dd_bin());
        assert_eq!(
            fs::read_to_string(&bin).unwrap(),
            format!("binary {DD_MEMBER}")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&bin).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }
        assert_eq!(names(&paths.bin_dir()), [DD_MEMBER, "api_cache.json"]);
        assert_eq!(server.paths(), ["/assets/1"]);
        fs::write(&bin, "edited").unwrap();
        ensure_depotdownloader(&paths, None, false).unwrap();
        assert_eq!(fs::read_to_string(&bin).unwrap(), "edited");
        assert_eq!(server.paths().len(), 1);
        ensure_depotdownloader(&paths, None, true).unwrap();
        assert_eq!(
            fs::read_to_string(&bin).unwrap(),
            format!("binary {DD_MEMBER}")
        );
        assert_eq!(names(&paths.bin_dir()), [DD_MEMBER, "api_cache.json"]);
        assert_eq!(server.paths().len(), 2);
        assert!(take_recorded().is_empty());
    }

    #[test]
    fn failed_downloads_remove_the_part_and_the_archive() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(&dir);
        let server = Server::start(|request| match request.path.as_str() {
            "/short" => Reply::new(200, archive(&["README.md", "DepotDownloader.dll"])),
            "/garbage" => Reply::new(200, b"not a zip".to_vec()),
            "/limited" => Reply::rate_limit(),
            _ => Reply::new(500, Vec::new()),
        });
        fs::create_dir_all(paths.bin_dir()).unwrap();
        fs::write(paths.dd_bin(), "old").unwrap();
        let attempt = |path: &str| {
            seeded(&paths, &server.url(path));
            fs::write(part(&paths), "stale").unwrap();
            fs::write(paths.dd_zip(), "stale").unwrap();
            let error = ensure_depotdownloader(&paths, None, true).unwrap_err();
            assert_eq!(names(&paths.bin_dir()), [DD_MEMBER, "api_cache.json"]);
            assert_eq!(fs::read_to_string(paths.dd_bin()).unwrap(), "old");
            error
        };
        assert_eq!(
            attempt("/short").to_string(),
            "DepotDownloader download failed"
        );
        let lines = take_recorded();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("DepotDownloader download failed - "),
            "{lines:?}"
        );
        assert_eq!(
            attempt("/garbage").to_string(),
            "DepotDownloader download failed"
        );
        assert_eq!(take_recorded().len(), 1);
        let limited = attempt("/limited");
        assert!(limited.is::<RateLimitError>());
        assert_eq!(
            limited.to_string(),
            "GitHub rate limit reached, try again in 2 minutes"
        );
        assert!(take_recorded().is_empty());
        assert_eq!(
            attempt("/broken").to_string(),
            "DepotDownloader download failed"
        );
        assert_eq!(take_recorded().len(), 1);
        assert_eq!(
            server.paths(),
            [
                "/short", "/garbage", "/limited", "/broken", "/broken", "/broken"
            ]
        );
    }

    #[test]
    fn token_stores_are_removed_at_any_depth() {
        let dir = tempfile::tempdir().unwrap();
        let iso = dir.path().join("IsolatedStorage");
        assert_eq!(wipe_stores(&iso), Ok(()));
        token(
            &iso.join("abc.def")
                .join("ghi.jkl")
                .join("Url.x")
                .join("Url.y"),
        );
        token(&iso.join("abc.def").join("other").join("Url.z"));
        token(
            &iso.join("abc.def")
                .join("other")
                .join("Url.z")
                .join("nested"),
        );
        let foreign = iso.join("abc.def").join("foreign").join("AssemFiles");
        fs::create_dir_all(&foreign).unwrap();
        fs::write(foreign.join("other.config"), "keep").unwrap();
        fs::create_dir_all(iso.join("AssemFiles")).unwrap();
        token(&iso.join(".hidden").join("store"));
        let folder_token = iso.join("folder").join("AssemFiles").join("account.config");
        fs::create_dir_all(&folder_token).unwrap();
        assert_eq!(wipe_stores(&iso), Ok(()));
        assert!(!iso.join(".hidden").join("store").exists());
        assert!(iso.join(".hidden").exists());
        assert!(!iso.join("folder").exists());
        assert!(
            !iso.join("abc.def")
                .join("ghi.jkl")
                .join("Url.x")
                .join("Url.y")
                .exists()
        );
        assert!(iso.join("abc.def").join("ghi.jkl").join("Url.x").exists());
        assert!(!iso.join("abc.def").join("other").join("Url.z").exists());
        assert!(iso.join("abc.def").join("other").exists());
        assert!(foreign.join("other.config").exists());
        assert!(iso.join("AssemFiles").exists());
        token(&iso);
        assert_eq!(wipe_stores(&iso), Ok(()));
        assert!(!iso.exists());
    }

    #[cfg(unix)]
    #[test]
    fn linked_folders_are_not_searched() {
        let dir = tempfile::tempdir().unwrap();
        let iso = dir.path().join("IsolatedStorage");
        let outside = dir.path().join("outside").join("store");
        token(&outside);
        fs::create_dir_all(&iso).unwrap();
        std::os::unix::fs::symlink(dir.path().join("outside"), iso.join("link")).unwrap();
        let dangling = iso.join("dangling").join("AssemFiles");
        fs::create_dir_all(&dangling).unwrap();
        std::os::unix::fs::symlink(dir.path().join("gone"), dangling.join("account.config"))
            .unwrap();
        assert_eq!(wipe_stores(&iso), Ok(()));
        assert!(outside.join("AssemFiles").join("account.config").exists());
        assert!(iso.join("link").exists());
        assert!(!iso.join("dangling").exists());
    }

    const QR_HEADER: &str = "Use the Steam Mobile App to sign in with this QR code:";

    fn drawn(code: &QrCode) -> Vec<bool> {
        let size = code.size();
        let cells = (0..size).flat_map(|y| (0..size).map(move |x| (x, y)));
        cells.map(|(x, y)| code.dark(x, y)).collect()
    }

    #[test]
    fn a_qr_code_is_read_from_the_block_characters_below_its_header() {
        let mut reader = QrReader::default();
        assert_eq!(reader.read("Logging in with QR code..."), None);
        assert_eq!(reader.read("██  ██"), None);
        assert_eq!(reader.read(QR_HEADER), None);
        assert_eq!(reader.read("██  ██"), None);
        assert_eq!(reader.read("  ██  "), None);
        let code = reader.read("██████").unwrap();
        assert_eq!(code.size(), 3);
        let expected = [true, false, true, false, true, false, true, true, true];
        assert_eq!(drawn(&code), expected);
        assert_eq!(reader.read("██████"), None);
    }

    #[test]
    fn a_qr_code_is_read_in_any_console_code_page_and_without_its_quiet_zone() {
        let blocks: [&[u8]; 5] = [
            "\u{2588}".as_bytes(),
            &[0xdb],
            b"?",
            &[0xa8, 0x80],
            &[0xa2, 0x69],
        ];
        for block in blocks {
            let encoded = |row: &str| {
                let bytes: Vec<u8> = row
                    .bytes()
                    .flat_map(|cell| match cell {
                        b' ' => vec![b' '],
                        _ => block.to_vec(),
                    })
                    .collect();
                String::from_utf8_lossy(&bytes).into_owned()
            };
            let mut reader = QrReader::default();
            reader.read(QR_HEADER);
            assert_eq!(reader.read(&encoded("        ")), None);
            assert_eq!(reader.read(&encoded("  ##    ")), None);
            assert_eq!(reader.read(&encoded("    ##  ")), None);
            let code = reader.read(&encoded("        ")).unwrap();
            assert_eq!(drawn(&code), [true, false, false, true], "{block:?}");
        }
    }

    #[test]
    fn a_qr_code_starts_over_on_a_new_header_and_after_a_broken_row() {
        let mut reader = QrReader::default();
        reader.read(QR_HEADER);
        assert_eq!(reader.read("\u{fffd}\u{fffd}  "), None);
        reader.read("The QR code has changed:");
        reader.read(QR_HEADER);
        assert_eq!(reader.read("??  "), None);
        let code = reader.read("  \u{fffd}\u{fffd}").unwrap();
        assert_eq!(drawn(&code), [true, false, false, true]);
        reader.read(QR_HEADER);
        assert_eq!(reader.read("██  "), None);
        assert_eq!(reader.read("██"), None);
        assert_eq!(reader.read("  ██"), None);
        assert_eq!(reader.read(""), None);
    }

    #[test]
    fn the_account_name_comes_from_the_line_after_a_qr_login() {
        let line = "Success! Next time you can login with -username ash_99 \
                    -remember-password instead of -qr.";
        assert_eq!(qr_account(line), Some("ash_99"));
        assert_eq!(qr_account("Logging 'ash' into Steam3..."), None);
        assert!(logged_on(" Done!"));
        assert!(!logged_on("Connecting to Steam3... Done!"));
    }

    #[test]
    fn a_qr_login_fetches_only_the_manifest_of_the_worldwide_depot() {
        let season = &crate::manifest::downloads()[0];
        let args = qr_login(season, Path::new("/tmp/qr"));
        let args: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into())
            .collect();
        let (depot, app) = (season.depot_ww.to_string(), season.app.to_string());
        let expected = [
            "-depot",
            depot.as_str(),
            "-manifest",
            season.manifest_ww.as_str(),
            "-app",
            app.as_str(),
            "-qr",
            "-remember-password",
            "-manifest-only",
            "-dir",
            "/tmp/qr",
        ];
        assert_eq!(args, expected);
    }
}
