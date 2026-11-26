use std::collections::VecDeque;

use crate::ui::px;
use gpui::{
    App, DispatchPhase, Div, ElementId, HitboxBehavior, InteractiveElement, ParentElement, Pixels,
    ScrollEvent, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Window, canvas,
    div, point,
};

use crate::ui::motion::sub;
use crate::ui::selectable::{autoscroll, selectable, selection, selection_area};
use crate::ui::theme::{BORDER, FONT_MONO, TEXT_LABEL, TEXT_MUTED, Typeface, WELL};

const CAP: usize = 1000;
const NEAR_END: f32 = 4.;

#[derive(Default)]
pub struct Log {
    lines: VecDeque<SharedString>,
    first: u64,
}

impl Log {
    pub fn push(&mut self, payload: &str) {
        self.lines
            .extend(payload.split('\n').map(SharedString::new));
        let surplus = self.lines.len().saturating_sub(CAP);
        self.lines.drain(..surplus);
        self.first += surplus as u64;
    }

    pub fn follow(&mut self, history: &VecDeque<String>) {
        let most = self.lines.len().min(history.len());
        let kept = (0..=most).rev().find(|kept| {
            let tail = self.lines.iter().skip(self.lines.len() - kept);
            tail.eq(history.iter().take(*kept))
        });
        let kept = kept.unwrap_or_default();
        if kept < self.lines.len() && history.len() < CAP {
            self.first += self.lines.len() as u64;
            self.lines.clear();
            self.lines.extend(history.iter().map(SharedString::new));
        } else {
            for line in history.iter().skip(kept) {
                self.push(line);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    fn end(&self) -> u64 {
        self.first + self.lines.len() as u64
    }
}

#[derive(Default)]
struct Tail {
    scroll: ScrollHandle,
    away: bool,
    settled: Pixels,
    pinned: Pixels,
    seen: u64,
}

impl Tail {
    fn follows(&mut self, end: u64) -> bool {
        let (offset, limit) = (self.scroll.offset().y, self.scroll.max_offset().y);
        if limit <= px(0.) {
            self.away = false;
        } else if offset != self.settled {
            self.away = offset != self.pinned && limit + offset >= px(NEAR_END);
        }
        std::mem::replace(&mut self.seen, end) != end && !self.away
    }
}

pub fn log_box(id: impl Into<ElementId>, log: &Log, window: &mut Window, cx: &mut App) -> Div {
    let id = id.into();
    let owner = selection(sub(&id, "selection"), window, cx);
    let tail = window.use_keyed_state(id.clone(), cx, |_, _| Tail::default());
    let scroll = tail.read(cx).scroll.clone();
    autoscroll(&owner, &scroll, window, cx);
    let jump = tail.update(cx, |tail, _| tail.follows(log.end()));
    if jump {
        scroll.scroll_to_bottom();
    }
    let lines = log
        .lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !line.is_empty())
        .map(|(index, line)| selectable(&owner, log.first + index as u64, line.clone()));
    let lines = div()
        .on_children_prepainted(move |_, _, cx| {
            tail.update(cx, |tail, _| {
                tail.settled = tail.scroll.offset().y;
                if jump {
                    tail.pinned = tail.settled;
                }
            })
        })
        .id(id)
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .p(px(12.))
        .children(lines);
    let (placed, moved) = (scroll.clone(), scroll);
    let wheel = canvas(
        move |_, window, _| window.insert_hitbox(placed.bounds(), HitboxBehavior::Normal),
        move |_, hitbox, window, _| {
            let (view, line) = (window.current_view(), window.line_height());
            window.on_mouse_event(move |event: &ScrollEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || !hitbox.should_handle_scroll(window) {
                    return;
                }
                let offset = moved.offset();
                let top = (offset.y + event.delta.pixel_delta(line).y)
                    .clamp(-moved.max_offset().y, px(0.));
                if top != offset.y {
                    moved.set_offset(point(offset.x, top));
                    cx.notify(view);
                    cx.stop_propagation();
                }
            });
        },
    );
    let frame = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(96.))
        .rounded(px(8.))
        .border_1()
        .border_color(BORDER.solid())
        .bg(WELL.solid())
        .face(FONT_MONO, TEXT_LABEL)
        .line_height(px(1.5 * TEXT_LABEL))
        .text_color(TEXT_MUTED.solid());
    selection_area(&owner, frame, cx)
        .child(lines)
        .child(wheel.absolute().inset_0())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{
        IntoElement, Modifiers, MouseButton, MouseDownEvent, ScrollDelta, TestAppContext,
        VisualTestContext,
    };

