use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::ui::px;
use gpui::{
    Action, App, Bounds, Canvas, Context, DispatchPhase, Div, ElementId, Entity, Hitbox,
    HitboxBehavior, InteractiveElement, KeyDownEvent, KeyUpEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, ScrollEvent, ScrollHandle, Stateful,
    StatefulInteractiveElement, Styled, Task, Window, canvas, fill, point, size,
};

use crate::ui::motion::{self, Bezier, EASE_IN_OUT};
use crate::ui::theme::TEXT_MUTED;

const STRIP: f32 = 15.;
const INSET: f32 = 3.;
const THIN: f32 = 4.;
const WIDE: f32 = 9.;
const SHORTEST: f32 = 17.;
const PAGE: f32 = 0.875;
const LINE: f32 = 40.;
const GLIDE: Duration = Duration::from_millis(150);
const GLIDE_EASE: Bezier = Bezier(0.26, 0.46, 0.45, 0.94);
const LINGER: Duration = Duration::from_millis(750);
const FADE: Duration = Duration::from_millis(100);
const THUMB_REST: f32 = 0.6;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = scroll, no_json)]
struct Reveal(Bounds<Pixels>);

pub fn reveal<E: InteractiveElement + ParentElement>(control: E) -> E {
    let place = Rc::new(Cell::new(Bounds::default()));
    let measured = place.clone();
    control
        .on_key_up(move |event: &KeyUpEvent, window, cx| {
            if event.keystroke.key == "tab" {
                window.dispatch_action(Box::new(Reveal(place.get())), cx);
            }
        })
        .child(
            canvas(move |bounds, _, _| measured.set(bounds), |_, (), _, _| {})
                .absolute()
                .inset_0(),
        )
}

fn thumb(visible: Pixels, max: Pixels) -> (Pixels, Pixels) {
    let track = visible - px(2. * INSET);
    let length = (track * (visible / (visible + max)))
        .max(px(SHORTEST))
        .min(track);
    (length, track - length)
}

struct Glide {
    from: Pixels,
    to: Pixels,
    start: Instant,
}

struct Bar {
    handle: ScrollHandle,
    grab: Option<Pixels>,
    glide: Option<Glide>,
    moved: Instant,
    linger: Option<Task<()>>,
}

impl Bar {
    fn top(&self) -> Pixels {
        -self.handle.offset().y
    }

    fn max(&self) -> Pixels {
        self.handle.max_offset().y
    }

    fn page(&self) -> Pixels {
        self.handle.bounds().size.height * PAGE
    }

    fn thumb(&self) -> (Pixels, Pixels) {
        thumb(self.handle.bounds().size.height, self.max())
    }

    fn thumb_top(&self) -> Pixels {
        let (_, travel) = self.thumb();
        px(INSET) + travel * (self.top() / self.max())
    }

