use crate::ui::px;
use backend::manifest::season_entry;
use gpui::{
    App, AppContext, Context, Div, DragMoveEvent, ElementId, Entity, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Pixels, Render, RetainAllImageCache, SharedString,
    Stateful, StatefulInteractiveElement, Styled, Window, div, point, prelude::FluentBuilder,
};

use crate::assets::HEATED_METAL_ART;
use crate::model::download::{Downloader, State};
use crate::model::{Edition, Launcher};
use crate::shell;
use crate::ui::glow::Highlight;
use crate::ui::icon::{CANCEL, PAUSE, PLAY, icon};
use crate::ui::key_art::key_art;
use crate::ui::kit::{focus_ring_inside, truncate};
use crate::ui::motion::{self, EASE_IN_OUT, FAST, SLOW, sub};
use crate::ui::scroll::scroll;
use crate::ui::theme::{
    BORDER, FONT_DISPLAY_BOLD, FONT_MONO, ON_ART, REM, SURFACE_2, TEXT, TEXT_LABEL, TEXT_MICRO,
    TEXT_MUTED, Theme, Typeface, shadow,
};

const GAP: f32 = 8.;
const CARD: f32 = 48.;
const CORNER: f32 = 6.;
const GLOW: f32 = 16.;
const TITLE: f32 = 0.8 * REM;
const ACTION: f32 = 36.;
const LINE_INSET: f32 = 3.;
const ROW: f32 = 24.;
const QUEUE: f32 = 144.;
const LIFTED: f32 = 0.75;
const HELD: f32 = 0.4;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Control {
    Pause,
    Continue,
    Cancel,
    Stop,
}

impl Control {
    fn of(download: &Downloader) -> Option<Self> {
        match download.state() {
            State::Downloading if download.verifying() => Some(Control::Cancel),
            State::Downloading => Some(Control::Pause),
            State::Paused => Some(Control::Continue),
            _ => None,
        }
    }

    fn glyph(self) -> &'static [u8] {
        match self {
            Control::Pause => PAUSE,
            Control::Continue => PLAY,
            Control::Cancel | Control::Stop => CANCEL,
        }
    }

    fn run(self, key: &str, launcher: &mut Launcher, cx: &mut Context<Launcher>) {
        match self {
            Control::Pause => launcher.set_paused(true, cx),
            Control::Continue => launcher.set_paused(false, cx),
            Control::Cancel => launcher.cancel_download(cx),
            Control::Stop => launcher.stop_season(key, cx),
        }
    }
}

fn moved(queue: &[Edition], from: usize, to: usize) -> Option<Vec<Edition>> {
    if from >= queue.len() || to >= queue.len() {
        return None;
    }
    let mut order = queue.to_vec();
    let entry = order.remove(from);
    order.insert(to, entry);
    Some(order)
}

fn shift(
    edition: &Edition,
    target: impl FnOnce(&[Edition], usize) -> Option<usize>,
    cx: &mut App,
) -> Option<usize> {
    let launcher = Launcher::handle(cx);
    let queue: Vec<Edition> = launcher.read(cx).download.queue().collect();
    let from = queue.iter().position(|entry| entry == edition)?;
    let to = target(&queue, from)?;
    let order = moved(&queue, from, to)?;
    launcher.update(cx, |launcher, cx| launcher.reorder_queue(&order, cx));
    Some(queue.len() - 1 - to)
}

struct Activity {
    edition: Edition,
    title: SharedString,
    highlight: Highlight,
    action: Option<Control>,
    progress: Option<f32>,
}

