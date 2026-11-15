use std::path::{Path, PathBuf};

use backend::cheatengine::{
    CheatEngineStatus, add_cheat_engine, cheat_engine_status, install_cheat_engine,
    remove_cheat_engine,
};
use backend::env::Paths;
use backend::steam::{list_protons, resolve_proton};
use gpui::{AppContext, Context, EventEmitter, Result, SharedString};

use super::Launcher;
use super::dialogs::pick_file;

pub enum CheatEngineEvent {
    Done,
    Error(SharedString),
}

type Setup = fn(&Paths, &[PathBuf], &str, &[String; 2], &Path) -> Result<()>;

pub struct CheatEngine {
    busy: bool,
    setup: Setup,
}

impl EventEmitter<CheatEngineEvent> for Launcher {}

fn setup(
    paths: &Paths,
    libraries: &[PathBuf],
    key: &str,
    choices: &[String; 2],
    installer: &Path,
) -> Result<()> {
    let protons = list_protons(paths);
    let proton = resolve_proton(&protons, &choices[0], &choices[1]);
    install_cheat_engine(paths, libraries, key, proton, installer)
}

impl Default for CheatEngine {
    fn default() -> Self {
        Self { busy: false, setup }
    }
}

impl CheatEngine {
    pub fn busy(&self) -> bool {
        self.busy
    }
}

impl Launcher {
    pub fn cheat_engine_status(&self, key: &str) -> CheatEngineStatus {
        cheat_engine_status(&self.paths, self.settings.roots(), key)
    }

    pub fn add_cheat_engine(&self, key: &str, cx: &mut Context<Self>) {
        if let Err(error) = add_cheat_engine(&self.paths, self.settings.roots(), key) {
            cx.emit(CheatEngineEvent::Error(error.to_string().into()));
        }
    }

    pub fn remove_cheat_engine(&self, key: &str, cx: &mut Context<Self>) {
        if let Err(error) = remove_cheat_engine(self.settings.roots(), key) {
            cx.emit(CheatEngineEvent::Error(error.to_string().into()));
        }
    }

    pub fn install_cheat_engine(&self, key: &str, cx: &mut Context<Self>) {
        if self.cheat_engine.busy {
            return;
        }
        let picked = pick_file("Select Cheat Engine installer", cx);
        let key = key.to_owned();
        cx.spawn(async move |this, cx| {
            let picked = picked.await;
            this.update(cx, |this, cx| match picked {
                Ok(Some(installer)) => this.run_cheat_engine_installer(key, installer, cx),
                Ok(None) => {}
                Err(message) => cx.emit(CheatEngineEvent::Error(message)),
            })
            .ok();
        })
        .detach();
    }