    use super::*;
    use crate::ui::scroll::scroll;
    use crate::ui::selectable::PUSH_PACE;
    use crate::ui::testing::mount;

    fn probe<'a>(
        log: &Rc<RefCell<Log>>,
        cx: &'a mut TestAppContext,
    ) -> (ScrollHandle, &'a mut VisualTestContext) {
        let (shown, stash) = (log.clone(), Rc::new(RefCell::new(None)));
        let keep = stash.clone();
        let cx = mount(cx, move |window, cx| {
            let element = log_box("log", &shown.borrow(), window, cx);
            let tail = window.use_keyed_state("log", cx, |_, _| -> Tail { unreachable!() });
            *keep.borrow_mut() = Some(tail.read(cx).scroll.clone());
            div()
                .flex()
                .flex_col()
                .w(px(200.))
                .h(px(100.))
                .child(element)
                .into_any_element()
        });
        let scroll = stash.borrow_mut().take().expect("scroll handle");
        (scroll, cx)
    }

    fn feed(log: &Rc<RefCell<Log>>, payload: &str, cx: &mut VisualTestContext) {
        log.borrow_mut().push(payload);
        cx.update(|window, _| window.refresh());
    }

    fn wheel(cx: &mut VisualTestContext, up: f32) {
        cx.simulate_event(ScrollEvent {
            position: point(px(50.), px(50.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(up))),
            ..Default::default()
        });
    }

    #[test]
    fn log_keeps_the_newest_lines_and_counts_the_dropped_ones() {
        let mut log = Log::default();
        log.push("a\nb");
        assert_eq!(log.lines, ["a", "b"]);
        for _ in 0..CAP {
            log.push("x");
        }
        assert_eq!((log.lines.len(), log.first, log.end()), (CAP, 2, 1002));
        log.follow(&VecDeque::from(["c".to_owned(), "d".to_owned()]));
        assert_eq!((log.lines.len(), log.first), (2, 1002));
        log.follow(&VecDeque::new());
        assert!(log.is_empty());
        assert_eq!(log.end(), 1004);
    }

    #[gpui::test]
    fn view_follows_the_end_until_the_user_scrolls_away(cx: &mut TestAppContext) {
        let log = Rc::new(RefCell::new(Log::default()));
        log.borrow_mut().push("0\n1\n2\n3\n4\n5\n6\n7\n8\n9");
        let (scroll, cx) = probe(&log, cx);
        let state = |scroll: &ScrollHandle| {
            (
                f32::from(scroll.offset().y),
                f32::from(scroll.max_offset().y),
            )
        };
        assert_eq!(state(&scroll), (-106., 106.));
        feed(&log, "10", cx);
        assert_eq!(state(&scroll), (-124., 124.));
        wheel(cx, 50.);
        feed(&log, "11", cx);
        assert_eq!(state(&scroll), (-74., 142.));
        wheel(cx, -66.);
        assert_eq!(state(&scroll), (-140., 142.));
        feed(&log, "12", cx);
        assert_eq!(state(&scroll), (-160., 160.));
        wheel(cx, 3.);
        feed(&log, "13", cx);
        assert_eq!(state(&scroll), (-178., 178.));
        wheel(cx, 4.);
        feed(&log, "14", cx);
        assert_eq!(state(&scroll), (-174., 196.));
    }

    #[gpui::test]
    fn view_follows_again_at_the_place_of_its_last_jump(cx: &mut TestAppContext) {
        let log = Rc::new(RefCell::new(Log::default()));
        log.borrow_mut().push("0\n1\n2\n3\n4\n5\n6\n7\n8\n9");
        let (scroll, cx) = probe(&log, cx);
        let state = |scroll: &ScrollHandle| {
            (
                f32::from(scroll.offset().y),
                f32::from(scroll.max_offset().y),
            )
        };
        wheel(cx, 50.);
        feed(&log, "10", cx);
        assert_eq!(state(&scroll), (-56., 124.));
        wheel(cx, -50.);
        assert_eq!(state(&scroll), (-106., 124.));
        feed(&log, "11", cx);
        assert_eq!(state(&scroll), (-142., 142.));
        wheel(cx, 50.);
        feed(&log, "12", cx);
        wheel(cx, -14.);
        assert_eq!(state(&scroll), (-106., 160.));
        feed(&log, "13", cx);
        assert_eq!(state(&scroll), (-106., 178.));
    }

    #[gpui::test]
    fn the_wheel_moves_the_log_before_the_page_around_it(cx: &mut TestAppContext) {
        let log = Rc::new(RefCell::new(Log::default()));
        log.borrow_mut().push("0\n1\n2\n3\n4\n5\n6\n7\n8\n9");
        let stash = Rc::new(RefCell::new(None));
        let keep = stash.clone();
        let cx = mount(cx, move |window, cx| {
            let page = scroll("page", window, cx);
            let element = log_box("log", &log.borrow(), window, cx);
            let tail = window.use_keyed_state("log", cx, |_, _| -> Tail { unreachable!() });
            *keep.borrow_mut() = Some((tail.read(cx).scroll.clone(), page.handle().clone()));
            let boxed = div().flex().flex_col().h(px(100.)).child(element);
            page.area(
                div()
                    .w(px(200.))
                    .h(px(150.))
                    .child(div().h(px(300.)).child(boxed)),
            )
            .into_any_element()
        });
        let (inner, outer) = stash.borrow_mut().take().expect("scroll handles");
        let tops = || (f32::from(inner.offset().y), f32::from(outer.offset().y));
        assert_eq!(tops(), (-106., 0.));
        wheel(cx, 50.);
        assert_eq!(tops(), (-56., 0.));
        wheel(cx, -30.);
        assert_eq!(tops(), (-86., 0.));
        wheel(cx, -30.);
        assert_eq!(tops(), (-106., 0.));
        wheel(cx, -30.);
        assert_eq!(tops(), (-106., -30.));
        wheel(cx, 50.);
        assert_eq!(tops(), (-56., -30.));
    }

    #[gpui::test]
    fn a_log_that_fits_again_follows_again(cx: &mut TestAppContext) {
        let log = Rc::new(RefCell::new(Log::default()));
        log.borrow_mut().push("0\n1\n2\n3\n4\n5\n6\n7\n8\n9");
        let (scroll, cx) = probe(&log, cx);
        wheel(cx, 60.);
        log.borrow_mut()
            .follow(&VecDeque::from(["only".to_owned()]));
        cx.update(|window, _| window.refresh());
        assert_eq!(scroll.max_offset().y, px(0.));
        feed(&log, "0\n1\n2\n3\n4\n5\n6\n7\n8\n9", cx);
        assert_eq!(scroll.offset().y, px(-124.));
    }

    #[gpui::test]
    fn empty_lines_take_no_room_and_are_not_copied(cx: &mut TestAppContext) {
        let log = Rc::new(RefCell::new(Log::default()));
        log.borrow_mut().push("above\n\nbelow");
        let (_, cx) = probe(&log, cx);
        cx.simulate_event(MouseDownEvent {
            position: point(px(30.), px(40.)),
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 3,
            first_mouse: false,
        });
        cx.simulate_keystrokes("ctrl-c");
        let copied = cx.read_from_clipboard().and_then(|item| item.text());
        assert_eq!(copied.as_deref(), Some("below"));
        cx.simulate_keystrokes("ctrl-a ctrl-c");
        let copied = cx.read_from_clipboard().and_then(|item| item.text());
        assert_eq!(copied.as_deref(), Some("above\nbelow"));
    }

    #[gpui::test]
    fn dragging_past_the_edge_scrolls_and_extends_the_selection(cx: &mut TestAppContext) {
        let log = Rc::new(RefCell::new(Log::default()));
        for number in 0..30 {
            log.borrow_mut().push(&format!("line {number}"));
        }
        let (scroll, cx) = probe(&log, cx);
        assert_eq!(scroll.offset().y, px(-466.));
        cx.simulate_mouse_down(
            point(px(30.), px(50.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_move(
            point(px(30.), px(-19.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        assert_eq!(scroll.offset().y, px(-446.));
        for _ in 0..3 {
            cx.executor().advance_clock(PUSH_PACE);
            cx.update(|window, cx| window.simulate_next_frame(cx));
        }
        assert_eq!(scroll.offset().y, px(-386.));
        cx.simulate_mouse_up(
            point(px(30.), px(-19.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.update(|window, cx| window.simulate_next_frame(cx));
        assert_eq!(scroll.offset().y, px(-386.));
        cx.simulate_keystrokes("ctrl-c");
        let copied = cx.read_from_clipboard().and_then(|item| item.text());
        assert_eq!(
            copied.as_deref(),
            Some("ne 21\nline 22\nline 23\nline 24\nline 25\nline 26\nli")
        );
    }

    #[gpui::test]
    fn leaving_the_box_in_small_steps_starts_the_scrolling(cx: &mut TestAppContext) {
        let log = Rc::new(RefCell::new(Log::default()));
        for number in 0..30 {
            log.borrow_mut().push(&format!("line {number}"));
        }
        let (scroll, cx) = probe(&log, cx);
        cx.simulate_mouse_down(
            point(px(30.), px(50.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        for y in [40., 30., 20., 10., 4., 2., 1.] {
            cx.simulate_mouse_move(
                point(px(30.), px(y)),
                MouseButton::Left,
                Modifiers::default(),
            );
        }
        assert_eq!(scroll.offset().y, px(-466.));
        cx.simulate_mouse_move(
            point(px(30.), px(-4.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        assert_eq!(scroll.offset().y, px(-461.));
        for _ in 0..3 {
            cx.executor().advance_clock(PUSH_PACE);
            cx.update(|window, cx| window.simulate_next_frame(cx));
        }
        assert_eq!(scroll.offset().y, px(-446.));
    }

    #[gpui::test]
    fn selection_stays_on_its_line_when_the_cap_drops_the_oldest(cx: &mut TestAppContext) {
        let log = Rc::new(RefCell::new(Log::default()));
        for number in 0..CAP {
            log.borrow_mut().push(&format!("line {number}"));
        }
        let (_, cx) = probe(&log, cx);
        cx.simulate_event(MouseDownEvent {
            position: point(px(30.), px(78.)),
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 3,
            first_mouse: false,
        });
        cx.simulate_mouse_up(
            point(px(30.), px(78.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        let copied = |cx: &mut VisualTestContext| {
            cx.simulate_keystrokes("ctrl-c");
            cx.read_from_clipboard().and_then(|item| item.text())
        };
        assert_eq!(copied(cx).as_deref(), Some("line 999"));
        feed(&log, "line 1000", cx);
        assert_eq!(log.borrow().first, 1);
        assert_eq!(copied(cx).as_deref(), Some("line 999"));
    }
}
