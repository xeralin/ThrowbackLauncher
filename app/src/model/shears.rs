use std::fmt::Display;
use std::path::PathBuf;

use backend::env::{BUSY_MESSAGE, NOT_INSTALLED, SEASON_RUNNING};
use backend::log;
use backend::manifest::installed_path;
use backend::shears::{Kind, Scan, cut_download, scan_download};
use backend::steam::is_season_running;
use gpui::{AppContext, Context, EventEmitter, SharedString};

use super::Launcher;

const EVENT_SEASONS: [(&str, &str); 30] = [
    ("Y3S1_Chimera", "fury"),
    ("Y4S1_BurntHorizon", "events"),
    ("Y4S2_PhantomSight", "events"),
    ("Y4S4_ShiftingTides", "events"),
    ("Y5S1_VoidEdge", "events"),
    ("Y5S2_SteelWave", "events"),
    ("Y5S3_ShadowLegacy", "events"),
    ("Y5S4_NeonDawn", "events"),
    ("Y6S1_CrimsonHeist", "events"),
    ("Y6S2_NorthStar", "events"),
    ("Y6S3_CrystalGuard", "events"),
    ("Y6S4_HighCalibre", "events"),
    ("Y7S1_DemonVeil", "events"),
    ("Y7S2_VectorGlare", "events"),
    ("Y7S3_BrutalSwarm", "events"),
    ("Y7S4_SolarRaid", "events"),
    ("Y8S1_CommandingForce", "events"),
    ("Y8S2_DreadFactor", "events"),
    ("Y8S3_HeavyMettle", "events"),
    ("Y8S4_DeepFreeze", "events"),
    ("Y9S1_DeadlyOmen", "events"),
    ("Y9S2_NewBlood", "events"),
    ("Y9S3_TwinShells", "events"),
    ("Y9S4_CollisionPoint", "events"),
    ("Y10S1_PrepPhase", "events"),
    ("Y10S2_Daybreak", "events"),
    ("Y10S3_HighStakes", "events"),
    ("Y10S4_TenfoldPursuit", "events"),
    ("Y11S1_SilentHunt", "events"),
    ("Y11S2_SystemOverride", "events"),
];

pub type Outcome = Result<Scan, SharedString>;

pub enum ShearsEvent {
    Scanned { key: SharedString, outcome: Outcome },
    Cut { key: SharedString, outcome: Outcome },
}

#[derive(Default)]
pub struct Shears {
    busy: Option<String>,
}

impl EventEmitter<ShearsEvent> for Launcher {}

impl Shears {
    pub fn busy_key(&self) -> Option<&str> {
        self.busy.as_deref()
    }
}

#[cfg(test)]
impl Shears {
    pub fn busy_with(key: &str) -> Self {
        Self {
            busy: Some(key.to_owned()),
        }
    }
}

fn event_pattern(key: &str) -> Option<&'static str> {
    let season = EVENT_SEASONS.iter().find(|(season, _)| *season == key);
    season.map(|(_, pattern)| *pattern)
}

fn failed(error: impl Display) -> String {
    log::fail("Shears failed", format!("{error:#}"))
}

fn scan(libraries: &[PathBuf], key: &str) -> Result<Scan, String> {
    match installed_path(libraries, key, false) {
        Some(folder) => scan_download(&folder, event_pattern(key)).map_err(failed),
        None => Ok(Scan::default()),
    }
}

fn cut(libraries: &[PathBuf], key: &str, kind: Kind) -> Result<Scan, String> {
    let Some(folder) = installed_path(libraries, key, false) else {
        return Err(NOT_INSTALLED.to_owned());
    };
    if is_season_running(libraries, key) {
        return Err(SEASON_RUNNING.to_owned());
    }
    let pattern = event_pattern(key);
    cut_download(&folder, kind, pattern)
        .and_then(|()| scan_download(&folder, pattern))
        .map_err(failed)
}

