use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::env::{Paths, write_atomic};
use crate::log::fail;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Played {
    #[serde(skip_serializing_if = "unset")]
    pub sessions: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<i64>,
    #[serde(skip_serializing_if = "unset")]
    pub until: i64,
    #[serde(skip_serializing_if = "unset")]
    pub minutes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    #[serde(flatten)]
    pub base: Played,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hm: Option<Played>,
}

pub type Playtime = BTreeMap<String, Entry>;

fn unset<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

impl Entry {
    pub fn edition(&mut self, hm: bool) -> &mut Played {
        if hm {
            self.hm.get_or_insert_default()
        } else {
            &mut self.base
        }
    }
}

pub fn read_playtime(paths: &Paths) -> Playtime {
    let read = match fs::read_to_string(paths.playtime_file()) {
        Ok(text) => toml::from_str(&text).map_err(|error| error.message().to_owned()),
        Err(error) if error.kind() == ErrorKind::NotFound => return Playtime::new(),
        Err(error) => Err(error.to_string()),
    };
    read.unwrap_or_else(|detail| {
        fail("Play time read failed", detail);
        Playtime::new()
    })
}

pub fn write_playtime(paths: &Paths, playtime: &Playtime) -> Result<()> {
    Ok(write_atomic(
        &paths.playtime_file(),
        &toml::to_string(playtime)?,
    )?)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::log::take_recorded;

    fn sandbox() -> (TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        fs::create_dir_all(&paths.data_root).unwrap();
        (dir, paths)
    }

    fn played(sessions: u64, since: Option<i64>, until: i64, minutes: u64) -> Played {
        Played {
            sessions,
            since,
            until,
            minutes,
        }
    }

    #[test]
    fn a_missing_file_is_an_empty_table() {
        let (_dir, paths) = sandbox();
        assert!(read_playtime(&paths).is_empty());
        assert!(take_recorded().is_empty());
    }

    #[test]
    fn a_file_of_the_python_launcher_is_read_per_edition() {
        let (_dir, paths) = sandbox();
        fs::write(
            paths.playtime_file(),
            "[Y5S3_ShadowLegacy]\nsessions = 3\nsince = 100\nuntil = 900\nminutes = 12\n\n\
             [Y9S2_NewBlood.hm]\nminutes = 7\nuntil = 500\n\n\
             [Y1S0_Vanilla]\nminutes = 4\nfirst = 5\n\n[days]\na = 1\n",
        )
        .unwrap();
        let playtime = read_playtime(&paths);
        assert_eq!(
            playtime["Y5S3_ShadowLegacy"],
            Entry {
                base: played(3, Some(100), 900, 12),
                hm: None
            }
        );
        assert_eq!(
            playtime["Y9S2_NewBlood"],
            Entry {
                base: Played::default(),
                hm: Some(played(0, None, 500, 7))
            }
        );
        assert_eq!(playtime["Y1S0_Vanilla"].base, played(0, None, 0, 4));
        assert_eq!(playtime["days"], Entry::default());
        assert!(take_recorded().is_empty());
    }

    #[test]
    fn a_broken_file_is_logged_and_read_as_empty() {
        let (_dir, paths) = sandbox();
        fs::write(paths.playtime_file(), "[Y5S3_ShadowLegacy\nminutes = 1\n").unwrap();
        assert!(read_playtime(&paths).is_empty());
        let lines = take_recorded();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("Play time read failed - "),
            "{lines:?}"
        );
        assert!(paths.playtime_file().exists());
    }

    #[test]
    fn only_played_editions_are_written_and_read_back() {
        let (_dir, paths) = sandbox();
        let mut playtime = Playtime::new();
        *playtime
            .entry("Y9S2_NewBlood".to_owned())
            .or_default()
            .edition(true) = played(1, Some(40), 100, 2);
        *playtime
            .entry("Y1S0_Vanilla".to_owned())
            .or_default()
            .edition(false) = played(2, None, 50, 9);
        let both = playtime.entry("Y5S3_ShadowLegacy".to_owned()).or_default();
        *both.edition(false) = played(3, Some(10), 900, 12);
        *both.edition(true) = played(1, Some(20), 950, 1);
        write_playtime(&paths, &playtime).unwrap();
        assert_eq!(
            fs::read_to_string(paths.playtime_file()).unwrap(),
            "[Y1S0_Vanilla]\nsessions = 2\nuntil = 50\nminutes = 9\n\n\
             [Y5S3_ShadowLegacy]\nsessions = 3\nsince = 10\nuntil = 900\nminutes = 12\n\n\
             [Y5S3_ShadowLegacy.hm]\nsessions = 1\nsince = 20\nuntil = 950\nminutes = 1\n\n\
             [Y9S2_NewBlood.hm]\nsessions = 1\nsince = 40\nuntil = 100\nminutes = 2\n"
        );
        assert_eq!(read_playtime(&paths), playtime);
    }
}
