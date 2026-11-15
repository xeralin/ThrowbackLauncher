use std::collections::VecDeque;
use std::ffi::OsString;
use std::fmt::Display;
use std::fs;
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use backend::depot::{
    self, DepotCommand, Fault, LOGIN_FAILED, STAGING_HEADROOM, depot_commands,
    ensure_depotdownloader,
};
use backend::env::{
    self, BUSY_MESSAGE, CACHE_CLEARING, DOWNLOAD_RUNNING, FOLDER_NOT_FOUND, IS_WINDOWS,
    NOT_INSTALLED, NOT_INSTALLED_HM, PARTIAL_EXISTS, Paths, REMOVING_FILES, SEASON_RUNNING,
    TL_LAUNCHER, UNINSTALL_RUNNING, UPDATE_RUNNING,
};
use backend::heatedmetal::{HM_UNSTABLE, apply_hm, cache_hm_archive, remove_hm_files};
use backend::log;
use backend::manifest::{Download, edition_folder, installed_path, is_installed, partial_path};
use backend::settings::{QueueEntry, clean_username};
use backend::shears::folder_size;
use backend::steam::is_season_running;
use backend::throwbackloader::{apply_tl, ensure_tl, write_launcher};
use backend::winspawn::hidden;
use gpui::{AppContext, Context, EventEmitter, SharedString, Task};

pub use backend::depot::LoginKind;

use super::dialogs::{Picked, pick_file};
use super::settings::Settings;
use super::{Edition, Launcher};

const HISTORY_LINES: usize = 1000;
const CHUNK: usize = 8192;
const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Idle,
    Preparing,
    Downloading,
    Applying,
    Paused,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Cancelled,
    Failed,
    NoSpace,
    Done,
    Verified,
    Updated,
    SwitchedHm,
    SwitchedTb,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DownloadEvent {
    Done {
        key: String,
        outcome: Outcome,
    },
    Error(SharedString),
    LoginRequired(Option<LoginKind>),
    DiskSpaceRequired(f64),
    PartialDeleted {
        key: String,
        error: Option<SharedString>,
    },
    RateLimited(SharedString),
    HmArchivePicked(bool),
    FilesMissing {
        key: String,
        library: PathBuf,
    },
}

impl EventEmitter<DownloadEvent> for Launcher {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Exit {
    Code(i32),
    Crashed,
}

enum Message {
    Output(Vec<u8>),
    Exited(Exit),
}

trait Process: Send {
    fn write(&mut self, line: &str);
    fn kill(&mut self);
    fn wait(&mut self, limit: Duration);
}

type Launched = (Box<dyn Process>, Receiver<Message>);

trait Host: Send + Sync {
    fn fetch(&self, paths: &Paths, loader: bool) -> Result<(), Fault>;
    fn launch(&self, bin: &Path, args: &[OsString]) -> io::Result<Launched>;
    fn apply_hm(
        &self,
        paths: &Paths,
        target: &Path,
        username: &str,
        release: &str,
        archive: Option<&Path>,
    ) -> Result<(), Fault>;
    fn cache_archive(&self, paths: &Paths, archive: &Path) -> Result<PathBuf, String>;
    fn season_running(&self, libraries: &[PathBuf], key: &str) -> bool;
    fn free_space(&self, folder: &Path) -> io::Result<u64>;
}

struct System;

struct Tool {
    child: Arc<Mutex<Child>>,
    input: Option<ChildStdin>,
    ended: mpsc::Receiver<()>,
}

impl Process for Tool {
    fn write(&mut self, line: &str) {
        if let Some(input) = &mut self.input {
            let _ = input.write_all(format!("{line}\n").as_bytes());
        }
    }

    fn kill(&mut self) {
        if let Ok(mut child) = self.child.try_lock() {
            let _ = child.kill();
        }
    }

    fn wait(&mut self, limit: Duration) {
        let _ = self.ended.recv_timeout(limit);
    }
}

fn pump(mut output: impl Read, messages: &Sender<Message>) {
    let mut chunk = [0; CHUNK];
    loop {
        match output.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => {
                let read = Message::Output(chunk[..count].to_vec());
                if messages.send_blocking(read).is_err() {
                    break;
                }
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
}

fn crashed(code: i32) -> bool {
    (0x8000_0000..0xD000_0000).contains(&code.cast_unsigned())
}

fn exit(status: io::Result<ExitStatus>) -> Exit {
    match status.ok().and_then(|status| status.code()) {
        Some(code) if !(IS_WINDOWS && crashed(code)) => Exit::Code(code),
        _ => Exit::Crashed,
    }
}

impl Host for System {
    fn fetch(&self, paths: &Paths, loader: bool) -> Result<(), Fault> {
        ensure_depotdownloader(paths, None, false)?;
        if loader {
            ensure_tl(paths, None, false)?;
        }
        Ok(())
    }

    fn launch(&self, bin: &Path, args: &[OsString]) -> io::Result<Launched> {
        let (output, merged) = io::pipe()?;
        let mut command = Command::new(bin);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(merged.try_clone()?)
            .stderr(merged);
        let mut child = hidden(&mut command).spawn()?;
        drop(command);
        let input = child.stdin.take();
        let child = Arc::new(Mutex::new(child));
        let (sender, messages) = async_channel::unbounded();
        let (alive, ended) = mpsc::channel();
        let reaped = child.clone();
        thread::spawn(move || {
            let _alive = alive;
            pump(output, &sender);
            let status = reaped.lock().unwrap_or_else(PoisonError::into_inner).wait();
            let _ = sender.send_blocking(Message::Exited(exit(status)));
        });
        let tool = Tool {
            child,
            input,
            ended,
        };
        Ok((Box::new(tool), messages))
    }

    fn apply_hm(
        &self,
        paths: &Paths,
        target: &Path,
        username: &str,
        release: &str,
        archive: Option<&Path>,
    ) -> Result<(), Fault> {
        Ok(apply_hm(paths, target, username, release, None, archive)?)
    }

    fn cache_archive(&self, paths: &Paths, archive: &Path) -> Result<PathBuf, String> {
        cache_hm_archive(paths, archive).map_err(|error| error.to_string())
    }

    fn season_running(&self, libraries: &[PathBuf], key: &str) -> bool {
        is_season_running(libraries, key)
    }

    fn free_space(&self, folder: &Path) -> io::Result<u64> {
        depot::free_space(folder)
    }
}

#[derive(Clone)]
enum Call {
    Start {
        key: String,
        hm: bool,
        library: String,
        skip_disk_check: bool,
    },
    Verify {
        key: String,
        hm: bool,
    },
    RemoveHm {
        key: String,
    },
    SwitchToHm {
        key: String,
    },
    ReapplyHm {
        key: String,
        verify: bool,
    },
}

enum Request {
    Account(Call),
    Disk {
        key: String,
        hm: bool,
        library: String,
        shortfall: f64,
    },
}

struct Switch {
    from: PathBuf,
    to: PathBuf,
    strip_hm: bool,
}

impl Switch {
    fn run(&self, reports: &Sender<Report>) -> Result<(), Fault> {
        let failed = |detail: &dyn Display| Fault::Message(log::fail("Switch failed", detail));
        fs::rename(&self.from, &self.to).map_err(|error| failed(&error))?;
        let _ = reports.try_send(Report::Renamed {
            old: name_of(&self.from),
            new: name_of(&self.to),
        });
        match env::unlink(&self.to.join(TL_LAUNCHER)) {
            Err(error) if error.kind() != ErrorKind::NotFound => return Err(failed(&error)),
            _ => {}
        }
        if self.strip_hm {
            remove_hm_files(&self.to).map_err(|error| failed(&format_args!("{error:#}")))?;
        }
        Ok(())
    }
}

struct Install {
    target: PathBuf,
    hm: bool,
    release: Option<String>,
    username: String,
    archive: Option<PathBuf>,
    switch: Option<Switch>,
}

impl Install {
    fn run(&self, host: &dyn Host, paths: &Paths, reports: &Sender<Report>) -> Result<(), Fault> {
        if let Some(switch) = &self.switch {
            switch.run(reports)?;
        }
        if !self.hm {
            return apply_tl(paths, &self.target, &self.username)
                .and_then(|()| write_launcher(paths, &self.target))
                .map_err(|error| {
                    let detail = format!("{error:#}");
                    Fault::Message(log::fail("ThrowbackLoader setup failed", detail))
                });
        }
        let Some(release) = &self.release else {
            let message = log::fail("Install failed", "hm_version is missing");
            return Err(Fault::Message(message));
        };
        let archive = self.archive.as_deref();
        host.apply_hm(paths, &self.target, &self.username, release, archive)
    }
}

enum Report {
    Renamed {
        old: String,
        new: String,
    },
    Prepared(Result<(), Fault>),
    Applied(Result<(), Fault>),
    Deleted {
        key: String,
        hm: bool,
        error: Option<String>,
    },
    Cached {
        result: Result<PathBuf, String>,
        resume: Option<Call>,
    },
}

struct Running {
    process: Box<dyn Process>,
    _pump: Task<()>,
}

pub struct Downloader {
    host: Arc<dyn Host>,
    state: State,
    active: Option<Edition>,
    verifying: bool,
    queue: Vec<QueueEntry>,
    request: Option<Request>,
    login: Option<LoginKind>,
    login_pending: bool,
    password: String,
    progress: f64,
    step: usize,
    steps: usize,
    history: VecDeque<String>,
    season: Option<Download>,
    target: PathBuf,
    commands: Vec<DepotCommand>,
    index: usize,
    process: Option<Running>,
    buffer: Vec<u8>,
    dd_error: Option<&'static str>,
    cancelled: bool,
    rate_limited: bool,
    outcome: Option<Outcome>,
    deleting: Option<Edition>,
    picking: bool,
}

fn library_of(folder: &Path) -> &Path {
    folder.parent().unwrap_or(folder)
}

fn library_text(folder: &Path) -> String {
    library_of(folder).to_string_lossy().into_owned()
}

fn name_of(path: &Path) -> String {
    let name = path.file_name().unwrap_or_default();
    name.to_string_lossy().into_owned()
}

fn edition(key: &str, hm: bool) -> Edition {
    let key = key.to_owned();
    Edition { key, hm }
}

fn queued(key: &str, hm: bool, library: &str, verify: bool) -> QueueEntry {
    QueueEntry {
        key: key.to_owned(),
        hm,
        library: library.to_owned(),
        verify,
    }
}

fn valid_account(name: &str) -> bool {
    !name.is_empty() && clean_username(name) == name
}

fn resolve_root(
    libraries: &[PathBuf],
    key: &str,
    hm: bool,
    library: &str,
) -> (Option<PathBuf>, PathBuf) {
    match partial_path(libraries, key, hm) {
        Some(partial) => {
            let root = library_of(&partial).to_path_buf();
            (Some(partial), root)
        }
        None if library.is_empty() => (None, libraries.first().cloned().unwrap_or_default()),
        None => (None, env::resolve(Path::new(library))),
    }
}

fn queued_root(entry: &QueueEntry, libraries: &[PathBuf]) -> Option<PathBuf> {
    if entry.verify {
        let installed = installed_path(libraries, &entry.key, entry.hm)?;
        return Some(env::resolve(library_of(&installed)));
    }
    let (_, root) = resolve_root(libraries, &entry.key, entry.hm, &entry.library);
    Some(env::resolve(&root))
}

fn error(message: impl Into<SharedString>, cx: &mut Context<Launcher>) {
    cx.emit(DownloadEvent::Error(message.into()));
}

impl Default for Downloader {
    fn default() -> Self {
        Self {
            host: Arc::new(System),
            state: State::Idle,
            active: None,
            verifying: false,
            queue: Vec::new(),
            request: None,
            login: None,
            login_pending: false,
            password: String::new(),
            progress: 0.,
            step: 0,
            steps: 0,
            history: VecDeque::new(),
            season: None,
            target: PathBuf::new(),
            commands: Vec::new(),
            index: 0,
            process: None,
            buffer: Vec::new(),
            dd_error: None,
            cancelled: false,
            rate_limited: false,
            outcome: None,
            deleting: None,
            picking: false,
        }
    }
}

impl Downloader {
    pub fn new(settings: &mut Settings, downloads: &[Download]) -> Self {
        let libraries = settings.roots();
        let mut queue = settings.store.queue();
        queue.retain(|entry| {
            downloads.iter().any(|download| download.key == entry.key)
                && installed_path(libraries, &entry.key, entry.hm).is_some() == entry.verify
        });
        let mut downloader = Self {
            queue,
            ..Self::default()
        };
        if settings.store.queue_paused() {
            if downloader.enter_paused_from_queue(libraries) {
                downloader.state = State::Paused;
            } else {
                settings.store.set_queue_paused(false);
            }
        }
        downloader
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn running(&self) -> bool {
        matches!(
            self.state,
            State::Preparing | State::Downloading | State::Applying
        )
    }

    pub fn active(&self) -> Option<&Edition> {
        self.active.as_ref()
    }

    pub fn verifying(&self) -> bool {
        self.verifying
    }

    pub fn queue(&self) -> impl Iterator<Item = Edition> + '_ {
        let waiting = self.queue.iter();
        waiting.map(|entry| edition(&entry.key, entry.hm))
    }

    pub fn waits(&self, key: &str, hm: bool) -> bool {
        let wanted = |entry: &QueueEntry| entry.key == key && entry.hm == hm;
        self.queue.iter().any(wanted)
    }

    pub fn percent(&self) -> Option<f64> {
        let measured = self.step >= self.steps && self.progress > 0. && self.progress < 100.;
        measured.then_some(self.progress)
    }

    pub fn login_kind(&self) -> Option<LoginKind> {
        self.login
    }

    pub fn disk_shortfall(&self) -> Option<f64> {
        match self.request {
            Some(Request::Disk { shortfall, .. }) => Some(shortfall),
            _ => None,
        }
    }

    pub fn history(&self) -> &VecDeque<String> {
        &self.history
    }

    pub fn busy_with(&self, key: &str) -> bool {
        self.running() && self.active.as_ref().is_some_and(|active| active.key == key)
    }

    pub fn uses_library(&self, root: &Path, libraries: &[PathBuf]) -> bool {
        let root = env::resolve(root);
        let same = |folder: PathBuf| env::same_path(&folder, &root);
        let held = self.running() || self.state == State::Paused;
        (held && self.active.is_some() && same(env::resolve(library_of(&self.target))))
            || self
                .queue
                .iter()
                .any(|entry| queued_root(entry, libraries).is_some_and(same))
    }

    pub fn shutdown(&mut self) {
        if let Some(mut running) = self.process.take() {
            running.process.kill();
            running.process.wait(SHUTDOWN_WAIT);
        }
    }

    fn is_active(&self, key: &str, hm: bool) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.is(key, hm))
    }

    fn holds(&self, key: &str, hm: bool) -> bool {
        self.is_active(key, hm) || self.waits(key, hm)
    }

    fn enter_paused_from_queue(&mut self, libraries: &[PathBuf]) -> bool {
        let Some(index) = self.queue.iter().position(|entry| !entry.verify) else {
            return false;
        };
        let QueueEntry {
            key, hm, library, ..
        } = self.queue.remove(index);
        let (partial, root) = resolve_root(libraries, &key, hm, &library);
        self.target = partial.unwrap_or_else(|| root.join(edition_folder(&key, hm)));
        self.active = Some(Edition { key, hm });
        self.progress = 0.;
        true
    }

    fn tail(&self) -> String {
        depot::output_tail(self.history.iter().map(String::as_str))
    }

    fn out_of_space(&self) -> bool {
        let free = self.host.free_space(library_of(&self.target));
        free.is_ok_and(|free| free < STAGING_HEADROOM)
    }

    fn season_running(&self, libraries: &[PathBuf], key: &str) -> bool {
        self.host.season_running(libraries, key)
    }
}

#[cfg(test)]
impl Downloader {
    pub fn with(
        state: State,
        active: Option<Edition>,
        verifying: bool,
        queue: Vec<Edition>,
    ) -> Self {
        let waiting = queue.iter();
        Self {
            state,
            active,
            verifying,
            queue: waiting
                .map(|edition| queued(&edition.key, edition.hm, "", false))
                .collect(),
            ..Self::default()
        }
    }

