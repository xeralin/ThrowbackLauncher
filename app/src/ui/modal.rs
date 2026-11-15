use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use crate::ui::px;
use gpui::{
    AnyElement, App, Bounds, Deferred, Div, ElementId, FocusHandle, Global, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Pixels, ScrollHandle, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, anchored, canvas, deferred, div, fill,
    linear_color_stop, linear_gradient, point, prelude::FluentBuilder, size,
};

use crate::ui::button::{primary, secondary};
use crate::ui::frame;
use crate::ui::input::{Commit, Revert};
use crate::ui::kit::{OVERLAY, busy, disable, focus_ring, list_row, panel, ring, truncate};
use crate::ui::motion::{self, EASE_OUT, FAST, RISE};
use crate::ui::scroll::{reveal, scroll};
use crate::ui::theme::{
    BLACK, BORDER, FONT_DISPLAY_BOLD, REM, SURFACE, TEXT, Theme, Typeface, shadow,
};
use crate::ui::{Dismiss, FocusNext, FocusPrevious};

const ENTRANCE: Duration = Duration::from_millis(180);
const CONTEXT: &str = "Modal";
const DIM: f32 = 0.6;
const WIDTH: f32 = 440.;
const WIDE: f32 = 760.;
const PAD: f32 = 24.;
const BODY_PAD: f32 = 20.;
const BODY_MAX: f32 = 25.5 * REM;
const FADE: f32 = 28.;
const TITLE_GAP: f32 = 12.;

pub type Handler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Default)]
struct Dimmed {
    sheets: Rc<Cell<usize>>,
    serial: usize,
}

impl Global for Dimmed {}

struct Lease(Rc<Cell<usize>>);

impl Drop for Lease {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

pub struct Modal {
    panel: FocusHandle,
    previous: Option<FocusHandle>,
    lease: Option<Lease>,
    opened: usize,
}

struct Confirmation {
    label: &'static str,
    busy: bool,
    confirm: Handler,
    cancel: Handler,
}

pub struct Sheet<'a> {
    modal: &'a Modal,
    title: Option<AnyElement>,
    body: AnyElement,
    footer: Vec<AnyElement>,
    aside: Option<AnyElement>,
    width: f32,
    close: Option<Handler>,
    confirm: Option<Handler>,
    confirmation: Option<Confirmation>,
    form: bool,
}

fn fades(list: ScrollHandle) -> impl IntoElement {
    let paint = move |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
        let (scrolled, reach) = (-list.offset().y, list.max_offset().y);
        let mut shade = |top: Pixels, from: f32, to: f32| {
            let strip = Bounds::new(point(bounds.left(), top), size(bounds.size.width, px(FADE)));
            let fade = linear_gradient(
                180.,
                linear_color_stop(SURFACE.alpha(from), 0.),
                linear_color_stop(SURFACE.alpha(to), 1.),
            );
            window.paint_quad(fill(strip, fade));
        };
        if scrolled > px(0.) {
            shade(bounds.top(), 1., 0.);
        }
        if scrolled < reach {
            shade(bounds.bottom() - px(FADE), 0., 1.);
        }
    };
    canvas(|_, _, _| (), paint).absolute().inset_0()
}

pub fn dim(window: &mut Window, cx: &mut App) -> f32 {
    let dimmed = cx.default_global::<Dimmed>();
    if dimmed.sheets.get() == 0 {
        return 0.;
    }
    let serial = dimmed.serial;
    DIM * motion::entrance(("frame-dim", serial), ENTRANCE, EASE_OUT, window, cx)
}

pub fn modal_title(content: impl IntoElement) -> Div {
    div()
        .flex_none()
        .face(FONT_DISPLAY_BOLD, 1.2 * REM)
        .text_color(TEXT.solid())
        .child(content)
}

impl Modal {
    pub fn new(cx: &mut App) -> Self {
        Self {
            panel: cx.focus_handle(),
            previous: None,
            lease: None,
            opened: 0,
        }
    }

    pub fn open(&mut self, window: &mut Window, cx: &mut App) {
        if self.lease.is_none() {
            let dimmed = cx.default_global::<Dimmed>();
            dimmed.serial += 1;
            dimmed.sheets.set(dimmed.sheets.get() + 1);
            self.lease = Some(Lease(dimmed.sheets.clone()));
            self.opened += 1;
            self.previous = window.focused(cx);
            window.focus(&self.panel, cx);
        }
    }

