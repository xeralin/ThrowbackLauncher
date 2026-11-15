use std::rc::Rc;

use crate::ui::px;
use gpui::{
    App, BoxShadow, Div, InteractiveElement, KeyDownEvent, ParentElement, Stateful,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder,
};

use crate::ui::kit::roving;
use crate::ui::motion::{self, EASE_IN_OUT, FAST};
use crate::ui::scroll::reveal;
use crate::ui::theme::{BORDER, TEXT, hex};

const CELL: f32 = 24.;
const RING: f32 = 2.;
const INNER_RADIUS: f32 = 5.;

pub type ColorCommit = Rc<dyn Fn(u32, &mut Window, &mut App)>;

pub fn color_bar(
    id: &'static str,
    colors: &[u32],
    value: u32,
    on_select: ColorCommit,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let count = colors.len();
    let chosen = colors.iter().position(|color| *color == value);
    let stop = chosen.unwrap_or(0);
    let handles = roving(&id.into(), count, window, cx);
    let step = {
        let (handles, on_select, colors) = (handles.clone(), on_select.clone(), colors.to_vec());
        move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
            let delta = match event.keystroke.key.as_str() {
                "right" | "down" => 1,
                "left" | "up" => count - 1,
                _ => return,
            };
            let next = (stop + delta) % count;
            cx.stop_propagation();
            on_select(colors[next], window, cx);
            window.focus(&handles[next], cx);
        }
    };
    let mut bar = div()
        .id(id)
        .flex()
        .flex_none()
        .h(px(32.))
        .rounded(px(6.))
        .border_1()
        .border_color(BORDER.solid())
        .on_key_down(step);
    for (index, color) in colors.iter().copied().enumerate() {
        let select = on_select.clone();
        let ring = motion::transition(
            (id, index),
            if chosen == Some(index) { 1. } else { 0. },
            FAST,
            EASE_IN_OUT,
            window,
            cx,
        );
        bar = bar.child(
            div()
                .id(index)
                .flex_none()
                .w(px(CELL))
                .h_full()
                .when(index == 0, |cell| cell.rounded_l(px(INNER_RADIUS)))
                .when(index + 1 == count, |cell| cell.rounded_r(px(INNER_RADIUS)))
                .bg(hex(color).solid())
                .border(px(RING))
                .border_color(TEXT.alpha(ring))
                .track_focus(&handles[index].clone().tab_stop(index == stop))
                .map(reveal)
                .focus_visible(|style| {
                    style.shadow(vec![
                        BoxShadow::new(px(0.), px(0.), TEXT.solid())
                            .spread_radius(px(2. * RING))
                            .inset(),
                    ])
                })
                .on_click(move |_, window, cx| select(color, window, cx)),
        );
    }
    bar
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui::{IntoElement, TestAppContext};

    use super::*;
    use crate::ui::testing::mount;

    #[gpui::test]
    fn arrows_rove_through_the_colors(cx: &mut TestAppContext) {
        let colors = [0x111111, 0x222222, 0x333333];
        let value = Rc::new(RefCell::new(0x222222));
        let shown = value.clone();
        let cx = mount(cx, move |window, cx| {
            let store = shown.clone();
            color_bar(
                "probe",
                &colors,
                *shown.borrow(),
                Rc::new(move |color, window, _| {
                    *store.borrow_mut() = color;
                    window.refresh();
                }),
                window,
                cx,
            )
            .into_any_element()
        });
        cx.simulate_keystrokes("tab right");
        assert_eq!(*value.borrow(), 0x333333);
        cx.simulate_keystrokes("down");
        assert_eq!(*value.borrow(), 0x111111);
        cx.simulate_keystrokes("left up");
        assert_eq!(*value.borrow(), 0x222222);
    }
}
