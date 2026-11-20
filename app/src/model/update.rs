use std::sync::Arc;
use std::time::Duration;

use backend::env::{APP_NAME, CACHE_CLEARING, DOWNLOAD_RUNNING, Paths, UNINSTALL_RUNNING};
use backend::self_update::take_outcome;
use backend::update::{self, Check, Component, Failure, Install, Pending, Release};
use gpui::{App, AppContext, Context, EventEmitter, SharedString, Task};

use super::Launcher;

const RESTART_DELAY: Duration = Duration::from_millis(800);

pub enum UpdateEvent {
    Done(Component, Result<(), SharedString>),
    Error(&'static str),
}

impl EventEmitter<UpdateEvent> for Launcher {}

pub(crate) trait Work: Send + Sync {
    fn available(&self, install: &Install, force: bool) -> Check;

    fn apply(
        &self,
        install: &Install,
        component: Component,
        progress: &dyn Fn(f64),
    ) -> Result<(), String>;

    fn relaunch(&self, paths: &Paths);
}

struct Live;

impl Work for Live {
    fn available(&self, install: &Install, force: bool) -> Check {
        update::available(install, force)
    }

    fn apply(
        &self,
        install: &Install,
        component: Component,
        progress: &dyn Fn(f64),
    ) -> Result<(), String> {
        let applied = component.apply(install, Some(progress));
        applied.map_err(|error| error.to_string())
    }

    fn relaunch(&self, paths: &Paths) {
        update::relaunch(paths);
    }
}

pub struct Updater {
    checking: bool,
    pending: Vec<Pending>,
    failure: Option<Failure>,
    history: Vec<(Component, Vec<Release>)>,
    progress: u8,
    applying: Option<Component>,
    work: Arc<dyn Work>,
}

impl Default for Updater {
    fn default() -> Self {
        Self {
            checking: false,
            pending: Vec::new(),
            failure: None,
            history: Vec::new(),
            progress: 0,
            applying: None,
            work: Arc::new(Live),
        }
    }
}

impl Updater {
    pub fn busy(&self) -> bool {
        self.applying.is_some()
    }

    pub fn checking(&self) -> bool {
        self.checking
    }

    pub fn pending(&self) -> &[Pending] {
        &self.pending
    }

    pub fn failure(&self) -> Option<&Failure> {
        self.failure.as_ref()
    }

    pub fn history(&self) -> &[(Component, Vec<Release>)] {
        &self.history
    }

    pub fn progress(&self) -> u8 {
        self.progress
    }

    pub fn applying(&self) -> Option<Component> {
        self.applying
    }