    pub fn close(&mut self, window: &mut Window, cx: &mut App) {
        if self.lease.take().is_some() {
            match self.previous.take() {
                Some(previous) => window.focus(&previous, cx),
                None => window.blur(cx),
            }
        }
    }

    pub fn is_open(&self) -> bool {
        self.lease.is_some()
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.panel, cx);
    }

    pub fn sheet(&self, body: impl IntoElement) -> Sheet<'_> {
        Sheet {
            modal: self,
            title: None,
            body: body.into_any_element(),
            footer: Vec::new(),
            aside: None,
            width: WIDTH,
            close: None,
            confirm: None,
            confirmation: None,
            form: false,
        }
    }
}

impl Sheet<'_> {
    pub fn title(mut self, title: impl IntoElement) -> Self {
        self.title = Some(title.into_any_element());
        self
    }

    pub fn footer(mut self, buttons: impl IntoIterator<Item = impl IntoElement>) -> Self {
        let buttons = buttons.into_iter().map(IntoElement::into_any_element);
        self.footer = buttons.collect();
        self
    }

    pub fn aside(mut self, aside: impl IntoElement) -> Self {
        self.aside = Some(aside.into_any_element());
        self
    }

    pub fn on_close(mut self, close: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.close = Some(Rc::new(close));
        self
    }

    pub fn on_confirm(mut self, confirm: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.confirm = Some(Rc::new(confirm));
        self
    }

    pub fn confirm(
        mut self,
        label: &'static str,
        busy: bool,
        on_confirm: impl Fn(&mut Window, &mut App) + 'static,
        on_cancel: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        let (confirm, cancel): (Handler, Handler) = (Rc::new(on_confirm), Rc::new(on_cancel));
        if !busy {
            self.close = Some(cancel.clone());
            self.confirm = Some(confirm.clone());
        }
        self.confirmation = Some(Confirmation {
            label,
            busy,
            confirm,
            cancel,
        });
        self
    }

    pub fn form(mut self) -> Self {
        self.form = true;
        self
    }

    pub fn wide(mut self) -> Self {
        self.width = WIDE;
        self
    }

    pub fn without_enter(mut self) -> Self {
        self.confirm = None;
        self
    }

    pub fn show(self, window: &mut Window, cx: &mut App) -> Deferred {
        let area = frame::area(window);
        let viewport = area.size;
        let list = scroll("modal-body", window, cx);
        let serial = self.modal.opened;
        let shown = motion::entrance(("modal-entrance", serial), ENTRANCE, EASE_OUT, window, cx);
        let focus = self.modal.panel.clone();
        let cycle = |forward: bool| {
            let panel = focus.clone();
            move |window: &mut Window, cx: &mut App| {
                let step = |window: &mut Window, cx: &mut App| {
                    if forward {
                        window.focus_next(cx)
                    } else {
                        window.focus_prev(cx)
                    }
                };
                step(window, cx);
                if panel.contains_focused(window, cx) {
                    return;
                }
                if forward {
                    window.focus(&panel, cx);
                } else {
                    window.blur(cx);
                }
                step(window, cx);
                if !panel.contains_focused(window, cx) {
                    window.focus(&panel, cx);
                }
            }
        };
        let (next, previous) = (cycle(true), cycle(false));
        let (escape, outside) = (self.close.clone(), self.close);
        let width = |wanted: f32| px(wanted).min(viewport.width - px(2. * REM));
        let room = viewport.height - px(3. * REM);
        let footer = match self.confirmation {
            Some(confirmation) => {
                let (confirm, cancel) = (confirmation.confirm, confirmation.cancel);
                let dismiss = secondary("modal-cancel", "Cancel", window, cx);
                let dismiss = dismiss.on_click(move |_, window, cx| cancel(window, cx));
                let accept = primary("modal-confirm", confirmation.label, window, cx);
                let (dismiss, accept) = if confirmation.busy {
                    (disable(dismiss), busy(accept, window, cx))
                } else {
                    let accept = accept.on_click(move |_, window, cx| confirm(window, cx));
                    (dismiss, accept)
                };
                vec![dismiss.into_any_element(), accept.into_any_element()]
            }
            None => self.footer,
        };
        let body = div()
            .flex()
            .flex_col()
            .min_h_0()
            .mx(px(-PAD))
            .my(px(-BODY_PAD))
            .child(
                list.area(
                    div()
                        .min_h_0()
                        .px(px(PAD))
                        .py(px(BODY_PAD))
                        .max_h(px(BODY_MAX))
                        .child(self.body),
                ),
            )
            .child(fades(list.handle().clone()));
        let frame = panel()
            .flex()
            .flex_col()
            .min_h_0()
            .w(width(self.width))
            .p(px(PAD))
            .child(
                div()
                    .flex()
                    .flex_col_reverse()
                    .min_h_0()
                    .child(body)
                    .children(self.title.map(|title| modal_title(title).mb(px(TITLE_GAP)))),
            )
            .when(!footer.is_empty(), |frame| {
                frame.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_end()
                        .gap(px(8.))
                        .mt(px(24.))
                        .children(footer),
                )
            });
        let aside = self
            .aside
            .map(|aside| panel().flex_none().w(width(WIDTH)).p(px(PAD)).child(aside));
        let stack = div()
            .track_focus(&focus)
            .occlude()
            .flex()
            .flex_col()
            .gap(px(16.))
            .max_h(room)
            .top(px(RISE * (1. - shown)))
            .opacity(shown)
            .child(frame)
            .children(aside);
        let enter = self.confirm;
        let (typed, reverted) = match self.form {
            true => (enter.clone(), escape.clone()),
            false => (None, None),
        };
        let overlay = div()
            .id("modal")
            .key_context(CONTEXT)
            .occlude()
            .w(viewport.width)
            .h(viewport.height)
            .flex()
            .items_center()
            .justify_center()
            .rounded(frame::veil(window))
            .bg(BLACK.alpha(DIM * shown))
            .on_any_mouse_down(move |_, window, cx| {
                if let Some(close) = &outside {
                    close(window, cx)
                }
            })
            .on_action(move |_: &FocusNext, window, cx| next(window, cx))
            .on_action(move |_: &FocusPrevious, window, cx| previous(window, cx))
            .on_action(move |_: &Dismiss, window, cx| {
                if let Some(close) = &escape {
                    close(window, cx)
                }
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let keystroke = &event.keystroke;
                if let Some(confirm) = &enter
                    && keystroke.key == "enter"
                    && !keystroke.modifiers.modified()
                    && focus.is_focused(window)
                {
                    confirm(window, cx)
                }
            })
            .when_some(reverted, |overlay, close| {
                overlay.capture_action(move |_: &Revert, window, cx| {
                    cx.stop_propagation();
                    close(window, cx);
                })
            })
            .when_some(typed, |overlay, confirm| {
                overlay.capture_action(move |_: &Commit, window, cx| {
                    cx.stop_propagation();
                    confirm(window, cx);
                })
            })
            .map(|overlay| list.keys(overlay))
            .child(stack);
        deferred(anchored().position(area.origin).child(overlay)).with_priority(OVERLAY)
    }
}

