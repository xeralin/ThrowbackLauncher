use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use async_channel::{Receiver, Sender};
use backend::env::{HM_FOLDER_SUFFIX, IS_WINDOWS, NO_PROTON, Paths};
use backend::liberator::{ensure_liberator, liberator_ready};
use backend::log::{exit_text, fail};
use backend::settings::{Entries, Flag};
use backend::steam::{Proton, ProtonEnv, list_protons, resolve_proton, running_game_env};
use gpui::{App, AppContext, Context, EventEmitter, SharedString, Task};
use serde::{Deserialize, Serialize};

use super::Launcher;

const ATTACH_DELAY: Duration = Duration::from_millis(1500);
const ATTACH_DELAY_MAX: Duration = Duration::from_secs(30);
const ACCEPT_POLL: Duration = Duration::from_millis(200);

pub enum LiberatorEvent {
    Error(SharedString),
    Exclusion,
    Restored,
}

impl EventEmitter<LiberatorEvent> for Launcher {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    Deathless,
    DisableAi,
    UnlimitedAmmo,
    UnlimitedEquip,
    InfiniteTime,
    DisablePrimary,
    DisableSecondary,
    DisablePrimaryGadget,
    DisableSecondaryGadget,
    DisplayBuild,
    Skip,
    EndMatch,
    FullFeature,
}

