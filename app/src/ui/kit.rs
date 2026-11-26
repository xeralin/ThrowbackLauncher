use std::panic::Location;
use std::time::{Duration, Instant};

use crate::ui::{frame, px};
use gpui::{
    AbsoluteLength, Anchor, AnyView, App, AppContext, AvailableSpace, Bounds, BoxShadow, Context,
    CursorStyle, Deferred, Div, Element, ElementId, FocusHandle, GlobalElementId,
    InspectorElementId, InteractiveElement, IntoElement, LayoutId, ParentElement, Pixels,
    Refineable, Render, ShapedLine, SharedString, Stateful, StatefulInteractiveElement, Style,
    StyleRefinement, Styled, TextAlign, TextRun, Transformation, Window, anchored, deferred, div,
    point, prelude::FluentBuilder, size,
};

use crate::ui::icon::{CHECK_BOLD, icon};
use crate::ui::motion::{self, EASE, FAST, Hover, sub};
use crate::ui::scroll::reveal;
use crate::ui::theme::{
    BORDER, FONT_BODY, FONT_DISPLAY_BOLD, FONT_MONO, NOTICE_BG, NOTICE_EDGE, NOTICE_LINK,
    NOTICE_LINK_HOVER, NOTICE_TEXT, REM, SUCCESS, SURFACE, SURFACE_2, TEXT, TEXT_LABEL, TEXT_MUTED,
    TEXT_UI, Typeface,
};

pub const OVERLAY: usize = 1;
const PREVIEW: usize = 2;
pub const NAV_BREAKPOINT: f32 = 900.;
pub const TOAST: usize = 3;
const FADE_EDGE: f32 = 28.;
const SAVE_CHECK: Duration = Duration::from_millis(1400);

pub fn roving(id: &ElementId, count: usize, window: &mut Window, cx: &mut App) -> Vec<FocusHandle> {
    window
        .use_keyed_state(sub(id, "roving"), cx, |_, cx| {
            (0..count)
                .map(|_| cx.focus_handle().tab_index(0))
                .collect::<Vec<_>>()
        })
        .read(cx)
        .clone()
}

pub fn inner_ring() -> BoxShadow {
    BoxShadow::new(px(0.), px(0.), TEXT.solid())
        .spread_radius(px(2.))
        .inset()
}

pub fn focus_ring_inside(style: StyleRefinement) -> StyleRefinement {
    style.border_color(TEXT.solid()).shadow(vec![inner_ring()])
}

fn cover(control: &mut Stateful<Div>) -> Div {
    let border = control.style().border_widths.clone();
    let outward = |width: Option<AbsoluteLength>| match width.unwrap_or_default() {
        AbsoluteLength::Pixels(width) => AbsoluteLength::Pixels(-width),
        AbsoluteLength::Rems(width) => AbsoluteLength::Rems(-width),
    };
    div()
        .absolute()
        .top(outward(border.top))
        .right(outward(border.right))
        .bottom(outward(border.bottom))
        .left(outward(border.left))
        .occlude()
}

pub fn disable(mut control: Stateful<Div>) -> Stateful<Div> {
    let cover = cover(&mut control).cursor(CursorStyle::OperationNotAllowed);
    control
        .tab_stop(false)
        .opacity(0.4)
        .capture_key_down(|event, window, _| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                window.prevent_default();
            }
        })
        .child(cover)
}

pub fn busy(mut control: Stateful<Div>, window: &mut Window, cx: &mut App) -> Stateful<Div> {
    let cover = cover(&mut control);
    control.opacity(motion::pulse(window, cx)).child(cover)
}

pub fn panel() -> Div {
    div()
        .rounded(px(8.))
        .border_1()
        .border_color(BORDER.solid())
        .bg(SURFACE.solid())
}

pub fn icon_box() -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(32.))
        .rounded(px(8.))
        .border_1()
        .border_color(BORDER.solid())
        .pl(px(0.2 * REM))
        .pr(px(0.075 * REM))
}

pub fn row() -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .min_h(px(32.))
}

pub fn heading(label: &'static str) -> Div {
    div()
        .flex_none()
        .relative()
        .top(px(1.))
        .face(FONT_DISPLAY_BOLD, 1.05 * REM)
        .text_color(TEXT.solid())
        .child(label)
}