    pub fn downloading(key: &str, hm: bool) -> Self {
        Self::with(
            State::Downloading,
            Some(edition(key, hm)),
            false,
            Vec::new(),
        )
    }

    pub fn loading(verifying: bool, typed: Arc<Mutex<Vec<String>>>) -> Self {
        struct Keys(Arc<Mutex<Vec<String>>>);

        impl Process for Keys {
            fn write(&mut self, line: &str) {
                self.0.lock().unwrap().push(line.to_owned());
            }

            fn kill(&mut self) {
                self.0.lock().unwrap().push("killed".to_owned());
            }

            fn wait(&mut self, _: Duration) {}
        }

        Self {
            state: State::Downloading,
            active: Some(edition("Y1S0_Vanilla", false)),
            verifying,
            process: Some(Running {
                process: Box::new(Keys(typed)),
                _pump: Task::ready(()),
            }),
            ..Self::default()
        }
    }

    pub fn asking(kind: LoginKind, typed: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            login: Some(kind),
            login_pending: true,
            ..Self::loading(false, typed)
        }
    }

    pub fn lacking(key: &str, shortfall: f64) -> Self {
        Self {
            request: Some(Request::Disk {
                key: key.to_owned(),
                hm: false,
                library: String::new(),
                shortfall,
            }),
            ..Self::default()
        }
    }
}

#[cfg(feature = "qa")]
impl Downloader {
    pub fn stage(&mut self, plan: &str) {
        let parsed = |text: &str| match text.strip_suffix("+hm") {
            Some(key) => edition(key, true),
            None => edition(text, false),
        };
        for (name, value) in plan.split(',').filter_map(|part| part.split_once('=')) {
            match name {
                "state" => {
                    self.state = match value {
                        "preparing" => State::Preparing,
                        "downloading" => State::Downloading,
                        "applying" => State::Applying,
                        "paused" => State::Paused,
                        "failed" => State::Failed,
                        _ => State::Idle,
                    }
                }
                "active" => self.active = Some(parsed(value)),
                "verifying" => self.verifying = value == "1",
                "progress" => self.progress = value.parse().unwrap_or_default(),
                "step" => self.step = value.parse().unwrap_or_default(),
                "steps" => self.steps = value.parse().unwrap_or_default(),
                "login" => {
                    self.login = match value {
                        "account" => Some(LoginKind::Account),
                        "password" => Some(LoginKind::Password),
                        "guard" => Some(LoginKind::Guard),
                        "email" => Some(LoginKind::GuardEmail),
                        "mobile" => Some(LoginKind::GuardMobile),
                        _ => None,
                    }
                }
                "disk" => {
                    self.request = Some(Request::Disk {
                        key: String::new(),
                        hm: false,
                        library: String::new(),
                        shortfall: value.parse().unwrap_or_default(),
                    })
                }
                "queue" => {
                    let waiting = value.split('|').map(parsed);
                    self.queue = waiting
                        .map(|edition| queued(&edition.key, edition.hm, "", false))
                        .collect();
                }
                "log" => self.history = value.split('|').map(str::to_owned).collect(),
                _ => {}
            }
        }
    }
}

impl Launcher {
    pub fn enqueue(&mut self, key: &str, hm: bool, library: &str, cx: &mut Context<Self>) {
        if !self.download.running() {
            return self.start_download(key, hm, library, false, cx);
        }
        if self.download.holds(key, hm) {
            return;
        }
        if installed_path(self.settings.roots(), key, hm).is_some() {
            return self.verify(key, hm, cx);
        }
        self.download.queue.push(queued(key, hm, library, false));
        self.queue_updated(cx);
    }

    pub fn dequeue(&mut self, key: &str, hm: bool, cx: &mut Context<Self>) {
        let queue = &mut self.download.queue;
        let before = queue.len();
        queue.retain(|entry| entry.key != key || entry.hm != hm);
        if queue.len() != before {
            self.queue_updated(cx);
        }
    }

    pub fn reorder_queue(&mut self, order: &[Edition], cx: &mut Context<Self>) {
        let mut rest = self.download.queue.clone();
        let mut sorted = Vec::with_capacity(rest.len());
        for edition in order {
            let wanted = |entry: &QueueEntry| edition.is(&entry.key, entry.hm);
            if let Some(index) = rest.iter().position(wanted) {
                sorted.push(rest.remove(index));
            }
        }
        sorted.append(&mut rest);
        if sorted != self.download.queue {
            self.download.queue = sorted;
            self.queue_updated(cx);
        }
    }

    pub fn set_paused(&mut self, paused: bool, cx: &mut Context<Self>) {
        if paused == self.settings.store.queue_paused() {
            return;
        }
        if paused {
            self.settings.store.set_queue_paused(true);
            if self.download.running() {
                if !self.download.verifying {
                    self.cancel_download(cx);
                }
            } else if self.download.state != State::Paused
                && self.download.enter_paused_from_queue(self.settings.roots())
            {
                self.set_download_state(State::Paused, cx);
                self.queue_updated(cx);
            }
        } else if self.download.state == State::Paused
            && let Some(active) = self.download.active.clone()
        {
            let library = library_text(&self.download.target);
            self.start_download(&active.key, active.hm, &library, false, cx);
        } else {
            self.settings.store.set_queue_paused(false);
            self.start_next(cx);
        }
    }

    pub fn verify(&mut self, key: &str, hm: bool, cx: &mut Context<Self>) {
        if partial_path(self.settings.roots(), key, hm).is_some() {
            return self.enqueue(key, hm, "", cx);
        }
        if self.download.running() {
            if self.download.holds(key, hm) {
                return;
            }
            if installed_path(self.settings.roots(), key, hm).is_none() {
                return error(NOT_INSTALLED, cx);
            }
            self.download.queue.push(queued(key, hm, "", true));
            return self.queue_updated(cx);
        }
        let call = Call::Verify {
            key: key.to_owned(),
            hm,
        };
        let request = Request::Account(call.clone());
        let Some(download) = self.accept_download(key, request, cx) else {
            return;
        };
        let Some(target) = installed_path(self.settings.roots(), key, hm) else {
            return error(NOT_INSTALLED, cx);
        };
        if self.download.season_running(self.settings.roots(), key) {
            return error(SEASON_RUNNING, cx);
        }
        if hm && !self.ensure_hm_archive(&download, call, cx) {
            return;
        }
        self.start_run(download, edition(key, hm), target, true, None, cx);
    }

    pub fn delete_partial(&mut self, key: &str, hm: bool, cx: &mut Context<Self>) {
        let download = &self.download;
        let active = download.is_active(key, hm);
        let refusal = if download.running() && active {
            Some(BUSY_MESSAGE)
        } else if download.deleting.is_some() {
            Some(REMOVING_FILES)
        } else if let Some(blocker) = self.download_blocker(key) {
            Some(blocker)
        } else if self.download.season_running(self.settings.roots(), key) {
            Some(SEASON_RUNNING)
        } else {
            None
        };
        if let Some(message) = refusal {
            return self.partial_deleted(key, Some(message.into()), cx);
        }
        let folder = edition_folder(key, hm);
        let folders = self.settings.roots().iter().map(|root| root.join(&folder));
        let targets: Vec<PathBuf> = folders
            .filter(|path| path.is_dir() && !is_installed(path))
            .collect();
        if targets.is_empty() {
            if active {
                return self.partial_removed(key, hm, None, cx);
            }
            return self.partial_deleted(key, Some("Nothing to remove".into()), cx);
        }
        self.download.deleting = Some(edition(key, hm));
        let key = key.to_owned();
        Self::download_job(cx, move |reports| {
            let removed = targets.iter().try_for_each(fs::remove_dir_all);
            let error = removed
                .err()
                .map(|error| log::fail("File removal failed", error));
            let _ = reports.try_send(Report::Deleted { key, hm, error });
        });
    }

    pub fn switch_to_hm(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.download_blocked(key, cx) {
            return;
        }
        let libraries = self.settings.roots();
        let Some(folder) = installed_path(libraries, key, false) else {
            return error("Not installed as Throwback", cx);
        };
        let Some(download) = self.heated_season(key) else {
            return error("Heated Metal is not available for this season", cx);
        };
        if installed_path(libraries, key, true).is_some() {
            return error("Already installed as Heated Metal", cx);
        }
        if partial_path(libraries, key, true).is_some() {
            return error(PARTIAL_EXISTS, cx);
        }
        if self.download.season_running(self.settings.roots(), key) {
            return error(SEASON_RUNNING, cx);
        }
        let resume = Call::SwitchToHm {
            key: key.to_owned(),
        };
        if !self.ensure_hm_archive(&download, resume, cx) {
            return;
        }
        self.dequeue(key, true, cx);
        let target = library_of(&folder).join(edition_folder(key, true));
        let heated = edition(key, true);
        self.begin_run(download, heated, target.clone(), false, State::Applying, cx);
        self.download.outcome = Some(Outcome::SwitchedHm);
        let switch = Switch {
            from: folder,
            to: target,
            strip_hm: false,
        };
        self.start_apply(Some(switch), cx);
    }

    pub fn remove_hm(&mut self, key: &str, cx: &mut Context<Self>) {
        let call = Call::RemoveHm {
            key: key.to_owned(),
        };
        let Some(download) = self.accept_download(key, Request::Account(call), cx) else {
            return;
        };
        let libraries = self.settings.roots();
        let Some(folder) = installed_path(libraries, key, true) else {
            return error(NOT_INSTALLED_HM, cx);
        };
        if installed_path(libraries, key, false).is_some() {
            return error("Already installed as Throwback", cx);
        }
        if partial_path(libraries, key, false).is_some() {
            return error(PARTIAL_EXISTS, cx);
        }
        if self.download.season_running(self.settings.roots(), key) {
            return error(SEASON_RUNNING, cx);
        }
        self.dequeue(key, false, cx);
        let target = library_of(&folder).join(edition_folder(key, false));
        let switch = Switch {
            from: folder,
            to: target.clone(),
            strip_hm: true,
        };
        self.start_run(
            download,
            edition(key, false),
            target,
            true,
            Some(switch),
            cx,
        );
        self.download.outcome = Some(Outcome::SwitchedTb);
    }

    pub fn restore_hm(&mut self, key: &str, cx: &mut Context<Self>) {
        self.reapply_hm(key, true, cx);
    }

    pub fn update_hm(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.download_blocked(key, cx) {
            return;
        }
        if installed_path(self.settings.roots(), key, true).is_none() {
            return error(NOT_INSTALLED_HM, cx);
        }
        let unstable = |download: &Download| {
            download.key == key && download.hm_version.as_deref() == Some(HM_UNSTABLE)
        };
        if !self.downloads.iter().any(unstable) {
            return;
        }
        if self.download.season_running(self.settings.roots(), key) {
            return error(SEASON_RUNNING, cx);
        }
        let resume = Call::ReapplyHm {
            key: key.to_owned(),
            verify: false,
        };
        self.select_hm_archive(Some(resume), cx);
    }

    pub fn hm_archive_cached(&self) -> bool {
        self.cached_hm_archive().is_some()
    }

    pub fn pick_hm_archive(&mut self, cx: &mut Context<Self>) {
        if !self.download.picking {
            self.select_hm_archive(None, cx);
        }
    }

    pub fn cancel_download(&mut self, cx: &mut Context<Self>) {
        let download = &mut self.download;
        if download.running() {
            download.cancelled = true;
            if let Some(running) = &mut download.process {
                running.process.kill();
            }
            return;
        }
        let request = download.request.take();
        download.login = None;
        download.password.clear();
        cx.notify();
        if let Some(Request::Disk { .. }) = request {
            self.persist_queue();
            Self::start_next_later(cx);
        }
    }

    pub fn submit_login(&mut self, text: &str, cx: &mut Context<Self>) {
        let download = &mut self.download;
        let Some(running) = &mut download.process else {
            return;
        };
        if !download.login_pending {
            return;
        }
        download.login_pending = false;
        download.login = None;
        running.process.write(text);
        cx.notify();
    }

    pub fn submit_account_login(&mut self, account: &str, password: &str, cx: &mut Context<Self>) {
        let account = account.trim();
        if !valid_account(account) {
            error("Invalid Steam account", cx);
            cx.emit(DownloadEvent::LoginRequired(Some(LoginKind::Account)));
            return;
        }
        self.download.login = None;
        cx.notify();
        self.settings.set_steam_account(account, cx);
        self.download.password = password.to_owned();
        if let Some(Request::Account(call)) = self.download.request.take() {
            self.resume_call(call, cx);
        }
    }

    pub fn confirm_disk_space(&mut self, cx: &mut Context<Self>) {
        match self.download.request.take() {
            Some(Request::Disk {
                key, hm, library, ..
            }) => {
                cx.notify();
                self.start_download(&key, hm, &library, true, cx);
            }
            other => self.download.request = other,
        }
    }

    pub fn start_next(&mut self, cx: &mut Context<Self>) {
        if self.settings.store.queue_paused() || !valid_account(&self.settings.steam_account()) {
            return;
        }
        loop {
            let download = &self.download;
            if download.running() || download.picking || download.request.is_some() {
                return;
            }
            let Some(next) = download.queue.first() else {
                return;
            };
            let libraries = self.settings.roots();
            let installed = || installed_path(libraries, &next.key, next.hm).is_some();
            if download
                .deleting
                .as_ref()
                .is_some_and(|gone| gone.key == next.key)
                || self.download_blocker(&next.key).is_some()
                || ((next.verify || installed()) && download.season_running(libraries, &next.key))
            {
                return;
            }
            let next = self.download.queue.remove(0);
            self.queue_updated(cx);
            if next.verify {
                self.verify(&next.key, next.hm, cx);
            } else {
                self.start_download(&next.key, next.hm, &next.library, false, cx);
            }
        }
    }

    pub(super) fn start_next_later(cx: &mut Context<Self>) {
        cx.spawn(async move |launcher, cx| {
            launcher
                .update(cx, |launcher, cx| launcher.start_next(cx))
                .ok();
        })
        .detach();
    }

    fn resume_call(&mut self, call: Call, cx: &mut Context<Self>) {
        match call {
            Call::Start {
                key,
                hm,
                library,
                skip_disk_check,
            } => self.start_download(&key, hm, &library, skip_disk_check, cx),
            Call::Verify { key, hm } => self.verify(&key, hm, cx),
            Call::RemoveHm { key } => self.remove_hm(&key, cx),
            Call::SwitchToHm { key } => self.switch_to_hm(&key, cx),
            Call::ReapplyHm { key, verify } => self.reapply_hm(&key, verify, cx),
        }
    }

