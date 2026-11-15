use std::time::Duration;

use crate::ui::px;
use gpui::{
    Animation, AnimationExt, AnyView, App, AppContext, AvailableSpace, ClipboardItem, Context, Div,
    Entity, Global, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, deferred, div, size,
};

use crate::ui::kit::{TOAST, focus_ring};
use crate::ui::motion::{self, EASE_OUT_CUBIC, SLOW};
use crate::ui::theme::{BORDER, FONT_MONO, SURFACE_2, TEXT, TEXT_UI, Typeface};

const LIMIT: usize = 3;
const VISIBLE: Duration = Duration::from_secs(4);
const TAB_FIRST: isize = -1;
const EDGE: f32 = 20.;
const WIDTH: f32 = 360.;
const GAP: f32 = 8.;

struct Entry {
    id: usize,
    text: SharedString,
    key: Option<&'static str>,
    leaving: bool,
}

#[derive(Default)]
struct Toasts {
    entries: Vec<Entry>,
    next: usize,
}

struct Handle(Entity<Toasts>);

impl Global for Handle {}

pub fn init(cx: &mut App) {
    let toasts = cx.new(|_| Toasts::default());
    cx.set_global(Handle(toasts));
}

fn handle(cx: &App) -> Entity<Toasts> {
    cx.global::<Handle>().0.clone()
}

pub fn stack(cx: &App) -> AnyView {
    handle(cx).into()
}

pub fn show(text: impl Into<SharedString>, key: Option<&'static str>, cx: &mut App) {
    let text = text.into();
    handle(cx).update(cx, |toasts, cx| toasts.show(text, key, cx));
}

pub fn dismiss(key: &'static str, cx: &mut App) {
    handle(cx).update(cx, |toasts, cx| {
        toasts.dismiss(|entry| entry.key == Some(key), cx)
    });
}

impl Toasts {
    fn show(&mut self, text: SharedString, key: Option<&'static str>, cx: &mut Context<Self>) {
        if key.is_some() {
            self.entries.retain(|entry| entry.key != key);
        }
        if self.entries.iter().filter(|entry| !entry.leaving).count() >= LIMIT {
            self.dismiss(|_| true, cx);
        }
        let id = self.next;
        self.next += 1;
        self.entries.push(Entry {
            id,
            text,
            key,
            leaving: false,
        });
        Self::after(VISIBLE, cx, move |toasts, cx| {
            toasts.dismiss(|entry| entry.id == id, cx)
        });
        cx.notify();
    }

    fn dismiss(&mut self, matches: impl Fn(&Entry) -> bool, cx: &mut Context<Self>) {
        let mut entries = self.entries.iter_mut();
        let Some(entry) = entries.find(|entry| !entry.leaving && matches(entry)) else {
            return;
        };
        entry.leaving = true;
        let id = entry.id;
        Self::after(SLOW, cx, move |toasts, cx| {
            toasts.entries.retain(|entry| entry.id != id);
            cx.notify();
        });
        cx.notify();
    }

    fn after(
        delay: Duration,
        cx: &mut Context<Self>,
        act: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        cx.spawn(async move |toasts, cx| {
            cx.background_executor().timer(delay).await;
            toasts.update(cx, act).ok();
        })
        .detach();
    }
}

fn bubble(text: SharedString) -> Div {
    div()
        .max_w(px(WIDTH))
        .rounded(px(6.))
        .border_1()
        .border_color(BORDER.solid())
        .bg(SURFACE_2.solid())
        .p(px(10.))
        .face(FONT_MONO, TEXT_UI)
        .text_color(TEXT.solid())
        .child(text)
}

