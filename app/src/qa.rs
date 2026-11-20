use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{
    AnyWindowHandle, AppContext, HeadlessAppContext, KeyUpEvent, Keystroke, Modifiers, MouseButton,
    MouseDownEvent, MouseUpEvent, PlatformInput, ScrollDelta, ScrollEvent, point, px, size,
};

use crate::assets::Assets;
use crate::model::Launcher;
use crate::model::liberator::LiberatorEvent;
use backend::env::{Paths, TL_DLLS_COMMON, TL_LAUNCHER, TL_LOADERS, TL_TOML};

use crate::shell::{Page, Shell};
use crate::ui::toast;

const FRAME: Duration = Duration::from_millis(16);
const TOAST_GAP: Duration = Duration::from_millis(700);

fn shoot(cx: &mut HeadlessAppContext, window: AnyWindowHandle, path: &str, crop: (u32, u32)) {
    let frame = cx
        .update_window(window, |_, window, cx| {
            window.draw(cx).clear(cx);
            window.render_to_image()
        })
        .expect("window update")
        .expect("headless render");
    image::imageops::crop_imm(
        &frame,
        0,
        0,
        crop.0.min(frame.width()),
        crop.1.min(frame.height()),
    )
    .to_image()
    .save(path)
    .expect("screenshot file");
}

const LIBRARY: &str = "Y5S3_ShadowLegacy,Y9S2_HeatedMetal";
const ZONE: &str = "Europe/Berlin";
const API_CACHE: &str = include_str!("qa/api_cache.json");
const FETCHED: &str = "\"secs_since_epoch\": ";
const NOTES_RELEASE: &str = "https://api.github.com/repos/xeralin/ThrowbackLoader/releases";

struct Sandbox {
    home: PathBuf,
    paths: Paths,
    game: Option<Child>,
}

