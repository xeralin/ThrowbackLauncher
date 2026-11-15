use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use backend::env::{
    BUSY_MESSAGE, IS_WINDOWS, NO_PROTON, NOT_INSTALLED, Paths, TL_LAUNCHER, UPDATE_RUNNING,
};
use backend::heatedmetal::{hm_build_time, hm_crash_log, hm_files_present};
use backend::log;
use backend::manifest::{edition_folder, hm_folder_name, installed_path, partial_path};
use backend::playtime::{Playtime, read_playtime, write_playtime};
use backend::steam::{
    LaunchCommand, Proton, ProtonEnv, expand_launch_args, game_started, list_protons, prefix_pids,
    proton_env, resolve_proton, running_game_folders, stop_game,
};
use backend::throwbackloader::{read_tl_args, tl_present, write_tl_args};
use gpui::{AppContext, AsyncApp, Context, EventEmitter, SharedString, Task, WeakEntity};

use super::{Edition, Launcher};

const SUSPEND_GAP: i64 = 30;
const CHECKPOINT: i64 = 300;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchEvent {
    Error(SharedString),
    FilesMissing { edition: Edition, library: PathBuf },
}

impl EventEmitter<LaunchEvent> for Launcher {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Install {
    pub installed: bool,
    pub partial: bool,
    pub built: Option<u32>,
    pub crash_log: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Installs {
    pub tb: Install,
    pub hm: Install,
    pub prefix: bool,
}

impl Installs {
    pub fn edition(&self, hm: bool) -> Install {
        if hm { self.hm } else { self.tb }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Renderer {
    Vulkan,
    Dx11,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GameArgs {
    pub args: String,
    pub renderer: Option<Renderer>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaytimeEntry {
    pub edition: Edition,
    pub code: String,
    pub minutes: u64,
    pub sessions: u64,
    pub last_played: i64,
    pub first_played: i64,
}

pub(super) struct Game {
    pid: u32,
    exit: Arc<OnceLock<String>>,
}

pub(super) trait Host: Send + Sync {
    fn windows(&self) -> bool;
    fn now(&self) -> i64;
    fn games(&self, libraries: &[PathBuf]) -> BTreeMap<String, Vec<u32>>;
    fn started(&self, pids: &[u32]) -> Option<i64>;
    fn prefix_pids(&self, prefix: &Path) -> Vec<u32>;
    fn stop(&self, pids: &[u32]);
    fn protons(&self, paths: &Paths) -> Vec<Proton>;
    fn proton_env(&self, prefix: &Path) -> io::Result<ProtonEnv>;
    fn spawn(&self, command: Command) -> io::Result<Game>;
}

struct System;

impl Host for System {
    fn windows(&self) -> bool {
        IS_WINDOWS
    }

    fn now(&self) -> i64 {
        let elapsed = SystemTime::now().duration_since(UNIX_EPOCH);
        elapsed.unwrap_or_default().as_secs().cast_signed()
    }

    fn games(&self, libraries: &[PathBuf]) -> BTreeMap<String, Vec<u32>> {
        running_game_folders(libraries)
    }

    fn started(&self, pids: &[u32]) -> Option<i64> {
        game_started(pids)
    }

    fn prefix_pids(&self, prefix: &Path) -> Vec<u32> {
        prefix_pids(prefix)
    }

    fn stop(&self, pids: &[u32]) {
        stop_game(pids);
    }

    fn protons(&self, paths: &Paths) -> Vec<Proton> {
        list_protons(paths)
    }

    fn proton_env(&self, prefix: &Path) -> io::Result<ProtonEnv> {
        proton_env(prefix)
    }

    fn spawn(&self, mut command: Command) -> io::Result<Game> {
        let mut child = command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let game = Game {
            pid: child.id(),
            exit: Arc::default(),
        };
        let exit = Arc::clone(&game.exit);
        thread::Builder::new()
            .name("game-exit".to_owned())
            .spawn(move || {
                if let Ok(status) = child.wait() {
                    let _ = exit.set(log::exit_text(status));
                }
            })?;
        Ok(game)
    }
}

struct Order {
    exe: PathBuf,
    folder: PathBuf,
    prefix: PathBuf,
    paths: Paths,
    libraries: Vec<PathBuf>,
    season_proton: String,
    proton: String,
    options: String,
}

struct Launching {
    edition: Edition,
    game: Option<Game>,
    ended: bool,
}

pub struct Launch {
    pub(super) host: Arc<dyn Host>,
    running: Vec<Edition>,
    launching: Option<Launching>,
    generation: u64,
    jobs: Option<Task<()>>,
    playtime: Playtime,
    sessions: Vec<(Edition, i64)>,
    last_tick: i64,
}

fn refusal(launching: bool, updating: bool, downloading: bool) -> Option<&'static str> {
    if launching {
        Some("R6S is launching")
    } else if updating {
        Some(UPDATE_RUNNING)
    } else if downloading {
        Some(BUSY_MESSAGE)
    } else {
        None
    }
}

fn proton_command(host: &dyn Host, proton: &Proton, order: &Order) -> Result<Command, String> {
    let direct = [
        proton.binary.to_string_lossy().into_owned(),
        "run".to_owned(),
        order.exe.to_string_lossy().into_owned(),
    ];
    let LaunchCommand { argv, env } =
        expand_launch_args(&order.options, &direct).map_err(|error| format!("{error:#}"))?;
    let compat = host.proton_env(&order.prefix);
    let compat = compat.map_err(|error| error.to_string())?;
    let mut words = argv.into_iter();
    let mut command = Command::new(words.next().unwrap_or_default());
    command.args(words).envs(compat.vars()).envs(env);
    Ok(command)
}

fn start(host: &dyn Host, order: &Order) -> Result<Game, String> {
    let running = host.games(&order.libraries);
    host.stop(&running.into_values().flatten().collect::<Vec<_>>());
    let mut command = if host.windows() {
        Command::new(&order.exe)
    } else {
        let protons = host.protons(&order.paths);
        let proton =
            resolve_proton(&protons, &order.season_proton, &order.proton).ok_or(NO_PROTON)?;
        let command = proton_command(host, proton, order);
        command.map_err(|detail| log::fail("Launch failed", detail))?
    };
    command.current_dir(&order.folder);
    host.spawn(command)
        .map_err(|error| log::fail("Launch failed", error))
}

fn halt(host: &dyn Host, game: &Game) {
    if game.exit.get().is_none() {
        host.stop(&[game.pid]);
    }
}

impl Launch {
    pub fn new(paths: &Paths) -> Self {
        Self {
            host: Arc::new(System),
            running: Vec::new(),
            launching: None,
            generation: 0,
            jobs: None,
            playtime: read_playtime(paths),
            sessions: Vec::new(),
            last_tick: 0,
        }
    }

    pub fn running(&self) -> &[Edition] {
        &self.running
    }

    pub fn launching(&self) -> Option<&Edition> {
        self.launching.as_ref().map(|launching| &launching.edition)
    }

    fn book(&mut self, edition: &Edition, start: i64, end: i64) -> i64 {
        let booked = ((end - start).max(0) + 30) / 60;
        let until = start + booked * 60;
        let entry = self.playtime.entry(edition.key.clone()).or_default();
        let played = entry.edition(edition.hm);
        played.minutes += booked.unsigned_abs();
        played.until = until;
        until
    }

    fn count_playtime(&mut self, games: &[(Edition, &[u32])], paths: &Paths) -> bool {
        let now = self.host.now();
        let gap = now - self.last_tick;
        self.last_tick = now;
        let mut changed = false;
        let mut open = Vec::with_capacity(games.len());
        for (edition, mut start) in std::mem::take(&mut self.sessions) {
            if gap > SUSPEND_GAP {
                start += gap;
            }
            let running = games.iter().any(|(game, _)| *game == edition);
            if !running || now - start >= CHECKPOINT {
                start = self.book(&edition, start, now);
                changed = true;
            }
            if running {
                open.push((edition, start));
            }
        }
        for (edition, pids) in games {
            if open.iter().any(|(known, _)| known == edition) {
                continue;
            }
            let started = self.host.started(pids).unwrap_or(now);
            let entry = self.playtime.entry(edition.key.clone()).or_default();
            let played = entry.edition(edition.hm);
            if started > played.until {
                played.sessions += 1;
                let first = if played.until == 0 {
                    started
                } else {
                    played.until
                };
                played.since.get_or_insert(first);
                played.until = started;
                changed = true;
            }
            open.push((edition.clone(), started.max(played.until)));
        }
        self.sessions = open;
        if changed && let Err(error) = write_playtime(paths, &self.playtime) {
            log::fail("Play time save failed", format!("{error:#}"));
        }
        changed
    }
}

#[cfg(test)]
impl Launch {
    pub fn with(running: Vec<Edition>, launching: Option<Edition>) -> Self {
        Self {
            host: Arc::new(System),
            running,
            launching: launching.map(|edition| Launching {
                edition,
                game: None,
                ended: false,
            }),
            generation: 0,
            jobs: None,
            playtime: Playtime::default(),
            sessions: Vec::new(),
            last_tick: 0,
        }
    }
}

impl Launcher {
    pub fn playtime(&self) -> Vec<PlaytimeEntry> {
        let editions = self.downloads.iter();
        let editions = editions.flat_map(|download| [false, true].map(|hm| (download, hm)));
        editions
            .filter_map(|(download, hm)| {
                let entry = self.launch.playtime.get(&download.key)?;
                let played = if hm { entry.hm? } else { entry.base };
                (played.minutes > 0).then(|| PlaytimeEntry {
                    edition: Edition {
                        key: download.key.clone(),
                        hm,
                    },
                    code: download.label.split(' ').next().unwrap_or_default().into(),
                    minutes: played.minutes,
                    sessions: played.sessions,
                    last_played: played.until,
                    first_played: played.since.unwrap_or(played.until),
                })
            })
            .collect()
    }

    pub fn installs(&self, key: &str) -> Installs {
        let libraries = self.settings.roots();
        let edition = |hm: bool| match installed_path(libraries, key, hm) {
            Some(folder) => Install {
                installed: true,
                partial: false,
                built: hm.then(|| hm_build_time(&folder)).flatten(),
                crash_log: hm && !hm_crash_log(&folder).is_empty(),
            },
            None => Install {
                partial: partial_path(libraries, key, hm).is_some(),
                ..Install::default()
            },
        };
        Installs {
            tb: edition(false),
            hm: edition(true),
            prefix: self.paths.prefix_dir().join(key).is_dir(),
        }
    }

    pub fn game_args(&self, key: &str, hm: bool) -> GameArgs {
        let Some(folder) = installed_path(self.settings.roots(), key, hm) else {
            return GameArgs::default();
        };
        let has = |exe: &str| !hm && folder.join(exe).exists();
        let renderer = if has("RainbowSix_Vulkan.exe") {
            Some(Renderer::Vulkan)
        } else if has("RainbowSix_DX11.exe") {
            Some(Renderer::Dx11)
        } else {
            None
        };
        GameArgs {
            args: read_tl_args(&folder),
            renderer,
        }
    }

    pub fn set_game_args(&self, key: &str, hm: bool, value: &str) -> Result<(), String> {
        let folder = installed_path(self.settings.roots(), key, hm).ok_or(NOT_INSTALLED)?;
        write_tl_args(&folder, value.trim())
            .map_err(|error| log::fail("Config.toml edit failed", format!("{error:#}")))
    }

    pub fn launch_season(&mut self, key: &str, hm: bool, cx: &mut Context<Self>) {
        let refused = refusal(
            self.launch.launching.is_some(),
            self.update.busy(),
            self.download.busy_with(key),
        );
        if let Some(message) = refused {
            cx.emit(LaunchEvent::Error(message.into()));
            return;
        }
        let Some(folder) = installed_path(self.settings.roots(), key, hm) else {
            cx.emit(LaunchEvent::Error(NOT_INSTALLED.into()));
            return;
        };
        let edition = Edition {
            key: key.to_owned(),
            hm,
        };
        if !tl_present(&folder) || (hm && !hm_files_present(&folder)) {
            let library = folder.parent().unwrap_or(&folder).to_path_buf();
            cx.emit(LaunchEvent::FilesMissing { edition, library });
            return;
        }
        self.launch.generation += 1;
        let generation = self.launch.generation;
        let host = Arc::clone(&self.launch.host);
        let name = if hm { "RainbowSix.exe" } else { TL_LAUNCHER };
        let order = Order {
            exe: folder.join(name),
            folder,
            prefix: self.paths.prefix_dir().join(key),
            paths: self.paths.clone(),
            libraries: self.settings.roots().to_vec(),
            season_proton: self.settings.season_proton(key).to_owned(),
            proton: self.settings.stored_proton().to_owned(),
            options: self.settings.launch_args(key).to_owned(),
        };
        self.launch.launching = Some(Launching {
            edition,
            game: None,
            ended: false,
        });
        cx.notify();
        self.poke_watchdog(cx);
        self.queue_game_job(
            async move |this, cx| {
                let current = this.read_with(cx, |this, _| this.launch.generation == generation);
                if !current.unwrap_or(false) {
                    return;
                }
                let starter = Arc::clone(&host);
                let started = cx.background_spawn(async move { start(&*starter, &order) });
                let started = started.await;
                let stale =
                    this.update(cx, |this, cx| this.launch_started(started, generation, cx));
                if let Ok(Some(game)) = stale {
                    cx.background_spawn(async move { halt(&*host, &game) })
                        .await;
                }
            },
            cx,
        );
    }

    pub fn stop_season(&mut self, key: &str, cx: &mut Context<Self>) {
        let launching = &mut self.launch.launching;
        let stopped = launching.take_if(|launching| launching.edition.key == key);
        if stopped.is_some() {
            self.launch.generation += 1;
            cx.notify();
        }
        let game = stopped.and_then(|launching| launching.game);
        let host = Arc::clone(&self.launch.host);
        let libraries = self.settings.roots().to_vec();
        let prefix = self.paths.prefix_dir().join(key);
        let key = key.to_owned();
        self.queue_game_job(
            async move |this, cx| {
                let stop = cx.background_spawn(async move {
                    if let Some(game) = game {
                        halt(&*host, &game);
                    }
                    let pids = if host.windows() {
                        let games = host.games(&libraries);
                        let heated = hm_folder_name(&key);
                        let folders = [key, heated];
                        let found = folders.iter().filter_map(|folder| games.get(folder));
                        found.flatten().copied().collect()
                    } else {
                        host.prefix_pids(&prefix)
                    };
                    host.stop(&pids);
                });
                stop.await;
                this.update(cx, |this, cx| this.poke_watchdog(cx)).ok();
            },
            cx,
        );
    }

    pub fn save_playtime(&mut self, cx: &mut Context<Self>) {
        if self.launch.count_playtime(&[], &self.paths) {
            cx.notify();
        }
    }

    pub fn reset_playtime(&mut self, cx: &mut Context<Self>) -> Result<(), SharedString> {
        let emptied = write_playtime(&self.paths, &Playtime::new());
        emptied.map_err(|error| log::fail("Play time save failed", format!("{error:#}")))?;
        self.launch.playtime.clear();
        cx.notify();
        Ok(())
    }

    pub(super) fn follow_games(&mut self, cx: &mut Context<Self>) {
        let folders = self.watchdog.folders();
        let editions = self.downloads.iter();
        let editions = editions.flat_map(|download| [false, true].map(|hm| (download, hm)));
        let games: Vec<(Edition, &[u32])> = editions
            .filter(|(download, hm)| !hm || download.hm_version.is_some())
            .filter_map(|(download, hm)| {
                let pids = folders.get(&edition_folder(&download.key, hm))?;
                let key = download.key.clone();
                Some((Edition { key, hm }, pids.as_slice()))
            })
            .collect();
        if self.launch.count_playtime(&games, &self.paths) {
            cx.notify();
        }
        let running: Vec<Edition> = games.into_iter().map(|(edition, _)| edition).collect();
        if running != self.launch.running {
            let before = std::mem::replace(&mut self.launch.running, running);
            cx.notify();
            self.start_next(cx);
            let running = &self.launch.running;
            let open = |key: &String| running.iter().any(|edition| edition.key == *key);
            let keys = before.into_iter().map(|edition| edition.key);
            let mut closed: Vec<String> = keys.filter(|key| !open(key)).collect();
            closed.dedup();
            for key in closed {
                self.sweep_prefix(key, cx);
            }
        }
        if let Some(Launching {
            edition,
            game: Some(game),
            ended,
        }) = &mut self.launch.launching
        {
            if self.launch.running.contains(edition) {
                self.launch.launching = None;
                cx.notify();
            } else if let Some(exit) = game.exit.get() {
                if std::mem::replace(ended, true) {
                    let message = log::fail("Launch failed", exit);
                    self.launch_failed(message, self.launch.generation, cx);
                } else {
                    self.scan_games(cx);
                }
            }
        }
    }

    fn queue_game_job(
        &mut self,
        job: impl AsyncFnOnce(WeakEntity<Self>, &mut AsyncApp) + 'static,
        cx: &mut Context<Self>,
    ) {
        let earlier = self.launch.jobs.take();
        self.launch.jobs = Some(cx.spawn(async move |this, cx| {
            if let Some(earlier) = earlier {
                earlier.await;
            }
            job(this, cx).await;
        }));
    }

    fn launch_started(
        &mut self,
        started: Result<Game, String>,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> Option<Game> {
        match started {
            Err(message) => {
                self.launch_failed(message, generation, cx);
                None
            }
            Ok(game) if generation != self.launch.generation => Some(game),
            Ok(game) => {
                if let Some(launching) = &mut self.launch.launching {
                    launching.game = Some(game);
                }
                self.poke_watchdog(cx);
                None
            }
        }
    }

    fn launch_failed(&mut self, message: String, generation: u64, cx: &mut Context<Self>) {
        if generation != self.launch.generation {
            return;
        }
        if let Some(failed) = self.launch.launching.take() {
            cx.notify();
            self.sweep_prefix(failed.edition.key, cx);
        }
        cx.emit(LaunchEvent::Error(message.into()));
    }

    fn sweep_prefix(&mut self, key: String, cx: &mut Context<Self>) {
        if self.launch.host.windows() {
            return;
        }
        let host = Arc::clone(&self.launch.host);
        let libraries = self.settings.roots().to_vec();
        let prefix = self.paths.prefix_dir().join(&key);
        self.queue_game_job(
            async move |this, cx| {
                let launching = this.read_with(cx, |this, _| {
                    let launching = this.launch.launching.as_ref();
                    launching.is_some_and(|launching| launching.edition.key == key)
                });
                if launching.unwrap_or(true) {
                    return;
                }
                let sweep = cx.background_spawn(async move {
                    let games = host.games(&libraries);
                    if !games.contains_key(&key) && !games.contains_key(&hm_folder_name(&key)) {
                        host.stop(&host.prefix_pids(&prefix));
                    }
                });
                sweep.await;
            },
            cx,
        );
    }
}

#[cfg(test)]
pub(super) mod testing {
    use std::cell::{Cell, RefCell};
    use std::fs;
    use std::rc::Rc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};

    use backend::env::{TL_DLLS_COMMON, TL_LOADERS, TL_TOML, resolve};
    use backend::manifest;
    use backend::settings::Settings as Store;
    use gpui::{Entity, TestAppContext};
    use tempfile::TempDir;

    use super::*;

    pub const KEY: &str = "Y5S3_ShadowLegacy";
    pub const HEATED: &str = "Y5S3_HeatedMetal";
    pub const NOW: i64 = 1_000_000;

    pub struct Spawned {
        pub argv: Vec<String>,
        pub cwd: PathBuf,
        pub env: BTreeMap<String, String>,
    }

    #[derive(Default)]
    pub struct FakeHost {
        pub windows: AtomicBool,
        pub clock: AtomicI64,
        pub scans: AtomicUsize,
        pub games: Mutex<BTreeMap<String, Vec<u32>>>,
        pub late: Mutex<Option<(String, Vec<u32>)>>,
        pub started: Mutex<Option<i64>>,
        pub prefix: Mutex<Vec<u32>>,
        pub protons: Mutex<Vec<Proton>>,
        pub refuse: AtomicBool,
        pub spawned: Mutex<Vec<Spawned>>,
        pub exits: Mutex<Vec<Arc<OnceLock<String>>>>,
        pub actions: Mutex<Vec<String>>,
    }

    impl FakeHost {
        pub fn run(&self, folder: &str, pids: &[u32]) {
            let mut games = self.games.lock().unwrap();
            games.insert(folder.to_owned(), pids.to_vec());
        }

        pub fn quit(&self, folder: &str) {
            self.games.lock().unwrap().remove(folder);
        }

        pub fn at(&self, now: i64) {
            self.clock.store(now, Ordering::Relaxed);
        }

        pub fn offer(&self, internals: &[&str]) {
            let named = |internal: &&str| Proton {
                display: internal.to_uppercase(),
                internal: (*internal).to_owned(),
                binary: PathBuf::from(format!("/p/{internal}/proton")),
            };
            *self.protons.lock().unwrap() = internals.iter().map(named).collect();
        }

        pub fn exit(&self, detail: &str) {
            let exits = self.exits.lock().unwrap();
            exits.last().unwrap().set(detail.to_owned()).unwrap();
        }

        pub fn actions(&self) -> Vec<String> {
            std::mem::take(&mut self.actions.lock().unwrap())
        }

        pub fn scans(&self) -> usize {
            self.scans.load(Ordering::Relaxed)
        }

        fn note(&self, action: String) {
            self.actions.lock().unwrap().push(action);
        }
    }

    impl Host for FakeHost {
        fn windows(&self) -> bool {
            self.windows.load(Ordering::Relaxed)
        }

        fn now(&self) -> i64 {
            self.clock.load(Ordering::Relaxed)
        }

        fn games(&self, _libraries: &[PathBuf]) -> BTreeMap<String, Vec<u32>> {
            self.scans.fetch_add(1, Ordering::Relaxed);
            let seen = self.games.lock().unwrap().clone();
            if let Some((folder, pids)) = self.late.lock().unwrap().take() {
                self.run(&folder, &pids);
                self.exit("exit=0");
            }
            seen
        }

        fn started(&self, _pids: &[u32]) -> Option<i64> {
            *self.started.lock().unwrap()
        }

        fn prefix_pids(&self, prefix: &Path) -> Vec<u32> {
            let name = prefix.file_name().unwrap().to_string_lossy();
            self.note(format!("prefix {name}"));
            self.prefix.lock().unwrap().clone()
        }

        fn stop(&self, pids: &[u32]) {
            self.note(format!("stop {pids:?}"));
            self.games.lock().unwrap().retain(|_, running| {
                running.retain(|pid| !pids.contains(pid));
                !running.is_empty()
            });
        }

        fn protons(&self, _paths: &Paths) -> Vec<Proton> {
            self.protons.lock().unwrap().clone()
        }

        fn proton_env(&self, prefix: &Path) -> io::Result<ProtonEnv> {
            Ok(ProtonEnv {
                prefix: prefix.to_owned(),
                steam: PathBuf::from("/steam"),
            })
        }

        fn spawn(&self, command: Command) -> io::Result<Game> {
            self.note("spawn".to_owned());
            if self.refuse.load(Ordering::Relaxed) {
                return Err(io::Error::other("refused"));
            }
            let text = |word: &std::ffi::OsStr| word.to_string_lossy().into_owned();
            let words = std::iter::once(command.get_program()).chain(command.get_args());
            let variables = command.get_envs();
            let mut spawned = self.spawned.lock().unwrap();
            spawned.push(Spawned {
                argv: words.map(text).collect(),
                cwd: command.get_current_dir().unwrap().to_path_buf(),
                env: variables
                    .map(|(name, value)| (text(name), text(value.unwrap())))
                    .collect(),
            });
            let exit = Arc::<OnceLock<String>>::default();
            self.exits.lock().unwrap().push(Arc::clone(&exit));
            let pid = 9000 + spawned.len() as u32;
            Ok(Game { pid, exit })
        }
    }

    pub struct Stage {
        _dir: TempDir,
        pub paths: Paths,
        pub library: PathBuf,
        pub host: Arc<FakeHost>,
        pub launcher: Entity<Launcher>,
        pub events: Rc<RefCell<Vec<LaunchEvent>>>,
        pub notified: Rc<Cell<usize>>,
    }

    impl Stage {
        pub fn new(cx: &mut TestAppContext, settings: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = resolve(dir.path());
            let paths = Paths::new(root.join("asset"), root.join("data"));
            let library = root.join("lib");
            fs::create_dir_all(&library).unwrap();
            fs::create_dir_all(&paths.data_root).unwrap();
            let libraries = format!("libraries = ['{}']", library.display());
            fs::write(
                paths.settings_file(),
                format!("[settings]\n{libraries}\n{settings}"),
            )
            .unwrap();
            let store = Store::load(&paths).unwrap();
            let host = Arc::new(FakeHost::default());
            host.at(NOW);
            let launcher = cx.new(|_| Launcher::new(paths.clone(), store, manifest::downloads()));
            let events = Rc::new(RefCell::new(Vec::new()));
            let sink = Rc::clone(&events);
            cx.update(|cx| {
                let record = move |_, event: &LaunchEvent, _: &mut gpui::App| {
                    sink.borrow_mut().push(event.clone());
                };
                cx.subscribe(&launcher, record).detach();
            });
            let notified = Rc::new(Cell::new(0));
            let count = Rc::clone(&notified);
            cx.update(|cx| {
                cx.observe(&launcher, move |_, _| count.set(count.get() + 1))
                    .detach();
            });
            let stage = Self {
                _dir: dir,
                paths,
                library,
                host,
                launcher,
                events,
                notified,
            };
            stage.restart(cx);
            stage
        }

        pub fn restart(&self, cx: &mut TestAppContext) {
            self.launcher.update(cx, |launcher, _| {
                launcher.launch = Launch::new(&self.paths);
                launcher.launch.host = self.host.clone();
            });
        }

        pub fn install(&self, folder: &str) -> PathBuf {
            let folder = self.library.join(folder);
            fs::create_dir_all(&folder).unwrap();
            let loader = TL_DLLS_COMMON.iter().chain(&TL_LOADERS);
            for name in loader.chain(&[TL_TOML, TL_LAUNCHER]) {
                fs::write(folder.join(name), "").unwrap();
            }
            folder
        }

        pub fn tick(&self, cx: &mut TestAppContext) {
            self.launcher
                .update(cx, |launcher, cx| launcher.scan_games(cx));
            cx.run_until_parked();
        }

        pub fn pass(&self, seconds: u64, cx: &mut TestAppContext) {
            cx.executor()
                .advance_clock(std::time::Duration::from_secs(seconds));
            cx.run_until_parked();
        }

        pub fn events(&self) -> Vec<LaunchEvent> {
            self.events.take()
        }

        pub fn notified(&self) -> usize {
            self.notified.take()
        }

        pub fn running(&self, cx: &mut TestAppContext) -> Vec<(String, bool)> {
            self.launcher.read_with(cx, |launcher, _| {
                let running = launcher.launch.running().iter();
                running
                    .map(|edition| (edition.key.clone(), edition.hm))
                    .collect()
            })
        }

        pub fn launching(&self, cx: &mut TestAppContext) -> Option<(String, bool)> {
            self.launcher.read_with(cx, |launcher, _| {
                let launching = launcher.launch.launching();
                launching.map(|edition| (edition.key.clone(), edition.hm))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::Ordering;

    use gpui::TestAppContext;

    use super::testing::{HEATED, KEY, NOW, Stage};
    use super::*;

    const VANILLA: &str = "Y1S0_Vanilla";
    const LATER: &str = "Y10S1_PrepPhase";

    type Played = (String, bool, u64, u64, i64, i64);

    fn error(message: &'static str) -> LaunchEvent {
        LaunchEvent::Error(message.into())
    }

    fn named(key: &str, hm: bool) -> (String, bool) {
        (key.to_owned(), hm)
    }

    fn play(stage: &Stage, key: &str, hm: bool, cx: &mut TestAppContext) {
        let launcher = &stage.launcher;
        launcher.update(cx, |launcher, cx| launcher.launch_season(key, hm, cx));
    }

    fn stop(stage: &Stage, key: &str, cx: &mut TestAppContext) {
        let launcher = &stage.launcher;
        launcher.update(cx, |launcher, cx| launcher.stop_season(key, cx));
    }

    fn elapse(stage: &Stage, seconds: i64, cx: &mut TestAppContext) {
        let end = stage.host.now() + seconds;
        while stage.host.now() < end {
            stage.host.at((stage.host.now() + SUSPEND_GAP).min(end));
            stage.tick(cx);
        }
    }

    fn played(stage: &Stage, cx: &mut TestAppContext) -> Vec<Played> {
        let entries = stage
            .launcher
            .read_with(cx, |launcher, _| launcher.playtime());
        let flat = |entry: PlaytimeEntry| {
            (
                entry.edition.key,
                entry.edition.hm,
                entry.minutes,
                entry.sessions,
                entry.last_played,
                entry.first_played,
            )
        };
        entries.into_iter().map(flat).collect()
    }

    fn stored(stage: &Stage) -> String {
        fs::read_to_string(stage.paths.playtime_file()).unwrap()
    }

    #[test]
    fn play_is_refused_while_launching_then_updating_then_downloading() {
        assert_eq!(refusal(true, true, true), Some("R6S is launching"));
        assert_eq!(refusal(false, true, true), Some("An update is running"));
        assert_eq!(
            refusal(false, false, true),
            Some("A download is running for this season")
        );
        assert_eq!(refusal(false, false, false), None);
    }

    #[gpui::test]
    fn play_refuses_a_missing_or_damaged_season(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "");
        play(&stage, KEY, false, cx);
        assert_eq!(stage.events(), [error("Not installed")]);
        let folder = stage.install(KEY);
        fs::remove_file(folder.join("upc_r2_loader64.dll")).unwrap();
        play(&stage, KEY, false, cx);
        stage.install(HEATED);
        play(&stage, KEY, true, cx);
        cx.run_until_parked();
        let missing = |hm: bool| LaunchEvent::FilesMissing {
            edition: Edition {
                key: KEY.to_owned(),
                hm,
            },
            library: stage.library.clone(),
        };
        assert_eq!(stage.events(), [missing(false), missing(true)]);
        assert_eq!(stage.launching(cx), None);
        assert_eq!(stage.host.scans(), 0);
        assert!(stage.host.actions().is_empty());
    }

    #[gpui::test]
    fn running_lists_the_known_editions_in_manifest_order(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "");
        let folders = [
            "Y9S2_HeatedMetal",
            LATER,
            VANILLA,
            "Y9S2_NewBlood",
            "Y1S0_HeatedMetal",
            "Y99S1_Probe",
        ];
        for (pid, folder) in folders.iter().enumerate() {
            stage.host.run(folder, &[pid as u32]);
        }
        stage.tick(cx);
        assert_eq!(
            stage.running(cx),
            [
                named(VANILLA, false),
                named("Y9S2_NewBlood", false),
                named("Y9S2_NewBlood", true),
                named(LATER, false)
            ]
        );
    }

    #[gpui::test]
    fn the_interface_is_notified_only_when_something_it_shows_changes(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "");
        stage.tick(cx);
        assert_eq!(stage.notified(), 0);
        stage.host.run(KEY, &[50]);
        stage.tick(cx);
        assert!(stage.notified() > 0);
        elapse(&stage, 60, cx);
        assert_eq!(stage.notified(), 0);
        elapse(&stage, 240, cx);
        assert_eq!(stage.notified(), 1);
        elapse(&stage, 40, cx);
        stage.host.quit(KEY);
        stage.tick(cx);
        assert!(stage.notified() > 0);
        stage.tick(cx);
        assert_eq!(stage.notified(), 0);

        let before = stored(&stage);
        *stage.host.started.lock().unwrap() = Some(NOW + 350);
        stage.host.at(NOW + 355);
        stage.host.run(KEY, &[51]);
        stage.tick(cx);
        assert_eq!(stage.notified(), 1);
        assert_eq!(stored(&stage), before);
    }

    #[gpui::test]
    fn play_time_is_booked_in_minutes_and_the_rest_is_carried_over(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "");
        let started = |at: i64| *stage.host.started.lock().unwrap() = Some(at);
        started(NOW - 10);
        stage.host.run(KEY, &[50]);
        stage.tick(cx);
        assert_eq!(
            stored(&stage),
            "[Y5S3_ShadowLegacy]\nsessions = 1\nsince = 999990\nuntil = 999990\n"
        );
        assert!(played(&stage, cx).is_empty());
        elapse(&stage, 280, cx);
        assert!(played(&stage, cx).is_empty());
        elapse(&stage, 12, cx);
        let first = NOW - 10;
        assert_eq!(
            played(&stage, cx),
            [(KEY.to_owned(), false, 5, 1, NOW + 290, first)]
        );
        elapse(&stage, 98, cx);
        stage.host.quit(KEY);
        stage.tick(cx);
        assert_eq!(
            played(&stage, cx),
            [(KEY.to_owned(), false, 7, 1, NOW + 410, first)]
        );

        started(NOW + 400);
        stage.host.at(NOW + 405);
        stage.host.run(KEY, &[51]);
        stage.tick(cx);
        elapse(&stage, 95, cx);
        stage.host.quit(KEY);
        stage.tick(cx);
        assert_eq!(
            played(&stage, cx),
            [(KEY.to_owned(), false, 9, 1, NOW + 530, first)]
        );

        started(NOW + 1000);
        stage.host.at(NOW + 1002);
        stage.host.run(KEY, &[52]);
        stage.tick(cx);
        elapse(&stage, 18, cx);
        stage.host.quit(KEY);
        stage.tick(cx);
        assert_eq!(
            stored(&stage),
            "[Y5S3_ShadowLegacy]\nsessions = 2\nsince = 999990\nuntil = 1001000\nminutes = 9\n"
        );
    }

    #[gpui::test]
    fn a_launcher_restart_inside_the_same_game_counts_nothing_twice(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "");
        *stage.host.started.lock().unwrap() = Some(NOW - 10);
        stage.host.run(KEY, &[50]);
        stage.tick(cx);
        elapse(&stage, 310, cx);
        assert_eq!(
            played(&stage, cx),
            [(KEY.to_owned(), false, 5, 1, NOW + 290, NOW - 10)]
        );
        stage.restart(cx);
        stage.tick(cx);
        elapse(&stage, 100, cx);
        stage.host.quit(KEY);
        stage.tick(cx);
        assert_eq!(
            played(&stage, cx),
            [(KEY.to_owned(), false, 7, 1, NOW + 410, NOW - 10)]
        );
    }

    #[gpui::test]
    fn a_gap_between_ticks_is_not_played_time(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "");
        stage.host.run(KEY, &[50]);
        stage.tick(cx);
        elapse(&stage, 100, cx);
        stage.host.at(NOW + 1100);
        stage.tick(cx);
        elapse(&stage, 50, cx);
        stage.host.quit(KEY);
        stage.tick(cx);
        assert_eq!(
            played(&stage, cx),
            [(KEY.to_owned(), false, 3, 1, NOW + 1180, NOW)]
        );
    }

    #[gpui::test]
    fn quitting_books_the_open_sessions_of_every_edition(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "");
        for (pid, folder) in [KEY, HEATED, LATER].iter().enumerate() {
            stage.host.run(folder, &[pid as u32]);
        }
        stage.tick(cx);
        elapse(&stage, 200, cx);
        let save = |cx: &mut TestAppContext| {
            let launcher = &stage.launcher;
            launcher.update(cx, |launcher, cx| launcher.save_playtime(cx));
        };
        save(cx);
        let section = "sessions = 1\nsince = 1000000\nuntil = 1000180\nminutes = 3\n";
        assert_eq!(
            stored(&stage),
            format!("[{LATER}]\n{section}\n[{KEY}]\n{section}\n[{KEY}.hm]\n{section}")
        );
        let booked = |key: &str, hm: bool| (key.to_owned(), hm, 3, 1, NOW + 180, NOW);
        let all = [booked(KEY, false), booked(KEY, true), booked(LATER, false)];
        assert_eq!(played(&stage, cx), all);
        stage.host.at(NOW + 400);
        save(cx);
        assert_eq!(played(&stage, cx), all);
    }

