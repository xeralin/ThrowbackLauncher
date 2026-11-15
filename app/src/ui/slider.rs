use std::rc::Rc;

use crate::ui::{px, units};
use gpui::{
    App, DispatchPhase, Div, ElementId, FocusHandle, HitboxBehavior, InteractiveElement,
    KeyDownEvent, KeyUpEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Stateful, Styled, Subscription, Window, canvas, div, linear_color_stop,
    linear_gradient, prelude::FluentBuilder, relative,
};

use crate::ui::scroll::reveal;
use crate::ui::theme::{BLACK, BORDER, TEXT, Theme, hex, shadow};

const HEIGHT: f32 = 20.;
const THUMB: f32 = 16.;
const BIG_STEPS: f64 = 10.;

pub type ValueHandler = Rc<dyn Fn(f64, &mut Window, &mut App)>;
pub type DoneHandler = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct Range {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

impl Range {
    fn snap(&self, value: f64) -> f64 {
        let steps = ((value - self.min) / self.step).round();
        (self.min + steps * self.step).clamp(self.min, self.max)
    }

    fn fraction(&self, value: f64) -> f32 {
        ((value - self.min) / (self.max - self.min)) as f32
    }
}

struct Grip {
    focus: FocusHandle,
    dragging: bool,
    unsaved: Option<DoneHandler>,
    _blur: Subscription,
}

impl Grip {
    fn save(&mut self, window: &mut Window, cx: &mut App) {
        if let Some(done) = self.unsaved.take() {
            done(window, cx);
        }
    }
}

pub fn slider(
    id: impl Into<ElementId>,
    range: Range,
    value: f64,
    stops: &[u32],
    (on_change, on_done): (ValueHandler, DoneHandler),
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let id = id.into();
    let range = Rc::new(range);
    let grip = window.use_keyed_state(id.clone(), cx, |window, cx| {
        let focus = cx.focus_handle().tab_index(0).tab_stop(true);
        let blur = cx.on_blur(&focus, window, |_, window, cx| {
            cx.defer_in(window, |grip: &mut Grip, window, cx| grip.save(window, cx))
        });
        Grip {
            focus,
            dragging: false,
            unsaved: None,
            _blur: blur,
        }
    });
    let (typed, lifted) = (grip.clone(), grip.clone());
    let focus = grip.read(cx).focus.clone();
    let thumb = if focus.is_focused(window) && window.last_input_was_keyboard() {
        TEXT.srgb()
    } else {
        Theme::get(cx).action
    };

    let spans = stops.len() - 1;
    let track = div()
        .absolute()
        .inset_0()
        .flex()
        .children(stops.windows(2).enumerate().map(|(index, pair)| {
            div()
                .flex_1()
                .h_full()
                .when(index == 0, |span| span.rounded_l(px(HEIGHT / 2.)))
                .when(index + 1 == spans, |span| span.rounded_r(px(HEIGHT / 2.)))
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(hex(pair[0]).solid(), 0.),
                    linear_color_stop(hex(pair[1]).solid(), 1.),
                ))
        }));

    let sensor = {
        let (range, on_change, on_done) = (range.clone(), on_change.clone(), on_done.clone());
        canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |bounds, hitbox, window, _| {
                let place = Rc::new(move |x: Pixels, window: &mut Window, cx: &mut App| {
                    let travel = units(bounds.size.width) - THUMB;
                    let offset = units(x - bounds.left()) - THUMB / 2.;
                    let fraction = f64::from((offset / travel).clamp(0., 1.));
                    let next = range.snap(range.min + fraction * (range.max - range.min));
                    if next != value {
                        on_change(next, window, cx);
                    }
                });
                let (pressed, moved, released) = (grip.clone(), grip.clone(), grip);
                let drag = place.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble
                        && event.button == MouseButton::Left
                        && hitbox.is_hovered(window)
                    {
                        pressed.update(cx, |grip, _| grip.dragging = true);
                        place(event.position.x, window, cx);
                    }
                });
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble && moved.read(cx).dragging {
                        drag(event.position.x, window, cx);
                    }
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble
                        && event.button == MouseButton::Left
                        && released.read(cx).dragging
                    {
                        released.update(cx, |grip, _| grip.dragging = false);
                        on_done(window, cx);
                    }
                });
            },
        )
        .absolute()
        .inset_0()
    };

    let keys = {
        let (range, on_done) = (range.clone(), on_done.clone());
        move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
            let big = ((range.max - range.min) / BIG_STEPS).max(range.step);
            let next = match event.keystroke.key.as_str() {
                "right" | "up" => value + range.step,
                "left" | "down" => value - range.step,
                "pageup" => value + big,
                "pagedown" => value - big,
                "home" => range.min,
                "end" => range.max,
                _ => return,
            };
            cx.stop_propagation();
            let next = range.snap(next);
            if next != value {
                on_change(next, window, cx);
                typed.update(cx, |grip, _| grip.unsaved = Some(on_done.clone()));
            }
        }
    };

    div()
        .id(id)
        .w_full()
        .min_w_0()
        .h(px(HEIGHT))
        .rounded(px(HEIGHT / 2.))
        .border_1()
        .border_color(BORDER.solid())
        .track_focus(&focus)
        .map(reveal)
        .on_key_down(keys)
        .on_key_up(move |_: &KeyUpEvent, window, cx| {
            lifted.update(cx, |grip, cx| grip.save(window, cx))
        })
        .child(track)
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .right(px(THUMB))
                .child(
                    div()
                        .absolute()
                        .top(px((HEIGHT - 2. - THUMB) / 2.))
                        .left(relative(range.fraction(value)))
                        .size(px(THUMB))
                        .rounded(px(THUMB / 2.))
                        .border_2()
                        .border_color(TEXT.solid())
                        .bg(thumb.solid())
                        .cursor_pointer()
                        .shadow(vec![shadow(0., 1., 2., 0., BLACK.alpha(0.45))]),
                ),
        )
        .child(sensor)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui::{IntoElement, TestAppContext};

    use super::*;
    use crate::ui::testing::mount;

    #[gpui::test]
    fn keys_move_by_steps_and_report_the_release(cx: &mut TestAppContext) {
        let value = Rc::new(RefCell::new(10.));
        let done = Rc::new(RefCell::new(0));
        let (shown, counted) = (value.clone(), done.clone());
        let cx = mount(cx, move |window, cx| {
            let (store, count) = (shown.clone(), counted.clone());
            slider(
                "probe",
                Range {
                    min: 0.,
                    max: 359.,
                    step: 1.,
                },
                *shown.borrow(),
                &[0xff0000, 0x00ff00],
                (
                    Rc::new(move |next, window, _| {
                        *store.borrow_mut() = next;
                        window.refresh();
                    }),
                    Rc::new(move |_, _| *count.borrow_mut() += 1),
                ),
                window,
                cx,
            )
            .into_any_element()
        });
        cx.simulate_keystrokes("tab right right up left");
        assert_eq!(*value.borrow(), 12.);
        cx.simulate_keystrokes("pageup");
        assert_eq!(*value.borrow(), 48.);
        cx.simulate_keystrokes("end");
        assert_eq!(*value.borrow(), 359.);
        cx.simulate_keystrokes("right home down");
        assert_eq!(*value.borrow(), 0.);
        assert_eq!(*done.borrow(), 0);
        let release = KeyUpEvent {
            keystroke: gpui::Keystroke::parse("down").expect("keystroke"),
        };
        cx.simulate_event(release.clone());
        cx.simulate_event(release.clone());
        assert_eq!(*done.borrow(), 1);
        cx.simulate_keystrokes("right");
        cx.update(|window, cx| window.blur(cx));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
        assert_eq!((*value.borrow(), *done.borrow()), (1., 2));
        cx.simulate_event(release);
        assert_eq!(*done.borrow(), 2);
    }

    #[test]
    fn values_snap_to_the_step_grid() {
        let range = Range {
            min: 0.,
            max: 1.,
            step: 0.01,
        };
        assert_eq!(range.snap(0.654), 0.65);
        assert_eq!(range.snap(1.2), 1.);
        assert_eq!(range.snap(-0.3), 0.);
    }
}
