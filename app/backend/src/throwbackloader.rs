use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::sync::LazyLock;

use anyhow::{Result, bail};
use regex::{NoExpand, Regex};
use toml::{Table, Value};
use zip::ZipArchive;

use crate::env::{
    DEFAULTARGS_DLL, HM_FOLDER_SUFFIX, IS_WINDOWS, Paths, TL_API_URL, TL_DLLS_COMMON, TL_LAUNCHER,
    TL_LOADERS, TL_TOML, copy_with_times,
};
use crate::github::{Progress, download_error, fetch_to, github_asset};

static USERNAME_RX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"username\s*=\s*["'][^"']*["']"#).expect("pattern"));

static TOOLS_RX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^tools\s*=\s*\[(?:[^\]'"]|'[^']*'|"(?:[^"\\]|\\.)*")*\][^\n]*\n?"#)
        .expect("pattern")
});

static LAUNCH_HEADER_RX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\[Launch\]\s*$").expect("pattern"));

fn literal(value: &str) -> String {
    if !value.contains('\'') && !value.chars().any(char::is_control) {
        return format!("'{value}'");
    }
    let mut quoted = String::from('"');
    for character in value.chars() {
        match character {
            '\\' | '"' => quoted.extend(['\\', character]),
            control if control.is_control() => {
                quoted.push_str(&format!("\\u{:04X}", u32::from(control)))
            }
            other => quoted.push(other),
        }
    }
    quoted + "\""
}

fn dll_names() -> impl Iterator<Item = &'static str> {
    TL_DLLS_COMMON.into_iter().chain(TL_LOADERS)
}

fn extract_names() -> impl Iterator<Item = &'static str> {
    dll_names().chain([TL_TOML, TL_LAUNCHER])
}

pub fn tl_present(folder: &Path) -> bool {
    extract_names().all(|name| folder.join(name).is_file())
}

fn launcher_version(data: &[u8]) -> Option<String> {
    let mut at = 0;
    loop {
        at += data
            .get(at..)?
            .windows(4)
            .position(|window| window == b"\xbd\x04\xef\xfe")?;
        if let Some(info) = data.get(at + 4..at + 16)
            && let [struc_version, ms, ls] = info.as_chunks::<4>().0
            && u32::from_le_bytes(*struc_version) == 0x0001_0000
        {
            let (ms, ls) = (u32::from_le_bytes(*ms), u32::from_le_bytes(*ls));
            return Some(format!("{}.{}.{}", ms >> 16, ms & 0xffff, ls >> 16));
        }
        at += 4;
    }
}

pub fn tl_folder_version(folder: &Path) -> Option<String> {
    launcher_version(&fs::read(folder.join(TL_LAUNCHER)).ok()?)
}

pub fn tl_version(paths: &Paths) -> Option<String> {
    tl_folder_version(&paths.tl_dir())
}

fn download(
    paths: &Paths,
    progress: Progress,
    folder: &Path,
    zip_path: &Path,
    tmp_dir: &Path,
) -> Result<()> {
    fs::create_dir_all(folder)?;
    let (_, asset_url) = github_asset(paths, TL_API_URL, ".zip")?;
    fetch_to(&asset_url, zip_path, progress, None)?;
    let _ = fs::remove_dir_all(tmp_dir);
    extract(zip_path, folder, tmp_dir)
}

fn extract(zip_path: &Path, folder: &Path, tmp_dir: &Path) -> Result<()> {
    let mut archive = ZipArchive::new(File::open(zip_path)?)?;
    for name in extract_names() {
        let mut source = archive.by_name(name)?;
        fs::create_dir_all(tmp_dir)?;
        io::copy(&mut source, &mut File::create(tmp_dir.join(name))?)?;
    }
    for name in extract_names() {
        fs::rename(tmp_dir.join(name), folder.join(name))?;
    }
    Ok(())
}

pub fn ensure_tl(paths: &Paths, progress: Progress, force: bool) -> Result<()> {
    let folder = paths.tl_dir();
    if tl_present(&folder) && !force {
        return Ok(());
    }
    let zip_path = folder.join("tl.zip");
    let tmp_dir = folder.join(".tl.tmp");
    let outcome = download(paths, progress, &folder, &zip_path, &tmp_dir)
        .map_err(|error| download_error("ThrowbackLoader download failed", error));
    let _ = fs::remove_file(&zip_path);
    let _ = fs::remove_dir_all(&tmp_dir);
    outcome
}

