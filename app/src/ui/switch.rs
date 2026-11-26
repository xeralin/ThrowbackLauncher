use crate::ui::px;
use gpui::{
    App, Div, ElementId, InteractiveElement, ParentElement, Stateful, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder,
};

use crate::ui::kit::focus_ring_inside;
use crate::ui::motion::{self, EASE_OUT, SLOW, sub};
use crate::ui::scroll::reveal;
use crate::ui::theme::{BLACK, BORDER, SURFACE_2, TEXT, Theme, shadow};

const TRAVEL: f32 = 16.;
const KNOB: f32 = 10.;
const INSET: f32 = 3.;
const PRESS_STRETCH: f32 = 1.25;

pub fn switch(
    id: impl Into<ElementId>,
    checked: bool,
    on_change: impl Fn(bool, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let id = id.into();
    let theme = Theme::get(cx);
    let (action, deep, knob_on) = (theme.action, theme.action_deep, theme.action_text);
    let hover = motion::hover(&id, SLOW, window, cx);
    let on = motion::transition(
        sub(&id, "on"),
        if checked { 1. } else { 0. },
        SLOW,
        EASE_OUT,
        window,
        cx,
    );
    let fill = SURFACE_2
        .mix(BORDER, hover.amount)
        .mix(action.mix(deep, hover.amount), on);
    let stretch = if hover.held { PRESS_STRETCH } else { 1. };
    let knob = div()
        .absolute()
        .top(px(INSET))
        .w(px(KNOB * stretch))
        .h(px(KNOB))
        .rounded(px(3.))
        .bg(TEXT.mix(knob_on, on).solid())
        .shadow(vec![shadow(0., 1., 2., 0., BLACK.alpha(0.45))]);
    div()
        .id(id)
        .flex_none()
        .w(px(34.))
        .h(px(18.))
        .rounded(px(6.))
        .border_1()
        .border_color(BORDER.mix(action, on).solid())
        .bg(fill.solid())
        .map(|element| hover.follow(element))
        .tab_index(0)
        .focus_visible(focus_ring_inside)
        .map(reveal)
        .on_click(move |_, window, cx| on_change(!checked, window, cx))
        .child(if checked {
            knob.right(px(INSET + TRAVEL * (1. - on)))
        } else {
            knob.left(px(INSET + TRAVEL * on))
        })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{IntoElement, Modifiers, TestAppContext, point};

    use super::*;
    use crate::ui::testing::{mount, press};

    #[gpui::test]
    fn keys_and_clicks_toggle(cx: &mut TestAppContext) {
        let checked = Rc::new(RefCell::new(false));
        let shown = checked.clone();
        let cx = mount(cx, move |window, cx| {
            let store = shown.clone();
            switch(
                "probe",
                *shown.borrow(),
                move |value, window, _| {
                    *store.borrow_mut() = value;
                    window.refresh();
                },
                window,
                cx,
            )
            .into_any_element()
        });
        press(cx, "tab space");
        assert!(*checked.borrow());
        press(cx, "enter");
        assert!(!*checked.borrow());
        cx.simulate_click(point(px(10.), px(9.)), Modifiers::default());
        assert!(*checked.borrow());
    }
}