pub fn label(heading_text: &'static str) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .child(heading(heading_text))
}

pub fn field() -> Div {
    div()
        .flex()
        .items_center()
        .h(px(32.))
        .rounded(px(6.))
        .border_1()
        .border_color(BORDER.solid())
        .bg(SURFACE_2.solid())
        .face(FONT_MONO, TEXT_UI)
        .text_color(TEXT.solid())
}

fn code_box(size: f32) -> Div {
    div()
        .flex_none()
        .rounded(px(6.))
        .border_1()
        .border_color(BORDER.solid())
        .bg(SURFACE_2.solid())
        .face(FONT_MONO, size)
        .line_height(px(size))
        .text_color(TEXT.solid())
}

pub fn chip(content: impl Into<SharedString>) -> Div {
    code_box(TEXT_UI)
        .px(px(0.15 * TEXT_UI))
        .pt(px(0.18 * TEXT_UI))
        .pb(px(0.12 * TEXT_UI))
        .child(content.into())
}

pub fn label_chip(content: impl Into<SharedString>) -> Div {
    code_box(TEXT_LABEL)
        .px(px(3.))
        .py(px(2.))
        .text_color(TEXT_MUTED.solid())
        .child(content.into())
}

pub fn box_text(content: impl Into<SharedString>) -> Div {
    div()
        .relative()
        .top(px(2.))
        .face(FONT_DISPLAY_BOLD, TEXT_UI)
        .text_color(TEXT.solid())
        .whitespace_nowrap()
        .child(content.into())
}

pub fn code_chip(content: impl Into<SharedString>) -> Div {
    code_box(11.)
        .px(px(2.))
        .pt(px(3.))
        .pb(px(2.))
        .whitespace_nowrap()
        .child(content.into())
}

pub fn version_chip(version: &str) -> Div {
    label_chip(format!("v{}", version.strip_prefix('v').unwrap_or(version)))
}

pub fn list_row() -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .min_h(px(32.))
        .rounded(px(6.))
        .border_1()
        .border_color(BORDER.solid())
        .bg(SURFACE_2.solid())
        .px(px(8.))
        .face(FONT_MONO, TEXT_LABEL)
        .text_color(TEXT.solid())
}

struct Truncate(SharedString);

pub fn truncate(content: impl Into<SharedString>) -> impl IntoElement {
    Truncate(content.into())
}

impl IntoElement for Truncate {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Truncate {
    type RequestLayoutState = ();
    type PrepaintState = ShapedLine;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, ()) {
        let line = StyleRefinement::default()
            .flex_1()
            .min_w_0()
            .h(window.line_height());
        let mut style = Style::default();
        style.refine(&line);
        let width = plain(&self.0, window).width;
        let layout = window.request_measured_layout(style, move |known, space, _, _| {
            let natural = match space.width {
                AvailableSpace::MinContent => Pixels::ZERO,
                AvailableSpace::MaxContent => width,
                AvailableSpace::Definite(room) => width.min(room),
            };
            size(
                known.width.unwrap_or(natural),
                known.height.unwrap_or_default(),
            )
        });
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) -> ShapedLine {
        let text = self.0.clone();
        let plain = plain(&text, window);
        let room = bounds.size.width;
        if plain.width <= room {
            return plain;
        }
        let style = window.text_style();
        let size = style.font_size.to_pixels(window.rem_size());
        let runs: Vec<TextRun> = text
            .char_indices()
            .map(|(start, letter)| {
                let end = start + letter.len_utf8();
                let middle = (plain.x_for_index(start) + plain.x_for_index(end)) / 2.;
                TextRun {
                    color: style.color.opacity((room - middle) / px(FADE_EDGE)),
                    ..style.to_run(end - start)
                }
            })
            .collect();
        window.text_system().shape_line(text, size, &runs, None)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        line: &mut ShapedLine,
        window: &mut Window,
        cx: &mut App,
    ) {
        let height = window.line_height();
        line.paint(bounds.origin, height, TextAlign::Left, None, window, cx)
            .ok();
    }
}

fn plain(text: &SharedString, window: &Window) -> ShapedLine {
    let style = window.text_style();
    let size = style.font_size.to_pixels(window.rem_size());
    let run = style.to_run(text.len());
    window
        .text_system()
        .shape_line(text.clone(), size, &[run], None)
}

