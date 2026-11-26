use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use serde::Deserialize;
use toml::{Table, Value};

use crate::env::{HM_FOLDER_SUFFIX, IS_WINDOWS, TL_LAUNCHER};
use crate::heatedmetal::HM_UNSTABLE;

const MANIFEST: &str = include_str!("../../../manifest.toml");

static SEASON_FOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("^Y[0-9]+S[0-9]+_").expect("pattern"));

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Download {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub build: String,
    pub app: i64,
    pub depot_content: i64,
    pub depot_ww: i64,
    pub manifest_content: String,
    pub manifest_ww: String,
    pub size_gb: f64,
    pub hm_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SeasonEntry {
    pub key: String,
    pub id: String,
    pub hm: bool,
    pub code: String,
    pub name: String,
    pub size_gb: f64,
    pub build: String,
    pub hm_available: bool,
    pub hm_beta: bool,
    pub partial: bool,
}

impl SeasonEntry {
    pub fn title(&self, hm: bool) -> String {
        let name = if hm { "Heated Metal" } else { &self.name };
        format!("{} {name}", self.code)
    }
}

pub fn downloads() -> Vec<Download> {
    parse(MANIFEST).expect("bundled manifest")
}

fn parse(text: &str) -> Result<Vec<Download>, toml::de::Error> {
    let data: Table = toml::from_str(text)?;
    let defaults: Table = data
        .iter()
        .filter(|(_, value)| !value.is_table())
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    data.iter()
        .filter_map(|(key, value)| Some((key, value.as_table()?)))
        .map(|(key, block)| {
            let mut season = defaults.clone();
            season.extend(block.clone());
            season.insert("key".to_owned(), key.as_str().into());
            Value::Table(season).try_into()
        })
        .collect()
}

pub fn is_season_folder(folder: &Path) -> bool {
    folder
        .file_name()
        .is_some_and(|name| SEASON_FOLDER.is_match(&name.to_string_lossy()))
}

fn rank(path: &Path) -> OsString {
    if IS_WINDOWS {
        path.to_string_lossy().to_lowercase().into()
    } else {
        path.as_os_str().to_os_string()
    }
}

fn season_folders(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut folders: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && is_season_folder(path))
        .collect();
    folders.sort_by_cached_key(|path| rank(path));
    folders
}

pub fn local_downloads(libraries: &[PathBuf]) -> Vec<PathBuf> {
    libraries
        .iter()
        .flat_map(|root| season_folders(root))
        .collect()
}

pub fn resolve_install<'a>(
    folder_name: &str,
    downloads: &'a [Download],
) -> Option<(&'a Download, bool)> {
    let Some(code) = folder_name.strip_suffix(HM_FOLDER_SUFFIX) else {
        let download = downloads
            .iter()
            .find(|download| download.key == folder_name)?;
        return Some((download, false));
    };
    let prefix = format!("{code}_");
    let download = downloads
        .iter()
        .find(|download| download.key.starts_with(&prefix) && download.hm_version.is_some())?;
    Some((download, true))
}

pub fn is_installed(folder: &Path) -> bool {
    folder.join(TL_LAUNCHER).exists()
}

pub fn installed_downloads(libraries: &[PathBuf]) -> Vec<PathBuf> {
    let mut folders = local_downloads(libraries);
    folders.retain(|folder| is_installed(folder));
    folders
}

pub fn hm_folder_name(key: &str) -> String {
    let code = key.split_once('_').map_or(key, |(code, _)| code);
    format!("{code}{HM_FOLDER_SUFFIX}")
}

pub fn edition_folder(key: &str, hm: bool) -> String {
    if hm {
        hm_folder_name(key)
    } else {
        key.to_string()
    }
}

pub fn installed_path(libraries: &[PathBuf], key: &str, hm: bool) -> Option<PathBuf> {
    let folder = edition_folder(key, hm);
    libraries
        .iter()
        .map(|root| root.join(&folder))
        .find(|path| is_installed(path))
}

pub fn partial_path(libraries: &[PathBuf], key: &str, hm: bool) -> Option<PathBuf> {
    if installed_path(libraries, key, hm).is_some() {
        return None;
    }
    let folder = edition_folder(key, hm);
    libraries
        .iter()
        .map(|root| root.join(&folder))
        .find(|path| path.is_dir())
}

