use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use async_channel::Sender;
use backend::env::Paths;
use backend::github::CancelledError;
use backend::log;
use backend::rvpn::{
    Reporter, Session as WineSession, ensure_wine, is_installed, is_radmin_installer, preflight,
    service_version, uninstall,
};
use backend::settings::Flag;
use gpui::{AppContext, Context, EventEmitter, Result, SharedString};

use super::Launcher;
use super::dialogs::pick_file;

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

pub enum RvpnEvent {
    Error(SharedString),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Building,
    Running,
    Uninstalling,
}

trait Host: Send + Sync {
    fn installed(&self) -> bool;
    fn version(&self) -> String;
    fn prepare(&self, reporter: &Reporter, cancelled: &dyn Fn() -> bool) -> Result<()>;
    fn session(&self) -> Arc<dyn Session>;
    fn uninstall(&self) -> Result<()>;
}

trait Session: Send + Sync {
    fn run(
        &self,
        installer: Option<&Path>,
        reporter: &Reporter,
        on_running: &dyn Fn(),
    ) -> Result<()>;
    fn stop(&self);
}

struct Wine(Paths);

impl Host for Wine {
    fn installed(&self) -> bool {
        is_installed(&self.0)
    }

    fn version(&self) -> String {
        service_version(&self.0)
    }

    fn prepare(&self, reporter: &Reporter, cancelled: &dyn Fn() -> bool) -> Result<()> {
        preflight()?;
        ensure_wine(&self.0, reporter, cancelled)
    }

    fn session(&self) -> Arc<dyn Session> {
        Arc::new(WineSession::new(&self.0))
    }

    fn uninstall(&self) -> Result<()> {
        uninstall(&self.0)
    }
}

impl Session for WineSession {
    fn run(
        &self,
        installer: Option<&Path>,
        reporter: &Reporter,
        on_running: &dyn Fn(),
    ) -> Result<()> {
        WineSession::run(self, installer, reporter, on_running)
    }

    fn stop(&self) {
        WineSession::stop(self);
    }
}

enum Message {
    Step(String),
    Error(String),
    Running,
}

struct Job {
    cancel: Arc<AtomicBool>,
    session: Option<Arc<dyn Session>>,
    ended: Receiver<()>,
}

pub struct Rvpn {
    host: Arc<dyn Host>,
    state: State,
    installed: bool,
    version: SharedString,
    step: SharedString,
    installer: Option<PathBuf>,
    job: Option<Job>,
}

impl EventEmitter<RvpnEvent> for Launcher {}

fn reporting(
    messages: &Sender<Message>,
    wrap: fn(String) -> Message,
) -> Box<dyn Fn(&str) + Send + Sync> {
    let messages = messages.clone();
    Box::new(move |text| {
        messages.try_send(wrap(text.to_owned())).ok();
    })
}

impl Rvpn {
    pub fn new(paths: &Paths) -> Self {
        Self::on(Arc::new(Wine(paths.clone())))
    }