    fn download_blocker(&self, key: &str) -> Option<&'static str> {
        if self.uninstall.busy_key() == Some(key) {
            Some(UNINSTALL_RUNNING)
        } else if self.shears.busy_key() == Some(key) {
            Some("Shears is running")
        } else if self.update.busy() {
            Some(UPDATE_RUNNING)
        } else if self.settings.clearing_cache() {
            Some(CACHE_CLEARING)
        } else {
            None
        }
    }

    fn download_blocked(&self, key: &str, cx: &mut Context<Self>) -> bool {
        let download = &self.download;
        if download.running() {
            error(DOWNLOAD_RUNNING, cx);
            return true;
        }
        if download.picking || download.request.is_some() {
            return true;
        }
        if download
            .deleting
            .as_ref()
            .is_some_and(|gone| gone.key == key)
        {
            error(REMOVING_FILES, cx);
            return true;
        }
        let blocker = self.download_blocker(key);
        if let Some(message) = blocker {
            error(message, cx);
        }
        blocker.is_some()
    }

    fn accept_download(
        &mut self,
        key: &str,
        request: Request,
        cx: &mut Context<Self>,
    ) -> Option<Download> {
        if self.download_blocked(key, cx) {
            return None;
        }
        let known = self.downloads.iter().find(|download| download.key == key);
        let Some(download) = known.cloned() else {
            error("Unknown season", cx);
            return None;
        };
        let account = self.settings.steam_account();
        if !valid_account(&account) {
            self.download.request = Some(request);
            self.download.login = Some(LoginKind::Account);
            cx.emit(DownloadEvent::LoginRequired(Some(LoginKind::Account)));
            cx.notify();
            return None;
        }
        Some(download)
    }

    fn heated_season(&self, key: &str) -> Option<Download> {
        let heated = |download: &&Download| download.key == key && download.hm_version.is_some();
        self.downloads.iter().find(heated).cloned()
    }

    fn cached_hm_archive(&self) -> Option<PathBuf> {
        let name = Path::new(self.settings.store.hm_archive()).file_name()?;
        let cached = self.paths.bin_dir().join(name);
        cached.is_file().then_some(cached)
    }

    fn ensure_hm_archive(
        &mut self,
        download: &Download,
        resume: Call,
        cx: &mut Context<Self>,
    ) -> bool {
        if download.hm_version.as_deref() != Some(HM_UNSTABLE) || self.cached_hm_archive().is_some()
        {
            return true;
        }
        self.select_hm_archive(Some(resume), cx);
        false
    }

    fn select_hm_archive(&mut self, resume: Option<Call>, cx: &mut Context<Self>) {
        self.download.picking = true;
        let picked = pick_file("Select Heated Metal", cx);
        cx.spawn(async move |launcher, cx| {
            let picked = picked.await;
            launcher
                .update(cx, |launcher, cx| {
                    launcher.hm_archive_selected(picked, resume, cx)
                })
                .ok();
        })
        .detach();
    }

    fn hm_archive_selected(
        &mut self,
        picked: Picked,
        resume: Option<Call>,
        cx: &mut Context<Self>,
    ) {
        let archive = picked.unwrap_or_else(|message| {
            error(message, cx);
            None
        });
        let Some(archive) = archive else {
            self.download.picking = false;
            cx.emit(DownloadEvent::HmArchivePicked(false));
            return self.start_next(cx);
        };
        let (host, paths) = (self.download.host.clone(), self.paths.clone());
        Self::download_job(cx, move |reports| {
            let result = host.cache_archive(&paths, &archive);
            let _ = reports.try_send(Report::Cached { result, resume });
        });
    }

    fn hm_archive_copied(
        &mut self,
        result: Result<PathBuf, String>,
        resume: Option<Call>,
        cx: &mut Context<Self>,
    ) {
        self.download.picking = false;
        let cached = result.is_ok();
        match result {
            Ok(archive) => {
                self.settings.store.set_hm_archive(&name_of(&archive));
                if let Some(call) = resume {
                    self.resume_call(call, cx);
                }
            }
            Err(message) => error(message, cx),
        }
        cx.emit(DownloadEvent::HmArchivePicked(cached));
        self.start_next(cx);
    }

    fn start_download(
        &mut self,
        key: &str,
        hm: bool,
        library: &str,
        skip_disk_check: bool,
        cx: &mut Context<Self>,
    ) {
        self.dequeue(key, hm, cx);
        let call = |skip_disk_check| Call::Start {
            key: key.to_owned(),
            hm,
            library: library.to_owned(),
            skip_disk_check,
        };
        let Some(download) = self.accept_download(key, Request::Account(call(false)), cx) else {
            return;
        };
        let libraries = self.settings.roots();
        let (partial, root) = resolve_root(libraries, key, hm, library);
        let target = match &partial {
            Some(partial) => {
                if !library.is_empty() && !env::same_path(&env::resolve(Path::new(library)), &root)
                {
                    return error(PARTIAL_EXISTS, cx);
                }
                partial.clone()
            }
            None => {
                if installed_path(libraries, key, hm).is_some() {
                    return self.verify(key, hm, cx);
                }
                if !self.settings.is_library(&root) {
                    return error("Unknown library", cx);
                }
                if !root.exists() {
                    return error(FOLDER_NOT_FOUND, cx);
                }
                root.join(edition_folder(key, hm))
            }
        };
        if hm && !self.ensure_hm_archive(&download, call(skip_disk_check), cx) {
            return;
        }
        if !skip_disk_check {
            let present = partial.map_or(0, |partial| folder_size(&partial));
            let free = self.download.host.free_space(&root);
            let short = free.map(|free| depot::disk_shortfall(download.size_gb, present, free));
            if let Ok(Some(shortfall)) = short {
                self.download.request = Some(Request::Disk {
                    key: key.to_owned(),
                    hm,
                    library: library.to_owned(),
                    shortfall,
                });
                self.persist_queue();
                cx.emit(DownloadEvent::DiskSpaceRequired(shortfall));
                cx.notify();
                return;
            }
        }
        self.start_run(download, edition(key, hm), target, false, None, cx);
    }

    fn reapply_hm(&mut self, key: &str, verify: bool, cx: &mut Context<Self>) {
        if self.download_blocked(key, cx) {
            return;
        }
        let Some(target) = installed_path(self.settings.roots(), key, true) else {
            return error(NOT_INSTALLED_HM, cx);
        };
        let Some(download) = self.heated_season(key) else {
            return error("Heated Metal is not available for this season", cx);
        };
        if self.download.season_running(self.settings.roots(), key) {
            return error(SEASON_RUNNING, cx);
        }
        let resume = Call::ReapplyHm {
            key: key.to_owned(),
            verify,
        };
        if !self.ensure_hm_archive(&download, resume, cx) {
            return;
        }
        self.begin_run(
            download,
            edition(key, true),
            target,
            verify,
            State::Applying,
            cx,
        );
        if !verify {
            self.download.outcome = Some(Outcome::Updated);
        }
        self.start_apply(None, cx);
    }

    fn release(&mut self, cx: &mut Context<Self>) {
        let paused = self.settings.store.queue_paused();
        if paused && self.download.enter_paused_from_queue(self.settings.roots()) {
            self.set_download_state(State::Paused, cx);
        } else {
            self.settings.store.set_queue_paused(false);
            self.download.active = None;
            self.set_download_state(State::Idle, cx);
        }
    }

    fn set_download_state(&mut self, state: State, cx: &mut Context<Self>) {
        if self.download.state != state {
            self.download.state = state;
            cx.notify();
            self.refresh_home(cx);
        }
    }

    fn queue_updated(&mut self, cx: &mut Context<Self>) {
        self.persist_queue();
        cx.notify();
    }

    fn persist_queue(&mut self) {
        let download = &self.download;
        let held = download.running() || download.state == State::Paused;
        let active = download.active.as_ref().filter(|_| held);
        let mut entries = Vec::with_capacity(download.queue.len() + 2);
        if let Some(active) = active {
            let library = library_text(&download.target);
            entries.push(queued(&active.key, active.hm, &library, download.verifying));
        }
        if let Some(Request::Disk {
            key, hm, library, ..
        }) = &download.request
            && !active.is_some_and(|active| active.is(key, *hm))
        {
            entries.push(queued(key, *hm, library, false));
        }
        entries.extend(download.queue.iter().cloned());
        self.settings.store.set_queue(&entries);
    }

    fn partial_deleted(&mut self, key: &str, error: Option<SharedString>, cx: &mut Context<Self>) {
        cx.emit(DownloadEvent::PartialDeleted {
            key: key.to_owned(),
            error,
        });
        self.refresh_disk_usage(cx);
        self.refresh_home(cx);
    }

    fn partial_removed(
        &mut self,
        key: &str,
        hm: bool,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let download = &mut self.download;
        download.deleting = None;
        if error.is_none() && !download.running() && download.is_active(key, hm) {
            self.release(cx);
            self.queue_updated(cx);
        }
        self.partial_deleted(key, error.map(SharedString::from), cx);
        self.start_next(cx);
    }

    fn download_job(cx: &mut Context<Self>, job: impl FnOnce(&Sender<Report>) + Send + 'static) {
        let (sender, reports) = async_channel::unbounded();
        cx.background_spawn(async move { job(&sender) }).detach();
        cx.spawn(async move |launcher, cx| {
            while let Ok(report) = reports.recv().await {
                let handled =
                    launcher.update(cx, |launcher, cx| launcher.download_report(report, cx));
                if handled.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn download_report(&mut self, report: Report, cx: &mut Context<Self>) {
        match report {
            Report::Renamed { old, new } => self.rename_home_entry(&old, &new, cx),
            Report::Prepared(result) => self.run_prepared(result, cx),
            Report::Applied(result) => self.run_applied(result, cx),
            Report::Deleted { key, hm, error } => self.partial_removed(&key, hm, error, cx),
            Report::Cached { result, resume } => self.hm_archive_copied(result, resume, cx),
        }
    }

    fn begin_run(
        &mut self,
        season: Download,
        edition: Edition,
        target: PathBuf,
        verify: bool,
        state: State,
        cx: &mut Context<Self>,
    ) {
        let download = &mut self.download;
        if download.state == State::Paused
            && let Some(active) = &download.active
            && *active != edition
            && download.deleting.as_ref() != Some(active)
        {
            let library = library_text(&download.target);
            let waiting = queued(&active.key, active.hm, &library, false);
            download.queue.insert(0, waiting);
        }
        self.settings.store.set_queue_paused(false);
        download.season = Some(season);
        download.outcome = None;
        download.verifying = verify;
        download.target = target;
        download.cancelled = false;
        download.rate_limited = false;
        download.dd_error = None;
        download.history.clear();
        download.active = Some(edition);
        download.progress = 0.;
        download.step = 0;
        download.steps = 0;
        cx.notify();
        self.set_download_state(state, cx);
        self.persist_queue();
    }

    fn start_run(
        &mut self,
        season: Download,
        edition: Edition,
        target: PathBuf,
        verify: bool,
        switch: Option<Switch>,
        cx: &mut Context<Self>,
    ) {
        let hm = edition.hm;
        self.begin_run(season, edition, target, verify, State::Preparing, cx);
        let (host, paths) = (self.download.host.clone(), self.paths.clone());
        Self::download_job(cx, move |reports| {
            let prepared = host.fetch(&paths, !hm).and_then(|()| match &switch {
                Some(switch) => switch.run(reports),
                None => Ok(()),
            });
            let _ = reports.try_send(Report::Prepared(prepared));
        });
    }

    fn run_prepared(&mut self, result: Result<(), Fault>, cx: &mut Context<Self>) {
        let cancelled = self.download.cancelled;
        if let Err(fault) = result {
            let limited = matches!(fault, Fault::RateLimited(_));
            if limited || !cancelled {
                self.run_fault(fault, cx);
            }
            if limited && !cancelled {
                self.download.cancelled = true;
                self.set_paused(true, cx);
            }
            return self.end_run(false, cx);
        }
        if cancelled {
            return self.end_run(false, cx);
        }
        let download = &mut self.download;
        let Some(season) = &download.season else {
            return;
        };
        let (account, limit) = (self.settings.steam_account(), self.settings.max_downloads());
        download.commands = depot_commands(season, &account, &download.target, limit).into();
        if let Err(cause) = fs::create_dir_all(&download.target) {
            error(log::fail("Setup failed", cause), cx);
            return self.end_run(false, cx);
        }
        download.index = 0;
        self.set_download_state(State::Downloading, cx);
        self.run_next_depot(cx);
    }

    fn run_next_depot(&mut self, cx: &mut Context<Self>) {
        let download = &mut self.download;
        let Some(command) = download.commands.get(download.index) else {
            download.progress = 100.;
            cx.notify();
            return self.start_apply(None, cx);
        };
        download.step = download.index + 1;
        download.steps = download.commands.len();
        download.progress = 0.;
        download.buffer.clear();
        download.login_pending = false;
        cx.notify();
        let bin = self.paths.dd_bin();
        match download.host.launch(&bin, &command.args) {
            Ok((process, messages)) => {
                let pump = cx.spawn(async move |launcher, cx| {
                    while let Ok(message) = messages.recv().await {
                        let handled = launcher.update(cx, |launcher, cx| {
                            launcher.depot_message(message, cx);
                        });
                        if handled.is_err() {
                            break;
                        }
                    }
                });
                download.process = Some(Running {
                    process,
                    _pump: pump,
                });
            }
            Err(cause) => {
                let detail = format!("{}: {cause}", bin.display());
                error(log::fail("DepotDownloader failed to start", detail), cx);
                self.end_run(false, cx);
            }
        }
    }

    fn depot_message(&mut self, message: Message, cx: &mut Context<Self>) {
        match message {
            Message::Output(bytes) => self.depot_output(&bytes, cx),
            Message::Exited(exit) => self.depot_exited(exit, cx),
        }
    }

    fn depot_output(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let download = &mut self.download;
        download.buffer.extend_from_slice(bytes);
        let lines = depot::take_lines(&mut download.buffer);
        for line in &lines {
            if let Some(percent) = depot::percent(line)
                && percent - download.progress >= 0.1
            {
                download.progress = percent;
            }
            let mobile = depot::mobile_confirmation(line);
            if mobile != (download.login == Some(LoginKind::GuardMobile)) {
                download.login = mobile.then_some(LoginKind::GuardMobile);
                cx.emit(DownloadEvent::LoginRequired(download.login));
            }
            if let Some(reason) = depot::output_error(line) {
                download.dd_error = Some(reason);
            }
        }
        if !lines.is_empty() {
            download.history.extend(lines);
            while download.history.len() > HISTORY_LINES {
                download.history.pop_front();
            }
            cx.notify();
        }
        if download.login_pending {
            return;
        }
        let Some(kind) = depot::prompt(&String::from_utf8_lossy(&download.buffer)) else {
            return;
        };
        if kind == LoginKind::Password && !download.password.is_empty() {
            let password = std::mem::take(&mut download.password);
            if let Some(running) = &mut download.process {
                running.process.write(&password);
            }
            return;
        }
        download.login_pending = true;
        download.login = Some(kind);
        cx.emit(DownloadEvent::LoginRequired(Some(kind)));
        cx.notify();
    }

    fn depot_exited(&mut self, exit: Exit, cx: &mut Context<Self>) {
        let download = &mut self.download;
        download.process = None;
        if download.cancelled {
            return self.end_run(false, cx);
        }
        if let Some(reason) = download.dd_error {
            let tail = download.tail();
            if reason == LOGIN_FAILED {
                self.settings.set_steam_account("", cx);
            }
            error(log::fail(reason, tail), cx);
            return self.end_run(false, cx);
        }
        if exit == Exit::Code(0) {
            download.index += 1;
            return self.run_next_depot(cx);
        }
        let depot = download.commands.get(download.index);
        let name = depot.map_or("", |command| command.name);
        let reason = match exit {
            Exit::Crashed => "crashed".to_owned(),
            Exit::Code(code) => format!("exit code {code}"),
        };
        let detail = format!("{name} depot {reason}\n{}", download.tail());
        let message = log::fail("Download failed", detail);
        if !download.out_of_space() {
            error(message, cx);
        }
        self.end_run(false, cx);
    }

    fn start_apply(&mut self, switch: Option<Switch>, cx: &mut Context<Self>) {
        self.download.step = 0;
        self.download.steps = 0;
        cx.notify();
        self.set_download_state(State::Applying, cx);
        let download = &self.download;
        let season = download.season.as_ref();
        let install = Install {
            target: download.target.clone(),
            hm: download.active.as_ref().is_some_and(|active| active.hm),
            release: season.and_then(|season| season.hm_version.clone()),
            username: self.settings.username().to_string(),
            archive: self.cached_hm_archive(),
            switch,
        };
        let (host, paths) = (download.host.clone(), self.paths.clone());
        Self::download_job(cx, move |reports| {
            let applied = install.run(&*host, &paths, reports);
            let _ = reports.try_send(Report::Applied(applied));
        });
    }

    fn run_applied(&mut self, result: Result<(), Fault>, cx: &mut Context<Self>) {
        match result {
            Ok(()) => {
                self.download.cancelled = false;
                self.end_run(true, cx);
            }
            Err(fault) => {
                self.run_fault(fault, cx);
                self.end_run(false, cx);
            }
        }
    }

    fn run_fault(&mut self, fault: Fault, cx: &mut Context<Self>) {
        let download = &mut self.download;
        match fault {
            Fault::RateLimited(message) => {
                download.rate_limited = true;
                cx.emit(DownloadEvent::RateLimited(message.into()));
            }
            Fault::FilesMissing => {
                if let Some(active) = &download.active {
                    cx.emit(DownloadEvent::FilesMissing {
                        key: active.key.clone(),
                        library: library_of(&download.target).to_path_buf(),
                    });
                }
            }
            Fault::Message(message) => error(message, cx),
        }
    }

    fn end_run(&mut self, success: bool, cx: &mut Context<Self>) {
        let download = &mut self.download;
        download.login = None;
        download.password.clear();
        let Some(done) = download.active.clone() else {
            return;
        };
        let verifying = std::mem::take(&mut download.verifying);
        cx.notify();
        let outcome = if download.cancelled {
            if self.settings.store.queue_paused() && !verifying {
                self.set_download_state(State::Paused, cx);
            } else {
                self.release(cx);
            }
            Outcome::Cancelled
        } else if !success {
            let full =
                !download.rate_limited && download.dd_error.is_none() && download.out_of_space();
            self.set_download_state(State::Failed, cx);
            if full {
                Outcome::NoSpace
            } else {
                Outcome::Failed
            }
        } else {
            let plain = if verifying {
                Outcome::Verified
            } else {
                Outcome::Done
            };
            let outcome = download.outcome.take().unwrap_or(plain);
            self.release(cx);
            outcome
        };
        self.persist_queue();
        cx.emit(DownloadEvent::Done {
            key: done.key,
            outcome,
        });
        self.refresh_disk_usage(cx);
        if !self.download.queue.is_empty() {
            Self::start_next_later(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use backend::env::{GIB, TL_DLLS_COMMON, TL_LOADERS, TL_TOML};
    use backend::manifest;
    use backend::settings::Settings as Store;
    use gpui::{Entity, TestAppContext};

    use super::*;
    use crate::model::shears::Shears;
    use crate::model::uninstall::Uninstaller;
    use crate::model::update::Updater;

    const VANILLA: &str = "Y1S0_Vanilla";
    const ICE: &str = "Y1S1_BlackIce";
    const SHADOW: &str = "Y5S3_ShadowLegacy";
    const SHADOW_HM: &str = "Y5S3_HeatedMetal";
    const BLOOD: &str = "Y9S2_NewBlood";
    const BLOOD_HM: &str = "Y9S2_HeatedMetal";
    const ACCOUNT: &str = "steam_account = 'ash'\n";
    const LIMIT: &str = "GitHub rate limit reached, try again in 2 minutes";

    #[derive(Default)]
    struct Fake {
        running: Mutex<Vec<String>>,
        free: Mutex<Option<u64>>,
        fetch: Mutex<Option<Fault>>,
        hm: Mutex<Option<Fault>>,
        archive: Mutex<Option<String>>,
        missing: AtomicBool,
        runs: Mutex<Vec<Run>>,
        calls: Mutex<Vec<String>>,
    }

    struct Run {
        args: Vec<String>,
        output: Sender<Message>,
        input: Arc<Mutex<Vec<String>>>,
        killed: Arc<AtomicBool>,
    }

    struct Puppet {
        output: Sender<Message>,
        input: Arc<Mutex<Vec<String>>>,
        killed: Arc<AtomicBool>,
    }

    impl Process for Puppet {
        fn write(&mut self, line: &str) {
            self.input.lock().unwrap().push(line.to_owned());
        }

        fn kill(&mut self) {
            self.killed.store(true, Ordering::SeqCst);
            let _ = self.output.try_send(Message::Exited(Exit::Crashed));
        }

        fn wait(&mut self, limit: Duration) {
            let waited = format!("waited {}", limit.as_secs());
            self.input.lock().unwrap().push(waited);
        }
    }

    impl Host for Fake {
        fn fetch(&self, _paths: &Paths, loader: bool) -> Result<(), Fault> {
            let call = format!("fetch loader={loader}");
            self.calls.lock().unwrap().push(call);
            self.fetch.lock().unwrap().take().map_or(Ok(()), Err)
        }

        fn launch(&self, bin: &Path, args: &[OsString]) -> io::Result<Launched> {
            if self.missing.load(Ordering::SeqCst) {
                return Err(io::Error::new(ErrorKind::NotFound, "no such file"));
            }
            let (output, messages) = async_channel::unbounded();
            let input = Arc::new(Mutex::new(Vec::new()));
            let killed = Arc::new(AtomicBool::new(false));
            let args = args.iter().map(|arg| arg.to_string_lossy().into_owned());
            self.runs.lock().unwrap().push(Run {
                args: std::iter::once(name_of(bin)).chain(args).collect(),
                output: output.clone(),
                input: input.clone(),
                killed: killed.clone(),
            });
            let puppet = Puppet {
                output,
                input,
                killed,
            };
            Ok((Box::new(puppet), messages))
        }

        fn apply_hm(
            &self,
            _paths: &Paths,
            target: &Path,
            username: &str,
            release: &str,
            archive: Option<&Path>,
        ) -> Result<(), Fault> {
            let archive = archive.map(name_of).unwrap_or_default();
            let folder = name_of(target);
            let call = format!("apply_hm {folder} {username} {release} {archive}");
            self.calls.lock().unwrap().push(call);
            if let Some(fault) = self.hm.lock().unwrap().take() {
                return Err(fault);
            }
            fs::write(target.join(TL_LAUNCHER), "").unwrap();
            Ok(())
        }

        fn cache_archive(&self, paths: &Paths, archive: &Path) -> Result<PathBuf, String> {
            if let Some(message) = self.archive.lock().unwrap().take() {
                return Err(message);
            }
            let cached = paths.bin_dir().join(archive.file_name().unwrap());
            fs::write(&cached, "").unwrap();
            Ok(cached)
        }

        fn season_running(&self, _libraries: &[PathBuf], key: &str) -> bool {
            self.running
                .lock()
                .unwrap()
                .iter()
                .any(|season| season == key)
        }

        fn free_space(&self, _folder: &Path) -> io::Result<u64> {
            Ok(self.free.lock().unwrap().unwrap_or(u64::MAX))
        }
    }

    struct Rig {
        launcher: Entity<Launcher>,
        host: Arc<Fake>,
        events: Rc<RefCell<Vec<DownloadEvent>>>,
        paths: Paths,
        lib: PathBuf,
        other: PathBuf,
        _dir: tempfile::TempDir,
    }

    fn install(library: &Path, folder: &str) {
        fs::create_dir_all(library.join(folder)).unwrap();
        fs::write(library.join(folder).join(TL_LAUNCHER), "").unwrap();
    }

    fn partial(library: &Path, folder: &str) {
        fs::create_dir_all(library.join(folder)).unwrap();
        fs::write(library.join(folder).join("data.forge"), "part").unwrap();
    }

    fn rig(cx: &mut TestAppContext, settings: &str, prepare: impl FnOnce(&Path, &Path)) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let root = env::resolve(dir.path());
        let paths = Paths::new(root.join("asset"), root.join("data"));
        let (lib, other) = (root.join("lib"), root.join("other"));
        let loader = paths.tl_dir();
        for folder in [&lib, &other, &loader] {
            fs::create_dir_all(folder).unwrap();
        }
        let names = TL_DLLS_COMMON.into_iter().chain(TL_LOADERS);
        for name in names.chain([TL_LAUNCHER]) {
            fs::write(loader.join(name), name).unwrap();
        }
        fs::write(loader.join(TL_TOML), "[User]\nusername = 'x'\n").unwrap();
        prepare(&lib, &other);
        let libraries = format!("['{}', '{}']", lib.display(), other.display());
        let text = format!("[settings]\nlibraries = {libraries}\n{settings}");
        fs::write(paths.settings_file(), text).unwrap();
        let store = Store::load(&paths).unwrap();
        let host = Arc::new(Fake::default());
        let launcher = cx.new(|_| {
            let mut launcher = Launcher::new(paths.clone(), store, manifest::downloads());
            launcher.download.host = host.clone();
            launcher
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = events.clone();
            let record = move |_, event: &DownloadEvent, _: &mut gpui::App| {
                events.borrow_mut().push(event.clone());
            };
            cx.subscribe(&launcher, record).detach();
        });
        Rig {
            launcher,
            host,
            events,
            paths,
            lib,
            other,
            _dir: dir,
        }
    }

    impl Rig {
        fn act(
            &self,
            cx: &mut TestAppContext,
            action: impl FnOnce(&mut Launcher, &mut Context<Launcher>),
        ) {
            self.launcher.update(cx, action);
            cx.run_until_parked();
        }

        fn seen(&self) -> Vec<DownloadEvent> {
            self.events.take()
        }

        fn state(&self, cx: &mut TestAppContext) -> State {
            self.launcher.read_with(cx, |it, _| it.download.state())
        }

        fn active(&self, cx: &mut TestAppContext) -> Option<Edition> {
            self.launcher
                .read_with(cx, |it, _| it.download.active().cloned())
        }

        fn queued(&self, cx: &mut TestAppContext) -> Vec<Edition> {
            self.launcher
                .read_with(cx, |it, _| it.download.queue().collect())
        }

        fn store(&self) -> Store {
            Store::load(&self.paths).unwrap()
        }

        fn stored(&self) -> Vec<String> {
            let entry = |entry: QueueEntry| {
                let edition = if entry.hm { "+hm" } else { "" };
                let kind = if entry.verify { "verify" } else { "download" };
                let library = name_of(Path::new(&entry.library));
                format!("{}{edition} @{library} {kind}", entry.key)
            };
            self.store().queue().into_iter().map(entry).collect()
        }

        fn runs(&self) -> usize {
            self.host.runs.lock().unwrap().len()
        }

        fn args(&self, run: usize) -> Vec<String> {
            self.host.runs.lock().unwrap()[run].args.clone()
        }

        fn calls(&self) -> Vec<String> {
            std::mem::take(&mut self.host.calls.lock().unwrap())
        }

        fn send(&self, cx: &mut TestAppContext, message: Message) {
            let runs = self.host.runs.lock().unwrap();
            runs.last().unwrap().output.try_send(message).unwrap();
            drop(runs);
            cx.run_until_parked();
        }

        fn feed(&self, cx: &mut TestAppContext, text: &str) {
            self.send(cx, Message::Output(text.as_bytes().to_vec()));
        }

        fn close(&self, cx: &mut TestAppContext, exit: Exit) {
            self.send(cx, Message::Exited(exit));
        }

        fn typed(&self) -> Vec<String> {
            let runs = self.host.runs.lock().unwrap();
            runs.last().unwrap().input.lock().unwrap().clone()
        }

        fn killed(&self) -> bool {
            let runs = self.host.runs.lock().unwrap();
            runs.last().unwrap().killed.load(Ordering::SeqCst)
        }

        fn enqueue(&self, cx: &mut TestAppContext, key: &str, hm: bool, library: &str) {
            self.act(cx, |it, cx| it.enqueue(key, hm, library, cx));
        }

        fn download(&self, cx: &mut TestAppContext, key: &str) {
            self.enqueue(cx, key, false, "");
            assert_eq!(self.state(cx), State::Downloading);
            assert_eq!(self.active(cx), Some(edition(key, false)));
        }
    }

    fn failed(message: &str) -> DownloadEvent {
        DownloadEvent::Error(message.to_owned().into())
    }

    fn done(key: &str, outcome: Outcome) -> DownloadEvent {
        let key = key.to_owned();
        DownloadEvent::Done { key, outcome }
    }

    fn login(kind: LoginKind) -> DownloadEvent {
        DownloadEvent::LoginRequired(Some(kind))
    }

    fn refused(key: &str, message: &str) -> DownloadEvent {
        DownloadEvent::PartialDeleted {
            key: key.to_owned(),
            error: Some(message.to_owned().into()),
        }
    }

    fn queue(entries: &[(&str, bool, &str, bool)]) -> String {
        let entry = |(key, hm, library, verify): &(&str, bool, &str, bool)| {
            format!("{{ key = '{key}', hm = {hm}, library = '{library}', verify = {verify} }}")
        };
        let entries: Vec<String> = entries.iter().map(entry).collect();
        format!("queue = [{}]\n", entries.join(", "))
    }

    fn pick(cx: &mut TestAppContext, picked: Option<&Path>) {
        cx.simulate_path_prompt_response(|options| {
            assert!(options.files && !options.directories && !options.multiple);
            assert_eq!(options.prompt.as_deref(), Some("Select Heated Metal"));
            picked.map(|path| vec![path.to_path_buf()])
        });
        cx.run_until_parked();
    }

    #[test]
    fn a_percentage_exists_only_inside_the_last_step() {
        let percent = |progress: f64, step: usize, steps: usize| {
            let mut download = Downloader::downloading(VANILLA, false);
            (download.progress, download.step, download.steps) = (progress, step, steps);
            download.percent()
        };
        assert_eq!(percent(42.5, 2, 2), Some(42.5));
        assert_eq!(percent(0.1, 0, 0), Some(0.1));
        assert_eq!(percent(42.5, 1, 2), None);
        assert_eq!(percent(0., 2, 2), None);
        assert_eq!(percent(100., 2, 2), None);
    }

    #[test]
    fn output_is_passed_on_in_chunks_until_the_pipe_closes() {
        let (reader, mut writer) = io::pipe().unwrap();
        let (sender, messages) = async_channel::unbounded();
        let pumping = thread::spawn(move || pump(reader, &sender));
        writer.write_all(b" 12.5% a\nEnter").unwrap();
        writer.write_all(b" password: ").unwrap();
        drop(writer);
        pumping.join().unwrap();
        let mut output = Vec::new();
        while let Ok(Message::Output(bytes)) = messages.try_recv() {
            output.extend(bytes);
        }
        assert_eq!(output, b" 12.5% a\nEnter password: ");
        assert!(messages.is_closed());
    }

    #[cfg(unix)]
    #[test]
    fn a_signal_counts_as_a_crash_and_everything_else_as_an_exit_code() {
        use std::os::unix::process::ExitStatusExt;

        assert_eq!(exit(Ok(ExitStatus::from_raw(0))), Exit::Code(0));
        assert_eq!(exit(Ok(ExitStatus::from_raw(1 << 8))), Exit::Code(1));
        assert_eq!(exit(Ok(ExitStatus::from_raw(9))), Exit::Crashed);
        assert_eq!(exit(Ok(ExitStatus::from_raw(134))), Exit::Crashed);
        assert_eq!(exit(Err(ErrorKind::Other.into())), Exit::Crashed);
    }

    #[test]
    fn windows_fault_codes_are_crashes_and_a_dotnet_exception_is_an_exit_code() {
        for fault in [0x8000_0000_u32, 0xC000_0005, 0xCFFF_FFFF] {
            assert!(crashed(fault.cast_signed()), "{fault:#x}");
        }
        let managed = 0xE043_4352_u32.cast_signed();
        assert_eq!(managed, -532462766);
        for code in [0, 1, -1, i32::MAX, managed, 0xD000_0000_u32.cast_signed()] {
            assert!(!crashed(code), "{code}");
        }
    }

    #[gpui::test]
    fn a_download_runs_both_depots_applies_the_loader_and_reports_done(cx: &mut TestAppContext) {
        let rig = rig(cx, ACCOUNT, |_, _| {});
        rig.launcher
            .update(cx, |it, cx| it.enqueue(VANILLA, false, "", cx));
        assert_eq!(rig.state(cx), State::Preparing);
        assert_eq!(rig.active(cx), Some(edition(VANILLA, false)));
        assert_eq!(rig.stored(), ["Y1S0_Vanilla @lib download"]);
        assert_eq!(rig.runs(), 0);
        cx.run_until_parked();
        assert_eq!(rig.state(cx), State::Downloading);
        assert_eq!(rig.calls(), ["fetch loader=true"]);
        let target = rig.lib.join(VANILLA);
        assert!(target.is_dir());
        let common = [
            "-app",
            "359550",
            "-username",
            "ash",
            "-remember-password",
            "-dir",
            &target.to_string_lossy(),
            "-validate",
            "-max-downloads",
            "25",
        ];
        let worldwide = [
            "DepotDownloader",
            "-depot",
            "377237",
            "-manifest",
            "8358812283631269928",
        ];
        assert_eq!(rig.args(0), [&worldwide[..], &common[..]].concat());
        let progress = |cx: &mut TestAppContext| {
            rig.launcher.read_with(cx, |it, _| {
                let download = &it.download;
                (download.progress, download.step, download.steps)
            })
        };
        assert_eq!(progress(cx), (0., 1, 2));
        rig.feed(cx, " 12.34% a\r\n 12.39% b\n 55.5% c\nplain\n 3");
        assert_eq!(progress(cx), (55.5, 1, 2));
        rig.feed(cx, "0.00% d\n250% e\n");
        assert_eq!(progress(cx), (100., 1, 2));
        let history = rig.launcher.read_with(cx, |it, _| {
            let lines = it.download.history().iter();
            lines.cloned().collect::<Vec<_>>()
        });
        assert_eq!(
            history,
            [
                " 12.34% a",
                " 12.39% b",
                " 55.5% c",
                "plain",
                " 30.00% d",
                "250% e"
            ]
        );
        rig.close(cx, Exit::Code(0));
        assert_eq!(rig.runs(), 2);
        assert_eq!(
            rig.args(1)[..5],
            [
                "DepotDownloader",
                "-depot",
                "359551",
                "-manifest",
                "3893422760579204530"
            ]
        );
        assert_eq!(rig.args(1)[5..], common);
        assert_eq!(progress(cx), (0., 2, 2));
        assert!(rig.seen().is_empty());
        assert!(!target.join(TL_LAUNCHER).exists());
        rig.close(cx, Exit::Code(0));
        assert_eq!(rig.state(cx), State::Idle);
        assert_eq!(progress(cx), (100., 0, 0));
        assert_eq!(rig.seen(), [done(VANILLA, Outcome::Done)]);
        assert_eq!(rig.active(cx), None);
        assert!(rig.stored().is_empty());
        assert!(target.join(TL_LAUNCHER).exists());
        let config = fs::read_to_string(target.join(TL_TOML)).unwrap();
        assert!(config.contains("username = 'ThrowbackUser'"), "{config}");
        assert_eq!(rig.calls(), Vec::<String>::new());
    }

    #[gpui::test]
    fn the_stored_queue_is_restored_and_a_stored_pause_comes_back_paused(cx: &mut TestAppContext) {
        let stored = queue(&[
            ("Y0S0_Gone", false, "", false),
            (SHADOW, false, "", false),
            (ICE, false, "", true),
            (BLOOD, false, "", true),
            (VANILLA, false, "", false),
            (ICE, true, "", false),
        ]);
        let settings = format!("{ACCOUNT}queue_paused = true\n{stored}");
        let rig = rig(cx, &settings, |lib, other| {
            install(lib, SHADOW);
            install(lib, BLOOD);
            partial(other, VANILLA);
        });
        assert_eq!(rig.state(cx), State::Paused);
        assert_eq!(rig.active(cx), Some(edition(VANILLA, false)));
        assert_eq!(rig.queued(cx), [edition(BLOOD, false), edition(ICE, true)]);
        let uses = |cx: &mut TestAppContext, root: &Path| {
            rig.launcher.read_with(cx, |it, _| {
                it.download.uses_library(root, it.settings.roots())
            })
        };
        assert!(uses(cx, &rig.other));
        assert!(uses(cx, &rig.lib));
        assert!(!uses(cx, &rig.paths.default_downloads_dir()));
        rig.act(cx, |it, cx| it.start_next(cx));
        assert_eq!((rig.state(cx), rig.runs()), (State::Paused, 0));
        assert!(rig.store().queue_paused());
        rig.act(cx, |it, cx| it.set_paused(false, cx));
        assert_eq!(rig.state(cx), State::Downloading);
        assert!(
            rig.args(0)
                .contains(&rig.other.join(VANILLA).to_string_lossy().into_owned())
        );
        assert!(!rig.store().queue_paused());
        assert_eq!(
            rig.stored(),
            [
                "Y1S0_Vanilla @other download",
                "Y9S2_NewBlood @ verify",
                "Y1S1_BlackIce+hm @ download"
            ]
        );
        assert!(rig.seen().is_empty());
    }

    #[gpui::test]
    fn a_stored_pause_without_a_download_to_hold_is_dropped(cx: &mut TestAppContext) {
        let stored = queue(&[(BLOOD, false, "", true)]);
        let settings = format!("{ACCOUNT}queue_paused = true\n{stored}");
        let rig = rig(cx, &settings, |lib, _| install(lib, BLOOD));
        assert_eq!(rig.state(cx), State::Idle);
        assert_eq!(rig.active(cx), None);
        assert!(!rig.store().queue_paused());
        rig.host.running.lock().unwrap().push(BLOOD.to_owned());
        rig.act(cx, |it, cx| it.start_next(cx));
        assert_eq!(rig.queued(cx), [edition(BLOOD, false)]);
        rig.host.running.lock().unwrap().clear();
        rig.act(cx, |it, cx| it.start_next(cx));
        assert_eq!(rig.state(cx), State::Downloading);
        assert!(rig.launcher.read_with(cx, |it, _| it.download.verifying()));
        assert!(rig.queued(cx).is_empty());
        assert_eq!(rig.stored(), ["Y9S2_NewBlood @lib verify"]);
    }

    #[gpui::test]
    fn a_start_without_a_steam_account_asks_for_it_and_answers_the_password_prompt(
        cx: &mut TestAppContext,
    ) {
        let rig = rig(cx, "", |_, _| {});
        let kind =
            |cx: &mut TestAppContext| rig.launcher.read_with(cx, |it, _| it.download.login_kind());
        rig.enqueue(cx, VANILLA, false, "");
        assert_eq!(rig.seen(), [login(LoginKind::Account)]);
        assert_eq!(
            (rig.state(cx), kind(cx)),
            (State::Idle, Some(LoginKind::Account))
        );
        rig.enqueue(cx, ICE, false, "");
        rig.act(cx, |it, cx| it.verify(ICE, false, cx));
        rig.act(cx, |it, cx| it.switch_to_hm(SHADOW, cx));
        assert!(rig.seen().is_empty());
        rig.act(cx, |it, cx| {
            it.submit_account_login("not valid", "secret", cx)
        });
        assert_eq!(
            rig.seen(),
            [failed("Invalid Steam account"), login(LoginKind::Account)]
        );
        assert_eq!(kind(cx), Some(LoginKind::Account));
        rig.act(cx, |it, cx| it.submit_account_login(" ash ", "secret", cx));
        assert_eq!(rig.store().steam_account(), "ash");
        assert_eq!((rig.state(cx), kind(cx)), (State::Downloading, None));
        assert_eq!(rig.active(cx), Some(edition(VANILLA, false)));
        assert_eq!(rig.args(0)[8], "ash");
        rig.feed(
            cx,
            "Connecting to Steam3...\nEnter account password for \"ash\": ",
        );
        assert_eq!(rig.typed(), ["secret"]);
        rig.feed(cx, "\nEnter account password for \"ash\": ");
        assert_eq!(rig.seen(), [login(LoginKind::Password)]);
        assert_eq!(rig.typed(), ["secret"]);
        rig.act(cx, |it, cx| it.submit_login("again", cx));
        rig.act(cx, |it, cx| it.submit_login("ignored", cx));
        assert_eq!(rig.typed(), ["secret", "again"]);
        assert_eq!(kind(cx), None);
        let guard =
            "STEAM GUARD! Please enter your 2-factor auth code from your authenticator app: ";
        rig.feed(cx, &format!("\n{guard}"));
        rig.feed(cx, " ");
        assert_eq!(rig.seen(), [login(LoginKind::Guard)]);
        rig.act(cx, |it, cx| it.submit_login("12345", cx));
        rig.feed(
            cx,
            "\nPlease enter the auth code sent to the email at a@b.c: ",
        );
        assert_eq!(rig.seen(), [login(LoginKind::GuardEmail)]);
        assert_eq!(kind(cx), Some(LoginKind::GuardEmail));
        rig.act(cx, |it, cx| it.submit_login("67890", cx));
        assert_eq!(rig.typed(), ["secret", "again", "12345", "67890"]);
        rig.feed(
            cx,
            "\nSTEAM GUARD! Use the Steam Mobile App to confirm your sign in\n",
        );
        assert_eq!(rig.seen(), [login(LoginKind::GuardMobile)]);
        assert_eq!(kind(cx), Some(LoginKind::GuardMobile));
        rig.feed(cx, "Done\n 1.00% a\n");
        assert_eq!(rig.seen(), [DownloadEvent::LoginRequired(None)]);
        assert_eq!(kind(cx), None);
        rig.feed(cx, "Enter account password for \"ash\": ");
        assert_eq!(kind(cx), Some(LoginKind::Password));
        rig.act(cx, |it, cx| it.cancel_download(cx));
        assert!(rig.killed());
        assert_eq!(
            rig.seen(),
            [
                login(LoginKind::Password),
                done(VANILLA, Outcome::Cancelled)
            ]
        );
        assert_eq!((rig.state(cx), kind(cx)), (State::Idle, None));
        assert_eq!(rig.active(cx), None);
        assert!(rig.lib.join(VANILLA).is_dir());
        assert!(rig.stored().is_empty());
    }

    #[gpui::test]
    fn a_cancelled_account_request_is_forgotten(cx: &mut TestAppContext) {
        let settings = queue(&[(ICE, false, "", false)]);
        let rig = rig(cx, &settings, |_, _| {});
        rig.act(cx, |it, cx| it.start_next(cx));
        assert_eq!((rig.state(cx), rig.runs()), (State::Idle, 0));
        rig.act(cx, |it, cx| it.verify(VANILLA, false, cx));
        assert_eq!(rig.seen(), [login(LoginKind::Account)]);
        rig.act(cx, |it, cx| it.cancel_download(cx));
        assert_eq!(
            rig.launcher.read_with(cx, |it, _| it.download.login_kind()),
            None
        );
        rig.act(cx, |it, cx| it.start_next(cx));
        assert_eq!((rig.state(cx), rig.runs()), (State::Idle, 0));
    }

    #[gpui::test]
    fn a_new_download_is_checked_against_season_library_and_folder(cx: &mut TestAppContext) {
        let rig = rig(cx, ACCOUNT, |lib, other| {
            partial(other, VANILLA);
            install(lib, BLOOD);
        });
        let fixed = rig
            .paths
            .default_downloads_dir()
            .to_string_lossy()
            .into_owned();
        let lib = rig.lib.to_string_lossy().into_owned();
        for (key, library, message) in [
            ("Y0S0_Gone", "", "Unknown season"),
            (ICE, "/somewhere/else", "Unknown library"),
            (ICE, fixed.as_str(), FOLDER_NOT_FOUND),
            (VANILLA, lib.as_str(), PARTIAL_EXISTS),
        ] {
            rig.enqueue(cx, key, false, library);
            assert_eq!(rig.seen(), [failed(message)], "{key}");
            assert_eq!((rig.state(cx), rig.runs()), (State::Idle, 0));
        }
        rig.enqueue(cx, VANILLA, false, "");
        assert_eq!(rig.state(cx), State::Downloading);
        assert!(
            rig.args(0)
                .contains(&rig.other.join(VANILLA).to_string_lossy().into_owned())
        );
        rig.enqueue(cx, VANILLA, false, "");
        rig.enqueue(cx, ICE, false, &lib);
        rig.enqueue(cx, ICE, false, "");
        rig.enqueue(cx, BLOOD, false, "");
        rig.act(cx, |it, cx| it.verify(BLOOD, false, cx));
        rig.act(cx, |it, cx| it.verify(ICE, false, cx));
        assert!(rig.seen().is_empty());
        rig.act(cx, |it, cx| it.verify(SHADOW, false, cx));
        assert_eq!(rig.seen(), [failed(NOT_INSTALLED)]);
        assert_eq!(
            rig.stored(),
            [
                "Y1S0_Vanilla @other download",
                "Y1S1_BlackIce @lib download",
                "Y9S2_NewBlood @ verify"
            ]
        );
        assert_eq!(rig.queued(cx), [edition(ICE, false), edition(BLOOD, false)]);
        for action in [
            Launcher::switch_to_hm,
            Launcher::remove_hm,
            Launcher::restore_hm,
            Launcher::update_hm,
        ] {
            rig.act(cx, |it, cx| action(it, SHADOW, cx));
            assert_eq!(rig.seen(), [failed(DOWNLOAD_RUNNING)]);
        }
        rig.act(cx, |it, cx| {
            it.reorder_queue(&[edition(BLOOD, false), edition(SHADOW, true)], cx);
        });
        assert_eq!(rig.queued(cx), [edition(BLOOD, false), edition(ICE, false)]);
        assert_eq!(rig.stored()[1], "Y9S2_NewBlood @ verify");
        rig.act(cx, |it, cx| it.dequeue(BLOOD, false, cx));
        rig.act(cx, |it, cx| it.dequeue(BLOOD, true, cx));
        assert_eq!(rig.queued(cx), [edition(ICE, false)]);
        assert_eq!(rig.stored().len(), 2);
        assert!(rig.launcher.read_with(cx, |it, _| {
            it.download.busy_with(VANILLA) && !it.download.busy_with(ICE)
        }));
    }

    #[gpui::test]
    fn an_installed_edition_is_verified_instead_of_downloaded(cx: &mut TestAppContext) {
        let rig = rig(cx, ACCOUNT, |lib, _| install(lib, VANILLA));
        rig.host.running.lock().unwrap().push(VANILLA.to_owned());
        rig.enqueue(cx, VANILLA, false, "");
        assert_eq!(rig.seen(), [failed(SEASON_RUNNING)]);
        rig.act(cx, |it, cx| it.verify(ICE, false, cx));
        assert_eq!(rig.seen(), [failed(NOT_INSTALLED)]);
        rig.host.running.lock().unwrap().clear();
        *rig.host.free.lock().unwrap() = Some(0);
        rig.enqueue(cx, VANILLA, false, "");
        assert_eq!(rig.state(cx), State::Downloading);
        assert!(rig.launcher.read_with(cx, |it, _| it.download.verifying()));
        assert_eq!(rig.stored(), ["Y1S0_Vanilla @lib verify"]);
        rig.act(cx, |it, cx| it.set_paused(true, cx));
        assert!(!rig.killed());
        assert_eq!(rig.state(cx), State::Downloading);
        assert!(rig.store().queue_paused());
        *rig.host.free.lock().unwrap() = None;
        rig.close(cx, Exit::Code(0));
        rig.close(cx, Exit::Code(0));
        assert_eq!(rig.seen(), [done(VANILLA, Outcome::Verified)]);
        assert_eq!(rig.state(cx), State::Idle);
        assert!(!rig.launcher.read_with(cx, |it, _| it.download.verifying()));
        assert!(!rig.store().queue_paused());
    }

    #[gpui::test]
    fn a_folder_without_the_marker_is_downloaded_even_when_a_verify_was_asked(
        cx: &mut TestAppContext,
    ) {
        let rig = rig(cx, ACCOUNT, |lib, _| partial(lib, VANILLA));
        rig.host.running.lock().unwrap().push(VANILLA.to_owned());
        rig.act(cx, |it, cx| it.verify(VANILLA, false, cx));
        assert_eq!(rig.state(cx), State::Downloading);
        assert!(!rig.launcher.read_with(cx, |it, _| it.download.verifying()));
        assert_eq!(rig.stored(), ["Y1S0_Vanilla @lib download"]);
    }

    #[gpui::test]
    fn a_download_that_does_not_fit_asks_first(cx: &mut TestAppContext) {
        let rig = rig(cx, ACCOUNT, |_, other| {
            partial(other, ICE);
            let file = fs::File::create(other.join(ICE).join("big.forge")).unwrap();
            file.set_len(80 * GIB).unwrap();
        });
        let shortfall = |cx: &mut TestAppContext| {
            rig.launcher
                .read_with(cx, |it, _| it.download.disk_shortfall())
        };
        *rig.host.free.lock().unwrap() = Some(10 * GIB);
        rig.enqueue(cx, VANILLA, false, "");
        assert_eq!(rig.seen(), [DownloadEvent::DiskSpaceRequired(6.2)]);
        assert_eq!((rig.state(cx), shortfall(cx)), (State::Idle, Some(6.2)));
        assert_eq!(rig.stored(), ["Y1S0_Vanilla @ download"]);
        rig.enqueue(cx, ICE, false, "");
        rig.act(cx, |it, cx| it.start_next(cx));
        assert!(rig.seen().is_empty());
        rig.act(cx, |it, cx| it.cancel_download(cx));
        assert_eq!(shortfall(cx), None);
        assert!(rig.stored().is_empty());
        rig.act(cx, |it, cx| it.confirm_disk_space(cx));
        assert_eq!((rig.state(cx), rig.runs()), (State::Idle, 0));
        rig.enqueue(cx, VANILLA, false, "");
        assert_eq!(rig.seen(), [DownloadEvent::DiskSpaceRequired(6.2)]);
        rig.act(cx, |it, cx| it.confirm_disk_space(cx));
        assert_eq!((rig.state(cx), shortfall(cx)), (State::Downloading, None));
        assert_eq!(rig.stored(), ["Y1S0_Vanilla @lib download"]);
        rig.act(cx, |it, cx| it.cancel_download(cx));
        assert_eq!(rig.seen(), [done(VANILLA, Outcome::Cancelled)]);
        rig.enqueue(cx, ICE, false, "");
        assert_eq!(rig.state(cx), State::Downloading);
        assert_eq!(rig.stored(), ["Y1S1_BlackIce @other download"]);
        assert!(rig.seen().is_empty());
    }

    #[gpui::test]
    fn a_pending_disk_question_keeps_its_edition_in_the_stored_queue(cx: &mut TestAppContext) {
        let settings = format!("{ACCOUNT}{}", queue(&[(ICE, false, "", false)]));
        let rig = rig(cx, &settings, |_, _| {});
        *rig.host.free.lock().unwrap() = Some(0);
        rig.act(cx, |it, cx| it.start_next(cx));
        assert_eq!(rig.seen(), [DownloadEvent::DiskSpaceRequired(18.7)]);
        assert!(rig.queued(cx).is_empty());
        assert_eq!(rig.stored(), ["Y1S1_BlackIce @ download"]);
        *rig.host.free.lock().unwrap() = None;
        rig.act(cx, |it, cx| it.cancel_download(cx));
        assert!(rig.stored().is_empty());
        assert_eq!((rig.state(cx), rig.runs()), (State::Idle, 0));
    }

    #[gpui::test]
    fn failures_of_depotdownloader_are_told_apart(cx: &mut TestAppContext) {
        let rig = rig(cx, ACCOUNT, |_, _| {});
        rig.download(cx, VANILLA);
        rig.feed(
            cx,
            "Failed to authenticate with Steam: x with result InvalidPassword.\n\
             Unable to get steam3 credentials.\nError: InitializeSteam failed\n",
        );
        rig.close(cx, Exit::Code(1));
        assert_eq!(
            rig.seen(),
            [failed(LOGIN_FAILED), done(VANILLA, Outcome::Failed)]
        );
        assert_eq!(rig.state(cx), State::Failed);
        assert_eq!(rig.active(cx), Some(edition(VANILLA, false)));
        assert_eq!(rig.store().steam_account(), "");
        assert!(rig.stored().is_empty());
        rig.enqueue(cx, VANILLA, false, "");
        assert_eq!(rig.seen(), [login(LoginKind::Account)]);
        rig.act(cx, |it, cx| it.submit_account_login("ash", "", cx));
        assert_eq!(rig.state(cx), State::Downloading);

        rig.feed(cx, "Depot 377237 is not available from this account.\n");
        rig.close(cx, Exit::Code(0));
        assert_eq!(
            rig.seen(),
            [
                failed("Your R6S license is not supported"),
                done(VANILLA, Outcome::Failed)
            ]
        );
        assert_eq!((rig.state(cx), rig.runs()), (State::Failed, 2));
        assert_eq!(rig.store().steam_account(), "ash");

        rig.download(cx, VANILLA);
        rig.feed(
            cx,
            "Unhandled exception. System.IO.IOException\n   at Download()\n",
        );
        rig.close(cx, Exit::Crashed);
        assert_eq!(
            rig.seen(),
            [failed("Download failed"), done(VANILLA, Outcome::Failed)]
        );

        rig.download(cx, VANILLA);
        *rig.host.free.lock().unwrap() = Some(2 * GIB - 1);
        rig.close(cx, Exit::Code(1));
        assert_eq!(rig.seen(), [done(VANILLA, Outcome::NoSpace)]);
        assert_eq!(rig.state(cx), State::Failed);

        *rig.host.free.lock().unwrap() = None;
        rig.download(cx, VANILLA);
        *rig.host.free.lock().unwrap() = Some(0);
        rig.feed(cx, "Could not connect to Steam after 10 tries\n");
        rig.close(cx, Exit::Code(1));
        assert_eq!(
            rig.seen(),
            [
                failed("Could not connect to Steam"),
                done(VANILLA, Outcome::Failed)
            ]
        );

        *rig.host.free.lock().unwrap() = None;
        rig.host.missing.store(true, Ordering::SeqCst);
        rig.enqueue(cx, VANILLA, false, "");
        assert_eq!(
            rig.seen(),
            [
                failed("DepotDownloader failed to start"),
                done(VANILLA, Outcome::Failed)
            ]
        );
        rig.host.missing.store(false, Ordering::SeqCst);

        let broken = "DepotDownloader download failed".to_owned();
        *rig.host.fetch.lock().unwrap() = Some(Fault::Message(broken));
        rig.enqueue(cx, VANILLA, false, "");
        assert_eq!(
            rig.seen(),
            [
                failed("DepotDownloader download failed"),
                done(VANILLA, Outcome::Failed)
            ]
        );
        assert_eq!(rig.state(cx), State::Failed);
        assert!(!rig.store().queue_paused());
    }

    #[gpui::test]
    fn busy_peers_and_broken_setups_answer_with_their_own_message(cx: &mut TestAppContext) {
        let rig = rig(cx, ACCOUNT, |lib, _| fs::write(lib.join(ICE), "").unwrap());
        rig.launcher
            .update(cx, |it, _| it.uninstall = Uninstaller::busy_with(VANILLA));
        rig.enqueue(cx, VANILLA, false, "");
        rig.launcher.update(cx, |it, _| {
            it.uninstall = Uninstaller::default();
            it.shears = Shears::busy_with(VANILLA);
        });
        rig.enqueue(cx, VANILLA, false, "");
        rig.launcher.update(cx, |it, _| {
            it.shears = Shears::busy_with(ICE);
            it.update = Updater::busy_now();
        });
        rig.act(cx, |it, cx| it.restore_hm(VANILLA, cx));
        rig.launcher.update(cx, |it, _| {
            it.shears = Shears::default();
            it.update = Updater::default();
        });
        assert_eq!(
            rig.seen(),
            [
                failed(UNINSTALL_RUNNING),
                failed("Shears is running"),
                failed(UPDATE_RUNNING)
            ]
        );
        assert_eq!((rig.state(cx), rig.calls().len()), (State::Idle, 0));

        rig.enqueue(cx, ICE, false, "");
        assert_eq!(
            rig.seen(),
            [failed("Setup failed"), done(ICE, Outcome::Failed)]
        );
        assert_eq!((rig.state(cx), rig.runs()), (State::Failed, 0));

        rig.enqueue(cx, VANILLA, true, "");
        assert_eq!(rig.state(cx), State::Downloading);
        let target = rig.lib.join("Y1S0_HeatedMetal");
        assert!(rig.args(0).contains(&target.to_string_lossy().into_owned()));
        assert_eq!(rig.calls(), ["fetch loader=true", "fetch loader=false"]);
        rig.close(cx, Exit::Code(0));
        rig.close(cx, Exit::Code(0));
        assert_eq!(
            rig.seen(),
            [failed("Install failed"), done(VANILLA, Outcome::Failed)]
        );
        assert!(!target.join(TL_LAUNCHER).exists());

        fs::remove_file(rig.paths.tl_dir().join("steam_api64.dll")).unwrap();
        rig.download(cx, SHADOW);
        rig.close(cx, Exit::Code(0));
        rig.close(cx, Exit::Code(0));
        assert_eq!(
            rig.seen(),
            [
                failed("ThrowbackLoader setup failed"),
                done(SHADOW, Outcome::Failed)
            ]
        );
        assert!(!rig.lib.join(SHADOW).join(TL_LAUNCHER).exists());
        assert_eq!(rig.state(cx), State::Failed);
    }

    #[gpui::test]
    fn a_rate_limit_while_preparing_pauses_the_queue(cx: &mut TestAppContext) {
        let rig = rig(cx, ACCOUNT, |lib, _| install(lib, BLOOD));
        *rig.host.fetch.lock().unwrap() = Some(Fault::RateLimited(LIMIT.to_owned()));
        rig.enqueue(cx, VANILLA, false, "");
        assert_eq!(
            rig.seen(),
            [
                DownloadEvent::RateLimited(LIMIT.into()),
                done(VANILLA, Outcome::Cancelled)
            ]
        );
        assert_eq!(rig.state(cx), State::Paused);
        assert_eq!(rig.active(cx), Some(edition(VANILLA, false)));
        assert!(rig.store().queue_paused());
        assert_eq!(rig.stored(), ["Y1S0_Vanilla @lib download"]);
        rig.act(cx, |it, cx| it.set_paused(false, cx));
        assert_eq!(rig.state(cx), State::Downloading);
        assert!(!rig.store().queue_paused());
        rig.act(cx, |it, cx| it.cancel_download(cx));
        assert_eq!(rig.seen(), [done(VANILLA, Outcome::Cancelled)]);

        *rig.host.fetch.lock().unwrap() = Some(Fault::RateLimited(LIMIT.to_owned()));
        rig.act(cx, |it, cx| it.verify(BLOOD, false, cx));
        assert_eq!(
            rig.seen(),
            [
                DownloadEvent::RateLimited(LIMIT.into()),
                done(BLOOD, Outcome::Cancelled)
            ]
        );
        assert_eq!((rig.state(cx), rig.active(cx)), (State::Idle, None));
        assert!(!rig.store().queue_paused());

        *rig.host.fetch.lock().unwrap() = Some(Fault::RateLimited(LIMIT.to_owned()));
        rig.act(cx, |it, cx| {
            it.verify(BLOOD, false, cx);
            it.cancel_download(cx);
        });
        assert_eq!(
            rig.seen(),
            [
                DownloadEvent::RateLimited(LIMIT.into()),
                done(BLOOD, Outcome::Cancelled)
            ]
        );
        assert_eq!((rig.state(cx), rig.active(cx)), (State::Idle, None));
        assert!(!rig.store().queue_paused());
    }

    #[gpui::test]
    fn pause_keeps_the_download_and_another_start_puts_it_back_in_line(cx: &mut TestAppContext) {
        let rig = rig(cx, ACCOUNT, |_, _| {});
        rig.act(cx, |it, cx| it.set_paused(true, cx));
        assert_eq!(rig.state(cx), State::Idle);
        assert!(rig.store().queue_paused());
        rig.act(cx, |it, cx| it.set_paused(false, cx));
        assert!(!rig.store().queue_paused());

        rig.download(cx, VANILLA);
        rig.enqueue(cx, SHADOW, false, "");
        rig.feed(cx, " 43.21% a\n");
        rig.act(cx, |it, cx| {
            it.set_paused(true, cx);
            it.set_paused(true, cx);
        });
        assert!(rig.killed());
        assert_eq!(rig.seen(), [done(VANILLA, Outcome::Cancelled)]);
        assert_eq!(rig.state(cx), State::Paused);
        assert_eq!(rig.active(cx), Some(edition(VANILLA, false)));
        assert_eq!(rig.queued(cx), [edition(SHADOW, false)]);
        assert!(rig.store().queue_paused());
        assert_eq!(
            rig.stored(),
            ["Y1S0_Vanilla @lib download", "Y5S3_ShadowLegacy @ download"]
        );
        let shown = rig.launcher.read_with(cx, |it, _| {
            (it.download.progress, it.download.step, it.download.steps)
        });
        assert_eq!(shown, (43.21, 1, 2));
        rig.act(cx, |it, cx| it.start_next(cx));
        assert_eq!((rig.state(cx), rig.runs()), (State::Paused, 1));

        rig.enqueue(cx, ICE, false, "");
        assert_eq!(rig.state(cx), State::Downloading);
        assert_eq!(rig.active(cx), Some(edition(ICE, false)));
        assert_eq!(
            rig.queued(cx),
            [edition(VANILLA, false), edition(SHADOW, false)]
        );
        assert!(!rig.store().queue_paused());
        assert_eq!(
            rig.stored(),
            [
                "Y1S1_BlackIce @lib download",
                "Y1S0_Vanilla @lib download",
                "Y5S3_ShadowLegacy @ download"
            ]
        );

        rig.act(cx, |it, cx| it.set_paused(true, cx));
        assert_eq!(rig.seen(), [done(ICE, Outcome::Cancelled)]);
        assert_eq!(rig.state(cx), State::Paused);
        rig.act(cx, |it, cx| it.set_paused(false, cx));
        assert_eq!(rig.state(cx), State::Downloading);
        assert_eq!(rig.active(cx), Some(edition(ICE, false)));
        assert_eq!(rig.runs(), 3);
        assert!(!rig.store().queue_paused());
    }

    #[gpui::test]
    fn a_pause_while_applying_is_ignored_when_the_install_succeeds(cx: &mut TestAppContext) {
        let rig = rig(cx, ACCOUNT, |_, _| {});
        rig.download(cx, VANILLA);
        rig.enqueue(cx, ICE, false, "");
        rig.close(cx, Exit::Code(0));
        rig.launcher.update(cx, |it, cx| {
            it.depot_message(Message::Exited(Exit::Code(0)), cx);
            assert_eq!(it.download.state(), State::Applying);
            it.set_paused(true, cx);
        });
        cx.run_until_parked();
        assert_eq!(rig.seen(), [done(VANILLA, Outcome::Done)]);
        assert_eq!(rig.state(cx), State::Paused);
        assert_eq!(rig.active(cx), Some(edition(ICE, false)));
        assert!(rig.queued(cx).is_empty());
        assert_eq!(rig.stored(), ["Y1S1_BlackIce @lib download"]);
        assert!(rig.store().queue_paused());
        assert_eq!(rig.runs(), 2);
    }

    #[gpui::test]
    fn pausing_an_idle_queue_holds_its_first_download(cx: &mut TestAppContext) {
        let stored = queue(&[(BLOOD, false, "", true), (ICE, false, "", false)]);
        let rig = rig(cx, &stored, |lib, _| install(lib, BLOOD));
        rig.act(cx, |it, cx| it.set_paused(true, cx));
        assert_eq!(rig.state(cx), State::Paused);
        assert_eq!(rig.active(cx), Some(edition(ICE, false)));
        assert_eq!(rig.queued(cx), [edition(BLOOD, false)]);
        assert_eq!(
            rig.stored(),
            ["Y1S1_BlackIce @lib download", "Y9S2_NewBlood @ verify"]
        );
        assert!(rig.store().queue_paused());
        rig.act(cx, |it, cx| it.set_paused(false, cx));
        assert_eq!(rig.seen(), [login(LoginKind::Account)]);
        assert_eq!(rig.state(cx), State::Paused);
        assert!(rig.store().queue_paused());
    }

    #[gpui::test]
    fn the_queue_goes_on_after_a_run_and_waits_for_its_blockers(cx: &mut TestAppContext) {
        let stored = queue(&[
            (VANILLA, false, "/gone", false),
            (ICE, false, "", false),
            (BLOOD, false, "", true),
            (SHADOW, false, "", false),
        ]);
        let settings = format!("{ACCOUNT}{stored}");
        let rig = rig(cx, &settings, |lib, _| install(lib, BLOOD));
        rig.launcher
            .update(cx, |it, _| it.update = Updater::busy_now());
        rig.act(cx, |it, cx| it.start_next(cx));
        rig.launcher.update(cx, |it, _| {
            it.update = Updater::default();
            it.uninstall = Uninstaller::busy_with(VANILLA);
        });
        rig.act(cx, |it, cx| it.start_next(cx));
        rig.launcher.update(cx, |it, _| {
            it.uninstall = Uninstaller::busy_with(ICE);
            it.shears = Shears::busy_with(VANILLA);
        });
        rig.act(cx, |it, cx| it.start_next(cx));
        assert_eq!((rig.state(cx), rig.queued(cx).len()), (State::Idle, 4));
        assert!(rig.seen().is_empty());
        rig.launcher
            .update(cx, |it, _| it.shears = Shears::default());
        rig.act(cx, |it, cx| it.start_next(cx));
        assert_eq!(rig.seen(), [failed("Unknown library")]);
        assert_eq!((rig.state(cx), rig.queued(cx).len()), (State::Idle, 3));
        rig.launcher
            .update(cx, |it, _| it.uninstall = Uninstaller::default());
        rig.act(cx, |it, cx| it.start_next(cx));
        assert_eq!(rig.active(cx), Some(edition(ICE, false)));
        rig.host.running.lock().unwrap().push(BLOOD.to_owned());
        rig.close(cx, Exit::Code(0));
        rig.close(cx, Exit::Code(0));
        assert_eq!(rig.seen(), [done(ICE, Outcome::Done)]);
        assert_eq!(rig.state(cx), State::Idle);
        assert_eq!(
            rig.queued(cx),
            [edition(BLOOD, false), edition(SHADOW, false)]
        );
        rig.host.running.lock().unwrap().clear();
        let order = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let order = order.clone();
            let note =
                move |launcher: Entity<Launcher>, event: &DownloadEvent, cx: &mut gpui::App| {
                    let state = launcher.read(cx).download.state();
                    order.borrow_mut().push((event.clone(), state));
                };
            cx.subscribe(&rig.launcher, note).detach();
        });
        rig.act(cx, |it, cx| it.start_next(cx));
        assert!(rig.launcher.read_with(cx, |it, _| it.download.verifying()));
        rig.close(cx, Exit::Code(0));
        rig.close(cx, Exit::Code(0));
        assert_eq!(
            *order.borrow(),
            [(done(BLOOD, Outcome::Verified), State::Idle)]
        );
        assert_eq!(rig.state(cx), State::Downloading);
        assert_eq!(rig.active(cx), Some(edition(SHADOW, false)));
        assert!(rig.queued(cx).is_empty());
    }

    #[gpui::test]
    fn partial_downloads_are_removed_everywhere_but_a_complete_copy_stays(cx: &mut TestAppContext) {
        let settings = format!("{ACCOUNT}{}", queue(&[(ICE, false, "", false)]));
        let rig = rig(cx, &settings, |lib, other| {
            partial(lib, VANILLA);
            install(other, VANILLA);
            partial(lib, ICE);
            partial(other, ICE);
        });
        rig.launcher
            .update(cx, |it, _| it.uninstall = Uninstaller::busy_with(VANILLA));
        rig.act(cx, |it, cx| it.delete_partial(VANILLA, false, cx));
        rig.launcher.update(cx, |it, _| {
            it.uninstall = Uninstaller::default();
            it.shears = Shears::busy_with(VANILLA);
        });
        rig.act(cx, |it, cx| it.delete_partial(VANILLA, false, cx));
        rig.launcher.update(cx, |it, _| {
            it.shears = Shears::default();
            it.update = Updater::busy_now();
        });
        rig.act(cx, |it, cx| it.delete_partial(VANILLA, false, cx));
        rig.launcher
            .update(cx, |it, _| it.update = Updater::default());
        rig.host.running.lock().unwrap().push(VANILLA.to_owned());
        rig.act(cx, |it, cx| it.delete_partial(VANILLA, false, cx));
        rig.host.running.lock().unwrap().clear();
        rig.act(cx, |it, cx| it.delete_partial(SHADOW, false, cx));
        assert_eq!(
            rig.seen(),
            [
                refused(VANILLA, UNINSTALL_RUNNING),
                refused(VANILLA, "Shears is running"),
                refused(VANILLA, UPDATE_RUNNING),
                refused(VANILLA, SEASON_RUNNING),
                refused(SHADOW, "Nothing to remove"),
            ]
        );
        rig.launcher.update(cx, |it, cx| {
            it.delete_partial(ICE, false, cx);
            it.delete_partial(VANILLA, false, cx);
            it.switch_to_hm(ICE, cx);
            it.start_next(cx);
            assert_eq!(it.download.state(), State::Idle);
        });
        assert_eq!(
            rig.seen(),
            [refused(VANILLA, REMOVING_FILES), failed(REMOVING_FILES)]
        );
        assert!(rig.lib.join(ICE).exists());
        cx.run_until_parked();
        assert!(!rig.lib.join(ICE).join("data.forge").exists());
        assert!(!rig.other.join(ICE).exists());
        let removed = |key: &str| DownloadEvent::PartialDeleted {
            key: key.to_owned(),
            error: None,
        };
        assert_eq!(rig.seen(), [removed(ICE)]);
        assert_eq!(rig.state(cx), State::Downloading);
        assert_eq!(rig.active(cx), Some(edition(ICE, false)));

        rig.act(cx, |it, cx| it.delete_partial(ICE, false, cx));
        assert_eq!(rig.seen(), [refused(ICE, BUSY_MESSAGE)]);
        rig.act(cx, |it, cx| it.delete_partial(VANILLA, false, cx));
        assert_eq!(rig.seen(), [removed(VANILLA)]);
        assert!(!rig.lib.join(VANILLA).exists());
        assert!(rig.other.join(VANILLA).join(TL_LAUNCHER).exists());

        rig.close(cx, Exit::Code(3));
        assert_eq!(
            rig.seen(),
            [failed("Download failed"), done(ICE, Outcome::Failed)]
        );
        fs::remove_dir_all(rig.lib.join(ICE)).unwrap();
        rig.act(cx, |it, cx| it.delete_partial(ICE, false, cx));
        assert_eq!(rig.seen(), [removed(ICE)]);
        assert_eq!((rig.state(cx), rig.active(cx)), (State::Idle, None));
    }

    #[gpui::test]
    fn removing_the_paused_download_hands_the_pause_to_the_next_one(cx: &mut TestAppContext) {
        let rig = rig(cx, ACCOUNT, |_, _| {});
        rig.download(cx, VANILLA);
        rig.enqueue(cx, ICE, false, "");
        rig.act(cx, |it, cx| it.set_paused(true, cx));
        assert_eq!(rig.seen(), [done(VANILLA, Outcome::Cancelled)]);
        rig.act(cx, |it, cx| it.delete_partial(VANILLA, false, cx));
        assert_eq!(rig.seen().len(), 1);
        assert_eq!(rig.state(cx), State::Paused);
        assert_eq!(rig.active(cx), Some(edition(ICE, false)));
        assert_eq!(rig.stored(), ["Y1S1_BlackIce @lib download"]);
        assert!(rig.store().queue_paused());
        fs::create_dir_all(rig.lib.join(ICE)).unwrap();
        rig.act(cx, |it, cx| it.delete_partial(ICE, false, cx));
        assert_eq!((rig.state(cx), rig.active(cx)), (State::Idle, None));
        assert!(!rig.store().queue_paused());
        assert!(rig.stored().is_empty());
    }

    #[gpui::test]
    fn a_paused_download_does_not_return_to_the_queue_while_it_is_removed(cx: &mut TestAppContext) {
        let rig = rig(cx, ACCOUNT, |_, _| {});
        rig.download(cx, VANILLA);
        rig.act(cx, |it, cx| it.set_paused(true, cx));
        rig.act(cx, |it, cx| {
            it.delete_partial(VANILLA, false, cx);
            it.enqueue(ICE, false, "", cx);
        });
        assert_eq!(rig.active(cx), Some(edition(ICE, false)));
        assert!(rig.queued(cx).is_empty());
        assert_eq!(rig.stored(), ["Y1S1_BlackIce @lib download"]);
        assert!(!rig.lib.join(VANILLA).exists());
    }

    #[gpui::test]
    fn switching_to_heated_metal_renames_the_folder_and_keeps_its_home_place(
        cx: &mut TestAppContext,
    ) {
        let settings = "home_order = ['a', 'Y5S3_ShadowLegacy', 'b']\n";
        let rig = rig(cx, settings, |lib, _| install(lib, VANILLA));
        let switch =
            |cx: &mut TestAppContext, key: &str| rig.act(cx, |it, cx| it.switch_to_hm(key, cx));
        switch(cx, SHADOW);
        assert_eq!(rig.seen(), [failed("Not installed as Throwback")]);
        switch(cx, VANILLA);
        assert_eq!(
            rig.seen(),
            [failed("Heated Metal is not available for this season")]
        );
        install(&rig.lib, SHADOW);
        install(&rig.other, SHADOW_HM);
        switch(cx, SHADOW);
        assert_eq!(rig.seen(), [failed("Already installed as Heated Metal")]);
        fs::remove_file(rig.other.join(SHADOW_HM).join(TL_LAUNCHER)).unwrap();
        switch(cx, SHADOW);
        assert_eq!(rig.seen(), [failed(PARTIAL_EXISTS)]);
        fs::remove_dir_all(rig.other.join(SHADOW_HM)).unwrap();
        rig.host.running.lock().unwrap().push(SHADOW.to_owned());
        switch(cx, SHADOW);
        assert_eq!(rig.seen(), [failed(SEASON_RUNNING)]);
        rig.host.running.lock().unwrap().clear();
        assert_eq!((rig.state(cx), rig.calls().len()), (State::Idle, 0));

        let broken = "Heated Metal extraction failed".to_owned();
        *rig.host.hm.lock().unwrap() = Some(Fault::Message(broken));
        rig.launcher.update(cx, |it, cx| {
            it.switch_to_hm(SHADOW, cx);
            assert_eq!(it.download.state(), State::Applying);
            assert_eq!(it.download.active(), Some(&edition(SHADOW, true)));
        });
        cx.run_until_parked();
        assert_eq!(
            rig.seen(),
            [
                failed("Heated Metal extraction failed"),
                done(SHADOW, Outcome::Failed)
            ]
        );
        assert_eq!(
            rig.calls(),
            ["apply_hm Y5S3_HeatedMetal ThrowbackUser 0.2.3 "]
        );
        assert!(!rig.lib.join(SHADOW).exists());
        assert!(rig.lib.join(SHADOW_HM).is_dir());
        assert!(!rig.lib.join(SHADOW_HM).join(TL_LAUNCHER).exists());
        assert_eq!(rig.store().home_order(), ["a", SHADOW_HM, "b"]);
        assert_eq!(rig.state(cx), State::Failed);

        rig.enqueue(cx, SHADOW, true, "");
        assert_eq!(rig.seen(), [login(LoginKind::Account)]);
        rig.act(cx, |it, cx| it.cancel_download(cx));
        fs::rename(rig.lib.join(SHADOW_HM), rig.lib.join(SHADOW)).unwrap();
        install(&rig.lib, SHADOW);
        switch(cx, SHADOW);
        assert_eq!(rig.seen(), [done(SHADOW, Outcome::SwitchedHm)]);
        assert!(rig.lib.join(SHADOW_HM).join(TL_LAUNCHER).exists());
        assert_eq!(rig.state(cx), State::Idle);
        assert_eq!(rig.runs(), 0);
    }

    #[gpui::test]
    fn a_switch_whose_folder_cannot_be_moved_fails_as_a_switch(cx: &mut TestAppContext) {
        let settings = format!("{ACCOUNT}home_order = ['{SHADOW}', '{BLOOD_HM}']\n");
        let rig = rig(cx, &settings, |lib, _| {
            install(lib, SHADOW);
            install(lib, BLOOD_HM);
        });
        rig.launcher
            .update(cx, |it, cx| it.switch_to_hm(SHADOW, cx));
        fs::remove_dir_all(rig.lib.join(SHADOW)).unwrap();
        cx.run_until_parked();
        assert_eq!(
            rig.seen(),
            [failed("Switch failed"), done(SHADOW, Outcome::Failed)]
        );
        assert!(rig.calls().is_empty());
        rig.launcher.update(cx, |it, cx| it.remove_hm(BLOOD, cx));
        fs::remove_dir_all(rig.lib.join(BLOOD_HM)).unwrap();
        cx.run_until_parked();
        assert_eq!(
            rig.seen(),
            [failed("Switch failed"), done(BLOOD, Outcome::Failed)]
        );
        assert_eq!(rig.calls(), ["fetch loader=true"]);
        assert_eq!(rig.store().home_order(), [SHADOW, BLOOD_HM]);
        assert_eq!((rig.state(cx), rig.runs()), (State::Failed, 0));
    }

    #[gpui::test]
    fn heated_metal_is_applied_again_without_depotdownloader(cx: &mut TestAppContext) {
        let rig = rig(cx, "", |lib, _| {
            install(lib, SHADOW_HM);
            install(lib, VANILLA);
        });
        let restore =
            |cx: &mut TestAppContext, key: &str| rig.act(cx, |it, cx| it.restore_hm(key, cx));
        restore(cx, VANILLA);
        assert_eq!(rig.seen(), [failed(NOT_INSTALLED_HM)]);
        rig.host.running.lock().unwrap().push(SHADOW.to_owned());
        restore(cx, SHADOW);
        assert_eq!(rig.seen(), [failed(SEASON_RUNNING)]);
        rig.host.running.lock().unwrap().clear();
        rig.launcher.update(cx, |it, cx| {
            it.restore_hm(SHADOW, cx);
            assert!(it.download.verifying());
            assert_eq!(it.download.state(), State::Applying);
        });
        cx.run_until_parked();
        assert_eq!(rig.seen(), [done(SHADOW, Outcome::Verified)]);
        assert_eq!(
            rig.calls(),
            ["apply_hm Y5S3_HeatedMetal ThrowbackUser 0.2.3 "]
        );

        *rig.host.hm.lock().unwrap() = Some(Fault::FilesMissing);
        restore(cx, SHADOW);
        let missing = DownloadEvent::FilesMissing {
            key: SHADOW.to_owned(),
            library: rig.lib.clone(),
        };
        assert_eq!(rig.seen(), [missing, done(SHADOW, Outcome::Failed)]);
        assert_eq!(rig.state(cx), State::Failed);

        *rig.host.hm.lock().unwrap() = Some(Fault::RateLimited(LIMIT.to_owned()));
        restore(cx, SHADOW);
        assert_eq!(
            rig.seen(),
            [
                DownloadEvent::RateLimited(LIMIT.into()),
                done(SHADOW, Outcome::Failed)
            ]
        );
        assert_eq!(rig.state(cx), State::Failed);
        assert!(!rig.store().queue_paused());
        rig.act(cx, |it, cx| it.update_hm(SHADOW, cx));
        rig.act(cx, |it, cx| it.update_hm(VANILLA, cx));
        assert_eq!(rig.seen(), [failed(NOT_INSTALLED_HM)]);
        assert!(!cx.did_prompt_for_paths());
    }

    #[gpui::test]
    fn removing_heated_metal_goes_back_to_throwback_through_a_verify(cx: &mut TestAppContext) {
        let settings = "home_order = ['Y5S3_HeatedMetal']\n";
        let rig = rig(cx, settings, |lib, _| {
            install(lib, SHADOW_HM);
            fs::create_dir_all(lib.join(SHADOW_HM).join("HeatedMetal")).unwrap();
            fs::write(lib.join(SHADOW_HM).join("DefaultArgs.dll"), "hm").unwrap();
            install(lib, VANILLA);
        });
        let remove =
            |cx: &mut TestAppContext, key: &str| rig.act(cx, |it, cx| it.remove_hm(key, cx));
        remove(cx, SHADOW);
        assert_eq!(rig.seen(), [login(LoginKind::Account)]);
        rig.act(cx, |it, cx| it.cancel_download(cx));
        rig.launcher
            .update(cx, |it, cx| it.settings.set_steam_account("ash", cx));
        remove(cx, VANILLA);
        assert_eq!(rig.seen(), [failed(NOT_INSTALLED_HM)]);
        install(&rig.other, SHADOW);
        remove(cx, SHADOW);
        assert_eq!(rig.seen(), [failed("Already installed as Throwback")]);
        fs::remove_file(rig.other.join(SHADOW).join(TL_LAUNCHER)).unwrap();
        remove(cx, SHADOW);
        assert_eq!(rig.seen(), [failed(PARTIAL_EXISTS)]);
        fs::remove_dir_all(rig.other.join(SHADOW)).unwrap();
        rig.host.running.lock().unwrap().push(SHADOW.to_owned());
        remove(cx, SHADOW);
        assert_eq!(rig.seen(), [failed(SEASON_RUNNING)]);
        rig.host.running.lock().unwrap().clear();

        let broken = "ThrowbackLoader download failed".to_owned();
        *rig.host.fetch.lock().unwrap() = Some(Fault::Message(broken));
        remove(cx, SHADOW);
        assert_eq!(
            rig.seen(),
            [
                failed("ThrowbackLoader download failed"),
                done(SHADOW, Outcome::Failed)
            ]
        );
        assert!(rig.lib.join(SHADOW_HM).join(TL_LAUNCHER).exists());
        assert_eq!(rig.calls(), ["fetch loader=true"]);

        rig.launcher.update(cx, |it, cx| {
            it.remove_hm(SHADOW, cx);
            assert_eq!(it.download.state(), State::Preparing);
            assert!(it.download.verifying());
            assert_eq!(it.download.active(), Some(&edition(SHADOW, false)));
        });
        cx.run_until_parked();
        let target = rig.lib.join(SHADOW);
        assert_eq!(rig.state(cx), State::Downloading);
        assert!(!rig.lib.join(SHADOW_HM).exists());
        assert!(target.is_dir());
        for gone in [TL_LAUNCHER, "HeatedMetal", "DefaultArgs.dll"] {
            assert!(!target.join(gone).exists(), "{gone}");
        }
        assert_eq!(rig.store().home_order(), [SHADOW]);
        assert!(rig.args(0).contains(&target.to_string_lossy().into_owned()));
        assert_eq!(rig.stored(), ["Y5S3_ShadowLegacy @lib verify"]);
        rig.close(cx, Exit::Code(0));
        rig.close(cx, Exit::Code(0));
        assert_eq!(rig.seen(), [done(SHADOW, Outcome::SwitchedTb)]);
        assert!(target.join(TL_LAUNCHER).exists());
        assert_eq!(rig.state(cx), State::Idle);
    }

    #[gpui::test]
    fn an_unstable_season_asks_for_its_archive_before_anything_starts(cx: &mut TestAppContext) {
        let rig = rig(cx, ACCOUNT, |lib, _| install(lib, BLOOD));
        let cached =
            |cx: &mut TestAppContext| rig.launcher.read_with(cx, |it, _| it.hm_archive_cached());
        let archive = rig.other.join("Unstable.7z");
        assert!(!cached(cx));
        rig.enqueue(cx, BLOOD, true, "");
        assert!(cx.did_prompt_for_paths());
        assert_eq!(rig.state(cx), State::Idle);
        rig.enqueue(cx, VANILLA, false, "");
        rig.act(cx, |it, cx| {
            it.pick_hm_archive(cx);
            it.start_next(cx);
        });
        pick(cx, None);
        assert!(!cx.did_prompt_for_paths());
        assert_eq!(rig.seen(), [DownloadEvent::HmArchivePicked(false)]);

        rig.act(cx, |it, cx| it.switch_to_hm(BLOOD, cx));
        let wrong = "Unstable.7z does not contain the Heated Metal files";
        *rig.host.archive.lock().unwrap() = Some(wrong.to_owned());
        pick(cx, Some(&archive));
        assert_eq!(
            rig.seen(),
            [failed(wrong), DownloadEvent::HmArchivePicked(false)]
        );
        assert_eq!((rig.state(cx), cached(cx)), (State::Idle, false));

        rig.enqueue(cx, BLOOD, true, "");
        pick(cx, Some(&archive));
        assert_eq!(rig.seen(), [DownloadEvent::HmArchivePicked(true)]);
        assert_eq!(rig.store().hm_archive(), "Unstable.7z");
        assert!(cached(cx));
        assert_eq!(rig.state(cx), State::Downloading);
        assert_eq!(rig.active(cx), Some(edition(BLOOD, true)));
        assert_eq!(rig.calls(), ["fetch loader=false"]);
        rig.close(cx, Exit::Code(0));
        rig.close(cx, Exit::Code(0));
        assert_eq!(rig.seen(), [done(BLOOD, Outcome::Done)]);
        assert_eq!(
            rig.calls(),
            ["apply_hm Y9S2_HeatedMetal ThrowbackUser unstable Unstable.7z"]
        );

        rig.act(cx, |it, cx| it.update_hm(BLOOD, cx));
        let newer = rig.other.join("Newer.7z");
        pick(cx, Some(&newer));
        assert_eq!(
            rig.seen(),
            [
                DownloadEvent::HmArchivePicked(true),
                done(BLOOD, Outcome::Updated)
            ]
        );
        assert_eq!(
            rig.calls(),
            ["apply_hm Y9S2_HeatedMetal ThrowbackUser unstable Newer.7z"]
        );
        rig.host.running.lock().unwrap().push(BLOOD.to_owned());
        rig.act(cx, |it, cx| it.update_hm(BLOOD, cx));
        assert_eq!(rig.seen(), [failed(SEASON_RUNNING)]);
        rig.act(cx, |it, cx| it.pick_hm_archive(cx));
        pick(cx, Some(&archive));
        assert_eq!(rig.seen(), [DownloadEvent::HmArchivePicked(true)]);
        assert_eq!(rig.store().hm_archive(), "Unstable.7z");
        assert_eq!(rig.state(cx), State::Idle);
        assert!(rig.lib.join(BLOOD_HM).join(TL_LAUNCHER).exists());
    }

    #[gpui::test]
    fn the_queue_goes_on_when_the_archive_dialog_closes(cx: &mut TestAppContext) {
        let stored = queue(&[(BLOOD, true, "", false), (VANILLA, false, "", false)]);
        let rig = rig(cx, &format!("{ACCOUNT}{stored}"), |_, _| {});
        rig.act(cx, |it, cx| it.start_next(cx));
        assert!(cx.did_prompt_for_paths());
        assert_eq!(rig.state(cx), State::Idle);
        assert_eq!(rig.queued(cx), [edition(VANILLA, false)]);
        pick(cx, None);
        assert_eq!(rig.seen(), [DownloadEvent::HmArchivePicked(false)]);
        assert_eq!(rig.state(cx), State::Downloading);
        assert_eq!(rig.active(cx), Some(edition(VANILLA, false)));

        rig.enqueue(cx, ICE, false, "");
        rig.act(cx, |it, cx| {
            it.pick_hm_archive(cx);
            it.cancel_download(cx);
        });
        assert_eq!(rig.seen(), [done(VANILLA, Outcome::Cancelled)]);
        assert_eq!(rig.state(cx), State::Idle);
        assert_eq!(rig.queued(cx), [edition(ICE, false)]);
        pick(cx, Some(&rig.other.join("Unstable.7z")));
        assert_eq!(rig.seen(), [DownloadEvent::HmArchivePicked(true)]);
        assert_eq!(rig.state(cx), State::Downloading);
        assert_eq!(rig.active(cx), Some(edition(ICE, false)));

        rig.enqueue(cx, SHADOW, false, "");
        rig.act(cx, |it, cx| {
            it.download.picking = true;
            it.cancel_download(cx);
        });
        assert_eq!(rig.seen(), [done(ICE, Outcome::Cancelled)]);
        assert_eq!(rig.queued(cx), [edition(SHADOW, false)]);
        rig.act(cx, |it, cx| {
            it.hm_archive_selected(Err("File dialog failed".into()), None, cx);
        });
        assert_eq!(
            rig.seen(),
            [
                failed("File dialog failed"),
                DownloadEvent::HmArchivePicked(false)
            ]
        );
        assert_eq!(rig.state(cx), State::Downloading);
        assert_eq!(rig.active(cx), Some(edition(SHADOW, false)));
    }

    #[gpui::test]
    fn update_hm_checks_the_blockers_again_after_the_archive_dialog(cx: &mut TestAppContext) {
        let rig = rig(cx, ACCOUNT, |lib, _| install(lib, BLOOD_HM));
        let newer = rig.other.join("Newer.7z");
        rig.act(cx, |it, cx| it.update_hm(BLOOD, cx));
        rig.launcher
            .update(cx, |it, _| it.update = Updater::busy_now());
        pick(cx, Some(&newer));
        assert_eq!(
            rig.seen(),
            [failed(UPDATE_RUNNING), DownloadEvent::HmArchivePicked(true)]
        );
        rig.launcher
            .update(cx, |it, _| it.update = Updater::default());
        rig.act(cx, |it, cx| it.update_hm(BLOOD, cx));
        rig.launcher
            .update(cx, |it, _| it.uninstall = Uninstaller::busy_with(BLOOD));
        pick(cx, Some(&newer));
        assert_eq!(
            rig.seen(),
            [
                failed(UNINSTALL_RUNNING),
                DownloadEvent::HmArchivePicked(true)
            ]
        );
        assert_eq!(rig.state(cx), State::Idle);
        assert!(rig.calls().is_empty());
        assert_eq!(rig.store().hm_archive(), "Newer.7z");
    }

    #[gpui::test]
    fn libraries_in_use_are_those_of_the_active_and_the_queued_editions(cx: &mut TestAppContext) {
        let rig = rig(cx, "", |_, _| {});
        let other = rig.other.to_string_lossy().into_owned();
        let uses = |cx: &mut TestAppContext, root: &Path| {
            rig.launcher.read_with(cx, |it, _| {
                it.download.uses_library(root, it.settings.roots())
            })
        };
        assert!(!uses(cx, &rig.lib) && !uses(cx, &rig.other));
        rig.launcher.update(cx, |it, cx| {
            let download = &mut it.download;
            download.state = State::Downloading;
            it.enqueue(ICE, false, &other, cx);
        });
        assert!(uses(cx, &rig.other) && !uses(cx, &rig.lib));
        rig.act(cx, |it, cx| it.enqueue(SHADOW, false, "", cx));
        assert!(uses(cx, &rig.lib));
        rig.act(cx, |it, cx| it.dequeue(SHADOW, false, cx));
        assert!(!uses(cx, &rig.lib));
        partial(&rig.lib, ICE);
        assert!(uses(cx, &rig.lib) && !uses(cx, &rig.other));
    }

    #[gpui::test]
    fn shutdown_kills_depotdownloader_and_leaves_the_stored_queue(cx: &mut TestAppContext) {
        let rig = rig(cx, ACCOUNT, |_, _| {});
        rig.download(cx, VANILLA);
        rig.enqueue(cx, ICE, false, "");
        rig.act(cx, |it, _| it.download.shutdown());
        assert!(rig.killed());
        assert_eq!(rig.typed(), ["waited 2"]);
        assert!(rig.seen().is_empty());
        assert_eq!(rig.state(cx), State::Downloading);
        assert_eq!(
            rig.stored(),
            ["Y1S0_Vanilla @lib download", "Y1S1_BlackIce @ download"]
        );
    }
}
