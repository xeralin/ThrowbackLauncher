use std::time::Duration;

use crate::ui::{Bound, px, units};
use gpui::{
    App, Bounds, CursorStyle, Div, ElementId, InteractiveElement, KeyDownEvent, ParentElement,
    Pixels, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder,
};

use crate::ui::kit::{focus_ring_inside, roving};
use crate::ui::motion::{self, EASE_OUT, FAST, sub};
use crate::ui::scroll::reveal;
use crate::ui::theme::{BORDER, FONT_MONO_12, TEXT, TEXT_LABEL, TEXT_MUTED, Theme, Typeface};

const INDICATOR: Duration = Duration::from_millis(300);
const GAP: f32 = 4.;
const PAD: f32 = 16.;

pub struct Tab {
    pub label: &'static str,
    pub disabled: bool,
}

fn group(
    key: ElementId,
    items: &[Tab],
    active: usize,
    on_select: Bound<usize>,
    radio: bool,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let action = Theme::get(cx).action.solid();
    let handles = roving(&key, items.len(), window, cx);
    let mut stops = items.iter().zip(&handles);
    let stranded = stops.any(|(item, handle)| item.disabled && handle.is_focused(window));
    if stranded && items.get(active).is_some_and(|item| !item.disabled) {
        window.focus(&handles[active], cx);
    }
    let spans = window.use_keyed_state(sub(&key, "spans"), cx, |_, _| Vec::<Bounds<Pixels>>::new());
    let (left, width) = {
        let spans = spans.read(cx);
        match (spans.first(), spans.get(active)) {
            (Some(first), Some(tab)) => (units(tab.left() - first.left()), units(tab.size.width)),
            _ => (0., 0.),
        }
    };
    let left = motion::transition(sub(&key, "left"), left, INDICATOR, EASE_OUT, window, cx);
    let width = motion::transition(sub(&key, "width"), width, INDICATOR, EASE_OUT, window, cx);

    let step = {
        let (handles, on_select) = (handles.clone(), on_select.clone());
        let enabled: Vec<usize> = (0..items.len())
            .filter(|index| !items[*index].disabled)
            .collect();
        move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
            let forward = match event.keystroke.key.as_str() {
                "right" => true,
                "left" => false,
                "down" if radio => true,
                "up" if radio => false,
                _ => return,
            };
            let Some(place) = enabled.iter().position(|index| *index == active) else {
                return;
            };
            let count = enabled.len();
            let next = enabled[(place + if forward { 1 } else { count - 1 }) % count];
            cx.stop_propagation();
            on_select(next, window, cx);
            window.focus(&handles[next], cx);
        }
    };
    let mut tabs = div()
        .on_children_prepainted(move |bounds, window, cx| {
            if *spans.read(cx) != bounds {
                spans.update(cx, |spans, _| *spans = bounds);
                window.request_animation_frame();
            }
        })
        .flex()
        .gap(px(GAP))
        .on_key_down(step);
    for (index, item) in items.iter().enumerate() {
        let current = index == active;
        let id = sub(&key, item.label);
        let hover = motion::hover(&id, FAST, window, cx);
        let color = if current {
            TEXT.solid()
        } else if item.disabled {
            TEXT_MUTED.alpha(0.4)
        } else {
            TEXT_MUTED.mix(TEXT, hover.amount).solid()
        };
        let tab = div()
            .id(id)
            .py(px(8.))
            .rounded(px(6.))
            .face(FONT_MONO_12, TEXT_LABEL)
            .text_color(color)
            .map(|element| hover.follow(element));
        let tab = if item.disabled {
            tab.when(!current, |tab| tab.cursor(CursorStyle::OperationNotAllowed))
        } else {
            let select = on_select.clone();
            tab.track_focus(&handles[index].clone().tab_stop(current))
                .focus_visible(focus_ring_inside)
                .map(reveal)
                .on_click(move |_, window, cx| select(index, window, cx))
        };
        tabs = tabs.child(tab.px(px(PAD)).child(item.label.to_uppercase()));
    }
    div().child(tabs).child(
        div()
            .absolute()
            .left(px(left))
            .bottom(px(-1.))
            .w(px(width))
            .h(px(2.))
            .bg(action),
    )
}