fn read_config(target_dir: &Path) -> Result<String> {
    let Ok(text) = String::from_utf8(fs::read(target_dir.join(TL_TOML))?) else {
        bail!("Config.toml is not valid UTF-8");
    };
    Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
}

fn write_config(target_dir: &Path, text: &str) -> io::Result<()> {
    let text = if IS_WINDOWS {
        text.replace('\n', "\r\n")
    } else {
        text.to_owned()
    };
    fs::write(target_dir.join(TL_TOML), text)
}

fn with_username(text: &str, username: &str) -> String {
    let line = format!("username = {}", literal(username));
    let text = USERNAME_RX.replacen(text, 1, NoExpand(&line));
    TOOLS_RX.replacen(&text, 1, NoExpand("")).into_owned()
}

pub fn write_tl_username(target_dir: &Path, username: &str) -> Result<()> {
    if !target_dir.join(TL_TOML).exists() {
        return Ok(());
    }
    let text = with_username(&read_config(target_dir)?, username);
    Ok(write_config(target_dir, &text)?)
}

fn launch_table(target_dir: &Path) -> Table {
    let data = read_config(target_dir)
        .ok()
        .and_then(|text| toml::from_str::<Table>(&text).ok());
    match data.and_then(|mut data| data.remove("Launch")) {
        Some(Value::Table(launch)) => launch,
        _ => Table::new(),
    }
}