impl Sandbox {
    fn create() -> Self {
        let home = std::env::temp_dir().join(format!("throwback-qa-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let library = home.join("lib");
        std::fs::create_dir_all(&library).expect("sandbox library");
        let editions = std::env::var("TB_QA_LIBRARY").unwrap_or(LIBRARY.into());
        for edition in editions.split(',').filter(|edition| !edition.is_empty()) {
            let mut files = edition.split(':');
            let name = files.next().unwrap_or_default();
            let folder = library.join(name.trim_end_matches('?'));
            std::fs::create_dir_all(&folder).expect("sandbox edition");
            if !name.ends_with('?') {
                std::fs::write(folder.join(TL_LAUNCHER), "").expect("sandbox edition marker");
            }
            for file in files {
                std::fs::write(folder.join(file), "").expect("sandbox edition file");
            }
        }
        let steam = home.join(".local/share/Steam/steamapps/common");
        for proton in ["Proton - Experimental", "Proton 11.0", "Proton Hotfix"] {
            std::fs::create_dir_all(steam.join(proton)).expect("sandbox proton");
            std::fs::write(steam.join(proton).join("proton"), "").expect("sandbox proton");
        }
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("XDG_DATA_HOME", home.join("xdg"));
            std::env::set_var("APPIMAGE", home.join("ThrowbackLauncher.AppImage"));
            std::env::set_var("TZ", ZONE);
        }
        let paths = Paths::detect();
        std::fs::create_dir_all(paths.default_downloads_dir()).expect("sandbox downloads");
        let loader = paths.tl_dir();
        std::fs::create_dir_all(&loader).expect("sandbox tools");
        let now = SystemTime::now().duration_since(UNIX_EPOCH);
        let fetched = format!("{FETCHED}{}", now.expect("clock").as_secs());
        let mut answers = API_CACHE.replace(&format!("{FETCHED}0"), &fetched);
        if let Ok(notes) = std::env::var("TB_QA_NOTES") {
            let body = std::fs::read_to_string(notes).expect("notes fixture");
            let mut cache: serde_json::Value =
                serde_json::from_str(&answers).expect("release answers");
            cache[NOTES_RELEASE]["data"][0]["body"] = body.into();
            answers = cache.to_string();
        }
        std::fs::write(paths.api_cache_file(), answers).expect("sandbox release answers");
        if std::env::var("TB_QA_UPDATES").as_deref() == Ok("pending") {
            let files = TL_DLLS_COMMON.into_iter().chain(TL_LOADERS);
            for name in files.chain([TL_TOML, TL_LAUNCHER]) {
                std::fs::write(loader.join(name), "").expect("sandbox loader");
            }
        }
        let account = std::env::var("TB_QA_ACCOUNT")
            .map(|account| format!("steam_account = '{account}'\n"))
            .unwrap_or_default();
        let extra = std::env::var("TB_QA_SETTINGS")
            .map(|lines| lines.replace("\\n", "\n"))
            .unwrap_or_default();
        let liberator = std::env::var_os("TB_QA_LIBERATOR").is_some();
        std::fs::write(
            paths.settings_file(),
            format!(
                "[settings]\nlibraries = ['{}']\nliberator_enabled = {liberator}\ndiscord_rpc = false\n{account}{extra}\n",
                library.display()
            ),
        )
        .expect("sandbox settings");
        if let Ok(played) = std::env::var("TB_QA_PLAYTIME") {
            std::fs::write(paths.playtime_file(), played.replace("\\n", "\n"))
                .expect("sandbox play time");
        }
        let game = std::env::var("TB_QA_RUNNING").ok().map(|folder| {
            Command::new("sleep")
                .arg0(library.join(&folder).join("RainbowSix.exe"))
                .arg("600")
                .env("STEAM_COMPAT_DATA_PATH", paths.prefix_dir().join(&folder))
                .spawn()
                .expect("sandbox game")
        });
        Self { home, paths, game }
    }

    fn store(&self) -> backend::settings::Settings {
        backend::settings::Settings::load(&self.paths).expect("sandbox settings load")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        if let Some(mut game) = self.game.take() {
            let _ = game.kill();
            let _ = game.wait();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

pub fn run_from_env() -> bool {
    let Ok(path) = std::env::var("TB_QA_SHOT") else {
        return false;
    };
    let number = |name: &str, fallback: f32| {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(fallback)
    };
    let width = number("TB_QA_WIDTH", 1280.);
    let height = number("TB_QA_HEIGHT", 720.);
    let scale = number("TB_QA_SCALE", 1.);
    let crop = (
        (width * scale).round() as u32,
        (height * scale).round() as u32,
    );
    let page = match std::env::var("TB_QA_PAGE").as_deref() {
        Ok("download") => Page::Download,
        Ok("liberator") => Page::Liberator,
        Ok("settings") => Page::Settings,
        Ok("updates") => Page::Updates,
        _ => Page::Home,
    };

    let text_system = Arc::new(gpui_wgpu::CosmicTextSystem::new("Barlow"));
    let mut cx = HeadlessAppContext::with_platform(text_system, Arc::new(Assets), || {
        gpui_platform::current_headless_renderer()
    });
    cx.update(crate::init);
    let sandbox = Sandbox::create();
    cx.update(|cx| {
        let downloads = backend::manifest::downloads();
        let mut launcher = Launcher::new(sandbox.paths.clone(), sandbox.store(), downloads);
        if let Ok(plan) = std::env::var("TB_QA_DOWNLOAD") {
            launcher.download.stage(&plan);
        }
        if let Ok(plan) = std::env::var("TB_QA_RVPN") {
            launcher.rvpn.stage(&plan);
        }
        if let Ok(gif) = std::env::var("TB_QA_NYAN") {
            launcher.stage_nyan(std::fs::read(gif).expect("nyan gif"), cx);
        }
        let lines = std::env::var("TB_QA_LIBERATOR").unwrap_or_default();
        launcher.liberator.stage(&lines);
        launcher.settings.tray_available = std::env::var_os("TB_QA_TRAY").is_some();
        launcher.init(cx)
    });
    let viewport = size(px(width), px(height));
    let shell = cx.open_window(viewport, |window, cx| {
        cx.new(|cx| Shell::new(page.into(), window, cx))
    });
    let window: AnyWindowHandle = shell.expect("headless window").into();
    cx.update_window(window, |_, window, _| window.activate_window())
        .expect("window update");
    let exclusion = std::env::var_os("TB_QA_EXCLUSION").is_some();
    cx.update(|cx| {
        Launcher::handle(cx).update(cx, |launcher, cx| {
            launcher.check_updates(false, cx);
            if exclusion {
                cx.emit(LiberatorEvent::Exclusion);
            }
        });
    });
    if let Ok(toasts) = std::env::var("TB_QA_TOASTS") {
        let toasts: &'static str = toasts.leak();
        cx.update(|cx| {
            cx.spawn(async move |cx| {
                for entry in toasts.split('|').filter(|entry| !entry.is_empty()) {
                    let keyed = entry.strip_prefix("key:");
                    let (key, text) = match keyed.and_then(|rest| rest.split_once(':')) {
                        Some((key, text)) => (Some(key), text),
                        None => (None, entry),
                    };
                    cx.update(|cx| toast::show(text, key, cx));
                    cx.background_executor().timer(TOAST_GAP).await;
                }
            })
            .detach();
        });
    }
    cx.run_until_parked();
    cx.update_window(window, |_, window, _| window.set_scale_factor(scale))
        .expect("window update");
    cx.run_until_parked();

    let steps: &'static str = std::env::var("TB_QA_STEPS").unwrap_or_default().leak();
    for step in steps.split(';').filter(|step| !step.is_empty()) {
        let (kind, argument) = step.split_once(':').unwrap_or((step, ""));
        let position = || {
            let (x, y) = argument.split_once(',').expect("x,y");
            point(px(x.parse().expect("x")), px(y.parse().expect("y")))
        };
        match kind {
            "move" => {
                let position = position();
                cx.update_window(window, |_, window, cx| {
                    window.simulate_mouse_move(position, cx)
                })
                .expect("window update");
            }
            "click" | "down" | "up" | "double" | "triple" | "shift" | "right" => {
                let position = position();
                let button = match kind {
                    "right" => MouseButton::Right,
                    _ => MouseButton::Left,
                };
                let clicks = match kind {
                    "double" => 2,
                    "triple" => 3,
                    _ => 1,
                };
                let modifiers = Modifiers {
                    shift: kind == "shift",
                    ..Default::default()
                };
                cx.update_window(window, |_, window, cx| {
                    for click_count in 1..=clicks {
                        if kind != "up" {
                            window.dispatch_event(
                                PlatformInput::MouseDown(MouseDownEvent {
                                    button,
                                    position,
                                    modifiers,
                                    click_count,
                                    first_mouse: false,
                                }),
                                cx,
                            );
                        }
                        if kind != "down" {
                            window.dispatch_event(
                                PlatformInput::MouseUp(MouseUpEvent {
                                    button,
                                    position,
                                    modifiers,
                                    click_count,
                                }),
                                cx,
                            );
                        }
                    }
                })
                .expect("window update");
            }
            "key" => {
                let keystroke = Keystroke::parse(argument).expect("keystroke");
                cx.update_window(window, |_, window, cx| {
                    window.dispatch_keystroke(keystroke.clone(), cx);
                    window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke }), cx);
                })
                .expect("window update");
            }
            "wheel" => {
                let mut values = argument.split(',').map(|value| value.parse::<f32>());
                let mut value = || values.next().expect("x,y,dy").expect("number");
                let (x, y, dy) = (value(), value(), value());
                cx.update_window(window, |_, window, cx| {
                    window.dispatch_event(
                        PlatformInput::Scroll(ScrollEvent {
                            position: point(px(x), px(y)),
                            delta: ScrollDelta::Pixels(point(px(0.), px(-dy))),
                            ..Default::default()
                        }),
                        cx,
                    );
                })
                .expect("window update");
            }
            "notch" => {
                let mut values = argument.split(',').map(|value| value.parse::<f32>());
                let mut value = || values.next().expect("x,y,notches").expect("number");
                let (x, y, notches) = (value(), value(), value());
                cx.update_window(window, |_, window, cx| {
                    window.dispatch_event(
                        PlatformInput::Scroll(ScrollEvent {
                            position: point(px(x), px(y)),
                            delta: ScrollDelta::Lines(point(0., -3. * notches)),
                            ..Default::default()
                        }),
                        cx,
                    );
                })
                .expect("window update");
            }
            "wait" => {
                let total = Duration::from_millis(argument.parse().expect("milliseconds"));
                let mut elapsed = Duration::ZERO;
                let mut costs = Vec::new();
                while elapsed < total {
                    cx.advance_clock(FRAME);
                    elapsed += FRAME;
                    cx.run_until_parked();
                    let started = std::time::Instant::now();
                    cx.update_window(window, |_, window, cx| {
                        window.simulate_next_frame(cx);
                        window.draw(cx).clear(cx);
                    })
                    .expect("window update");
                    costs.push(started.elapsed().as_secs_f32() * 1000.);
                }
                if std::env::var_os("TB_QA_TIMES").is_some() {
                    let worst = costs.iter().copied().fold(0., f32::max);
                    let mean = costs.iter().sum::<f32>() / costs.len().max(1) as f32;
                    eprintln!(
                        "{step}: {} frames, mean {mean:.2} ms, worst {worst:.2} ms",
                        costs.len()
                    );
                }
            }
            "shot" => shoot(
                &mut cx,
                window,
                &path.replace(".png", &format!("-{argument}.png")),
                crop,
            ),
            _ => panic!("unknown step {step}"),
        }
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx))
            .expect("window update");
    }
    shoot(&mut cx, window, &path, crop);
    true
}
