use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    App, ElementId, Entity, InteractiveElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, Window, div,
};

#[derive(Clone, Copy)]
pub struct Bezier(pub f32, pub f32, pub f32, pub f32);

pub const EASE: Bezier = Bezier(0.25, 0.1, 0.25, 1.);
pub const EASE_IN_OUT: Bezier = Bezier(0.4, 0., 0.2, 1.);
pub const EASE_OUT: Bezier = Bezier(0., 0., 0.2, 1.);
pub const EASE_OUT_CUBIC: Bezier = Bezier(0.33, 1., 0.68, 1.);
const LINEAR: Bezier = Bezier(0., 0., 1., 1.);
pub const RISE: f32 = 14.;
const RISE_TIME: Duration = Duration::from_millis(200);
pub const FAST: Duration = Duration::from_millis(150);
pub const SLOW: Duration = Duration::from_millis(200);
const PULSE: Duration = Duration::from_secs(2);
const PULSE_EASE: Bezier = Bezier(0.4, 0., 0.6, 1.);

impl Bezier {
    fn sample(start: f32, end: f32, t: f32) -> f32 {
        let c = 3. * start;
        let b = 3. * (end - start) - c;
        let a = 1. - c - b;
        ((a * t + b) * t + c) * t
    }

    pub fn at(self, x: f32) -> f32 {
        let Bezier(x1, y1, x2, y2) = self;
        let (mut low, mut high) = (0., 1.);
        for _ in 0..20 {
            let middle = (low + high) / 2.;
            if Self::sample(x1, x2, middle) < x {
                low = middle;
            } else {
                high = middle;
            }
        }
        Self::sample(y1, y2, (low + high) / 2.)
    }
}

struct Transition {
    from: f32,
    to: f32,
    start: Instant,
}

pub fn transition(
    key: impl Into<ElementId>,
    target: f32,
    duration: Duration,
    easing: Bezier,
    window: &mut Window,
    cx: &mut App,
) -> f32 {
    let reduced = cx.reduce_motion();
    tween(key, target, duration, easing, reduced, window, cx)
}

pub fn progress(
    key: impl Into<ElementId>,
    target: f32,
    duration: Duration,
    easing: Bezier,
    window: &mut Window,
    cx: &mut App,
) -> f32 {
    tween(key, target, duration, easing, false, window, cx)
}

fn tween(
    key: impl Into<ElementId>,
    target: f32,
    duration: Duration,
    easing: Bezier,
    reduced: bool,
    window: &mut Window,
    cx: &mut App,
) -> f32 {
    let now = cx.background_executor().now();
    let state = window.use_keyed_state(key, cx, |_, _| Transition {
        from: target,
        to: target,
        start: now,
    });
    let (value, running) = state.update(cx, |state, _| {
        let progress = |state: &Transition| {
            (now.duration_since(state.start).as_secs_f32() / duration.as_secs_f32()).min(1.)
        };
        let value =
            |state: &Transition| state.from + (state.to - state.from) * easing.at(progress(state));
        if state.to != target {
            state.from = if reduced { target } else { value(state) };
            state.to = target;
            state.start = now;
        }
        (value(state), state.from != state.to && progress(state) < 1.)
    });
    if running {
        window.request_animation_frame();
    }
    value
}

pub struct Hover {
    pub over: bool,
    pub held: bool,
    pub amount: f32,
    state: Entity<Pointer>,
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Pointer {
    over: bool,
    held: bool,
}

fn track(state: &Entity<Pointer>, change: impl FnOnce(&mut Pointer), cx: &mut App) {
    state.update(cx, |pointer, cx| {
        let before = *pointer;
        change(pointer);
        if *pointer != before {
            cx.notify();
        }
    });
}

impl Hover {
    pub fn follow<E: StatefulInteractiveElement + ParentElement>(&self, element: E) -> E {
        let [over, press, release, leave] = [(); 4].map(|()| self.state.clone());
        let area = div()
            .id("pointer")
            .absolute()
            .inset_0()
            .on_hover(move |hovered, _, cx| track(&over, |pointer| pointer.over = *hovered, cx));
        element
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                track(&press, |pointer| pointer.held = true, cx);
            })
            .capture_any_mouse_up(move |_, _, cx| {
                track(&release, |pointer| pointer.held = false, cx);
            })
            .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
                track(&leave, |pointer| pointer.held = false, cx);
            })
            .child(area)
    }
}

pub fn hover(id: &ElementId, duration: Duration, window: &mut Window, cx: &mut App) -> Hover {
    let state = window.use_keyed_state(id.clone(), cx, |_, _| Pointer::default());
    let Pointer { over, held } = *state.read(cx);
    let over = over || held;
    let target = if over { 1. } else { 0. };
    Hover {
        over,
        held,
        amount: transition(sub(id, "hover"), target, duration, EASE_IN_OUT, window, cx),
        state,
    }
}

pub fn sub(id: &ElementId, part: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), part.into())
}

#[derive(Default)]
struct Entrance {
    drawn: bool,
    start: Option<Instant>,
}

