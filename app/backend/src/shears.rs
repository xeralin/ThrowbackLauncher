use std::collections::BTreeMap;
use std::fs::{self, DirEntry};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};

use crate::env::unlink;

const TEXTURE_QUALITIES: [&str; 5] = ["Low", "Medium", "High", "Very High", "Ultra"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier {
    pub level: usize,
    pub quality: &'static str,
    pub size: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scan {
    pub tiers: Vec<Tier>,
    pub videos: u64,
    pub events: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Videos,
    Events,
    Textures { above: usize },
}

pub fn folder_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in list(&dir).unwrap_or_default() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                pending.push(path);
            } else if let Ok(meta) = fs::symlink_metadata(&path) {
                total += meta.len();
            }
        }
    }
    total
}

pub fn scan_download(dir: &Path, event_pattern: Option<&str>) -> Result<Scan> {
    Ok(Scan {
        tiers: texture_tiers(dir)?,
        videos: files_size(&video_files(dir, event_pattern)?) + folder_size(&startup_dir(dir)),
        events: match event_pattern {
            Some(pattern) => files_size(&event_files(dir, pattern)?),
            None => 0,
        },
    })
}

pub fn cut_download(dir: &Path, kind: Kind, event_pattern: Option<&str>) -> Result<()> {
    match kind {
        Kind::Videos => {
            let deleted = delete_files(&video_files(dir, event_pattern)?);
            let startup = startup_dir(dir);
            let removed = match fs::remove_dir_all(&startup) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => {
                    Err(with_path(error, &startup))
                }
                _ => Ok(()),
            };
            deleted.and(removed)
        }
        Kind::Events => match event_pattern {
            Some(pattern) => delete_files(&event_files(dir, pattern)?),
            None => Ok(()),
        },
        Kind::Textures { above } => {
            let files: Vec<PathBuf> = texture_forges(dir)?
                .into_iter()
                .filter(|(_, level, _)| *level > above)
                .map(|(path, _, _)| path)
                .collect();
            delete_files(&files)
        }
    }
}

fn with_path(error: io::Error, path: &Path) -> anyhow::Error {
    anyhow!("{error}: '{}'", path.display())
}

fn list(dir: &Path) -> io::Result<Vec<DirEntry>> {
    fs::read_dir(dir)?.collect()
}