fn row() -> Div {
    div().flex().justify_between().pb(px(1.)).child(
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(px(1.))
            .bg(BORDER.solid()),
    )
}

fn plain(labels: &[&'static str]) -> Vec<Tab> {
    let enabled = |label: &&'static str| Tab {
        label,
        disabled: false,
    };
    labels.iter().map(enabled).collect()
}

pub fn tabs_of(
    id: &'static str,
    items: &[Tab],
    active: usize,
    on_select: Bound<usize>,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    row().child(group(
        id.into(),
        items,
        active,
        on_select,
        false,
        window,
        cx,
    ))
}

pub fn tabs(
    id: &'static str,
    labels: &[&'static str],
    active: usize,
    on_select: Bound<usize>,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    tabs_of(id, &plain(labels), active, on_select, window, cx)
}

pub fn options(
    id: &'static str,
    items: &[&'static str],
    active: usize,
    on_select: Bound<usize>,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    group(
        id.into(),
        &plain(items),
        active,
        on_select,
        true,
        window,
        cx,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{IntoElement, Modifiers, TestAppContext, point};

    use super::*;
    use crate::ui::testing::{mount, press};

    #[gpui::test]
    fn arrows_select_and_focus_the_neighbour(cx: &mut TestAppContext) {
        let active = Rc::new(RefCell::new(0));
        let shown = active.clone();
        let cx = mount(cx, move |window, cx| {
            let store = shown.clone();
            tabs(
                "probe",
                &["One", "Two", "Three"],
                *shown.borrow(),
                Rc::new(move |index, window, _| {
                    *store.borrow_mut() = index;
                    window.refresh();
                }),
                window,
                cx,
            )
            .into_any_element()
        });
        press(cx, "tab right");
        assert_eq!(*active.borrow(), 1);
        press(cx, "left left");
        assert_eq!(*active.borrow(), 2);
        *active.borrow_mut() = 0;
        press(cx, "space");
        assert_eq!(*active.borrow(), 2);
    }

    #[gpui::test]
    fn disabled_tabs_are_passed_over_and_hand_the_focus_on(cx: &mut TestAppContext) {
        let state = Rc::new(RefCell::new((0, [false, true, false, false])));
        let shown = state.clone();
        let cx = mount(cx, move |window, cx| {
            let (active, disabled) = *shown.borrow();
            let tab = |label, disabled| Tab { label, disabled };
            let labels = ["One", "Two", "Three", "Four"];
            let store = shown.clone();
            tabs_of(
                "probe",
                &[0, 1, 2, 3].map(|index| tab(labels[index], disabled[index])),
                active,
                Rc::new(move |index, window, _| {
                    store.borrow_mut().0 = index;
                    window.refresh();
                }),
                window,
                cx,
            )
            .into_any_element()
        });
        let active = || state.borrow().0;
        let set = |next, cx: &mut gpui::VisualTestContext| {
            *state.borrow_mut() = next;
            cx.update(|window, _| window.refresh());
        };
        press(cx, "tab right");
        assert_eq!(active(), 2);
        press(cx, "right right left");
        assert_eq!(active(), 3);

        set((0, [false, false, true, true]), cx);
        press(cx, "right");
        assert_eq!(active(), 1);
        set((1, [false, true, false, false]), cx);
        press(cx, "right left");
        assert_eq!(active(), 1);

        let two = point(px(88.), px(15.));
        set((0, [false, true, false, false]), cx);
        cx.simulate_click(two, Modifiers::default());
        assert_eq!(active(), 0);
        set((0, [false; 4]), cx);
        cx.simulate_click(two, Modifiers::default());
        assert_eq!(active(), 1);
    }
}
