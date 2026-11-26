use crate::ui::px;
use gpui::{
    Div, Entity, ObjectFit, ParentElement, RetainAllImageCache, SharedString, Styled, StyledImage,
    container_query, div, img, linear_color_stop, linear_gradient, prelude::FluentBuilder,
    relative,
};

use crate::assets;
use crate::ui::theme::{BG, BLACK, Srgb, hex};

const BACKDROP: Srgb = hex(0x1a0d12);
const WIDEST: f32 = 3.2;
const FADE: f32 = 0.7;

pub fn key_art(
    name: &str,
    fade: bool,
    radius: f32,
    zoom: f32,
    cache: &Entity<RetainAllImageCache>,
) -> Div {
    let (name, cache) = (name.to_owned(), cache.clone());
    let picture = container_query(move |frame, window, _| {
        let covering = frame.width.max(frame.height * WIDEST);
        let sharp = assets::key_art(&name, f32::from(covering) * window.scale_factor());
        let quick = assets::key_art(&name, 0.);
        let grown = frame.map(|side| side * zoom);
        let shift = frame.map(|side| side * (1. - zoom) / 2.);
        let layer = |path: SharedString| {
            img(path)
                .image_cache(&cache)
                .absolute()
                .left(shift.width)
                .top(shift.height)
                .w(grown.width)
                .h(grown.height)
                .rounded(px(radius))
                .object_fit(ObjectFit::Cover)
        };
        div()
            .size_full()
            .child(layer(quick.clone()))
            .when(sharp != quick, |picture| picture.child(layer(sharp)))
    });
    div()
        .absolute()
        .inset_0()
        .rounded(px(radius))
        .bg(linear_gradient(
            135.,
            linear_color_stop(BACKDROP.solid(), 0.),
            linear_color_stop(BG.solid(), 1.),
        ))
        .child(picture)
        .when(fade, |art| art.child(shade(radius)))
}

pub fn shade(radius: f32) -> Div {
    let band = |low: f32, high: f32| {
        div().flex_1().bg(linear_gradient(
            0.,
            linear_color_stop(BLACK.alpha(low), 0.),
            linear_color_stop(BLACK.alpha(high), 1.),
        ))
    };
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .h(relative(FADE))
        .flex()
        .flex_col()
        .child(band(0.55, 0.))
        .child(band(0.9, 0.55).rounded_b(px(radius)))
}
