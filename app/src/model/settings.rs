use std::collections::HashMap;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use backend::depot::wipe_depot_token;
use backend::env::{
    self, FOLDER_NOT_FOUND, GAME_RUNNING, Paths, SEASON_RUNNING, TRANSFER_RUNNING, UPDATE_RUNNING,
    parse_version,
};
use backend::log;
use backend::manifest::{installed_downloads, is_season_folder};
use backend::settings::{Color, Entries, Flag, Settings as Store};
use backend::steam::{self, is_game_running, list_protons, prefix_pids, resolve_proton};
use backend::throwbackloader::{
    apply_tl, tl_folder_version, tl_present, tl_version, write_launcher, write_tl_username,
};
use gpui::{AppContext, Context, EventEmitter, SharedString, Task};

use super::Launcher;
use super::dialogs::{Picked, pick_folder};

pub const DEFAULT_SPAN: (u8, u8) = (1, 2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsEvent {
    UsernameSaved,
    LaunchArgsSaved,
    CacheCleared,
    PrefixRemoved,
    UsernameRejected(SharedString),
    Rejected(SharedString),
}

#[derive(Clone, PartialEq)]
pub struct Library {
    pub path: SharedString,
    pub display: SharedString,
    pub exists: bool,
    pub default: bool,
    pub fixed: bool,
}

#[derive(PartialEq)]
pub struct Proton {
    pub internal: SharedString,
    pub display: SharedString,
}

struct Host {
    wipe_token: fn() -> Result<(), String>,
    game_running: fn(&[PathBuf]) -> bool,
    prefix_pids: fn(&Path) -> Vec<u32>,
    protons: fn(&Paths) -> Vec<steam::Proton>,
}

pub struct Settings {
    pub(super) store: Store,
    libraries: Vec<Library>,
    pub proton: SharedString,
    pub protons: Option<Vec<Proton>>,
    pub tray_available: bool,
    pub(super) clearing_cache: bool,
    proton_scan: Option<Task<()>>,
    logging_out: bool,
    removing_prefix: bool,
    host: Host,
}

impl EventEmitter<SettingsEvent> for Launcher {}

fn survey(store: &Store) -> Vec<Library> {
    let fixed = store.fixed_library();
    let home = env::resolve(&env::home());
    let roots = store.libraries().iter().enumerate();
    roots
        .map(|(index, root)| Library {
            path: root.to_string_lossy().into_owned().into(),
            display: display_path(root, &home).into(),
            exists: root.exists(),
            default: index == 0,
            fixed: env::same_path(root, fixed),
        })
        .collect()
}

fn display_path(root: &Path, home: &Path) -> String {
    match root.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => Path::new("~").join(rest).to_string_lossy().into_owned(),
        Err(_) => root.to_string_lossy().into_owned(),
    }
}

fn reject(message: impl Into<SharedString>, cx: &mut Context<Launcher>) {
    cx.emit(SettingsEvent::Rejected(message.into()));
}

fn missing_ok(removed: io::Result<()>, path: &Path) -> Result<(), String> {
    match removed {
        Err(error) if error.kind() != ErrorKind::NotFound => {
            Err(format!("{error}: '{}'", path.display()))
        }
        _ => Ok(()),
    }
}

fn remove_cache(paths: &Paths) -> Result<(), String> {
    let entries = fs::read_dir(paths.bin_dir())
        .into_iter()
        .flatten()
        .flatten();
    let archives = entries.map(|entry| entry.path()).filter(|path| {
        path.extension()
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case("7z"))
    });
    let tools = [paths.dd_bin(), paths.sevenz_bin(), paths.errors_file()];
    for file in tools.into_iter().chain(archives) {
        missing_ok(fs::remove_file(&file), &file)?;
    }
    let loader = paths.tl_dir();
    missing_ok(fs::remove_dir_all(&loader), &loader)
}

fn loader_plan(
    paths: &Paths,
    libraries: &[PathBuf],
    root: &Path,
    game_running: fn(&[PathBuf]) -> bool,
) -> Option<(Vec<u64>, Vec<PathBuf>)> {
    let latest = parse_version(&tl_version(paths)?)?;
    if !tl_present(&paths.tl_dir()) || game_running(libraries) {
        return None;
    }
    Some((latest, installed_downloads(&[root.to_path_buf()])))
}

fn align_loader(paths: &Paths, folder: &Path, latest: &[u64], username: &str) -> gpui::Result<()> {
    let current = tl_folder_version(folder).and_then(|version| parse_version(&version));
    if current.is_some_and(|current| current.as_slice() >= latest) {
        return write_tl_username(folder, username);
    }
    apply_tl(paths, folder, username)?;
    write_launcher(paths, folder)
}

impl Settings {
    pub fn new(store: Store) -> Self {
        Self {
            libraries: survey(&store),
            store,
            proton: SharedString::default(),
            protons: None,
            tray_available: false,
            clearing_cache: false,
            proton_scan: None,
            logging_out: false,
            removing_prefix: false,
            host: Host {
                wipe_token: wipe_depot_token,
                game_running: is_game_running,
                prefix_pids,
                protons: list_protons,
            },
        }
    }

    pub fn username(&self) -> SharedString {
        self.store.username().to_owned().into()
    }

    pub fn set_username(&mut self, value: &str, cx: &mut Context<Launcher>) {
        match self.store.set_username(value) {
            Ok(true) => {
                let username = self.store.username().to_owned();
                for season in installed_downloads(self.roots()) {
                    if let Err(error) = write_tl_username(&season, &username) {
                        log::fail("Username write failed", format!("{error:#}"));
                    }
                }
                cx.emit(SettingsEvent::UsernameSaved);
                cx.notify();
            }
            Ok(false) => {}
            Err(message) => cx.emit(SettingsEvent::UsernameRejected(message.into())),
        }
    }

    pub fn steam_account(&self) -> SharedString {
        self.store.steam_account().to_owned().into()
    }

    pub fn set_steam_account(&mut self, value: &str, cx: &mut Context<Launcher>) -> bool {
        let changed = self.store.set_steam_account(value);
        if changed {
            cx.notify();
        }
        changed
    }

    pub fn flag(&self, flag: Flag) -> bool {
        self.store.flag(flag)
    }

    pub fn set_flag(&mut self, flag: Flag, value: bool, cx: &mut Context<Launcher>) {
        if self.store.set_flag(flag, value) {
            cx.notify();
        }
    }

    pub fn max_downloads(&self) -> u32 {
        self.store.max_downloads()
    }

    pub fn set_max_downloads(&mut self, value: u32, cx: &mut Context<Launcher>) {
        if self.store.set_max_downloads(value) {
            cx.notify();
        }
    }

    pub fn scale(&self) -> u32 {
        self.store.scale()
    }

