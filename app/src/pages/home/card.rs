use crate::ui::px;
use backend::manifest::SeasonEntry;
use gpui::{
    AnyElement, App, AppContext, Bounds, Context, CursorStyle, Div, DragMoveEvent, ElementId,
    Entity, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, ParentElement, Pixels, Render, RetainAllImageCache, SharedString, Size,
    Stateful, StatefulInteractiveElement, Styled, Svg, Window, div, prelude::FluentBuilder,
};

use super::HomePage;
use super::grid::{Grid, Resize, Span};
use crate::assets::HEATED_METAL_ART;
use crate::model::Launcher;
use crate::model::download::{Downloader, State};
use crate::model::launch::Launch;
use crate::shell;
use crate::ui::button::{primary, secondary};
use crate::ui::glow::Highlight;
use crate::ui::icon::{GRIP, icon};
use crate::ui::key_art::key_art;
use crate::ui::kit::{disable, focus_ring_inside, panel, truncate};
use crate::ui::motion::{self, EASE, SLOW, sub};
use crate::ui::theme::{BORDER, FONT_DISPLAY_BOLD, ON_ART, REM, TEXT, Theme, Typeface, shadow};

const CORNER: f32 = 7.;
const GLOW: f32 = 28.;
const LINE_INSET: f32 = 4.;
const TITLE_LINE: f32 = 1.25 * 1.05 * REM;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Cancel,
    Pause,
    Continue,
    Dequeue,
    Verify { blocked: bool },
    Stop,
    Play { blocked: bool },
}

impl Action {
    pub fn of(
        entry: &SeasonEntry,
        download: &Downloader,
        launch: &Launch,
        updating: bool,
    ) -> Option<Self> {
        let (key, hm) = (entry.key.as_str(), entry.hm);
        let active = download.active().is_some_and(|edition| edition.is(key, hm));
        let downloading = download.state() == State::Downloading;
        let mut started = launch.running().iter().chain(launch.launching());
        if entry.partial {
            if download.running() && active {
                let stop = if download.verifying() {
                    Action::Cancel
                } else {
                    Action::Pause
                };
                return downloading.then_some(stop);
            }
            if active && download.state() == State::Paused {
                return Some(Action::Continue);
            }
            if download.waits(key, hm) {
                return Some(Action::Dequeue);
            }
            return Some(Action::Verify {
                blocked: started.any(|edition| edition.key == key),
            });
        }
        if download.running() && active {
            return downloading.then_some(Action::Cancel);
        }
        if started.any(|edition| edition.is(key, hm)) {
            return Some(Action::Stop);
        }
        Some(Action::Play {
            blocked: updating || download.busy_with(key),
        })
    }

    fn label(self) -> &'static str {
        match self {
            Action::Cancel => "Cancel",
            Action::Pause => "Pause",
            Action::Continue => "Continue",
            Action::Dequeue => "Remove from queue",
            Action::Verify { .. } => "Verify",
            Action::Stop => "Stop",
            Action::Play { .. } => "Play",
        }
    }

    fn blocked(self) -> bool {
        matches!(
            self,
            Action::Verify { blocked: true } | Action::Play { blocked: true }
        )
    }

    fn run(self, key: &str, hm: bool, launcher: &mut Launcher, cx: &mut Context<Launcher>) {
        match self {
            Action::Cancel => launcher.cancel_download(cx),
            Action::Pause => launcher.set_paused(true, cx),
            Action::Continue => launcher.set_paused(false, cx),
            Action::Dequeue => launcher.dequeue(key, hm, cx),
            Action::Verify { .. } => launcher.enqueue(key, hm, "", cx),
            Action::Stop => launcher.stop_season(key, cx),
            Action::Play { .. } => launcher.launch_season(key, hm, cx),
        }
    }

    fn primary(self) -> bool {
        matches!(self, Action::Stop | Action::Play { .. })
    }

    fn button(self, id: ElementId, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        if self.primary() {
            primary(id, self.label(), window, cx)
        } else {
            secondary(id, self.label(), window, cx)
        }
    }
}

impl Highlight {
    pub fn of(entry: &SeasonEntry, download: &Downloader) -> Self {
        let active = download
            .active()
            .is_some_and(|edition| edition.is(&entry.key, entry.hm));
        if download.verifying() && active {
            Highlight::Transfer
        } else if !entry.partial {
            Highlight::Accent
        } else if download.running() && active {
            Highlight::Transfer
        } else {
            Highlight::Muted
        }
    }
}

#[derive(Clone)]
pub struct Card {
    pub id: String,
    pub key: String,
    pub hm: bool,
    pub title: SharedString,
    pub span: Span,
    pub action: Option<Action>,
    pub highlight: Highlight,
}