    fn touch(&mut self, cx: &mut Context<Self>) {
        self.moved = cx.background_executor().now();
        if self.linger.is_some() || self.max() <= px(0.) {
            return;
        }
        self.linger = Some(cx.spawn(async move |bar, cx| {
            while let Ok(rest) = bar.read_with(cx, |bar, cx| {
                (bar.moved + LINGER).saturating_duration_since(cx.background_executor().now())
            }) && !rest.is_zero()
            {
                cx.background_executor().timer(rest).await;
            }
            bar.update(cx, |bar, cx| {
                bar.linger = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn jump(&mut self, top: Pixels, cx: &mut Context<Self>) {
        self.glide = None;
        self.handle
            .set_offset(point(px(0.), -top.clamp(px(0.), self.max())));
        self.touch(cx);
        cx.notify();
    }

    fn goal(&self) -> Pixels {
        self.glide.as_ref().map_or(self.top(), |glide| glide.to)
    }

    fn glide(&mut self, top: Pixels, cx: &mut Context<Self>) {
        if cx.reduce_motion() {
            return self.jump(top, cx);
        }
        self.glide = Some(Glide {
            from: self.top(),
            to: top.clamp(px(0.), self.max()),
            start: cx.background_executor().now(),
        });
        self.touch(cx);
        cx.notify();
    }

    fn step(&mut self, now: Instant) -> bool {
        let Some(glide) = &self.glide else {
            return false;
        };
        let progress = now.duration_since(glide.start).as_secs_f32() / GLIDE.as_secs_f32();
        let top = if progress < 1. {
            glide.from + (glide.to - glide.from) * GLIDE_EASE.at(progress)
        } else {
            glide.to
        };
        self.handle.set_offset(point(px(0.), -top));
        if progress >= 1. {
            self.glide = None;
        }
        true
    }

    fn wheel(&mut self, event: &ScrollEvent, cx: &mut Context<Self>) -> bool {
        let from = self.goal();
        let top = (from - event.delta.pixel_delta(gpui::px(LINE)).y).clamp(px(0.), self.max());
        if top == from {
            return false;
        }
        if event.delta.precise() {
            self.jump(top, cx);
        } else {
            self.glide(top, cx);
        }
        true
    }

    fn reveal(&mut self, target: Bounds<Pixels>, cx: &mut Context<Self>) {
        let view = self.handle.bounds();
        let above = (target.top() - view.top()).min(px(0.));
        let below = (target.bottom() - view.bottom()).max(px(0.));
        self.jump(self.top() + above + below, cx);
    }

    fn press(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let (length, _) = self.thumb();
        let grab = y - self.handle.bounds().top() - self.thumb_top();
        if grab < px(0.) {
            self.glide(self.goal() - self.page(), cx);
        } else if grab > length {
            self.glide(self.goal() + self.page(), cx);
        } else {
            self.grab = Some(grab);
            cx.notify();
        }
    }

    fn drag(&mut self, y: Pixels, cx: &mut Context<Self>) -> bool {
        let Some(grab) = self.grab else {
            return false;
        };
        let (_, travel) = self.thumb();
        let along = y - grab - self.handle.bounds().top() - px(INSET);
        self.jump(self.max() * (along / travel), cx);
        true
    }
}

pub struct Scroll {
    id: ElementId,
    handle: ScrollHandle,
    bar: Entity<Bar>,
}

pub fn scroll(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> Scroll {
    let id = id.into();
    let bar = window.use_keyed_state(id.clone(), cx, |_, cx| Bar {
        handle: ScrollHandle::new(),
        grab: None,
        glide: None,
        moved: cx.background_executor().now(),
        linger: None,
    });
    let now = cx.background_executor().now();
    if bar.update(cx, |bar, _| bar.step(now)) {
        window.request_animation_frame();
    }
    let handle = bar.read(cx).handle.clone();
    Scroll { id, handle, bar }
}

impl Scroll {
    pub fn handle(&self) -> &ScrollHandle {
        &self.handle
    }

    pub fn keys<E: InteractiveElement>(&self, element: E) -> E {
        let bar = self.bar.clone();
        element.on_key_down(move |event: &KeyDownEvent, _, cx| {
            bar.update(cx, |bar, cx| {
                let top = match event.keystroke.key.as_str() {
                    "pageup" => bar.goal() - bar.page(),
                    "pagedown" => bar.goal() + bar.page(),
                    "home" => px(0.),
                    "end" => bar.max(),
                    _ => return,
                };
                cx.stop_propagation();
                bar.glide(top, cx);
            })
        })
    }

    pub fn wheel<E: InteractiveElement>(&self, element: E) -> E {
        let bar = self.bar.clone();
        element.on_scroll(move |event, _, cx| {
            bar.update(cx, |bar, cx| bar.wheel(event, cx));
        })
    }

    pub fn area(&self, area: Div) -> Stateful<Div> {
        let (moved, focused) = (self.bar.clone(), self.bar.clone());
        area.id(self.id.clone())
            .overflow_y_scroll()
            .track_scroll(&self.handle)
            .on_mouse_move(move |_, _, cx| moved.update(cx, Bar::touch))
            .on_action(move |Reveal(target): &Reveal, _, cx| {
                focused.update(cx, |bar, cx| bar.reveal(*target, cx))
            })
            .child(self.bar())
    }

    fn bar(&self) -> Canvas<(Hitbox, Option<Hitbox>)> {
        let (handle, bar) = (self.handle.clone(), self.bar.clone());
        canvas(
            move |_, window, _| {
                let bounds = handle.bounds();
                let view = window.insert_hitbox(bounds, HitboxBehavior::Normal);
                let strip = (handle.max_offset().y > px(0.)).then(|| {
                    window.insert_hitbox(
                        Bounds::new(
                            point(bounds.right() - px(STRIP), bounds.top()),
                            size(px(STRIP), bounds.size.height),
                        ),
                        HitboxBehavior::BlockMouseExceptScroll,
                    )
                });
                (view, strip)
            },
            move |_, (view, strip), window, cx| {
                window.on_mouse_event({
                    let bar = bar.clone();
                    move |event: &ScrollEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture
                            && view.should_handle_scroll(window)
                            && bar.update(cx, |bar, cx| bar.wheel(event, cx))
                        {
                            cx.stop_propagation();
                        }
                    }
                });
                window.on_mouse_event({
                    let bar = bar.clone();
                    move |_: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            bar.update(cx, |bar, cx| {
                                if bar.grab.take().is_some() {
                                    cx.notify();
                                }
                            });
                        }
                    }
                });
                let Some(strip) = strip else {
                    return;
                };
                let over = strip.is_hovered(window);
                let (wide, shown, top, (length, _)) = {
                    let bar = bar.read(cx);
                    let wide = over || bar.grab.is_some();
                    let shown = wide || bar.linger.is_some();
                    (wide, shown, bar.thumb_top(), bar.thumb())
                };
                let fade = motion::transition("fade", shown.into(), FADE, EASE_IN_OUT, window, cx);
                let spread =
                    motion::transition("spread", wide.into(), FADE, EASE_IN_OUT, window, cx);
                let width = px(THIN + (WIDE - THIN) * spread);
                let edge = strip.bounds.right() - px(INSET);
                window.paint_quad(
                    fill(
                        Bounds::new(
                            point(edge - width, strip.bounds.top() + top),
                            size(width, length),
                        ),
                        TEXT_MUTED.alpha((THUMB_REST + (1. - THUMB_REST) * spread) * fade),
                    )
                    .corner_radii(width / 2.),
                );
                let view = window.current_view();
                window.on_mouse_event({
                    let (bar, strip) = (bar.clone(), strip.clone());
                    move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble
                            && !bar.update(cx, |bar, cx| bar.drag(event.position.y, cx))
                            && strip.is_hovered(window) != over
                        {
                            cx.notify(view);
                        }
                    }
                });
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble
                        && event.button == MouseButton::Left
                        && strip.is_hovered(window)
                    {
                        bar.update(cx, |bar, cx| bar.press(event.position.y, cx));
                    }
                });
            },
        )
        .absolute()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui::{
        AnyElement, AppContext, IntoElement, Modifiers, ScrollDelta, ScrollEvent, TestAppContext,
        VisualTestContext, div,
    };

    use super::*;
    use crate::ui::input::{TextInput, text_field};
    use crate::ui::testing::{mount, press};

    fn mounted(
        cx: &mut TestAppContext,
        content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> (Scroll, &mut VisualTestContext) {
        let slot = Rc::new(RefCell::new(None));
        let out = slot.clone();
        let cx = mount(cx, move |window, cx| {
            let list = scroll("list", window, cx);
            let area = list.area(div().w(px(200.)).h(px(100.)).child(content(window, cx)));
            let root = list.wheel(list.keys(div().size_full())).child(area);
            *out.borrow_mut() = Some(list);
            root.into_any_element()
        });
        let list = slot.take().expect("rendered");
        (list, cx)
    }

    fn rows(
        cx: &mut TestAppContext,
        height: f32,
    ) -> (Scroll, Rc<Cell<usize>>, &mut VisualTestContext) {
        let clicks = Rc::new(Cell::new(0));
        let count = clicks.clone();
        let (list, cx) = mounted(cx, move |_, _| {
            let count = count.clone();
            div()
                .id("rows")
                .tab_index(0)
                .h(px(height))
                .on_click(move |_, _, _| count.set(count.get() + 1))
                .into_any_element()
        });
        (list, clicks, cx)
    }

    fn top(list: &Scroll) -> Pixels {
        -list.handle.offset().y
    }

    fn shown(list: &Scroll, cx: &mut VisualTestContext) -> bool {
        list.bar.read_with(cx, |bar, _| bar.linger.is_some())
    }

    fn wait(cx: &mut VisualTestContext, milliseconds: u64) {
        cx.executor()
            .advance_clock(Duration::from_millis(milliseconds));
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
    }

    #[test]
    fn thumb_is_the_visible_share_and_never_shorter_than_the_floor() {
        assert_eq!(thumb(px(100.), px(300.)), (px(23.5), px(70.5)));
        assert_eq!(thumb(px(100.), px(9900.)), (px(17.), px(77.)));
    }

    fn notch(cx: &mut VisualTestContext, x: f32, lines: f32) {
        cx.simulate_event(ScrollEvent {
            position: point(px(x), px(50.)),
            delta: ScrollDelta::Lines(point(0., lines)),
            ..Default::default()
        });
    }

    #[gpui::test]
    fn a_wheel_notch_glides_and_adds_up(cx: &mut TestAppContext) {
        let (list, _, cx) = rows(cx, 400.);
        notch(cx, 100., -3.);
        assert!(top(&list) < px(1.));
        wait(cx, 75);
        assert!(top(&list) > px(60.) && top(&list) < px(120.));
        notch(cx, 100., -3.);
        wait(cx, 150);
        assert_eq!(top(&list), px(240.));
        notch(cx, 100., -3.);
        notch(cx, 100., 3.);
        wait(cx, 150);
        assert_eq!(top(&list), px(180.));
        notch(cx, 300., 3.);
        wait(cx, 150);
        assert_eq!(top(&list), px(60.));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        notch(cx, 100., 1.);
        assert_eq!(top(&list), px(20.));
    }

    #[gpui::test]
    fn keys_page_through_and_jump_to_the_ends(cx: &mut TestAppContext) {
        let (list, _, cx) = rows(cx, 400.);
        cx.update(|_, cx| cx.set_reduce_motion(true));
        press(cx, "tab pagedown");
        assert_eq!(top(&list), px(87.5));
        press(cx, "end");
        assert_eq!(top(&list), px(300.));
        assert_eq!(list.bar.read_with(cx, |bar, _| bar.thumb_top()), px(73.5));
        press(cx, "pageup");
        assert_eq!(top(&list), px(212.5));
        press(cx, "home");
        assert_eq!(top(&list), px(0.));
    }

    #[gpui::test]
    fn home_and_end_stay_in_a_focused_text_field(cx: &mut TestAppContext) {
        let field = Rc::new(RefCell::new(None));
        let slot = field.clone();
        cx.update(|cx| cx.set_reduce_motion(true));
        let (list, cx) = mounted(cx, move |window, cx| {
            let input = slot
                .borrow_mut()
                .get_or_insert_with(|| cx.new(|cx| TextInput::new("abc", window, cx)))
                .clone();
            div()
                .h(px(400.))
                .child(text_field(&input, window, cx))
                .into_any_element()
        });
        press(cx, "tab home");
        cx.simulate_input("x");
        let text = field
            .borrow()
            .as_ref()
            .map(|input| input.read_with(cx, |input, _| input.text().to_owned()));
        assert_eq!(text.as_deref(), Some("xabc"));
        press(cx, "end");
        assert_eq!(top(&list), px(0.));
        press(cx, "pagedown");
        assert_eq!(top(&list), px(87.5));
    }

    #[gpui::test]
    fn tab_brings_the_focused_control_into_view(cx: &mut TestAppContext) {
        let (list, cx) = mounted(cx, |_, _| {
            div()
                .children((0..4usize).map(|index| {
                    let stop = div().id(("stop", index)).tab_index(0);
                    reveal(stop.h(px(100.)).mt(px(50.)))
                }))
                .into_any_element()
        });
        press(cx, "tab");
        assert_eq!(top(&list), px(50.));
        press(cx, "tab tab");
        assert_eq!(top(&list), px(350.));
        press(cx, "shift-tab");
        assert_eq!(top(&list), px(200.));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        press(cx, "shift-tab pagedown");
        assert_eq!(top(&list), px(137.5));
    }

    #[gpui::test]
    fn a_control_that_uses_the_key_keeps_it(cx: &mut TestAppContext) {
        let (list, cx) = mounted(cx, |_, _| {
            div()
                .id("slider")
                .tab_index(0)
                .h(px(400.))
                .on_key_down(|_, _, cx| cx.stop_propagation())
                .into_any_element()
        });
        press(cx, "tab pagedown end");
        assert_eq!(top(&list), px(0.));
    }

    #[gpui::test]
    fn strip_pages_and_the_thumb_drags(cx: &mut TestAppContext) {
        let (list, clicks, cx) = rows(cx, 400.);
        cx.update(|_, cx| cx.set_reduce_motion(true));
        let (none, left) = (Modifiers::default(), MouseButton::Left);
        let strip = |y: f32| point(px(192.), px(y));
        cx.simulate_mouse_move(strip(90.), None, none);
        cx.simulate_mouse_down(strip(90.), left, none);
        cx.simulate_mouse_up(strip(90.), left, none);
        assert_eq!(top(&list), px(87.5));
        cx.simulate_mouse_down(strip(30.), left, none);
        cx.simulate_mouse_move(strip(50.), Some(left), none);
        assert!((top(&list) - px(172.606)).abs() < px(0.01));
        cx.simulate_mouse_move(point(px(20.), px(500.)), Some(left), none);
        assert_eq!(top(&list), px(300.));
        cx.simulate_mouse_up(point(px(20.), px(500.)), left, none);
        cx.simulate_mouse_move(strip(10.), None, none);
        assert_eq!(top(&list), px(300.));
        cx.simulate_mouse_down(strip(10.), left, none);
        cx.simulate_mouse_up(strip(10.), left, none);
        assert_eq!(top(&list), px(212.5));
        assert_eq!(clicks.get(), 0);
        cx.simulate_click(point(px(100.), px(50.)), none);
        assert_eq!(clicks.get(), 1);
    }

    #[gpui::test]
    fn bar_lingers_after_the_last_move_or_scroll(cx: &mut TestAppContext) {
        let (list, _, cx) = rows(cx, 400.);
        let inside = point(px(100.), px(50.));
        assert!(!shown(&list, cx));
        cx.simulate_mouse_move(inside, None, Modifiers::default());
        assert!(shown(&list, cx));
        wait(cx, 700);
        cx.simulate_event(ScrollEvent {
            position: inside,
            delta: ScrollDelta::Pixels(point(px(0.), px(-40.))),
            ..Default::default()
        });
        assert_eq!(top(&list), px(40.));
        wait(cx, 700);
        assert!(shown(&list, cx));
        wait(cx, 50);
        assert!(!shown(&list, cx));
    }

    #[gpui::test]
    fn content_that_fits_gets_no_bar(cx: &mut TestAppContext) {
        let (list, clicks, cx) = rows(cx, 60.);
        cx.simulate_click(point(px(192.), px(50.)), Modifiers::default());
        assert_eq!(clicks.get(), 1);
        assert!(!shown(&list, cx));
        press(cx, "tab end");
        assert_eq!(top(&list), px(0.));
    }
}
