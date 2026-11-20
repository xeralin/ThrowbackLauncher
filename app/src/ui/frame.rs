use std::mem;

use gpui::{
    AnyElement, App, Bounds, CursorStyle, Decorations, Div, ElementId, Hsla, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Pixels, ResizeEdge, Size, Stateful,
    StatefulInteractiveElement, Styled, Tiling, Window, WindowBackgroundAppearance,
    WindowControlArea, div, point, prelude::FluentBuilder, size,
};

use crate::ui::icon::{WINDOW_CLOSE, icon};
use crate::ui::motion::{self, FAST};
use crate::ui::theme::{BLACK, BORDER, BRAND, SURFACE_2, TEXT, TEXT_MUTED, WHITE};
use crate::ui::{px, scale};

const EDGE: f32 = 4.;
const CORNER: f32 = 12.;
const RADIUS: f32 = 8.;
const LINE: f32 = 1.;
const VEIL: f32 = 5.5;
const GLYPH: f32 = 10.;
const GLYPH_OFFSET: f32 = 2.;

#[derive(Clone, Copy)]
enum Glyph {
    Minimize,
    Maximize,
    Restore,
    Close,
}

pub fn backdrop() -> WindowBackgroundAppearance {
    match cfg!(windows) {
        true => WindowBackgroundAppearance::Opaque,
        false => WindowBackgroundAppearance::Transparent,
    }
}

pub fn own(window: &Window) -> bool {
    cfg!(windows) || matches!(window.window_decorations(), Decorations::Client { .. })
}

fn floating(window: &Window) -> bool {
    let free = matches!(
        window.window_decorations(),
        Decorations::Client { tiling } if tiling == Tiling::default()
    );
    free && !window.is_maximized() && !window.is_fullscreen()
}

fn line(window: &Window) -> Pixels {
    gpui::px(if floating(window) { LINE } else { 0. })
}

pub fn radius(window: &Window) -> Pixels {
    gpui::px(if floating(window) { RADIUS - LINE } else { 0. })
}

pub fn veil(window: &Window) -> Pixels {
    gpui::px(if floating(window) { VEIL } else { 0. })
}

pub fn area(window: &Window) -> Bounds<Pixels> {
    let (line, room) = (line(window), window.viewport_size());
    let inner = size(room.width - 2. * line, room.height - 2. * line);
    Bounds::new(point(line, line), inner)
}

pub fn framed(content: Size<Pixels>) -> Size<Pixels> {
    let rim = gpui::px(if cfg!(windows) { 0. } else { 2. * LINE });
    size(content.width + rim, content.height + rim)
}

#[cfg(windows)]
pub fn border(dim: f32, window: &mut Window, cx: &mut App) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DwmSetWindowAttribute};

    let color = BORDER.mix(BLACK, dim);
    let channel = |value: f32| (value * 255.).round() as u32;
    let wanted = channel(color.r) | channel(color.g) << 8 | channel(color.b) << 16;
    let shown = window.use_keyed_state("frame-border", cx, |_, _| None);
    if *shown.read(cx) == Some(wanted) {
        return;
    }
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let value = std::ptr::from_ref(&wanted).cast();
    let size = size_of::<u32>() as u32;
    let attribute = DWMWA_BORDER_COLOR as u32;
    unsafe { DwmSetWindowAttribute(handle.hwnd.get() as HWND, attribute, value, size) };
    shown.update(cx, |shown, _| *shown = Some(wanted));
}

pub fn outline(dim: f32, window: &Window) -> Div {
    div().size_full().when(floating(window), |outline| {
        outline
            .rounded(gpui::px(RADIUS))
            .p(gpui::px(LINE))
            .bg(BORDER.mix(BLACK, dim).solid())
    })
}

pub fn grip(id: &'static str, area: Div, window: &mut Window, cx: &mut App) -> Stateful<Div> {
    let area = area.id(id);
    if !own(window) {
        return area;
    }
    if cfg!(windows) {
        return area.window_control_area(WindowControlArea::Drag);
    }
    let held = window.use_keyed_state(id, cx, |_, _| false);
    let moved = held.clone();
    area.on_mouse_down(MouseButton::Left, move |event, window, cx| {
        if event.click_count == 2 {
            window.zoom_window();
        }
        held.update(cx, |held, _| *held = event.click_count == 1);
    })
    .on_mouse_move(move |event, window, cx| {
        if moved.update(cx, |held, _| mem::take(held)) && event.dragging() {
            window.start_window_move();
        }
    })
    .on_mouse_down(MouseButton::Right, |event, window, _| {
        window.show_window_menu(event.position);
    })
}

