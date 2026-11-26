use backend::manifest::{self, SeasonEntry};
use gpui::{AppContext, Context};

use super::{Launcher, Scan};

#[derive(Default)]
pub struct Home {
    editions: Option<Vec<SeasonEntry>>,
    scan: Scan,
}

impl Home {
    pub fn editions(&self) -> Option<&[SeasonEntry]> {
        self.editions.as_deref()
    }
}

impl Launcher {
    pub fn seasons(&self) -> Vec<SeasonEntry> {
        manifest::seasons(&self.downloads)
    }

    pub fn refresh_home(&mut self, cx: &mut Context<Self>) {
        if !self.home.scan.begin() {
            return;
        }
        let downloads = self.downloads.clone();
        let libraries = self.settings.roots().to_vec();
        let scan = cx.background_spawn(async move { manifest::home(&downloads, &libraries) });
        cx.spawn(async move |this, cx| {
            let editions = scan.await;
            this.update(cx, |this, cx| this.home_scanned(editions, cx))
                .ok();
        })
        .detach();
    }

    fn home_scanned(&mut self, editions: Vec<SeasonEntry>, cx: &mut Context<Self>) {
        if self.home.editions.as_ref() != Some(&editions) {
            self.home.editions = Some(editions);
            cx.notify();
        }
        if self.home.scan.end() {
            self.refresh_home(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use backend::env::{Paths, TL_LAUNCHER};
    use backend::settings::Settings as Store;
    use gpui::TestAppContext;

    use super::*;

    #[gpui::test]
    fn home_lists_what_the_libraries_hold(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        let library = paths.default_downloads_dir();
        fs::create_dir_all(library.join("Y1S0_Vanilla")).unwrap();
        fs::write(library.join("Y1S0_Vanilla").join(TL_LAUNCHER), "").unwrap();
        fs::create_dir_all(library.join("Y9S2_HeatedMetal")).unwrap();
        let store = Store::load(&paths).unwrap();
        let launcher = cx.new(|_| Launcher::new(paths.clone(), store, manifest::downloads()));
        assert!(launcher.read_with(cx, |launcher, _| launcher.home.editions().is_none()));

        launcher.update(cx, |launcher, cx| {
            launcher.refresh_home(cx);
            launcher.refresh_home(cx);
        });
        cx.run_until_parked();
        let shown = launcher.read_with(cx, |launcher, _| {
            let editions = launcher.home.editions().unwrap();
            editions
                .iter()
                .map(|edition| (edition.title(edition.hm), edition.partial))
                .collect::<Vec<_>>()
        });
        assert_eq!(
            shown,
            [
                ("Y1S0 Vanilla".to_owned(), false),
                ("Y9S2 Heated Metal".to_owned(), true)
            ]
        );

        fs::remove_dir_all(library.join("Y9S2_HeatedMetal")).unwrap();
        launcher.update(cx, |launcher, cx| launcher.refresh_home(cx));
        cx.run_until_parked();
        assert_eq!(
            launcher.read_with(cx, |launcher, _| launcher.home.editions().unwrap().len()),
            1
        );
        assert_eq!(
            launcher.read_with(cx, |launcher, _| launcher.seasons().len()),
            manifest::downloads().len()
        );
    }
}
