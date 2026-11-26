use std::time::Instant;

use crate::ui::px;
use gpui::{
    App, Div, ElementId, InteractiveElement, ParentElement, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Svg, Window, div, prelude::FluentBuilder,
};

use crate::ui::icon::{COPY, icon};
use crate::ui::kit::{check_mark, focus_ring_inside};
use crate::ui::motion::{self, EASE_IN_OUT, FAST, SLOW, sub};
use crate::ui::scroll::reveal;
use crate::ui::theme::{
    BORDER, FONT_MONO_8, REM, SURFACE_2, TEXT, TEXT_LABEL, TEXT_MUTED, Theme, Typeface, shadow,
};

fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> (Stateful<Div>, f32) {
    let id = id.into();
    let hover = motion::hover(&id, SLOW, window, cx);
    let halo = Theme::get(cx).action_glow.opacity(hover.amount);
    let button = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .h(px(32.))
        .px(px(1.1 * REM))
        .rounded(px(6.))
        .face(FONT_MONO_8, TEXT_LABEL)
        .whitespace_nowrap()
        .shadow(vec![shadow(0., 2., 14., 0., halo.into())])
        .tab_index(0)
        .focus_visible(focus_ring_inside)
        .map(reveal)
        .map(|element| hover.follow(element))
        .child(label.into());
    (button, hover.amount)
}

pub fn primary(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let (button, lit) = button(id, label, window, cx);
    let theme = Theme::get(cx);
    button
        .bg(theme.action.mix(theme.action_deep, lit).solid())
        .text_color(theme.action_text.solid())
}

pub fn secondary(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let (button, lit) = button(id, label, window, cx);
    button
        .border_1()
        .border_color(BORDER.solid())
        .bg(SURFACE_2.mix(BORDER, lit).solid())
        .text_color(TEXT_MUTED.mix(TEXT, lit).solid())
}

pub fn glyph_button(
    id: impl Into<ElementId>,
    glyph: Svg,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let id = id.into();
    let hover = motion::hover(&id, FAST, window, cx);
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(6.))
        .tab_index(0)
        .focus_visible(focus_ring_inside)
        .map(reveal)
        .map(|element| hover.follow(element))
        .child(glyph.text_color(TEXT_MUTED.mix(TEXT, hover.amount).solid()))
}

pub fn icon_button(
    id: impl Into<ElementId>,
    data: &'static [u8],
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    glyph_button(id, icon(data, 16.), window, cx).p(px(4.))
}

pub fn stepper_button(
    id: impl Into<ElementId>,
    data: &'static [u8],
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    glyph_button(id, icon(data, 16.), window, cx)
        .h_full()
        .px(px(2.))
}

pub fn copy_button(
    id: impl Into<ElementId>,
    copy: impl Fn(&mut Window, &mut App) -> bool + 'static,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let id = id.into();
    let copied = window.use_keyed_state(sub(&id, "copied"), cx, |_, _| None::<Instant>);
    let now = cx.background_executor().now();
    let mark = (*copied.read(cx)).and_then(|at| check_mark(now.duration_since(at), window));
    let rest = if mark.is_some() { 0. } else { 1. };
    let shown = motion::transition(sub(&id, "fade"), rest, SLOW, EASE_IN_OUT, window, cx);
    glyph_button(id, icon(COPY, 16.).opacity(shown), window, cx)
        .p(px(4.))
        .on_click(move |_, window, cx| {
            if copy(window, cx) {
                let now = cx.background_executor().now();
                copied.update(cx, |copied, cx| {
                    *copied = Some(now);
                    cx.notify();
                });
            }
        })
        .children(mark.map(|mark| mark.left(px(4.)).top(px(4.))))
}