impl Card {
    fn art(&self, cache: &Entity<RetainAllImageCache>) -> Div {
        let name = if self.hm { HEATED_METAL_ART } else { &self.key };
        key_art(name, !self.hm, CORNER, 1., cache)
    }

    fn caption(&self, button: Option<Stateful<Div>>) -> Div {
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .flex()
            .items_end()
            .justify_between()
            .gap(px(8.))
            .py(px(8.))
            .pl(px(12.))
            .pr(px(8.))
            .face(FONT_DISPLAY_BOLD, 1.05 * REM)
            .line_height(px(TITLE_LINE))
            .text_color(ON_ART.solid())
            .child(truncate(self.title.clone()))
            .children(button)
    }
}

fn grip() -> Svg {
    icon(GRIP, 16.).text_color(ON_ART.alpha(0.9))
}

#[derive(Clone)]
struct Ghost {
    card: Card,
    size: Size<Pixels>,
    art: Entity<RetainAllImageCache>,
}

impl Render for Ghost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let face = panel()
            .size_full()
            .child(self.card.art(&self.art))
            .child(self.card.caption(None));
        div().w(self.size.width).h(self.size.height).child(face)
    }
}

impl HomePage {
    pub(super) fn tile(
        &self,
        card: &Card,
        frame: Bounds<Pixels>,
        stop: &FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = ElementId::Name(card.id.clone().into());
        let ringed = stop.is_focused(window) && window.last_input_was_keyboard();
        let hover = motion::hover(&id, SLOW, window, cx);
        let over = if hover.over { 1. } else { 0. };
        let reveal = motion::transition(sub(&id, "line"), over, SLOW, EASE, window, cx);
        let glow = card
            .highlight
            .glow(&self.launcher.read(cx).settings, Theme::get(cx));
        let halo = glow.halo.opacity(hover.amount);
        let run = frame.size.width - px(2. + 2. * LINE_INSET);
        let button = card.action.map(|action| {
            let (launcher, key, hm) = (self.launcher.clone(), card.key.clone(), card.hm);
            action
                .button(sub(&id, "action"), window, cx)
                .cursor_default()
                .block_mouse_except_scroll()
                .on_click(move |_, _, cx| {
                    launcher.update(cx, |launcher, cx| action.run(&key, hm, launcher, cx));
                })
                .when(action.blocked(), disable)
        });
        panel()
            .absolute()
            .left(frame.origin.x)
            .top(frame.origin.y)
            .w(frame.size.width)
            .h(frame.size.height)
            .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
            .border_color(if ringed {
                TEXT.solid()
            } else {
                BORDER.mix(glow.edge, hover.amount).solid()
            })
            .shadow(vec![shadow(0., 0., GLOW, 0., halo.into())])
            .child(card.art(&self.art))
            .child(
                div()
                    .id(sub(&id, "open"))
                    .absolute()
                    .inset(px(-1.))
                    .cursor_pointer()
                    .track_focus(stop)
                    .on_click({
                        let (key, hm) = (card.key.clone(), card.hm);
                        move |_, window, cx| shell::open_season(&key, hm, window, cx)
                    }),
            )
            .child(card.caption(button))
            .child(
                div()
                    .absolute()
                    .left(px(LINE_INSET))
                    .bottom_0()
                    .w(run * reveal)
                    .h(px(1.))
                    .overflow_hidden()
                    .child(div().w(run).h_full().bg(glow.line)),
            )
            .child(
                div()
                    .id(sub(&id, "hover"))
                    .absolute()
                    .inset(px(-1.))
                    .map(|element| hover.follow(element))
                    .on_mouse_down(MouseButton::Right, {
                        let (key, hm) = (card.key.clone(), card.hm);
                        cx.listener(move |page, event: &MouseDownEvent, window, cx| {
                            page.offer(&key, hm, event.position, window, cx)
                        })
                    }),
            )
            .into_any_element()
    }

