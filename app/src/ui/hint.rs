use std::time::Duration;

use crate::ui::px;
use gpui::{
    App, Context, Div, ElementId, FocusHandle, InteractiveElement, KeystrokeEvent, ParentElement,
    Stateful, StatefulInteractiveElement, Styled, Subscription, Task, Window, anchored, deferred,
    div,
};

use crate::ui::button::glyph_button;
use crate::ui::icon::{INFO, icon};
use crate::ui::kit::OVERLAY;
use crate::ui::motion::{self, EASE_IN_OUT, sub};
use crate::ui::theme::{BORDER, FONT_BODY, SURFACE_2, TEXT, TEXT_UI, Typeface, lifted};

const OPEN_DELAY: Duration = Duration::from_millis(300);
const CLOSE_DELAY: Duration = Duration::from_millis(100);
const FADE: Duration = Duration::from_millis(100);
const ICON: f32 = 14.;
const GAP: f32 = 6.;

#[derive(Default)]
struct Hint {
    open: bool,
    over_icon: bool,
    over_panel: bool,
    timer: Option<Task<()>>,
    focused: Option<FocusHandle>,
    keys: Option<[Subscription; 2]>,
}

impl Hint {
    fn set_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.timer = None;
        if self.open == open {
            return;
        }
        self.open = open;
        self.focused = window.focused(cx);
        self.keys = open.then(|| {
            let this = cx.weak_entity();
            [
                cx.intercept_keystrokes(move |event, window, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        this.update(cx, |hint, cx| hint.set_open(false, window, cx))
                            .ok();
                    }
                }),
                cx.observe_keystrokes(Self::key),
            ]
        });
        cx.notify();
    }

    fn key(&mut self, _: &KeystrokeEvent, window: &mut Window, cx: &mut Context<Self>) {
        if window.focused(cx) != self.focused {
            self.set_open(false, window, cx);
        }
    }

    fn pointer(
        &mut self,
        icon: Option<bool>,
        panel: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.over_icon || self.over_panel;
        self.over_icon = icon.unwrap_or(self.over_icon);
        self.over_panel = panel.unwrap_or(self.over_panel);
        let inside = self.over_icon || self.over_panel;
        if inside == before {
            return;
        }
        let delay = if inside { OPEN_DELAY } else { CLOSE_DELAY };
        self.timer = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update_in(cx, |this, window, cx| this.set_open(inside, window, cx))
                .ok();
        }));
    }
}

pub fn hint(
    id: impl Into<ElementId>,
    content: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let id = id.into();
    let state = window.use_keyed_state(id.clone(), cx, |_, _| Hint::default());
    let open = state.read(cx).open;
    let opacity = motion::transition(
        sub(&id, "fade"),
        if open { 1. } else { 0. },
        FADE,
        EASE_IN_OUT,
        window,
        cx,
    );
    let (show, over_icon) = (state.clone(), state.clone());
    let button = glyph_button(sub(&id, "button"), icon(INFO, ICON), window, cx)
        .border_1()
        .m(px(-1.));
    let wrapper = div()
        .id(id)
        .flex_none()
        .on_hover(move |hovered, window, cx| {
            over_icon.update(cx, |hint, cx| {
                hint.pointer(Some(*hovered), None, window, cx)
            });
        })
        .child(button.on_click(move |_, window, cx| {
            show.update(cx, |hint, cx| hint.set_open(true, window, cx));
        }));
    if !open && opacity <= 0. {
        return wrapper;
    }
    let (over_panel, outside) = (state.clone(), state);
    let panel = div()
        .id("panel")
        .occlude()
        .w(px(240.))
        .rounded(px(6.))
        .border_1()
        .border_color(BORDER.solid())
        .bg(SURFACE_2.solid())
        .p(px(8.))
        .face(FONT_BODY, TEXT_UI)
        .line_height(px(1.375 * TEXT_UI))
        .text_color(TEXT.solid())
        .opacity(opacity)
        .shadow(lifted())
        .on_hover(move |hovered, window, cx| {
            over_panel.update(cx, |hint, cx| {
                hint.pointer(None, Some(*hovered), window, cx)
            });
        })
        .on_mouse_down_out(move |_, window, cx| {
            outside.update(cx, |hint, cx| {
                if !hint.over_icon {
                    hint.set_open(false, window, cx);
                }
            });
        })
        .child(content);
    wrapper.child(div().absolute().left_0().top(px(ICON + GAP)).child(
        deferred(anchored().snap_to_window_with_margin(px(8.)).child(panel)).with_priority(OVERLAY),
    ))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{AppContext, IntoElement, TestAppContext};

    use super::*;
    use crate::ui::Dismiss;
    use crate::ui::testing::mount;

    #[gpui::test]
    fn pointer_opens_late_and_escape_closes(cx: &mut TestAppContext) {
        let cx = mount(cx, |_, _| div().into_any_element());
        let dismissed = Rc::new(Cell::new(false));
        let flag = dismissed.clone();
        cx.update(|_, cx| {
            cx.on_action(move |_: &Dismiss, _| flag.set(true));
        });
        let hint = cx.new(|_| Hint::default());
        let open = |cx: &mut gpui::VisualTestContext| hint.read_with(cx, |hint, _| hint.open);
        hint.update_in(cx, |hint, window, cx| {
            hint.pointer(Some(true), None, window, cx)
        });
        cx.executor().advance_clock(OPEN_DELAY / 2);
        cx.run_until_parked();
        assert!(!open(cx));
        cx.executor().advance_clock(OPEN_DELAY);
        cx.run_until_parked();
        assert!(open(cx));
        hint.update_in(cx, |hint, window, cx| {
            hint.pointer(Some(false), None, window, cx)
        });
        hint.update_in(cx, |hint, window, cx| {
            hint.pointer(None, Some(true), window, cx)
        });
        cx.executor().advance_clock(OPEN_DELAY);
        cx.run_until_parked();
        assert!(open(cx));
        cx.simulate_keystrokes("escape");
        assert!(!open(cx));
        assert!(!dismissed.get());
        cx.simulate_keystrokes("escape");
        assert!(dismissed.get());
        hint.update_in(cx, |hint, window, cx| hint.set_open(true, window, cx));
        hint.update_in(cx, |hint, window, cx| {
            hint.pointer(None, Some(false), window, cx)
        });
        cx.executor().advance_clock(CLOSE_DELAY);
        cx.run_until_parked();
        assert!(!open(cx));
    }
}
