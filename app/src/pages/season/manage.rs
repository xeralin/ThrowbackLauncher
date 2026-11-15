use std::f32::consts::PI;

use crate::ui::px;
use backend::env::IS_WINDOWS;
use backend::settings::Color;
use gpui::{
    AnyElement, App, Context, Div, ElementId, InteractiveElement, IntoElement, ParentElement,
    Stateful, StatefulInteractiveElement, Styled, Transformation, Window, deferred, div,
    prelude::FluentBuilder, radians,
};

use super::SeasonPage;
use super::dialogs::{Dialog, Pending};
use crate::model::Launcher;
use crate::model::download::{Downloader, State};
use crate::model::launch::{Installs, Launch, Renderer};
use crate::ui::button::{copy_button, icon_button, primary, secondary};
use crate::ui::icon::{CHEVRON_DOWN, CLOCK, FOLDER, PROTON, REMOVE, icon};
use crate::ui::kit::{OVERLAY, box_text, busy, disable, focus_ring_inside, icon_box, panel};
use crate::ui::log_box::log_box;
use crate::ui::motion::{self, EASE_IN_OUT, EASE_OUT, EASE_OUT_CUBIC, FAST, SLOW, sub};
use crate::ui::scroll::reveal;
use crate::ui::theme::{
    BORDER, FONT_BODY_SEMIBOLD, FONT_DISPLAY_BOLD, REM, SURFACE, SURFACE_2, TEXT, TEXT_MUTED,
    TEXT_UI, Theme, Typeface, hex, lifted,
};
use crate::ui::toast;
use crate::ui::transfer::transfer_bar;

const RENDERER_ARGS: [&str; 2] = ["/vulkan", "/dx11"];
const OPTION: f32 = 25.;
const PERCENT: f32 = 1.2 * REM;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Flags {
    pub downloading: bool,
    pub downloading_season: bool,
    pub downloading_edition: bool,
    pub verifying_edition: bool,
    pub queued: bool,
    pub state: State,
    pub playing_edition: bool,
    pub running_season: bool,
    pub playing_season: bool,
    pub updating: bool,
}

impl Flags {
    pub fn of(key: &str, hm: bool, download: &Downloader, launch: &Launch, updating: bool) -> Self {
        let active = download.active();
        let season = active.is_some_and(|edition| edition.key == key);
        let edition = active.is_some_and(|edition| edition.is(key, hm));
        let (running, launching) = (launch.running(), launch.launching());
        let running_season = running.iter().any(|edition| edition.key == key);
        Self {
            downloading: download.running(),
            downloading_season: season && download.running(),
            downloading_edition: edition && download.running(),
            verifying_edition: edition && download.verifying(),
            queued: download.waits(key, hm),
            state: if edition {
                download.state()
            } else {
                State::Idle
            },
            playing_edition: running
                .iter()
                .chain(launching)
                .any(|edition| edition.is(key, hm)),
            running_season,
            playing_season: running_season || launching.is_some_and(|edition| edition.key == key),
            updating,
        }
    }