impl Launcher {
    pub fn scan_season(&self, key: &str, cx: &mut Context<Self>) {
        let (libraries, key) = (self.settings.roots().to_vec(), SharedString::from(key));
        let scan = cx.background_spawn({
            let key = key.clone();
            async move { scan(&libraries, &key) }
        });
        cx.spawn(async move |this, cx| {
            let outcome = scan.await.map_err(SharedString::from);
            this.update(cx, |_, cx| cx.emit(ShearsEvent::Scanned { key, outcome }))
                .ok();
        })
        .detach();
    }

    pub fn cut_season(&mut self, key: &str, kind: Kind, cx: &mut Context<Self>) {
        if self.shears.busy.is_some() {
            return;
        }
        if self.download.busy_with(key) {
            return self.cut_done(key.into(), Err(BUSY_MESSAGE.into()), cx);
        }
        self.shears.busy = Some(key.to_owned());
        let (libraries, key) = (self.settings.roots().to_vec(), SharedString::from(key));
        let cut = cx.background_spawn({
            let key = key.clone();
            async move { cut(&libraries, &key, kind) }
        });
        cx.spawn(async move |this, cx| {
            let outcome = cut.await.map_err(SharedString::from);
            this.update(cx, |this, cx| {
                this.shears.busy = None;
                this.cut_done(key, outcome, cx);
            })
            .ok();
        })
        .detach();
    }