    pub fn set_scale(&mut self, value: u32, cx: &mut Context<Launcher>) {
        if self.store.set_scale(value) {
            cx.notify();
        }
    }

    pub fn color(&self, color: Color) -> u32 {
        self.store.color(color)
    }

    pub fn set_color(&mut self, color: Color, value: u32, cx: &mut Context<Launcher>) {
        if self.store.set_color(color, value) {
            cx.notify();
        }
    }

    pub fn launch_args(&self, key: &str) -> &str {
        self.store.entry(Entries::LaunchArgs, key)
    }

    pub fn set_launch_args(&mut self, key: &str, value: &str, cx: &mut Context<Launcher>) {
        match self.store.set_launch_args(key, value) {
            Ok(true) => {
                cx.emit(SettingsEvent::LaunchArgsSaved);
                cx.notify();
            }
            Ok(false) => {}
            Err(message) => reject(message, cx),
        }
    }

    pub fn stored_proton(&self) -> &str {
        self.store.proton()
    }

    pub fn season_proton(&self, key: &str) -> &str {
        self.store.entry(Entries::SeasonProton, key)
    }

    fn offers(&self, internal: &str) -> bool {
        let mut protons = self.protons.iter().flatten();
        protons.any(|proton| proton.internal.as_ref() == internal)
    }

    pub fn set_proton(&mut self, internal: &str, cx: &mut Context<Launcher>) {
        if self.offers(internal) && self.store.set_proton(internal) {
            self.proton = internal.to_owned().into();
            cx.notify();
        }
    }

    pub fn set_season_proton(&mut self, key: &str, internal: &str, cx: &mut Context<Launcher>) {
        let known = internal.is_empty() || self.offers(internal);
        if known && self.store.set_entry(Entries::SeasonProton, key, internal) {
            cx.notify();
        }
    }

    fn protons_scanned(&mut self, found: Vec<steam::Proton>, cx: &mut Context<Launcher>) {
        let default = resolve_proton(&found, "", self.store.proton());
        let default = default.map(|proton| proton.internal.clone());
        let proton = SharedString::from(default.unwrap_or_default());
        let protons = found.into_iter().map(|proton| Proton {
            internal: proton.internal.into(),
            display: proton.display.into(),
        });
        let protons = Some(protons.collect());
        if self.proton != proton || self.protons != protons {
            self.proton = proton;
            self.protons = protons;
            cx.notify();
        }
    }

    pub fn proton_display(&self) -> SharedString {
        self.protons
            .iter()
            .flatten()
            .find(|proton| proton.internal == self.proton)
            .map_or(self.proton.clone(), |proton| proton.display.clone())
    }

    pub fn home_order(&self) -> Vec<String> {
        self.store.home_order()
    }

    pub fn set_home_order(&mut self, order: Vec<String>, cx: &mut Context<Launcher>) {
        if self.store.set_home_order(order) {
            cx.notify();
        }
    }

    pub fn home_spans(&self) -> HashMap<String, (u8, u8)> {
        let spans = self.store.entries(Entries::HomeSizes).into_iter();
        spans
            .filter_map(|(id, size)| {
                let (width, height) = size.split_once('x')?;
                Some((id, (width.parse().ok()?, height.parse().ok()?)))
            })
            .collect()
    }

    pub fn set_home_span(&mut self, id: &str, span: (u8, u8), cx: &mut Context<Launcher>) {
        let size = if span == DEFAULT_SPAN {
            String::new()
        } else {
            format!("{}x{}", span.0, span.1)
        };
        if self.store.set_entry(Entries::HomeSizes, id, &size) {
            cx.notify();
        }
    }

    pub fn reset_home_layout(&mut self, cx: &mut Context<Launcher>) {
        if self.store.reset_home_layout() {
            cx.notify();
        }
    }

    pub fn clearing_cache(&self) -> bool {
        self.clearing_cache
    }

    pub fn roots(&self) -> &[PathBuf] {
        self.store.libraries()
    }

    pub fn is_library(&self, root: &Path) -> bool {
        self.store.is_library(root)
    }

    pub fn warning(&self) -> Option<&str> {
        self.store.warning()
    }

    pub fn libraries(&self) -> &[Library] {
        &self.libraries
    }
}

impl Launcher {
    fn transfers_busy(&self) -> bool {
        self.download.running() || self.update.busy()
    }

