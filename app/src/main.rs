#![cfg_attr(feature = "packaged", windows_subsystem = "windows")]

mod activity;
mod assets;
mod data;
mod failure;
mod instance;
mod modals;
mod model;
mod notices;
mod pages;
#[cfg(feature = "qa")]
mod qa;
mod shell;
mod tray;
mod ui;

use std::time::Duration;

use backend::env::{APP_NAME, DIR_NAME, IS_WINDOWS, Paths, RELAUNCH_ARG, UNINSTALL_ARG};
use backend::instance::Lock;
use backend::settings::Settings;
use gpui::{
    App, AppContext, Global, Pixels, TextRenderingMode, TitlebarOptions, Window, WindowBounds,
    WindowDecorations, WindowOptions, px, size,
};

const RELAUNCH_WAIT: Duration = Duration::from_secs(30);
const LEAST: (f32, f32) = (560., 480.);
const USUAL: [(f32, f32); 2] = [(940., 540.), (1280., 720.)];

struct Closed {
    bounds: WindowBounds,
    place: shell::Place,
}

impl Global for Closed {}

fn init(cx: &mut App) {
    assets::load_fonts(cx);
    cx.set_text_rendering_mode(TextRenderingMode::Grayscale);
    cx.set_global(ui::theme::Theme::for_action(ui::theme::BRAND));
    ui::init(cx);
}

fn open_window(cx: &mut App) {
    let least = size(ui::px(LEAST.0), ui::px(LEAST.1));
    let (bounds, place) = match cx.try_global::<Closed>() {
        Some(closed) => (closed.bounds, closed.place.clone()),
        None => {
            let fit = |room: Pixels, least: Pixels, usual: (f32, f32)| {
                let least = usual.0.max(f32::from(least));
                let wanted = (0.9 * f32::from(room)).round();
                px(wanted.clamp(least, usual.1.max(least)))
            };
            let screen = cx.primary_display().map(|display| display.visible_bounds());
            let [low, high] = USUAL;
            let extent = screen.map_or(size(px(high.0), px(high.1)), |screen| {
                size(
                    fit(screen.size.width, least.width, (low.0, high.0)),
                    fit(screen.size.height, least.height, (low.1, high.1)),
                )
            });
            let extent = ui::frame::framed(extent);
            (WindowBounds::centered(extent, cx), shell::Place::default())
        }
    };
    let options = WindowOptions {
        window_bounds: Some(bounds),
        window_min_size: Some(ui::frame::framed(least)),
        titlebar: Some(TitlebarOptions {
            title: Some(APP_NAME.into()),
            appears_transparent: true,
            ..Default::default()
        }),
        window_decorations: Some(WindowDecorations::Client),
        window_background: ui::frame::backdrop(),
        app_id: Some(DIR_NAME.into()),
        ..Default::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        window.on_window_should_close(cx, |window, cx| {
            remember(window, cx);
            true
        });
        cx.new(|cx| shell::Shell::new(place, window, cx))
    });
    if let Err(error) = opened {
        backend::log::fail("Window creation failed", format!("{error:#}"));
        cx.quit();
    }
}

fn remember(window: &mut Window, cx: &mut App) {
    let shell = window.root::<shell::Shell>().flatten();
    let place = shell.map(|shell| shell.read(cx).place(cx));
    cx.set_global(Closed {
        bounds: window.inner_window_bounds(),
        place: place.unwrap_or_default(),
    });
}

fn close_window(window: &mut Window, cx: &mut App) {
    remember(window, cx);
    window.remove_window();
}

fn show_window(cx: &mut App) {
    cx.activate(true);
    if cx.windows().is_empty() {
        open_window(cx);
    }
    if let Some(window) = cx.windows().first() {
        let _ = window.update(cx, |_, window, _| window.activate_window());
    }
}

fn run(launch: impl FnOnce(&mut App) + 'static) {
    let application = gpui_platform::application().with_assets(assets::Assets);
    application.run(move |cx| {
        if cfg!(target_os = "linux") && cx.graphical_environment().is_none() {
            backend::log::fail("Window creation failed", "WAYLAND_DISPLAY is not set");
            std::process::exit(1);
        }
        init(cx);
        launch(cx);
        if cx.windows().is_empty() {
            std::process::exit(1);
        }
    });
}