    fn set_progress(&mut self, fraction: f64, cx: &mut Context<Launcher>) {
        let percent = (fraction * 100.) as u8;
        if percent != self.progress {
            self.progress = percent;
            cx.notify();
        }
    }
}

fn install(launcher: &Launcher) -> Install {
    Install {
        paths: launcher.paths.clone(),
        libraries: launcher.settings.roots().to_vec(),
        downloads: launcher.downloads.clone(),
        username: launcher.settings.username().to_string(),
    }
}

impl Launcher {
    pub fn check_updates(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.update.busy() || self.update.checking {
            return;
        }
        self.update.checking = true;
        cx.notify();
        let (work, install) = (self.update.work.clone(), install(self));
        let check = cx.background_spawn(async move { work.available(&install, force) });
        cx.spawn(async move |this, cx| {
            let check = check.await;
            this.update(cx, |this, cx| {
                if check.failure.is_none() || !check.pending.is_empty() {
                    this.update.pending = check.pending;
                }
                if check
                    .history
                    .iter()
                    .any(|(_, releases)| !releases.is_empty())
                {
                    this.update.history = check.history;
                }
                this.update.failure = check.failure;
                this.update.checking = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn apply_update(&mut self, component: Component, cx: &mut Context<Self>) {
        let mut pending = self.update.pending.iter();
        let listed = pending.any(|update| update.component == component);
        if self.update.busy() || self.update.checking || !listed {
            return;
        }
        if self.settings.clearing_cache() {
            cx.emit(UpdateEvent::Error(CACHE_CLEARING));
            return;
        }
        if self.download.running() {
            cx.emit(UpdateEvent::Error(DOWNLOAD_RUNNING));
            return;
        }
        if self.uninstall.busy_key().is_some() {
            cx.emit(UpdateEvent::Error(UNINSTALL_RUNNING));
            return;
        }
        self.update.applying = Some(component);
        self.update.progress = 0;
        cx.notify();
        let (work, install) = (self.update.work.clone(), install(self));
        let (sender, progress) = async_channel::unbounded();
        let apply = cx.background_spawn(async move {
            work.apply(&install, component, &|fraction| {
                let _ = sender.try_send(fraction);
            })
        });
        cx.spawn(async move |this, cx| {
            while let Ok(fraction) = progress.recv().await {
                this.update(cx, |this, cx| this.update.set_progress(fraction, cx))
                    .ok();
            }
            let outcome = apply.await;
            this.update(cx, |this, cx| this.update_applied(component, outcome, cx))
                .ok();
        })
        .detach();
    }

    pub fn installed_versions(&self, cx: &App) -> Task<Vec<(Component, String)>> {
        let install = install(self);
        cx.background_spawn(async move { update::installed(&install) })
    }

    pub fn announce_update_outcome(&self, cx: &mut Context<Self>) {
        let Some(ok) = take_outcome(&self.paths) else {
            return;
        };
        let outcome = if ok {
            Ok(())
        } else {
            Err(format!("{APP_NAME} update failed").into())
        };
        cx.emit(UpdateEvent::Done(Component::Launcher, outcome));
    }

    fn update_applied(
        &mut self,
        component: Component,
        outcome: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        if outcome.is_ok() && component.restarts() {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(RESTART_DELAY).await;
                this.update(cx, |this, cx| {
                    this.update.work.relaunch(&this.paths);
                    cx.quit();
                })
                .ok();
            })
            .detach();
            return;
        }
        if outcome.is_ok() {
            let pending = &mut self.update.pending;
            pending.retain(|update| update.component != component);
        }
        self.update.applying = None;
        self.start_next(cx);
        self.check_updates(false, cx);
        cx.emit(UpdateEvent::Done(
            component,
            outcome.map_err(SharedString::from),
        ));
    }
}

#[cfg(test)]
impl Updater {
    pub fn busy_now() -> Self {
        Self {
            applying: Some(Component::SevenZip),
            ..Self::default()
        }
    }

    pub fn with_work(work: Arc<dyn Work>) -> Self {
        Self {
            work,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::fs;
    use std::rc::Rc;
    use std::sync::Mutex;

    use backend::env::VERSION;
    use backend::self_update::write_outcome;
    use backend::settings::Settings as Store;
    use backend::update::ReleaseNotes;
    use gpui::{Entity, TestAppContext};
    use tempfile::TempDir;

    use super::*;
    use crate::model::download::{Downloader, State};
    use crate::model::uninstall::Uninstaller;

    type Apply = (Vec<f64>, Result<(), String>);

    #[derive(Default)]
    struct Script {
        checks: Mutex<VecDeque<Check>>,
        applies: Mutex<VecDeque<Apply>>,
        calls: Mutex<Vec<String>>,
    }

    impl Script {
        fn call(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }
    }

    impl Work for Script {
        fn available(&self, install: &Install, force: bool) -> Check {
            self.call(format!(
                "check force={force} user={} libraries={}",
                install.username,
                install.libraries.len()
            ));
            self.checks.lock().unwrap().pop_front().unwrap_or_default()
        }

        fn apply(
            &self,
            _install: &Install,
            component: Component,
            report: &dyn Fn(f64),
        ) -> Result<(), String> {
            self.call(format!("apply {}", component.name()));
            let (progress, outcome) = self.applies.lock().unwrap().pop_front().unwrap();
            for fraction in progress {
                report(fraction);
            }
            outcome
        }

        fn relaunch(&self, _paths: &Paths) {
            self.call("relaunch".to_owned());
        }
    }

    struct Bench {
        _dir: TempDir,
        paths: Paths,
        launcher: Entity<Launcher>,
        script: Arc<Script>,
        seen: Rc<RefCell<Vec<String>>>,
    }

    fn state(update: &Updater) -> String {
        let phase = match (update.applying(), update.checking()) {
            (Some(component), _) => format!("applying {} {}%", component.name(), update.progress()),
            (None, true) => "checking".to_owned(),
            (None, false) => "idle".to_owned(),
        };
        let pending: Vec<&str> = update
            .pending()
            .iter()
            .map(|pending| pending.component.name())
            .collect();
        format!("{phase} {pending:?} {:?}", update.failure())
    }

    fn told(event: &UpdateEvent) -> String {
        match event {
            UpdateEvent::Done(component, Ok(())) => format!("done {}", component.name()),
            UpdateEvent::Done(component, Err(message)) => {
                format!("done {}: {message}", component.name())
            }
            UpdateEvent::Error(message) => format!("error {message}"),
        }
    }

    fn result(pending: Vec<Pending>, failure: Option<Failure>) -> Check {
        Check {
            pending,
            failure,
            history: Vec::new(),
        }
    }

    fn found(component: Component) -> Pending {
        Pending {
            component,
            target: "9.9".to_owned(),
            notes: ReleaseNotes::default(),
        }
    }

    impl Bench {
        fn new(cx: &mut TestAppContext) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
            fs::create_dir_all(&paths.data_root).unwrap();
            fs::write(paths.settings_file(), "[settings]\nusername = 'Ash'\n").unwrap();
            let store = Store::load(&paths).unwrap();
            let script = Arc::new(Script::default());
            let launcher = cx.new(|_| {
                let mut launcher = Launcher::new(paths.clone(), store, Vec::new());
                launcher.update.work = script.clone();
                launcher
            });
            let seen = Rc::new(RefCell::new(Vec::new()));
            cx.update(|cx| {
                let states = seen.clone();
                cx.observe(&launcher, move |launcher, cx| {
                    let state = state(&launcher.read(cx).update);
                    states.borrow_mut().push(state);
                })
                .detach();
                let events = seen.clone();
                cx.subscribe(&launcher, move |_, event: &UpdateEvent, _| {
                    events.borrow_mut().push(told(event));
                })
                .detach();
            });
            Self {
                _dir: dir,
                paths,
                launcher,
                script,
                seen,
            }
        }

        fn check(&self, force: bool, cx: &mut TestAppContext) {
            self.launcher
                .update(cx, |launcher, cx| launcher.check_updates(force, cx));
        }

        fn apply(&self, component: Component, cx: &mut TestAppContext) {
            self.launcher
                .update(cx, |launcher, cx| launcher.apply_update(component, cx));
        }

        fn list(&self, components: &[Component], cx: &mut TestAppContext) {
            let pending = components.iter().copied().map(found).collect();
            self.script
                .checks
                .lock()
                .unwrap()
                .push_back(result(pending, None));
            self.check(false, cx);
            cx.run_until_parked();
            self.seen();
            self.calls();
        }

        fn answer(&self, check: Check) {
            self.script.checks.lock().unwrap().push_back(check);
        }

        fn outcome(&self, progress: &[f64], outcome: Result<(), &str>) {
            let outcome = outcome.map_err(str::to_owned);
            let mut applies = self.script.applies.lock().unwrap();
            applies.push_back((progress.to_vec(), outcome));
        }

        fn seen(&self) -> Vec<String> {
            self.seen.take()
        }

        fn calls(&self) -> Vec<String> {
            std::mem::take(&mut self.script.calls.lock().unwrap())
        }

        fn state(&self, cx: &mut TestAppContext) -> String {
            self.launcher
                .read_with(cx, |launcher, _| state(&launcher.update))
        }
    }

    #[gpui::test]
    fn a_check_runs_alone_and_lists_what_it_found(cx: &mut TestAppContext) {
        let bench = Bench::new(cx);
        bench.answer(result(vec![found(Component::SevenZip)], None));
        bench.check(false, cx);
        bench.check(true, cx);
        assert_eq!(bench.state(cx), "checking [] None");
        assert!(bench.calls().is_empty());
        cx.run_until_parked();
        assert_eq!(bench.seen(), ["checking [] None", "idle [\"7z\"] None"]);
        assert_eq!(bench.calls(), ["check force=false user=Ash libraries=1"]);

        bench.check(true, cx);
        cx.run_until_parked();
        assert_eq!(bench.seen(), ["checking [\"7z\"] None", "idle [] None"]);
        assert_eq!(bench.calls(), ["check force=true user=Ash libraries=1"]);
    }

    #[gpui::test]
    fn a_failed_check_without_results_keeps_the_list(cx: &mut TestAppContext) {
        let bench = Bench::new(cx);
        bench.list(&[Component::SevenZip], cx);

        bench.answer(result(Vec::new(), Some(Failure::Error)));
        bench.check(false, cx);
        cx.run_until_parked();
        assert_eq!(bench.state(cx), "idle [\"7z\"] Some(Error)");

        let limit = Failure::RateLimit("GitHub rate limit reached, try again later".to_owned());
        bench.answer(result(vec![found(Component::ThrowbackLoader)], Some(limit)));
        bench.check(false, cx);
        cx.run_until_parked();
        assert_eq!(
            bench.state(cx),
            "idle [\"ThrowbackLoader\"] \
             Some(RateLimit(\"GitHub rate limit reached, try again later\"))"
        );

        bench.check(false, cx);
        cx.run_until_parked();
        assert_eq!(bench.state(cx), "idle [] None");
    }

    #[gpui::test]
    fn a_check_keeps_the_last_history_when_it_brings_none(cx: &mut TestAppContext) {
        let bench = Bench::new(cx);
        let release = |version: &str| Release {
            version: version.to_owned(),
            notes: ReleaseNotes::default(),
        };
        let history = |cx: &mut TestAppContext| {
            bench
                .launcher
                .read_with(cx, |launcher, _| launcher.update.history().to_vec())
        };
        let first = vec![
            (Component::Launcher, vec![release("0.2.3")]),
            (Component::SevenZip, Vec::new()),
        ];
        bench.answer(Check {
            history: first.clone(),
            ..Check::default()
        });
        bench.check(false, cx);
        cx.run_until_parked();
        assert_eq!(history(cx), first);

        bench.answer(Check {
            failure: Some(Failure::Error),
            history: vec![(Component::Launcher, Vec::new())],
            ..Check::default()
        });
        bench.check(true, cx);
        cx.run_until_parked();
        assert_eq!(history(cx), first);

        let newer = vec![(
            Component::Launcher,
            vec![release("0.2.4"), release("0.2.3")],
        )];
        bench.answer(Check {
            history: newer.clone(),
            ..Check::default()
        });
        bench.check(false, cx);
        cx.run_until_parked();
        assert_eq!(history(cx), newer);
    }

    #[gpui::test]
    fn only_a_listed_update_starts_and_only_when_nothing_else_runs(cx: &mut TestAppContext) {
        let bench = Bench::new(cx);
        bench.apply(Component::SevenZip, cx);
        cx.run_until_parked();
        assert!(bench.calls().is_empty());
        assert!(bench.seen().is_empty());

        bench.list(&[Component::SevenZip, Component::ThrowbackLoader], cx);
        bench.apply(Component::HeatedMetal, cx);

        bench.check(false, cx);
        bench.apply(Component::SevenZip, cx);
        cx.run_until_parked();
        assert_eq!(bench.calls(), ["check force=false user=Ash libraries=1"]);
        assert_eq!(
            bench.seen(),
            [
                "checking [\"7z\", \"ThrowbackLoader\"] None",
                "idle [] None"
            ]
        );

        bench.list(&[Component::SevenZip, Component::ThrowbackLoader], cx);
        bench.outcome(&[], Ok(()));
        bench.apply(Component::SevenZip, cx);
        bench.apply(Component::ThrowbackLoader, cx);
        bench.check(true, cx);
        assert_eq!(
            bench.state(cx),
            "applying 7z 0% [\"7z\", \"ThrowbackLoader\"] None"
        );
        assert!(
            bench
                .launcher
                .read_with(cx, |launcher, _| launcher.update.busy())
        );
        cx.run_until_parked();
        assert_eq!(
            bench.calls(),
            ["apply 7z", "check force=false user=Ash libraries=1"]
        );
    }

    #[gpui::test]
    fn an_update_is_refused_while_the_cache_is_cleared_or_a_download_or_uninstall_runs(
        cx: &mut TestAppContext,
    ) {
        let bench = Bench::new(cx);
        bench.list(&[Component::SevenZip], cx);
        bench.launcher.update(cx, |launcher, _| {
            launcher.settings.clearing_cache = true;
            launcher.download = Downloader::with(State::Downloading, None, false, Vec::new());
            launcher.uninstall = Uninstaller::busy_with("Y1S0_Vanilla");
        });
        let refused = |cx: &mut TestAppContext| {
            bench.apply(Component::SevenZip, cx);
            cx.run_until_parked();
            assert!(bench.calls().is_empty());
            assert_eq!(bench.state(cx), "idle [\"7z\"] None");
            bench.seen()
        };
        assert_eq!(refused(cx), ["error The cache is being cleared"]);
        bench
            .launcher
            .update(cx, |launcher, _| launcher.settings.clearing_cache = false);
        assert_eq!(refused(cx), ["error A download is running"]);
        bench
            .launcher
            .update(cx, |launcher, _| launcher.download = Downloader::default());
        assert_eq!(refused(cx), ["error An uninstall is running"]);
        bench.launcher.update(cx, |launcher, _| {
            launcher.uninstall = Uninstaller::default()
        });
        bench.outcome(&[], Ok(()));
        bench.apply(Component::SevenZip, cx);
        assert_eq!(bench.state(cx), "applying 7z 0% [\"7z\"] None");
    }

    #[gpui::test]
    fn an_applied_update_leaves_the_list_before_the_recheck(cx: &mut TestAppContext) {
        let bench = Bench::new(cx);
        bench.list(&[Component::SevenZip, Component::ThrowbackLoader], cx);
        bench.outcome(&[0.004, 0.5, 0.509, 0.2, 1.], Ok(()));
        bench.answer(result(Vec::new(), Some(Failure::Error)));
        bench.apply(Component::SevenZip, cx);
        cx.run_until_parked();
        assert_eq!(
            bench.seen(),
            [
                "applying 7z 0% [\"7z\", \"ThrowbackLoader\"] None",
                "applying 7z 50% [\"7z\", \"ThrowbackLoader\"] None",
                "applying 7z 20% [\"7z\", \"ThrowbackLoader\"] None",
                "applying 7z 100% [\"7z\", \"ThrowbackLoader\"] None",
                "checking [\"ThrowbackLoader\"] None",
                "done 7z",
                "idle [\"ThrowbackLoader\"] Some(Error)",
            ]
        );
        assert_eq!(
            bench.calls(),
            ["apply 7z", "check force=false user=Ash libraries=1"]
        );
    }

    #[gpui::test]
    fn a_failed_update_stays_listed_and_reports_its_message(cx: &mut TestAppContext) {
        let bench = Bench::new(cx);
        bench.list(&[Component::HeatedMetal], cx);
        bench.outcome(&[0.3], Err("Stop R6S first"));
        bench.answer(result(Vec::new(), Some(Failure::Error)));
        bench.apply(Component::HeatedMetal, cx);
        cx.run_until_parked();
        assert_eq!(
            bench.seen(),
            [
                "applying Heated Metal 0% [\"Heated Metal\"] None",
                "applying Heated Metal 30% [\"Heated Metal\"] None",
                "checking [\"Heated Metal\"] None",
                "done Heated Metal: Stop R6S first",
                "idle [\"Heated Metal\"] Some(Error)",
            ]
        );

        bench.outcome(&[], Ok(()));
        bench.apply(Component::HeatedMetal, cx);
        assert_eq!(
            bench.state(cx),
            "applying Heated Metal 0% [\"Heated Metal\"] Some(Error)"
        );
        cx.run_until_parked();
        assert_eq!(bench.state(cx), "idle [] None");
    }

    #[gpui::test]
    fn a_launcher_update_restarts_after_the_delay(cx: &mut TestAppContext) {
        let bench = Bench::new(cx);
        bench.list(&[Component::Launcher, Component::SevenZip], cx);
        bench.outcome(&[1.], Ok(()));
        bench.apply(Component::Launcher, cx);
        cx.run_until_parked();
        let applying = "applying Throwback Launcher 100% [\"Throwback Launcher\", \"7z\"] None";
        assert_eq!(
            bench.seen(),
            [
                "applying Throwback Launcher 0% [\"Throwback Launcher\", \"7z\"] None",
                applying
            ]
        );
        bench.check(true, cx);
        bench.apply(Component::SevenZip, cx);
        cx.executor()
            .advance_clock(RESTART_DELAY - Duration::from_millis(1));
        assert_eq!(bench.calls(), ["apply Throwback Launcher"]);
        cx.executor().advance_clock(Duration::from_millis(1));
        assert_eq!(bench.calls(), ["relaunch"]);
        assert_eq!(bench.state(cx), applying);
        assert!(bench.seen().is_empty());
    }

    #[gpui::test]
    fn a_failed_launcher_update_does_not_restart(cx: &mut TestAppContext) {
        let bench = Bench::new(cx);
        bench.list(&[Component::Launcher], cx);
        bench.outcome(&[], Err("Throwback Launcher update failed"));
        bench.answer(result(vec![found(Component::Launcher)], None));
        bench.apply(Component::Launcher, cx);
        cx.executor().advance_clock(RESTART_DELAY * 2);
        assert_eq!(
            bench.seen(),
            [
                "applying Throwback Launcher 0% [\"Throwback Launcher\"] None",
                "checking [\"Throwback Launcher\"] None",
                "done Throwback Launcher: Throwback Launcher update failed",
                "idle [\"Throwback Launcher\"] None",
            ]
        );
        assert_eq!(
            bench.calls(),
            [
                "apply Throwback Launcher",
                "check force=false user=Ash libraries=1"
            ]
        );
    }

    #[gpui::test]
    async fn installed_versions_come_from_this_install(cx: &mut TestAppContext) {
        let bench = Bench::new(cx);
        let versions = bench
            .launcher
            .read_with(cx, |launcher, cx| launcher.installed_versions(cx));
        assert_eq!(versions.await, [(Component::Launcher, VERSION.to_owned())]);
        assert!(bench.calls().is_empty());
    }

    #[gpui::test]
    fn the_outcome_of_a_launcher_update_is_announced_once_after_the_restart(
        cx: &mut TestAppContext,
    ) {
        let bench = Bench::new(cx);
        let announce = |cx: &mut TestAppContext| {
            bench
                .launcher
                .update(cx, |launcher, cx| launcher.announce_update_outcome(cx));
            bench.seen()
        };
        assert!(announce(cx).is_empty());
        write_outcome(&bench.paths, true);
        assert_eq!(announce(cx), ["done Throwback Launcher"]);
        assert!(announce(cx).is_empty());
        write_outcome(&bench.paths, false);
        assert_eq!(
            announce(cx),
            ["done Throwback Launcher: Throwback Launcher update failed"]
        );
        assert!(bench.calls().is_empty());
    }
}
