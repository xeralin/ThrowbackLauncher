use std::cell::Cell;
use std::rc::Rc;

use crate::ui::{Bound, px};
use gpui::{
    App, AppContext, ClickEvent, Context, Div, ElementId, Entity, InteractiveElement, KeyDownEvent,
    ParentElement, Stateful, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder,
};

use crate::ui::button::stepper_button;
use crate::ui::icon::{CHEVRON_LEFT, CHEVRON_RIGHT};
use crate::ui::input::{Hooks, TextInput};
use crate::ui::kit::{disable, field};
use crate::ui::motion::sub;
use crate::ui::scroll::reveal;
use crate::ui::theme::{BORDER, CH, Theme};

pub struct Stepper {
    input: Entity<TextInput>,
    seen: u32,
}

struct Steps {
    value: Cell<u32>,
    min: u32,
    max: u32,
    on_commit: Bound<u32>,
}

impl Steps {
    fn current(&self, draft: &str) -> u32 {
        draft
            .parse::<u32>()
            .map_or(self.value.get(), |parsed| parsed.clamp(self.min, self.max))
    }

    fn commit(&self, next: u32, window: &mut Window, cx: &mut App) -> String {
        if self.value.replace(next) != next {
            (self.on_commit)(next, window, cx);
        }
        next.to_string()
    }

    fn step(
        &self,
        input: &mut TextInput,
        delta: i32,
        refocus: bool,
        window: &mut Window,
        cx: &mut Context<TextInput>,
    ) {
        let next = self
            .current(input.text())
            .saturating_add_signed(delta)
            .clamp(self.min, self.max);
        input.set_text(&self.commit(next, window, cx), cx);
        if refocus && (next <= self.min || next >= self.max) {
            window.focus(input.focus_handle(), cx);
        }
    }
}

impl Stepper {
    pub fn new(value: u32, window: &mut Window, cx: &mut App) -> Self {
        Self {
            input: cx.new(|cx| TextInput::new(&value.to_string(), window, cx).centered()),
            seen: value,
        }
    }

    pub fn build(
        &mut self,
        id: impl Into<ElementId>,
        value: u32,
        (min, max): (u32, u32),
        on_commit: Bound<u32>,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        let adopt = self.seen != value;
        self.seen = value;
        let steps = Rc::new(Steps {
            value: Cell::new(value),
            min,
            max,
            on_commit,
        });
        let digits = max.to_string().len();
        let (blur, revert) = (steps.clone(), steps.clone());
        self.input.update(cx, |input, cx| {
            if adopt {
                input.set_text(&value.to_string(), cx);
            }
            input.hooks = Hooks {
                max_length: None,
                filter: Some(Rc::new(move |raw| {
                    raw.chars()
                        .filter(char::is_ascii_digit)
                        .take(digits)
                        .collect()
                })),
                commit: Some(Rc::new(move |draft, window, cx| {
                    blur.commit(blur.current(draft), window, cx)
                })),
                revert: Some(Rc::new(move |input, cx| {
                    input.set_text(&revert.value.get().to_string(), cx);
                })),
            };
        });
        let current = steps.current(self.input.read(cx).text());
        let edge = if self.input.read(cx).focus_handle().is_focused(window) {
            Theme::get(cx).action
        } else {
            BORDER.srgb()
        };
        let nudge = |delta: i32| {
            let (steps, input) = (steps.clone(), self.input.clone());
            move |event: &ClickEvent, window: &mut Window, cx: &mut App| {
                let keyboard = matches!(event, ClickEvent::Keyboard(_));
                input.update(cx, |input, cx| {
                    steps.step(input, delta, keyboard, window, cx);
                });
            }
        };
        let (keys, input) = (steps.clone(), self.input.clone());
        let id = id.into();
        let less = stepper_button(sub(&id, "less"), CHEVRON_LEFT, window, cx);
        let more = stepper_button(sub(&id, "more"), CHEVRON_RIGHT, window, cx);
        field()
            .id(id)
            .flex_none()
            .border_color(edge.solid())
            .child(less.on_click(nudge(-1)).when(current <= min, disable))
            .child(
                div()
                    .w(px(3.5 * CH))
                    .h_full()
                    .map(reveal)
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        let delta = match event.keystroke.key.as_str() {
                            "up" => 1,
                            "down" => -1,
                            _ => return,
                        };
                        cx.stop_propagation();
                        input.update(cx, |input, cx| {
                            keys.step(input, delta, false, window, cx);
                        });
                    })
                    .child(self.input.clone()),
            )
            .child(more.on_click(nudge(1)).when(current >= max, disable))
    }
}