    fn on(host: Arc<dyn Host>) -> Self {
        Self {
            state: State::Idle,
            installed: host.installed(),
            version: host.version().into(),
            step: SharedString::default(),
            installer: None,
            job: None,
            host,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn installed(&self) -> bool {
        self.installed
    }

    pub fn version(&self) -> SharedString {
        self.version.clone()
    }

    pub fn step(&self) -> SharedString {
        self.step.clone()
    }

    pub fn select_installer(&self, cx: &mut Context<Launcher>) {
        let picked = pick_file("Select Radmin VPN installer", cx);
        cx.spawn(async move |this, cx| {
            let installer = match picked.await {
                Ok(Some(installer)) => installer,
                Ok(None) => return,
                Err(message) => {
                    this.update(cx, |_, cx| cx.emit(RvpnEvent::Error(message)))
                        .ok();
                    return;
                }
            };
            let file = installer.clone();
            let valid = cx
                .background_spawn(async move { is_radmin_installer(&file) })
                .await;
            this.update(cx, |this, cx| {
                if valid {
                    this.rvpn.installer = Some(installer);
                    this.rvpn.run(cx);
                } else {
                    cx.emit(RvpnEvent::Error("Radmin VPN installer not found".into()));
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn run(&mut self, cx: &mut Context<Launcher>) {
        if self.job.is_some() {
            return;
        }
        let (host, session) = (self.host.clone(), self.host.session());
        let (worker, installer) = (session.clone(), self.installer.clone());
        let work = move |messages: Sender<Message>, cancel: &AtomicBool| {
            let reporter = Reporter {
                fail: reporting(&messages, Message::Error),
                update: reporting(&messages, Message::Step),
            };
            let cancelled = || cancel.load(Ordering::Relaxed);
            let outcome = host.prepare(&reporter, &cancelled).and_then(|()| {
                if cancelled() {
                    return Ok(());
                }
                worker.run(installer.as_deref(), &reporter, &|| {
                    messages.try_send(Message::Running).ok();
                })
            });
            if let Err(error) = outcome
                && !error.is::<CancelledError>()
            {
                messages.try_send(Message::Error(error.to_string())).ok();
            }
        };
        self.start(State::Building, Some(session), work, cx);
    }

    pub fn stop(&self) {
        let Some(job) = &self.job else {
            return;
        };
        job.cancel.store(true, Ordering::Relaxed);
        if let Some(session) = &job.session {
            session.stop();
        }
    }

    pub fn shutdown(&self) {
        self.stop();
        if let Some(job) = &self.job {
            job.ended.recv_timeout(SHUTDOWN_TIMEOUT).ok();
        }
    }

    pub fn uninstall(&mut self, cx: &mut Context<Launcher>) {
        if self.job.is_some() {
            return;
        }
        let host = self.host.clone();
        let work = move |messages: Sender<Message>, _: &AtomicBool| {
            if let Err(error) = host.uninstall() {
                let message = log::fail("Radmin VPN removal failed", format!("{error:#}"));
                messages.try_send(Message::Error(message)).ok();
            }
        };
        self.start(State::Uninstalling, None, work, cx);
    }

    fn start(
        &mut self,
        state: State,
        session: Option<Arc<dyn Session>>,
        work: impl FnOnce(Sender<Message>, &AtomicBool) + Send + 'static,
        cx: &mut Context<Launcher>,
    ) {
        let (messages, inbox) = async_channel::unbounded();
        let (alive, ended) = mpsc::channel::<()>();
        let cancel = Arc::new(AtomicBool::new(false));
        self.job = Some(Job {
            cancel: cancel.clone(),
            session,
            ended,
        });
        self.enter(state, cx);
        thread::spawn(move || {
            let _alive = alive;
            work(messages, &cancel);
        });
        cx.spawn(async move |this, cx| {
            while let Ok(message) = inbox.recv().await {
                let applied = this.update(cx, |this, cx| this.rvpn.apply(message, cx));
                if applied.is_err() {
                    return;
                }
            }
            this.update(cx, |this, cx| this.rvpn.idle(cx)).ok();
        })
        .detach();
    }

    fn enter(&mut self, state: State, cx: &mut Context<Launcher>) {
        self.state = state;
        self.installed = self.host.installed();
        self.version = self.host.version().into();
        self.step = SharedString::default();
        cx.notify();
    }

    fn apply(&mut self, message: Message, cx: &mut Context<Launcher>) {
        match message {
            Message::Step(text) => {
                self.step = text.into();
                cx.notify();
            }
            Message::Error(text) => {
                let stopped = |job: &Job| job.cancel.load(Ordering::Relaxed);
                if !self.job.as_ref().is_some_and(stopped) {
                    cx.emit(RvpnEvent::Error(text.into()));
                }
            }
            Message::Running => self.enter(State::Running, cx),
        }
    }

    fn idle(&mut self, cx: &mut Context<Launcher>) {
        self.job = None;
        self.enter(State::Idle, cx);
        if !self.installed {
            self.installer = None;
        }
    }
}

#[cfg(feature = "qa")]
impl Rvpn {
    pub fn stage(&mut self, plan: &str) {
        self.installed = true;
        for (name, value) in plan.split(',').filter_map(|part| part.split_once('=')) {
            match name {
                "state" => {
                    self.state = match value {
                        "building" => State::Building,
                        "running" => State::Running,
                        "uninstalling" => State::Uninstalling,
                        _ => State::Idle,
                    }
                }
                "installed" => self.installed = value == "1",
                "version" => self.version = value.to_owned().into(),
                "step" => self.step = value.to_owned().into(),
                _ => {}
            }
        }
    }
}

impl Launcher {
    pub fn autorun_rvpn(&mut self, cx: &mut Context<Self>) {
        if self.settings.flag(Flag::RvpnAutorun) && self.rvpn.installed {
            self.rvpn.run(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::io;
    use std::rc::Rc;
    use std::sync::Mutex;
    use std::time::Instant;

    use backend::settings::Settings as Store;
    use gpui::{Entity, TestAppContext};

    use super::*;

    const PATIENCE: Duration = Duration::from_secs(10);

    enum Cue {
        Step(&'static str),
        Fail(&'static str),
        Running,
        End(Result<()>),
    }

    struct Script {
        installed: AtomicBool,
        version: Mutex<&'static str>,
        calls: Mutex<Vec<String>>,
        cues: Mutex<Receiver<Cue>>,
        played: mpsc::Sender<()>,
    }

    #[derive(Clone)]
    struct Fake(Arc<Script>);

    impl Fake {
        fn call(&self, name: impl Into<String>) {
            self.0.calls.lock().unwrap().push(name.into());
        }

        fn play(&self, reporter: Option<&Reporter>, on_running: Option<&dyn Fn()>) -> Result<()> {
            loop {
                let Ok(cue) = self.0.cues.lock().unwrap().recv() else {
                    return Ok(());
                };
                let end = match cue {
                    Cue::Step(text) => {
                        (reporter.unwrap().update)(text);
                        None
                    }
                    Cue::Fail(text) => {
                        (reporter.unwrap().fail)(text);
                        None
                    }
                    Cue::Running => {
                        on_running.unwrap()();
                        None
                    }
                    Cue::End(outcome) => Some(outcome),
                };
                self.0.played.send(()).ok();
                if let Some(outcome) = end {
                    return outcome;
                }
            }
        }
    }

    impl Host for Fake {
        fn installed(&self) -> bool {
            self.0.installed.load(Ordering::Relaxed)
        }

        fn version(&self) -> String {
            self.0.version.lock().unwrap().to_string()
        }

        fn prepare(&self, reporter: &Reporter, cancelled: &dyn Fn() -> bool) -> Result<()> {
            self.call("prepare");
            let outcome = self.play(Some(reporter), None);
            if cancelled() {
                self.call("cancelled");
            }
            outcome
        }

        fn session(&self) -> Arc<dyn Session> {
            Arc::new(self.clone())
        }

        fn uninstall(&self) -> Result<()> {
            self.call("uninstall");
            self.play(None, None)
        }
    }

    impl Session for Fake {
        fn run(
            &self,
            installer: Option<&Path>,
            reporter: &Reporter,
            on_running: &dyn Fn(),
        ) -> Result<()> {
            let installer = installer.and_then(Path::file_name);
            self.call(format!("run {installer:?}"));
            self.play(Some(reporter), Some(on_running))
        }

        fn stop(&self) {
            self.call("stop");
        }
    }

    struct Stage {
        launcher: Entity<Launcher>,
        script: Arc<Script>,
        cues: mpsc::Sender<Cue>,
        played: Receiver<()>,
        errors: Rc<RefCell<Vec<String>>>,
    }

    impl Stage {
        fn new(cx: &mut TestAppContext, dir: &Path, settings: &str, installed: bool) -> Self {
            cx.executor().allow_parking();
            let paths = Paths::new(dir.join("asset"), dir.join("data"));
            fs::create_dir_all(&paths.data_root).unwrap();
            fs::write(paths.settings_file(), settings).unwrap();
            let store = Store::load(&paths).unwrap();
            let (cues, inbox) = mpsc::channel();
            let (played, acks) = mpsc::channel();
            let script = Arc::new(Script {
                installed: AtomicBool::new(installed),
                version: Mutex::new("2.1.4951.1"),
                calls: Mutex::default(),
                cues: Mutex::new(inbox),
                played,
            });
            let launcher = cx.new(|_| {
                let mut launcher = Launcher::new(paths, store, Vec::new());
                launcher.rvpn = Rvpn::on(Arc::new(Fake(script.clone())));
                launcher
            });
            let errors = Rc::new(RefCell::new(Vec::new()));
            cx.update(|cx| {
                let seen = errors.clone();
                cx.subscribe(&launcher, move |_, event: &RvpnEvent, _| {
                    let RvpnEvent::Error(text) = event;
                    seen.borrow_mut().push(text.to_string());
                })
                .detach();
            });
            Self {
                launcher,
                script,
                cues,
                played: acks,
                errors,
            }
        }

        fn rvpn<R>(
            &self,
            cx: &mut TestAppContext,
            act: impl FnOnce(&mut Rvpn, &mut Context<Launcher>) -> R,
        ) -> R {
            self.launcher
                .update(cx, |launcher, cx| act(&mut launcher.rvpn, cx))
        }

        fn cue(&self, cx: &mut TestAppContext, cue: Cue) {
            self.cues.send(cue).unwrap();
            self.played.recv_timeout(PATIENCE).unwrap();
            cx.run_until_parked();
        }

        fn shown(&self, cx: &mut TestAppContext) -> (State, bool, String, String) {
            self.launcher.read_with(cx, |launcher, _| {
                let rvpn = &launcher.rvpn;
                let (step, version) = (rvpn.step().to_string(), rvpn.version().to_string());
                (rvpn.state(), rvpn.installed(), step, version)
            })
        }

        fn settle(&self, cx: &mut TestAppContext) {
            self.rvpn(cx, |rvpn, _| {
                if let Some(job) = &rvpn.job {
                    let ended = job.ended.recv_timeout(PATIENCE);
                    let gone = Err(mpsc::RecvTimeoutError::Disconnected);
                    assert_eq!(ended, gone, "the worker never ended");
                }
            });
            cx.run_until_parked();
            assert_eq!(self.shown(cx).0, State::Idle);
        }

        fn calls(&self) -> Vec<String> {
            self.script.calls.lock().unwrap().clone()
        }

        fn errors(&self) -> Vec<String> {
            self.errors.borrow().clone()
        }
    }

    fn failure(text: &'static str) -> Cue {
        Cue::End(Err(io::Error::other(text).into()))
    }

    fn shown(
        state: State,
        installed: bool,
        step: &str,
        version: &str,
    ) -> (State, bool, String, String) {
        (state, installed, step.to_owned(), version.to_owned())
    }

    #[gpui::test]
    fn a_run_shows_its_steps_and_returns_to_idle(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let stage = Stage::new(cx, dir.path(), "", true);
        assert_eq!(stage.shown(cx), shown(State::Idle, true, "", "2.1.4951.1"));

        stage.rvpn(cx, |rvpn, cx| {
            rvpn.run(cx);
            rvpn.run(cx);
            rvpn.uninstall(cx);
        });
        assert_eq!(stage.shown(cx).0, State::Building);
        stage.cue(cx, Cue::Step("Fetching Wine"));
        assert_eq!(
            stage.shown(cx),
            shown(State::Building, true, "Fetching Wine", "2.1.4951.1")
        );
        stage.cue(cx, Cue::End(Ok(())));
        stage.cue(cx, Cue::Step("Running daemon"));
        assert_eq!(stage.shown(cx).2, "Running daemon");

        *stage.script.version.lock().unwrap() = "2.1.4951.2";
        stage.cue(cx, Cue::Running);
        assert_eq!(
            stage.shown(cx),
            shown(State::Running, true, "", "2.1.4951.2")
        );
        stage.rvpn(cx, |rvpn, cx| rvpn.run(cx));

        stage.cue(cx, Cue::End(Ok(())));
        stage.settle(cx);
        assert_eq!(stage.shown(cx), shown(State::Idle, true, "", "2.1.4951.2"));
        assert_eq!(stage.calls(), ["prepare", "run None"]);
        assert!(stage.errors().is_empty());
    }

    #[gpui::test]
    fn an_error_is_shown_at_once_unless_the_user_stopped(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let stage = Stage::new(cx, dir.path(), "", true);
        stage.rvpn(cx, |rvpn, cx| rvpn.run(cx));
        stage.cue(cx, Cue::End(Ok(())));
        stage.cue(cx, Cue::Fail("Network setup failed"));
        assert_eq!(stage.errors(), ["Network setup failed"]);
        assert_eq!(stage.shown(cx).0, State::Building);
        stage.cue(cx, Cue::End(Ok(())));
        stage.settle(cx);

        stage.rvpn(cx, |rvpn, cx| rvpn.run(cx));
        stage.cue(cx, failure("iproute2 not found"));
        stage.settle(cx);
        assert_eq!(
            stage.errors(),
            ["Network setup failed", "iproute2 not found"]
        );
        assert_eq!(stage.calls(), ["prepare", "run None", "prepare"]);

        stage.rvpn(cx, |rvpn, cx| rvpn.run(cx));
        stage.cue(cx, Cue::End(Ok(())));
        stage.cue(cx, Cue::Running);
        stage.rvpn(cx, |rvpn, _| rvpn.stop());
        assert_eq!(stage.calls().last().unwrap(), "stop");
        stage.cue(cx, Cue::Fail("Radmin VPN stopped unexpectedly"));
        stage.cue(cx, failure("late"));
        stage.settle(cx);
        assert_eq!(stage.errors().len(), 2);

        stage.rvpn(cx, |rvpn, cx| rvpn.run(cx));
        stage.cue(cx, Cue::End(Err(CancelledError.into())));
        stage.settle(cx);
        assert_eq!(stage.errors().len(), 2);
        stage.rvpn(cx, |rvpn, cx| rvpn.run(cx));
        stage.cue(cx, failure("pkexec not found, a polkit agent is required"));
        stage.settle(cx);
        assert_eq!(stage.errors().len(), 3, "a new run shows errors again");
    }

    #[gpui::test]
    fn a_stop_before_the_session_keeps_it_from_starting(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let stage = Stage::new(cx, dir.path(), "", true);
        stage.rvpn(cx, |rvpn, _| rvpn.stop());
        stage.rvpn(cx, |rvpn, cx| rvpn.run(cx));
        stage.cue(cx, Cue::Step("Fetching Wine"));
        stage.rvpn(cx, |rvpn, _| rvpn.stop());
        stage.cue(cx, Cue::End(Ok(())));
        stage.settle(cx);
        assert_eq!(stage.calls(), ["prepare", "stop", "cancelled"]);
        assert!(stage.errors().is_empty());
    }

    #[gpui::test]
    fn a_picked_installer_is_checked_used_and_forgotten(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let stage = Stage::new(cx, dir.path(), "", false);
        let (wrong, setup) = (dir.path().join("wrong.exe"), dir.path().join("setup.exe"));
        fs::write(&wrong, "MZ Radmin Viewer").unwrap();
        fs::write(&setup, "MZ Radmin VPN").unwrap();
        let pick = |cx: &mut TestAppContext, file: Option<&PathBuf>| {
            stage.rvpn(cx, |rvpn, cx| rvpn.select_installer(cx));
            cx.simulate_path_prompt_response(|options| {
                assert!(options.files && !options.directories && !options.multiple);
                assert_eq!(
                    options.prompt.as_deref(),
                    Some("Select Radmin VPN installer")
                );
                file.map(|file| vec![file.clone()])
            });
            cx.run_until_parked();
        };
        let end = |cx: &mut TestAppContext, installed: bool| {
            stage.cue(cx, Cue::End(Ok(())));
            stage.script.installed.store(installed, Ordering::Relaxed);
            stage.cue(cx, Cue::End(Ok(())));
            stage.settle(cx);
            assert_eq!(stage.shown(cx).1, installed);
        };

        pick(cx, None);
        assert!(stage.errors().is_empty());
        pick(cx, Some(&wrong));
        assert_eq!(stage.errors(), ["Radmin VPN installer not found"]);
        assert_eq!(stage.shown(cx).0, State::Idle);
        assert!(stage.calls().is_empty());

        pick(cx, Some(&setup));
        assert_eq!(
            stage.shown(cx),
            shown(State::Building, false, "", "2.1.4951.1")
        );
        end(cx, false);
        stage.rvpn(cx, |rvpn, cx| rvpn.run(cx));
        end(cx, false);

        pick(cx, Some(&setup));
        end(cx, true);
        stage.rvpn(cx, |rvpn, cx| rvpn.run(cx));
        end(cx, true);

        stage.rvpn(cx, |rvpn, cx| rvpn.uninstall(cx));
        stage.script.installed.store(false, Ordering::Relaxed);
        stage.cue(cx, Cue::End(Ok(())));
        stage.settle(cx);
        stage.rvpn(cx, |rvpn, cx| rvpn.run(cx));
        end(cx, false);

        let mut runs = stage.calls();
        runs.retain(|call| call.starts_with("run"));
        assert_eq!(
            runs,
            [
                "run Some(\"setup.exe\")",
                "run None",
                "run Some(\"setup.exe\")",
                "run Some(\"setup.exe\")",
                "run None",
            ]
        );
        assert_eq!(stage.errors().len(), 1);
    }

    #[gpui::test]
    fn an_uninstall_blocks_other_jobs_and_names_its_failure(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let stage = Stage::new(cx, dir.path(), "", true);
        stage.rvpn(cx, |rvpn, cx| {
            rvpn.uninstall(cx);
            rvpn.uninstall(cx);
            rvpn.run(cx);
        });
        assert_eq!(stage.shown(cx).0, State::Uninstalling);
        stage.script.installed.store(false, Ordering::Relaxed);
        stage.cue(cx, Cue::End(Ok(())));
        stage.settle(cx);
        assert_eq!(stage.shown(cx), shown(State::Idle, false, "", "2.1.4951.1"));
        assert!(stage.errors().is_empty());

        stage.rvpn(cx, |rvpn, cx| rvpn.uninstall(cx));
        stage.cue(cx, failure("pkill: No such file or directory"));
        stage.settle(cx);
        assert_eq!(stage.errors(), ["Radmin VPN removal failed"]);
        assert_eq!(stage.calls(), ["uninstall", "uninstall"]);
    }

    #[gpui::test]
    fn autorun_needs_the_setting_and_an_installation(cx: &mut TestAppContext) {
        let autorun = "[settings]\nrvpn_autorun = true\n";
        for (settings, installed, runs) in [
            ("", true, false),
            (autorun, false, false),
            (autorun, true, true),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let stage = Stage::new(cx, dir.path(), settings, installed);
            stage
                .launcher
                .update(cx, |launcher, cx| launcher.autorun_rvpn(cx));
            let state = stage.shown(cx).0;
            assert_eq!(state == State::Building, runs, "{settings:?} {installed}");
            if runs {
                stage.cue(cx, Cue::End(Ok(())));
                stage.cue(cx, Cue::End(Ok(())));
                stage.settle(cx);
            }
        }
    }

    #[gpui::test]
    fn shutdown_stops_the_session_and_waits_for_the_worker(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let stage = Stage::new(cx, dir.path(), "", true);
        stage.rvpn(cx, |rvpn, _| rvpn.shutdown());
        stage.rvpn(cx, |rvpn, cx| rvpn.run(cx));
        stage.cue(cx, Cue::End(Ok(())));
        stage.cue(cx, Cue::Running);

        let (script, cues) = (stage.script.clone(), stage.cues.clone());
        let teardown = Duration::from_millis(60);
        let session = thread::spawn(move || {
            while script.calls.lock().unwrap().last().unwrap() != "stop" {
                thread::sleep(Duration::from_millis(1));
            }
            thread::sleep(teardown);
            cues.send(Cue::End(Ok(()))).unwrap();
        });
        let started = Instant::now();
        stage.rvpn(cx, |rvpn, _| rvpn.shutdown());
        assert!(started.elapsed() >= teardown);
        assert!(started.elapsed() < SHUTDOWN_TIMEOUT);
        session.join().unwrap();
        stage.settle(cx);
        assert_eq!(stage.calls(), ["prepare", "run None", "stop"]);
    }
}