pub fn entrance(
    key: impl Into<ElementId>,
    duration: Duration,
    easing: Bezier,
    window: &mut Window,
    cx: &mut App,
) -> f32 {
    let now = cx.background_executor().now();
    let state = window.use_keyed_state(key, cx, |_, _| Entrance::default());
    let elapsed = state.update(cx, |state, _| {
        if std::mem::replace(&mut state.drawn, true) {
            now.duration_since(*state.start.get_or_insert(now))
        } else {
            Duration::ZERO
        }
    });
    if elapsed >= duration || cx.reduce_motion() {
        return 1.;
    }
    window.request_animation_frame();
    easing.at(elapsed.as_secs_f32() / duration.as_secs_f32())
}

pub fn rise(key: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> f32 {
    RISE * (1. - entrance(key, RISE_TIME, LINEAR, window, cx))
}

pub fn clock(key: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> Duration {
    let now = cx.background_executor().now();
    let start = *window.use_keyed_state(key, cx, |_, _| now).read(cx);
    now.duration_since(start)
}

pub fn pulse(window: &mut Window, cx: &mut App) -> f32 {
    let beat = clock("pulse", window, cx).as_secs_f32() / PULSE.as_secs_f32();
    window.request_animation_frame();
    let phase = 2. * beat.fract();
    1. - 0.5 * PULSE_EASE.at(if phase < 1. { phase } else { 2. - phase })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{
        InteractiveElement, IntoElement, Modifiers, ParentElement, Styled, TestAppContext,
        VisualTestContext, div, point, prelude::FluentBuilder, px,
    };

    use super::*;
    use crate::ui::testing::mount;

    fn draw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
    }

    struct Stage {
        lit: Rc<Cell<bool>>,
        clicks: Rc<Cell<usize>>,
    }

    fn stage(covering: bool, cx: &mut TestAppContext) -> (Stage, &mut VisualTestContext) {
        let stage = Stage {
            lit: Rc::default(),
            clicks: Rc::default(),
        };
        let (lit, clicks) = (stage.lit.clone(), stage.clicks.clone());
        let cx = mount(cx, move |window, cx| {
            let id = ElementId::from("control");
            let hover = hover(&id, FAST, window, cx);
            lit.set(hover.over);
            let clicked = clicks.clone();
            let control = div()
                .id(id)
                .absolute()
                .left(px(20.))
                .top(px(20.))
                .size(px(40.))
                .map(|element| hover.follow(element))
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    clicked.set(clicked.get() + 1);
                    window.refresh();
                });
            div()
                .size_full()
                .child(control)
                .when(covering && clicks.get() > 0, |root| {
                    root.child(div().absolute().inset_0().occlude())
                })
                .into_any_element()
        });
        draw(cx);
        (stage, cx)
    }

    #[gpui::test]
    fn a_control_covered_by_its_own_click_does_not_stay_lit(cx: &mut TestAppContext) {
        let (stage, cx) = stage(true, cx);
        let (left, none) = (MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(point(px(30.), px(30.)), None, none);
        draw(cx);
        assert!(stage.lit.get());

        cx.simulate_mouse_down(point(px(30.), px(30.)), left, none);
        cx.simulate_mouse_move(point(px(31.), px(30.)), left, none);
        draw(cx);
        assert!(stage.lit.get());

        cx.simulate_mouse_up(point(px(31.), px(30.)), left, none);
        draw(cx);
        assert_eq!(stage.clicks.get(), 1);
        cx.simulate_mouse_move(point(px(200.), px(200.)), None, none);
        draw(cx);
        assert!(!stage.lit.get());
    }

    #[gpui::test]
    fn a_control_is_lit_under_the_pointer_and_while_it_is_held(cx: &mut TestAppContext) {
        let (stage, cx) = stage(false, cx);
        let (left, none) = (MouseButton::Left, Modifiers::default());
        let [inside, moved, outside] =
            [(30., 30.), (31., 30.), (200., 200.)].map(|(x, y): (f32, f32)| point(px(x), px(y)));
        cx.simulate_mouse_move(inside, None, none);
        cx.simulate_mouse_down(inside, left, none);
        cx.simulate_mouse_move(moved, left, none);
        cx.simulate_mouse_up(moved, left, none);
        draw(cx);
        assert_eq!(stage.clicks.get(), 1);
        assert!(stage.lit.get());
        cx.simulate_mouse_move(outside, None, none);
        draw(cx);
        assert!(!stage.lit.get());

        cx.simulate_mouse_move(inside, None, none);
        cx.simulate_mouse_down(inside, left, none);
        cx.simulate_mouse_move(outside, left, none);
        draw(cx);
        assert!(stage.lit.get());
        cx.simulate_mouse_up(outside, left, none);
        draw(cx);
        assert_eq!(stage.clicks.get(), 1);
        assert!(!stage.lit.get());
    }

    #[test]
    fn bezier_matches_reference_points() {
        let points = [0.1, 0.25, 0.5, 0.75, 0.9];
        let ease = [0.094_796, 0.408_511, 0.802_403, 0.960_459, 0.994_316];
        let in_out = [0.025_863, 0.236_587, 0.775_561, 0.959_368, 0.994_354];
        for index in 0..points.len() {
            assert!((EASE.at(points[index]) - ease[index]).abs() < 1e-4);
            assert!((EASE_IN_OUT.at(points[index]) - in_out[index]).abs() < 1e-4);
        }
        assert!(EASE.at(0.).abs() < 1e-4);
        assert!((EASE.at(1.) - 1.).abs() < 1e-4);
    }
}