    pub(super) fn loose(
        &self,
        card: &Card,
        frame: Bounds<Pixels>,
        grid: Grid,
        focus: &FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = ElementId::Name(card.id.clone().into());
        let place = div()
            .id(id.clone())
            .absolute()
            .left(frame.origin.x)
            .top(frame.origin.y)
            .w(frame.size.width)
            .h(frame.size.height)
            .track_focus(focus);
        if self.drag.as_ref().is_some_and(|drag| drag.id == card.id) {
            return place.into_any_element();
        }
        let ringed = focus.is_focused(window) && window.last_input_was_keyboard();
        let ghost = Ghost {
            card: card.clone(),
            size: frame.size,
            art: self.art.clone(),
        };
        let page = cx.weak_entity();
        let (stepping, passing, stretching, sizing) = (
            card.id.clone(),
            card.id.clone(),
            card.id.clone(),
            card.id.clone(),
        );
        let (pressing, holding, kept) = (focus.clone(), focus.clone(), focus.clone());
        let extent = self
            .placed
            .get(&card.id)
            .map_or(frame.size, |target| target.size);
        let stored = card.span;
        let hold = div()
            .id(sub(&id, "hold"))
            .absolute()
            .inset(px(-1.))
            .block_mouse_except_scroll()
            .cursor(CursorStyle::ClosedHand)
            .hover(|style| style.cursor(CursorStyle::OpenHand))
            .active(|style| style.cursor(CursorStyle::ClosedHand))
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                window.focus(&pressing, cx)
            })
            .on_drag(ghost, move |ghost, offset, _, cx| {
                page.update(cx, |page, cx| page.grab(&ghost.card.id, offset, cx))
                    .ok();
                cx.new(|_| ghost.clone())
            })
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<Ghost>, _, cx| {
                    if event.bounds.contains(&event.event.position) {
                        this.pass(&passing, cx);
                    }
                }),
            );
        let handle = div()
            .id(sub(&id, "grip"))
            .absolute()
            .right(px(-2.))
            .bottom(px(-2.))
            .rounded(px(6.))
            .cursor(CursorStyle::ResizeUpLeftDownRight)
            .tab_index(0)
            .focus_visible(focus_ring_inside)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    let resize =
                        Resize::new(stretching.clone(), stored, event.position, extent, grid);
                    this.resize = Some(resize);
                    window.focus(&holding, cx);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                let by = match event.keystroke.key.as_str() {
                    _ if event.keystroke.modifiers.alt => return,
                    "right" => (1, 0),
                    "left" => (-1, 0),
                    "down" => (0, 1),
                    "up" => (0, -1),
                    _ => return,
                };
                cx.stop_propagation();
                this.nudge(&sizing, stored, by, grid.columns, cx);
            }))
            .child(grip());
        place
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                let delta = match event.keystroke.key.as_str() {
                    "left" if event.keystroke.modifiers.alt => -1,
                    "right" if event.keystroke.modifiers.alt => 1,
                    _ => return,
                };
                cx.stop_propagation();
                this.shift(&stepping, delta, cx);
                window.focus(&kept, cx);
            }))
            .child(
                panel()
                    .size_full()
                    .when(ringed, |panel| panel.border_color(TEXT.solid()))
                    .child(card.art(&self.art))
                    .child(card.caption(None))
                    .child(hold),
            )
            .child(handle)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use backend::manifest::{downloads, season_entry};

    use super::*;
    use crate::model::Edition;

    const KEY: &str = "Y5S3_ShadowLegacy";

    fn entry(hm: bool, partial: bool) -> SeasonEntry {
        let seasons = downloads();
        let season = seasons.iter().find(|season| season.key == KEY).unwrap();
        SeasonEntry {
            partial,
            ..season_entry(season, hm)
        }
    }

    fn edition(key: &str, hm: bool) -> Edition {
        Edition {
            key: key.into(),
            hm,
        }
    }

    fn loading(state: State, hm: bool, verifying: bool) -> Downloader {
        Downloader::with(state, Some(edition(KEY, hm)), verifying, Vec::new())
    }

    fn started(running: &[Edition], launching: Option<Edition>) -> Launch {
        Launch::with(running.to_vec(), launching)
    }

    fn act(entry: &SeasonEntry, download: &Downloader, launch: &Launch) -> Option<Action> {
        Action::of(entry, download, launch, false)
    }

    #[test]
    fn an_installed_edition_plays_unless_an_update_or_its_season_is_busy() {
        let installed = entry(false, false);
        let (idle, still) = (Downloader::default(), Launch::with(Vec::new(), None));
        let free = Some(Action::Play { blocked: false });
        let held = Some(Action::Play { blocked: true });
        assert_eq!(act(&installed, &idle, &still), free);
        assert_eq!(Action::of(&installed, &idle, &still, true), held);
        for state in [State::Preparing, State::Downloading, State::Applying] {
            assert_eq!(act(&installed, &loading(state, true, false), &still), held);
        }
        for state in [State::Paused, State::Failed] {
            assert_eq!(act(&installed, &loading(state, true, false), &still), free);
        }
        let elsewhere = Downloader::with(
            State::Downloading,
            Some(edition("Y1S0_Vanilla", false)),
            false,
            vec![edition(KEY, false)],
        );
        assert_eq!(act(&installed, &elsewhere, &still), free);
        assert_eq!(
            act(&installed, &idle, &started(&[edition(KEY, true)], None)),
            free
        );
    }

    #[test]
    fn a_running_or_starting_edition_stops() {
        let installed = entry(true, false);
        let idle = Downloader::default();
        let running = started(&[edition("Y1S0_Vanilla", false), edition(KEY, true)], None);
        let launching = started(&[], Some(edition(KEY, true)));
        assert_eq!(act(&installed, &idle, &running), Some(Action::Stop));
        assert_eq!(act(&installed, &idle, &launching), Some(Action::Stop));
    }

    #[test]
    fn an_installed_edition_being_verified_cancels_only_while_downloading() {
        let installed = entry(false, false);
        let running = started(&[edition(KEY, false)], None);
        let verify = |state| loading(state, false, true);
        assert_eq!(
            act(&installed, &verify(State::Downloading), &running),
            Some(Action::Cancel)
        );
        assert_eq!(act(&installed, &verify(State::Preparing), &running), None);
        assert_eq!(act(&installed, &verify(State::Applying), &running), None);
    }

    #[test]
    fn a_partial_download_pauses_continues_leaves_the_queue_or_verifies() {
        let partial = entry(false, true);
        let still = Launch::with(Vec::new(), None);
        let action = |download: &Downloader| act(&partial, download, &still);
        assert_eq!(
            action(&loading(State::Downloading, false, false)),
            Some(Action::Pause)
        );
        assert_eq!(
            action(&loading(State::Downloading, false, true)),
            Some(Action::Cancel)
        );
        assert_eq!(action(&loading(State::Preparing, false, false)), None);
        assert_eq!(action(&loading(State::Applying, false, true)), None);
        assert_eq!(
            action(&loading(State::Paused, false, false)),
            Some(Action::Continue)
        );
        let queued = Downloader::with(
            State::Downloading,
            Some(edition(KEY, true)),
            false,
            vec![edition("Y1S0_Vanilla", false), edition(KEY, false)],
        );
        assert_eq!(action(&queued), Some(Action::Dequeue));
        let verify = Some(Action::Verify { blocked: false });
        assert_eq!(action(&Downloader::default()), verify);
        assert_eq!(action(&loading(State::Failed, false, false)), verify);
        assert_eq!(action(&loading(State::Paused, true, false)), verify);
        assert_eq!(action(&loading(State::Downloading, true, false)), verify);
    }

    #[test]
    fn verify_waits_while_any_edition_of_the_season_runs_or_starts() {
        let partial = entry(false, true);
        let idle = Downloader::default();
        let held = Some(Action::Verify { blocked: true });
        let running = started(&[edition(KEY, true)], None);
        let launching = started(&[], Some(edition(KEY, true)));
        let other = started(&[edition("Y1S0_Vanilla", false)], None);
        assert_eq!(act(&partial, &idle, &running), held);
        assert_eq!(act(&partial, &idle, &launching), held);
        assert_eq!(
            act(&partial, &idle, &other),
            Some(Action::Verify { blocked: false })
        );
    }

    #[test]
    fn only_play_and_stop_are_primary_and_only_blocked_actions_are_disabled() {
        let looks = [
            (Action::Cancel, "Cancel", false, false),
            (Action::Pause, "Pause", false, false),
            (Action::Continue, "Continue", false, false),
            (Action::Dequeue, "Remove from queue", false, false),
            (Action::Verify { blocked: false }, "Verify", false, false),
            (Action::Verify { blocked: true }, "Verify", false, true),
            (Action::Stop, "Stop", true, false),
            (Action::Play { blocked: false }, "Play", true, false),
            (Action::Play { blocked: true }, "Play", true, true),
        ];
        for (action, label, primary, blocked) in looks {
            assert_eq!(
                (action.label(), action.primary(), action.blocked()),
                (label, primary, blocked)
            );
        }
    }

    #[test]
    fn the_tone_follows_the_download() {
        let (installed, partial) = (entry(false, false), entry(false, true));
        let idle = Downloader::default();
        assert_eq!(Highlight::of(&installed, &idle), Highlight::Accent);
        assert_eq!(Highlight::of(&partial, &idle), Highlight::Muted);
        for state in [State::Preparing, State::Downloading, State::Applying] {
            assert_eq!(
                Highlight::of(&partial, &loading(state, false, false)),
                Highlight::Transfer
            );
            assert_eq!(
                Highlight::of(&installed, &loading(state, false, false)),
                Highlight::Accent
            );
            assert_eq!(
                Highlight::of(&installed, &loading(state, false, true)),
                Highlight::Transfer
            );
            assert_eq!(
                Highlight::of(&installed, &loading(state, true, true)),
                Highlight::Accent
            );
            assert_eq!(
                Highlight::of(&partial, &loading(state, true, false)),
                Highlight::Muted
            );
        }
        for state in [State::Paused, State::Failed] {
            assert_eq!(
                Highlight::of(&partial, &loading(state, false, false)),
                Highlight::Muted
            );
        }
        assert_eq!(
            Highlight::of(&partial, &loading(State::Paused, false, true)),
            Highlight::Transfer
        );
    }
}