impl Activity {
    fn card(
        self,
        width: f32,
        art: &Entity<RetainAllImageCache>,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        let Edition { key, hm } = self.edition;
        let id = ElementId::Name(format!("activity-{key}-{hm}").into());
        let hover = motion::hover(&id, SLOW, window, cx);
        let launcher = Launcher::handle(cx);
        let glow = self
            .highlight
            .glow(&launcher.read(cx).settings, Theme::get(cx));
        let halo = glow.halo.opacity(hover.amount);
        let name = if hm { HEATED_METAL_ART } else { &key };
        let room = if self.action.is_some() { 44. } else { 8. };
        let held = key.clone();
        let action = self.action.map(|control| {
            div()
                .id(sub(&id, "action"))
                .absolute()
                .top_0()
                .bottom_0()
                .right_0()
                .w(px(ACTION))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(CORNER))
                .cursor_pointer()
                .occlude()
                .tab_index(0)
                .focus_visible(focus_ring_inside)
                .on_click(move |_, _, cx| {
                    launcher.update(cx, |launcher, cx| control.run(&held, launcher, cx))
                })
                .child(icon(control.glyph(), 24.).text_color(ON_ART.solid()))
        });
        let line = self.progress.map(|progress| {
            let share = progress / 100.;
            let shown = motion::transition(sub(&id, "line"), share, SLOW, EASE_IN_OUT, window, cx);
            let fill = div()
                .absolute()
                .left(px(-LINE_INSET))
                .top_0()
                .w(px((width - 2.) * shown))
                .h_full()
                .bg(glow.line);
            div()
                .absolute()
                .left(px(LINE_INSET))
                .right(px(LINE_INSET))
                .bottom_0()
                .h(px(1.))
                .overflow_hidden()
                .child(fill)
        });
        let open = key.clone();
        div()
            .id(id.clone())
            .flex_none()
            .h(px(CARD))
            .rounded(px(CORNER))
            .border_1()
            .border_color(BORDER.mix(glow.edge, hover.amount).solid())
            .shadow(vec![shadow(0., 0., GLOW, 0., halo.into())])
            .child(key_art(name, !hm, CORNER - 1., 1., art))
            .child(
                div()
                    .id(sub(&id, "open"))
                    .absolute()
                    .inset_0()
                    .rounded(px(CORNER))
                    .cursor_pointer()
                    .tab_index(0)
                    .focus_visible(focus_ring_inside)
                    .on_click(move |_, window, cx| shell::open_season(&open, hm, window, cx)),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .flex()
                    .p(px(8.))
                    .pr(px(room))
                    .face(FONT_DISPLAY_BOLD, TITLE)
                    .line_height(px(TITLE))
                    .text_color(ON_ART.solid())
                    .child(truncate(self.title)),
            )
            .children(line)
            .children(action)
            .child(
                div()
                    .id(sub(&id, "hover"))
                    .absolute()
                    .inset(px(-1.))
                    .map(|element| hover.follow(element)),
            )
    }
}

#[derive(Clone)]
struct Lifted {
    edition: Edition,
    number: usize,
    title: SharedString,
    width: Pixels,
}

impl Render for Lifted {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        entry(self.number, self.title.clone(), 1.)
            .w(self.width)
            .opacity(LIFTED)
    }
}

#[derive(Default)]
struct Waiting {
    rows: usize,
    lifted: Option<Edition>,
}

fn entry(number: usize, title: SharedString, lit: f32) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .pt(px(7.))
        .pb(px(5.))
        .rounded(px(CORNER))
        .bg(BORDER.alpha(lit))
        .child(
            div()
                .flex_none()
                .face(FONT_MONO, TEXT_MICRO)
                .text_color(TEXT_MUTED.solid())
                .child(number.to_string()),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .face(FONT_DISPLAY_BOLD, TEXT_LABEL)
                .line_height(px(TEXT_LABEL))
                .text_color(TEXT_MUTED.mix(TEXT, lit).solid())
                .child(truncate(title)),
        )
}

fn queue(
    waiting: Vec<(Edition, SharedString)>,
    width: f32,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let list = scroll("queue", window, cx);
    let state = window.use_keyed_state("queue-rows", cx, |_, _| Waiting::default());
    let dragging = cx.has_active_drag();
    let (grown, lifted) = state.update(cx, |state, _| {
        if !dragging {
            state.lifted = None;
        }
        let before = std::mem::replace(&mut state.rows, waiting.len());
        (waiting.len() as f32 - before as f32, state.lifted.clone())
    });
    if grown != 0. {
        let offset = list.handle().offset();
        let anchored = point(offset.x, offset.y - px(ROW * grown));
        list.handle().set_offset(anchored);
    }
    let mut rows = Vec::with_capacity(waiting.len());
    for (index, (edition, title)) in waiting.into_iter().enumerate().rev() {
        let id = ElementId::Name(format!("queue-{}-{}", edition.key, edition.hm).into());
        let hover = motion::hover(&id, FAST, window, cx);
        let held = lifted.as_ref() == Some(&edition);
        let lit = if held { 1. } else { hover.amount };
        let ghost = Lifted {
            edition: edition.clone(),
            number: index + 1,
            title: title.clone(),
            width: px(width - 2.),
        };
        let (opened, stepped, passed) = (edition.clone(), edition.clone(), edition);
        let (picked, view) = (state.clone(), list.handle().clone());
        let row = entry(index + 1, title, lit)
            .id(id)
            .cursor_pointer()
            .when(held, |row| row.opacity(HELD))
            .tab_index(0)
            .focus_visible(focus_ring_inside)
            .map(|element| hover.follow(element))
            .on_click(move |_, window, cx| shell::open_season(&opened.key, opened.hm, window, cx))
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                let keystroke = &event.keystroke;
                let up = match keystroke.key.as_str() {
                    "up" if keystroke.modifiers.alt => true,
                    "down" if keystroke.modifiers.alt => false,
                    _ => return,
                };
                cx.stop_propagation();
                let target = |_: &[Edition], from: usize| {
                    if up {
                        Some(from + 1)
                    } else {
                        from.checked_sub(1)
                    }
                };
                if let Some(row) = shift(&stepped, target, cx) {
                    view.scroll_to_item(row);
                }
            })
            .on_drag(ghost, move |ghost, _, _, cx| {
                picked.update(cx, |state, cx| {
                    state.lifted = Some(ghost.edition.clone());
                    cx.notify();
                });
                cx.new(|_| ghost.clone())
            })
            .on_drag_move(move |event: &DragMoveEvent<Lifted>, _, cx| {
                let lifted = event.drag(cx).edition.clone();
                if lifted != passed && event.bounds.contains(&event.event.position) {
                    let target =
                        |queue: &[Edition], _| queue.iter().position(|entry| *entry == passed);
                    shift(&lifted, target, cx);
                }
            });
        rows.push(row);
    }
    list.area(
        div()
            .flex_none()
            .max_h(px(QUEUE))
            .rounded(px(CORNER))
            .border_1()
            .border_color(BORDER.solid())
            .bg(SURFACE_2.solid())
            .children(rows),
    )
}