    #[gpui::test]
    fn a_file_without_sessions_starts_counting_them_at_its_last_booking(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "");
        fs::write(
            stage.paths.playtime_file(),
            "[Y5S3_ShadowLegacy]\nminutes = 12\nuntil = 900\n",
        )
        .unwrap();
        stage.restart(cx);
        assert_eq!(
            played(&stage, cx),
            [(KEY.to_owned(), false, 12, 0, 900, 900)]
        );
        stage.host.run(KEY, &[50]);
        stage.tick(cx);
        assert_eq!(
            played(&stage, cx),
            [(KEY.to_owned(), false, 12, 1, NOW, 900)]
        );
    }

    #[gpui::test]
    fn a_reset_empties_the_file_and_a_failed_one_keeps_the_count(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "");
        stage.host.run(KEY, &[50]);
        stage.tick(cx);
        elapse(&stage, 90, cx);
        stage.host.quit(KEY);
        stage.tick(cx);
        let booked = [(KEY.to_owned(), false, 2, 1, NOW + 120, NOW)];
        assert_eq!(played(&stage, cx), booked);
        let reset = |cx: &mut TestAppContext| {
            let launcher = &stage.launcher;
            launcher.update(cx, |launcher, cx| launcher.reset_playtime(cx))
        };
        fs::remove_file(stage.paths.playtime_file()).unwrap();
        fs::create_dir_all(stage.paths.playtime_file()).unwrap();
        assert_eq!(reset(cx), Err("Play time save failed".into()));
        assert_eq!(played(&stage, cx), booked);
        fs::remove_dir(stage.paths.playtime_file()).unwrap();
        assert_eq!(reset(cx), Ok(()));
        assert!(played(&stage, cx).is_empty());
        assert_eq!(stored(&stage), "");
        stage.restart(cx);
        assert!(played(&stage, cx).is_empty());
    }

