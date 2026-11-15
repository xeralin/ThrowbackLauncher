use std::mem;

use async_channel::Sender;
use backend::settings::Flag;
use gpui::{App, QuitMode};

use crate::model::Launcher;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux as host;
#[cfg(windows)]
use windows as host;

#[derive(Clone, Copy)]
enum Event {
    Open,
    Quit,
    Wanted(bool),
}

const ENTRIES: [(&str, Event); 2] = [("Open", Event::Open), ("Quit", Event::Quit)];

pub fn init(cx: &mut App) {
    follow(host::available, host::show, cx);
}

fn follow<Handle: 'static>(
    available: impl AsyncFn() -> bool + 'static,
    show: impl AsyncFn(Sender<Event>) -> Option<Handle> + 'static,
    cx: &mut App,
) {
    let launcher = Launcher::handle(cx);
    let (sender, events) = async_channel::unbounded();
    let mut wanted = launcher.read(cx).settings.flag(Flag::CloseToTray);
    let _ = sender.try_send(Event::Wanted(wanted));
    cx.observe(&launcher, {
        let sender = sender.clone();
        move |launcher, cx| {
            let flag = launcher.read(cx).settings.flag(Flag::CloseToTray);
            if mem::replace(&mut wanted, flag) != flag {
                let _ = sender.try_send(Event::Wanted(flag));
            }
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        let mut icon = None;
        while let Ok(event) = events.recv().await {
            match event {
                Event::Open => cx.update(crate::show_window),
                Event::Quit => cx.update(|cx| cx.quit()),
                Event::Wanted(wanted) => {
                    drop(icon.take());
                    let offered = available().await;
                    if wanted && offered {
                        icon = show(sender.clone()).await;
                    }
                    let quit = if icon.is_some() {
                        QuitMode::Explicit
                    } else {
                        QuitMode::LastWindowClosed
                    };
                    cx.update(|cx| {
                        cx.set_quit_mode(quit);
                        launcher.update(cx, |launcher, cx| {
                            let known = &mut launcher.settings.tray_available;
                            if mem::replace(known, offered) != offered {
                                cx.notify();
                            }
                        });
                    });
                }
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::rc::Rc;

    use backend::env::Paths;
    use backend::settings::Settings;
    use gpui::TestAppContext;
    use tempfile::TempDir;

    use super::*;

    #[derive(Default)]
    struct Host {
        log: RefCell<Vec<&'static str>>,
        events: RefCell<Option<Sender<Event>>>,
    }

    struct Shown(Rc<Host>);

    impl Drop for Shown {
        fn drop(&mut self) {
            self.0.log.borrow_mut().push("hide");
        }
    }

    fn stage(cx: &mut TestAppContext, available: bool, wanted: bool) -> (Rc<Host>, TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        fs::create_dir_all(&paths.data_root).unwrap();
        let settings = format!("[settings]\ndiscord_rpc = false\nclose_to_tray = {wanted}\n");
        fs::write(paths.settings_file(), settings).unwrap();
        let host = Rc::new(Host::default());
        cx.update(|cx| {
            crate::init(cx);
            let store = Settings::load(&paths).unwrap();
            Launcher::new(paths, store, Vec::new()).init(cx);
            let host = host.clone();
            let show = async move |events| {
                host.log.borrow_mut().push("show");
                host.events.replace(Some(events));
                Some(Shown(host.clone()))
            };
            follow(async move || available, show, cx);
        });
        cx.run_until_parked();
        (host, dir)
    }

    fn switch(on: bool, cx: &mut TestAppContext) {
        cx.update(|cx| {
            Launcher::handle(cx).update(cx, |launcher, cx| {
                launcher.settings.set_flag(Flag::CloseToTray, on, cx);
            });
        });
        cx.run_until_parked();
    }

    fn offered(cx: &mut TestAppContext) -> bool {
        cx.update(|cx| Launcher::handle(cx).read(cx).settings.tray_available)
    }

    #[gpui::test]
    fn the_icon_follows_the_setting(cx: &mut TestAppContext) {
        let (host, _dir) = stage(cx, true, true);
        assert_eq!(*host.log.borrow(), ["show"]);
        assert!(offered(cx));

        switch(false, cx);
        assert_eq!(*host.log.borrow(), ["show", "hide"]);
        assert!(offered(cx));

        switch(true, cx);
        cx.update(|cx| Launcher::handle(cx).update(cx, |_, cx| cx.notify()));
        cx.run_until_parked();
        assert_eq!(*host.log.borrow(), ["show", "hide", "show"]);
    }

    #[gpui::test]
    fn the_icon_opens_the_window_once(cx: &mut TestAppContext) {
        let (host, _dir) = stage(cx, true, true);
        let events = host.events.take().unwrap();
        assert!(cx.windows().is_empty());
        for _ in 0..2 {
            events.try_send(Event::Open).unwrap();
            cx.run_until_parked();
            assert_eq!(cx.windows().len(), 1);
        }
    }

    #[gpui::test]
    fn no_icon_without_a_tray(cx: &mut TestAppContext) {
        let (host, _dir) = stage(cx, false, true);
        assert!(host.log.borrow().is_empty());
        assert!(!offered(cx));

        switch(false, cx);
        switch(true, cx);
        assert!(host.log.borrow().is_empty());
    }

    #[gpui::test]
    fn the_tray_is_offered_while_the_setting_is_off(cx: &mut TestAppContext) {
        let (host, _dir) = stage(cx, true, false);
        assert!(host.log.borrow().is_empty());
        assert!(offered(cx));
    }
}
