mod card;
mod grid;

use std::collections::HashMap;
use std::mem;
use std::time::{Duration, Instant};

use crate::ui::{frame, px};
use backend::manifest::SeasonEntry;
use gpui::{
    AnyElement, App, Bounds, Context, CursorStyle, DismissEvent, DispatchPhase, Div, ElementId,
    Entity, FocusHandle, InteractiveElement, IntoElement, MouseButton, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, Point, Render, RetainAllImageCache, Stateful,
    StatefulInteractiveElement, Styled, Subscription, Transformation, Window, canvas, div, point,
    prelude::FluentBuilder, size,
};

use crate::model::Launcher;
use crate::model::settings::DEFAULT_SPAN;
use crate::pages::season::SeasonMenu;
use crate::shell::{self, Page};
use crate::ui::Dismiss;
use crate::ui::button::{glyph_button, icon_button, primary, secondary};
use crate::ui::glow::Highlight;
use crate::ui::icon::{ERASER, PENCIL, PENCIL_FILLED, REVERSE, icon};
use crate::ui::kit::{disable, focus_ring, icon_box};
use crate::ui::motion::{self, Bezier, EASE, EASE_IN_OUT, EASE_OUT_CUBIC, FAST};
use crate::ui::theme::{FONT_DISPLAY_BOLD, REM, SURFACE_2, TEXT, TEXT_MUTED, Theme, Typeface};
use card::{Action, Card};
use grid::{Grid, Resize, Span, dropped, merged, nudged, rank, stepped};

const ERASE: Duration = Duration::from_millis(400);
const SHAKE: f32 = 3.;
const FLIP: Pace = Pace {
    duration: Duration::from_millis(260),
    curve: Bezier(0.2, 0., 0., 1.),
    sized: false,
};
const MORPH: Pace = Pace {
    duration: Duration::from_millis(320),
    curve: EASE_OUT_CUBIC,
    sized: true,
};

#[derive(Clone, Copy)]
struct Pace {
    duration: Duration,
    curve: Bezier,
    sized: bool,
}

struct Slide {
    from: Bounds<Pixels>,
    start: Instant,
    pace: Pace,
}

impl Slide {
    fn at(&self, to: Bounds<Pixels>, now: Instant) -> Option<Bounds<Pixels>> {
        let elapsed = now.duration_since(self.start).as_secs_f32();
        let progress = elapsed / self.pace.duration.as_secs_f32();
        (progress < 1.).then(|| {
            let eased = self.pace.curve.at(progress);
            let blend = |from: Pixels, to: Pixels| from + (to - from) * eased;
            Bounds::new(
                point(
                    blend(self.from.origin.x, to.origin.x),
                    blend(self.from.origin.y, to.origin.y),
                ),
                size(
                    blend(self.from.size.width, to.size.width),
                    blend(self.from.size.height, to.size.height),
                ),
            )
        })
    }
}

struct Drag {
    id: String,
    order: Vec<String>,
    moved: bool,
}

pub struct HomePage {
    launcher: Entity<Launcher>,
    art: Entity<RetainAllImageCache>,
    editing: bool,
    erased: usize,
    drag: Option<Drag>,
    resize: Option<Resize>,
    morph: bool,
    grid: Option<Grid>,
    placed: HashMap<String, Bounds<Pixels>>,
    slides: HashMap<String, Slide>,
    stops: HashMap<String, FocusHandle>,
    menu: Option<(Entity<SeasonMenu>, Subscription)>,
    _subscriptions: [Subscription; 2],
}

fn shake(serial: usize, window: &mut Window, cx: &mut App) -> f32 {
    if serial == 0 {
        return 0.;
    }
    let elapsed = motion::clock(("home-erase", serial), window, cx);
    let progress = elapsed.as_secs_f32() / ERASE.as_secs_f32();
    if progress >= 1. {
        return 0.;
    }
    window.request_animation_frame();
    let (from, to, part) = if progress < 0.25 {
        (0., -SHAKE, progress / 0.25)
    } else if progress < 0.75 {
        (-SHAKE, SHAKE, (progress - 0.25) / 0.5)
    } else {
        (SHAKE, 0., (progress - 0.75) / 0.25)
    };
    from + (to - from) * EASE.at(part)
}