    pub fn transferring(&self) -> bool {
        self.downloading_edition || self.state == State::Paused
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Act {
    Continue,
    Pause,
    Cancel,
    Remove,
    Stop,
    Play,
    Dequeue,
    Verify,
    UpdateHm,
    Shears,
    Download,
    SwitchHm,
    SwitchTb,
}

impl Act {
    fn id(self) -> &'static str {
        match self {
            Act::Continue => "season-continue",
            Act::Pause => "season-pause",
            Act::Cancel => "season-cancel",
            Act::Remove => "season-remove",
            Act::Stop => "season-stop",
            Act::Play => "season-play",
            Act::Dequeue => "season-dequeue",
            Act::Verify => "season-verify",
            Act::UpdateHm => "season-update-hm",
            Act::Shears => "season-shears",
            Act::Download => "season-download",
            Act::SwitchHm => "season-switch-hm",
            Act::SwitchTb => "season-switch-tb",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Button {
    pub act: Act,
    pub label: &'static str,
    pub primary: bool,
    pub disabled: bool,
    pub busy: bool,
}

fn button(act: Act, label: &'static str, primary: bool, disabled: bool) -> Button {
    Button {
        act,
        label,
        primary,
        disabled,
        busy: false,
    }
}

pub(super) fn buttons(
    flags: &Flags,
    installs: &Installs,
    hm: bool,
    hm_beta: bool,
    cuts: bool,
) -> Vec<Button> {
    let (install, other) = (installs.edition(hm), installs.edition(!hm));
    let playing = flags.playing_season;
    if flags.transferring() {
        if flags.state == State::Paused {
            return vec![
                button(Act::Continue, "Continue", true, false),
                button(Act::Remove, "Remove", false, false),
            ];
        }
        let (act, label) = if flags.verifying_edition || install.installed {
            (Act::Cancel, "Cancel")
        } else {
            (Act::Pause, "Pause")
        };
        return vec![Button {
            busy: flags.state != State::Downloading,
            ..button(act, label, false, false)
        }];
    }
    if install.installed {
        let mut row = vec![if flags.playing_edition {
            button(Act::Stop, "Stop", true, false)
        } else {
            let held = flags.downloading_season || flags.updating;
            button(Act::Play, "Play", true, held)
        }];
        row.push(if flags.queued {
            button(Act::Dequeue, "Remove from queue", false, false)
        } else if flags.downloading {
            button(Act::Verify, "Queue verify", false, playing)
        } else {
            button(Act::Verify, "Verify", false, playing)
        });
        if hm && hm_beta {
            let held = playing || flags.downloading;
            row.push(button(Act::UpdateHm, "Update HM", false, held));
        }
        if !hm {
            let held = flags.downloading_season || playing || !cuts;
            row.push(button(Act::Shears, "Shears", false, held));
        }
        return row;
    }
    if flags.queued {
        return vec![button(Act::Dequeue, "Remove from queue", false, false)];
    }
    let label = match (install.partial, flags.downloading) {
        (true, true) => "Queue verify",
        (true, false) => "Verify",
        (false, true) => "Queue download",
        (false, false) => "Download",
    };
    let mut row = vec![button(
        Act::Download,
        label,
        true,
        install.partial && playing,
    )];
    if install.partial {
        row.push(button(Act::Remove, "Remove", false, playing));
    } else if other.installed {
        let held = playing || flags.downloading;
        row.push(if hm {
            button(Act::SwitchHm, "Switch to HM", false, held)
        } else {
            button(Act::SwitchTb, "Switch to TB", false, held)
        });
    }
    row
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tool {
    Clock,
    CrashLog,
    Proton,
    Folder,
    Uninstall,
}

pub(super) struct Shelf {
    pub time: bool,
    pub tools: Vec<(Tool, bool)>,
}

pub(super) fn shelf(
    flags: &Flags,
    installs: &Installs,
    hm: bool,
    minutes: u64,
    any_playtime: bool,
    protons: Option<usize>,
) -> Shelf {
    let install = installs.edition(hm);
    let mut shelf = Shelf {
        time: false,
        tools: Vec::new(),
    };
    if flags.transferring() {
        return shelf;
    }
    shelf.time = minutes > 0;
    if !install.installed {
        return shelf;
    }
    if minutes == 0 {
        shelf.tools.push((Tool::Clock, any_playtime));
    }
    if hm {
        shelf.tools.push((Tool::CrashLog, installs.hm.crash_log));
    }
    if let Some(count) = protons {
        shelf.tools.push((Tool::Proton, count > 0));
    }
    shelf.tools.push((Tool::Folder, true));
    let free = !flags.downloading_season && !flags.playing_season;
    shelf.tools.push((Tool::Uninstall, free));
    shelf
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Bar {
    pub percent: u32,
    pub progress: f32,
    pub paused: bool,
}

pub(super) fn bar(state: State, percent: Option<f64>) -> Option<Bar> {
    let shown = matches!(state, State::Downloading | State::Paused);
    percent.filter(|_| shown).map(|progress| Bar {
        percent: progress.floor() as u32,
        progress: progress as f32,
        paused: state == State::Paused,
    })
}

pub(super) fn format_playtime(minutes: u64) -> String {
    if minutes < 60 {
        format!("{minutes} min")
    } else {
        let tenths = (minutes + 3) / 6;
        format!("{}.{} h", tenths / 10, tenths % 10)
    }
}

fn renderers(renderer: Renderer) -> [(&'static str, &'static str); 2] {
    match renderer {
        Renderer::Vulkan => [("", "DirectX"), ("/vulkan", "Vulkan")],
        Renderer::Dx11 => [("", "DirectX 12"), ("/dx11", "DirectX 11")],
    }
}

fn active_renderer(args: &str) -> &str {
    let mut tokens = args.split_whitespace();
    tokens
        .find(|token| RENDERER_ARGS.contains(token))
        .unwrap_or_default()
}

fn with_renderer(args: &str, renderer: &str) -> String {
    let rest = args
        .split_whitespace()
        .filter(|token| !RENDERER_ARGS.contains(token));
    let chosen = (!renderer.is_empty()).then_some(renderer);
    rest.chain(chosen).collect::<Vec<_>>().join(" ")
}

impl SeasonPage {
    pub(super) fn flags(&self, launcher: &Launcher) -> Flags {
        Flags::of(
            &self.season.key,
            self.hm,
            &launcher.download,
            &launcher.launch,
            launcher.update.busy(),
        )
    }

    pub(super) fn act(&mut self, act: Act, window: &mut Window, cx: &mut Context<Self>) {
        let (key, hm) = (self.season.key.clone(), self.hm);
        let launcher = self.launcher.clone();
        match act {
            Act::Continue => launcher.update(cx, |launcher, cx| launcher.set_paused(false, cx)),
            Act::Pause => launcher.update(cx, |launcher, cx| launcher.set_paused(true, cx)),
            Act::Cancel => launcher.update(cx, |launcher, cx| launcher.cancel_download(cx)),
            Act::Remove => self.show(Dialog::RemoveDownload, window, cx),
            Act::Stop => launcher.update(cx, |launcher, cx| launcher.stop_season(&key, cx)),
            Act::Play => launcher.update(cx, |launcher, cx| launcher.launch_season(&key, hm, cx)),
            Act::Dequeue => launcher.update(cx, |launcher, cx| launcher.dequeue(&key, hm, cx)),
            Act::Verify => self.ensure_hm(Pending::Verify, window, cx),
            Act::UpdateHm => launcher.update(cx, |launcher, cx| launcher.update_hm(&key, cx)),
            Act::Shears => self.show(Dialog::Shears, window, cx),
            Act::Download => self.prompt_download(window, cx),
            Act::SwitchHm => self.ensure_hm(Pending::SwitchToHm, window, cx),
            Act::SwitchTb => launcher.update(cx, |launcher, cx| launcher.remove_hm(&key, cx)),
        }
    }

    fn set_renderer(&mut self, renderer: &str, cx: &mut Context<Self>) {
        let args = with_renderer(&self.game_args.args, renderer);
        let launcher = self.launcher.read(cx);
        match launcher.set_game_args(&self.season.key, self.hm, &args) {
            Ok(()) => self.game_args.args = args,
            Err(message) => toast::show(message, None, cx),
        }
        cx.notify();
    }

    fn control(
        &self,
        button: Button,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let control = if button.primary {
            primary(button.act.id(), button.label, window, cx)
        } else {
            secondary(button.act.id(), button.label, window, cx)
        };
        if button.busy {
            return busy(control, window, cx);
        }
        let act = button.act;
        control
            .on_click(cx.listener(move |this, _, window, cx| this.act(act, window, cx)))
            .when(button.disabled, disable)
    }

    fn split(
        &self,
        play: Button,
        renderer: Renderer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = ElementId::from("season-renderer");
        let open = if self.renderers { 1. } else { 0. };
        let turn = motion::transition(sub(&id, "turn"), open, SLOW, EASE_IN_OUT, window, cx);
        let unfold = motion::transition(sub(&id, "unfold"), open, SLOW, EASE_OUT_CUBIC, window, cx);
        let over = motion::hover(&id, SLOW, window, cx).over;
        let chevron = icon(CHEVRON_DOWN, 14.)
            .text_color(Theme::get(cx).action_text.solid())
            .with_transformation(Transformation::rotate(radians(PI * turn)));
        let caret = primary(id.clone(), "", window, cx)
            .w(px(24.))
            .px(px(0.))
            .justify_center()
            .rounded_l(px(0.))
            .child(chevron)
            .on_click(cx.listener(|this, _, _, cx| {
                this.renderers = !this.renderers;
                cx.notify();
            }))
            .when(play.disabled, disable);
        let options = renderers(renderer);
        let active = active_renderer(&self.game_args.args);
        let theme = Theme::get(cx);
        let dim = theme.action_dim;
        let mut list = div().flex().flex_col().p(px(7.));
        for (arg, label) in options {
            let option = sub(&id, label);
            let hover = motion::hover(&option, FAST, window, cx);
            let chosen = arg == active;
            let lit = if chosen { 1. } else { hover.amount };
            list = list.child(
                div()
                    .id(option)
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(OPTION))
                    .rounded(px(6.))
                    .border_l_2()
                    .border_color(BORDER.alpha(0.))
                    .px(px(8.))
                    .face(FONT_BODY_SEMIBOLD, TEXT_UI)
                    .whitespace_nowrap()
                    .text_color(TEXT_MUTED.mix(TEXT, lit).solid())
                    .map(|option| {
                        if chosen {
                            option.bg(dim)
                        } else {
                            option.bg(SURFACE_2.alpha(hover.amount))
                        }
                    })
                    .tab_index(0)
                    .focus_visible(focus_ring_inside)
                    .map(reveal)
                    .map(|element| hover.follow(element))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.renderers = false;
                        if !chosen {
                            this.set_renderer(arg, cx);
                        }
                        cx.notify();
                    }))
                    .child(div().top(px(-1.)).child(label)),
            );
        }
        let height = 2. * OPTION + 16.;
        let menu = (unfold > 0.).then(|| {
            deferred(
                div()
                    .id(sub(&id, "menu"))
                    .occlude()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(36.))
                    .h(px(height * unfold))
                    .overflow_hidden()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(BORDER.solid())
                    .bg(SURFACE.solid())
                    .shadow(lifted())
                    .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                        if !over {
                            this.renderers = false;
                            cx.notify();
                        }
                    }))
                    .child(list),
            )
            .with_priority(OVERLAY)
        });
        div()
            .track_focus(&self.split)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .child(self.control(play, window, cx).rounded_r(px(0.)))
            .child(caret)
            .children(menu)
            .into_any_element()
    }

    pub(super) fn use_tool(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        match tool {
            Tool::Clock => self.show(Dialog::Playtime, window, cx),
            Tool::CrashLog => {
                self.copy_crash_log(cx);
            }
            Tool::Proton => self.open_proton(window, cx),
            Tool::Folder => {
                let (key, hm) = (self.season.key.clone(), self.hm);
                self.launcher
                    .update(cx, |launcher, cx| launcher.open_season_folder(&key, hm, cx));
            }
            Tool::Uninstall => self.show(Dialog::Uninstall { busy: false }, window, cx),
        }
    }

    fn tool(
        &self,
        tool: Tool,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (id, glyph) = match tool {
            Tool::Clock => ("season-clock", CLOCK),
            Tool::CrashLog => {
                let page = cx.weak_entity();
                let copy = move |_: &mut Window, cx: &mut App| {
                    let copied = page.update(cx, |page, cx| page.copy_crash_log(cx));
                    copied.unwrap_or(false)
                };
                let control = copy_button("season-crash-log", copy, window, cx);
                return control.when(!enabled, disable);
            }
            Tool::Proton => ("season-proton", PROTON),
            Tool::Folder => ("season-folder", FOLDER),
            Tool::Uninstall => ("season-uninstall", REMOVE),
        };
        icon_button(id, glyph, window, cx)
            .on_click(cx.listener(move |this, _, window, cx| this.use_tool(tool, window, cx)))
            .when(!enabled, disable)
    }

    pub(super) fn manage(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let (key, hm) = (self.season.key.as_str(), self.hm);
        let (flags, minutes, any_playtime, protons, bar, tones) = {
            let launcher = self.launcher.read(cx);
            let flags = self.flags(launcher);
            let played = launcher.playtime();
            let mine = played.iter().find(|entry| entry.edition.is(key, hm));
            let download = &launcher.download;
            let settings = &launcher.settings;
            let offered = settings.protons.as_ref().map_or(1, Vec::len);
            (
                flags,
                mine.map_or(0, |entry| entry.minutes),
                !played.is_empty(),
                (!IS_WINDOWS).then_some(offered),
                bar(flags.state, download.percent()),
                (
                    hex(settings.color(Color::BarFill)),
                    hex(settings.color(Color::BarStripe)),
                ),
            )
        };
        let cuts = !super::dialogs::cuts(&self.scan).is_empty();
        let mut actions = div().flex().flex_none().items_center().gap(px(8.));
        for button in buttons(&flags, &self.installs, hm, self.season.hm_beta, cuts) {
            actions = actions.child(match self.game_args.renderer {
                Some(renderer) if button.act == Act::Play => {
                    self.split(button, renderer, window, cx)
                }
                _ => self.control(button, window, cx).into_any_element(),
            });
        }
        let mut transfer = div().flex().flex_1().min_w_0().items_center().gap(px(12.));
        if let Some(bar) = bar {
            let fill = motion::progress("season-fill", bar.progress, SLOW, EASE_OUT, window, cx);
            let track = transfer_bar("season-transfer", fill, tones, bar.paused, window, cx);
            transfer = transfer.child(track.flex_1().min_w_0()).child(
                div()
                    .flex_none()
                    .top(px(1.))
                    .face(FONT_DISPLAY_BOLD, PERCENT)
                    .line_height(px(PERCENT))
                    .text_color(TEXT.solid())
                    .child(format!("{}%", bar.percent)),
            );
        }
        let shelf = shelf(&flags, &self.installs, hm, minutes, any_playtime, protons);
        let time = shelf.time.then(|| {
            panel().flex().flex_none().h(px(32.)).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .pl(px(0.35 * REM))
                    .pr(px(0.6 * REM))
                    .child(self.tool(Tool::Clock, true, window, cx))
                    .child(box_text(format_playtime(minutes))),
            )
        });
        let tools = (!shelf.tools.is_empty()).then(|| {
            let tools = shelf.tools.iter();
            let tools = tools.map(|(tool, enabled)| self.tool(*tool, *enabled, window, cx));
            icon_box()
                .bg(SURFACE.solid())
                .children(tools.collect::<Vec<_>>())
        });
        let side = (time.is_some() || tools.is_some()).then(|| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .ml_auto()
                .children(time)
                .children(tools)
        });
        let log = (!self.log.is_empty() && flags.state != State::Idle).then(|| {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .mt(px(12.))
                .child(log_box("season-log", &self.log, window, cx))
        });
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.))
                    .mt(px(16.))
                    .child(actions)
                    .child(transfer)
                    .children(side),
            )
            .children(log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Edition;
    use crate::model::launch::Install;

    const KEY: &str = "Y5S3_ShadowLegacy";

    fn edition(key: &str, hm: bool) -> Edition {
        Edition {
            key: key.into(),
            hm,
        }
    }

    fn loading(state: State, hm: bool, verifying: bool) -> Downloader {
        Downloader::with(state, Some(edition(KEY, hm)), verifying, Vec::new())
    }

    fn idle() -> Launch {
        Launch::with(Vec::new(), None)
    }

    fn flags(hm: bool, download: &Downloader, launch: &Launch) -> Flags {
        Flags::of(KEY, hm, download, launch, false)
    }

    fn installed() -> Install {
        Install {
            installed: true,
            ..Install::default()
        }
    }

    fn partial() -> Install {
        Install {
            partial: true,
            ..Install::default()
        }
    }

    fn installs(tb: Install, hm: Install) -> Installs {
        Installs {
            tb,
            hm,
            prefix: false,
        }
    }

    fn row(flags: &Flags, installs: &Installs, hm: bool) -> Vec<(&'static str, bool, bool, bool)> {
        buttons(flags, installs, hm, hm, true)
            .into_iter()
            .map(|button| (button.label, button.primary, button.disabled, button.busy))
            .collect()
    }

    #[test]
    fn flags_tell_the_season_from_the_edition() {
        let still = idle();
        let other = flags(false, &loading(State::Downloading, true, true), &still);
        assert!(other.downloading && other.downloading_season);
        assert!(!other.downloading_edition && !other.verifying_edition);
        assert_eq!(other.state, State::Idle);
        assert!(!other.transferring());

        let mine = flags(true, &loading(State::Applying, true, true), &still);
        assert!(mine.downloading_edition && mine.verifying_edition && mine.transferring());
        assert_eq!(mine.state, State::Applying);

        let paused = flags(false, &loading(State::Paused, false, false), &still);
        assert!(!paused.downloading && !paused.downloading_season && paused.transferring());
        let failed = flags(false, &loading(State::Failed, false, false), &still);
        assert!(!failed.transferring());
        assert_eq!(failed.state, State::Failed);

        let elsewhere = Downloader::with(
            State::Downloading,
            Some(edition("Y1S0_Vanilla", false)),
            false,
            vec![edition(KEY, true)],
        );
        let waiting = flags(true, &elsewhere, &still);
        assert!(waiting.queued && waiting.downloading && !waiting.downloading_season);
        assert!(!flags(false, &elsewhere, &still).queued);
    }

    #[test]
    fn a_running_or_starting_edition_holds_the_whole_season() {
        let quiet = Downloader::default();
        let running = Launch::with(vec![edition(KEY, true)], None);
        let heated = flags(true, &quiet, &running);
        assert!(heated.playing_edition && heated.running_season && heated.playing_season);
        let throwback = flags(false, &quiet, &running);
        assert!(!throwback.playing_edition && throwback.running_season);
        assert!(throwback.playing_season);

        let launching = Launch::with(Vec::new(), Some(edition(KEY, false)));
        let starting = flags(false, &quiet, &launching);
        assert!(starting.playing_edition && starting.playing_season && !starting.running_season);
        let beside = flags(true, &quiet, &launching);
        assert!(!beside.playing_edition && beside.playing_season);

        let elsewhere = Launch::with(vec![edition("Y1S0_Vanilla", false)], None);
        assert!(!flags(false, &quiet, &elsewhere).playing_season);
    }

    #[test]
    fn a_transfer_shows_pause_or_cancel_and_pulses_outside_the_download_step() {
        let still = idle();
        let fresh = installs(partial(), Install::default());
        let kept = installs(installed(), Install::default());
        let state = |state, verifying| flags(false, &loading(state, false, verifying), &still);
        assert_eq!(
            row(&state(State::Downloading, false), &fresh, false),
            [("Pause", false, false, false)]
        );
        for held in [State::Preparing, State::Applying] {
            assert_eq!(
                row(&state(held, false), &fresh, false),
                [("Pause", false, false, true)]
            );
            assert_eq!(
                row(&state(held, true), &kept, false),
                [("Cancel", false, false, true)]
            );
        }
        assert_eq!(
            row(&state(State::Downloading, true), &fresh, false),
            [("Cancel", false, false, false)]
        );
        assert_eq!(
            row(&state(State::Downloading, false), &kept, false),
            [("Cancel", false, false, false)]
        );
        assert_eq!(
            row(&state(State::Paused, false), &fresh, false),
            [
                ("Continue", true, false, false),
                ("Remove", false, false, false)
            ]
        );
    }

    #[test]
    fn an_installed_edition_plays_verifies_and_offers_its_extras() {
        let still = idle();
        let quiet = Downloader::default();
        let both = installs(installed(), installed());
        assert_eq!(
            row(&flags(false, &quiet, &still), &both, false),
            [
                ("Play", true, false, false),
                ("Verify", false, false, false),
                ("Shears", false, false, false)
            ]
        );
        assert_eq!(
            row(&flags(true, &quiet, &still), &both, true),
            [
                ("Play", true, false, false),
                ("Verify", false, false, false),
                ("Update HM", false, false, false)
            ]
        );
        let stable = buttons(&flags(true, &quiet, &still), &both, true, false, true);
        assert_eq!(stable.len(), 2);
        let bare = buttons(&flags(false, &quiet, &still), &both, false, false, false);
        assert_eq!(
            (bare[2].act, bare[2].disabled),
            (Act::Shears, true),
            "nothing to cut"
        );

        let updating = Flags::of(KEY, false, &quiet, &still, true);
        assert_eq!(
            row(&updating, &both, false)[0],
            ("Play", true, true, false),
            "greyed out, not hidden"
        );

        let other = flags(false, &loading(State::Downloading, true, false), &still);
        assert_eq!(
            row(&other, &both, false),
            [
                ("Play", true, true, false),
                ("Queue verify", false, false, false),
                ("Shears", false, true, false)
            ]
        );
        let apart = Downloader::with(
            State::Downloading,
            Some(edition("Y1S0_Vanilla", false)),
            false,
            vec![edition(KEY, true)],
        );
        assert_eq!(
            row(&flags(true, &apart, &still), &both, true),
            [
                ("Play", true, false, false),
                ("Remove from queue", false, false, false),
                ("Update HM", false, true, false)
            ]
        );
        assert_eq!(
            row(&flags(false, &apart, &still), &both, false),
            [
                ("Play", true, false, false),
                ("Queue verify", false, false, false),
                ("Shears", false, false, false)
            ]
        );
    }

    #[test]
    fn a_running_season_stops_its_edition_and_locks_every_change() {
        let quiet = Downloader::default();
        let both = installs(installed(), installed());
        let running = Launch::with(vec![edition(KEY, true)], None);
        assert_eq!(
            row(&flags(true, &quiet, &running), &both, true),
            [
                ("Stop", true, false, false),
                ("Verify", false, true, false),
                ("Update HM", false, true, false)
            ]
        );
        assert_eq!(
            row(&flags(false, &quiet, &running), &both, false),
            [
                ("Play", true, false, false),
                ("Verify", false, true, false),
                ("Shears", false, true, false)
            ]
        );
        let launching = Launch::with(Vec::new(), Some(edition(KEY, false)));
        assert_eq!(
            row(&flags(false, &quiet, &launching), &both, false)[0],
            ("Stop", true, false, false)
        );

        let half = installs(partial(), installed());
        assert_eq!(
            row(&flags(false, &quiet, &running), &half, false),
            [
                ("Verify", true, true, false),
                ("Remove", false, true, false)
            ]
        );
        let lone = installs(Install::default(), installed());
        assert_eq!(
            row(&flags(false, &quiet, &running), &lone, false),
            [
                ("Download", true, false, false),
                ("Switch to TB", false, true, false)
            ]
        );
    }

    #[test]
    fn a_missing_edition_downloads_resumes_switches_or_leaves_the_queue() {
        let still = idle();
        let quiet = Downloader::default();
        let nothing = installs(Install::default(), Install::default());
        assert_eq!(
            row(&flags(false, &quiet, &still), &nothing, false),
            [("Download", true, false, false)]
        );
        let apart = Downloader::with(
            State::Downloading,
            Some(edition("Y1S0_Vanilla", false)),
            false,
            vec![edition(KEY, true)],
        );
        assert_eq!(
            row(&flags(false, &apart, &still), &nothing, false),
            [("Queue download", true, false, false)]
        );
        assert_eq!(
            row(&flags(true, &apart, &still), &nothing, true),
            [("Remove from queue", false, false, false)]
        );

        let half = installs(partial(), Install::default());
        assert_eq!(
            row(&flags(false, &quiet, &still), &half, false),
            [
                ("Verify", true, false, false),
                ("Remove", false, false, false)
            ]
        );
        assert_eq!(
            row(&flags(false, &apart, &still), &half, false),
            [
                ("Queue verify", true, false, false),
                ("Remove", false, false, false)
            ]
        );
        let failed = flags(false, &loading(State::Failed, false, false), &still);
        assert_eq!(
            row(&failed, &half, false)[0],
            ("Verify", true, false, false)
        );

        let throwback = installs(installed(), Install::default());
        let heated = buttons(&flags(true, &quiet, &still), &throwback, true, true, true);
        assert_eq!(
            heated.iter().map(|button| button.act).collect::<Vec<_>>(),
            [Act::Download, Act::SwitchHm]
        );
        assert_eq!(
            row(&flags(true, &apart, &still), &throwback, true),
            [("Remove from queue", false, false, false)]
        );
        let busy = Downloader::with(
            State::Downloading,
            Some(edition("Y1S0_Vanilla", false)),
            false,
            Vec::new(),
        );
        assert_eq!(
            row(&flags(true, &busy, &still), &throwback, true),
            [
                ("Queue download", true, false, false),
                ("Switch to HM", false, true, false)
            ]
        );
        let metal = installs(Install::default(), installed());
        assert_eq!(
            row(&flags(false, &quiet, &still), &metal, false),
            [
                ("Download", true, false, false),
                ("Switch to TB", false, false, false)
            ]
        );
        let halves = installs(partial(), partial());
        assert_eq!(row(&flags(true, &quiet, &still), &halves, true).len(), 2);
    }

    #[test]
    fn buttons_of_different_meaning_never_share_an_id() {
        let acts = [
            Act::Continue,
            Act::Pause,
            Act::Cancel,
            Act::Remove,
            Act::Stop,
            Act::Play,
            Act::Dequeue,
            Act::Verify,
            Act::UpdateHm,
            Act::Shears,
            Act::Download,
            Act::SwitchHm,
            Act::SwitchTb,
        ];
        let mut ids: Vec<_> = acts.iter().map(|act| act.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), acts.len());
    }

    #[test]
    fn the_bar_needs_a_percentage_of_the_last_step_and_stands_still_when_paused() {
        let running = bar(State::Downloading, Some(42.9));
        assert_eq!(
            running,
            Some(Bar {
                percent: 42,
                progress: 42.9,
                paused: false
            })
        );
        let held = bar(State::Paused, Some(0.5));
        assert_eq!(held.map(|bar| (bar.percent, bar.paused)), Some((0, true)));
        assert_eq!(bar(State::Downloading, None), None);
        for state in [
            State::Idle,
            State::Preparing,
            State::Applying,
            State::Failed,
        ] {
            assert_eq!(bar(state, Some(42.)), None);
        }
    }

    #[test]
    fn the_right_side_follows_the_play_time_rules() {
        let still = idle();
        let quiet = Downloader::default();
        let free = flags(true, &quiet, &still);
        let logged = Install {
            crash_log: true,
            ..installed()
        };
        let both = installs(installed(), logged);
        let look = |flags: &Flags, installs: &Installs, hm, minutes, any, protons| {
            let shelf = shelf(flags, installs, hm, minutes, any, protons);
            (shelf.time, shelf.tools)
        };
        assert_eq!(
            look(&free, &both, true, 744, true, Some(3)),
            (
                true,
                vec![
                    (Tool::CrashLog, true),
                    (Tool::Proton, true),
                    (Tool::Folder, true),
                    (Tool::Uninstall, true)
                ]
            )
        );
        assert_eq!(
            look(&free, &both, false, 0, true, None),
            (
                false,
                vec![
                    (Tool::Clock, true),
                    (Tool::Folder, true),
                    (Tool::Uninstall, true)
                ]
            )
        );
        let plain = installs(installed(), installed());
        assert_eq!(
            look(&free, &plain, true, 0, false, Some(0)),
            (
                false,
                vec![
                    (Tool::Clock, false),
                    (Tool::CrashLog, false),
                    (Tool::Proton, false),
                    (Tool::Folder, true),
                    (Tool::Uninstall, true)
                ]
            )
        );
        let absent = installs(Install::default(), installed());
        assert_eq!(look(&free, &absent, false, 12, true, None), (true, vec![]));
        assert_eq!(look(&free, &absent, false, 0, true, None), (false, vec![]));

        let running = Launch::with(vec![edition(KEY, false)], None);
        let locked = look(&flags(true, &quiet, &running), &both, true, 5, true, None);
        assert_eq!(locked.1.last(), Some(&(Tool::Uninstall, false)));
        let other = flags(true, &loading(State::Downloading, false, false), &still);
        let locked = look(&other, &both, true, 5, true, None);
        assert_eq!(locked.1.last(), Some(&(Tool::Uninstall, false)));

        let paused = flags(true, &loading(State::Paused, true, false), &still);
        assert_eq!(look(&paused, &both, true, 744, true, None), (false, vec![]));
    }

    #[test]
    fn play_time_reads_as_minutes_or_tenths_of_hours() {
        for (minutes, text) in [
            (1, "1 min"),
            (59, "59 min"),
            (60, "1.0 h"),
            (63, "1.1 h"),
            (69, "1.2 h"),
            (75, "1.3 h"),
            (90, "1.5 h"),
            (744, "12.4 h"),
            (6000, "100.0 h"),
        ] {
            assert_eq!(format_playtime(minutes), text);
        }
    }

    #[test]
    fn only_the_renderer_token_of_the_game_arguments_changes() {
        assert_eq!(active_renderer(""), "");
        assert_eq!(active_renderer("-skipintro  /vulkan -x"), "/vulkan");
        assert_eq!(active_renderer("/dx11"), "/dx11");
        assert_eq!(with_renderer("", "/vulkan"), "/vulkan");
        assert_eq!(with_renderer("-a  /vulkan -b", ""), "-a -b");
        assert_eq!(with_renderer("-a /vulkan -b", "/vulkan"), "-a -b /vulkan");
        assert_eq!(with_renderer("/dx11 /vulkan", "/dx11"), "/dx11");
        assert_eq!(with_renderer(" -a ", ""), "-a");
        assert_eq!(renderers(Renderer::Vulkan)[1], ("/vulkan", "Vulkan"));
        assert_eq!(renderers(Renderer::Dx11)[0], ("", "DirectX 12"));
    }
}