pub fn level_stepper(
    id: impl Into<ElementId>,
    value: u32,
    levels: &'static [u32],
    on_commit: Bound<u32>,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let id = id.into();
    let index = levels.iter().position(|level| *level == value);
    let focus = window
        .use_keyed_state(id.clone(), cx, |_, cx| {
            [(); 2].map(|_| cx.focus_handle().tab_index(0).tab_stop(true))
        })
        .read(cx)
        .clone();
    let nudge = |forward: bool| {
        let (on_commit, focus) = (on_commit.clone(), focus.clone());
        move |event: &ClickEvent, window: &mut Window, cx: &mut App| {
            let Some(next) = index.and_then(|index| {
                if forward {
                    index.checked_add(1)
                } else {
                    index.checked_sub(1)
                }
            }) else {
                return;
            };
            let Some(level) = levels.get(next) else {
                return;
            };
            on_commit(*level, window, cx);
            if matches!(event, ClickEvent::Keyboard(_)) {
                if next == 0 {
                    window.focus(&focus[1], cx);
                } else if next + 1 == levels.len() {
                    window.focus(&focus[0], cx);
                }
            }
        }
    };
    let first = index.is_none_or(|index| index == 0);
    let last = index.is_none_or(|index| index + 1 >= levels.len());
    let less = stepper_button(sub(&id, "less"), CHEVRON_LEFT, window, cx);
    let more = stepper_button(sub(&id, "more"), CHEVRON_RIGHT, window, cx);
    field()
        .id(id)
        .flex_none()
        .child(
            less.track_focus(&focus[0].clone().tab_stop(!first))
                .on_click(nudge(false))
                .when(first, disable),
        )
        .child(
            div()
                .flex()
                .justify_center()
                .w(px(5. * CH))
                .child(format!("{value}%")),
        )
        .child(
            more.track_focus(&focus[1].clone().tab_stop(!last))
                .on_click(nudge(true))
                .when(last, disable),
        )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui::{IntoElement, Render, TestAppContext, VisualTestContext};

    use super::*;
    use crate::ui::testing::{mount, press};

    struct Probe {
        stepper: Stepper,
        value: u32,
        commits: Rc<RefCell<Vec<u32>>>,
    }

    impl Render for Probe {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let this = cx.weak_entity();
            let commits = self.commits.clone();
            div().size_full().child(self.stepper.build(
                "probe",
                self.value,
                (1, 100),
                Rc::new(move |next, _, cx| {
                    commits.borrow_mut().push(next);
                    this.update(cx, |probe, cx| {
                        probe.value = next;
                        cx.notify();
                    })
                    .ok();
                }),
                window,
                cx,
            ))
        }
    }

    struct Bench<'a> {
        input: Entity<TextInput>,
        commits: Rc<RefCell<Vec<u32>>>,
        cx: &'a mut VisualTestContext,
    }

    fn bench(value: u32, cx: &mut TestAppContext) -> Bench<'_> {
        cx.update(crate::init);
        let commits = Rc::new(RefCell::new(Vec::new()));
        let log = commits.clone();
        let (view, cx) = cx.add_window_view(|window, cx| Probe {
            stepper: Stepper::new(value, window, cx),
            value,
            commits: log,
        });
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        let input = view.read_with(cx, |probe, _| probe.stepper.input.clone());
        cx.update(|window, cx| {
            let handle = input.read(cx).focus_handle().clone();
            window.focus(&handle, cx);
        });
        cx.run_until_parked();
        Bench { input, commits, cx }
    }

    impl Bench<'_> {
        fn draft(&mut self) -> String {
            self.input
                .read_with(self.cx, |input, _| input.text().to_owned())
        }
    }

    #[gpui::test]
    fn arrows_step_and_commit(cx: &mut TestAppContext) {
        let mut bench = bench(25, cx);
        bench.cx.simulate_keystrokes("up up down");
        assert_eq!(*bench.commits.borrow(), [26, 27, 26]);
        assert_eq!(bench.draft(), "26");
    }

    #[gpui::test]
    fn typing_keeps_digits_and_clamps_on_blur(cx: &mut TestAppContext) {
        let mut bench = bench(25, cx);
        bench.cx.simulate_keystrokes("ctrl-a");
        bench.cx.simulate_input("5x0019");
        assert_eq!(bench.draft(), "500");
        bench.cx.simulate_keystrokes("enter");
        assert_eq!(*bench.commits.borrow(), [100]);
        assert_eq!(bench.draft(), "100");
    }

    #[gpui::test]
    fn empty_draft_falls_back_to_the_value(cx: &mut TestAppContext) {
        let mut bench = bench(25, cx);
        bench.cx.simulate_keystrokes("ctrl-a backspace enter");
        assert!(bench.commits.borrow().is_empty());
        assert_eq!(bench.draft(), "25");
    }

    #[gpui::test]
    fn escape_restores_without_committing(cx: &mut TestAppContext) {
        let mut bench = bench(25, cx);
        bench.cx.simulate_keystrokes("ctrl-a");
        bench.cx.simulate_input("7");
        bench.cx.simulate_keystrokes("escape");
        assert!(bench.commits.borrow().is_empty());
        assert_eq!(bench.draft(), "25");
    }

    #[gpui::test]
    fn steps_stop_at_the_bounds(cx: &mut TestAppContext) {
        let mut bench = bench(99, cx);
        bench.cx.simulate_keystrokes("up up up");
        assert_eq!(*bench.commits.borrow(), [100]);
        assert_eq!(bench.draft(), "100");
    }

    #[gpui::test]
    fn level_buttons_walk_the_levels(cx: &mut TestAppContext) {
        static LEVELS: [u32; 3] = [75, 100, 125];
        let value = Rc::new(RefCell::new(100));
        let shown = value.clone();
        let cx = mount(cx, move |window, cx| {
            let store = shown.clone();
            level_stepper(
                "probe",
                *shown.borrow(),
                &LEVELS,
                Rc::new(move |level, window, _| {
                    *store.borrow_mut() = level;
                    window.refresh();
                }),
                window,
                cx,
            )
            .into_any_element()
        });
        press(cx, "tab enter");
        assert_eq!(*value.borrow(), 75);
        press(cx, "enter enter");
        assert_eq!(*value.borrow(), 125);
        press(cx, "enter");
        assert_eq!(*value.borrow(), 100);
    }
}