impl HomePage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let launcher = Launcher::handle(cx);
        launcher.update(cx, |launcher, cx| launcher.refresh_home(cx));
        let subscriptions = [
            cx.observe(&launcher, |_, _, cx| cx.notify()),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.launcher
                        .update(cx, |launcher, cx| launcher.refresh_home(cx));
                }
            }),
        ];
        Self {
            launcher,
            art: RetainAllImageCache::new(cx),
            editing: false,
            erased: 0,
            drag: None,
            resize: None,
            morph: false,
            grid: None,
            placed: HashMap::new(),
            slides: HashMap::new(),
            stops: HashMap::new(),
            menu: None,
            _subscriptions: subscriptions,
        }
    }

    fn cards(&self, cx: &App) -> Option<Vec<Card>> {
        let launcher = self.launcher.read(cx);
        let spans = launcher.settings.home_spans();
        let span = |id: &String| match &self.resize {
            Some(resize) if resize.id == *id => resize.span,
            _ => spans.get(id).copied().unwrap_or(DEFAULT_SPAN),
        };
        let card = |entry: &SeasonEntry| Card {
            id: entry.id.clone(),
            key: entry.key.clone(),
            hm: entry.hm,
            title: entry.title(entry.hm).into(),
            span: span(&entry.id),
            action: Action::of(
                entry,
                &launcher.download,
                &launcher.launch,
                launcher.update.busy(),
            ),
            highlight: Highlight::of(entry, &launcher.download),
        };
        let mut cards: Vec<Card> = launcher.home.editions()?.iter().map(card).collect();
        let saved = launcher.settings.home_order();
        cards.sort_by_key(|card| rank(&saved, &card.id));
        if let Some(drag) = &self.drag {
            cards.sort_by_key(|card| rank(&drag.order, &card.id));
        }
        Some(cards)
    }

    fn order(&self, cx: &App) -> Vec<String> {
        let cards = self.cards(cx).unwrap_or_default();
        cards.into_iter().map(|card| card.id).collect()
    }

    fn store(&mut self, order: &[String], cx: &mut Context<Self>) {
        self.launcher.update(cx, |launcher, cx| {
            let saved = launcher.settings.home_order();
            launcher.settings.set_home_order(merged(&saved, order), cx);
        });
    }

    pub fn stop(&self, id: &str) -> Option<FocusHandle> {
        self.stops.get(id).cloned()
    }

    fn offer(
        &mut self,
        key: &str,
        hm: bool,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = SeasonMenu::open(key, hm, at, window, cx).map(|menu| {
            let closed = cx.subscribe(&menu, |page, _, _: &DismissEvent, cx| {
                page.menu = None;
                cx.notify();
            });
            (menu, closed)
        });
        cx.notify();
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        self.editing = !self.editing;
        self.erased = 0;
        self.drag = None;
        self.resize = None;
        cx.notify();
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        self.launcher
            .update(cx, |launcher, cx| launcher.settings.reset_home_layout(cx));
        self.erased += 1;
        self.morph = true;
        cx.notify();
    }

    fn reverse(&mut self, cx: &mut Context<Self>) {
        let mut order = self.order(cx);
        order.reverse();
        self.store(&order, cx);
        self.morph = true;
        cx.notify();
    }

    fn shift(&mut self, id: &str, delta: isize, cx: &mut Context<Self>) {
        if let Some(order) = stepped(&self.order(cx), id, delta) {
            self.store(&order, cx);
        }
    }

    fn nudge(&mut self, id: &str, stored: Span, by: (i8, i8), columns: u8, cx: &mut Context<Self>) {
        let span = nudged(stored, by, columns);
        if span != stored {
            self.launcher.update(cx, |launcher, cx| {
                launcher.settings.set_home_span(id, span, cx)
            });
        }
    }

    fn grab(&mut self, id: &str, cx: &mut Context<Self>) {
        self.drag = Some(Drag {
            id: id.to_owned(),
            order: self.order(cx),
            moved: false,
        });
        cx.notify();
    }

    fn pass(&mut self, onto: &str, cx: &mut Context<Self>) {
        let Some(drag) = &mut self.drag else {
            return;
        };
        if !self.slides.is_empty() {
            return;
        }
        if let Some(order) = dropped(&drag.order, &drag.id, onto) {
            drag.order = order;
            drag.moved = true;
            cx.notify();
        }
    }

    fn pull(&mut self, pointer: Point<Pixels>, cx: &mut Context<Self>) {
        if self
            .resize
            .as_mut()
            .is_some_and(|resize| resize.pull(pointer))
        {
            cx.notify();
        }
    }

    fn release(&mut self, inside: bool, cx: &mut Context<Self>) {
        if let Some(drag) = self.drag.take() {
            if inside && drag.moved {
                self.store(&drag.order, cx);
            }
            cx.notify();
        }
        if let Some(resize) = self.resize.take() {
            self.launcher.update(cx, |launcher, cx| {
                launcher.settings.set_home_span(&resize.id, resize.span, cx)
            });
            cx.notify();
        }
    }

    fn sensor(resizing: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let page = cx.weak_entity();
        let sensor = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                if resizing {
                    window.set_window_cursor_style(CursorStyle::ResizeUpLeftDownRight);
                }
                let zone = bounds.intersect(&window.content_mask().bounds);
                let pulling = page.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        pulling
                            .update(cx, |page, cx| page.pull(event.position, cx))
                            .ok();
                    }
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                        let inside = zone.contains(&event.position);
                        page.update(cx, |page, cx| page.release(inside, cx)).ok();
                    }
                });
            },
        );
        sensor.absolute().size_full()
    }

    fn settle(
        &mut self,
        id: &str,
        before: Option<Bounds<Pixels>>,
        target: Bounds<Pixels>,
        pace: Option<Pace>,
        now: Instant,
    ) -> Bounds<Pixels> {
        if let Some(before) = before.filter(|before| *before != target) {
            let running = self.slides.get(id).and_then(|slide| slide.at(before, now));
            let from = running.unwrap_or(before);
            let far = |from: Pixels, to: Pixels| (from - to).abs() > px(2.);
            let moved = far(from.origin.x, target.origin.x) || far(from.origin.y, target.origin.y);
            match pace {
                Some(pace) if moved || (pace.sized && from.size != target.size) => {
                    let from = if pace.sized {
                        from
                    } else {
                        Bounds::new(from.origin, target.size)
                    };
                    let slide = Slide {
                        from,
                        start: now,
                        pace,
                    };
                    self.slides.insert(id.to_owned(), slide);
                }
                _ => {
                    self.slides.remove(id);
                }
            }
        }
        let shown = self.slides.get(id).and_then(|slide| slide.at(target, now));
        if shown.is_none() {
            self.slides.remove(id);
        }
        shown.unwrap_or(target)
    }

    fn pencil(&self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let id = ElementId::from("home-edit");
        let hover = motion::hover(&id, FAST, window, cx);
        let editing = if self.editing { 1. } else { 0. };
        let lit = motion::transition("home-edit-lit", editing, FAST, EASE_IN_OUT, window, cx);
        let idle = TEXT_MUTED.mix(TEXT, hover.amount);
        let color = idle.mix(Theme::get(cx).action, lit);
        let glyph = if self.editing { PENCIL_FILLED } else { PENCIL };
        div()
            .id(id)
            .flex_none()
            .p(px(4.))
            .rounded(px(6.))
            .tab_index(0)
            .focus_visible(focus_ring)
            .map(|element| hover.follow(element))
            .on_click(cx.listener(|this, _, _, cx| this.toggle(cx)))
            .child(icon(glyph, 16.).text_color(color.solid()))
    }

    pub fn tools(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let count = self.launcher.read(cx).home.editions()?.len();
        if count == 0 {
            return None;
        }
        let nudge = point(px(shake(self.erased, window, cx)), px(0.));
        let eraser = icon(ERASER, 16.).with_transformation(Transformation::translate(nudge));
        let tools = icon_box()
            .bg(SURFACE_2.solid())
            .child(
                glyph_button("home-reset", eraser, window, cx)
                    .p(px(4.))
                    .on_click(cx.listener(|this, _, _, cx| this.reset(cx))),
            )
            .when(count > 1, |tools| {
                tools.child(
                    icon_button("home-reverse", REVERSE, window, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.reverse(cx))),
                )
            })
            .child(self.pencil(window, cx));
        Some(tools.into_any_element())
    }

    fn empty(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let launcher = self.launcher.clone();
        let downloading = launcher.read(cx).download.running();
        div()
            .flex()
            .flex_col()
            .items_center()
            .pt(0.12 * frame::area(window).size.height)
            .child(
                div()
                    .face(FONT_DISPLAY_BOLD, 1.9 * REM)
                    .child("Your library is empty"),
            )
            .child(
                div()
                    .mt(px(24.))
                    .flex()
                    .gap(px(8.))
                    .child(
                        primary("empty-download", "Download", window, cx)
                            .cursor_pointer()
                            .on_click(|_, window, cx| shell::navigate(Page::Download, window, cx)),
                    )
                    .child(
                        secondary("empty-library", "Add library", window, cx)
                            .on_click(move |_, _, cx| {
                                launcher.update(cx, |launcher, cx| launcher.add_library(cx));
                            })
                            .when(downloading, disable),
                    ),
            )
            .into_any_element()
    }

    fn board(
        &mut self,
        cards: Vec<Card>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let viewport = frame::area(window).size.width;
        let grid = Grid::new(shell::page_size(window), viewport, window.scale_factor());
        let steady = self.grid.replace(grid) == Some(grid);
        let morph = mem::take(&mut self.morph);
        let moving = !cx.reduce_motion();
        let pace = match (moving && steady, self.editing, morph) {
            (false, ..) => None,
            (_, true, _) => Some(FLIP),
            (_, _, true) => Some(MORPH),
            _ => None,
        };
        let now = cx.background_executor().now();
        let spans: Vec<Span> = cards.iter().map(|card| grid.span(card.span)).collect();
        let (frames, height) = grid.frames(&spans);
        let before = mem::take(&mut self.placed);
        let mut tiles = Vec::with_capacity(cards.len());
        for (card, target) in cards.iter().zip(frames) {
            let frame = self.settle(&card.id, before.get(&card.id).copied(), target, pace, now);
            self.placed.insert(card.id.clone(), target);
            let stop = self.stops.entry(card.id.clone());
            let stop = stop.or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true));
            let stop = stop.clone();
            tiles.push(if self.editing {
                self.loose(card, frame, grid, &stop, window, cx)
            } else {
                self.tile(card, frame, &stop, window, cx)
            });
        }
        self.slides.retain(|id, _| self.placed.contains_key(id));
        self.stops.retain(|id, _| self.placed.contains_key(id));
        if !self.slides.is_empty() {
            window.request_animation_frame();
        }
        div()
            .h(height)
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                if this.drag.take().is_some() {
                    cx.stop_active_drag(window);
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }))
            .children(
                self.editing
                    .then(|| Self::sensor(self.resize.is_some(), cx)),
            )
            .children(tiles)
            .children(self.menu.as_ref().map(|(menu, _)| menu.clone()))
            .into_any_element()
    }
}