pub fn season_entry(download: &Download, hm: bool) -> SeasonEntry {
    let (code, name) = download
        .label
        .split_once(' ')
        .unwrap_or((&download.label, ""));
    SeasonEntry {
        key: download.key.clone(),
        id: edition_folder(&download.key, hm),
        hm,
        code: code.to_string(),
        name: if name.is_empty() {
            download.label.clone()
        } else {
            name.to_string()
        },
        size_gb: download.size_gb,
        build: download.build.clone(),
        hm_available: download.hm_version.is_some(),
        hm_beta: download.hm_version.as_deref() == Some(HM_UNSTABLE),
        partial: false,
    }
}

pub fn seasons(downloads: &[Download]) -> Vec<SeasonEntry> {
    downloads
        .iter()
        .map(|download| season_entry(download, false))
        .collect()
}

pub fn home(downloads: &[Download], libraries: &[PathBuf]) -> Vec<SeasonEntry> {
    let mut entries = Vec::new();
    for download in downloads {
        for hm in [false, true] {
            if hm && download.hm_version.is_none() {
                continue;
            }
            let partial = if installed_path(libraries, &download.key, hm).is_some() {
                false
            } else if partial_path(libraries, &download.key, hm).is_some() {
                true
            } else {
                continue;
            };
            entries.push(SeasonEntry {
                partial,
                ..season_entry(download, hm)
            });
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    const SMALL: &str = "app = 1\ndepot_content = 2\ndepot_ww = 3\n\n[Y1S0_Vanilla]\nlabel = 'Y1S0 Vanilla'\nbuild = '8194013'\nmanifest_content = '11'\nmanifest_ww = '12'\nsize_gb = 14.2\n\n[Y5S3_ShadowLegacy]\nlabel = 'Y5S3 Shadow Legacy'\nbuild = '2'\nmanifest_content = '21'\nmanifest_ww = '22'\nsize_gb = 50\nhm_version = '0.2.3'\n\n[Y5S4_NeonDawn]\nlabel = 'Y5S4 Neon Dawn'\nbuild = '3'\nmanifest_content = '31'\nmanifest_ww = '32'\nsize_gb = 51.5\nhm_version = 'latest'\n\n[Y9S2_NewBlood]\nlabel = 'Y9S2'\nbuild = '4'\nmanifest_content = '41'\nmanifest_ww = '42'\nsize_gb = 60.1\nhm_version = 'unstable'\n";

    fn small() -> Vec<Download> {
        parse(SMALL).unwrap()
    }

    fn library(dir: &TempDir, name: &str, folders: &[(&str, bool)]) -> PathBuf {
        let root = dir.path().join(name);
        for (folder, installed) in folders {
            fs::create_dir_all(root.join(folder)).unwrap();
            if *installed {
                fs::write(root.join(folder).join(TL_LAUNCHER), "").unwrap();
            }
        }
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn the_bundled_manifest_loads_in_file_order() {
        let downloads = downloads();
        assert!(downloads.len() >= 43);
        assert_eq!(
            downloads[0],
            Download {
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
            }
        );
        let code = Regex::new(r"^Y(\d+)S(\d+)_\w+$").unwrap();
        let chronological: Vec<(u32, u32)> = downloads
            .iter()
            .map(|download| {
                let parts = code.captures(&download.key).unwrap();
                (parts[1].parse().unwrap(), parts[2].parse().unwrap())
            })
            .collect();
        assert!(chronological.is_sorted_by(|earlier, later| earlier < later));
        assert_eq!(chronological[42], (11, 2));
        let heated: Vec<&Download> = downloads
            .iter()
            .filter(|download| download.hm_version.is_some())
            .collect();
        assert!(
            heated
                .iter()
                .any(|download| download.key == "Y9S2_NewBlood")
        );
        for download in &heated {
            let folder = hm_folder_name(&download.key);
            let resolved = resolve_install(&folder, &downloads).unwrap();
            assert_eq!(
                (resolved.0.key.as_str(), resolved.1),
                (download.key.as_str(), true)
            );
        }
        for download in &downloads {
            assert!(is_season_folder(Path::new(&download.key)), "{download:?}");
            assert_eq!(
                download.label.split(' ').next(),
                download.key.split('_').next()
            );
            assert!(download.size_gb > 0.0);
            assert!(download.build.bytes().all(|digit| digit.is_ascii_digit()));
            assert!(
                download
                    .manifest_content
                    .bytes()
                    .all(|digit| digit.is_ascii_digit())
            );
            assert!(
                download
                    .manifest_ww
                    .bytes()
                    .all(|digit| digit.is_ascii_digit())
            );
            assert_eq!(
                (download.app, download.depot_content, download.depot_ww),
                (359550, 359551, 377237)
            );
            let resolved = resolve_install(&download.key, &downloads).unwrap();
            assert_eq!((resolved.0, resolved.1), (download, false));
        }
    }

    #[test]
    fn blocks_override_defaults_and_keep_their_order() {
        let downloads = small();
        let keys: Vec<&str> = downloads.iter().map(|d| d.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "Y1S0_Vanilla",
                "Y5S3_ShadowLegacy",
                "Y5S4_NeonDawn",
                "Y9S2_NewBlood"
            ]
        );
        assert_eq!(downloads[1].size_gb, 50.0);
        assert_eq!(
            (
                downloads[1].app,
                downloads[1].depot_content,
                downloads[1].depot_ww
            ),
            (1, 2, 3)
        );
        let text = format!(
            "{SMALL}app = 9\nkey = 'Renamed'\n\n[Y1S1_BlackIce]\nlabel = 'Y1S1 Black Ice'\nbuild = '5'\nmanifest_content = '51'\nmanifest_ww = '52'\nsize_gb = 1.0\ndepot_ww = 7\nextra = {{ a = 1 }}\n"
        );
        let downloads = parse(&text).unwrap();
        assert_eq!(downloads.len(), 5);
        assert_eq!(downloads[3].key, "Y9S2_NewBlood");
        assert_eq!((downloads[3].app, downloads[3].depot_ww), (9, 3));
        assert_eq!(downloads[4].key, "Y1S1_BlackIce");
        assert_eq!((downloads[4].app, downloads[4].depot_ww), (1, 7));
        let inline = "app = 1\ndepot_content = 2\ndepot_ww = 3\nZ = { label = 'Z z', build = 'b', manifest_content = 'c', manifest_ww = 'w', size_gb = 1 }\nA.label = 'A'\nA.build = 'b'\nA.manifest_content = 'c'\nA.manifest_ww = 'w'\nA.size_gb = 2.5\n";
        let downloads = parse(inline).unwrap();
        let keys: Vec<&str> = downloads.iter().map(|d| d.key.as_str()).collect();
        assert_eq!(keys, ["Z", "A"]);
    }

    #[test]
    fn a_block_with_a_missing_or_mistyped_field_is_refused() {
        assert!(parse(&SMALL.replace("build = '2'", "build = 2")).is_err());
        let unbuilt = parse(&SMALL.replace("build = '2'\n", "")).unwrap();
        assert_eq!(unbuilt[1].build, "");
        assert!(parse(&SMALL.replace("depot_ww = 3\n", "")).is_err());
        assert!(parse("[Y1S0_Vanilla\n").is_err());
    }

    #[test]
    fn season_folder_names_start_with_the_season_code() {
        for name in [
            "Y1S0_Vanilla",
            "Y10S4_TenfoldPursuit",
            "Y9S2_HeatedMetal",
            "Y1S1_",
        ] {
            assert!(
                is_season_folder(Path::new("/lib").join(name).as_path()),
                "{name}"
            );
        }
        for name in [
            "y1S0_Vanilla",
            "Y1s0_Vanilla",
            "Y1S0",
            "Y1S0-Vanilla",
            "YS1_x",
            " Y1S0_Vanilla",
            "XY1S0_Vanilla",
            "Y1S_x",
            "Y٣S1_x",
            "",
        ] {
            assert!(
                !is_season_folder(Path::new("/lib").join(name).as_path()),
                "{name}"
            );
        }
        assert!(!is_season_folder(Path::new("/")));
        assert!(!is_season_folder(Path::new("Y1S0_Vanilla/..")));
    }

    #[test]
    fn edition_folders_follow_the_naming_rule() {
        assert_eq!(hm_folder_name("Y9S2_NewBlood"), "Y9S2_HeatedMetal");
        assert_eq!(hm_folder_name("Y10S2_Day_Break"), "Y10S2_HeatedMetal");
        assert_eq!(hm_folder_name("Y9S2"), "Y9S2_HeatedMetal");
        assert_eq!(hm_folder_name(""), "_HeatedMetal");
        assert_eq!(hm_folder_name("_x"), "_HeatedMetal");
        assert_eq!(edition_folder("Y9S2_NewBlood", true), "Y9S2_HeatedMetal");
        assert_eq!(edition_folder("Y9S2_NewBlood", false), "Y9S2_NewBlood");
    }

    #[test]
    fn folders_resolve_to_their_manifest_entry() {
        let downloads = small();
        let resolved = |name: &str| {
            resolve_install(name, &downloads).map(|(download, hm)| (download.key.as_str(), hm))
        };
        assert_eq!(resolved("Y1S0_Vanilla"), Some(("Y1S0_Vanilla", false)));
        assert_eq!(resolved("Y9S2_NewBlood"), Some(("Y9S2_NewBlood", false)));
        assert_eq!(resolved("Y9S2_HeatedMetal"), Some(("Y9S2_NewBlood", true)));
        assert_eq!(
            resolved("Y5S3_HeatedMetal"),
            Some(("Y5S3_ShadowLegacy", true))
        );
        assert_eq!(resolved("Y1S0_HeatedMetal"), None);
        assert_eq!(resolved("Y5S_HeatedMetal"), None);
        assert_eq!(resolved("Y1S0"), None);
        assert_eq!(resolved("y1s0_vanilla"), None);
        assert_eq!(resolved("_HeatedMetal"), None);
        assert_eq!(resolved(""), None);
    }

    #[test]
    fn local_downloads_are_sorted_per_library() {
        let dir = tempfile::tempdir().unwrap();
        let first = library(
            &dir,
            "first",
            &[
                ("Y9S2_NewBlood", true),
                ("Y10S1_PrepPhase", false),
                ("Y1S0_Vanilla", true),
                ("Y9S2_HeatedMetal", false),
                ("notes", false),
                (".Y1S1_hidden", false),
            ],
        );
        fs::write(first.join("Y2S1_File"), "").unwrap();
        let second = library(&dir, "second", &[("Y1S0_Vanilla", false), ("Y2S2_x", true)]);
        let missing = dir.path().join("missing");
        let libraries = [missing, second.clone(), first.clone()];
        assert_eq!(
            local_downloads(&libraries),
            [
                second.join("Y1S0_Vanilla"),
                second.join("Y2S2_x"),
                first.join("Y10S1_PrepPhase"),
                first.join("Y1S0_Vanilla"),
                first.join("Y9S2_HeatedMetal"),
                first.join("Y9S2_NewBlood"),
            ]
        );
        assert_eq!(
            installed_downloads(&libraries),
            [
                second.join("Y2S2_x"),
                first.join("Y1S0_Vanilla"),
                first.join("Y9S2_NewBlood"),
            ]
        );
        assert!(local_downloads(&[]).is_empty());
        assert!(local_downloads(&[first.join("Y2S1_File")]).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn linked_season_folders_count_as_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = library(&dir, "root", &[("Y1S0_Vanilla", true)]);
        std::os::unix::fs::symlink(root.join("Y1S0_Vanilla"), root.join("Y1S1_Link")).unwrap();
        std::os::unix::fs::symlink(root.join("gone"), root.join("Y1S2_Broken")).unwrap();
        assert_eq!(
            installed_downloads(std::slice::from_ref(&root)),
            [root.join("Y1S0_Vanilla"), root.join("Y1S1_Link")]
        );
    }

    #[test]
    fn the_first_library_with_the_marker_wins() {
        let dir = tempfile::tempdir().unwrap();
        let first = library(
            &dir,
            "first",
            &[("Y9S2_NewBlood", false), ("Y1S0_Vanilla", false)],
        );
        let second = library(
            &dir,
            "second",
            &[("Y9S2_NewBlood", true), ("Y9S2_HeatedMetal", false)],
        );
        let third = library(
            &dir,
            "third",
            &[("Y9S2_NewBlood", true), ("Y1S0_Vanilla", false)],
        );
        fs::write(first.join("Y5S3_ShadowLegacy"), "").unwrap();
        let libraries = [first.clone(), second.clone(), third];
        assert_eq!(
            installed_path(&libraries, "Y9S2_NewBlood", false),
            Some(second.join("Y9S2_NewBlood"))
        );
        assert_eq!(partial_path(&libraries, "Y9S2_NewBlood", false), None);
        assert_eq!(installed_path(&libraries, "Y9S2_NewBlood", true), None);
        assert_eq!(
            partial_path(&libraries, "Y9S2_NewBlood", true),
            Some(second.join("Y9S2_HeatedMetal"))
        );
        assert_eq!(installed_path(&libraries, "Y1S0_Vanilla", false), None);
        assert_eq!(
            partial_path(&libraries, "Y1S0_Vanilla", false),
            Some(first.join("Y1S0_Vanilla"))
        );
        assert_eq!(installed_path(&libraries, "Y5S3_ShadowLegacy", false), None);
        assert_eq!(partial_path(&libraries, "Y5S3_ShadowLegacy", false), None);
        assert_eq!(partial_path(&[], "Y1S0_Vanilla", false), None);
        assert!(is_installed(&second.join("Y9S2_NewBlood")));
        assert!(!is_installed(&first.join("Y9S2_NewBlood")));
        fs::remove_file(second.join("Y9S2_NewBlood").join(TL_LAUNCHER)).unwrap();
        fs::create_dir(second.join("Y9S2_NewBlood").join(TL_LAUNCHER)).unwrap();
        assert!(is_installed(&second.join("Y9S2_NewBlood")));
    }

    #[test]
    fn season_entries_split_the_label() {
        let downloads = small();
        let heated = season_entry(&downloads[1], true);
        assert_eq!(
            heated,
            SeasonEntry {
                key: "Y5S3_ShadowLegacy".to_string(),
                id: "Y5S3_HeatedMetal".to_string(),
                hm: true,
                code: "Y5S3".to_string(),
                name: "Shadow Legacy".to_string(),
                size_gb: 50.0,
                build: "2".to_string(),
                hm_available: true,
                hm_beta: false,
                partial: false,
            }
        );
        assert_eq!(heated.title(true), "Y5S3 Heated Metal");
        assert_eq!(heated.title(false), "Y5S3 Shadow Legacy");
        let plain = season_entry(&downloads[0], false);
        assert_eq!(
            (plain.id.as_str(), plain.code.as_str(), plain.name.as_str()),
            ("Y1S0_Vanilla", "Y1S0", "Vanilla")
        );
        assert!(!plain.hm_available && !plain.hm_beta);
        let bare = season_entry(&downloads[3], false);
        assert_eq!((bare.code.as_str(), bare.name.as_str()), ("Y9S2", "Y9S2"));
        assert!(bare.hm_available && bare.hm_beta);
        let spaced = Download {
            label: "Y1S0 ".to_string(),
            ..downloads[0].clone()
        };
        let entry = season_entry(&spaced, false);
        assert_eq!(
            (entry.code.as_str(), entry.name.as_str()),
            ("Y1S0", "Y1S0 ")
        );
        let listed = seasons(&downloads);
        assert_eq!(listed.len(), 4);
        assert!(listed.iter().all(|entry| !entry.hm && !entry.partial));
        assert_eq!(listed[3].id, "Y9S2_NewBlood");
    }

    #[test]
    fn home_lists_installed_and_partial_editions_in_manifest_order() {
        let dir = tempfile::tempdir().unwrap();
        let downloads = small();
        let root = library(
            &dir,
            "root",
            &[
                ("Y9S2_HeatedMetal", true),
                ("Y9S2_NewBlood", false),
                ("Y5S4_HeatedMetal", false),
                ("Y1S0_HeatedMetal", true),
                ("Y1S0_Vanilla", true),
                ("Y7S1_Unknown", true),
            ],
        );
        let entries = home(&downloads, &[root]);
        let shown: Vec<(&str, bool, bool)> = entries
            .iter()
            .map(|entry| (entry.id.as_str(), entry.hm, entry.partial))
            .collect();
        assert_eq!(
            shown,
            [
                ("Y1S0_Vanilla", false, false),
                ("Y5S4_HeatedMetal", true, true),
                ("Y9S2_NewBlood", false, true),
                ("Y9S2_HeatedMetal", true, false),
            ]
        );
        assert!(home(&downloads, &[]).is_empty());
    }
}
