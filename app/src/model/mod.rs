pub mod cheatengine;
mod dialogs;
pub mod download;
pub mod home;
pub mod info;
pub mod launch;
pub mod liberator;
pub mod presence;
pub mod rvpn;
pub mod settings;
pub mod shears;
pub mod uninstall;
pub mod update;
pub mod watchdog;

use backend::env::Paths;
use backend::manifest::Download;
use backend::presence::PresenceWorker;
use backend::settings::{Color, Flag, Settings as Store};
use gpui::{App, AppContext, Entity, Global};

use crate::ui::theme::{Theme, hex, set_palette};
use cheatengine::CheatEngine;
use download::Downloader;
use home::Home;
use info::Info;
use launch::Launch;
use liberator::Liberator;
use rvpn::Rvpn;
use settings::Settings;
use shears::Shears;
use uninstall::Uninstaller;
use update::Updater;
use watchdog::Watchdog;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edition {
    pub key: String,
    pub hm: bool,
}

impl Edition {
    pub fn is(&self, key: &str, hm: bool) -> bool {
        self.key == key && self.hm == hm
    }
}

pub struct Launcher {
    pub paths: Paths,
    pub downloads: Vec<Download>,
    pub settings: Settings,
    pub info: Info,
    pub home: Home,
    pub download: Downloader,
    pub launch: Launch,
    pub presence: Option<PresenceWorker>,
    pub liberator: Liberator,
    pub rvpn: Rvpn,
    pub update: Updater,
    pub uninstall: Uninstaller,
    pub shears: Shears,
    pub cheat_engine: CheatEngine,
    pub watchdog: Watchdog,
}

struct Handle(Entity<Launcher>);

impl Global for Handle {}

#[derive(Default)]
struct Scan {
    running: bool,
    again: bool,
}

impl Scan {
    fn begin(&mut self) -> bool {
        self.again = self.running;
        self.running = true;
        !self.again
    }

    fn end(&mut self) -> bool {
        self.running = false;
        std::mem::take(&mut self.again)
    }
}

impl Launcher {
    pub fn new(paths: Paths, store: Store, downloads: Vec<Download>) -> Self {
        let mut settings = Settings::new(store);
        let download = Downloader::new(&mut settings, &downloads);
        Self {
            launch: Launch::new(&paths),
            presence: Some(PresenceWorker::new(downloads.clone())),
            liberator: Liberator::new(paths.clone()),
            rvpn: Rvpn::new(&paths),
            paths,
            downloads,
            settings,
            info: Info::default(),
            home: Home::default(),
            download,
            update: Updater::default(),
            uninstall: Uninstaller::default(),
            shears: Shears::default(),
            cheat_engine: CheatEngine::default(),
            watchdog: Watchdog::default(),
        }
    }

    pub fn init(self, cx: &mut App) {
        let launcher = cx.new(|_| self);
        launcher.update(cx, |launcher, cx| {
            launcher.refresh_disk_usage(cx);
            launcher.refresh_home(cx);
            launcher.scan_games(cx);
            Self::start_next_later(cx);
            if launcher.settings.flag(Flag::LiberatorEnabled) {
                launcher.start_liberator(cx);
            }
            launcher.autorun_rvpn(cx);
            launcher.load_nyan(cx);
        });
        let mut accent = None;
        Self::apply(&launcher, &mut accent, cx);
        cx.observe(&launcher, move |launcher, cx| {
            Self::apply(&launcher, &mut accent, cx)
        })
        .detach();
        cx.set_global(Handle(launcher));
    }

    fn apply(launcher: &Entity<Self>, shown: &mut Option<u32>, cx: &mut App) {
        let settings = &launcher.read(cx).settings;
        let accent = settings.color(Color::Accent);
        let reduce_motion = settings.flag(Flag::ReduceMotion);
        let scaled = crate::ui::set_scale(settings.scale());
        let toned = set_palette(settings.color(Color::Background));
        let tinted = shown.replace(accent) != Some(accent);
        if tinted {
            cx.set_global(Theme::for_action(hex(accent)));
        }
        if tinted || scaled || toned {
            cx.refresh_windows();
        }
        cx.set_reduce_motion(reduce_motion);
        launcher.update(cx, |launcher, _| launcher.sync_presence());
    }

    pub fn handle(cx: &App) -> Entity<Self> {
        cx.global::<Handle>().0.clone()
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;

    #[gpui::test]
    fn a_previewed_accent_stays_until_the_stored_one_changes(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        std::fs::create_dir_all(&paths.data_root).unwrap();
        std::fs::write(paths.settings_file(), "[settings]\ndiscord_rpc = false\n").unwrap();
        let launcher = cx.update(|cx| {
            crate::init(cx);
            let store = Store::load(&paths).unwrap();
            Launcher::new(paths, store, backend::manifest::downloads()).init(cx);
            Launcher::handle(cx)
        });
        let shown = |cx: &mut TestAppContext| cx.update(|cx| Theme::get(cx).action);
        let (preview, stored) = (hex(0x33_66_ff), hex(0x22_aa_44));

        cx.update(|cx| cx.set_global(Theme::for_action(preview)));
        launcher.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert!(shown(cx) == preview);

        launcher.update(cx, |launcher, cx| {
            launcher.settings.set_color(Color::Accent, 0x22_aa_44, cx)
        });
        cx.run_until_parked();
        assert!(shown(cx) == stored);
    }
}
