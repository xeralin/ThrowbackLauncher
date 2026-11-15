use backend::env::{BUSY_MESSAGE, UPDATE_RUNNING};
use backend::steam;
use gpui::{AppContext, Context, EventEmitter, SharedString};

use super::{Edition, Launcher};

pub enum UninstallEvent {
    Done(Result<(), SharedString>),
}

#[derive(Default)]
pub struct Uninstaller {
    busy: Option<Edition>,
}

impl EventEmitter<UninstallEvent> for Launcher {}

impl Uninstaller {
    pub fn busy_key(&self) -> Option<&str> {
        self.busy.as_ref().map(|edition| edition.key.as_str())
    }
}

#[cfg(test)]
impl Uninstaller {
    pub fn busy_with(key: &str) -> Self {
        Self {
            busy: Some(Edition {
                key: key.to_owned(),
                hm: false,
            }),
        }
    }
}

impl Launcher {
    pub fn uninstall_season(&mut self, key: &str, hm: bool, cx: &mut Context<Self>) {
        if self.uninstall.busy.is_some() {
            return;
        }
        if self.download.busy_with(key) {
            return self.uninstall_done(Err(BUSY_MESSAGE.into()), cx);
        }
        if self.update.busy() {
            return self.uninstall_done(Err(UPDATE_RUNNING.into()), cx);
        }
        let edition = Edition {
            key: key.to_owned(),
            hm,
        };
        self.uninstall.busy = Some(edition.clone());
        let (paths, libraries) = (self.paths.clone(), self.settings.roots().to_vec());
        let removal = cx.background_spawn(async move {
            steam::uninstall(&paths, &libraries, &edition.key, edition.hm)
        });
        cx.spawn(async move |this, cx| {
            let outcome = removal.await.map_err(|error| error.to_string().into());
            this.update(cx, |this, cx| {
                if let Some(edition) = this.uninstall.busy.take()
                    && outcome.is_ok()
                {
                    this.dequeue(&edition.key, edition.hm, cx);
                }
                this.uninstall_done(outcome, cx);
            })
            .ok();
        })
        .detach();
    }

    fn uninstall_done(&mut self, outcome: Result<(), SharedString>, cx: &mut Context<Self>) {
        cx.emit(UninstallEvent::Done(outcome));
        self.start_next(cx);
        self.refresh_disk_usage(cx);
        self.refresh_home(cx);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use backend::env::{GIB, NOT_INSTALLED, Paths, TL_LAUNCHER};
    use backend::manifest;
    use backend::settings::Settings as Store;
    use gpui::{Entity, TestAppContext};

    use super::*;
    use crate::model::download::Downloader;
    use crate::model::update::Updater;

    const KEY: &str = "Y5S3_ShadowLegacy";

    type Events = Rc<RefCell<Vec<Result<(), String>>>>;

    struct Rig {
        launcher: Entity<Launcher>,
        events: Events,
        library: PathBuf,
        prefix: PathBuf,
    }

    fn install(folder: &Path) {
        fs::create_dir_all(folder).unwrap();
        fs::write(folder.join(TL_LAUNCHER), "").unwrap();
        let data = fs::File::create(folder.join("data.forge")).unwrap();
        data.set_len(GIB / 5).unwrap();
    }

    fn rig(cx: &mut TestAppContext, dir: &Path) -> Rig {
        let paths = Paths::new(dir.join("asset"), dir.join("data"));
        let library = paths.default_downloads_dir();
        let prefix = paths.prefix_dir().join(KEY);
        install(&library.join(KEY));
        fs::create_dir_all(&prefix).unwrap();
        let store = Store::load(&paths).unwrap();
        let launcher = cx.new(|_| Launcher::new(paths, store, manifest::downloads()));
        let events = Events::default();
        cx.update(|cx| {
            let seen = events.clone();
            cx.subscribe(&launcher, move |_, event: &UninstallEvent, _| {
                let UninstallEvent::Done(outcome) = event;
                let outcome = outcome.clone().map_err(|message| message.to_string());
                seen.borrow_mut().push(outcome);
            })
            .detach();
        });
        Rig {
            launcher,
            events,
            library,
            prefix,
        }
    }

    #[gpui::test]
    fn an_uninstall_removes_the_edition_and_refreshes_what_shows_it(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let rig = rig(cx, dir.path());
        rig.launcher.update(cx, |launcher, cx| {
            launcher.refresh_home(cx);
            launcher.refresh_disk_usage(cx);
        });
        cx.run_until_parked();
        let shown = |cx: &mut TestAppContext| {
            rig.launcher.read_with(cx, |launcher, _| {
                let editions = launcher.home.editions().unwrap().len();
                (editions, launcher.info.disk_usage())
            })
        };
        assert_eq!(shown(cx), (1, Some(0.2)));

        rig.launcher.update(cx, |launcher, cx| {
            launcher.uninstall_season(KEY, false, cx);
            assert_eq!(launcher.uninstall.busy_key(), Some(KEY));
            launcher.uninstall_season("Y1S0_Vanilla", false, cx);
            launcher.uninstall_season(KEY, false, cx);
        });
        assert!(rig.events.borrow().is_empty());
        assert!(rig.library.join(KEY).exists());

        cx.run_until_parked();
        assert_eq!(*rig.events.borrow(), [Ok(())]);
        assert!(!rig.library.join(KEY).exists());
        assert!(!rig.prefix.exists());
        assert_eq!(shown(cx), (0, Some(0.)));
        rig.launcher.read_with(cx, |launcher, _| {
            assert_eq!(launcher.uninstall.busy_key(), None)
        });
    }

    #[gpui::test]
    fn a_running_download_or_update_rejects_the_uninstall(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let rig = rig(cx, dir.path());
        rig.launcher.update(cx, |launcher, cx| {
            launcher.update = Updater::busy_now();
            launcher.uninstall_season(KEY, false, cx);
            launcher.download = Downloader::downloading(KEY, true);
            launcher.uninstall_season(KEY, false, cx);
            launcher.uninstall_season("Y1S0_Vanilla", false, cx);
            assert_eq!(launcher.uninstall.busy_key(), None);
        });
        cx.run_until_parked();
        assert_eq!(
            *rig.events.borrow(),
            [
                Err(UPDATE_RUNNING.to_owned()),
                Err(BUSY_MESSAGE.to_owned()),
                Err(UPDATE_RUNNING.to_owned()),
            ]
        );
        assert!(rig.library.join(KEY).join(TL_LAUNCHER).exists());
        assert!(rig.prefix.exists());
        let usage = rig
            .launcher
            .read_with(cx, |launcher, _| launcher.info.disk_usage());
        assert_eq!(usage, Some(0.2), "a rejected uninstall recounts as well");
    }

    #[gpui::test]
    fn a_refused_removal_reports_its_reason_and_the_part_stays_usable(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let rig = rig(cx, dir.path());
        rig.launcher.update(cx, |launcher, cx| {
            launcher.uninstall_season("Y1S0_Vanilla", false, cx)
        });
        cx.run_until_parked();
        assert_eq!(*rig.events.borrow(), [Err(NOT_INSTALLED.to_owned())]);

        install(&rig.library.join("Y5S3_HeatedMetal"));
        rig.launcher
            .update(cx, |launcher, cx| launcher.uninstall_season(KEY, true, cx));
        cx.run_until_parked();
        assert_eq!(rig.events.borrow().last(), Some(&Ok(())));
        assert!(!rig.library.join("Y5S3_HeatedMetal").exists());
        assert!(rig.library.join(KEY).exists());
        assert!(
            rig.prefix.exists(),
            "the other edition still uses the prefix"
        );
    }
}