impl Capability {
    pub const fn key(self) -> &'static str {
        match self {
            Self::Deathless => "deathless",
            Self::DisableAi => "disableAI",
            Self::UnlimitedAmmo => "unlimitedAmmo",
            Self::UnlimitedEquip => "unlimitedEquip",
            Self::InfiniteTime => "infiniteTime",
            Self::DisablePrimary => "disablePrimary",
            Self::DisableSecondary => "disableSecondary",
            Self::DisablePrimaryGadget => "disablePrimaryGadget",
            Self::DisableSecondaryGadget => "disableSecondaryGadget",
            Self::DisplayBuild => "displayBuild",
            Self::Skip => "skip",
            Self::EndMatch => "endMatch",
            Self::FullFeature => "fullFeature",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Capabilities(BTreeMap<String, bool>);

impl Capabilities {
    pub fn reported(&self, capability: Capability) -> bool {
        self.0.contains_key(capability.key())
    }

    pub fn has(&self, capability: Capability) -> bool {
        self.0.get(capability.key()) == Some(&true)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct State {
    pub attached: bool,
    pub unlocked: bool,
    pub unsupported: bool,
    pub capabilities: Capabilities,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Node {
    pub text: SharedString,
    pub id: SharedString,
    #[serde(default)]
    pub children: Vec<Node>,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
enum Event {
    State(State),
    Tree { tree: Option<Vec<Node>> },
}

#[derive(Serialize)]
#[serde(tag = "cmd", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Request<'a> {
    SetMod { r#mod: &'a str, enabled: bool },
    SetPlaylist { playlist_id: &'a str },
    Skip,
    EndMatch,
}

trait Process: Send {
    fn exited(&mut self) -> Option<String>;

    fn stop(self: Box<Self>);
}

impl Process for Child {
    fn exited(&mut self) -> Option<String> {
        Some(exit_text(self.try_wait().ok().flatten()?))
    }

    fn stop(mut self: Box<Self>) {
        let _ = self.kill();
        thread::spawn(move || self.wait());
    }
}

trait Host: Send + Sync {
    fn fetch(&self, paths: &Paths, force: bool) -> Result<PathBuf, String>;

    fn game_env(&self, libraries: &[PathBuf]) -> Option<ProtonEnv>;

    fn protons(&self, paths: &Paths) -> Vec<Proton>;

    fn spawn(&self, command: Command) -> io::Result<Box<dyn Process>>;
}

struct System;

impl Host for System {
    fn fetch(&self, paths: &Paths, force: bool) -> Result<PathBuf, String> {
        ensure_liberator(paths, None, force).map_err(|error| error.to_string())
    }

    fn game_env(&self, libraries: &[PathBuf]) -> Option<ProtonEnv> {
        running_game_env(libraries)
    }

    fn protons(&self, paths: &Paths) -> Vec<Proton> {
        list_protons(paths)
    }

    fn spawn(&self, mut command: Command) -> io::Result<Box<dyn Process>> {
        Ok(Box::new(command.spawn()?))
    }
}

struct Plan {
    paths: Paths,
    libraries: Vec<PathBuf>,
    season_proton: Vec<(String, String)>,
    proton: String,
}

type Launched = (Box<dyn Process>, TcpListener);

fn launch(host: &dyn Host, plan: &Plan) -> Result<Launched, Option<String>> {
    let failed = |error: io::Error| Some(fail("Liberator loading failed", error));
    let bin = host.fetch(&plan.paths, false).map_err(Some)?;
    let mut command = if IS_WINDOWS {
        Command::new(bin)
    } else {
        let Some(env) = host.game_env(&plan.libraries) else {
            return Err(None);
        };
        let key = env.prefix.file_name().unwrap_or_default().to_string_lossy();
        let pinned = plan
            .season_proton
            .iter()
            .find(|(season, _)| *season == *key);
        let season = pinned.map_or("", |(_, internal)| internal);
        let protons = host.protons(&plan.paths);
        let Some(proton) = resolve_proton(&protons, season, &plan.proton) else {
            return Err(Some(NO_PROTON.to_owned()));
        };
        let target = format!("Z:{}", bin.to_string_lossy().replace('/', "\\"));
        let mut command = Command::new(&proton.binary);
        command.arg("run").arg(target).envs(env.vars());
        command
    };
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(failed)?;
    listener.set_nonblocking(true).map_err(failed)?;
    let port = listener.local_addr().map_err(failed)?.port();
    command
        .arg("--runtime")
        .arg(port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let process = host.spawn(command).map_err(failed)?;
    Ok((process, listener))
}

fn accept(listener: &TcpListener) -> io::Result<Option<(TcpStream, TcpStream)>> {
    match listener.accept() {
        Ok((stream, _)) => {
            stream.set_nonblocking(false)?;
            Ok(Some((stream.try_clone()?, stream)))
        }
        Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
        Err(error) => Err(error),
    }
}

fn read(stream: TcpStream, events: Sender<Event>) {
    for line in BufReader::new(stream).split(b'\n') {
        let Ok(line) = line else {
            break;
        };
        if let Ok(event) = serde_json::from_slice(&line)
            && events.send_blocking(event).is_err()
        {
            break;
        }
    }
}

struct Session {
    process: Box<dyn Process>,
    stream: Option<TcpStream>,
    _task: Task<()>,
}

#[derive(Default)]
struct Selection {
    mods: BTreeSet<&'static str>,
    path: Vec<usize>,
    picked: SharedString,
}

pub struct Liberator {
    paths: Paths,
    host: Arc<dyn Host>,
    active: bool,
    busy: bool,
    tried: bool,
    keys: BTreeSet<String>,
    delay: Duration,
    next_attempt: Option<Instant>,
    session: Option<Session>,
    state: State,
    tree: Option<Vec<Node>>,
    selection: Selection,
}

impl Liberator {
    pub fn new(paths: Paths) -> Self {
        Self {
            paths,
            host: Arc::new(System),
            active: false,
            busy: false,
            tried: false,
            keys: BTreeSet::new(),
            delay: ATTACH_DELAY,
            next_attempt: None,
            session: None,
            state: State::default(),
            tree: None,
            selection: Selection::default(),
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn tree(&self) -> Option<&[Node]> {
        self.tree.as_deref()
    }

    pub fn available(&self) -> bool {
        !self.tried || liberator_ready(&self.paths)
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    pub fn switched(&self, capability: Capability) -> bool {
        self.selection.mods.contains(capability.key())
    }

    pub fn path(&self) -> &[usize] {
        &self.selection.path
    }

    pub fn picked(&self) -> &SharedString {
        &self.selection.picked
    }

    pub fn open(&mut self, path: Vec<usize>, cx: &mut Context<Launcher>) {
        self.selection.path = path;
        cx.notify();
    }

    pub fn set_mod(&mut self, capability: Capability, enabled: bool, cx: &mut Context<Launcher>) {
        let r#mod = capability.key();
        if enabled {
            self.selection.mods.insert(r#mod);
        } else {
            self.selection.mods.remove(r#mod);
        }
        self.send(&Request::SetMod { r#mod, enabled });
        cx.notify();
    }

    pub fn set_playlist(&mut self, playlist_id: &SharedString, cx: &mut Context<Launcher>) {
        self.selection.picked = playlist_id.clone();
        self.send(&Request::SetPlaylist { playlist_id });
        cx.notify();
    }

    pub fn skip(&self) {
        self.send(&Request::Skip);
    }

    pub fn end_match(&self) {
        self.send(&Request::EndMatch);
    }

    pub fn restore(&mut self, cx: &mut Context<Launcher>) {
        self.fetch(true, cx);
    }

    fn fetch(&mut self, asked: bool, cx: &mut Context<Launcher>) {
        if self.busy {
            return;
        }
        self.busy = true;
        cx.notify();
        let (host, paths) = (self.host.clone(), self.paths.clone());
        let fetch = cx.background_spawn(async move { host.fetch(&paths, true) });
        cx.spawn(async move |this, cx| {
            let fetched = fetch.await;
            this.update(cx, |this, cx| {
                this.liberator.busy = false;
                if asked {
                    this.liberator.tried = true;
                    cx.emit(match fetched {
                        Ok(_) => LiberatorEvent::Restored,
                        Err(message) => LiberatorEvent::Error(message.into()),
                    });
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn stop(&mut self, cx: &mut Context<Launcher>) {
        self.active = false;
        self.kill(cx);
    }

    fn send(&self, request: &Request) {
        let session = self.session.as_ref();
        let stream = session.and_then(|session| session.stream.as_ref());
        if let (Some(mut stream), Ok(mut line)) = (stream, serde_json::to_vec(request)) {
            line.push(b'\n');
            let _ = stream.write_all(&line);
        }
    }

    fn retrying(&self) -> bool {
        self.delay > ATTACH_DELAY
    }

    fn reset_delay(&mut self) {
        self.delay = ATTACH_DELAY;
        self.next_attempt = None;
    }

    fn back_off(&mut self, cx: &App) {
        self.next_attempt = Some(cx.background_executor().now() + self.delay);
        self.delay = (self.delay * 2).min(ATTACH_DELAY_MAX);
    }

    fn kill(&mut self, cx: &mut Context<Launcher>) {
        if let Some(session) = self.session.take() {
            if let Some(stream) = &session.stream {
                let _ = stream.shutdown(Shutdown::Both);
            }
            session.process.stop();
        }
        self.set_state(State::default());
        self.tree = None;
        cx.notify();
    }

    fn set_state(&mut self, state: State) {
        if !state.attached {
            self.selection = Selection::default();
        }
        self.state = state;
    }

    fn launched(&mut self, launched: Result<Launched, Option<String>>, cx: &mut Context<Launcher>) {
        self.busy = false;
        self.tried = true;
        cx.notify();
        let (process, listener) = match launched {
            Ok(launched) if self.active => launched,
            Ok((process, _)) => {
                process.stop();
                return self.back_off(cx);
            }
            Err(message) => {
                if let Some(message) = message {
                    cx.emit(LiberatorEvent::Error(message.into()));
                }
                return self.back_off(cx);
            }
        };
        let task = cx.spawn(async move |this, cx| {
            let events = loop {
                match this.update(cx, |this, cx| this.liberator.connect(&listener, cx)) {
                    Ok(ControlFlow::Continue(())) => {
                        cx.background_executor().timer(ACCEPT_POLL).await;
                    }
                    Ok(ControlFlow::Break(Some(events))) => break events,
                    _ => return,
                }
            };
            drop(listener);
            while let Ok(event) = events.recv().await {
                if this
                    .update(cx, |this, cx| this.liberator.apply(event, cx))
                    .is_err()
                {
                    return;
                }
            }
            this.update(cx, |this, cx| this.liberator.lost(cx)).ok();
        });
        self.session = Some(Session {
            process,
            stream: None,
            _task: task,
        });
    }

    fn connect(
        &mut self,
        listener: &TcpListener,
        cx: &mut Context<Launcher>,
    ) -> ControlFlow<Option<Receiver<Event>>> {
        let Some(session) = &mut self.session else {
            return ControlFlow::Break(None);
        };
        let failure = match session.process.exited() {
            Some(exit) => {
                fail("Liberator Runtime exited", exit);
                None
            }
            None => match accept(listener) {
                Ok(None) => return ControlFlow::Continue(()),
                Ok(Some((stream, reader))) => {
                    let (sender, events) = async_channel::unbounded();
                    thread::spawn(move || read(reader, sender));
                    session.stream = Some(stream);
                    self.delay = ATTACH_DELAY;
                    return ControlFlow::Break(Some(events));
                }
                Err(error) => Some(fail("Liberator loading failed", error)),
            },
        };
        if let Some(message) = failure {
            cx.emit(LiberatorEvent::Error(message.into()));
        }
        self.lost(cx);
        ControlFlow::Break(None)
    }

    fn apply(&mut self, event: Event, cx: &mut Context<Launcher>) {
        match event {
            Event::State(state) => self.set_state(state),
            Event::Tree { tree } => self.tree = tree,
        }
        cx.notify();
    }

    fn lost(&mut self, cx: &mut Context<Launcher>) {
        self.back_off(cx);
        self.kill(cx);
    }
}

#[cfg(any(test, feature = "qa"))]
struct Offline;

#[cfg(any(test, feature = "qa"))]
impl Host for Offline {
    fn fetch(&self, paths: &Paths, _: bool) -> Result<PathBuf, String> {
        let bin = paths.liberator_bin();
        std::fs::create_dir_all(paths.bin_dir())
            .and_then(|()| std::fs::write(&bin, ""))
            .and_then(|()| std::fs::write(bin.with_extension("version"), backend::env::VERSION))
            .map_err(|error| error.to_string())?;
        Ok(bin)
    }

    fn game_env(&self, _: &[PathBuf]) -> Option<ProtonEnv> {
        None
    }

    fn protons(&self, _: &Paths) -> Vec<Proton> {
        Vec::new()
    }

    fn spawn(&self, _: Command) -> io::Result<Box<dyn Process>> {
        Err(ErrorKind::Unsupported.into())
    }
}

#[cfg(any(test, feature = "qa"))]
impl Liberator {
    pub fn stage(&mut self, lines: &str) {
        self.host = Arc::new(Offline);
        for line in lines.lines() {
            match (line, serde_json::from_str(line)) {
                ("tried", _) => self.tried = true,
                ("busy", _) => self.busy = true,
                (_, Ok(Event::State(state))) => self.set_state(state),
                (_, Ok(Event::Tree { tree })) => self.tree = tree,
                _ => {}
            }
        }
    }
}

impl Launcher {
    pub fn set_liberator_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.settings.flag(Flag::LiberatorEnabled) == enabled {
            return;
        }
        self.settings.set_flag(Flag::LiberatorEnabled, enabled, cx);
        if enabled {
            self.start_liberator(cx);
        } else {
            self.liberator.stop(cx);
        }
    }

    pub fn prepare_liberator(&mut self, windows: bool, cx: &mut Context<Self>) {
        let paths = &self.liberator.paths;
        if !self.settings.flag(Flag::LiberatorEnabled) || liberator_ready(paths) {
            return;
        }
        if windows && !paths.liberator_bin().exists() {
            cx.emit(LiberatorEvent::Exclusion);
        } else {
            self.liberator.fetch(false, cx);
        }
    }

    pub fn start_liberator(&mut self, cx: &mut Context<Self>) {
        self.liberator.active = true;
        self.liberator.reset_delay();
        self.tick_liberator(cx);
    }

    pub fn tick_liberator(&mut self, cx: &mut Context<Self>) {
        let part = &mut self.liberator;
        if !part.active {
            return;
        }
        let folders = self.watchdog.folders().keys();
        let throwback = folders.filter(|folder| !folder.ends_with(HM_FOLDER_SUFFIX));
        let keys: BTreeSet<String> = throwback.cloned().collect();
        let alive = part.session.is_some();
        if keys.is_empty() || (alive && keys.is_disjoint(&part.keys)) {
            part.reset_delay();
            if alive {
                part.kill(cx);
            }
        }
        let idle = part.session.is_none() && !part.busy;
        let now = cx.background_executor().now();
        let due = part.next_attempt.is_none_or(|at| now >= at);
        if keys.is_empty() || !idle || !due || (part.retrying() && !liberator_ready(&part.paths)) {
            return;
        }
        part.busy = true;
        part.keys = keys;
        cx.notify();
        let host = part.host.clone();
        let season_proton = self.settings.store.entries(Entries::SeasonProton);
        let proton = self.settings.stored_proton().to_owned();
        let plan = Plan {
            paths: part.paths.clone(),
            libraries: self.settings.roots().to_vec(),
            season_proton,
            proton,
        };
        let launch = cx.background_spawn(async move { launch(&*host, &plan) });
        cx.spawn(async move |this, cx| {
            let launched = launch.await;
            this.update(cx, |this, cx| this.liberator.launched(launched, cx))
                .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::ffi::OsStr;
    use std::fs;
    use std::rc::Rc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use backend::settings::Settings as Store;
    use gpui::{Entity, TestAppContext};
    use tempfile::TempDir;

    use super::*;

    const KEY: &str = "Y5S3_ShadowLegacy";
    const OTHER: &str = "Y1S0_Vanilla";
    const HEATED: &str = "Y5S3_HeatedMetal";
    const ATTACHED: &str = "{\"event\":\"state\",\"attached\":true,\"unlocked\":true,\
        \"capabilities\":{\"fullFeature\":true,\"skip\":false,\"later\":true}}\n";
    const TREE: &str = "{\"event\":\"tree\",\"tree\":[{\"text\":\"Multiplayer\",\"id\":\"\",\
        \"children\":[{\"text\":\"Bomb\",\"id\":\"bomb\",\"children\":[]}]}]}\n";

    #[derive(Default)]
    struct Fake {
        fetches: Mutex<Vec<bool>>,
        fetch_error: Mutex<Option<String>>,
        env: Mutex<Option<ProtonEnv>>,
        protons: Mutex<Vec<Proton>>,
        commands: Mutex<Vec<Vec<String>>>,
        refuse: AtomicBool,
        exit: Arc<Mutex<Option<String>>>,
        stopped: Arc<AtomicUsize>,
    }

    struct Runtime {
        exit: Arc<Mutex<Option<String>>>,
        stopped: Arc<AtomicUsize>,
    }

    impl Process for Runtime {
        fn exited(&mut self) -> Option<String> {
            self.exit.lock().unwrap().clone()
        }

        fn stop(self: Box<Self>) {
            self.stopped.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn install(paths: &Paths) {
        Offline.fetch(paths, true).unwrap();
    }

    impl Host for Fake {
        fn fetch(&self, paths: &Paths, force: bool) -> Result<PathBuf, String> {
            self.fetches.lock().unwrap().push(force);
            match self.fetch_error.lock().unwrap().clone() {
                Some(error) => Err(error),
                None => {
                    install(paths);
                    Ok(paths.liberator_bin())
                }
            }
        }

        fn game_env(&self, _: &[PathBuf]) -> Option<ProtonEnv> {
            self.env.lock().unwrap().clone()
        }

        fn protons(&self, _: &Paths) -> Vec<Proton> {
            self.protons.lock().unwrap().clone()
        }

        fn spawn(&self, command: Command) -> io::Result<Box<dyn Process>> {
            if self.refuse.load(Ordering::SeqCst) {
                return Err(ErrorKind::PermissionDenied.into());
            }
            let text = |part: &OsStr| part.to_string_lossy().into_owned();
            let mut line = vec![text(command.get_program())];
            line.extend(command.get_args().map(text));
            let envs = command.get_envs();
            line.extend(envs.map(|(name, value)| {
                format!("{}={}", text(name), text(value.unwrap_or_default()))
            }));
            self.commands.lock().unwrap().push(line);
            Ok(Box::new(Runtime {
                exit: self.exit.clone(),
                stopped: self.stopped.clone(),
            }))
        }
    }

    struct Peer {
        stream: TcpStream,
        lines: BufReader<TcpStream>,
    }

    impl Peer {
        fn say(&mut self, text: &str) {
            self.stream.write_all(text.as_bytes()).unwrap();
        }

        fn heard(&mut self) -> String {
            let mut line = String::new();
            self.lines.read_line(&mut line).unwrap();
            line
        }

        fn closed(&mut self) -> bool {
            self.heard().is_empty()
        }
    }

    struct Stage {
        dir: TempDir,
        paths: Paths,
        host: Arc<Fake>,
        launcher: Entity<Launcher>,
        errors: Rc<RefCell<Vec<String>>>,
    }

    fn proton(internal: &str) -> Proton {
        Proton {
            display: internal.to_uppercase(),
            internal: internal.to_owned(),
            binary: PathBuf::from(format!("/p/{internal}/proton")),
        }
    }

    impl Stage {
        fn new(cx: &mut TestAppContext, settings: &str) -> Self {
            cx.executor().allow_parking();
            let dir = tempfile::tempdir().unwrap();
            let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
            fs::create_dir_all(&paths.data_root).unwrap();
            fs::write(paths.settings_file(), settings).unwrap();
            let store = Store::load(&paths).unwrap();
            let host = Arc::new(Fake::default());
            *host.env.lock().unwrap() = Some(ProtonEnv {
                prefix: dir.path().join("prefixes").join(KEY),
                steam: dir.path().join("steam"),
            });
            *host.protons.lock().unwrap() = ["proton_experimental", "ge"].map(proton).to_vec();
            let launcher = cx.new(|_| {
                let mut launcher = Launcher::new(paths.clone(), store, Vec::new());
                launcher.liberator.host = host.clone();
                launcher
            });
            let errors = Rc::new(RefCell::new(Vec::new()));
            let seen = errors.clone();
            cx.update(|cx| {
                cx.subscribe(&launcher, move |_, event: &LiberatorEvent, _| {
                    seen.borrow_mut().push(match event {
                        LiberatorEvent::Error(message) => message.to_string(),
                        LiberatorEvent::Exclusion => "exclusion".to_owned(),
                        LiberatorEvent::Restored => "restored".to_owned(),
                    });
                })
                .detach();
            });
            Self {
                dir,
                paths,
                host,
                launcher,
                errors,
            }
        }

        fn show(&self, cx: &mut TestAppContext, folders: &[&str]) {
            let folders = folders.iter().map(|folder| ((*folder).to_owned(), vec![1]));
            let folders = folders.collect();
            self.launcher
                .update(cx, |launcher, _| launcher.watchdog.set_folders(folders));
        }

        fn tick(&self, cx: &mut TestAppContext, folders: &[&str]) {
            self.show(cx, folders);
            self.launcher
                .update(cx, |launcher, cx| launcher.tick_liberator(cx));
            cx.run_until_parked();
        }

        fn start(&self, cx: &mut TestAppContext, folders: &[&str]) {
            self.show(cx, folders);
            self.launcher
                .update(cx, |launcher, cx| launcher.start_liberator(cx));
            cx.run_until_parked();
        }

        fn stop(&self, cx: &mut TestAppContext) {
            self.launcher
                .update(cx, |launcher, cx| launcher.liberator.stop(cx));
            cx.run_until_parked();
        }

        fn pass(&self, cx: &mut TestAppContext, time: Duration) {
            cx.executor().advance_clock(time);
        }

        fn attempts(&self) -> usize {
            self.host.fetches.lock().unwrap().len()
        }

        fn spawned(&self) -> usize {
            self.host.commands.lock().unwrap().len()
        }

        fn stopped(&self) -> usize {
            self.host.stopped.load(Ordering::SeqCst)
        }

        fn errors(&self) -> Vec<String> {
            self.errors.borrow().clone()
        }

        fn read<R>(&self, cx: &mut TestAppContext, read: impl FnOnce(&Liberator) -> R) -> R {
            self.launcher
                .read_with(cx, |launcher, _| read(&launcher.liberator))
        }

        fn connect(&self, cx: &mut TestAppContext) -> Peer {
            let port = {
                let commands = self.host.commands.lock().unwrap();
                let mut args = commands.last().unwrap().iter();
                args.find(|arg| *arg == "--runtime").unwrap();
                args.next().unwrap().parse::<u16>().unwrap()
            };
            let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
            let wait = Some(Duration::from_secs(10));
            stream.set_read_timeout(wait).unwrap();
            let lines = BufReader::new(stream.try_clone().unwrap());
            self.pass(cx, ACCEPT_POLL);
            Peer { stream, lines }
        }

        fn settle(&self, cx: &mut TestAppContext, done: impl Fn(&Liberator) -> bool) {
            for _ in 0..2000 {
                cx.run_until_parked();
                if self.read(cx, &done) {
                    return;
                }
                thread::sleep(Duration::from_millis(5));
            }
            panic!("the liberator did not settle");
        }

        fn attach(&self, cx: &mut TestAppContext) -> Peer {
            let mut peer = self.connect(cx);
            peer.say(ATTACHED);
            self.settle(cx, |liberator| liberator.state().attached);
            peer
        }
    }

    #[test]
    fn lines_of_the_runtime_are_read_leniently() {
        let event = |line: &str| serde_json::from_str::<Event>(line).ok();
        let Some(Event::State(state)) = event(ATTACHED) else {
            panic!("state expected");
        };
        assert!(state.attached && state.unlocked && !state.unsupported);
        assert!(state.capabilities.has(Capability::FullFeature));
        assert!(state.capabilities.reported(Capability::Skip));
        assert!(!state.capabilities.has(Capability::Skip));
        assert!(!state.capabilities.reported(Capability::EndMatch));
        assert_eq!(
            event("{\"event\":\"state\",\"unsupported\":true,\"build\":5}"),
            Some(Event::State(State {
                unsupported: true,
                ..State::default()
            }))
        );
        let Some(Event::Tree { tree: Some(tree) }) = event(TREE) else {
            panic!("tree expected");
        };
        assert_eq!((tree.len(), tree[0].text.as_ref()), (1, "Multiplayer"));
        assert_eq!(tree[0].children[0].id, "bomb");
        assert!(tree[0].children[0].children.is_empty());
        for empty in ["{\"event\":\"tree\",\"tree\":null}", "{\"event\":\"tree\"}"] {
            assert_eq!(event(empty), Some(Event::Tree { tree: None }));
        }
        assert_eq!(Capability::DisableAi.key(), "disableAI");
    }

    #[gpui::test]
    fn ticks_count_only_once_started_and_never_for_heated_metal(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "[settings]\n");
        stage.tick(cx, &[KEY]);
        assert_eq!(stage.attempts(), 0);
        stage.start(cx, &[]);
        stage.tick(cx, &[HEATED]);
        assert_eq!(stage.attempts(), 0);
        stage.tick(cx, &[HEATED, KEY]);
        assert_eq!((stage.attempts(), stage.spawned()), (1, 1));
        assert_eq!(*stage.host.fetches.lock().unwrap(), [false]);
        stage.tick(cx, &[HEATED, KEY]);
        stage.tick(cx, &[KEY]);
        assert_eq!((stage.attempts(), stage.spawned()), (1, 1));
        assert!(stage.errors().is_empty());
    }

    #[cfg(not(windows))]
    #[gpui::test]
    fn the_runtime_runs_under_the_proton_and_prefix_of_the_game(cx: &mut TestAppContext) {
        let settings =
            format!("[settings]\nproton = 'missing'\nseason_proton = {{ {KEY} = 'ge' }}\n");
        let stage = Stage::new(cx, &settings);
        stage.start(cx, &[KEY]);
        let command = stage.host.commands.lock().unwrap()[0].clone();
        let bin = stage.paths.liberator_bin();
        let target = format!("Z:{}", bin.to_str().unwrap().replace('/', "\\"));
        assert!(!target.contains('/') && target.ends_with("\\bin\\Liberator.exe"));
        let root = stage.dir.path();
        assert_eq!(
            command,
            [
                "/p/ge/proton".to_owned(),
                "run".to_owned(),
                target,
                "--runtime".to_owned(),
                command[4].clone(),
                format!(
                    "STEAM_COMPAT_CLIENT_INSTALL_PATH={}",
                    root.join("steam").display()
                ),
                format!(
                    "STEAM_COMPAT_DATA_PATH={}",
                    root.join("prefixes").join(KEY).display()
                ),
            ]
        );
        assert!(command[4].parse::<u16>().unwrap() > 0);
        stage.stop(cx);
        let other = root.join("prefixes").join(OTHER);
        stage.host.env.lock().unwrap().as_mut().unwrap().prefix = other;
        stage.start(cx, &[OTHER]);
        let command = stage.host.commands.lock().unwrap()[1].clone();
        assert_eq!(command[0], "/p/proton_experimental/proton");
    }

    #[gpui::test]
    fn state_tree_and_calls_cross_the_socket(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "[settings]\n");
        stage.start(cx, &[KEY]);
        stage.read(cx, |liberator| {
            liberator.skip();
            assert_eq!(liberator.state(), &State::default());
            assert!(liberator.tree().is_none() && liberator.available());
        });
        let mut peer = stage.connect(cx);
        peer.say("attached\n{\"event\":\"other\"}\n");
        peer.say(ATTACHED);
        stage.settle(cx, |liberator| liberator.state().attached);
        stage.read(cx, |liberator| {
            let state = liberator.state();
            assert!(state.unlocked && !state.unsupported);
            assert!(state.capabilities.has(Capability::FullFeature));
            assert!(!state.capabilities.has(Capability::Skip));
            assert!(liberator.tree().is_none());
        });
        peer.say(TREE);
        stage.settle(cx, |liberator| liberator.tree().is_some());
        stage.launcher.update(cx, |launcher, cx| {
            let liberator = &mut launcher.liberator;
            assert_eq!(liberator.tree().unwrap()[0].children[0].text, "Bomb");
            liberator.set_mod(Capability::DisableAi, true, cx);
            liberator.set_mod(Capability::Deathless, false, cx);
            liberator.set_playlist(&"bomb".into(), cx);
            liberator.skip();
            liberator.end_match();
        });
        let heard: Vec<String> = (0..5).map(|_| peer.heard()).collect();
        assert_eq!(
            heard,
            [
                "{\"cmd\":\"setMod\",\"mod\":\"disableAI\",\"enabled\":true}\n",
                "{\"cmd\":\"setMod\",\"mod\":\"deathless\",\"enabled\":false}\n",
                "{\"cmd\":\"setPlaylist\",\"playlistId\":\"bomb\"}\n",
                "{\"cmd\":\"skip\"}\n",
                "{\"cmd\":\"endMatch\"}\n",
            ]
        );
        peer.say("{\"event\":\"tree\",\"tree\":null}\n");
        stage.settle(cx, |liberator| liberator.tree().is_none());
        peer.say("{\"event\":\"state\",\"attached\":true,\"unsupported\":true}\n");
        stage.settle(cx, |liberator| liberator.state().unsupported);
        assert!(stage.read(cx, |liberator| !liberator.state().unlocked));
        assert!(stage.errors().is_empty());
        assert_eq!((stage.spawned(), stage.stopped()), (1, 0));
    }

    #[gpui::test]
    fn the_selection_lasts_until_the_runtime_is_gone(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "[settings]\n");
        let select = |cx: &mut TestAppContext| {
            stage.launcher.update(cx, |launcher, cx| {
                let liberator = &mut launcher.liberator;
                liberator.set_mod(Capability::Deathless, true, cx);
                liberator.set_mod(Capability::DisableAi, true, cx);
                liberator.set_mod(Capability::DisableAi, false, cx);
                liberator.open(vec![0, 2], cx);
                liberator.set_playlist(&"bomb".into(), cx);
            });
        };
        let selected = |cx: &mut TestAppContext| {
            stage.read(cx, |liberator| {
                let mods = [Capability::Deathless, Capability::DisableAi];
                let path = liberator.path().to_vec();
                (
                    mods.map(|on| liberator.switched(on)),
                    path,
                    liberator.picked().clone(),
                )
            })
        };
        let kept = ([true, false], vec![0, 2], "bomb".into());
        let cleared = ([false, false], Vec::new(), SharedString::default());
        assert_eq!(selected(cx), cleared);

        stage.start(cx, &[KEY]);
        let mut peer = stage.attach(cx);
        select(cx);
        peer.say("{\"event\":\"state\",\"attached\":true,\"unsupported\":true}\n");
        stage.settle(cx, |liberator| liberator.state().unsupported);
        peer.say(TREE);
        stage.settle(cx, |liberator| liberator.tree().is_some());
        assert_eq!(selected(cx), kept);
        peer.say("{\"event\":\"state\",\"unlocked\":true}\n");
        stage.settle(cx, |liberator| !liberator.state().attached);
        assert_eq!(selected(cx), cleared);

        select(cx);
        let heard: Vec<String> = (0..8).map(|_| peer.heard()).collect();
        assert_eq!(heard[..4], heard[4..]);
        assert_eq!(
            heard[3],
            "{\"cmd\":\"setPlaylist\",\"playlistId\":\"bomb\"}\n"
        );
        stage.tick(cx, &[]);
        assert!(peer.closed());
        assert_eq!(selected(cx), cleared);

        stage.tick(cx, &[KEY]);
        let peer = stage.attach(cx);
        select(cx);
        drop(peer);
        stage.settle(cx, |liberator| !liberator.state().attached);
        assert_eq!(selected(cx), cleared);

        select(cx);
        stage.stop(cx);
        assert_eq!(selected(cx), cleared);
    }

    #[gpui::test]
    fn the_runtime_ends_with_its_season_and_follows_a_new_one(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "[settings]\n");
        stage.start(cx, &[KEY]);
        let mut peer = stage.attach(cx);
        peer.say(TREE);
        stage.settle(cx, |liberator| liberator.tree().is_some());
        stage.tick(cx, &[KEY, OTHER]);
        assert_eq!((stage.spawned(), stage.stopped()), (1, 0));

        stage.tick(cx, &[]);
        assert!(peer.closed());
        assert_eq!((stage.spawned(), stage.stopped()), (1, 1));
        stage.read(cx, |liberator| {
            assert_eq!(liberator.state(), &State::default());
            assert!(liberator.tree().is_none());
            liberator.skip();
        });

        stage.tick(cx, &[KEY]);
        assert_eq!(stage.spawned(), 2);
        let mut peer = stage.attach(cx);
        stage.tick(cx, &[OTHER]);
        assert!(peer.closed());
        assert_eq!((stage.spawned(), stage.stopped()), (3, 2));
        assert!(stage.read(cx, |liberator| !liberator.state().attached));
        let mut peer = stage.attach(cx);

        stage.tick(cx, &[HEATED]);
        assert!(peer.closed());
        assert_eq!((stage.spawned(), stage.stopped()), (3, 3));
        assert!(stage.errors().is_empty());
    }

    #[gpui::test]
    fn a_season_change_while_connecting_replaces_the_runtime(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "[settings]\n");
        stage.host.refuse.store(true, Ordering::SeqCst);
        stage.start(cx, &[KEY]);
        stage.host.refuse.store(false, Ordering::SeqCst);
        stage.pass(cx, ATTACH_DELAY);
        stage.tick(cx, &[KEY]);
        stage.pass(cx, Duration::from_secs(90));
        stage.tick(cx, &[KEY]);
        assert_eq!((stage.spawned(), stage.stopped()), (1, 0));

        stage.host.refuse.store(true, Ordering::SeqCst);
        stage.tick(cx, &[OTHER]);
        assert_eq!((stage.attempts(), stage.stopped()), (3, 1));
        stage.host.refuse.store(false, Ordering::SeqCst);
        stage.pass(cx, ATTACH_DELAY - Duration::from_millis(1));
        stage.tick(cx, &[OTHER]);
        assert_eq!(stage.attempts(), 3);
        stage.pass(cx, Duration::from_millis(1));
        stage.tick(cx, &[OTHER]);
        assert_eq!((stage.spawned(), stage.stopped()), (2, 1));
        stage.attach(cx);
        assert_eq!(stage.errors(), ["Liberator loading failed"; 2]);
    }

    #[cfg(not(windows))]
    #[gpui::test]
    fn failed_attempts_wait_longer_each_time(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "[settings]\n");
        *stage.host.env.lock().unwrap() = None;
        stage.start(cx, &[KEY]);
        let mut attempts = 1;
        assert_eq!(stage.attempts(), attempts);
        for wait in [1500, 3000, 6000, 12000, 24000, 30000, 30000] {
            stage.pass(cx, Duration::from_millis(wait - 1));
            stage.tick(cx, &[KEY]);
            assert_eq!(stage.attempts(), attempts, "{wait}");
            stage.pass(cx, Duration::from_millis(1));
            stage.tick(cx, &[KEY]);
            attempts += 1;
            assert_eq!(stage.attempts(), attempts, "{wait}");
        }
        stage.tick(cx, &[]);
        stage.tick(cx, &[KEY]);
        assert_eq!(stage.attempts(), attempts + 1);
        stage.pass(cx, Duration::from_millis(1499));
        stage.tick(cx, &[KEY]);
        assert_eq!(stage.attempts(), attempts + 1);
        stage.start(cx, &[KEY]);
        assert_eq!(stage.attempts(), attempts + 2);
        assert_eq!((stage.spawned(), stage.stopped()), (0, 0));
        assert!(stage.errors().is_empty());
    }

    #[gpui::test]
    fn a_runtime_that_cannot_start_is_reported(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "[settings]\n");
        stage.host.refuse.store(true, Ordering::SeqCst);
        stage.start(cx, &[KEY]);
        assert_eq!(stage.errors(), ["Liberator loading failed"]);
        stage.tick(cx, &[KEY]);
        assert_eq!(stage.attempts(), 1);
        stage.pass(cx, ATTACH_DELAY);
        stage.tick(cx, &[KEY]);
        assert_eq!(stage.errors(), ["Liberator loading failed"; 2]);
        stage.host.refuse.store(false, Ordering::SeqCst);
        if !IS_WINDOWS {
            stage.host.protons.lock().unwrap().clear();
            stage.pass(cx, 2 * ATTACH_DELAY);
            stage.tick(cx, &[KEY]);
            assert_eq!(stage.errors()[2], "No Proton found");
            *stage.host.protons.lock().unwrap() = vec![proton("ge")];
        }
        stage.pass(cx, ATTACH_DELAY_MAX);
        stage.tick(cx, &[KEY]);
        assert_eq!((stage.spawned(), stage.stopped()), (1, 0));
        stage.attach(cx);
    }

    #[gpui::test]
    fn an_older_file_counts_as_available_until_its_replacement_fails(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "[settings]\n");
        install(&stage.paths);
        let stamp = stage.paths.liberator_bin().with_extension("version");
        fs::write(stamp, "0.0.1").unwrap();
        assert!(stage.read(cx, Liberator::available));
        *stage.host.fetch_error.lock().unwrap() = Some("Liberator download failed".to_owned());
        stage.start(cx, &[KEY]);
        assert_eq!(*stage.host.fetches.lock().unwrap(), [false]);
        assert!(!stage.read(cx, Liberator::available));
    }

    #[gpui::test]
    fn a_failed_download_is_reported_once_per_session(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "[settings]\n");
        let failure = "Liberator download failed";
        *stage.host.fetch_error.lock().unwrap() = Some(failure.to_owned());
        assert!(stage.read(cx, Liberator::available));
        stage.start(cx, &[KEY]);
        assert_eq!(
            (stage.attempts(), stage.errors()),
            (1, vec![failure.to_owned()])
        );
        assert!(!stage.read(cx, Liberator::available));
        stage.pass(cx, ATTACH_DELAY_MAX);
        stage.tick(cx, &[KEY]);
        assert_eq!(stage.attempts(), 1);
        stage.tick(cx, &[]);
        assert!(!stage.read(cx, Liberator::available));
        stage.tick(cx, &[KEY]);
        assert_eq!((stage.attempts(), stage.errors().len()), (2, 2));

        *stage.host.fetch_error.lock().unwrap() = None;
        stage
            .launcher
            .update(cx, |launcher, cx| launcher.liberator.restore(cx));
        cx.run_until_parked();
        assert!(stage.read(cx, Liberator::available));
        stage.pass(cx, ATTACH_DELAY_MAX);
        stage.tick(cx, &[KEY]);
        assert_eq!(
            *stage.host.fetches.lock().unwrap(),
            [false, false, true, false]
        );
        assert_eq!(stage.spawned(), 1);
        assert_eq!(stage.errors(), [failure, failure, "restored"]);
    }

    #[gpui::test]
    fn a_runtime_that_exits_before_connecting_is_dropped(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "[settings]\n");
        stage.start(cx, &[KEY]);
        stage.pass(cx, Duration::from_secs(60));
        assert_eq!((stage.spawned(), stage.stopped()), (1, 0));
        *stage.host.exit.lock().unwrap() = Some("exit=1".to_owned());
        stage.pass(cx, ACCEPT_POLL);
        assert_eq!(stage.stopped(), 1);
        *stage.host.exit.lock().unwrap() = None;
        stage.pass(cx, ATTACH_DELAY - Duration::from_millis(1));
        stage.tick(cx, &[KEY]);
        assert_eq!(stage.spawned(), 1);
        stage.pass(cx, Duration::from_millis(1));
        stage.tick(cx, &[KEY]);
        assert_eq!(stage.spawned(), 2);
        stage.attach(cx);
        assert!(stage.errors().is_empty());
    }

    #[gpui::test]
    fn a_lost_connection_is_cleared_and_retried(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "[settings]\n");
        stage.host.refuse.store(true, Ordering::SeqCst);
        stage.start(cx, &[KEY]);
        stage.host.refuse.store(false, Ordering::SeqCst);
        stage.pass(cx, ATTACH_DELAY);
        stage.tick(cx, &[KEY]);
        let mut peer = stage.attach(cx);
        peer.say(TREE);
        stage.settle(cx, |liberator| liberator.tree().is_some());
        drop(peer);
        stage.settle(cx, |liberator| !liberator.state().attached);
        assert!(stage.read(cx, |liberator| liberator.tree().is_none()));
        assert_eq!((stage.spawned(), stage.stopped()), (1, 1));
        stage.pass(cx, ATTACH_DELAY - Duration::from_millis(1));
        stage.tick(cx, &[KEY]);
        assert_eq!(stage.spawned(), 1);
        stage.pass(cx, Duration::from_millis(1));
        stage.tick(cx, &[KEY]);
        assert_eq!(stage.spawned(), 2);
        stage.attach(cx);
        assert_eq!(stage.errors(), ["Liberator loading failed"]);
    }

    #[gpui::test]
    fn stop_ends_the_runtime_and_undoes_a_start_that_was_under_way(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "[settings]\n");
        stage.start(cx, &[KEY]);
        let mut peer = stage.attach(cx);
        stage.stop(cx);
        assert!(peer.closed());
        assert_eq!(stage.stopped(), 1);
        assert!(stage.read(cx, |liberator| !liberator.state().attached));
        stage.tick(cx, &[KEY]);
        assert_eq!(stage.spawned(), 1);

        stage.show(cx, &[KEY]);
        stage.launcher.update(cx, |launcher, cx| {
            launcher.start_liberator(cx);
            launcher.liberator.stop(cx);
        });
        cx.run_until_parked();
        assert_eq!((stage.spawned(), stage.stopped()), (2, 2));
        stage.pass(cx, ATTACH_DELAY_MAX);
        stage.tick(cx, &[KEY]);
        assert_eq!(stage.spawned(), 2);
        stage.stop(cx);
        assert_eq!(stage.stopped(), 2);
        assert!(stage.errors().is_empty());
    }

    #[gpui::test]
    fn the_setting_starts_and_stops_the_liberator(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "[settings]\n");
        stage.show(cx, &[KEY]);
        let set = |cx: &mut TestAppContext, enabled: bool| {
            stage.launcher.update(cx, |launcher, cx| {
                launcher.set_liberator_enabled(enabled, cx);
            });
            cx.run_until_parked();
        };
        let stored = || fs::read_to_string(stage.paths.settings_file()).unwrap();
        set(cx, true);
        assert_eq!((stage.spawned(), stored().as_str()), (0, "[settings]\n"));
        set(cx, false);
        assert_eq!(stored(), "[settings]\nliberator_enabled = false\n");
        stage.tick(cx, &[KEY]);
        assert_eq!(stage.spawned(), 0);
        set(cx, true);
        assert_eq!((stage.spawned(), stored().as_str()), (1, "[settings]\n"));
        let mut peer = stage.attach(cx);
        set(cx, true);
        assert_eq!(stage.stopped(), 0);
        set(cx, false);
        assert!(peer.closed());
        assert_eq!((stage.spawned(), stage.stopped()), (1, 1));
    }

    #[gpui::test]
    fn the_start_fetches_quietly_or_asks_for_an_exclusion_on_windows(cx: &mut TestAppContext) {
        let prepare = |stage: &Stage, windows: bool, cx: &mut TestAppContext| {
            let launcher = &stage.launcher;
            launcher.update(cx, |launcher, cx| launcher.prepare_liberator(windows, cx));
            cx.run_until_parked();
            (stage.host.fetches.lock().unwrap().len(), stage.errors())
        };
        let stage = Stage::new(cx, "[settings]\n");
        assert_eq!(prepare(&stage, true, cx), (0, vec!["exclusion".to_owned()]));

        let silent = Stage::new(cx, "[settings]\n");
        *silent.host.fetch_error.lock().unwrap() = Some("offline".to_owned());
        assert_eq!(prepare(&silent, false, cx), (1, Vec::new()));
        assert!(silent.read(cx, |liberator| !liberator.busy() && liberator.available()));

        *silent.host.fetch_error.lock().unwrap() = None;
        assert_eq!(prepare(&silent, false, cx).0, 2);
        assert_eq!(prepare(&silent, true, cx).0, 2);

        let stamp = silent.paths.liberator_bin().with_extension("version");
        fs::write(stamp, "0.0.1").unwrap();
        assert_eq!(prepare(&silent, true, cx), (3, Vec::new()));

        let off = Stage::new(cx, "[settings]\nliberator_enabled = false\n");
        assert_eq!(prepare(&off, true, cx), (0, Vec::new()));
        assert_eq!(prepare(&off, false, cx), (0, Vec::new()));
    }

    #[gpui::test]
    fn restore_fetches_the_file_again_and_reports_a_failure(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "[settings]\n");
        let failure = "GitHub rate limit reached, try again in 3 minutes";
        *stage.host.fetch_error.lock().unwrap() = Some(failure.to_owned());
        stage.show(cx, &[KEY]);
        stage.launcher.update(cx, |launcher, cx| {
            launcher.liberator.restore(cx);
            launcher.liberator.restore(cx);
            assert!(launcher.liberator.busy());
            launcher.start_liberator(cx);
        });
        cx.run_until_parked();
        assert_eq!(*stage.host.fetches.lock().unwrap(), [true]);
        assert_eq!(stage.errors(), [failure]);
        assert!(stage.read(cx, |liberator| !liberator.busy() && !liberator.available()));

        *stage.host.fetch_error.lock().unwrap() = None;
        stage.launcher.update(cx, |launcher, cx| {
            launcher.tick_liberator(cx);
            launcher.liberator.restore(cx);
            assert!(launcher.liberator.busy());
        });
        cx.run_until_parked();
        assert_eq!(*stage.host.fetches.lock().unwrap(), [true, false]);
        assert!(stage.read(cx, |liberator| !liberator.busy() && liberator.available()));
        stage.stop(cx);
        fs::remove_file(stage.paths.liberator_bin()).unwrap();
        assert!(!stage.read(cx, Liberator::available));
        stage
            .launcher
            .update(cx, |launcher, cx| launcher.liberator.restore(cx));
        cx.run_until_parked();
        assert_eq!(*stage.host.fetches.lock().unwrap(), [true, false, true]);
        assert!(stage.read(cx, Liberator::available));
        assert_eq!(stage.errors(), [failure, "restored"]);
    }
}