fn children(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = list(dir).map_err(|error| with_path(error, dir))?;
    Ok(entries.iter().map(DirEntry::path).collect())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn has_extension(path: &Path, names: &[&str]) -> bool {
    let extension = path.extension().unwrap_or_default();
    names
        .iter()
        .any(|name| extension.eq_ignore_ascii_case(name))
}

fn texture_level(path: &Path) -> Option<usize> {
    if !has_extension(path, &["forge"]) {
        return None;
    }
    let stem = path.file_stem()?.to_string_lossy();
    let level = stem
        .match_indices("_textures")
        .find_map(|(start, tag)| stem[start + tag.len()..].chars().next()?.to_digit(10))?
        as usize;
    (level < TEXTURE_QUALITIES.len()).then_some(level)
}

fn texture_forges(dir: &Path) -> Result<Vec<(PathBuf, usize, u64)>> {
    Ok(children(dir)?
        .into_iter()
        .filter_map(|path| {
            let level = texture_level(&path)?;
            let size = fs::metadata(&path).ok()?.len();
            Some((path, level, size))
        })
        .collect())
}

fn texture_tiers(dir: &Path) -> Result<Vec<Tier>> {
    let mut sizes = BTreeMap::new();
    for (_, level, size) in texture_forges(dir)? {
        *sizes.entry(level).or_insert(0) += size;
    }
    Ok(sizes
        .into_iter()
        .map(|(level, size)| Tier {
            level,
            quality: TEXTURE_QUALITIES[level],
            size,
        })
        .collect())
}

fn startup_dir(dir: &Path) -> PathBuf {
    dir.join("videos").join("startup")
}

fn videos(dir: &Path) -> Result<Vec<PathBuf>> {
    let folder = dir.join("videos");
    if !folder.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = children(&folder)?;
    files.retain(|path| path.is_file());
    Ok(files)
}

fn event_video(path: &Path, event_pattern: Option<&str>) -> bool {
    event_pattern == Some("fury") && {
        let name = file_name(path).to_lowercase();
        name.starts_with("fury_") || name.ends_with("_gameover.bik")
    }
}

fn video_files(dir: &Path, event_pattern: Option<&str>) -> Result<Vec<PathBuf>> {
    let mut files = videos(dir)?;
    files.retain(|path| !event_video(path, event_pattern));
    Ok(files)
}

fn files_size(files: &[PathBuf]) -> u64 {
    files
        .iter()
        .filter_map(|path| fs::metadata(path).ok())
        .map(|meta| meta.len())
        .sum()
}

fn event_files(dir: &Path, pattern: &str) -> Result<Vec<PathBuf>> {
    let mut files = children(dir)?;
    files.retain(|path| {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        has_extension(path, &["forge", "depgraphbin"])
            && stem.to_lowercase().contains(pattern)
            && path.is_file()
    });
    let mut event_videos = videos(dir)?;
    event_videos.retain(|path| event_video(path, Some(pattern)));
    files.extend(event_videos);
    Ok(files)
}

fn delete_files(files: &[PathBuf]) -> Result<()> {
    let mut failed = 0;
    let mut last = None;
    for path in files {
        if let Err(error) = unlink(path) {
            failed += 1;
            last = Some(with_path(error, path));
        }
    }
    match last {
        Some(last) => bail!("{failed} files not deleted, last: {last}"),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILES: &[(&str, usize)] = &[
        ("datapc64_merged_bnk_textures0.forge", 100),
        ("datapc64_merged_bnk_textures1.forge", 200),
        ("datapc64_merged_playgo_bnk_textures1.forge", 50),
        ("datapc64_merged_bnk_textures2.FORGE", 300),
        ("datapc64_merged_bnk_textures3.forge", 400),
        ("datapc64_tdm01_mtown_bnk_textures4.forge", 500),
        ("datapc64_merged_bnk_guitextures0.forge", 11),
        ("datapc64_merged_bnk_guitextures1.forge", 12),
        ("datapc64_merged_bnk_textures3.forge.bak", 13),
        ("datapc64_merged_bnk_textures5.forge", 17),
        ("datapc64_fury_a.forge", 1000),
        ("datapc64_FURY_b.DEPGRAPHBIN", 2000),
        ("datapc64_fury.txt", 5),
        ("datapc64_events_a.forge", 4000),
        ("datapc64_events_textures3.forge", 8000),
        ("datapc64_events_folder.forge/inside.bin", 9),
        ("datapc64_merged.forge", 16000),
        ("RainbowSix.exe", 32000),
        ("videos/intro.bik", 10),
        ("videos/training_m01.bik", 20),
        ("videos/fury_intro.bik", 1100),
        ("videos/FURY_Outro.BIK", 1200),
        ("videos/map_gameover.bik", 1300),
        ("videos/map_gameover.bik.bak", 40),
        ("videos/extra/kept.bik", 80),
        ("videos/startup/logo.bik", 160),
        ("videos/startup/lang/en.bik", 320),
        ("videos/startup/lang/deep/de.bik", 640),
    ];

    fn put(path: &Path, size: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![0; size]).unwrap();
    }

    fn game_folder(root: &Path) -> PathBuf {
        let game = root.join("Y3S1_Chimera");
        for &(name, size) in FILES {
            put(&game.join(name), size);
        }
        fs::create_dir_all(game.join("videos/startup/empty")).unwrap();
        game
    }

    fn files(game: &Path) -> Vec<String> {
        let mut found = Vec::new();
        let mut pending = vec![game.to_path_buf()];
        while let Some(dir) = pending.pop() {
            for path in children(&dir).unwrap() {
                if fs::symlink_metadata(&path).unwrap().is_dir() {
                    pending.push(path);
                } else {
                    let name = path.strip_prefix(game).unwrap().to_string_lossy();
                    found.push(name.replace('\\', "/"));
                }
            }
        }
        found.sort();
        found
    }

    fn removed(game: &Path, kind: Kind, event_pattern: Option<&str>) -> Vec<String> {
        let before = files(game);
        cut_download(game, kind, event_pattern).unwrap();
        let after = files(game);
        before
            .into_iter()
            .filter(|name| !after.contains(name))
            .collect()
    }

    fn tiers(sizes: [u64; 5]) -> Vec<Tier> {
        sizes
            .into_iter()
            .enumerate()
            .map(|(level, size)| Tier {
                level,
                quality: TEXTURE_QUALITIES[level],
                size,
            })
            .collect()
    }

    #[test]
    fn texture_level_needs_the_underscore_and_a_listed_digit() {
        for (name, level) in [
            ("datapc64_merged_bnk_textures0.forge", Some(0)),
            ("datapc64_merged_bnk_textures3.forge", Some(3)),
            ("datapc64_tdm01_mtown_bnk_textures4.forge", Some(4)),
            ("datapc64_merged_bnk_textures3.FoRgE", Some(3)),
            ("x_textures12.forge", Some(1)),
            ("x_textures_textures2.forge", Some(2)),
            ("x_texturesa_textures1.forge", Some(1)),
            ("x_textures1..forge", Some(1)),
            ("datapc64_merged_bnk_guitextures0.forge", None),
            ("datapc64_merged_bnk_guitextures1.forge", None),
            ("x_textures5.forge", None),
            ("x_textures9.forge", None),
            ("x_textures7_textures2.forge", None),
            ("x_textures.forge", None),
            ("x_TEXTURES3.forge", None),
            ("x_textures3.forge.bak", None),
            ("x_textures3.forge ", None),
            ("x_textures3", None),
            ("x_textures3.", None),
            (".forge", None),
        ] {
            assert_eq!(
                texture_level(Path::new("game").join(name).as_path()),
                level,
                "{name}"
            );
        }
    }

    #[test]
    fn event_videos_exist_for_fury_only() {
        for (name, event) in [
            ("fury_intro.bik", true),
            ("FURY_Outro.BIK", true),
            ("fury_", true),
            ("map_gameover.bik", true),
            ("MAP2_GameOver.Bik", true),
            ("_gameover.bik", true),
            ("map_gameover.bik\n", false),
            ("map_gameover.bik.bak", false),
            ("map_gameover.bk2", false),
            ("xfury_a.bik", false),
            ("\nfury_a.bik", false),
            ("fury.bik", false),
            ("intro.bik", false),
        ] {
            let path = Path::new("videos").join(name);
            assert_eq!(event_video(&path, Some("fury")), event, "{name:?}");
            assert!(!event_video(&path, Some("events")), "{name:?}");
            assert!(!event_video(&path, None), "{name:?}");
        }
    }

    #[test]
    fn folder_size_sums_every_file_below_the_folder() {
        let root = tempfile::tempdir().unwrap();
        let game = game_folder(root.path());
        assert_eq!(folder_size(&game), 69487);
        assert_eq!(folder_size(&game.join("videos")), 4870);
        assert_eq!(folder_size(&game.join("videos/startup/empty")), 0);
        assert_eq!(folder_size(&game.join("missing")), 0);
        assert_eq!(folder_size(&game.join("RainbowSix.exe")), 0);
    }

    #[cfg(unix)]
    #[test]
    fn folder_size_counts_links_as_links() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("game");
        put(&root.path().join("outside/big.bin"), 5000);
        put(&game.join("real.bin"), 7);
        symlink("../outside/big.bin", game.join("file_link")).unwrap();
        symlink("../outside", game.join("folder_link")).unwrap();
        symlink("missing", game.join("dangling")).unwrap();
        assert_eq!(folder_size(&game), 7 + 18 + 10 + 7);
        assert_eq!(folder_size(&game.join("folder_link")), 5000);
    }

    #[test]
    fn scan_sums_texture_tiers_videos_and_events() {
        let root = tempfile::tempdir().unwrap();
        let game = game_folder(root.path());
        let plain = Scan {
            tiers: tiers([100, 250, 300, 8400, 500]),
            videos: 4790,
            events: 0,
        };
        assert_eq!(scan_download(&game, None).unwrap(), plain);
        assert_eq!(
            scan_download(&game, Some("fury")).unwrap(),
            Scan {
                videos: 1190,
                events: 6600,
                ..plain.clone()
            }
        );
        assert_eq!(
            scan_download(&game, Some("events")).unwrap(),
            Scan {
                events: 12000,
                ..plain
            }
        );
    }

    #[test]
    fn scan_lists_only_levels_that_exist() {
        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("game");
        put(&game.join("b_textures3.forge"), 0);
        put(&game.join("a_textures1.forge"), 9);
        fs::create_dir(game.join("folder_textures1.forge")).unwrap();
        let folder = fs::metadata(game.join("folder_textures1.forge"))
            .unwrap()
            .len();
        let scan = scan_download(&game, Some("events")).unwrap();
        assert_eq!(
            scan.tiers,
            [
                Tier {
                    level: 1,
                    quality: "Medium",
                    size: 9 + folder
                },
                Tier {
                    level: 3,
                    quality: "Very High",
                    size: 0
                },
            ]
        );
        assert_eq!((scan.videos, scan.events), (0, 0));
    }

    #[test]
    fn scan_fails_without_the_game_folder() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");
        let error = scan_download(&missing, Some("events")).unwrap_err();
        assert!(
            error
                .to_string()
                .ends_with(&format!(": '{}'", missing.display())),
            "{error}"
        );
        assert!(cut_download(&missing, Kind::Textures { above: 0 }, None).is_err());
        assert!(cut_download(&missing, Kind::Events, Some("events")).is_err());
        assert!(cut_download(&missing, Kind::Events, None).is_ok());
        assert!(cut_download(&missing, Kind::Videos, None).is_ok());
    }

    #[test]
    fn cut_textures_removes_the_levels_above() {
        let root = tempfile::tempdir().unwrap();
        let game = game_folder(root.path());
        assert_eq!(removed(&game, Kind::Textures { above: 4 }, None), [""; 0]);
        assert_eq!(
            removed(&game, Kind::Textures { above: 3 }, None),
            ["datapc64_tdm01_mtown_bnk_textures4.forge"]
        );
        assert_eq!(
            removed(&game, Kind::Textures { above: 1 }, Some("events")),
            [
                "datapc64_events_textures3.forge",
                "datapc64_merged_bnk_textures2.FORGE",
                "datapc64_merged_bnk_textures3.forge",
            ]
        );
        assert_eq!(
            removed(&game, Kind::Textures { above: 0 }, None),
            [
                "datapc64_merged_bnk_textures1.forge",
                "datapc64_merged_playgo_bnk_textures1.forge",
            ]
        );
        assert_eq!(
            scan_download(&game, None).unwrap().tiers,
            [Tier {
                level: 0,
                quality: "Low",
                size: 100
            }]
        );
    }

    #[test]
    fn cut_videos_keeps_event_videos_and_drops_the_startup_folder() {
        let root = tempfile::tempdir().unwrap();
        let game = game_folder(root.path());
        assert_eq!(
            removed(&game, Kind::Videos, Some("fury")),
            [
                "videos/intro.bik",
                "videos/map_gameover.bik.bak",
                "videos/startup/lang/deep/de.bik",
                "videos/startup/lang/en.bik",
                "videos/startup/logo.bik",
                "videos/training_m01.bik",
            ]
        );
        assert!(!game.join("videos/startup").exists());
        assert_eq!(
            removed(&game, Kind::Videos, Some("events")),
            [
                "videos/FURY_Outro.BIK",
                "videos/fury_intro.bik",
                "videos/map_gameover.bik",
            ]
        );
        assert_eq!(removed(&game, Kind::Videos, None), [""; 0]);
        assert_eq!(scan_download(&game, None).unwrap().videos, 0);
        assert!(game.join("videos/extra/kept.bik").is_file());
    }

    #[test]
    fn cut_events_removes_event_files_and_event_videos() {
        let root = tempfile::tempdir().unwrap();
        let game = game_folder(root.path());
        assert_eq!(removed(&game, Kind::Events, None), [""; 0]);
        assert_eq!(
            removed(&game, Kind::Events, Some("fury")),
            [
                "datapc64_FURY_b.DEPGRAPHBIN",
                "datapc64_fury_a.forge",
                "videos/FURY_Outro.BIK",
                "videos/fury_intro.bik",
                "videos/map_gameover.bik",
            ]
        );
        assert_eq!(
            removed(&game, Kind::Events, Some("events")),
            ["datapc64_events_a.forge", "datapc64_events_textures3.forge",]
        );
        assert_eq!(scan_download(&game, Some("events")).unwrap().events, 0);
        assert!(
            game.join("datapc64_events_folder.forge/inside.bin")
                .is_file()
        );
    }

    #[test]
    fn delete_tries_every_file_and_reports_the_count_and_the_last_error() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        let file = root.path().join("file.bin");
        let second = root.path().join("second");
        fs::create_dir(&first).unwrap();
        put(&file, 3);
        fs::create_dir(&second).unwrap();
        let error = delete_files(&[first.clone(), file.clone(), second.clone()]).unwrap_err();
        let last = with_path(fs::remove_file(&second).unwrap_err(), &second);
        assert_eq!(
            error.to_string(),
            format!("2 files not deleted, last: {last}")
        );
        assert!(!file.exists());
        assert!(first.is_dir() && second.is_dir());
        let error = delete_files(&[file, first.clone()]).unwrap_err();
        let last = with_path(fs::remove_file(&first).unwrap_err(), &first);
        assert_eq!(
            error.to_string(),
            format!("2 files not deleted, last: {last}")
        );
        delete_files(&[]).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn scan_sizes_linked_videos_and_event_files_by_their_targets() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("game");
        put(&root.path().join("outside/target.bin"), 31);
        put(&game.join("videos/plain.bik"), 2);
        put(&game.join("plain_events.forge"), 3);
        symlink("../../outside/target.bin", game.join("videos/link.bik")).unwrap();
        symlink("../outside/target.bin", game.join("link_events.forge")).unwrap();
        symlink("missing", game.join("videos/dangling.bik")).unwrap();
        symlink("missing", game.join("dangling_events.forge")).unwrap();
        let scan = scan_download(&game, Some("events")).unwrap();
        assert_eq!((scan.videos, scan.events), (2 + 31, 3 + 31));
    }

    #[test]
    fn cut_fails_on_a_folder_named_like_a_texture_file() {
        let root = tempfile::tempdir().unwrap();
        let game = game_folder(root.path());
        let folder = game.join("folder_textures4.forge");
        put(&folder.join("inside.bin"), 1);
        let error = cut_download(&game, Kind::Textures { above: 2 }, None).unwrap_err();
        let text = error.to_string();
        assert!(text.starts_with("1 files not deleted, last: "), "{text}");
        assert!(
            text.ends_with(&format!(": '{}'", folder.display())),
            "{text}"
        );
        assert!(folder.join("inside.bin").is_file());
        assert_eq!(
            scan_download(&game, None)
                .unwrap()
                .tiers
                .last()
                .map(|tier| tier.level),
            Some(4)
        );
        assert!(!game.join("datapc64_merged_bnk_textures3.forge").exists());
        assert!(
            !game
                .join("datapc64_tdm01_mtown_bnk_textures4.forge")
                .exists()
        );
        assert!(game.join("datapc64_merged_bnk_textures2.FORGE").exists());
    }

    #[cfg(unix)]
    #[test]
    fn names_outside_unicode_still_match() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("game");
        put(
            &game.join(OsStr::from_bytes(b"bad\xff_textures2.forge")),
            71,
        );
        put(
            &game.join(OsStr::from_bytes(b"bad\xff_events.depgraphbin")),
            73,
        );
        assert_eq!(
            scan_download(&game, Some("events")).unwrap(),
            Scan {
                tiers: vec![Tier {
                    level: 2,
                    quality: "High",
                    size: 71
                }],
                videos: 0,
                events: 73,
            }
        );
        cut_download(&game, Kind::Textures { above: 1 }, None).unwrap();
        cut_download(&game, Kind::Events, Some("events")).unwrap();
        assert!(children(&game).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn cut_removes_links_and_never_their_targets() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("game");
        let target = root.path().join("outside/target.bin");
        put(&target, 31);
        put(&root.path().join("outside/folder/inner.bik"), 77);
        put(&game.join("videos/startup/logo.bik"), 1);
        symlink(&target, game.join("link_textures2.forge")).unwrap();
        symlink("missing", game.join("dangling_textures3.forge")).unwrap();
        symlink(&target, game.join("videos/link.bik")).unwrap();
        symlink(&target, game.join("videos/startup/link")).unwrap();
        symlink(
            "../../../outside/folder",
            game.join("videos/startup/folder"),
        )
        .unwrap();
        let scan = scan_download(&game, None).unwrap();
        assert_eq!(
            scan.tiers,
            [Tier {
                level: 2,
                quality: "High",
                size: 31
            }]
        );
        assert_eq!(
            removed(&game, Kind::Textures { above: 0 }, None),
            ["link_textures2.forge"]
        );
        assert_eq!(
            removed(&game, Kind::Videos, None),
            [
                "videos/link.bik",
                "videos/startup/folder",
                "videos/startup/link",
                "videos/startup/logo.bik",
            ]
        );
        assert_eq!(files(&game), ["dangling_textures3.forge"]);
        assert_eq!(folder_size(&root.path().join("outside")), 31 + 77);
    }

    #[cfg(unix)]
    #[test]
    fn cut_videos_removes_a_linked_startup_folder_and_keeps_its_target() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("game");
        let elsewhere = root.path().join("elsewhere");
        put(&game.join("videos/intro.bik"), 64);
        put(&elsewhere.join("logo.bik"), 8);
        put(&elsewhere.join("deep/en.bik"), 16);
        symlink(&elsewhere, game.join("videos/startup")).unwrap();
        assert_eq!(scan_download(&game, None).unwrap().videos, 64 + 8 + 16);
        cut_download(&game, Kind::Videos, None).unwrap();
        assert!(files(&game).is_empty());
        assert!(game.join("videos").is_dir());
        assert_eq!(folder_size(&elsewhere), 8 + 16);
        cut_download(&game, Kind::Videos, None).unwrap();
    }
}
