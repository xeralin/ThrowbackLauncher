use std::path::Path;
use std::sync::Arc;

use backend::env::GIB;
use backend::heatedmetal::hm_crash_log;
use backend::log;
use backend::manifest::{installed_path, local_downloads};
use backend::settings::Flag;
use backend::shears::folder_size;
use gpui::{App, AppContext, Context, Image, ImageFormat, RenderImage, SharedString};

use super::{Launcher, Scan};

#[derive(Default)]
pub struct Info {
    usage: Option<f64>,
    scan: Scan,
    clipboard: Option<arboard::Clipboard>,
    cat: Option<Arc<RenderImage>>,
    fetching: bool,
}

impl Info {
    pub fn disk_usage(&self) -> Option<f64> {
        self.usage
    }
}

fn copy_failed(error: &arboard::Error) -> SharedString {
    log::fail("Crash log copy failed", error).into()
}

fn gibibytes(bytes: u64) -> f64 {
    (bytes as f64 / GIB as f64 * 10.).round() / 10.
}

fn animated(gif: Vec<u8>, cx: &App) -> Option<Arc<RenderImage>> {
    let picture = Image::from_bytes(ImageFormat::Gif, gif);
    picture.to_image_data(cx.svg_renderer()).ok()
}

impl Launcher {
    pub fn nyan(&self) -> Option<Arc<RenderImage>> {
        let cat = self.info.cat.as_ref();
        cat.filter(|_| self.settings.flag(Flag::BarNyan)).cloned()
    }