    pub fn logout(&mut self, cx: &mut Context<Self>) {
        if self.settings.logging_out {
            return;
        }
        if self.download.running() {
            return reject("A download is using your Steam session", cx);
        }
        if self.update.busy() {
            return reject(UPDATE_RUNNING, cx);
        }
        self.settings.set_steam_account("", cx);
        self.settings.logging_out = true;
        let wipe = self.settings.host.wipe_token;
        let wiped = cx.background_spawn(async move { wipe() });
        cx.spawn(async move |this, cx| {
            let wiped = wiped.await;
            this.update(cx, |this, cx| {
                this.settings.logging_out = false;
                if let Err(message) = wiped {
                    reject(message, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn refresh_protons(&mut self, cx: &mut Context<Self>) {
        let (paths, list) = (self.paths.clone(), self.settings.host.protons);
        let scan = cx.background_spawn(async move { list(&paths) });
        self.settings.proton_scan = Some(cx.spawn(async move |this, cx| {
            let found = scan.await;
            this.update(cx, |this, cx| this.settings.protons_scanned(found, cx))
                .ok();
        }));
    }

    pub fn survey_libraries(&mut self, cx: &mut Context<Self>) {
        let libraries = survey(&self.settings.store);
        if self.settings.libraries != libraries {
            self.settings.libraries = libraries;
            cx.notify();
        }
    }

    fn save_libraries(&mut self, roots: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.settings.store.set_libraries(roots);
        self.settings.libraries = survey(&self.settings.store);
        self.refresh_disk_usage(cx);
        self.refresh_home(cx);
        cx.notify();
    }

    pub fn add_library(&mut self, cx: &mut Context<Self>) {
        if self.transfers_busy() {
            return reject(TRANSFER_RUNNING, cx);
        }
        let picked = pick_folder(cx);
        cx.spawn(async move |this, cx| {
            let picked = picked.await;
            this.update(cx, |this, cx| this.library_picked(picked, cx))
                .ok();
        })
        .detach();
    }

    fn library_picked(&mut self, picked: Picked, cx: &mut Context<Self>) {
        let picked = match picked {
            Ok(folder) => folder,
            Err(message) => return reject(message, cx),
        };
        let Some(picked) = picked else {
            return;
        };
        let mut folder = env::resolve(&picked);
        if is_season_folder(&folder) {
            folder.pop();
        }
        if self.settings.is_library(&folder) {
            return reject("Folder is already a library", cx);
        }
        let mut roots = self.settings.roots().to_vec();
        let nested = |root: &PathBuf| folder.starts_with(root) || root.starts_with(&folder);
        if roots.iter().chain([&self.paths.data_root]).any(nested) {
            return reject("Library folders cannot contain each other", cx);
        }
        roots.push(folder.clone());
        self.save_libraries(roots, cx);
        self.align_library(folder, cx);
    }

    fn align_library(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        let paths = self.paths.clone();
        let libraries = self.settings.roots().to_vec();
        let username = self.settings.store.username().to_owned();
        let game_running = self.settings.host.game_running;
        cx.spawn(async move |this, cx| {
            let plan = cx.background_spawn({
                let (paths, root) = (paths.clone(), root.clone());
                async move { loader_plan(&paths, &libraries, &root, game_running) }
            });
            let Some((latest, folders)) = plan.await else {
                return;
            };
            for folder in folders {
                let listed = this.read_with(cx, |this, _| this.settings.is_library(&root));
                if !listed.unwrap_or(false) {
                    return;
                }
                let (paths, latest, username) = (paths.clone(), latest.clone(), username.clone());
                let aligned = cx.background_spawn(async move {
                    align_loader(&paths, &folder, &latest, &username).map_err(|error| {
                        log::fail("ThrowbackLoader setup failed", format!("{error:#}"))
                    })
                });
                if let Err(message) = aligned.await {
                    this.update(cx, |_, cx| reject(message, cx)).ok();
                    return;
                }
            }
        })
        .detach();
    }

    pub fn remove_library(&mut self, path: &str, cx: &mut Context<Self>) {
        let (target, settings) = (Path::new(path), &self.settings);
        if env::same_path(target, settings.store.fixed_library()) {
            return reject("The Launcher folder cannot be removed", cx);
        }
        if !settings.is_library(target) {
            return;
        }
        if self.update.busy() {
            return reject(UPDATE_RUNNING, cx);
        }
        if self.download.uses_library(target, settings.roots()) {
            return reject("A download is using this library", cx);
        }
        if (settings.host.game_running)(settings.roots()) {
            return reject(GAME_RUNNING, cx);
        }
        let mut roots = settings.roots().to_vec();
        roots.retain(|root| !env::same_path(root, target));
        self.save_libraries(roots, cx);
    }

    pub fn set_default_library(&mut self, path: &str, cx: &mut Context<Self>) {
        let target = Path::new(path);
        let mut roots = self.settings.roots().to_vec();
        let listed = roots.iter().position(|root| env::same_path(root, target));
        let Some(position @ 1..) = listed else {
            return;
        };
        if !target.exists() {
            return reject(FOLDER_NOT_FOUND, cx);
        }
        roots[..=position].rotate_right(1);
        self.save_libraries(roots, cx);
    }

    pub fn clear_cache(&mut self, cx: &mut Context<Self>) {
        if self.settings.clearing_cache {
            return;
        }
        if self.transfers_busy() {
            return reject(TRANSFER_RUNNING, cx);
        }
        self.settings.clearing_cache = true;
        let paths = self.paths.clone();
        let cleared = cx.background_spawn(async move {
            remove_cache(&paths).map_err(|detail| log::fail("Cache clearing failed", detail))
        });
        cx.spawn(async move |this, cx| {
            let outcome = cleared.await;
            this.update(cx, |this, cx| {
                this.settings.clearing_cache = false;
                match outcome {
                    Ok(()) => {
                        this.start_next(cx);
                        cx.emit(SettingsEvent::CacheCleared);
                    }
                    Err(message) => reject(message, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn remove_prefix(&mut self, key: &str, cx: &mut Context<Self>) {
        let known = self.downloads.iter().any(|download| download.key == key);
        if self.settings.removing_prefix || !known {
            return;
        }
        let prefix = self.paths.prefix_dir().join(key);
        if !(self.settings.host.prefix_pids)(&prefix).is_empty() {
            return reject(SEASON_RUNNING, cx);
        }
        self.settings.removing_prefix = true;
        let removed = cx.background_spawn(async move {
            missing_ok(fs::remove_dir_all(&prefix), &prefix)
                .map_err(|detail| log::fail("Proton prefix removal failed", detail))
        });
        cx.spawn(async move |this, cx| {
            let outcome = removed.await;
            this.update(cx, |this, cx| {
                this.settings.removing_prefix = false;
                match outcome {
                    Ok(()) => cx.emit(SettingsEvent::PrefixRemoved),
                    Err(message) => reject(message, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn rename_home_entry(&mut self, old: &str, new: &str, cx: &mut Context<Self>) {
        if self.settings.store.rename_home_entry(old, new) {
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use backend::env::{DEFAULTARGS_DLL, TL_DLLS_COMMON, TL_LAUNCHER, TL_LOADERS, TL_TOML};
    use backend::manifest;
    use backend::pe::versioned_exe;
    use gpui::{Entity, TestAppContext};
    use tempfile::TempDir;

    use super::super::download::{Downloader, State};
    use super::super::update::Updater;
    use super::*;

    const VANILLA: &str = "Y1S0_Vanilla";
    const SHADOW: &str = "Y5S3_ShadowLegacy";
    const HEATED: &str = "Y9S2_HeatedMetal";
    const CONFIG: &str = "[User]\nusername = 'Old'\n\n[Launch]\nargs = ''\n";
    const DLL: &str = "steam_api64.dll";
    const LOCKED: &str = "Steam token removal failed";

    #[test]
    fn paths_under_home_are_shortened() {
        let home = Path::new("/home/ash");
        assert_eq!(
            display_path(Path::new("/home/ash/Games/R6"), home),
            "~/Games/R6"
        );
        assert_eq!(display_path(Path::new("/home/ash"), home), "~");
        assert_eq!(display_path(Path::new("/mnt/games"), home), "/mnt/games");
        assert_eq!(
            display_path(Path::new("/home/ashley/x"), home),
            "/home/ashley/x"
        );
    }

    struct Stage {
        launcher: Entity<Launcher>,
        events: Rc<RefCell<Vec<SettingsEvent>>>,
        notified: Rc<Cell<usize>>,
        paths: Paths,
        root: PathBuf,
        _dir: TempDir,
    }

    fn stage(cx: &mut TestAppContext, libraries: &[&str], settings: &str) -> Stage {
        let dir = tempfile::tempdir().unwrap();
        let root = env::resolve(dir.path());
        let paths = Paths::new(root.join("asset"), root.join("data"));
        let listed = libraries.iter().map(|name| root.join(name));
        let listed: Vec<String> = listed.map(|path| format!("'{}'", path.display())).collect();
        let listed = if listed.is_empty() {
            String::new()
        } else {
            format!("libraries = [{}]\n", listed.join(", "))
        };
        fs::create_dir_all(&paths.data_root).unwrap();
        fs::write(
            paths.settings_file(),
            format!("[settings]\n{listed}{settings}"),
        )
        .unwrap();
        let store = Store::load(&paths).unwrap();
        let launcher = cx.new(|_| {
            let mut launcher = Launcher::new(paths.clone(), store, manifest::downloads());
            launcher.settings.host = Host {
                wipe_token: || Ok(()),
                game_running: |_| false,
                prefix_pids: |_| Vec::new(),
                protons: |_| Vec::new(),
            };
            launcher
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        let notified = Rc::new(Cell::new(0));
        cx.update(|cx| {
            let seen = Rc::clone(&events);
            let record = move |_, event: &SettingsEvent, _: &mut _| {
                seen.borrow_mut().push(event.clone());
            };
            cx.subscribe(&launcher, record).detach();
            let count = Rc::clone(&notified);
            cx.observe(&launcher, move |_, _| count.set(count.get() + 1))
                .detach();
        });
        Stage {
            launcher,
            events,
            notified,
            paths,
            root,
            _dir: dir,
        }
    }

    impl Stage {
        fn act(
            &self,
            cx: &mut TestAppContext,
            action: impl FnOnce(&mut Launcher, &mut Context<Launcher>),
        ) {
            self.launcher.update(cx, action);
            cx.run_until_parked();
        }

        fn seen(&self) -> Vec<SettingsEvent> {
            self.events.take()
        }

        fn notified(&self) -> usize {
            self.notified.take()
        }

        fn busy(&self, cx: &mut TestAppContext, download: State, update: bool) {
            self.launcher.update(cx, |it, _| {
                it.download = Downloader::with(download, None, false, Vec::new());
                it.update = if update {
                    Updater::busy_now()
                } else {
                    Updater::default()
                };
            });
        }

        fn add(&self, cx: &mut TestAppContext, folder: &Path) {
            self.act(cx, |it, cx| it.add_library(cx));
            let folder = folder.to_path_buf();
            cx.simulate_path_prompt_response(move |_| Some(vec![folder]));
            cx.run_until_parked();
        }

        fn remove(&self, cx: &mut TestAppContext, folder: &Path) -> Vec<SettingsEvent> {
            let path = folder.to_string_lossy().into_owned();
            self.act(cx, |it, cx| it.remove_library(&path, cx));
            self.seen()
        }

        fn roots(&self, cx: &mut TestAppContext) -> Vec<PathBuf> {
            self.launcher
                .read_with(cx, |it, _| it.settings.roots().to_vec())
        }

        fn home(&self, cx: &mut TestAppContext) -> Option<Vec<String>> {
            self.launcher.read_with(cx, |it, _| {
                let editions = it.home.editions()?.iter();
                Some(editions.map(|edition| edition.id.clone()).collect())
            })
        }

        fn stored(&self) -> Store {
            Store::load(&self.paths).unwrap()
        }
    }

    fn rejected(message: &str) -> SettingsEvent {
        SettingsEvent::Rejected(message.to_owned().into())
    }

    fn launcher_exe(patch: u32) -> Vec<u8> {
        versioned_exe(&format!("0.1.{patch}"))
    }

    fn loader_cache(paths: &Paths, patch: u32) {
        let cache = paths.tl_dir();
        fs::create_dir_all(&cache).unwrap();
        for name in TL_DLLS_COMMON.into_iter().chain(TL_LOADERS) {
            fs::write(cache.join(name), "new").unwrap();
        }
        fs::write(cache.join(TL_TOML), CONFIG).unwrap();
        fs::write(cache.join(TL_LAUNCHER), launcher_exe(patch)).unwrap();
    }

    fn install(library: &Path, folder: &str, patch: u32) -> PathBuf {
        let season = library.join(folder);
        fs::create_dir_all(&season).unwrap();
        fs::write(season.join(TL_TOML), CONFIG).unwrap();
        fs::write(season.join(TL_LAUNCHER), launcher_exe(patch)).unwrap();
        season
    }

    fn config(season: &Path) -> String {
        fs::read_to_string(season.join(TL_TOML)).unwrap()
    }

    fn proton(internal: &str, display: &str) -> steam::Proton {
        steam::Proton {
            display: display.to_owned(),
            internal: internal.to_owned(),
            binary: PathBuf::new(),
        }
    }

    fn valve(_: &Paths) -> Vec<steam::Proton> {
        vec![
            proton("GE-Proton9-20", "GE-Proton9-20"),
            proton("proton_hotfix", "Proton Hotfix"),
            proton("proton_experimental", "Proton Experimental"),
        ]
    }

    fn custom(_: &Paths) -> Vec<steam::Proton> {
        vec![
            proton("GE-Proton9-20", "GE-Proton9-20"),
            proton("Proton-R6HM", "Proton R6HM"),
        ]
    }

    #[gpui::test]
    fn a_new_username_reaches_every_installed_season(cx: &mut TestAppContext) {
        let stage = stage(cx, &["lib"], "username = 'Old'\n");
        let library = stage.root.join("lib");
        let installed = install(&library, SHADOW, 8);
        let partial = library.join(VANILLA);
        fs::create_dir_all(&partial).unwrap();
        fs::write(partial.join(TL_TOML), CONFIG).unwrap();
        stage.act(cx, |it, cx| it.settings.set_username("Ash", cx));
        assert!(config(&installed).contains("username = 'Ash'"));
        assert!(config(&partial).contains("username = 'Old'"));
        assert_eq!(stage.stored().username(), "Ash");
        assert_eq!(stage.seen(), [SettingsEvent::UsernameSaved]);
        stage.act(cx, |it, cx| it.settings.set_username("  ", cx));
        let empty = SettingsEvent::UsernameRejected("Username is empty".into());
        assert_eq!(stage.seen(), [empty]);
    }

    #[gpui::test]
    fn logout_waits_for_transfers_and_clears_the_account_before_the_token(cx: &mut TestAppContext) {
        let stage = stage(cx, &[], "steam_account = 'ash'\nusername = 'Ash'\n");
        let launcher = &stage.launcher;
        let account =
            |cx: &mut TestAppContext| launcher.read_with(cx, |it, _| it.settings.steam_account());
        launcher.update(cx, |it, _| {
            it.settings.host.wipe_token = || Err(LOCKED.to_owned())
        });
        stage.busy(cx, State::Downloading, true);
        stage.act(cx, |it, cx| it.logout(cx));
        let in_use = "A download is using your Steam session";
        assert_eq!(stage.seen(), [rejected(in_use)]);
        stage.busy(cx, State::Paused, true);
        stage.act(cx, |it, cx| it.logout(cx));
        assert_eq!(stage.seen(), [rejected(UPDATE_RUNNING)]);
        assert_eq!(account(cx), "ash");

        stage.busy(cx, State::Paused, false);
        launcher.update(cx, |it, cx| {
            it.logout(cx);
            it.logout(cx);
        });
        assert_eq!(account(cx), "");
        assert!(stage.seen().is_empty());
        assert_eq!(
            fs::read_to_string(stage.paths.settings_file()).unwrap(),
            "[settings]\nusername = \"Ash\"\n"
        );
        cx.run_until_parked();
        assert_eq!(stage.seen(), [rejected(LOCKED)]);
        stage.act(cx, |it, cx| it.logout(cx));
        assert_eq!(stage.seen(), [rejected(LOCKED)]);
        launcher.update(cx, |it, _| it.settings.host.wipe_token = || Ok(()));
        stage.act(cx, |it, cx| it.logout(cx));
        assert!(stage.seen().is_empty());
    }

    #[gpui::test]
    fn launch_options_are_checked_trimmed_and_announced_when_saved(cx: &mut TestAppContext) {
        let stage = stage(cx, &[], "");
        let options = |cx: &mut TestAppContext| {
            let launcher = &stage.launcher;
            launcher.read_with(cx, |it, _| it.settings.launch_args(SHADOW).to_owned())
        };
        let set = |cx: &mut TestAppContext, value: &str| {
            stage.act(cx, |it, cx| it.settings.set_launch_args(SHADOW, value, cx));
            stage.seen()
        };
        let unclosed = "Launch options have an unclosed quote or escape";
        assert_eq!(set(cx, "-name \"Ash"), [rejected(unclosed)]);
        assert_eq!(options(cx), "");
        assert_eq!(
            set(cx, " MANGOHUD=1 %command% "),
            [SettingsEvent::LaunchArgsSaved]
        );
        assert_eq!(options(cx), "MANGOHUD=1 %command%");
        assert_eq!(
            stage.stored().entry(Entries::LaunchArgs, SHADOW),
            "MANGOHUD=1 %command%"
        );
        assert!(set(cx, "MANGOHUD=1 %command%").is_empty());
        assert_eq!(set(cx, ""), [SettingsEvent::LaunchArgsSaved]);
        assert_eq!(options(cx), "");
        assert!(stage.stored().entries(Entries::LaunchArgs).is_empty());
    }

    #[gpui::test]
    fn the_proton_list_is_scanned_and_the_default_is_resolved(cx: &mut TestAppContext) {
        let stage = stage(cx, &[], "proton = 'gone'\n");
        let scan = |cx: &mut TestAppContext, list: fn(&Paths) -> Vec<steam::Proton>| {
            stage.act(cx, |it, cx| {
                it.settings.host.protons = list;
                it.refresh_protons(cx);
            });
            stage.launcher.read_with(cx, |it, _| {
                let settings = &it.settings;
                let protons = settings.protons.iter().flatten();
                let names: Vec<String> =
                    protons.map(|proton| proton.internal.to_string()).collect();
                (
                    names.join(" "),
                    settings.proton.to_string(),
                    settings.proton_display().to_string(),
                    stage.notified(),
                )
            })
        };
        let shown = |names: &str, proton: &str, display: &str, notified| {
            (
                names.to_owned(),
                proton.to_owned(),
                display.to_owned(),
                notified,
            )
        };
        let launcher = &stage.launcher;
        assert!(launcher.read_with(cx, |it, _| it.settings.protons.is_none()));
        assert_eq!(scan(cx, |_| Vec::new()), shown("", "", "", 1));
        assert!(launcher.read_with(cx, |it, _| it.settings.protons.is_some()));
        let found = "GE-Proton9-20 proton_hotfix proton_experimental";
        let default = shown(found, "proton_experimental", "Proton Experimental", 1);
        assert_eq!(scan(cx, valve), default);
        assert_eq!(scan(cx, valve), (default.0, default.1, default.2, 0));
        assert_eq!(
            scan(cx, custom),
            shown("GE-Proton9-20 Proton-R6HM", "Proton-R6HM", "Proton R6HM", 1)
        );
    }

    #[gpui::test]
    fn a_proton_choice_has_to_be_one_of_the_found_versions(cx: &mut TestAppContext) {
        let stage = stage(cx, &[], "");
        let choice = |cx: &mut TestAppContext| {
            stage.launcher.read_with(cx, |it, _| {
                let settings = &it.settings;
                let season = settings.season_proton(SHADOW).to_owned();
                let stored = settings.stored_proton().to_owned();
                (
                    settings.proton.to_string(),
                    stored,
                    season,
                    stage.notified(),
                )
            })
        };
        let chosen = |proton: &str, stored: &str, season: &str, notified| {
            (
                proton.to_owned(),
                stored.to_owned(),
                season.to_owned(),
                notified,
            )
        };
        let global = |cx: &mut TestAppContext, internal: &str| {
            stage.act(cx, |it, cx| it.settings.set_proton(internal, cx));
        };
        let pin = |cx: &mut TestAppContext, internal: &str| {
            stage.act(cx, |it, cx| {
                it.settings.set_season_proton(SHADOW, internal, cx)
            });
        };
        global(cx, "GE-Proton9-20");
        pin(cx, "GE-Proton9-20");
        assert_eq!(choice(cx), chosen("", "", "", 0));
        stage.act(cx, |it, cx| {
            it.settings.host.protons = valve;
            it.refresh_protons(cx);
        });
        assert_eq!(choice(cx), chosen("proton_experimental", "", "", 1));
        global(cx, "Proton-R6HM");
        pin(cx, "Proton-R6HM");
        assert_eq!(choice(cx), chosen("proton_experimental", "", "", 0));
        global(cx, "GE-Proton9-20");
        assert_eq!(choice(cx), chosen("GE-Proton9-20", "GE-Proton9-20", "", 1));
        global(cx, "GE-Proton9-20");
        assert_eq!(choice(cx).3, 0);
        pin(cx, "proton_hotfix");
        let pinned = chosen("GE-Proton9-20", "GE-Proton9-20", "proton_hotfix", 1);
        assert_eq!(choice(cx), pinned);
        let stored = stage.stored();
        assert_eq!(stored.proton(), "GE-Proton9-20");
        assert_eq!(stored.entry(Entries::SeasonProton, SHADOW), "proton_hotfix");
        pin(cx, "proton_hotfix");
        assert_eq!(choice(cx).3, 0);
        pin(cx, "");
        assert_eq!(choice(cx), chosen("GE-Proton9-20", "GE-Proton9-20", "", 1));
        assert!(stage.stored().entries(Entries::SeasonProton).is_empty());
        stage.act(cx, |it, cx| it.refresh_protons(cx));
        assert_eq!(choice(cx), chosen("GE-Proton9-20", "GE-Proton9-20", "", 0));
    }

    #[gpui::test]
    fn only_the_latest_proton_scan_is_applied(cx: &mut TestAppContext) {
        let stage = stage(cx, &[], "");
        stage.act(cx, |it, cx| {
            it.settings.host.protons = valve;
            it.refresh_protons(cx);
            it.settings.host.protons = custom;
            it.refresh_protons(cx);
        });
        let proton = stage
            .launcher
            .read_with(cx, |it, _| it.settings.proton.clone());
        assert_eq!(proton, "Proton-R6HM");
        assert_eq!(stage.notified(), 1);
    }

    #[gpui::test]
    fn a_library_is_not_added_during_transfers_or_for_known_and_nested_folders(
        cx: &mut TestAppContext,
    ) {
        let stage = stage(cx, &["one"], "");
        let one = stage.root.join("one");
        let before = stage.roots(cx);
        let busy = [
            (State::Preparing, false),
            (State::Downloading, false),
            (State::Applying, false),
            (State::Idle, true),
        ];
        for (download, update) in busy {
            stage.busy(cx, download, update);
            stage.act(cx, |it, cx| it.add_library(cx));
            assert_eq!(stage.seen(), [rejected(TRANSFER_RUNNING)]);
            assert!(!cx.did_prompt_for_paths());
        }
        stage.busy(cx, State::Paused, false);
        stage.act(cx, |it, cx| it.add_library(cx));
        cx.simulate_path_prompt_response(|options| {
            assert!(options.directories && !options.files && !options.multiple);
            assert_eq!(options.prompt, None);
            None
        });
        cx.run_until_parked();
        assert!(stage.seen().is_empty());

        let known = "Folder is already a library";
        for folder in [
            one.clone(),
            one.join(SHADOW),
            stage.paths.default_downloads_dir(),
        ] {
            stage.add(cx, &folder);
            assert_eq!(stage.seen(), [rejected(known)], "{folder:?}");
        }
        let nested = "Library folders cannot contain each other";
        for folder in [
            one.join("deep"),
            one.join("deep").join(SHADOW),
            stage.root.clone(),
            stage.paths.data_root.clone(),
            stage.paths.bin_dir(),
            stage.paths.default_downloads_dir().join("more"),
        ] {
            stage.add(cx, &folder);
            assert_eq!(stage.seen(), [rejected(nested)], "{folder:?}");
        }
        assert_eq!(stage.roots(cx), before);
        assert_eq!(stage.notified(), 0);
    }

    #[gpui::test]
    fn a_folder_dialog_that_cannot_open_is_reported(cx: &mut TestAppContext) {
        let stage = stage(cx, &["one"], "");
        let before = stage.roots(cx);
        stage.act(cx, |it, cx| {
            it.library_picked(Err("File dialog failed".into()), cx)
        });
        assert_eq!(stage.seen(), [rejected("File dialog failed")]);
        assert_eq!(stage.roots(cx), before);
        assert_eq!(stage.notified(), 0);
    }

    #[gpui::test]
    fn an_added_library_is_stored_last_and_refreshes_home_and_disk_usage(cx: &mut TestAppContext) {
        let stage = stage(cx, &["one"], "");
        let fresh = stage.root.join("fresh");
        fs::create_dir_all(fresh.join(SHADOW)).unwrap();
        assert_eq!(stage.home(cx), None);
        stage.add(cx, &fresh.join(SHADOW));
        let fixed = stage.paths.default_downloads_dir();
        let roots = [stage.root.join("one"), fixed, fresh];
        assert_eq!(stage.roots(cx), roots);
        assert_eq!(stage.stored().libraries(), roots);
        assert_eq!(stage.home(cx), Some(vec![SHADOW.to_owned()]));
        let usage = stage.launcher.read_with(cx, |it, _| it.info.disk_usage());
        assert_eq!(usage, Some(0.));
        assert!(stage.seen().is_empty());
    }

    #[gpui::test]
    fn the_loader_of_an_added_library_is_aligned_with_the_cache(cx: &mut TestAppContext) {
        let stage = stage(cx, &[], "username = 'Ash'\n");
        loader_cache(&stage.paths, 8);
        let fresh = stage.root.join("fresh");
        let outdated = install(&fresh, VANILLA, 7);
        let heated = install(&fresh, HEATED, 7);
        let current = install(&fresh, SHADOW, 8);
        let newer = install(&fresh, "Y6S1_CrimsonHeist", 9);
        let other = install(&fresh, "notes", 7);
        let partial = install(&fresh, "Y2S1_VelvetShell", 7);
        fs::remove_file(partial.join(TL_LAUNCHER)).unwrap();
        stage.add(cx, &fresh);
        assert!(stage.seen().is_empty());

        let exe = |season: &Path| fs::read(season.join(TL_LAUNCHER)).unwrap();
        for season in [&outdated, &heated] {
            assert_eq!(exe(season), launcher_exe(8));
            assert!(season.join(DLL).exists());
            assert!(config(season).contains("username = 'Ash'"));
        }
        assert!(outdated.join(DEFAULTARGS_DLL).exists());
        assert!(!heated.join(DEFAULTARGS_DLL).exists());
        for season in [&current, &newer] {
            assert!(!season.join(DLL).exists());
            assert!(config(season).contains("username = 'Ash'"));
        }
        assert_eq!(exe(&newer), launcher_exe(9));
        for folder in [&other, &partial] {
            assert!(!folder.join(DLL).exists());
            assert!(config(folder).contains("username = 'Old'"));
        }
        assert_eq!(exe(&other), launcher_exe(7));
    }

    #[gpui::test]
    fn loader_alignment_stops_at_a_failure_and_when_the_library_is_removed(
        cx: &mut TestAppContext,
    ) {
        let stage = stage(cx, &[], "");
        loader_cache(&stage.paths, 8);
        let fresh = stage.root.join("fresh");
        let first = install(&fresh, VANILLA, 8);
        let second = install(&fresh, SHADOW, 7);
        fs::write(first.join(TL_TOML), b"username = '\xff'\n").unwrap();
        stage.add(cx, &fresh);
        let failed = "ThrowbackLoader setup failed";
        assert_eq!(stage.seen(), [rejected(failed)]);
        assert!(!second.join(DLL).exists());

        assert!(stage.remove(cx, &fresh).is_empty());
        fs::write(first.join(TL_TOML), CONFIG).unwrap();
        let path = fresh.to_string_lossy().into_owned();
        stage.act(cx, |it, cx| {
            it.library_picked(Ok(Some(fresh.clone())), cx);
            it.remove_library(&path, cx);
        });
        assert!(config(&first).contains("username = 'Old'"));
        assert!(!second.join(DLL).exists());

        stage.add(cx, &fresh);
        assert!(config(&first).contains("username = 'ThrowbackUser'"));
        assert!(second.join(DLL).exists());
        assert!(stage.seen().is_empty());
    }

    #[gpui::test]
    fn loader_alignment_needs_a_complete_cache_and_no_running_game(cx: &mut TestAppContext) {
        let stage = stage(cx, &[], "");
        let fresh = stage.root.join("fresh");
        let season = install(&fresh, VANILLA, 7);
        let cache = stage.paths.tl_dir();
        let aligned = |cx: &mut TestAppContext| {
            stage.add(cx, &fresh);
            let aligned = season.join(DLL).exists();
            let launcher = &stage.launcher;
            launcher.update(cx, |it, _| it.settings.host.game_running = |_| false);
            assert!(stage.remove(cx, &fresh).is_empty());
            aligned
        };
        assert!(!aligned(cx));
        loader_cache(&stage.paths, 8);
        fs::remove_file(cache.join(DLL)).unwrap();
        assert!(!aligned(cx));
        fs::write(cache.join(DLL), "new").unwrap();
        fs::write(cache.join(TL_LAUNCHER), "no version").unwrap();
        assert!(!aligned(cx));
        fs::write(cache.join(TL_LAUNCHER), launcher_exe(8)).unwrap();
        let launcher = &stage.launcher;
        launcher.update(cx, |it, _| it.settings.host.game_running = |_| true);
        assert!(!aligned(cx));
        assert!(aligned(cx));
        assert!(stage.seen().is_empty());
    }

    #[gpui::test]
    fn removing_a_library_checks_the_update_and_running_games_in_order(cx: &mut TestAppContext) {
        let stage = stage(cx, &["one", "two"], "");
        let (one, two) = (stage.root.join("one"), stage.root.join("two"));
        let fixed = stage.paths.default_downloads_dir();
        fs::create_dir_all(two.join(SHADOW)).unwrap();
        let launcher = &stage.launcher;
        stage.busy(cx, State::Idle, true);
        launcher.update(cx, |it, _| it.settings.host.game_running = |_| true);
        let fixed_stays = "The Launcher folder cannot be removed";
        assert_eq!(stage.remove(cx, &fixed), [rejected(fixed_stays)]);
        assert!(stage.remove(cx, &stage.root.join("unknown")).is_empty());
        assert_eq!(stage.remove(cx, &two), [rejected(UPDATE_RUNNING)]);
        stage.busy(cx, State::Downloading, false);
        assert_eq!(stage.remove(cx, &two), [rejected(GAME_RUNNING)]);
        assert_eq!(stage.roots(cx), [one.clone(), two.clone(), fixed.clone()]);
        assert_eq!(stage.notified(), 0);

        launcher.update(cx, |it, _| {
            it.settings.host.game_running = |libraries| libraries.len() != 3;
        });
        stage.act(cx, |it, cx| it.refresh_home(cx));
        assert_eq!(stage.home(cx), Some(vec![SHADOW.to_owned()]));
        assert!(stage.remove(cx, &two).is_empty());
        assert_eq!(stage.roots(cx), [one.clone(), fixed.clone()]);
        assert_eq!(stage.stored().libraries(), [one, fixed]);
        assert_eq!(stage.home(cx), Some(Vec::new()));
        assert!(two.join(SHADOW).is_dir());
    }

    #[gpui::test]
    fn the_default_library_moves_to_the_front_and_is_stored(cx: &mut TestAppContext) {
        let stage = stage(cx, &["one", "two"], "");
        let (first, second) = (stage.root.join("one"), stage.root.join("two"));
        fs::create_dir_all(&first).unwrap();
        stage.act(cx, |it, cx| it.survey_libraries(cx));
        assert_eq!(stage.notified(), 1);
        let listed = |cx: &mut TestAppContext| {
            stage.launcher.read_with(cx, |launcher, _| {
                launcher
                    .settings
                    .libraries()
                    .iter()
                    .map(|library| {
                        (
                            library.path.to_string(),
                            library.default,
                            library.fixed,
                            library.exists,
                        )
                    })
                    .collect::<Vec<_>>()
            })
        };
        let prefer = |cx: &mut TestAppContext, path: &str| {
            stage.act(cx, |it, cx| it.set_default_library(path, cx));
            stage.seen()
        };
        let fixed = stage.paths.default_downloads_dir();
        let downloads = fixed.to_string_lossy().into_owned();
        let (one, two) = (
            first.to_string_lossy().into_owned(),
            second.to_string_lossy().into_owned(),
        );
        assert_eq!(
            listed(cx),
            [
                (one.clone(), true, false, true),
                (two.clone(), false, false, false),
                (downloads.clone(), false, true, false),
            ]
        );
        assert_eq!(prefer(cx, &two), [rejected(FOLDER_NOT_FOUND)]);
        assert!(prefer(cx, &one).is_empty());
        assert!(prefer(cx, "/nowhere").is_empty());
        assert_eq!(listed(cx)[0].0, one);
        assert_eq!(stage.notified(), 0);
        assert_eq!(stage.home(cx), None);

        fs::create_dir_all(second.join(SHADOW)).unwrap();
        assert!(prefer(cx, &two).is_empty());
        assert_eq!(
            listed(cx)
                .into_iter()
                .map(|entry| entry.0)
                .collect::<Vec<_>>(),
            [two.clone(), one.clone(), downloads.clone()]
        );
        let text = fs::read_to_string(stage.paths.settings_file()).unwrap();
        assert_eq!(
            text,
            format!("[settings]\nlibraries = [\"{two}\", \"{one}\", \"{downloads}\"]\n")
        );
        assert_eq!(stage.home(cx), Some(vec![SHADOW.to_owned()]));
        let usage = stage.launcher.read_with(cx, |it, _| it.info.disk_usage());
        assert_eq!(usage, Some(0.));
    }

    #[gpui::test]
    fn clearing_the_cache_removes_the_downloaded_tools_and_nothing_else(cx: &mut TestAppContext) {
        let stage = stage(cx, &[], "");
        let paths = &stage.paths;
        let bin = paths.bin_dir();
        let removed = [
            paths.dd_bin(),
            paths.sevenz_bin(),
            paths.errors_file(),
            bin.join("Unstable.7z"),
            bin.join("HeatedMetal-0.2.3.7z"),
        ];
        let kept = [
            paths.api_cache_file(),
            paths.dd_zip(),
            paths.proton_dir().join("GE").join("proton"),
            paths.wine_dir().join("bin").join("wine"),
            paths.default_downloads_dir().join("Unstable.7z"),
            paths.playtime_file(),
        ];
        loader_cache(paths, 8);
        for file in removed.iter().chain(&kept) {
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, "x").unwrap();
        }
        let clearing = |cx: &mut TestAppContext| {
            let launcher = &stage.launcher;
            launcher.read_with(cx, |it, _| it.settings.clearing_cache())
        };
        for (download, update) in [(State::Applying, false), (State::Idle, true)] {
            stage.busy(cx, download, update);
            stage.act(cx, |it, cx| it.clear_cache(cx));
            assert_eq!(stage.seen(), [rejected(TRANSFER_RUNNING)]);
            assert!(!clearing(cx) && paths.dd_bin().exists());
        }
        stage.busy(cx, State::Failed, false);
        stage.launcher.update(cx, |it, cx| {
            it.clear_cache(cx);
            it.clear_cache(cx);
        });
        assert!(clearing(cx));
        assert!(stage.seen().is_empty());
        cx.run_until_parked();
        assert!(!clearing(cx));
        assert_eq!(stage.seen(), [SettingsEvent::CacheCleared]);
        for file in removed.iter().chain([&paths.tl_dir()]) {
            assert!(!file.exists(), "{file:?}");
        }
        for file in &kept {
            assert!(file.exists(), "{file:?}");
        }
        stage.act(cx, |it, cx| it.clear_cache(cx));
        assert_eq!(stage.seen(), [SettingsEvent::CacheCleared]);
    }

    #[gpui::test]
    fn a_failed_cache_clearing_is_reported_and_leaves_the_rest(cx: &mut TestAppContext) {
        let stage = stage(cx, &[], "");
        let paths = &stage.paths;
        loader_cache(paths, 8);
        fs::create_dir_all(paths.sevenz_bin()).unwrap();
        fs::write(paths.dd_bin(), "x").unwrap();
        fs::write(paths.errors_file(), "x").unwrap();
        stage.act(cx, |it, cx| it.clear_cache(cx));
        assert_eq!(stage.seen(), [rejected("Cache clearing failed")]);
        assert!(!paths.dd_bin().exists());
        assert!(paths.errors_file().exists() && paths.tl_dir().exists());
        fs::remove_dir(paths.sevenz_bin()).unwrap();
        stage.act(cx, |it, cx| it.clear_cache(cx));
        assert_eq!(stage.seen(), [SettingsEvent::CacheCleared]);
        assert!(!paths.tl_dir().exists());
    }

    #[gpui::test]
    fn a_prefix_is_removed_for_a_known_season_that_is_not_running(cx: &mut TestAppContext) {
        let stage = stage(cx, &[], "");
        let prefixes = stage.paths.prefix_dir();
        let (prefix, stray) = (prefixes.join(SHADOW), prefixes.join("Y99S9_Unknown"));
        for folder in [&prefix, &stray] {
            fs::create_dir_all(folder.join("pfx").join("drive_c")).unwrap();
        }
        let remove = |cx: &mut TestAppContext, key: &str| {
            stage.act(cx, |it, cx| it.remove_prefix(key, cx));
            stage.seen()
        };
        let launcher = &stage.launcher;
        assert!(remove(cx, "Y99S9_Unknown").is_empty());
        assert!(remove(cx, "../prefixes/Y5S3_ShadowLegacy").is_empty());
        assert!(prefix.exists() && stray.exists());

        launcher.update(cx, |it, _| {
            it.settings.host.prefix_pids = |prefix| {
                let running = prefix.ends_with("prefixes/Y5S3_ShadowLegacy");
                Vec::from_iter(running.then_some(7))
            };
        });
        assert_eq!(remove(cx, SHADOW), [rejected(SEASON_RUNNING)]);
        assert!(prefix.exists());
        assert_eq!(remove(cx, VANILLA), [SettingsEvent::PrefixRemoved]);

        launcher.update(cx, |it, cx| {
            it.settings.host.prefix_pids = |_| Vec::new();
            it.remove_prefix(SHADOW, cx);
            it.remove_prefix(VANILLA, cx);
        });
        assert!(stage.seen().is_empty());
        cx.run_until_parked();
        assert_eq!(stage.seen(), [SettingsEvent::PrefixRemoved]);
        assert!(!prefix.exists() && stray.exists());

        fs::write(&prefix, "").unwrap();
        let failed = "Proton prefix removal failed";
        assert_eq!(remove(cx, SHADOW), [rejected(failed)]);
        fs::remove_file(&prefix).unwrap();
        assert_eq!(remove(cx, SHADOW), [SettingsEvent::PrefixRemoved]);
    }

    #[gpui::test]
    fn home_spans_are_stored_as_sizes_and_the_default_is_left_out(cx: &mut TestAppContext) {
        let layout = "home_order = ['b', 'a']\nhome_sizes = { a = '2x3', b = 'wide', c = '1x' }\n";
        let stage = stage(cx, &[], layout);
        let launcher = &stage.launcher;
        let spans = |cx: &mut TestAppContext| {
            launcher.read_with(cx, |launcher, _| launcher.settings.home_spans())
        };
        assert_eq!(spans(cx), HashMap::from([("a".to_owned(), (2, 3))]));

        launcher.update(cx, |launcher, cx| {
            launcher.settings.set_home_span("b", (4, 1), cx);
            launcher.settings.set_home_span("a", DEFAULT_SPAN, cx);
            launcher
                .settings
                .set_home_order(vec!["a".to_owned(), "b".to_owned()], cx);
        });
        assert_eq!(spans(cx), HashMap::from([("b".to_owned(), (4, 1))]));
        let reloaded = stage.stored();
        assert_eq!(reloaded.home_order(), ["a", "b"]);
        assert_eq!(
            reloaded.entries(Entries::HomeSizes),
            [
                ("b".to_owned(), "4x1".to_owned()),
                ("c".to_owned(), "1x".to_owned())
            ]
        );

        launcher.update(cx, |launcher, cx| launcher.settings.reset_home_layout(cx));
        assert!(spans(cx).is_empty());
        assert!(stage.stored().home_order().is_empty());
    }

    #[gpui::test]
    fn a_library_that_a_queued_download_uses_is_not_removed(cx: &mut TestAppContext) {
        let stage = stage(cx, &["one", "two"], "queue = [{ key = 'Y1S0_Vanilla' }]\n");
        let (one, two) = (stage.root.join("one"), stage.root.join("two"));
        let launcher = &stage.launcher;
        launcher.update(cx, |it, _| it.settings.host.game_running = |_| true);
        let in_use = "A download is using this library";
        assert_eq!(stage.remove(cx, &one), [rejected(in_use)]);
        assert_eq!(stage.remove(cx, &two), [rejected(GAME_RUNNING)]);
        launcher.update(cx, |it, _| it.update = Updater::busy_now());
        assert_eq!(stage.remove(cx, &one), [rejected(UPDATE_RUNNING)]);
        assert_eq!(stage.roots(cx).len(), 3);
    }
}