pub fn note() -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .rounded_r(px(4.))
        .border_l_3()
        .border_color(NOTICE_EDGE.solid())
        .bg(NOTICE_BG.solid())
        .px(px(0.4 * REM))
        .py(px(0.35 * REM))
        .face(FONT_BODY, TEXT_UI)
        .line_height(px(1.5 * TEXT_UI))
        .text_color(NOTICE_TEXT.solid())
        .whitespace_nowrap()
}

pub fn note_link(id: impl Into<ElementId>, content: &'static str) -> Stateful<Div> {
    note_run(content)
        .id(id)
        .tab_index(0)
        .focus_visible(focus_ring_inside)
        .map(reveal)
}

fn note_run(content: &'static str) -> Div {
    div()
        .rounded(px(6.))
        .cursor_pointer()
        .line_height(px(1.25 * TEXT_UI))
        .text_color(NOTICE_LINK.solid())
        .underline()
        .hover(|style| style.text_color(NOTICE_LINK_HOVER.solid()))
        .child(content)
}

pub fn note_external(
    id: impl Into<ElementId>,
    content: &'static str,
    url: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let id = id.into();
    let hover = motion::hover(&id, FAST, window, cx);
    external(id, note_run(content), url, &hover, window, cx)
}

pub fn external(
    id: ElementId,
    link: Div,
    url: impl Into<SharedString>,
    hover: &Hover,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let url = url.into();
    let focus = window
        .use_keyed_state(sub(&id, "focus"), cx, |_, cx| {
            cx.focus_handle().tab_index(0).tab_stop(true)
        })
        .read(cx)
        .clone();
    let previewed = hover.over || (focus.is_focused(window) && window.last_input_was_keyboard());
    link.id(id)
        .track_focus(&focus)
        .focus_visible(focus_ring_inside)
        .map(reveal)
        .map(|element| hover.follow(element))
        .when(previewed, |link| {
            link.children(link_preview(url.clone(), window))
        })
        .on_click(move |_, _, cx| cx.open_url(&url))
}

struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .rounded(px(6.))
            .border_1()
            .border_color(BORDER.solid())
            .bg(SURFACE_2.solid())
            .px(px(6.))
            .py(px(4.))
            .face(FONT_MONO, 11.)
            .text_color(TEXT.solid())
            .child(self.0.clone())
    }
}

pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let text = text.into();
    move |_, cx| cx.new(|_| Tip(text.clone())).into()
}

