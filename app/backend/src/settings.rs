use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::Serialize;
use toml::{Table, Value};

use crate::env::{
    DOWNLOADS_MAX, DOWNLOADS_MIN, MAX_USERNAME_LENGTH, Paths, SCALE_LEVELS, resolve, same_path,
    write_atomic,
};
use crate::log::fail;

const DEFAULT_USERNAME: &str = "ThrowbackUser";

const DEFAULT_MAX_DOWNLOADS: u32 = 25;
const DEFAULT_SCALE: u32 = 100;
pub const USERNAME_EMPTY: &str = "Username is empty";
pub const LAUNCH_ARGS_UNCLOSED: &str = "Launch options have an unclosed quote or escape";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Flag {
    DiscordRpc,
    CloseToTray,
    ReduceMotion,
    LiberatorEnabled,
    RvpnAutorun,
    BarNyan,
}

impl Flag {
    fn key(self) -> &'static str {
        match self {
            Flag::DiscordRpc => "discord_rpc",
            Flag::CloseToTray => "close_to_tray",
            Flag::ReduceMotion => "reduce_motion",
            Flag::LiberatorEnabled => "liberator_enabled",
            Flag::RvpnAutorun => "rvpn_autorun",
            Flag::BarNyan => "bar_nyan",
        }
    }

    fn default(self) -> bool {
        matches!(self, Flag::DiscordRpc | Flag::LiberatorEnabled)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Color {
    Accent,
    Background,
    BarFill,
    BarStripe,
}

impl Color {
    fn key(self) -> &'static str {
        match self {
            Color::Accent => "accent",
            Color::Background => "background",
            Color::BarFill => "bar_fill",
            Color::BarStripe => "bar_stripe",
        }
    }

    pub const fn default(self) -> u32 {
        match self {
            Color::Accent => 0xc0152a,
            Color::Background => 0x0d0d0f,
            Color::BarFill => 0xc388e3,
            Color::BarStripe => 0xdcbaef,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Entries {
    HomeSizes,
    LaunchArgs,
    SeasonProton,
}

impl Entries {
    fn key(self) -> &'static str {
        match self {
            Entries::HomeSizes => "home_sizes",
            Entries::LaunchArgs => "launch_args",
            Entries::SeasonProton => "season_proton",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueEntry {
    pub key: String,
    pub hm: bool,
    pub library: String,
    pub verify: bool,
}

#[derive(Serialize)]
struct Document<'a> {
    settings: &'a Table,
}

pub struct Settings {
    file: PathBuf,
    fixed_library: PathBuf,
    table: Table,
    libraries: Vec<PathBuf>,
    warning: Option<String>,
}

pub fn clean_username(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || "_.-".contains(*character))
        .collect()
}

fn parse(text: &str) -> Result<Table, String> {
    let mut document: Table = toml::from_str(text).map_err(|error| error.message().to_owned())?;
    match document.remove("settings") {
        Some(Value::Table(table)) => Ok(table),
        Some(other) => Err(format!("settings is not a table but {}", other.type_str())),
        None => Ok(Table::new()),
    }
}

fn back_up(file: &Path) -> String {
    let broken = file.with_added_extension("broken");
    match fs::rename(file, &broken) {
        Ok(()) => format!(
            "Settings were malformed, backup saved as {}",
            broken.file_name().unwrap_or_default().to_string_lossy()
        ),
        Err(error) => {
            fail("Settings backup failed", error);
            "Settings were malformed and reset".to_owned()
        }
    }
}

pub fn parse_color(text: &str) -> Option<u32> {
    let digits = text.strip_prefix('#')?;
    let plain = digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_hexdigit());
    plain
        .then(|| u32::from_str_radix(digits, 16).ok())
        .flatten()
}

pub fn color_text(value: u32) -> String {
    format!("#{value:06x}")
}

impl Settings {
    pub fn load(paths: &Paths) -> Result<Self> {
        let file = paths.settings_file();
        let malformed = |detail: String| {
            fail("Settings were malformed", detail);
            (Table::new(), Some(back_up(&file)))
        };
        let (table, warning) = match fs::read_to_string(&file) {
            Ok(text) => match parse(&text) {
                Ok(table) => (table, None),
                Err(detail) => malformed(detail),
            },
            Err(error) if error.kind() == ErrorKind::NotFound => (Table::new(), None),
            Err(error) if error.kind() == ErrorKind::InvalidData => malformed(error.to_string()),
            Err(error) => bail!(fail("Settings read failed", error)),
        };
        let mut settings = Self {
            file,
            fixed_library: paths.default_downloads_dir(),
            table,
            libraries: Vec::new(),
            warning,
        };
        settings.libraries = settings.resolved(settings.stored_libraries());
        Ok(settings)
    }

    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    fn save(&self) {
        let document = Document {
            settings: &self.table,
        };
        let written = toml::to_string(&document)
            .map_err(anyhow::Error::from)
            .and_then(|text| Ok(write_atomic(&self.file, &text)?));
        if let Err(error) = written {
            fail("Settings save failed", format!("{error:#}"));
        }
    }

    fn put(&mut self, key: &str, value: Value, default: Value) {
        if value == default {
            self.table.remove(key);
        } else {
            self.table.insert(key.to_owned(), value);
        }
        self.save();
    }

    fn text(&self, key: &str) -> Option<&str> {
        self.table.get(key).and_then(Value::as_str)
    }

    fn whole(&self, key: &str) -> Option<u32> {
        let number = self.table.get(key).and_then(Value::as_integer)?;
        u32::try_from(number).ok()
    }

    pub fn username(&self) -> &str {
        self.text("username").unwrap_or(DEFAULT_USERNAME)
    }

    pub fn set_username(&mut self, value: &str) -> Result<bool, &'static str> {
        let value = clean_username(value);
        if value.is_empty() {
            return Err(USERNAME_EMPTY);
        }
        if value.len() > MAX_USERNAME_LENGTH || value == self.username() {
            return Ok(false);
        }
        self.put("username", value.into(), DEFAULT_USERNAME.into());
        Ok(true)
    }

    pub fn steam_account(&self) -> &str {
        self.text("steam_account").unwrap_or_default()
    }

    pub fn set_steam_account(&mut self, value: &str) -> bool {
        let value = value.trim();
        if value == self.steam_account() {
            return false;
        }
        self.put("steam_account", value.into(), "".into());
        true
    }

    pub fn max_downloads(&self) -> u32 {
        self.whole("max_downloads")
            .filter(|value| (DOWNLOADS_MIN..=DOWNLOADS_MAX).contains(value))
            .unwrap_or(DEFAULT_MAX_DOWNLOADS)
    }

    pub fn set_max_downloads(&mut self, value: u32) -> bool {
        if !(DOWNLOADS_MIN..=DOWNLOADS_MAX).contains(&value) || value == self.max_downloads() {
            return false;
        }
        self.put("max_downloads", value.into(), DEFAULT_MAX_DOWNLOADS.into());
        true
    }

    pub fn scale(&self) -> u32 {
        self.whole("scale")
            .filter(|value| SCALE_LEVELS.contains(value))
            .unwrap_or(DEFAULT_SCALE)
    }

    pub fn set_scale(&mut self, value: u32) -> bool {
        if !SCALE_LEVELS.contains(&value) || value == self.scale() {
            return false;
        }
        self.put("scale", value.into(), DEFAULT_SCALE.into());
        true
    }

    pub fn flag(&self, flag: Flag) -> bool {
        let stored = self.table.get(flag.key()).and_then(Value::as_bool);
        stored.unwrap_or(flag.default())
    }

    pub fn set_flag(&mut self, flag: Flag, value: bool) -> bool {
        if value == self.flag(flag) {
            return false;
        }
        self.put(flag.key(), value.into(), flag.default().into());
        true
    }

    pub fn color(&self, color: Color) -> u32 {
        let stored = self.text(color.key()).and_then(parse_color);
        stored.unwrap_or(color.default())
    }

    pub fn set_color(&mut self, color: Color, value: u32) -> bool {
        if value == self.color(color) {
            return false;
        }
        let (value, default) = (color_text(value), color_text(color.default()));
        self.put(color.key(), value.into(), default.into());
        true
    }

    pub fn home_order(&self) -> Vec<String> {
        let items = self.table.get("home_order").and_then(Value::as_array);
        let names = items.into_iter().flatten().filter_map(Value::as_str);
        names.map(str::to_owned).collect()
    }

    pub fn set_home_order(&mut self, order: Vec<String>) -> bool {
        if order == self.home_order() {
            return false;
        }
        self.put("home_order", order.into(), Value::Array(Vec::new()));
        true
    }

    pub fn entries(&self, entries: Entries) -> Vec<(String, String)> {
        let table = self.table.get(entries.key()).and_then(Value::as_table);
        table
            .into_iter()
            .flatten()
            .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
            .collect()
    }

    fn put_entries(&mut self, entries: Entries, items: Vec<(String, String)>) {
        let table: Table = items
            .into_iter()
            .map(|(key, value)| (key, Value::String(value)))
            .collect();
        self.put(
            entries.key(),
            Value::Table(table),
            Value::Table(Table::new()),
        );
    }

    pub fn set_entry(&mut self, entries: Entries, entry: &str, value: &str) -> bool {
        let mut items = self.entries(entries);
        let position = items.iter().position(|(key, _)| key == entry);
        match (position, value.is_empty()) {
            (None, true) => return false,
            (Some(index), true) => {
                items.remove(index);
            }
            (Some(index), false) if items[index].1 == value => return false,
            (Some(index), false) => items[index].1 = value.to_owned(),
            (None, false) => items.push((entry.to_owned(), value.to_owned())),
        }
        self.put_entries(entries, items);
        true
    }

    pub fn entry(&self, entries: Entries, entry: &str) -> &str {
        let table = self.table.get(entries.key()).and_then(Value::as_table);
        let value = table.and_then(|table| table.get(entry));
        value.and_then(Value::as_str).unwrap_or_default()
    }

    pub fn proton(&self) -> &str {
        self.text("proton").unwrap_or_default()
    }

    pub fn set_proton(&mut self, internal: &str) -> bool {
        if internal == self.proton() {
            return false;
        }
        self.put("proton", internal.into(), "".into());
        true
    }

    pub fn set_launch_args(&mut self, key: &str, value: &str) -> Result<bool, &'static str> {
        let value = value.trim();
        if shlex::split(value).is_none() {
            return Err(LAUNCH_ARGS_UNCLOSED);
        }
        Ok(self.set_entry(Entries::LaunchArgs, key, value))
    }

    pub fn rename_home_entry(&mut self, old: &str, new: &str) -> bool {
        let renamed = |key: String| if key == old { new.to_owned() } else { key };
        let order = self.home_order().into_iter().filter(|key| key != new);
        let order_changed = self.set_home_order(order.map(renamed).collect());
        let before = self.entries(Entries::HomeSizes);
        let sizes = before.iter().filter(|(key, _)| key != new).cloned();
        let sizes: Vec<_> = sizes.map(|(key, size)| (renamed(key), size)).collect();
        let sizes_changed = sizes != before;
        if sizes_changed {
            self.put_entries(Entries::HomeSizes, sizes);
        }
        order_changed || sizes_changed
    }

    pub fn reset_home_layout(&mut self) -> bool {
        let removed = [
            self.table.remove("home_order"),
            self.table.remove("home_sizes"),
        ];
        let changed = removed.iter().any(Option::is_some);
        if changed {
            self.save();
        }
        changed
    }

    fn stored_libraries(&self) -> Vec<PathBuf> {
        let items = self.table.get("libraries").and_then(Value::as_array);
        let folders = items.into_iter().flatten().filter_map(Value::as_str);
        folders.map(PathBuf::from).collect()
    }

    fn resolved(&self, folders: Vec<PathBuf>) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = Vec::new();
        let named = folders
            .iter()
            .filter(|folder| !folder.as_os_str().is_empty());
        for root in named.map(|folder| resolve(folder)) {
            if !roots.iter().any(|known| same_path(known, &root)) {
                roots.push(root);
            }
        }
        if !roots
            .iter()
            .any(|root| same_path(root, &self.fixed_library))
        {
            roots.push(self.fixed_library.clone());
        }
        roots
    }

    pub fn libraries(&self) -> &[PathBuf] {
        &self.libraries
    }

    pub fn fixed_library(&self) -> &Path {
        &self.fixed_library
    }

    pub fn is_library(&self, root: &Path) -> bool {
        self.libraries
            .iter()
            .any(|library| same_path(library, root))
    }

    pub fn set_libraries(&mut self, roots: Vec<PathBuf>) {
        let stored: Vec<String> = roots
            .iter()
            .map(|root| root.to_string_lossy().into_owned())
            .collect();
        self.put("libraries", stored.into(), Value::Array(Vec::new()));
        self.libraries = self.resolved(roots);
    }

    pub fn queue(&self) -> Vec<QueueEntry> {
        let items = self.table.get("queue").and_then(Value::as_array);
        let entries = items.into_iter().flatten().filter_map(Value::as_table);
        entries
            .filter_map(|entry| {
                let flag = |name: &str| entry.get(name).and_then(Value::as_bool).unwrap_or(false);
                let library = entry.get("library").and_then(Value::as_str);
                Some(QueueEntry {
                    key: entry.get("key")?.as_str()?.to_owned(),
                    hm: flag("hm"),
                    library: library.unwrap_or_default().to_owned(),
                    verify: flag("verify"),
                })
            })
            .collect()
    }

    pub fn set_queue(&mut self, entries: &[QueueEntry]) -> bool {
        if entries == self.queue() {
            return false;
        }
        let items = entries.iter().map(|entry| {
            let mut item = Table::new();
            item.insert("key".to_owned(), entry.key.as_str().into());
            item.insert("hm".to_owned(), entry.hm.into());
            item.insert("library".to_owned(), entry.library.as_str().into());
            item.insert("verify".to_owned(), entry.verify.into());
            Value::Table(item)
        });
        self.put(
            "queue",
            Value::Array(items.collect()),
            Value::Array(Vec::new()),
        );
        true
    }

    pub fn queue_paused(&self) -> bool {
        let stored = self.table.get("queue_paused").and_then(Value::as_bool);
        stored.unwrap_or(false)
    }

    pub fn set_queue_paused(&mut self, paused: bool) -> bool {
        if paused == self.queue_paused() {
            return false;
        }
        self.put("queue_paused", paused.into(), false.into());
        true
    }

    pub fn hm_archive(&self) -> &str {
        self.text("hm_archive").unwrap_or_default()
    }

    pub fn set_hm_archive(&mut self, name: &str) -> bool {
        if name == self.hm_archive() {
            return false;
        }
        self.put("hm_archive", name.into(), "".into());
        true
    }
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

    fn loaded(paths: &Paths, content: &str) -> Settings {
        fs::write(paths.settings_file(), content).unwrap();
        Settings::load(paths).unwrap()
    }

    fn stored(paths: &Paths) -> String {
        fs::read_to_string(paths.settings_file()).unwrap()
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let (_dir, paths) = sandbox();
        let settings = Settings::load(&paths).unwrap();
        assert_eq!(settings.username(), "ThrowbackUser");
        assert_eq!(settings.steam_account(), "");
        assert_eq!(settings.max_downloads(), 25);
        assert_eq!(settings.scale(), 100);
        assert!(settings.flag(Flag::DiscordRpc));
        assert!(settings.flag(Flag::LiberatorEnabled));
        assert!(!settings.flag(Flag::CloseToTray));
        assert_eq!(settings.color(Color::Accent), 0xc0152a);
        assert_eq!(settings.libraries(), [paths.default_downloads_dir()]);
        assert_eq!(settings.warning(), None);
        assert!(!paths.settings_file().exists());
        assert!(take_recorded().is_empty());
    }

    #[test]
    fn a_file_written_by_the_python_launcher_is_read() {
        let (_dir, paths) = sandbox();
        let settings = loaded(
            &paths,
            "[settings]\nusername = 'Ash'\nqueue = [{ key = 'Y5S3_ShadowLegacy', hm = false }]\n\
             season_proton = { Y5S3_ShadowLegacy = 'proton_11' }\nsteam_account = 'ash'\n\
             bar_nyan = true\nmax_downloads = 15\naccent = '#0a84ff'\nhome_order = ['a', 'b']\n",
        );
        assert_eq!(settings.username(), "Ash");
        assert_eq!(settings.steam_account(), "ash");
        assert_eq!(settings.max_downloads(), 15);
        assert!(settings.flag(Flag::BarNyan));
        assert_eq!(settings.color(Color::Accent), 0x0a84ff);
        assert_eq!(settings.home_order(), ["a", "b"]);
        assert_eq!(
            settings.entries(Entries::SeasonProton),
            [("Y5S3_ShadowLegacy".to_owned(), "proton_11".to_owned())]
        );
    }

    #[test]
    fn values_of_the_wrong_type_fall_back_to_the_default() {
        let (_dir, paths) = sandbox();
        let settings = loaded(
            &paths,
            "[settings]\nusername = 5\nmax_downloads = 'many'\nscale = 99\ndiscord_rpc = 'no'\n\
             accent = 'red'\nhome_order = 'a'\nlibraries = 3\n",
        );
        assert_eq!(settings.username(), "ThrowbackUser");
        assert_eq!(settings.max_downloads(), 25);
        assert_eq!(settings.scale(), 100);
        assert!(settings.flag(Flag::DiscordRpc));
        assert_eq!(settings.color(Color::Accent), 0xc0152a);
        assert!(settings.home_order().is_empty());
        assert_eq!(settings.libraries(), [paths.default_downloads_dir()]);
    }

    #[test]
    fn a_changed_value_is_saved_and_every_other_key_stays() {
        let (_dir, paths) = sandbox();
        let mut settings = loaded(
            &paths,
            "[settings]\nusername = 'Ash'\nhm_archive = '/x.7z'\nqueue_paused = true\n",
        );
        assert!(settings.set_max_downloads(40));
        assert!(!settings.set_max_downloads(40));
        assert!(!settings.set_max_downloads(0));
        let reread = Settings::load(&paths).unwrap();
        assert_eq!(reread.max_downloads(), 40);
        assert_eq!(reread.username(), "Ash");
        let table: Table = toml::from_str(&stored(&paths)).unwrap();
        let keys: Vec<&String> = table["settings"].as_table().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["username", "hm_archive", "queue_paused", "max_downloads"]
        );
    }

    #[test]
    fn a_value_set_back_to_its_default_leaves_the_file() {
        let (_dir, paths) = sandbox();
        let mut settings = loaded(&paths, "[settings]\nscale = 125\ndiscord_rpc = false\n");
        assert!(settings.set_scale(100));
        assert!(settings.set_flag(Flag::DiscordRpc, true));
        assert!(!settings.set_flag(Flag::DiscordRpc, true));
        assert_eq!(stored(&paths), "[settings]\n");
    }

    #[test]
    fn usernames_are_cleaned_and_bounded() {
        let (_dir, paths) = sandbox();
        let mut settings = Settings::load(&paths).unwrap();
        assert_eq!(settings.set_username("  "), Err(USERNAME_EMPTY));
        assert_eq!(settings.set_username("ThrowbackUser"), Ok(false));
        assert_eq!(settings.set_username("a-very-long-user-name"), Ok(false));
        assert_eq!(settings.set_username("A sh!"), Ok(true));
        assert_eq!(settings.username(), "Ash");
        assert_eq!(stored(&paths), "[settings]\nusername = \"Ash\"\n");
    }

    #[test]
    fn colors_are_stored_as_lowercase_hex() {
        let (_dir, paths) = sandbox();
        let mut settings = Settings::load(&paths).unwrap();
        assert!(settings.set_color(Color::BarFill, 0xaabbcc));
        assert_eq!(stored(&paths), "[settings]\nbar_fill = \"#aabbcc\"\n");
        assert_eq!(settings.color(Color::BarFill), 0xaabbcc);
        assert!(!settings.set_color(Color::BarFill, 0xaabbcc));
        assert!(settings.set_color(Color::BarFill, Color::BarFill.default()));
        assert_eq!(stored(&paths), "[settings]\n");
    }

    #[test]
    fn color_text_needs_a_hash_and_six_hex_digits() {
        assert_eq!(parse_color("#c0152a"), Some(0xc0152a));
        assert_eq!(parse_color("#C0152A"), Some(0xc0152a));
        assert_eq!(parse_color("c0152a"), None);
        assert_eq!(parse_color("#c0152"), None);
        assert_eq!(color_text(0x00bdcf), "#00bdcf");
    }

    #[test]
    fn launch_options_need_closed_quotes() {
        let (_dir, paths) = sandbox();
        let mut settings = Settings::load(&paths).unwrap();
        let season = "Y5S3_ShadowLegacy";
        assert_eq!(
            settings.set_launch_args(season, "-name \"Ash"),
            Err(LAUNCH_ARGS_UNCLOSED)
        );
        assert_eq!(settings.set_launch_args(season, " -novid "), Ok(true));
        assert_eq!(settings.set_launch_args(season, "-novid"), Ok(false));
        assert_eq!(
            settings.entries(Entries::LaunchArgs),
            [(season.to_owned(), "-novid".to_owned())]
        );
        assert_eq!(settings.set_launch_args(season, ""), Ok(true));
        assert!(settings.entries(Entries::LaunchArgs).is_empty());
    }

    #[test]
    fn a_renamed_season_keeps_its_place_and_size_on_home() {
        let (_dir, paths) = sandbox();
        let mut settings = loaded(
            &paths,
            "[settings]\nhome_order = ['a', 'old', 'new']\n\
             home_sizes = { old = 'wide', new = 'tall' }\n",
        );
        assert!(settings.rename_home_entry("old", "new"));
        assert_eq!(settings.home_order(), ["a", "new"]);
        assert_eq!(
            settings.entries(Entries::HomeSizes),
            [("new".to_owned(), "wide".to_owned())]
        );
        assert!(settings.reset_home_layout());
        assert!(!settings.reset_home_layout());
        assert_eq!(stored(&paths), "[settings]\n");
    }

    #[test]
    fn the_proton_choice_and_single_season_entries_are_read() {
        let (_dir, paths) = sandbox();
        let settings = loaded(
            &paths,
            "[settings]\nproton = 'GE-Proton9-20'\nlaunch_args = { Y5S3_ShadowLegacy = '-dx11' }\n\
             season_proton = { Y5S3_ShadowLegacy = 'proton_hotfix', Y1S0_Vanilla = 7 }\n",
        );
        let season = "Y5S3_ShadowLegacy";
        assert_eq!(settings.proton(), "GE-Proton9-20");
        assert_eq!(settings.entry(Entries::LaunchArgs, season), "-dx11");
        assert_eq!(
            settings.entry(Entries::SeasonProton, season),
            "proton_hotfix"
        );
        assert_eq!(settings.entry(Entries::SeasonProton, "Y1S0_Vanilla"), "");
        assert_eq!(settings.entry(Entries::HomeSizes, season), "");
        assert_eq!(loaded(&paths, "[settings]\nproton = 3\n").proton(), "");
    }

    #[test]
    fn the_proton_choice_is_stored_until_another_one_is_picked() {
        let (_dir, paths) = sandbox();
        let mut settings = loaded(&paths, "[settings]\nusername = 'Ash'\n");
        assert!(settings.set_proton("GE-Proton9-20"));
        assert!(!settings.set_proton("GE-Proton9-20"));
        assert_eq!(settings.proton(), "GE-Proton9-20");
        assert_eq!(
            stored(&paths),
            "[settings]\nusername = \"Ash\"\nproton = \"GE-Proton9-20\"\n"
        );
        assert!(settings.set_proton("proton_hotfix"));
        assert_eq!(Settings::load(&paths).unwrap().proton(), "proton_hotfix");
    }

    #[test]
    fn libraries_are_resolved_once_and_end_with_the_default_folder() {
        let (dir, paths) = sandbox();
        let (first, second) = (dir.path().join("one"), dir.path().join("two"));
        let content = format!(
            "[settings]\nlibraries = ['{}', '', '{}/../two', '{}']\n",
            second.display(),
            first.display(),
            second.display()
        );
        let mut settings = loaded(&paths, &content);
        let fixed = paths.default_downloads_dir();
        assert_eq!(settings.libraries(), [second.clone(), fixed.clone()]);
        assert!(settings.is_library(&second));
        assert!(!settings.is_library(&first));
        settings.set_libraries(vec![first.clone(), fixed.clone(), second.clone()]);
        assert_eq!(settings.libraries(), [first, fixed, second]);
        let reread = Settings::load(&paths).unwrap();
        assert_eq!(reread.libraries(), settings.libraries());
    }

    #[test]
    fn the_queue_of_the_python_launcher_is_read_entry_by_entry() {
        let (_dir, paths) = sandbox();
        let settings = loaded(
            &paths,
            "[settings]\nqueue_paused = true\nhm_archive = 'Unstable.7z'\n\
             queue = [{ key = 'Y5S3_ShadowLegacy', hm = true, library = '/games', verify = false }, \
             { key = 'Y1S0_Vanilla', hm = false, library = '', verify = true }, \
             { key = 'Y9S2_NewBlood' }, { hm = true }, { key = 7 }, 'Y1S0_Vanilla']\n",
        );
        let entry = |key: &str, hm, library: &str, verify| QueueEntry {
            key: key.to_owned(),
            hm,
            library: library.to_owned(),
            verify,
        };
        assert_eq!(
            settings.queue(),
            [
                entry("Y5S3_ShadowLegacy", true, "/games", false),
                entry("Y1S0_Vanilla", false, "", true),
                entry("Y9S2_NewBlood", false, "", false),
            ]
        );
        assert!(settings.queue_paused());
        assert_eq!(settings.hm_archive(), "Unstable.7z");
        let empty = loaded(&paths, "[settings]\nqueue = 'x'\nqueue_paused = 'yes'\n");
        assert!(empty.queue().is_empty());
        assert!(!empty.queue_paused());
        assert_eq!(empty.hm_archive(), "");
    }

    #[test]
    fn the_queue_is_stored_with_its_four_fields_and_leaves_the_file_when_empty() {
        let (_dir, paths) = sandbox();
        let mut settings = loaded(&paths, "[settings]\nusername = 'Ash'\n");
        let entries = [
            QueueEntry {
                key: "Y9S2_NewBlood".to_owned(),
                hm: true,
                library: "/mnt/It's \"here\"".to_owned(),
                verify: false,
            },
            QueueEntry {
                key: "Y1S0_Vanilla".to_owned(),
                hm: false,
                library: String::new(),
                verify: true,
            },
        ];
        assert!(settings.set_queue(&entries));
        assert!(!settings.set_queue(&entries));
        assert!(settings.set_queue_paused(true));
        assert!(!settings.set_queue_paused(true));
        assert!(settings.set_hm_archive("Unstable.7z"));
        assert!(!settings.set_hm_archive("Unstable.7z"));
        assert!(settings.set_scale(125));
        let reread = Settings::load(&paths).unwrap();
        assert_eq!(reread.queue(), entries);
        assert!(reread.queue_paused());
        assert_eq!(reread.hm_archive(), "Unstable.7z");
        assert_eq!((reread.username(), reread.scale()), ("Ash", 125));
        let table: Table = toml::from_str(&stored(&paths)).unwrap();
        let first = table["settings"]["queue"][0].as_table().unwrap();
        let fields: Vec<&String> = first.keys().collect();
        assert_eq!(fields, ["key", "hm", "library", "verify"]);
        assert!(settings.set_queue(&[]));
        assert!(settings.set_queue_paused(false));
        assert!(settings.set_hm_archive(""));
        assert!(settings.set_scale(100));
        assert_eq!(stored(&paths), "[settings]\nusername = \"Ash\"\n");
        assert!(take_recorded().is_empty());
    }

    #[test]
    fn a_malformed_file_is_backed_up_and_reset() {
        let (_dir, paths) = sandbox();
        for content in ["[settings\nusername = 'Ash'\n", "settings = 5\n"] {
            let settings = loaded(&paths, content);
            assert_eq!(settings.username(), "ThrowbackUser");
            assert_eq!(
                settings.warning(),
                Some("Settings were malformed, backup saved as settings.toml.broken")
            );
            assert!(!paths.settings_file().exists());
            let broken = paths.settings_file().with_file_name("settings.toml.broken");
            assert_eq!(fs::read_to_string(broken).unwrap(), content);
            let lines = take_recorded();
            assert_eq!(lines.len(), 1, "{lines:?}");
            assert!(lines[0].contains("Settings were malformed - "), "{lines:?}");
            assert!(!lines[0].contains("Ash"), "{lines:?}");
        }
    }

    #[test]
    fn a_file_that_is_not_text_takes_the_malformed_path() {
        let (_dir, paths) = sandbox();
        fs::write(paths.settings_file(), b"[settings]\nusername = '\xff'\n").unwrap();
        let settings = Settings::load(&paths).unwrap();
        assert!(settings.warning().is_some());
        assert_eq!(take_recorded().len(), 1);
    }

    #[test]
    fn an_unreadable_file_stops_the_load() {
        let (_dir, paths) = sandbox();
        fs::create_dir_all(paths.settings_file()).unwrap();
        let error = Settings::load(&paths).err().unwrap();
        assert_eq!(error.to_string(), "Settings read failed");
        assert_eq!(take_recorded().len(), 1);
    }

    #[test]
    fn a_failed_save_is_logged_and_keeps_the_value_in_memory() {
        let (_dir, paths) = sandbox();
        let mut settings = Settings::load(&paths).unwrap();
        fs::remove_dir_all(&paths.data_root).unwrap();
        fs::write(&paths.data_root, "").unwrap();
        assert!(settings.set_scale(125));
        assert_eq!(settings.scale(), 125);
        let lines = take_recorded();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("Settings save failed - "), "{lines:?}");
    }
}