    fn cut_done(&mut self, key: SharedString, outcome: Outcome, cx: &mut Context<Self>) {
        cx.emit(ShearsEvent::Cut { key, outcome });
        self.start_next(cx);
        self.refresh_disk_usage(cx);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::path::Path;
    use std::rc::Rc;

    use backend::env::{GIB, Paths, TL_LAUNCHER};
    use backend::settings::Settings as Store;
    use backend::shears::Tier;
    use gpui::{Entity, TestAppContext};

    use super::*;
    use crate::model::download::Downloader;

    const KEY: &str = "Y3S1_Chimera";
    const PLAIN: &str = "Y1S0_Vanilla";

    type Events = Rc<RefCell<Vec<(&'static str, String, Result<Scan, String>)>>>;

    fn write(file: &Path, size: u64) {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::File::create(file).unwrap().set_len(size).unwrap();
    }

    fn rig(cx: &mut TestAppContext, dir: &Path) -> (Entity<Launcher>, Events, PathBuf) {
        let paths = Paths::new(dir.join("asset"), dir.join("data"));
        let season = paths.default_downloads_dir().join(KEY);
        for (file, size) in [
            (TL_LAUNCHER, 0),
            ("datapc64_merged_bnk_textures0.forge", 100),
            ("datapc64_merged_bnk_textures3.forge", 400),
            ("datapc64_fury_bnk_mesh.forge", 30),
            ("videos/intro.bik", GIB / 2),
            ("videos/fury_intro.bik", 7),
            ("videos/startup/logo.bik", 20),
        ] {
            write(&season.join(file), size);
        }
        write(
            &paths.default_downloads_dir().join(PLAIN).join(TL_LAUNCHER),
            0,
        );
        let store = Store::load(&paths).unwrap();
        let launcher = cx.new(|_| Launcher::new(paths, store, Vec::new()));
        let events = Events::default();
        cx.update(|cx| {
            let seen = events.clone();
            cx.subscribe(&launcher, move |_, event: &ShearsEvent, _| {
                let (name, key, outcome) = match event {
                    ShearsEvent::Scanned { key, outcome } => ("scan", key, outcome),
                    ShearsEvent::Cut { key, outcome } => ("cut", key, outcome),
                };
                let outcome = outcome.clone().map_err(|message| message.to_string());
                seen.borrow_mut().push((name, key.to_string(), outcome));
            })
            .detach();
        });
        (launcher, events, season)
    }

    fn tiers(sizes: [u64; 2]) -> Vec<Tier> {
        let tier = |level, quality, size| Tier {
            level,
            quality,
            size,
        };
        vec![tier(0, "Low", sizes[0]), tier(3, "Very High", sizes[1])]
    }

    #[gpui::test]
    fn a_scan_measures_what_can_be_cut_and_is_never_blocked(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let (launcher, events, _) = rig(cx, dir.path());
        launcher.update(cx, |launcher, cx| {
            launcher.shears.busy = Some(KEY.to_owned());
            launcher.download = Downloader::downloading(KEY, false);
            launcher.scan_season(KEY, cx);
            launcher.scan_season("Y2S1_VelvetShell", cx);
        });
        cx.run_until_parked();
        let full = Scan {
            tiers: tiers([100, 400]),
            videos: GIB / 2 + 20,
            events: 37,
        };
        let mut scanned = events.borrow().clone();
        scanned.sort_by(|left, right| left.1.cmp(&right.1));
        assert_eq!(
            scanned,
            [
                ("scan", "Y2S1_VelvetShell".to_owned(), Ok(Scan::default())),
                ("scan", KEY.to_owned(), Ok(full)),
            ]
        );
    }

    #[gpui::test]
    fn a_cut_deletes_the_files_and_answers_with_a_fresh_scan(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let (launcher, events, season) = rig(cx, dir.path());
        launcher.update(cx, |launcher, cx| {
            launcher.cut_season(KEY, Kind::Videos, cx);
            assert_eq!(launcher.shears.busy_key(), Some(KEY));
            launcher.cut_season(KEY, Kind::Events, cx);
            launcher.cut_season(PLAIN, Kind::Videos, cx);
        });
        assert!(events.borrow().is_empty());
        assert!(season.join("videos/intro.bik").exists());

        cx.run_until_parked();
        let left = Scan {
            tiers: tiers([100, 400]),
            videos: 0,
            events: 37,
        };
        assert_eq!(*events.borrow(), [("cut", KEY.to_owned(), Ok(left))]);
        assert!(!season.join("videos/intro.bik").exists());
        assert!(!season.join("videos/startup").exists());
        assert!(season.join("videos/fury_intro.bik").exists());
        launcher.read_with(cx, |launcher, _| {
            assert_eq!(launcher.shears.busy_key(), None);
            assert_eq!(launcher.info.disk_usage(), Some(0.));
        });

        launcher.update(cx, |launcher, cx| {
            launcher.cut_season(KEY, Kind::Textures { above: 0 }, cx);
        });
        cx.run_until_parked();
        let (_, _, outcome) = events.borrow().last().cloned().unwrap();
        assert_eq!(outcome.unwrap().tiers, [tiers([100, 0])[0].clone()]);
        assert!(!season.join("datapc64_merged_bnk_textures3.forge").exists());
    }

    #[gpui::test]
    fn a_cut_is_rejected_for_a_running_download_or_a_missing_season(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let (launcher, events, season) = rig(cx, dir.path());
        launcher.update(cx, |launcher, cx| {
            launcher.download = Downloader::downloading(PLAIN, true);
            launcher.cut_season(PLAIN, Kind::Videos, cx);
            assert_eq!(launcher.shears.busy_key(), None);
            launcher.cut_season("Y2S1_VelvetShell", Kind::Videos, cx);
            assert_eq!(launcher.shears.busy_key(), Some("Y2S1_VelvetShell"));
        });
        cx.run_until_parked();
        let rejected = |key: &str, message: &str| ("cut", key.to_owned(), Err(message.to_owned()));
        assert_eq!(
            *events.borrow(),
            [
                rejected(PLAIN, BUSY_MESSAGE),
                rejected("Y2S1_VelvetShell", NOT_INSTALLED),
            ]
        );
        assert!(season.join("videos/intro.bik").exists());
        launcher.read_with(cx, |launcher, _| {
            assert_eq!(launcher.shears.busy_key(), None);
            assert_eq!(
                launcher.info.disk_usage(),
                Some(0.5),
                "every cut answer recounts"
            );
        });
    }
}