pub fn activity(
    art: &Entity<RetainAllImageCache>,
    width: f32,
    window: &mut Window,
    cx: &mut App,
) -> Option<Div> {
    let launcher = Launcher::handle(cx);
    let (waiting, loading, live) = {
        let model = launcher.read(cx);
        let titled = |edition: &Edition| {
            let mut downloads = model.downloads.iter();
            let download = downloads.find(|download| download.key == edition.key)?;
            let title = season_entry(download, false).title(edition.hm);
            Some((edition.clone(), SharedString::from(title)))
        };
        let download = &model.download;
        let waiting: Vec<_> = download
            .queue()
            .filter_map(|edition| titled(&edition))
            .collect();
        let paused = download.state() == State::Paused;
        let held = download.held();
        let loading = download.active().filter(|_| held).and_then(titled);
        let loading = loading.map(|(edition, title)| Activity {
            edition,
            title,
            highlight: if paused {
                Highlight::Muted
            } else {
                Highlight::Transfer
            },
            action: Control::of(download),
            progress: download.percent().map(|percent| percent as f32),
        });
        let running = model.launch.running().iter();
        let starting = model.launch.launching();
        let starting = starting.filter(|edition| !model.launch.running().contains(edition));
        let live: Vec<_> = running.chain(starting).filter_map(titled).collect();
        (waiting, loading, live)
    };
    if waiting.is_empty() && loading.is_none() && live.is_empty() {
        return None;
    }
    let width = width - 2. * GAP;
    let mut dock = div()
        .flex()
        .flex_none()
        .flex_col()
        .gap(px(GAP))
        .px(px(GAP))
        .pb(px(GAP));
    dock = if waiting.is_empty() {
        dock.pt(px(GAP))
    } else {
        dock.child(queue(waiting, width, window, cx))
    };
    let entries = loading
        .into_iter()
        .chain(live.into_iter().map(|(edition, title)| Activity {
            edition,
            title,
            highlight: Highlight::Accent,
            action: Some(Control::Stop),
            progress: None,
        }));
    for entry in entries {
        dock = dock.child(entry.card(width, art, window, cx));
    }
    Some(dock)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edition(key: &str, hm: bool) -> Edition {
        Edition {
            key: key.into(),
            hm,
        }
    }

    #[test]
    fn the_card_pauses_a_download_cancels_a_verify_and_continues_a_pause() {
        let active = Some(edition("Y5S3_ShadowLegacy", false));
        let control = |state, verifying| {
            Control::of(&Downloader::with(
                state,
                active.clone(),
                verifying,
                Vec::new(),
            ))
        };
        assert_eq!(control(State::Downloading, false), Some(Control::Pause));
        assert_eq!(control(State::Downloading, true), Some(Control::Cancel));
        assert_eq!(control(State::Paused, false), Some(Control::Continue));
        assert_eq!(control(State::Paused, true), Some(Control::Continue));
        for state in [
            State::Preparing,
            State::Applying,
            State::Idle,
            State::Failed,
        ] {
            assert_eq!(control(state, false), None);
            assert_eq!(control(state, true), None);
        }
    }

    #[test]
    fn a_moved_entry_takes_the_place_it_is_moved_to() {
        let queue = ["a", "b", "c"].map(|key| edition(key, false));
        let order = |from, to| {
            let moved = moved(&queue, from, to)?;
            Some(moved.into_iter().map(|entry| entry.key).collect::<String>())
        };
        assert_eq!(order(0, 1).as_deref(), Some("bac"));
        assert_eq!(order(0, 2).as_deref(), Some("bca"));
        assert_eq!(order(2, 0).as_deref(), Some("cab"));
        assert_eq!(order(1, 1).as_deref(), Some("abc"));
        assert_eq!(order(2, 3), None);
        assert_eq!(order(3, 0), None);
    }
}