impl Render for HomePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.cards(cx) {
            None => div().into_any_element(),
            Some(cards) if cards.is_empty() => self.empty(window, cx),
            Some(cards) => self.board(cards, window, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use backend::env::{Paths, TL_LAUNCHER};
    use backend::manifest;
    use backend::settings::Settings as Store;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use tempfile::TempDir;

    use super::*;
    use crate::model::Edition;
    use crate::model::download::{Downloader, State};
    use crate::model::shears::ShearsEvent;

    const VANILLA: &str = "Y1S0_Vanilla";
    const SHADOW: &str = "Y5S3_ShadowLegacy";
    const METAL: &str = "Y9S2_HeatedMetal";

    struct Bench {
        _dir: TempDir,
        library: PathBuf,
        launcher: Entity<Launcher>,
        page: Entity<HomePage>,
    }

    fn install(library: &Path, edition: &str) {
        let folder = library.join(edition);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(TL_LAUNCHER), "").unwrap();
    }

    fn bench<'a>(
        cx: &'a mut TestAppContext,
        editions: &[&str],
        settings: &str,
    ) -> (Bench, &'a mut VisualTestContext) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        let library = dir.path().join("lib");
        fs::create_dir_all(&paths.data_root).unwrap();
        fs::create_dir_all(&library).unwrap();
        let folders = format!("libraries = ['{}']", library.display());
        fs::write(
            paths.settings_file(),
            format!("[settings]\ndiscord_rpc = false\n{folders}\n{settings}\n"),
        )
        .unwrap();
        let launcher = cx.update(|cx| {
            crate::init(cx);
            Launcher::new(
                paths.clone(),
                Store::load(&paths).unwrap(),
                manifest::downloads(),
            )
            .init(cx);
            Launcher::handle(cx)
        });
        cx.run_until_parked();
        for edition in editions {
            install(&library, edition);
        }
        let (page, cx) = cx.add_window_view(HomePage::new);
        cx.simulate_resize(size(px(1280.), px(720.)));
        cx.update(|window, _| window.activate_window());
        draw(cx);
        let bench = Bench {
            _dir: dir,
            library,
            launcher,
            page,
        };
        (bench, cx)
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn shown(bench: &Bench, cx: &mut VisualTestContext) -> Vec<String> {
        bench.page.read_with(cx, |page, cx| page.order(cx))
    }

    fn stored(bench: &Bench, cx: &mut VisualTestContext) -> Vec<String> {
        bench
            .launcher
            .read_with(cx, |launcher, _| launcher.settings.home_order())
    }

    fn sizes(bench: &Bench, cx: &mut VisualTestContext) -> Vec<(String, Span)> {
        let spans = bench
            .launcher
            .read_with(cx, |launcher, _| launcher.settings.home_spans());
        let mut sizes: Vec<_> = spans.into_iter().collect();
        sizes.sort();
        sizes
    }

    fn edit(bench: &Bench, cx: &mut VisualTestContext) {
        bench.page.update(cx, |page, cx| page.toggle(cx));
        draw(cx);
    }

    fn press(x: f32, y: f32, cx: &mut VisualTestContext) {
        cx.simulate_mouse_down(point(px(x), px(y)), MouseButton::Left, Modifiers::default());
    }

    fn slide(x: f32, y: f32, cx: &mut VisualTestContext) {
        cx.simulate_mouse_move(point(px(x), px(y)), MouseButton::Left, Modifiers::default());
        draw(cx);
    }

    fn lift(x: f32, y: f32, cx: &mut VisualTestContext) {
        cx.simulate_mouse_up(point(px(x), px(y)), MouseButton::Left, Modifiers::default());
        draw(cx);
    }

    fn hover(x: f32, y: f32, cx: &mut VisualTestContext) {
        cx.simulate_mouse_move(point(px(x), px(y)), None, Modifiers::default());
        draw(cx);
    }

    #[gpui::test]
    fn the_library_is_scanned_when_the_page_opens_and_when_the_window_returns(
        cx: &mut TestAppContext,
    ) {
        let (bench, cx) = bench(cx, &[VANILLA], "");
        assert_eq!(shown(&bench, cx), [VANILLA]);
        fs::create_dir_all(bench.library.join(SHADOW)).unwrap();
        cx.deactivate_window();
        assert_eq!(shown(&bench, cx), [VANILLA]);
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        assert_eq!(shown(&bench, cx), [VANILLA, SHADOW]);
        let partial = bench.page.read_with(cx, |page, cx| {
            let cards = page.cards(cx).unwrap();
            (cards[1].highlight, cards[1].action)
        });
        assert_eq!(
            partial,
            (Highlight::Muted, Some(Action::Verify { blocked: false }))
        );
    }

    #[gpui::test]
    fn a_right_click_offers_the_season_tools_until_they_are_dismissed(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, &[VANILLA, SHADOW], "");
        let offered =
            |cx: &mut VisualTestContext| bench.page.read_with(cx, |page, _| page.menu.is_some());
        let offer = |cx: &mut VisualTestContext| {
            let at = point(px(100.), px(80.));
            cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::default());
            draw(cx);
        };
        let scans = Rc::new(Cell::new(0));
        let count = scans.clone();
        let _scans = cx.update(|_, cx| {
            cx.subscribe(&bench.launcher, move |_, _: &ShearsEvent, _| {
                count.set(count.get() + 1)
            })
        });
        offer(cx);
        assert!(offered(cx));
        assert_eq!(scans.get(), 0);
        cx.simulate_keystrokes("escape");
        draw(cx);
        assert!(!offered(cx));

        offer(cx);
        press(1200., 700., cx);
        draw(cx);
        assert!(!offered(cx));

        offer(cx);
        cx.simulate_click(point(px(212.), px(96.)), Modifiers::default());
        draw(cx);
        assert!(offered(cx));
        cx.simulate_keystrokes("escape");
        draw(cx);
        assert!(!offered(cx));
        assert_eq!(shown(&bench, cx), [VANILLA, SHADOW]);

        edit(&bench, cx);
        offer(cx);
        assert!(!offered(cx));
    }

    #[gpui::test]
    fn a_click_on_the_action_of_a_card_does_not_open_its_season(cx: &mut TestAppContext) {
        let queue = "queue_paused = true\nqueue = [\
            { key = 'Y1S0_Vanilla', hm = false, library = '', verify = false }, \
            { key = 'Y5S3_ShadowLegacy', hm = false, library = '', verify = false }]";
        let (bench, cx) = bench(cx, &[], queue);
        fs::create_dir_all(bench.library.join(SHADOW)).unwrap();
        cx.deactivate_window();
        cx.update(|window, _| window.activate_window());
        draw(cx);
        let state = |cx: &mut VisualTestContext| {
            let action = bench
                .page
                .read_with(cx, |page, cx| page.cards(cx).unwrap()[0].action);
            let waiting = bench
                .launcher
                .read_with(cx, |launcher, _| launcher.download.waits(SHADOW, false));
            (action, waiting)
        };
        assert_eq!(state(cx), (Some(Action::Dequeue), true));
        cx.simulate_click(point(px(290.), px(140.)), Modifiers::default());
        draw(cx);
        assert_eq!(state(cx), (Some(Action::Verify { blocked: false }), false));
    }

    #[gpui::test]
    fn the_tools_reverse_and_reset_the_stored_layout(cx: &mut TestAppContext) {
        let saved = "home_order = ['Y9S2_HeatedMetal', 'Gone', 'Y1S0_Vanilla']\nhome_sizes = { Y1S0_Vanilla = '2x3' }";
        let (bench, cx) = bench(cx, &[VANILLA, SHADOW, METAL], saved);
        assert_eq!(shown(&bench, cx), [METAL, VANILLA, SHADOW]);

        bench.page.update(cx, |page, cx| page.reverse(cx));
        assert_eq!(shown(&bench, cx), [SHADOW, VANILLA, METAL]);
        assert_eq!(stored(&bench, cx), [SHADOW, "Gone", VANILLA, METAL]);
        assert_eq!(sizes(&bench, cx), [(VANILLA.to_owned(), (2, 3))]);

        bench.page.update(cx, |page, cx| {
            page.toggle(cx);
            page.reset(cx);
            page.reset(cx);
        });
        assert_eq!(shown(&bench, cx), [VANILLA, SHADOW, METAL]);
        assert!(stored(&bench, cx).is_empty());
        assert!(sizes(&bench, cx).is_empty());
        let state = |page: &HomePage, _: &App| (page.editing, page.erased);
        assert_eq!(bench.page.read_with(cx, state), (true, 2));
        bench.page.update(cx, |page, cx| page.toggle(cx));
        assert_eq!(bench.page.read_with(cx, state), (false, 0));
    }

    #[gpui::test]
    fn a_card_dropped_inside_the_grid_is_stored_and_one_dropped_outside_goes_back(
        cx: &mut TestAppContext,
    ) {
        let (bench, cx) = bench(cx, &[VANILLA, SHADOW, METAL], "reduce_motion = true");
        edit(&bench, cx);
        press(100., 80., cx);
        slide(110., 84., cx);
        lift(110., 84., cx);
        assert!(stored(&bench, cx).is_empty());

        press(100., 80., cx);
        slide(110., 84., cx);
        slide(450., 80., cx);
        assert_eq!(shown(&bench, cx), [SHADOW, VANILLA, METAL]);
        assert!(stored(&bench, cx).is_empty());
        lift(450., 80., cx);
        assert_eq!(stored(&bench, cx), [SHADOW, VANILLA, METAL]);

        bench.page.update(cx, |page, cx| page.reset(cx));
        draw(cx);
        press(100., 80., cx);
        slide(110., 84., cx);
        slide(450., 80., cx);
        slide(100., 80., cx);
        assert_eq!(shown(&bench, cx), [VANILLA, SHADOW, METAL]);
        assert!(stored(&bench, cx).is_empty());
        lift(100., 80., cx);
        assert_eq!(stored(&bench, cx), [VANILLA, SHADOW, METAL]);

        press(100., 80., cx);
        slide(110., 84., cx);
        slide(450., 80., cx);
        lift(450., 80., cx);
        assert_eq!(stored(&bench, cx), [SHADOW, VANILLA, METAL]);

        press(100., 80., cx);
        slide(110., 84., cx);
        slide(800., 80., cx);
        assert_eq!(shown(&bench, cx), [VANILLA, METAL, SHADOW]);
        lift(800., 400., cx);
        assert_eq!(shown(&bench, cx), [SHADOW, VANILLA, METAL]);
        assert_eq!(stored(&bench, cx), [SHADOW, VANILLA, METAL]);

        press(100., 80., cx);
        slide(110., 84., cx);
        slide(450., 80., cx);
        assert_eq!(shown(&bench, cx), [VANILLA, SHADOW, METAL]);
        cx.simulate_keystrokes("escape");
        assert_eq!(shown(&bench, cx), [SHADOW, VANILLA, METAL]);
        assert!(!cx.update(|_, cx| cx.has_active_drag()));
    }

    #[gpui::test]
    fn a_blocked_button_keeps_its_card_lit_and_offers_the_tools(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, &[SHADOW], "");
        bench.launcher.update(cx, |launcher, cx| {
            let other = Edition {
                key: SHADOW.into(),
                hm: true,
            };
            launcher.download =
                Downloader::with(State::Downloading, Some(other), false, Vec::new());
            cx.notify();
        });
        draw(cx);
        let action = bench
            .page
            .read_with(cx, |page, cx| page.cards(cx).unwrap()[0].action);
        assert_eq!(action, Some(Action::Play { blocked: true }));

        hover(700., 400., cx);
        let changes = Rc::new(Cell::new(0));
        let count = changes.clone();
        let _watch =
            cx.update(|_, cx| cx.observe(&bench.page, move |_, _| count.set(count.get() + 1)));
        hover(290., 140., cx);
        assert_eq!(changes.get(), 1);
        hover(100., 60., cx);
        assert_eq!(changes.get(), 1);
        hover(700., 400., cx);
        assert_eq!(changes.get(), 2);

        let at = point(px(290.), px(140.));
        cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::default());
        draw(cx);
        assert!(bench.page.read_with(cx, |page, _| page.menu.is_some()));
    }

    #[gpui::test]
    fn cards_that_still_move_do_not_trade_places_again(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, &[VANILLA, SHADOW, METAL], "");
        edit(&bench, cx);
        press(100., 80., cx);
        slide(110., 84., cx);
        slide(450., 80., cx);
        assert_eq!(shown(&bench, cx), [SHADOW, VANILLA, METAL]);
        slide(800., 80., cx);
        assert_eq!(shown(&bench, cx), [SHADOW, VANILLA, METAL]);
        cx.executor().advance_clock(FLIP.duration);
        draw(cx);
        slide(801., 80., cx);
        assert_eq!(shown(&bench, cx), [SHADOW, METAL, VANILLA]);
        lift(801., 80., cx);
        assert_eq!(stored(&bench, cx), [SHADOW, METAL, VANILLA]);
    }

    #[gpui::test]
    fn the_handle_previews_a_size_and_stores_it_on_release(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, &[VANILLA, SHADOW], "reduce_motion = true");
        edit(&bench, cx);
        press(317., 160., cx);
        lift(317., 160., cx);
        slide(700., 400., cx);
        assert!(bench.page.read_with(cx, |page, _| page.resize.is_none()));
        assert!(sizes(&bench, cx).is_empty());

        press(317., 160., cx);
        slide(317. + 340., 160. + 91., cx);
        let preview = bench
            .page
            .read_with(cx, |page, cx| page.cards(cx).unwrap()[0].span);
        assert_eq!(preview, (2, 3));
        assert!(sizes(&bench, cx).is_empty());
        lift(900., 700., cx);
        assert_eq!(sizes(&bench, cx), [(VANILLA.to_owned(), (2, 3))]);
        assert_eq!(shown(&bench, cx), [VANILLA, SHADOW]);
    }

    #[gpui::test]
    fn arrow_keys_move_the_focused_card_and_resize_at_the_focused_handle(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, &[VANILLA, SHADOW, METAL], "reduce_motion = true");
        edit(&bench, cx);
        cx.simulate_click(point(px(100.), px(80.)), Modifiers::default());
        cx.simulate_keystrokes("right alt-right");
        assert_eq!(stored(&bench, cx), [SHADOW, VANILLA, METAL]);
        cx.simulate_keystrokes("alt-right alt-right");
        assert_eq!(stored(&bench, cx), [SHADOW, METAL, VANILLA]);
        assert!(sizes(&bench, cx).is_empty());

        draw(cx);
        cx.update(|window, cx| window.focus_next(cx));
        cx.simulate_keystrokes("right down down space enter");
        assert_eq!(sizes(&bench, cx), [(VANILLA.to_owned(), (2, 4))]);
        cx.simulate_keystrokes("right right");
        assert_eq!(sizes(&bench, cx), [(VANILLA.to_owned(), (3, 4))]);
        cx.simulate_keystrokes("left left up up up");
        assert_eq!(sizes(&bench, cx), [(VANILLA.to_owned(), (1, 1))]);
        cx.simulate_keystrokes("down");
        assert!(sizes(&bench, cx).is_empty());

        cx.simulate_keystrokes("alt-left");
        assert_eq!(stored(&bench, cx), [SHADOW, VANILLA, METAL]);
        cx.simulate_keystrokes("down right");
        assert!(sizes(&bench, cx).is_empty());
    }
}