pub fn covering(window: &Window) -> bool {
    let path = window.context_stack();
    path.iter().any(|context| context.contains(CONTEXT))
}

pub fn picker_row(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let id = id.into();
    let hover = motion::hover(&id, FAST, window, cx);
    let theme = Theme::get(cx);
    let edge = BORDER.mix(theme.action_edge, if selected { 1. } else { hover.amount });
    let halo = shadow(0., 0., 18., 0., theme.action_glow.into());
    list_row()
        .id(id)
        .border_color(edge.solid())
        .tab_index(0)
        .on_any_mouse_down(|_, window, _| window.prevent_default())
        .map(|element| hover.follow(element))
        .when(selected, |row| {
            row.shadow(vec![halo.clone()])
                .focus_visible(|style| focus_ring(style).shadow(vec![halo, ring()]))
        })
        .when(!selected, |row| row.focus_visible(focus_ring))
        .map(reveal)
        .child(truncate(label))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{Modifiers, TestAppContext, VisualTestContext, point};

    use super::*;
    use crate::ui::button::secondary;
    use crate::ui::testing::{mount, press};

    #[gpui::test]
    fn the_outline_dims_while_a_sheet_is_open_even_if_its_owner_is_dropped(
        cx: &mut TestAppContext,
    ) {
        let seen = Rc::new(Cell::new(0.));
        let out = seen.clone();
        let cx = mount(cx, move |window, cx| {
            out.set(dim(window, cx));
            div().into_any_element()
        });
        let shown = |cx: &mut VisualTestContext| {
            cx.update(|window, cx| window.draw(cx).clear(cx));
            seen.get()
        };
        let (mut first, mut second) = cx.update(|_, cx| {
            cx.set_reduce_motion(true);
            (Modal::new(cx), Modal::new(cx))
        });
        assert_eq!(shown(cx), 0.);
        cx.update(|window, cx| {
            first.open(window, cx);
            second.open(window, cx);
        });
        assert_eq!(shown(cx), DIM);
        cx.update(|window, cx| first.close(window, cx));
        assert_eq!(shown(cx), DIM);
        drop(second);
        assert_eq!(shown(cx), 0.);
    }

    #[gpui::test]
    fn focus_stays_inside_and_returns_when_escape_or_the_backdrop_closes(cx: &mut TestAppContext) {
        let modal = Rc::new(RefCell::new(None::<Modal>));
        let picks = Rc::new(RefCell::new(Vec::new()));
        let (slot, log) = (modal.clone(), picks.clone());
        let cx = mount(cx, move |window, cx| {
            let (open, close) = (slot.clone(), slot.clone());
            let mut slot = slot.borrow_mut();
            let modal = slot.get_or_insert_with(|| Modal::new(cx));
            let rows = div().children((0..2usize).map(|index| {
                let log = log.clone();
                picker_row(("row", index), "Row", false, window, cx)
                    .on_click(move |_, _, _| log.borrow_mut().push(index))
            }));
            div()
                .child(
                    secondary("opener", "Open", window, cx).on_click(move |_, window, cx| {
                        if let Some(modal) = open.borrow_mut().as_mut() {
                            modal.open(window, cx);
                        }
                        window.refresh();
                    }),
                )
                .children(modal.is_open().then(|| {
                    let close = move |window: &mut Window, cx: &mut App| {
                        if let Some(modal) = close.borrow_mut().as_mut() {
                            modal.close(window, cx);
                        }
                        window.refresh();
                    };
                    let sheet = modal.sheet(rows).title("Probe");
                    sheet.on_close(close).show(window, cx)
                }))
                .into_any_element()
        });
        let is_open = || modal.borrow().as_ref().is_some_and(Modal::is_open);
        press(cx, "tab enter");
        assert!(is_open());
        press(cx, "tab tab enter");
        press(cx, "tab enter");
        press(cx, "shift-tab enter");
        assert_eq!(*picks.borrow(), [1, 0, 1]);
        press(cx, "escape");
        assert!(!is_open());
        press(cx, "enter");
        assert!(is_open());
        let viewport = cx.update(|window, _| window.viewport_size());
        let middle = point(viewport.width / 2., viewport.height / 2.);
        cx.simulate_click(middle, Modifiers::default());
        assert!(is_open());
        cx.simulate_click(point(px(1.), px(1.)), Modifiers::default());
        assert!(!is_open());
        press(cx, "enter");
        assert!(is_open());
    }

    #[gpui::test]
    fn a_dialog_without_controls_keeps_the_focus(cx: &mut TestAppContext) {
        let modal = Rc::new(RefCell::new(None::<Modal>));
        let opens = Rc::new(RefCell::new(0));
        let (slot, count) = (modal.clone(), opens.clone());
        let cx = mount(cx, move |window, cx| {
            let (open, close, count) = (slot.clone(), slot.clone(), count.clone());
            let mut slot = slot.borrow_mut();
            let modal = slot.get_or_insert_with(|| Modal::new(cx));
            div()
                .child(
                    secondary("opener", "Open", window, cx).on_click(move |_, window, cx| {
                        *count.borrow_mut() += 1;
                        if let Some(modal) = open.borrow_mut().as_mut() {
                            modal.open(window, cx);
                        }
                        window.refresh();
                    }),
                )
                .children(modal.is_open().then(|| {
                    let close = move |window: &mut Window, cx: &mut App| {
                        if let Some(modal) = close.borrow_mut().as_mut() {
                            modal.close(window, cx);
                        }
                        window.refresh();
                    };
                    let sheet = modal.sheet(div().child("Nothing to press")).title("Probe");
                    sheet.on_close(close).show(window, cx)
                }))
                .into_any_element()
        });
        let is_open = || modal.borrow().as_ref().is_some_and(Modal::is_open);
        press(cx, "tab enter");
        assert!(is_open());
        press(cx, "tab enter");
        press(cx, "shift-tab enter");
        assert_eq!(*opens.borrow(), 1);
        press(cx, "escape");
        assert!(!is_open());
    }
}
