use backend::log;
use backend::settings::Flag;

use super::Launcher;

impl Launcher {
    pub fn sync_presence(&mut self) {
        let Some(presence) = &mut self.presence else {
            return;
        };
        if let Err(error) = presence.set_enabled(self.settings.flag(Flag::DiscordRpc)) {
            log::fail("Discord presence failed", error);
        }
    }

    pub fn stop_presence(&mut self) {
        self.presence = None;
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::fs;
    use std::thread;
    use std::time::{Duration, Instant};

    use gpui::TestAppContext;

    use super::super::launch::testing::Stage;
    use super::*;

    fn workers() -> usize {
        let tasks = fs::read_dir("/proc/self/task").unwrap().flatten();
        let names = tasks.filter_map(|task| fs::read_to_string(task.path().join("comm")).ok());
        names
            .filter(|name| name.trim_end() == "discord-rpc")
            .count()
    }

    fn settled(wanted: usize) -> bool {
        let deadline = Instant::now() + Duration::from_secs(20);
        while workers() != wanted && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        workers() == wanted
    }

    #[gpui::test]
    fn the_presence_worker_runs_only_while_the_setting_is_on(cx: &mut TestAppContext) {
        let stage = Stage::new(cx, "");
        let launcher = &stage.launcher;
        let sync = |cx: &mut gpui::App| {
            let follow = |launcher: gpui::Entity<Launcher>, cx: &mut gpui::App| {
                launcher.update(cx, |launcher, _| launcher.sync_presence());
            };
            follow(launcher.clone(), cx);
            cx.observe(launcher, follow).detach();
        };
        let switch = |on: bool, cx: &mut TestAppContext| {
            launcher.update(cx, |launcher, cx| {
                launcher.settings.set_flag(Flag::DiscordRpc, on, cx);
            });
        };
        assert_eq!(workers(), 0);
        cx.update(sync);
        assert!(settled(1));
        switch(false, cx);
        assert!(settled(0));
        launcher.update(cx, |_, cx| cx.notify());
        assert!(settled(0));
        switch(true, cx);
        launcher.update(cx, |_, cx| cx.notify());
        assert!(settled(1));
        launcher.update(cx, |launcher, _| launcher.stop_presence());
        assert!(settled(0));
        launcher.update(cx, |_, cx| cx.notify());
        assert!(launcher.read_with(cx, |launcher, _| launcher.presence.is_none()));
        assert!(settled(0));
    }
}