    fn run_cheat_engine_installer(
        &mut self,
        key: String,
        installer: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if self.cheat_engine.busy {
            return;
        }
        self.cheat_engine.busy = true;
        cx.notify();
        let (paths, libraries) = (self.paths.clone(), self.settings.roots().to_vec());
        let settings = &self.settings;
        let choices = [settings.season_proton(&key), settings.stored_proton()].map(str::to_owned);
        let setup = self.cheat_engine.setup;
        let installed = cx
            .background_spawn(async move { setup(&paths, &libraries, &key, &choices, &installer) });
        cx.spawn(async move |this, cx| {
            let outcome = installed.await;
            this.update(cx, |this, cx| {
                this.cheat_engine.busy = false;
                if let Err(error) = outcome {
                    cx.emit(CheatEngineEvent::Error(error.to_string().into()));
                }
                cx.emit(CheatEngineEvent::Done);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::io;
    use std::rc::Rc;

    use backend::env::{NOT_INSTALLED, TL_LAUNCHER, TL_TOML};
    use backend::settings::Settings as Store;
    use backend::throwbackloader::read_tl_autorun;
    use gpui::{Entity, TestAppContext};

    use super::*;

    const KEY: &str = "Y5S3_ShadowLegacy";
    const EXE: &str = r"C:\Program Files\Cheat Engine 7.5\Cheat Engine.exe";
    const SETTINGS: &str = "[settings]\nproton = 'GE-Proton9-27'\nseason_proton = { Y5S3_ShadowLegacy = 'proton_9' }\n";

    struct Rig {
        launcher: Entity<Launcher>,
        events: Rc<RefCell<Vec<String>>>,
        paths: Paths,
        season: PathBuf,
    }

    fn place_exe(paths: &Paths) {
        let prefix = paths.prefix_dir().join(KEY).join("pfx").join("drive_c");
        let folder = prefix.join("Program Files").join("Cheat Engine 7.5");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("Cheat Engine.exe"), "MZ").unwrap();
    }

    fn fake_setup(
        paths: &Paths,
        libraries: &[PathBuf],
        key: &str,
        choices: &[String; 2],
        installer: &Path,
    ) -> Result<()> {
        let name = installer.file_name().unwrap().to_string_lossy();
        fs::write(
            paths.data_root.join("ran"),
            format!("{key} {choices:?} {name}"),
        )?;
        if name == "broken.exe" {
            let message = "Cheat Engine was not found after the installer closed";
            return Err(io::Error::other(message).into());
        }
        place_exe(paths);
        add_cheat_engine(paths, libraries, key)
    }

    fn rig(cx: &mut TestAppContext, dir: &Path) -> Rig {
        let paths = Paths::new(dir.join("asset"), dir.join("data"));
        let season = paths.default_downloads_dir().join(KEY);
        fs::create_dir_all(&season).unwrap();
        fs::write(season.join(TL_LAUNCHER), "").unwrap();
        fs::write(season.join(TL_TOML), "[Launch]\nautorun = []\n").unwrap();
        fs::write(paths.settings_file(), SETTINGS).unwrap();
        let store = Store::load(&paths).unwrap();
        let launcher = cx.new(|_| {
            let mut launcher = Launcher::new(paths.clone(), store, Vec::new());
            launcher.cheat_engine.setup = fake_setup;
            launcher
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let seen = events.clone();
            cx.subscribe(&launcher, move |_, event: &CheatEngineEvent, _| {
                seen.borrow_mut().push(match event {
                    CheatEngineEvent::Done => "done".to_owned(),
                    CheatEngineEvent::Error(message) => message.to_string(),
                });
            })
            .detach();
            let seen = events.clone();
            cx.observe(&launcher, move |launcher, cx| {
                let busy = launcher.read(cx).cheat_engine.busy();
                seen.borrow_mut().push(format!("busy {busy}"));
            })
            .detach();
        });
        Rig {
            launcher,
            events,
            paths,
            season,
        }
    }

    fn status(rig: &Rig, cx: &mut TestAppContext) -> (bool, bool) {
        let status = rig
            .launcher
            .read_with(cx, |launcher, _| launcher.cheat_engine_status(KEY));
        (status.enabled, status.present)
    }

    #[gpui::test]
    fn the_switch_adds_and_removes_the_autorun_entry(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let rig = rig(cx, dir.path());
        assert_eq!(status(&rig, cx), (false, false));
        rig.launcher.update(cx, |launcher, cx| {
            launcher.add_cheat_engine(KEY, cx);
            launcher.add_cheat_engine("Y1S0_Vanilla", cx);
            launcher.remove_cheat_engine("Y1S0_Vanilla", cx);
        });
        assert_eq!(
            *rig.events.borrow(),
            ["Cheat Engine not found", NOT_INSTALLED, NOT_INSTALLED]
        );

        place_exe(&rig.paths);
        assert_eq!(status(&rig, cx), (false, true));
        rig.launcher
            .update(cx, |launcher, cx| launcher.add_cheat_engine(KEY, cx));
        assert_eq!(read_tl_autorun(&rig.season), [EXE]);
        assert_eq!(status(&rig, cx), (true, true));
        rig.launcher
            .update(cx, |launcher, cx| launcher.remove_cheat_engine(KEY, cx));
        assert!(read_tl_autorun(&rig.season).is_empty());
        assert_eq!(status(&rig, cx), (false, true));
        assert_eq!(rig.events.borrow().len(), 3);
    }

    #[gpui::test]
    fn the_installer_runs_in_the_season_prefix_and_reports_done(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let rig = rig(cx, dir.path());
        let pick = |cx: &mut TestAppContext, name: Option<&str>| {
            rig.launcher
                .update(cx, |launcher, cx| launcher.install_cheat_engine(KEY, cx));
            cx.simulate_path_prompt_response(|options| {
                assert!(options.files && !options.directories && !options.multiple);
                assert_eq!(
                    options.prompt.as_deref(),
                    Some("Select Cheat Engine installer")
                );
                name.map(|name| vec![dir.path().join(name)])
            });
            cx.run_until_parked();
        };
        let ran = || fs::read_to_string(rig.paths.data_root.join("ran")).ok();

        pick(cx, None);
        assert!(rig.events.borrow().is_empty());
        assert_eq!(ran(), None);

        pick(cx, Some("broken.exe"));
        assert_eq!(
            rig.events.take(),
            [
                "busy true",
                "Cheat Engine was not found after the installer closed",
                "done",
                "busy false",
            ]
        );
        let choices = r#"Y5S3_ShadowLegacy ["proton_9", "GE-Proton9-27"]"#;
        assert_eq!(ran(), Some(format!("{choices} broken.exe")));

        pick(cx, Some("CheatEngine75.exe"));
        assert_eq!(rig.events.take(), ["busy true", "done", "busy false"]);
        assert_eq!(status(&rig, cx), (true, true));
        assert_eq!(ran(), Some(format!("{choices} CheatEngine75.exe")));
    }

    #[gpui::test]
    fn a_running_installer_blocks_the_next_one(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let rig = rig(cx, dir.path());
        rig.launcher.update(cx, |launcher, cx| {
            launcher.install_cheat_engine(KEY, cx);
            launcher.cheat_engine.busy = true;
            launcher.install_cheat_engine(KEY, cx);
        });
        cx.simulate_path_prompt_response(|_| Some(vec![dir.path().join("CheatEngine75.exe")]));
        cx.run_until_parked();
        assert!(!cx.did_prompt_for_paths(), "no second dialog while busy");
        assert!(!rig.paths.data_root.join("ran").exists());
        assert!(rig.events.borrow().is_empty());
    }
}