    pub fn load_nyan(&mut self, cx: &mut Context<Self>) {
        let wanted = self.settings.flag(Flag::BarNyan) && self.info.cat.is_none();
        if !wanted || std::mem::replace(&mut self.info.fetching, true) {
            return;
        }
        let cat = cx.background_spawn(async { backend::nyan::cat() });
        cx.spawn(async move |this, cx| {
            let cat = cat.await;
            this.update(cx, |this, cx| {
                this.info.fetching = false;
                this.info.cat = cat.and_then(|gif| animated(gif, cx));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn toggle_nyan(&mut self, cx: &mut Context<Self>) {
        let on = !self.settings.flag(Flag::BarNyan);
        self.settings.set_flag(Flag::BarNyan, on, cx);
        self.load_nyan(cx);
    }

    #[cfg(any(test, feature = "qa"))]
    pub fn stage_nyan(&mut self, gif: Vec<u8>, cx: &App) {
        self.info.cat = animated(gif, cx);
    }

    pub fn copy_crash_log(&mut self, key: &str) -> Result<(), SharedString> {
        let folder = installed_path(self.settings.roots(), key, true);
        let files = folder.map(|folder| hm_crash_log(&folder));
        let files = files.unwrap_or_default();
        if files.is_empty() {
            return Err("Crash log not found".into());
        }
        let clipboard = match &mut self.info.clipboard {
            Some(clipboard) => clipboard,
            closed => {
                let opened = arboard::Clipboard::new();
                closed.insert(opened.map_err(|error| copy_failed(&error))?)
            }
        };
        let copied = clipboard.set().file_list(&files);
        copied.map_err(|error| copy_failed(&error))
    }

    pub fn refresh_disk_usage(&mut self, cx: &mut Context<Self>) {
        if !self.info.scan.begin() {
            return;
        }
        let libraries = self.settings.roots().to_vec();
        let scan = cx.background_spawn(async move {
            let folders = local_downloads(&libraries);
            gibibytes(folders.iter().map(|folder| folder_size(folder)).sum())
        });
        cx.spawn(async move |this, cx| {
            let usage = scan.await;
            this.update(cx, |this, cx| this.disk_usage_scanned(usage, cx))
                .ok();
        })
        .detach();
    }

    fn disk_usage_scanned(&mut self, usage: f64, cx: &mut Context<Self>) {
        if self.info.usage != Some(usage) {
            self.info.usage = Some(usage);
            cx.notify();
        }
        if self.info.scan.end() {
            self.refresh_disk_usage(cx);
        }
    }

    pub fn open_library(&self, path: &str, cx: &mut Context<Self>) {
        let target = Path::new(path);
        if target.exists() {
            cx.open_with_system(target);
        }
    }

    pub fn open_season_folder(&self, key: &str, hm: bool, cx: &mut Context<Self>) {
        if let Some(target) = installed_path(self.settings.roots(), key, hm) {
            cx.open_with_system(&target);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use backend::env::Paths;
    use backend::settings::Settings as Store;
    use gpui::TestAppContext;

    use super::*;

    #[test]
    fn usage_is_rounded_to_a_tenth() {
        for (bytes, usage) in [
            (0, 0.),
            (GIB, 1.),
            (GIB / 20 - 1, 0.),
            (GIB / 20, 0.),
            (GIB / 20 + 1, 0.1),
            (GIB / 4, 0.3),
            (GIB * 3 / 4, 0.8),
            (GIB * 3 / 20, 0.1),
            (GIB * 5 / 4, 1.3),
            (GIB * 7 / 4, 1.8),
            (999 * GIB + GIB * 3 / 4, 999.8),
            (GIB * 123 / 10, 12.3),
            (87 * GIB + GIB / 2, 87.5),
        ] {
            assert_eq!(gibibytes(bytes), usage, "{bytes}");
        }
    }

    #[gpui::test]
    fn the_cat_shows_only_while_the_bar_setting_is_on(cx: &mut TestAppContext) {
        const DOT: &[u8] = b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff!\xf9\x04\x01\0\0\0\0,\0\0\0\0\x01\0\x01\0\0\x02\x02D\x01\0;";
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        let store = Store::load(&paths).unwrap();
        let launcher = cx.new(|_| Launcher::new(paths.clone(), store, Vec::new()));
        let shown = |cx: &mut TestAppContext| {
            launcher.read_with(cx, |launcher, _| launcher.nyan().is_some())
        };
        launcher.update(cx, |launcher, cx| {
            launcher.stage_nyan(b"not a picture".to_vec(), cx);
            assert!(launcher.info.cat.is_none());
            launcher.stage_nyan(DOT.to_vec(), cx);
        });
        assert!(!shown(cx));
        launcher.update(cx, |launcher, cx| launcher.toggle_nyan(cx));
        assert!(shown(cx));
        assert!(Store::load(&paths).unwrap().flag(Flag::BarNyan));
        launcher.update(cx, |launcher, cx| launcher.toggle_nyan(cx));
        assert!(!shown(cx));
        assert!(!Store::load(&paths).unwrap().flag(Flag::BarNyan));
    }

    #[gpui::test]
    fn disk_usage_sums_the_season_folders_of_every_library(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        let library = dir.path().join("lib");
        let season = library.join("Y5S3_ShadowLegacy");
        fs::create_dir_all(&season).unwrap();
        fs::create_dir_all(library.join("notes")).unwrap();
        fs::create_dir_all(&paths.data_root).unwrap();
        fs::write(library.join("notes").join("big"), vec![0; 1 << 20]).unwrap();
        fs::write(
            paths.settings_file(),
            format!("[settings]\nlibraries = ['{}']\n", library.display()),
        )
        .unwrap();
        let store = Store::load(&paths).unwrap();
        let launcher = cx.new(|_| Launcher::new(paths.clone(), store, Vec::new()));
        let usage = |cx: &mut TestAppContext| launcher.read_with(cx, |it, _| it.info.disk_usage());
        assert_eq!(usage(cx), None);
        launcher.update(cx, |launcher, cx| launcher.refresh_disk_usage(cx));
        cx.run_until_parked();
        assert_eq!(usage(cx), Some(0.));
        let file = fs::File::create(season.join("data.forge")).unwrap();
        file.set_len(GIB * 3 / 2).unwrap();
        launcher.update(cx, |launcher, cx| {
            launcher.refresh_disk_usage(cx);
            launcher.refresh_disk_usage(cx);
        });
        cx.run_until_parked();
        assert_eq!(usage(cx), Some(1.5));
    }
}