impl Render for Toasts {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let unbounded = size(AvailableSpace::MaxContent, AvailableSpace::MaxContent);
        let entrance = || Animation::new(SLOW).with_easing(|time| EASE_OUT_CUBIC.at(time));
        let mut column = div()
            .absolute()
            .right(px(EDGE))
            .bottom(px(EDGE))
            .flex()
            .flex_col()
            .items_end();
        for entry in &self.entries {
            let (id, text) = (entry.id, entry.text.clone());
            let mut probe = bubble(text.clone()).into_any_element();
            let full = px(GAP) + probe.layout_as_root(unbounded, window, cx).height;
            let target = if entry.leaving { 0. } else { 1. };
            let stay =
                motion::transition(("toast-stay", id), target, SLOW, EASE_OUT_CUBIC, window, cx);
            let toast = bubble(text.clone())
                .id("toast")
                .mt(px(GAP))
                .block_mouse_except_scroll()
                .cursor_pointer()
                .tab_index(TAB_FIRST)
                .focus_visible(focus_ring)
                .on_any_mouse_down(|_, window, _| window.prevent_default())
                .on_click(cx.listener(move |toasts, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                    toasts.dismiss(|entry| entry.id == id, cx);
                }));
            column = column.child(div().overflow_hidden().child(toast).with_animation(
                ("toast", id),
                entrance(),
                move |row, enter| {
                    let shown = enter * stay;
                    row.h(full * shown).opacity(shown)
                },
            ));
        }
        deferred(column).with_priority(TOAST)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use gpui::{Modifiers, TestAppContext, VisualTestContext, point};

    use super::*;
    use crate::ui::button::secondary;
    use crate::ui::modal::{Modal, picker_row};
    use crate::ui::testing::{mount, press};

    fn open(cx: &mut TestAppContext) -> &mut VisualTestContext {
        mount(cx, |_, cx| stack(cx).into_any_element())
    }

    fn listed(cx: &mut VisualTestContext) -> Vec<(String, bool)> {
        cx.update(|_, cx| {
            let entry = |entry: &Entry| (entry.text.to_string(), entry.leaving);
            handle(cx).read(cx).entries.iter().map(entry).collect()
        })
    }

    fn staying(texts: &[&str]) -> Vec<(String, bool)> {
        texts.iter().map(|text| (text.to_string(), false)).collect()
    }

    fn pass(cx: &mut VisualTestContext, time: Duration) {
        cx.executor().advance_clock(time);
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
        });
    }

    fn click_lowest(cx: &mut VisualTestContext) {
        let corner = cx.update(|window, _| window.viewport_size());
        let inset = px(EDGE + 5.);
        cx.simulate_click(
            point(corner.width - inset, corner.height - inset),
            Modifiers::default(),
        );
    }

    fn copied(cx: &mut VisualTestContext) -> Option<String> {
        cx.read_from_clipboard().and_then(|item| item.text())
    }

    #[gpui::test]
    fn a_toast_stays_four_seconds_and_leaves_within_200_ms(cx: &mut TestAppContext) {
        let cx = open(cx);
        cx.update(|_, cx| show("Stop R6S first", None, cx));
        pass(cx, Duration::from_millis(3999));
        assert_eq!(listed(cx), staying(&["Stop R6S first"]));
        pass(cx, Duration::from_millis(1));
        assert_eq!(listed(cx), [("Stop R6S first".to_owned(), true)]);
        pass(cx, Duration::from_millis(199));
        assert_eq!(listed(cx).len(), 1);
        pass(cx, Duration::from_millis(1));
        assert!(listed(cx).is_empty());
    }

    #[gpui::test]
    fn a_fourth_toast_sends_the_oldest_away(cx: &mut TestAppContext) {
        let cx = open(cx);
        for text in ["one", "two", "three", "four"] {
            cx.update(|_, cx| show(text, None, cx));
        }
        let mut expected = staying(&["one", "two", "three", "four"]);
        expected[0].1 = true;
        assert_eq!(listed(cx), expected);
        cx.update(|_, cx| show("five", None, cx));
        pass(cx, SLOW);
        assert_eq!(listed(cx), staying(&["three", "four", "five"]));
    }

    #[gpui::test]
    fn a_key_replaces_the_earlier_toast_and_dismisses_it(cx: &mut TestAppContext) {
        let cx = open(cx);
        cx.update(|_, cx| {
            show("first check", Some("update"), cx);
            show("other", None, cx);
            show("second check", Some("update"), cx);
        });
        assert_eq!(listed(cx), staying(&["other", "second check"]));
        cx.update(|_, cx| {
            dismiss("update", cx);
            dismiss("update", cx);
            dismiss("missing", cx);
        });
        let mut expected = staying(&["other", "second check"]);
        expected[1].1 = true;
        assert_eq!(listed(cx), expected);
        pass(cx, SLOW);
        assert_eq!(listed(cx), staying(&["other"]));
    }

    #[gpui::test]
    fn a_click_or_the_enter_key_copies_the_text_and_dismisses(cx: &mut TestAppContext) {
        let cx = open(cx);
        cx.update(|_, cx| show("Stop R6S first", None, cx));
        pass(cx, SLOW);
        click_lowest(cx);
        assert_eq!(copied(cx).as_deref(), Some("Stop R6S first"));
        assert_eq!(listed(cx), [("Stop R6S first".to_owned(), true)]);
        pass(cx, SLOW);
        cx.update(|_, cx| show("Cache cleared", None, cx));
        pass(cx, SLOW);
        press(cx, "tab enter");
        assert_eq!(copied(cx).as_deref(), Some("Cache cleared"));
        assert_eq!(listed(cx), [("Cache cleared".to_owned(), true)]);
    }

    #[gpui::test]
    fn a_click_on_a_toast_stays_away_from_what_lies_under_it(cx: &mut TestAppContext) {
        let under = Rc::new(Cell::new(0));
        let clicks = under.clone();
        let cx = mount(cx, move |_, cx| {
            let clicks = clicks.clone();
            let ground = div()
                .id("ground")
                .size_full()
                .on_click(move |_, _, _| clicks.set(clicks.get() + 1));
            div()
                .size_full()
                .child(ground)
                .child(stack(cx))
                .into_any_element()
        });
        click_lowest(cx);
        assert_eq!(under.get(), 1);
        cx.update(|_, cx| show("Stop R6S first", None, cx));
        pass(cx, SLOW);
        click_lowest(cx);
        assert_eq!(under.get(), 1);
        assert_eq!(copied(cx).as_deref(), Some("Stop R6S first"));
    }

    #[gpui::test]
    fn a_toast_over_an_open_modal_leaves_the_modal_and_its_focus_alone(cx: &mut TestAppContext) {
        let modal = Rc::new(RefCell::new(cx.update(Modal::new)));
        let log = Rc::new(RefCell::new(Vec::new()));
        let (shown, events) = (modal.clone(), log.clone());
        let cx = mount(cx, move |window, cx| {
            let note = |entry: &'static str| {
                let events = events.clone();
                move || events.borrow_mut().push(entry)
            };
            let (behind, closed) = (note("behind"), note("closed"));
            let rows = div().children(["first", "second"].map(|row| {
                let picked = note(row);
                picker_row(row, "Row", false, window, cx).on_click(move |_, _, _| picked())
            }));
            let behind =
                secondary("behind", "Behind", window, cx).on_click(move |_, _, _| behind());
            let modal = shown.borrow();
            let sheet = modal.sheet(rows).title("Probe");
            let modal = sheet.on_close(move |_, _| closed()).show(window, cx);
            div()
                .size_full()
                .child(behind)
                .child(modal)
                .child(stack(cx))
                .into_any_element()
        });
        cx.update(|window, cx| {
            modal.borrow_mut().open(window, cx);
            show("Stop R6S first", None, cx);
        });
        pass(cx, SLOW);
        press(cx, "shift-tab enter tab enter");
        click_lowest(cx);
        assert_eq!(copied(cx).as_deref(), Some("Stop R6S first"));
        pass(cx, SLOW);
        press(cx, "tab enter");
        assert_eq!(*log.borrow(), ["second", "first", "second"]);
    }

    #[gpui::test]
    fn a_leaving_toast_collapses_and_the_others_close_the_gap(cx: &mut TestAppContext) {
        let cx = open(cx);
        cx.update(|_, cx| {
            show("upper", None, cx);
            show("lower", None, cx);
        });
        pass(cx, SLOW);
        click_lowest(cx);
        cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
        click_lowest(cx);
        assert_eq!(copied(cx).as_deref(), Some("lower"));
        pass(cx, Duration::from_millis(150));
        click_lowest(cx);
        assert_eq!(copied(cx).as_deref(), Some("upper"));
    }

    #[gpui::test]
    fn toasts_grow_in_and_collapse_unless_motion_is_reduced(cx: &mut TestAppContext) {
        let cx = open(cx);
        cx.update(|_, cx| show("one", None, cx));
        click_lowest(cx);
        assert_eq!(copied(cx), None);
        cx.update(|_, cx| {
            cx.set_reduce_motion(true);
            show("two", None, cx);
        });
        click_lowest(cx);
        assert_eq!(copied(cx).as_deref(), Some("two"));
        click_lowest(cx);
        assert_eq!(copied(cx).as_deref(), Some("one"));
    }
}