pub fn controls(window: &mut Window, cx: &mut App) -> Option<Div> {
    if !own(window) {
        return None;
    }
    let minimize = control(
        "window-minimize",
        Glyph::Minimize,
        WindowControlArea::Min,
        window,
        cx,
    )
    .on_click(|_, window, _| window.minimize_window());
    let glyph = if window.is_maximized() {
        Glyph::Restore
    } else {
        Glyph::Maximize
    };
    let maximize = control("window-maximize", glyph, WindowControlArea::Max, window, cx)
        .on_click(|_, window, _| window.zoom_window());
    let close = control(
        "window-close",
        Glyph::Close,
        WindowControlArea::Close,
        window,
        cx,
    )
    .rounded_tr(radius(window))
    .on_click(|_, window, cx| crate::close_window(window, cx));
    let row = div().flex().flex_none().h_full();
    Some(row.child(minimize).child(maximize).child(close))
}

fn glyph(kind: Glyph, ink: Hsla) -> AnyElement {
    let side = (GLYPH * scale()).round();
    match kind {
        Glyph::Minimize => div()
            .w(gpui::px(side))
            .h(gpui::px(LINE))
            .bg(ink)
            .into_any_element(),
        Glyph::Maximize => div()
            .size(gpui::px(side))
            .border_1()
            .border_color(ink)
            .into_any_element(),
        Glyph::Restore => {
            let offset = (GLYPH_OFFSET * scale()).round();
            let (pane, offset) = (gpui::px(side - offset), gpui::px(offset));
            let part = || div().absolute().border_color(ink);
            div()
                .size(gpui::px(side))
                .child(part().bottom_0().left_0().size(pane).border_1())
                .child(
                    part()
                        .top_0()
                        .right_0()
                        .w(pane)
                        .h(offset)
                        .border_t_1()
                        .border_x_1(),
                )
                .child(
                    part()
                        .top(offset)
                        .right_0()
                        .w(offset)
                        .h(pane - offset)
                        .border_r_1()
                        .border_b_1(),
                )
                .into_any_element()
        }
        Glyph::Close => icon(WINDOW_CLOSE, 16.).text_color(ink).into_any_element(),
    }
}

fn control(
    id: &'static str,
    kind: Glyph,
    area: WindowControlArea,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let id = ElementId::from(id);
    let hover = motion::hover(&id, FAST, window, cx);
    let (fill, ink) = match area {
        WindowControlArea::Close => (BRAND, WHITE),
        _ => (SURFACE_2.srgb(), TEXT.srgb()),
    };
    div()
        .id(id)
        .occlude()
        .window_control_area(area)
        .flex()
        .items_center()
        .justify_center()
        .w(px(44.))
        .h_full()
        .bg(fill.alpha(hover.amount))
        .map(|element| hover.follow(element))
        .child(glyph(kind, TEXT_MUTED.mix(ink, hover.amount).solid()))
}

pub fn edges(window: &Window) -> Option<Div> {
    if !floating(window) {
        return None;
    }
    let zone = |edge: ResizeEdge| {
        let cursor = match edge {
            ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
            ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
            ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
            ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
        };
        div().absolute().occlude().cursor(cursor).on_mouse_down(
            MouseButton::Left,
            move |_, window, _| {
                window.start_window_resize(edge);
            },
        )
    };
    let (thin, long) = (gpui::px(EDGE), gpui::px(CORNER));
    let frame = div()
        .absolute()
        .inset_0()
        .child(zone(ResizeEdge::Top).top_0().left(long).right(long).h(thin))
        .child(
            zone(ResizeEdge::Bottom)
                .bottom_0()
                .left(long)
                .right(long)
                .h(thin),
        )
        .child(
            zone(ResizeEdge::Left)
                .left_0()
                .top(long)
                .bottom(long)
                .w(thin),
        )
        .child(
            zone(ResizeEdge::Right)
                .right_0()
                .top(long)
                .bottom(long)
                .w(thin),
        );
    let corners = [
        (ResizeEdge::TopLeft, true, true),
        (ResizeEdge::TopRight, true, false),
        (ResizeEdge::BottomLeft, false, true),
        (ResizeEdge::BottomRight, false, false),
    ];
    Some(corners.into_iter().fold(frame, |frame, (edge, top, left)| {
        let place = |zone: Div| {
            let zone = if top { zone.top_0() } else { zone.bottom_0() };
            if left { zone.left_0() } else { zone.right_0() }
        };
        frame
            .child(place(zone(edge)).w(long).h(thin))
            .child(place(zone(edge)).w(thin).h(long))
    }))
}