pub fn link_preview(url: SharedString, window: &Window) -> Option<Deferred> {
    let area = frame::area(window);
    let viewport = area.size;
    (viewport.width >= px(NAV_BREAKPOINT)).then(|| {
        deferred(
            anchored()
                .position(area.bottom_left() + point(px(8.), px(-8.)))
                .anchor(Anchor::BottomLeft)
                .child(
                    div()
                        .max_w(viewport.width - px(16.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(BORDER.solid())
                        .bg(SURFACE_2.solid())
                        .px(px(3.))
                        .pt(px(3.))
                        .pb(px(2.))
                        .face(FONT_MONO, 11.)
                        .line_height(px(11.))
                        .text_color(TEXT.solid())
                        .text_decoration_0()
                        .child(url),
                ),
        )
        .with_priority(PREVIEW)
    })
}

pub fn check_mark(elapsed: Duration, window: &mut Window) -> Option<Div> {
    let progress = elapsed.as_secs_f32() / SAVE_CHECK.as_secs_f32();
    if progress >= 1. {
        return None;
    }
    window.request_animation_frame();
    let (opacity, scale) = if progress < 0.2 {
        let eased = EASE.at(progress / 0.2);
        (eased, 0.6 + 0.4 * eased)
    } else if progress < 0.75 {
        (1., 1.)
    } else {
        (1. - EASE.at((progress - 0.75) / 0.25), 1.)
    };
    Some(
        div().absolute().opacity(opacity).child(
            icon(CHECK_BOLD, 16.)
                .text_color(SUCCESS.solid())
                .with_transformation(Transformation::scale(size(scale, scale))),
        ),
    )
}

pub fn save_check(saved: Option<Instant>, window: &mut Window, cx: &App) -> Option<Div> {
    let elapsed = cx.background_executor().now().duration_since(saved?);
    check_mark(elapsed, window).map(|check| check.right(px(10.)).top(px(8.)))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use gpui::{Modifiers, TestAppContext};

    use super::*;
    use crate::ui::button::icon_button;
    use crate::ui::icon::FOLDER;
    use crate::ui::testing::{mount, press};

    #[gpui::test]
    fn a_fading_label_is_as_wide_as_its_text(cx: &mut TestAppContext) {
        let clicks = Rc::new(Cell::new(0));
        let counted = clicks.clone();
        let cx = mount(cx, move |_, _| {
            let counted = counted.clone();
            div()
                .flex()
                .child(div().flex().min_w_0().child(truncate("Throwback Launcher")))
                .child(
                    div()
                        .id("after")
                        .size(px(20.))
                        .on_click(move |_, _, _| counted.set(counted.get() + 1)),
                )
                .into_any_element()
        });
        let text = SharedString::from("Throwback Launcher");
        let width = cx.update(|window, _| plain(&text, window).width);
        assert!(width > px(40.));
        cx.simulate_click(point(width + px(10.), px(5.)), Modifiers::default());
        assert_eq!(clicks.get(), 1);
        cx.simulate_click(point(width - px(10.), px(5.)), Modifiers::default());
        assert_eq!(clicks.get(), 1);
    }

    #[gpui::test]
    fn a_disabled_or_busy_control_ignores_clicks_on_its_border(cx: &mut TestAppContext) {
        let clicks = Rc::new(Cell::new(0));
        let blocked = Rc::new(Cell::new(0));
        let cx = mount(cx, {
            let (clicks, blocked) = (clicks.clone(), blocked.clone());
            move |window, cx| {
                let control = |id: &'static str, left: f32| {
                    let clicks = clicks.clone();
                    div()
                        .id(id)
                        .absolute()
                        .left(gpui::px(left))
                        .top(gpui::px(20.))
                        .size(gpui::px(40.))
                        .border_2()
                        .on_click(move |_, _, _| clicks.set(clicks.get() + 1))
                };
                let control = match blocked.get() {
                    0 => control("free", 20.),
                    1 => disable(control("disabled", 20.)),
                    _ => busy(control("busy", 20.), window, cx),
                };
                div().size_full().child(control).into_any_element()
            }
        });
        let click = |x: f32, cx: &mut gpui::VisualTestContext| {
            cx.simulate_click(point(gpui::px(x), gpui::px(40.)), Modifiers::default());
            clicks.replace(0)
        };
        assert_eq!(
            [click(20.5, cx), click(40., cx), click(59.5, cx)],
            [1, 1, 1]
        );
        for state in [1, 2] {
            blocked.set(state);
            cx.update(|window, _| window.refresh());
            cx.run_until_parked();
            let seen = [click(20.5, cx), click(40., cx), click(59.5, cx)];
            assert_eq!(seen, [0, 0, 0], "state {state}");
        }
    }

    #[gpui::test]
    fn disabled_controls_ignore_clicks_and_keys(cx: &mut TestAppContext) {
        let clicks = Rc::new(RefCell::new(Vec::new()));
        let blocked = Rc::new(Cell::new(false));
        let (log, later) = (clicks.clone(), blocked.clone());
        let cx = mount(cx, move |window, cx| {
            let (first, second) = (log.clone(), log.clone());
            let off = icon_button("off", FOLDER, window, cx);
            let on = icon_button("on", FOLDER, window, cx);
            div()
                .flex()
                .child(disable(
                    off.on_click(move |_, _, _| first.borrow_mut().push(0)),
                ))
                .child(
                    on.on_click(move |_, _, _| second.borrow_mut().push(1))
                        .when(later.get(), disable),
                )
                .into_any_element()
        });
        cx.simulate_click(point(px(12.), px(12.)), Modifiers::default());
        assert!(clicks.borrow().is_empty());
        cx.simulate_click(point(px(36.), px(12.)), Modifiers::default());
        press(cx, "tab enter");
        assert_eq!(*clicks.borrow(), [1, 1]);

        blocked.set(true);
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        press(cx, "enter space");
        assert_eq!(*clicks.borrow(), [1, 1]);
    }
}