fn main() {
    #[cfg(target_os = "linux")]
    unsafe {
        std::env::set_var("ZED_FONTS_GAMMA", "1.0");
        std::env::set_var("ZED_FONTS_GRAYSCALE_ENHANCED_CONTRAST", "0");
    }

    #[cfg(feature = "qa")]
    if qa::run_from_env() {
        return;
    }

    let args: Vec<String> = std::env::args_os()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    #[cfg(windows)]
    match backend::self_install::hand_over(&args) {
        Ok(true) => return,
        Ok(false) => {}
        Err(message) => failure::show(message),
    }

    let paths = Paths::detect();
    backend::log::init(paths.errors_file());
    backend::log::install_panic_hook();
    let relaunched = args.iter().any(|arg| arg == RELAUNCH_ARG);
    if args.iter().any(|arg| arg == UNINSTALL_ARG) {
        #[cfg(windows)]
        backend::self_uninstall::run(&paths);
        return;
    }
    if let Err(error) = std::fs::create_dir_all(&paths.data_root) {
        failure::show(backend::log::fail("Data folder creation failed", error));
    }
    let wait = if relaunched {
        RELAUNCH_WAIT
    } else {
        Duration::ZERO
    };
    let Some(_lock) = Lock::acquire(&paths.lock_file(), wait) else {
        backend::instance::wake(&paths.instance_key());
        return;
    };
    let _ = std::fs::create_dir_all(paths.default_downloads_dir());
    let store = match Settings::load(&paths) {
        Ok(store) => store,
        Err(error) => failure::show(error.to_string()),
    };
    std::thread::spawn({
        let (paths, libraries) = (paths.clone(), store.libraries().to_vec());
        move || backend::steam::prune_prefixes(&paths, &libraries)
    });

    #[cfg(windows)]
    std::thread::spawn({
        let paths = paths.clone();
        move || {
            backend::self_update::clear_leftovers(&paths);
            backend::self_install::register(&paths.data_root);
        }
    });

    let instance_key = paths.instance_key();
    let downloads = backend::manifest::downloads();

    run(move |cx| {
        model::Launcher::new(paths, store, downloads).init(cx);
        cx.on_app_quit(|cx| {
            model::Launcher::handle(cx).update(cx, |launcher, cx| {
                launcher.download.shutdown();
                launcher.liberator.stop(cx);
                launcher.stop_presence();
                launcher.save_playtime(cx);
                launcher.rvpn.shutdown();
            });
            async {}
        })
        .detach();
        show_window(cx);
        instance::watch(&instance_key, cx);
        tray::init(cx);
        model::Launcher::handle(cx).update(cx, |launcher, cx| {
            if let Some(warning) = launcher.settings.warning() {
                ui::toast::show(warning.to_owned(), None, cx);
            }
            launcher.announce_update_outcome(cx);
            launcher.check_updates(false, cx);
            launcher.prepare_liberator(IS_WINDOWS, cx);
        });
    });
}

#[cfg(test)]
mod tests {
    use gpui::{AnyWindowHandle, TestAppContext, VisualTestContext, Window};

    use super::*;

    fn shown(cx: &mut TestAppContext) -> AnyWindowHandle {
        cx.update(show_window);
        cx.run_until_parked();
        let [window] = cx.windows()[..] else {
            panic!("one window");
        };
        window
    }

    #[gpui::test]
    fn the_window_comes_back_as_it_was_closed(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        std::fs::create_dir_all(&paths.data_root).unwrap();
        std::fs::write(paths.settings_file(), "[settings]\ndiscord_rpc = false\n").unwrap();
        cx.update(|cx| {
            init(cx);
            let store = Settings::load(&paths).unwrap();
            model::Launcher::new(paths, store, backend::manifest::downloads()).init(cx);
        });
        let state = |window: AnyWindowHandle, cx: &mut TestAppContext| {
            let read = |_, window: &mut Window, cx: &mut App| {
                let shell = window.root::<shell::Shell>().flatten().unwrap();
                (shell.read(cx).place(cx), window.bounds().size)
            };
            cx.update_window(window, read).unwrap()
        };

        let first = shown(cx);
        assert_eq!(cx.update(|cx| cx.active_window()), Some(first));
        assert_eq!(shown(cx), first);
        assert!(state(first, cx).0 == shell::Place::default());

        let size = size(px(1000.), px(600.));
        cx.simulate_window_resize(first, size);
        cx.update_window(first, |_, window, cx| {
            shell::open_season("Y1S0_Vanilla", false, window, cx)
        })
        .unwrap();
        let place = state(first, cx).0;
        assert!(place != shell::Place::default());

        let mut closing = VisualTestContext::from_window(first, cx);
        assert!(closing.simulate_close());
        closing.update(|window, _| window.remove_window());
        cx.run_until_parked();
        assert!(cx.windows().is_empty());

        let second = shown(cx);
        assert_ne!(second, first);
        assert!(state(second, cx) == (place.clone(), size));

        cx.update_window(second, |_, window, cx| close_window(window, cx))
            .unwrap();
        cx.run_until_parked();
        assert!(cx.windows().is_empty());
        assert!(state(shown(cx), cx) == (place, size));
    }
}