    #[gpui::test]
    fn a_failed_save_keeps_the_count_in_memory(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "");
        fs::create_dir_all(stage.paths.playtime_file()).unwrap();
        stage.restart(cx);
        stage.host.run(KEY, &[50]);
        stage.tick(cx);
        elapse(&stage, 90, cx);
        stage.host.quit(KEY);
        stage.tick(cx);
        assert_eq!(
            played(&stage, cx),
            [(KEY.to_owned(), false, 2, 1, NOW + 120, NOW)]
        );
        assert!(stage.paths.playtime_file().is_dir());
    }

    #[gpui::test]
    fn installs_report_each_edition_and_the_prefix(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "");
        let installs = |cx: &mut TestAppContext| {
            let launcher = &stage.launcher;
            launcher.read_with(cx, |launcher, _| launcher.installs(KEY))
        };
        assert_eq!(installs(cx), Installs::default());
        fs::create_dir_all(stage.library.join(KEY)).unwrap();
        let heated = stage.install(HEATED);
        fs::create_dir_all(stage.paths.prefix_dir().join(KEY)).unwrap();
        let installed = Install {
            installed: true,
            ..Install::default()
        };
        let partial = Install {
            partial: true,
            ..Install::default()
        };
        assert_eq!(
            installs(cx),
            Installs {
                tb: partial,
                hm: installed,
                prefix: true
            }
        );
        let logs = heated.join("HeatedMetal").join("CrashLogs");
        fs::create_dir_all(&logs).unwrap();
        fs::write(logs.join("crash.dmp"), "").unwrap();
        let mut dll = vec![0; 0x100];
        dll[0x3C] = 0x80;
        dll[0x88..0x8C].copy_from_slice(&1_700_000_000_u32.to_le_bytes());
        fs::write(heated.join("HeatedMetal").join("HeatedMetal.dll"), dll).unwrap();
        let throwback = stage.install(KEY);
        fs::create_dir_all(throwback.join("HeatedMetal").join("CrashLogs")).unwrap();
        let built = Install {
            built: Some(1_700_000_000),
            crash_log: true,
            ..installed
        };
        assert_eq!(
            installs(cx),
            Installs {
                tb: installed,
                hm: built,
                prefix: true
            }
        );
    }

    #[gpui::test]
    fn game_arguments_name_the_other_renderer_and_are_written_back(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "");
        let args = |hm: bool, cx: &mut TestAppContext| {
            let launcher = &stage.launcher;
            launcher.read_with(cx, |launcher, _| launcher.game_args(KEY, hm))
        };
        let set = |value: &str, cx: &mut TestAppContext| {
            let launcher = &stage.launcher;
            launcher.read_with(cx, |launcher, _| launcher.set_game_args(KEY, false, value))
        };
        assert_eq!(args(false, cx), GameArgs::default());
        assert_eq!(set("/vulkan", cx), Err("Not installed".to_owned()));
        let folder = stage.install(KEY);
        assert_eq!(
            set("/vulkan", cx),
            Err("Config.toml edit failed".to_owned())
        );
        fs::write(
            folder.join("Config.toml"),
            "[Launch]\nargs = '/belaunch /nologo'\n",
        )
        .unwrap();
        let read = |renderer: Option<Renderer>, args: &str| GameArgs {
            args: args.to_owned(),
            renderer,
        };
        assert_eq!(args(false, cx), read(None, "/belaunch /nologo"));
        fs::write(folder.join("RainbowSix_DX11.exe"), "").unwrap();
        assert_eq!(set(" /belaunch /nologo /dx11\n", cx), Ok(()));
        let dx11 = "/belaunch /nologo /dx11";
        assert_eq!(args(false, cx), read(Some(Renderer::Dx11), dx11));
        fs::write(folder.join("RainbowSix_Vulkan.exe"), "").unwrap();
        assert_eq!(args(false, cx), read(Some(Renderer::Vulkan), dx11));
        let heated = stage.install(HEATED);
        fs::write(heated.join("RainbowSix_Vulkan.exe"), "").unwrap();
        assert_eq!(args(true, cx), GameArgs::default());
    }

    #[gpui::test(iterations = 10)]
    fn on_windows_play_launches_the_program_directly_and_a_failure_sweeps_nothing(
        cx: &mut TestAppContext,
    ) {
        let settings = "season_proton = { Y5S3_ShadowLegacy = 'ge' }\n\
            launch_args = { Y5S3_ShadowLegacy = 'PROTON_LOG=1 gamemoderun %command% -dx11' }\n";
        let stage = Stage::new(cx, settings);
        stage.host.windows.store(true, Ordering::Relaxed);
        let folder = stage.install(KEY);
        stage.host.refuse.store(true, Ordering::Relaxed);
        play(&stage, KEY, false, cx);
        cx.run_until_parked();
        assert_eq!(stage.events(), [error("Launch failed")]);
        assert_eq!(stage.launching(cx), None);
        assert_eq!(stage.host.actions(), ["stop []", "spawn"]);

        stage.host.refuse.store(false, Ordering::Relaxed);
        play(&stage, KEY, false, cx);
        cx.run_until_parked();
        assert_eq!(stage.host.actions(), ["stop []", "spawn"]);
        assert_eq!(stage.launching(cx), Some(named(KEY, false)));
        assert!(stage.events().is_empty());
        let spawned = stage.host.spawned.lock().unwrap();
        let exe = folder.join("LaunchR6.exe");
        assert_eq!(spawned[0].argv, [exe.to_str().unwrap()]);
        assert_eq!(spawned[0].cwd, folder);
        assert!(spawned[0].env.is_empty());
    }

    #[gpui::test(iterations = 10)]
    fn on_windows_stop_ends_both_editions_by_their_folders_and_sweeps_nothing(
        cx: &mut TestAppContext,
    ) {
        let stage = Stage::new(cx, "");
        stage.host.windows.store(true, Ordering::Relaxed);
        stage.host.run(KEY, &[50, 51]);
        stage.host.run(HEATED, &[60]);
        stage.host.run(VANILLA, &[70]);
        stage.tick(cx);
        stop(&stage, KEY, cx);
        cx.run_until_parked();
        assert_eq!(stage.running(cx), [named(VANILLA, false)]);
        assert_eq!(stage.host.actions(), ["stop [50, 51, 60]"]);
    }

    #[cfg(not(windows))]
    mod proton {
        use super::*;

        fn heat(folder: &Path) {
            let mods = folder.join("HeatedMetal");
            fs::create_dir_all(&mods).unwrap();
            for name in ["HeatedMetal.dll", "HeatedMetalAVX.dll"] {
                fs::write(mods.join(name), "").unwrap();
            }
        }

        #[gpui::test(iterations = 10)]
        fn play_stops_running_games_and_starts_through_the_chosen_proton(cx: &mut TestAppContext) {
            let settings = "proton = 'old'\nseason_proton = { Y5S3_ShadowLegacy = 'ge' }\n\
                launch_args = { Y5S3_ShadowLegacy = 'PROTON_LOG=1 gamemoderun %command% -dx11' }\n";
            let stage = Stage::new(cx, settings);
            let folder = stage.install(KEY);
            heat(&stage.install(HEATED));
            stage.host.offer(&["proton_experimental", "ge", "old"]);
            stage.host.run(VANILLA, &[11, 12]);
            stage.tick(cx);
            assert_eq!(stage.running(cx), [named(VANILLA, false)]);

            stage.notified();
            play(&stage, KEY, false, cx);
            assert_eq!(stage.launching(cx), Some(named(KEY, false)));
            assert_eq!(stage.notified(), 1);
            play(&stage, KEY, false, cx);
            play(&stage, VANILLA, false, cx);
            assert_eq!(stage.notified(), 0);
            let launching = error("R6S is launching");
            assert_eq!(stage.events(), [launching.clone(), launching]);
            cx.run_until_parked();
            assert_eq!(
                stage.host.actions(),
                ["stop [11, 12]", "spawn", "prefix Y1S0_Vanilla", "stop []"]
            );
            let prefix = stage.paths.prefix_dir().join(KEY);
            let variables = |variables: &[(&str, &str)]| -> BTreeMap<String, String> {
                let owned =
                    |(name, value): &(&str, &str)| ((*name).to_owned(), (*value).to_owned());
                variables.iter().map(owned).collect()
            };
            let compat = [
                ("STEAM_COMPAT_CLIENT_INSTALL_PATH", "/steam"),
                ("STEAM_COMPAT_DATA_PATH", prefix.to_str().unwrap()),
            ];
            {
                let spawned = stage.host.spawned.lock().unwrap();
                let exe = folder.join("LaunchR6.exe");
                assert_eq!(
                    spawned[0].argv,
                    [
                        "gamemoderun",
                        "/p/ge/proton",
                        "run",
                        exe.to_str().unwrap(),
                        "-dx11"
                    ]
                );
                assert_eq!(spawned[0].cwd, folder);
                let mut env = variables(&compat);
                env.insert("PROTON_LOG".to_owned(), "1".to_owned());
                assert_eq!(spawned[0].env, env);
            }
            assert_eq!(stage.launching(cx), Some(named(KEY, false)));
            assert!(stage.running(cx).is_empty());

            stage.notified();
            stage.pass(2, cx);
            assert_eq!(stage.notified(), 0);
            stage.host.run(KEY, &[9001]);
            stage.pass(2, cx);
            assert_eq!(stage.running(cx), [named(KEY, false)]);
            assert_eq!(stage.launching(cx), None);
            assert_eq!(stage.notified(), 1);

            play(&stage, KEY, true, cx);
            cx.run_until_parked();
            assert_eq!(stage.host.actions(), ["stop [9001]", "spawn"]);
            {
                let spawned = stage.host.spawned.lock().unwrap();
                let exe = stage.library.join(HEATED).join("RainbowSix.exe");
                assert_eq!(
                    spawned[1].argv,
                    [
                        "gamemoderun",
                        "/p/ge/proton",
                        "run",
                        exe.to_str().unwrap(),
                        "-dx11"
                    ]
                );
                assert_eq!(spawned[1].cwd, stage.library.join(HEATED));
                assert_eq!(spawned[1].env, spawned[0].env);
            }
            assert_eq!(stage.launching(cx), Some(named(KEY, true)));
            assert!(stage.events().is_empty());
        }

        #[gpui::test(iterations = 10)]
        fn the_global_proton_and_plain_options_apply_without_a_season_choice(
            cx: &mut TestAppContext,
        ) {
            let settings = "proton = 'old'\nseason_proton = { Y5S3_ShadowLegacy = 'gone' }\n\
                launch_args = { Y5S3_ShadowLegacy = '-windowed \"a b\"' }\n";
            let stage = Stage::new(cx, settings);
            let folder = stage.install(KEY);
            stage.host.offer(&["proton_experimental", "ge", "old"]);
            play(&stage, KEY, false, cx);
            cx.run_until_parked();
            let spawned = stage.host.spawned.lock().unwrap();
            let exe = folder.join("LaunchR6.exe");
            assert_eq!(
                spawned[0].argv,
                [
                    "/p/old/proton",
                    "run",
                    exe.to_str().unwrap(),
                    "-windowed",
                    "a b"
                ]
            );
            let names: Vec<&str> = spawned[0].env.keys().map(String::as_str).collect();
            assert_eq!(
                names,
                ["STEAM_COMPAT_CLIENT_INSTALL_PATH", "STEAM_COMPAT_DATA_PATH"]
            );
        }

        #[gpui::test(iterations = 10)]
        fn a_season_that_already_runs_stays_launching_until_its_new_process_is_adopted(
            cx: &mut TestAppContext,
        ) {
            let stage = Stage::new(cx, "");
            stage.install(KEY);
            stage.host.offer(&["ge"]);
            *stage.host.prefix.lock().unwrap() = vec![77];
            stage.host.run(KEY, &[50]);
            stage.tick(cx);
            assert_eq!(stage.running(cx), [named(KEY, false)]);
            play(&stage, KEY, false, cx);
            let launcher = &stage.launcher;
            launcher.update(cx, |launcher, cx| launcher.follow_games(cx));
            assert_eq!(stage.launching(cx), Some(named(KEY, false)));
            cx.run_until_parked();
            assert_eq!(stage.host.actions(), ["stop [50]", "spawn"]);
            assert_eq!(stage.launching(cx), Some(named(KEY, false)));
            assert!(stage.running(cx).is_empty());
            stage.host.run(KEY, &[9001]);
            stage.pass(2, cx);
            assert_eq!(stage.launching(cx), None);
            assert_eq!(stage.running(cx), [named(KEY, false)]);
            assert!(stage.events().is_empty());
        }

        #[gpui::test(iterations = 10)]
        fn play_without_proton_reports_it_and_sweeps_the_prefix(cx: &mut TestAppContext) {
            let stage = Stage::new(cx, "");
            stage.install(KEY);
            *stage.host.prefix.lock().unwrap() = vec![77];
            play(&stage, KEY, false, cx);
            assert_eq!(stage.notified(), 1);
            cx.run_until_parked();
            assert_eq!(stage.events(), [error("No Proton found")]);
            assert_eq!(stage.launching(cx), None);
            assert_eq!(stage.notified(), 1);
            assert_eq!(
                stage.host.actions(),
                ["stop []", "prefix Y5S3_ShadowLegacy", "stop [77]"]
            );
            assert_eq!(stage.host.scans(), 3);
            assert!(stage.host.spawned.lock().unwrap().is_empty());
        }

        #[gpui::test(iterations = 10)]
        fn broken_launch_options_and_a_refused_spawn_fail_the_launch(cx: &mut TestAppContext) {
            let stage = Stage::new(cx, "launch_args = { Y1S0_Vanilla = \"-name 'open\" }\n");
            stage.install(VANILLA);
            stage.install(KEY);
            stage.host.offer(&["ge"]);
            play(&stage, VANILLA, false, cx);
            cx.run_until_parked();
            assert_eq!(stage.events(), [error("Launch failed")]);
            assert_eq!(
                stage.host.actions(),
                ["stop []", "prefix Y1S0_Vanilla", "stop []"]
            );
            stage.host.refuse.store(true, Ordering::Relaxed);
            play(&stage, KEY, false, cx);
            cx.run_until_parked();
            assert_eq!(stage.events(), [error("Launch failed")]);
            assert_eq!(
                stage.host.actions(),
                ["stop []", "spawn", "prefix Y5S3_ShadowLegacy", "stop []"]
            );
            assert_eq!(stage.launching(cx), None);
        }

        #[gpui::test(iterations = 10)]
        fn a_launch_process_that_ends_before_the_game_shows_fails_the_launch(
            cx: &mut TestAppContext,
        ) {
            let stage = Stage::new(cx, "");
            stage.install(KEY);
            stage.host.offer(&["ge"]);
            play(&stage, KEY, false, cx);
            cx.run_until_parked();
            stage.pass(2, cx);
            assert_eq!(stage.launching(cx), Some(named(KEY, false)));
            stage.host.exit("exit=1");
            assert!(stage.events().is_empty());
            stage.pass(2, cx);
            assert_eq!(stage.events(), [error("Launch failed")]);
            assert_eq!(stage.launching(cx), None);
            assert_eq!(
                stage.host.actions(),
                ["stop []", "spawn", "prefix Y5S3_ShadowLegacy", "stop []"]
            );

            play(&stage, KEY, false, cx);
            cx.run_until_parked();
            stage.host.exit("exit=0");
            stage.host.run(KEY, &[60]);
            stage.pass(2, cx);
            assert_eq!(stage.launching(cx), None);
            assert_eq!(stage.running(cx), [named(KEY, false)]);
            assert!(stage.events().is_empty());
        }

        #[gpui::test(iterations = 10)]
        fn a_game_that_shows_up_while_a_scan_runs_is_no_failed_launch(cx: &mut TestAppContext) {
            let stage = Stage::new(cx, "");
            stage.install(KEY);
            stage.host.offer(&["ge"]);
            play(&stage, KEY, false, cx);
            cx.run_until_parked();
            *stage.host.late.lock().unwrap() = Some((KEY.to_owned(), vec![60]));
            stage.pass(2, cx);
            assert_eq!(stage.launching(cx), None);
            assert_eq!(stage.running(cx), [named(KEY, false)]);
            assert!(stage.events().is_empty());
        }

        #[gpui::test(iterations = 10)]
        fn stop_before_the_launch_job_runs_spawns_nothing(cx: &mut TestAppContext) {
            let stage = Stage::new(cx, "");
            stage.install(KEY);
            stage.host.offer(&["ge"]);
            *stage.host.prefix.lock().unwrap() = vec![77];
            play(&stage, KEY, false, cx);
            stop(&stage, KEY, cx);
            assert_eq!(stage.launching(cx), None);
            cx.run_until_parked();
            assert_eq!(
                stage.host.actions(),
                ["prefix Y5S3_ShadowLegacy", "stop [77]"]
            );
            assert!(stage.events().is_empty());
        }

        #[gpui::test(iterations = 10)]
        fn a_process_spawned_for_a_stopped_launch_is_halted_instead_of_adopted(
            cx: &mut TestAppContext,
        ) {
            let stage = Stage::new(cx, "");
            stage.install(KEY);
            stage.host.offer(&["ge"]);
            *stage.host.prefix.lock().unwrap() = vec![77];
            play(&stage, KEY, false, cx);
            while stage.host.spawned.lock().unwrap().is_empty() {
                assert!(cx.executor().tick());
            }
            stop(&stage, KEY, cx);
            assert_eq!(stage.launching(cx), None);
            cx.run_until_parked();
            assert_eq!(
                stage.host.actions(),
                [
                    "stop []",
                    "spawn",
                    "stop [9001]",
                    "prefix Y5S3_ShadowLegacy",
                    "stop [77]"
                ]
            );
            stage.host.exit("exit=1");
            stage.pass(2, cx);
            assert_eq!(stage.launching(cx), None);
            assert!(stage.events().is_empty());
        }

        #[gpui::test(iterations = 10)]
        fn stop_halts_an_adopted_launch_and_clears_the_prefix_of_its_season(
            cx: &mut TestAppContext,
        ) {
            let stage = Stage::new(cx, "");
            stage.install(KEY);
            stage.host.offer(&["ge"]);
            *stage.host.prefix.lock().unwrap() = vec![77, 78];
            play(&stage, KEY, false, cx);
            cx.run_until_parked();
            assert_eq!(stage.host.actions(), ["stop []", "spawn"]);
            stop(&stage, VANILLA, cx);
            assert_eq!(stage.launching(cx), Some(named(KEY, false)));
            cx.run_until_parked();
            assert_eq!(
                stage.host.actions(),
                ["prefix Y1S0_Vanilla", "stop [77, 78]"]
            );
            stage.notified();
            stop(&stage, KEY, cx);
            assert_eq!(stage.launching(cx), None);
            assert_eq!(stage.notified(), 1);
            cx.run_until_parked();
            assert_eq!(
                stage.host.actions(),
                ["stop [9001]", "prefix Y5S3_ShadowLegacy", "stop [77, 78]"]
            );
            assert_eq!(stage.notified(), 0);

            stage.host.run(KEY, &[77]);
            stage.tick(cx);
            assert_eq!(stage.running(cx), [named(KEY, false)]);
            stop(&stage, KEY, cx);
            cx.run_until_parked();
            assert!(stage.running(cx).is_empty());
            let sweep = ["prefix Y5S3_ShadowLegacy", "stop [77, 78]"];
            assert_eq!(stage.host.actions(), [sweep, sweep].concat());
            assert!(stage.events().is_empty());
        }

        #[gpui::test(iterations = 10)]
        fn a_closed_season_is_swept_once_no_edition_of_it_runs(cx: &mut TestAppContext) {
            let stage = Stage::new(cx, "");
            *stage.host.prefix.lock().unwrap() = vec![77];
            stage.host.run(KEY, &[50]);
            stage.host.run(HEATED, &[60]);
            stage.tick(cx);
            assert_eq!(stage.running(cx), [named(KEY, false), named(KEY, true)]);
            stage.host.quit(KEY);
            stage.tick(cx);
            assert_eq!(stage.running(cx), [named(KEY, true)]);
            assert!(stage.host.actions().is_empty());
            stage.host.quit(HEATED);
            stage.tick(cx);
            assert!(stage.running(cx).is_empty());
            assert_eq!(
                stage.host.actions(),
                ["prefix Y5S3_ShadowLegacy", "stop [77]"]
            );

            stage.host.run(KEY, &[50]);
            stage.host.run(HEATED, &[60]);
            stage.tick(cx);
            stage.host.quit(KEY);
            stage.host.quit(HEATED);
            stage.tick(cx);
            assert_eq!(
                stage.host.actions(),
                ["prefix Y5S3_ShadowLegacy", "stop [77]"]
            );

            stage.host.run(KEY, &[50]);
            stage.tick(cx);
            stage.host.quit(KEY);
            let launcher = &stage.launcher;
            launcher.update(cx, |launcher, cx| launcher.scan_games(cx));
            while !stage.running(cx).is_empty() {
                assert!(cx.executor().tick());
            }
            stage.host.run(HEATED, &[61]);
            cx.run_until_parked();
            assert!(stage.host.actions().is_empty());
        }
    }
}