pub fn read_tl_autorun(target_dir: &Path) -> Vec<String> {
    launch_table(target_dir)
        .get("autorun")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn with_launch_line(text: &str, key: &str, value: &str) -> Result<String> {
    let line = format!("{key} = {value}");
    let assignment = Regex::new(&format!(r"(?m)^{key}\s*=.*$"))?;
    if let Some(found) = assignment.find(text) {
        return Ok([&text[..found.start()], &line, &text[found.end()..]].concat());
    }
    let Some(header) = LAUNCH_HEADER_RX.find(text) else {
        bail!("Config.toml has no [Launch] section");
    };
    Ok([&text[..header.end()], "\n", &line, &text[header.end()..]].concat())
}

fn write_launch_line(target_dir: &Path, key: &str, value: &str) -> Result<()> {
    let text = with_launch_line(&read_config(target_dir)?, key, value)?;
    Ok(write_config(target_dir, &text)?)
}

pub fn write_tl_autorun<S: AsRef<str>>(target_dir: &Path, autorun: &[S]) -> Result<()> {
    let items: Vec<String> = autorun.iter().map(|item| literal(item.as_ref())).collect();
    write_launch_line(target_dir, "autorun", &format!("[{}]", items.join(", ")))
}

pub fn read_tl_args(target_dir: &Path) -> String {
    launch_table(target_dir)
        .get("args")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub fn write_tl_args(target_dir: &Path, args: &str) -> Result<()> {
    write_launch_line(target_dir, "args", &literal(args))
}

pub fn apply_tl(paths: &Paths, target_dir: &Path, username: &str) -> Result<()> {
    let hm = target_dir
        .file_name()
        .is_some_and(|name| name.to_string_lossy().ends_with(HM_FOLDER_SUFFIX));
    let cache = paths.tl_dir();
    for name in dll_names() {
        if hm && name == DEFAULTARGS_DLL {
            continue;
        }
        copy_with_times(&cache.join(name), &target_dir.join(name))?;
    }
    if !target_dir.join(TL_TOML).exists() {
        copy_with_times(&cache.join(TL_TOML), &target_dir.join(TL_TOML))?;
    }
    write_tl_username(target_dir, username)
}

pub fn write_launcher(paths: &Paths, target_dir: &Path) -> Result<()> {
    Ok(copy_with_times(
        &paths.tl_dir().join(TL_LAUNCHER),
        &target_dir.join(TL_LAUNCHER),
    )?)
}

#[cfg(test)]
pub(crate) mod testing {
    use std::fs;
    use std::path::Path;

    pub(crate) fn names(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};
    use std::sync::{Arc, Mutex};

    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;

    use super::testing::names;
    use super::*;
    use crate::github::RateLimitError;
    use crate::github::testing::{Reply, Server, seed};
    use crate::log::take_recorded;

    const BASE: &str = "[User]\nusername = \"ThrowbackUser\"\n\n[Launch]\nargs = '/belaunch /nologo'\nautorun = []\n";

    const USERS: &[(&str, &str, &str)] = &[
        (
            BASE,
            "Ash",
            "[User]\nusername = 'Ash'\n\n[Launch]\nargs = '/belaunch /nologo'\nautorun = []\n",
        ),
        (
            BASE,
            "It's $1 \\ \"x\"",
            "[User]\nusername = \"It's $1 \\\\ \\\"x\\\"\"\n\n[Launch]\nargs = '/belaunch /nologo'\nautorun = []\n",
        ),
        (
            "username='a'\nusername = \"b\"\n",
            "c",
            "username = 'c'\nusername = \"b\"\n",
        ),
        (
            "# myusername   =\n\n  'old'\ntools = ['a', \"b]\\\"\", 'c]'] # gone\nkeep = 1\n",
            "new",
            "# myusername = 'new'\nkeep = 1\n",
        ),
        (
            "username = 5\ntools = [\n  'x',\n  \"y\"\n]\nrest\n",
            "n",
            "username = 5\nrest\n",
        ),
        ("tools = ['a'\n", "n", "tools = ['a'\n"),
        (
            " tools = []\nusername\u{a0}=\u{2003}'x'\n",
            "n",
            " tools = []\nusername = 'n'\n",
        ),
        ("tools=[]", "n", ""),
        ("username = \"a'\n", "n", "username = 'n'\n"),
        (
            "username = ''\ntools = [] trailing\n\ntools = []\n",
            "tab\there",
            "username = \"tab\\u0009here\"\n\ntools = []\n",
        ),
    ];

    const LINES: &[(&str, &str, &str, Option<&str>)] = &[
        (
            BASE,
            "args",
            "'/belaunch /nologo /vulkan'",
            Some(
                "[User]\nusername = \"ThrowbackUser\"\n\n[Launch]\nargs = '/belaunch /nologo /vulkan'\nautorun = []\n",
            ),
        ),
        (
            BASE,
            "autorun",
            "['C:\\Tools\\ce.exe', \"it's\"]",
            Some(
                "[User]\nusername = \"ThrowbackUser\"\n\n[Launch]\nargs = '/belaunch /nologo'\nautorun = ['C:\\Tools\\ce.exe', \"it's\"]\n",
            ),
        ),
        ("[Launch]\n", "args", "''", Some("[Launch]\n\nargs = ''")),
        ("[Launch]", "args", "'x'", Some("[Launch]\nargs = 'x'")),
        (
            "[Launch]\n\n\nfoo = 1\n",
            "autorun",
            "[]",
            Some("[Launch]\n\n\nautorun = []\nfoo = 1\n"),
        ),
        (
            "[Launch]   \n# c\n",
            "args",
            "'a'",
            Some("[Launch]   \nargs = 'a'\n# c\n"),
        ),
        ("[Launch] # c\nother = 1\n", "args", "'a'", None),
        (
            "[User]\nargs   = 1 # old\n[Launch]\n",
            "args",
            "'$0 \\1'",
            Some("[User]\nargs = '$0 \\1'\n[Launch]\n"),
        ),
        (" args = 1\n[launch]\n", "args", "'a'", None),
        (
            "args\n\n = 'x'\n[Launch]\n",
            "args",
            "'y'",
            Some("args = 'y'\n[Launch]\n"),
        ),
        (
            "autorun = [\n 'a',\n]\n[Launch]\n",
            "autorun",
            "['b']",
            Some("autorun = ['b']\n 'a',\n]\n[Launch]\n"),
        ),
        (
            "[Launch]\nargsx = 1\n",
            "args",
            "'a'",
            Some("[Launch]\nargs = 'a'\nargsx = 1\n"),
        ),
        ("x = 1\n  [Launch]\n", "args", "'a'", None),
    ];

    fn season(text: &str) -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(TL_TOML), text).unwrap();
        dir
    }

    fn config(dir: &Path) -> String {
        fs::read_to_string(dir.join(TL_TOML)).unwrap()
    }

    fn version_info(struc_version: u32, ms: u32, ls: u32) -> Vec<u8> {
        let mut data = b"\xbd\x04\xef\xfe".to_vec();
        for field in [struc_version, ms, ls] {
            data.extend(field.to_le_bytes());
        }
        data
    }

    fn cache(paths: &Paths) {
        fs::create_dir_all(paths.tl_dir()).unwrap();
        for name in extract_names() {
            fs::write(paths.tl_dir().join(name), format!("cache {name}")).unwrap();
        }
        fs::write(paths.tl_dir().join(TL_TOML), BASE).unwrap();
    }

    fn paths(dir: &TempDir) -> Paths {
        Paths::new(dir.path().join("asset"), dir.path().join("data"))
    }

    fn archive(names: impl Iterator<Item = &'static str>) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut cursor);
        for name in names {
            writer
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(format!("new {name}").as_bytes()).unwrap();
        }
        drop(writer);
        cursor.into_inner()
    }

    fn sorted_members() -> Vec<&'static str> {
        let mut members: Vec<&str> = extract_names().collect();
        members.sort_unstable();
        members
    }

    fn assert_cache(folder: &Path, origin: &str) {
        assert_eq!(names(folder), sorted_members());
        for name in extract_names() {
            let expected = if name == TL_TOML && origin == "cache" {
                BASE.to_string()
            } else {
                format!("{origin} {name}")
            };
            assert_eq!(fs::read_to_string(folder.join(name)).unwrap(), expected);
        }
    }

    #[test]
    fn the_username_line_is_replaced_and_tools_is_dropped() {
        for (text, username, expected) in USERS {
            assert_eq!(with_username(text, username), *expected, "{text:?}");
        }
    }

    #[test]
    fn a_launch_line_is_replaced_or_added_under_the_header() {
        for (text, key, value, expected) in LINES {
            match (with_launch_line(text, key, value), expected) {
                (Ok(updated), Some(expected)) => assert_eq!(updated, *expected, "{text:?}"),
                (Err(error), None) => {
                    assert_eq!(error.to_string(), "Config.toml has no [Launch] section")
                }
                (outcome, _) => panic!("{text:?} gave {outcome:?}"),
            }
        }
    }

    #[test]
    fn version_comes_from_the_fixed_file_info() {
        let mut data = b"MZ\xbd\x04\xef".to_vec();
        data.extend(version_info(0x0001_0000, 0x0000_0001, 0x0008_0002));
        assert_eq!(launcher_version(&data).as_deref(), Some("0.1.8"));
        let mut data = version_info(0x0002_0000, 0x0009_0009, 0x0009_0000);
        data.extend(version_info(0x0001_0000, 0x0001_0002, 0x0003_0004));
        assert_eq!(launcher_version(&data).as_deref(), Some("1.2.3"));
        assert_eq!(
            launcher_version(&version_info(0x0001_0000, 0xffff_ffff, 0xffff_0000)).as_deref(),
            Some("65535.65535.65535")
        );
    }

    #[test]
    fn version_scan_steps_four_bytes_and_needs_sixteen() {
        let full = version_info(0x0001_0000, 0x0001_0000, 0);
        assert_eq!(launcher_version(&full[..15]), None);
        assert_eq!(launcher_version(b""), None);
        assert_eq!(launcher_version(b"\xbd\x04\xef"), None);
        let mut nested = b"\xbd\x04\xef\xfe\x00".to_vec();
        nested.extend(&full);
        assert_eq!(launcher_version(&nested).as_deref(), Some("1.0.0"));
        let mut overlap = b"\xbd\x04\xef\xfe\xbd\x04".to_vec();
        overlap.extend(&full);
        assert_eq!(launcher_version(&overlap).as_deref(), Some("1.0.0"));
        let mut hidden = b"\xbd\x04\xef\xfe\x00\x00".to_vec();
        hidden.extend(&full[..4]);
        hidden.extend([0, 0, 1, 0, 5, 0, 4, 0, 0, 0, 3, 0]);
        assert_eq!(launcher_version(&hidden).as_deref(), Some("4.5.3"));
    }

    #[test]
    fn folder_version_reads_the_launcher() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(tl_folder_version(dir.path()), None);
        fs::write(
            dir.path().join(TL_LAUNCHER),
            version_info(0x0001_0000, 0x0000_0002, 0x0001_0000),
        )
        .unwrap();
        assert_eq!(tl_folder_version(dir.path()).as_deref(), Some("0.2.1"));
        let paths = Paths::new(dir.path().to_path_buf(), dir.path().to_path_buf());
        assert_eq!(tl_version(&paths), None);
    }

    #[test]
    fn present_needs_all_eight_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(&dir);
        assert!(!tl_present(&paths.tl_dir()));
        cache(&paths);
        assert_eq!(extract_names().count(), 8);
        assert!(tl_present(&paths.tl_dir()));
        for name in extract_names() {
            let file = paths.tl_dir().join(name);
            fs::remove_file(&file).unwrap();
            assert!(!tl_present(&paths.tl_dir()), "{name}");
            fs::create_dir(&file).unwrap();
            assert!(!tl_present(&paths.tl_dir()), "{name}");
            fs::remove_dir(&file).unwrap();
            fs::write(&file, "x").unwrap();
        }
        assert!(tl_present(&paths.tl_dir()));
    }

    #[test]
    fn ensure_skips_the_network_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(&dir);
        cache(&paths);
        ensure_tl(&paths, None, false).unwrap();
        assert_eq!(
            fs::read_to_string(paths.tl_dir().join(TL_LAUNCHER)).unwrap(),
            "cache LaunchR6.exe"
        );
    }

    #[test]
    fn ensure_downloads_the_loader_and_clears_its_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(&dir);
        let folder = paths.tl_dir();
        let server =
            Server::start(|_| Reply::new(200, archive(extract_names().chain(["README.md"]))));
        let asset_url = server.url("/assets/1");
        seed(&paths, TL_API_URL, &[("ThrowbackLoader.zip", &asset_url)]);
        fs::create_dir_all(folder.join(".tl.tmp").join(TL_TOML)).unwrap();
        fs::write(folder.join(".tl.tmp").join("stale"), "").unwrap();
        fs::write(folder.join(TL_LAUNCHER), "old").unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let report = move |fraction: f64| sink.lock().unwrap().push(fraction);
        ensure_tl(&paths, Some(&report), false).unwrap();
        assert_cache(&folder, "new");
        assert_eq!(seen.lock().unwrap().last(), Some(&1.0));
        assert_eq!(server.paths(), ["/assets/1"]);
        fs::write(folder.join(TL_TOML), "edited").unwrap();
        ensure_tl(&paths, Some(&report), false).unwrap();
        assert_eq!(fs::read_to_string(folder.join(TL_TOML)).unwrap(), "edited");
        assert_eq!(server.paths().len(), 1);
        ensure_tl(&paths, Some(&report), true).unwrap();
        assert_cache(&folder, "new");
        assert_eq!(server.paths().len(), 2);
        assert!(take_recorded().is_empty());
    }

    #[test]
    fn failed_downloads_clean_up_and_leave_the_cache_alone() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(&dir);
        let folder = paths.tl_dir();
        let server = Server::start(|request| match request.path.as_str() {
            "/short" => Reply::new(
                200,
                archive(extract_names().filter(|name| *name != TL_LAUNCHER)),
            ),
            "/limited" => Reply::rate_limit(),
            _ => Reply::new(500, Vec::new()),
        });
        cache(&paths);
        let attempt = |path: &str| {
            let asset_url = server.url(path);
            seed(&paths, TL_API_URL, &[("ThrowbackLoader.zip", &asset_url)]);
            fs::write(folder.join("tl.zip"), "stale").unwrap();
            fs::create_dir_all(folder.join(".tl.tmp").join("stale")).unwrap();
            let error = ensure_tl(&paths, None, true).unwrap_err();
            assert_cache(&folder, "cache");
            error
        };
        assert_eq!(
            attempt("/short").to_string(),
            "ThrowbackLoader download failed"
        );
        let lines = take_recorded();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("ThrowbackLoader download failed - "),
            "{lines:?}"
        );
        let limited = attempt("/limited");
        assert!(limited.is::<RateLimitError>());
        assert_eq!(
            limited.to_string(),
            "GitHub rate limit reached, try again in 2 minutes"
        );
        assert!(take_recorded().is_empty());
        assert_eq!(
            attempt("/broken").to_string(),
            "ThrowbackLoader download failed"
        );
        assert_eq!(take_recorded().len(), 1);
        assert_eq!(
            server.paths(),
            ["/short", "/limited", "/broken", "/broken", "/broken"]
        );
    }

    #[test]
    fn config_reads_translate_newlines_and_reject_bad_utf8() {
        let dir = season("[Launch]\r\nargs = 'a'\rautorun = ['x']\r\n");
        assert_eq!(
            read_config(dir.path()).unwrap(),
            "[Launch]\nargs = 'a'\nautorun = ['x']\n"
        );
        assert_eq!(read_tl_args(dir.path()), "a");
        fs::write(dir.path().join(TL_TOML), b"[Launch]\nargs = '\xff'\n").unwrap();
        assert_eq!(
            read_config(dir.path()).unwrap_err().to_string(),
            "Config.toml is not valid UTF-8"
        );
        assert_eq!(
            write_tl_args(dir.path(), "x").unwrap_err().to_string(),
            "Config.toml is not valid UTF-8"
        );
        assert_eq!(read_tl_args(dir.path()), "");
        assert!(read_config(&dir.path().join("missing")).is_err());
    }

    #[test]
    fn the_username_reaches_only_folders_with_a_config() {
        let dir = tempfile::tempdir().unwrap();
        write_tl_username(dir.path(), "Ash").unwrap();
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        fs::write(dir.path().join(TL_TOML), "username = \"old\"\ntools = []\n").unwrap();
        write_tl_username(dir.path(), "Ash").unwrap();
        assert_eq!(
            config(dir.path()).replace("\r\n", "\n"),
            "username = 'Ash'\n"
        );
        fs::write(dir.path().join(TL_TOML), b"username = '\xff'\n").unwrap();
        assert_eq!(
            write_tl_username(dir.path(), "Ash")
                .unwrap_err()
                .to_string(),
            "Config.toml is not valid UTF-8"
        );
    }

    #[test]
    fn config_writes_use_the_platform_line_ending() {
        let dir = season("[User]\r\nusername = 'a'\r\n\r\n[Launch]\r\n");
        write_tl_username(dir.path(), "Ash").unwrap();
        let expected = "[User]\nusername = 'Ash'\n\n[Launch]\n";
        if IS_WINDOWS {
            assert_eq!(config(dir.path()), expected.replace('\n', "\r\n"));
        } else {
            assert_eq!(config(dir.path()), expected);
        }
    }

    #[test]
    fn launch_values_are_read_from_the_launch_table() {
        let dir = season(
            "[Launch]\nargs = \"/belaunch \\u0041\"\nautorun = ['a', 1, \"b\", ['c'], true]\n",
        );
        assert_eq!(read_tl_args(dir.path()), "/belaunch A");
        assert_eq!(read_tl_autorun(dir.path()), ["a", "b"]);
        for text in [
            "[Launch]\nargs = 'a'\nautorun = ['x']\nargs = 'b'\n",
            "[Launch\nargs = 'a'\n",
            "[Launch]\nargs = 5\nautorun = [1, ['x']]\n",
            "[Other]\nargs = 'a'\nautorun = ['x']\n",
            "",
        ] {
            let dir = season(text);
            assert_eq!(read_tl_args(dir.path()), "", "{text:?}");
            assert!(read_tl_autorun(dir.path()).is_empty(), "{text:?}");
        }
        let missing = tempfile::tempdir().unwrap();
        assert_eq!(read_tl_args(missing.path()), "");
        assert!(read_tl_autorun(missing.path()).is_empty());
    }

    #[test]
    fn odd_launch_values_are_read_as_empty() {
        for text in [
            "Launch = 5\n",
            "Launch = 'abc'\n",
            "[[Launch]]\nargs = 'a'\nautorun = ['x']\n",
            "[Launch]\nautorun = 'ab'\n",
            "[Launch]\nautorun = { x = 1, y = 'q' }\n",
            "[Launch]\nautorun = 5\n",
        ] {
            let dir = season(text);
            assert_eq!(read_tl_args(dir.path()), "", "{text:?}");
            assert!(read_tl_autorun(dir.path()).is_empty(), "{text:?}");
        }
    }

    #[test]
    fn launch_writes_round_trip() {
        let dir = season(BASE);
        write_tl_args(dir.path(), "/belaunch /nologo /vulkan").unwrap();
        write_tl_autorun(dir.path(), &["C:\\Tools\\ce.exe", "it's"]).unwrap();
        assert_eq!(read_tl_args(dir.path()), "/belaunch /nologo /vulkan");
        assert_eq!(read_tl_autorun(dir.path()), ["C:\\Tools\\ce.exe", "it's"]);
        write_tl_autorun::<&str>(dir.path(), &[]).unwrap();
        write_tl_args(dir.path(), "").unwrap();
        assert_eq!(
            config(dir.path()).replace("\r\n", "\n"),
            "[User]\nusername = \"ThrowbackUser\"\n\n[Launch]\nargs = ''\nautorun = []\n"
        );
        let bare = season("[User]\nusername = 'a'\n");
        assert_eq!(
            write_tl_autorun(bare.path(), &["x"])
                .unwrap_err()
                .to_string(),
            "Config.toml has no [Launch] section"
        );
        assert_eq!(config(bare.path()), "[User]\nusername = 'a'\n");
    }

    #[test]
    fn apply_copies_the_loader_and_keeps_an_existing_config() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(&dir);
        cache(&paths);
        let target = dir.path().join("Y1S0_Vanilla");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("steam_api64.dll"), "depot original").unwrap();
        apply_tl(&paths, &target, "Ash").unwrap();
        for name in dll_names() {
            assert_eq!(
                fs::read_to_string(target.join(name)).unwrap(),
                format!("cache {name}")
            );
        }
        assert_eq!(
            config(&target).replace("\r\n", "\n"),
            BASE.replace("\"ThrowbackUser\"", "'Ash'")
        );
        assert!(!target.join(TL_LAUNCHER).exists());
        fs::write(
            target.join(TL_TOML),
            "username = 'old'\ntools = ['x']\nmine = 1\n",
        )
        .unwrap();
        apply_tl(&paths, &target, "Zofia").unwrap();
        assert_eq!(
            config(&target).replace("\r\n", "\n"),
            "username = 'Zofia'\nmine = 1\n"
        );
        write_launcher(&paths, &target).unwrap();
        assert_eq!(
            fs::read_to_string(target.join(TL_LAUNCHER)).unwrap(),
            "cache LaunchR6.exe"
        );
    }

    #[test]
    fn apply_leaves_the_heated_metal_defaultargs_alone() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(&dir);
        cache(&paths);
        let target = dir.path().join("Y9S2_HeatedMetal");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join(DEFAULTARGS_DLL), "heated metal").unwrap();
        apply_tl(&paths, &target, "Ash").unwrap();
        assert_eq!(
            fs::read_to_string(target.join(DEFAULTARGS_DLL)).unwrap(),
            "heated metal"
        );
        assert_eq!(
            fs::read_to_string(target.join("steam_api64.dll")).unwrap(),
            "cache steam_api64.dll"
        );
        let bare = dir.path().join("Y9S3_HeatedMetal");
        fs::create_dir_all(&bare).unwrap();
        apply_tl(&paths, &bare, "Ash").unwrap();
        assert!(!bare.join(DEFAULTARGS_DLL).exists());
    }

    #[test]
    fn apply_stops_when_the_cache_is_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(&dir);
        cache(&paths);
        fs::remove_file(paths.tl_dir().join("upc_r1_loader64.dll")).unwrap();
        let target = dir.path().join("Y1S0_Vanilla");
        fs::create_dir_all(&target).unwrap();
        assert!(apply_tl(&paths, &target, "Ash").is_err());
        assert!(target.join("uplay_r1_loader64.dll").exists());
        assert!(!target.join("upc_r2_loader64.dll").exists());
        assert!(!target.join(TL_TOML).exists());
        assert!(write_launcher(&paths, &dir.path().join("missing")).is_err());
    }
}
