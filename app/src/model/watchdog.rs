use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AppContext, Context, Task};

use super::{Launcher, Scan};

const SCAN_INTERVAL: Duration = Duration::from_secs(2);
const FAST_WINDOW: Duration = Duration::from_secs(15);

#[derive(Default)]
pub struct Watchdog {
    folders: BTreeMap<String, Vec<u32>>,
    scan: Scan,
    fast_until: Option<Instant>,
    timer: Option<Task<()>>,
}

impl Watchdog {
    pub fn folders(&self) -> &BTreeMap<String, Vec<u32>> {
        &self.folders
    }
}

#[cfg(test)]
impl Watchdog {
    pub fn set_folders(&mut self, folders: BTreeMap<String, Vec<u32>>) {
        self.folders = folders;
    }
}

impl Launcher {
    pub fn scan_games(&mut self, cx: &mut Context<Self>) {
        if !self.watchdog.scan.begin() {
            return;
        }
        let host = Arc::clone(&self.launch.host);
        let libraries = self.settings.roots().to_vec();
        let scan = cx.background_spawn(async move { host.games(&libraries) });
        cx.spawn(async move |this, cx| {
            let folders = scan.await;
            this.update(cx, |this, cx| this.games_scanned(folders, cx))
                .ok();
        })
        .detach();
    }

    pub fn poke_watchdog(&mut self, cx: &mut Context<Self>) {
        self.watchdog.fast_until = Some(cx.background_executor().now() + FAST_WINDOW);
        self.scan_games(cx);
    }

    fn games_scanned(&mut self, folders: BTreeMap<String, Vec<u32>>, cx: &mut Context<Self>) {
        let again = self.watchdog.scan.end();
        let before = std::mem::replace(&mut self.watchdog.folders, folders);
        self.tick_liberator(cx);
        let shown = self.watchdog.folders.keys().next();
        if let Some(presence) = &mut self.presence {
            presence.set_folder(shown.map(String::as_str));
        }
        self.follow_games(cx);
        let now = cx.background_executor().now();
        let launching = self.launch.launching().is_some();
        let watchdog = &mut self.watchdog;
        if launching {
            watchdog.fast_until = Some(now + FAST_WINDOW);
        }
        let fast = !watchdog.folders.is_empty()
            || !before.is_empty()
            || watchdog.fast_until.is_some_and(|until| now < until);
        watchdog.timer = fast.then(|| {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SCAN_INTERVAL).await;
                this.update(cx, |this, cx| this.scan_games(cx)).ok();
            })
        });
        if again {
            self.scan_games(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::super::launch::testing::Stage;
    use super::*;

    const UNKNOWN: &str = "Y99S1_Probe";

    fn armed(stage: &Stage, cx: &mut TestAppContext) -> bool {
        let launcher = &stage.launcher;
        launcher.read_with(cx, |launcher, _| launcher.watchdog.timer.is_some())
    }

    fn poke(stage: &Stage, cx: &mut TestAppContext) {
        let launcher = &stage.launcher;
        launcher.update(cx, |launcher, cx| launcher.poke_watchdog(cx));
    }

    #[gpui::test]
    fn the_first_scan_ends_idle_and_a_poke_scans_for_fifteen_seconds(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "");
        stage.tick(cx);
        assert_eq!(stage.host.scans(), 1);
        stage.pass(60, cx);
        assert_eq!(stage.host.scans(), 1);
        assert!(!armed(&stage, cx));

        poke(&stage, cx);
        cx.run_until_parked();
        assert_eq!(stage.host.scans(), 2);
        for scan in 3..=10 {
            assert!(armed(&stage, cx));
            stage.pass(1, cx);
            assert_eq!(stage.host.scans(), scan - 1);
            stage.pass(1, cx);
            assert_eq!(stage.host.scans(), scan);
        }
        assert!(!armed(&stage, cx));
        stage.pass(60, cx);
        assert_eq!(stage.host.scans(), 10);
    }

    #[gpui::test]
    fn a_poke_restarts_the_two_second_countdown(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "");
        poke(&stage, cx);
        cx.run_until_parked();
        stage.pass(1, cx);
        poke(&stage, cx);
        cx.run_until_parked();
        assert_eq!(stage.host.scans(), 2);
        stage.pass(1, cx);
        assert_eq!(stage.host.scans(), 2);
        stage.pass(1, cx);
        assert_eq!(stage.host.scans(), 3);
    }

    #[gpui::test]
    fn a_running_game_keeps_the_rhythm_until_one_scan_after_it_closed(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "");
        stage.host.run(UNKNOWN, &[50, 51]);
        stage.tick(cx);
        let folders = |cx: &mut TestAppContext| {
            let launcher = &stage.launcher;
            launcher.read_with(cx, |launcher, _| launcher.watchdog.folders().clone())
        };
        assert_eq!(
            folders(cx),
            BTreeMap::from([(UNKNOWN.to_owned(), vec![50, 51])])
        );
        for scan in 2..=20 {
            stage.pass(2, cx);
            assert_eq!(stage.host.scans(), scan);
        }
        stage.host.quit(UNKNOWN);
        stage.pass(2, cx);
        assert_eq!(stage.host.scans(), 21);
        assert!(folders(cx).is_empty());
        assert!(armed(&stage, cx));
        stage.pass(2, cx);
        assert_eq!(stage.host.scans(), 22);
        assert!(!armed(&stage, cx));
        stage.pass(60, cx);
        assert_eq!(stage.host.scans(), 22);
    }

    #[gpui::test]
    fn requests_during_a_scan_chain_exactly_one_more(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "");
        stage.launcher.update(cx, |launcher, cx| {
            launcher.scan_games(cx);
            launcher.scan_games(cx);
            launcher.poke_watchdog(cx);
        });
        cx.run_until_parked();
        assert_eq!(stage.host.scans(), 2);
        assert!(armed(&stage, cx));
    }

    #[cfg(not(windows))]
    #[gpui::test]
    fn an_open_launch_keeps_the_watchdog_fast_until_fifteen_seconds_after_it_ended(
        cx: &mut TestAppContext,
    ) {
        use super::super::launch::testing::KEY;

        let stage = Stage::new(cx, "");
        stage.install(KEY);
        stage.host.offer(&["ge"]);
        let launcher = &stage.launcher;
        launcher.update(cx, |launcher, cx| launcher.launch_season(KEY, false, cx));
        cx.run_until_parked();
        let before = stage.host.scans();
        for scan in 1..=30 {
            stage.pass(2, cx);
            assert_eq!(stage.host.scans(), before + scan);
            assert!(armed(&stage, cx));
        }
        assert!(stage.launching(cx).is_some());
        launcher.update(cx, |launcher, cx| launcher.stop_season(KEY, cx));
        cx.run_until_parked();
        let before = stage.host.scans();
        stage.pass(14, cx);
        assert_eq!(stage.host.scans(), before + 7);
        assert!(armed(&stage, cx));
        stage.pass(60, cx);
        assert_eq!(stage.host.scans(), before + 8);
        assert!(!armed(&stage, cx));
    }
}
