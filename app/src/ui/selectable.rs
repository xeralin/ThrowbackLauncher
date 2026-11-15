use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ops::Range;
use std::time::{Duration, Instant};

use crate::ui::px;
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, DispatchPhase, Div, Element, ElementId,
    Entity, FocusHandle, Global, GlobalElementId, HighlightStyle, Hitbox, HitboxBehavior,
    InspectorElementId, InteractiveElement, IntoElement, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, ScrollHandle,
    SharedString, Styled, StyledText, TextLayout, WeakEntity, Window, canvas, point,
};

use crate::ui::input::{Copy, SelectAll};
use crate::ui::theme::{SELECTION, contrast};

pub const PUSH_PACE: Duration = Duration::from_micros(16_667);

struct Current(WeakEntity<Selection>);

impl Global for Current {}

fn current(cx: &App) -> Option<Entity<Selection>> {
    cx.try_global::<Current>()?.0.upgrade()
}

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-c", Copy, None),
        KeyBinding::new("ctrl-a", SelectAll, None),
    ]);
    cx.on_action(|_: &Copy, cx| {
        if let Some(current) = current(cx) {
            current.update(cx, |selection, cx| selection.copy(cx));
        }
    });
    cx.on_action(|_: &SelectAll, cx| {
        if let Some(current) = current(cx) {
            current.update(cx, |selection, cx| selection.select_all(cx));
        }
    });
}

type Spot = (u64, usize);

#[derive(Clone, Copy)]
enum Unit {
    Character,
    Word,
    Line,
}

struct Block {
    text: SharedString,
    layout: TextLayout,
}

pub struct Selection {
    focus: FocusHandle,
    blocks: BTreeMap<u64, Block>,
    picked: Option<(Range<Spot>, Range<Spot>)>,
    dragging: Option<Unit>,
    pushed: Option<Instant>,
}

impl Selection {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            blocks: BTreeMap::new(),
            picked: None,
            dragging: None,
            pushed: None,
        }
    }

    fn range(&self) -> Option<Range<Spot>> {
        let (origin, extent) = self.picked.as_ref()?;
        let range = origin.start.min(extent.start)..origin.end.max(extent.end);
        (range.start < range.end).then_some(range)
    }

    fn span(&self, key: u64, text: &str) -> Range<usize> {
        let Some(range) = self.range() else {
            return 0..0;
        };
        let offset = |spot: Spot, before: usize, after: usize| match spot.0.cmp(&key) {
            Ordering::Less => before,
            Ordering::Equal => spot.1,
            Ordering::Greater => after,
        };
        let start = offset(range.start, 0, text.len());
        let end = offset(range.end, 0, text.len());
        if start < end && text.is_char_boundary(start) && text.is_char_boundary(end) {
            start..end
        } else {
            0..0
        }
    }

    fn text(&self) -> String {
        let Some(range) = self.range() else {
            return String::new();
        };
        let parts: Vec<&str> = self
            .blocks
            .range(range.start.0..=range.end.0)
            .map(|(key, block)| &block.text[self.span(*key, &block.text)])
            .collect();
        parts.join("\n")
    }

    fn target(&self, position: Point<Pixels>, unit: Unit) -> Option<Range<Spot>> {
        let gap = |low: Pixels, high: Pixels, at: Pixels| (low - at).max(at - high).max(px(0.));
        let (key, block) = self.blocks.iter().min_by_key(|(_, block)| {
            let bounds = block.layout.bounds();
            (
                gap(bounds.top(), bounds.bottom(), position.y),
                gap(bounds.left(), bounds.right(), position.x),
            )
        })?;
        let index = index_at(block, position, matches!(unit, Unit::Character));
        let range = match unit {
            Unit::Character => index..index,
            Unit::Word => word(&block.text, index),
            Unit::Line => line(&block.text, index),
        };
        Some((*key, range.start)..(*key, range.end))
    }

    fn press(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let unit = match event.click_count {
            0 | 1 => Unit::Character,
            2 => Unit::Word,
            _ => Unit::Line,
        };
        let Some(target) = self.target(event.position, unit) else {
            return;
        };
        if let Some(other) = current(cx).filter(|other| *other != cx.entity()) {
            other.update(cx, |other, cx| {
                other.picked = None;
                cx.notify();
            });
        }
        let this = cx.weak_entity();
        cx.set_global(Current(this));
        self.picked = Some(match self.picked.take() {
            Some((origin, _)) if event.modifiers.shift => (origin, target),
            _ => (target.clone(), target),
        });
        self.dragging = Some(unit);
        cx.notify();
    }

    fn drag(&mut self, position: Point<Pixels>) {
        if let Some(unit) = self.dragging
            && let Some(target) = self.target(position, unit)
            && let Some((_, extent)) = &mut self.picked
        {
            *extent = target;
        }
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        if let Some((first, _)) = self.blocks.first_key_value()
            && let Some((last, block)) = self.blocks.last_key_value()
        {
            let all = (*first, 0)..(*last, block.text.len());
            self.picked = Some((all.clone(), all));
            cx.notify();
        }
    }

    fn copy(&self, cx: &mut Context<Self>) {
        let text = self.text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }
}

fn index_at(block: &Block, position: Point<Pixels>, closest: bool) -> usize {
    let (bounds, line_height) = (block.layout.bounds(), block.layout.line_height());
    let lines = block.layout.line_layouts();
    let (mut top, mut start) = (bounds.top(), 0);
    for (number, line) in lines.iter().enumerate() {
        let height = line.size(line_height).height;
        if position.y < top + height || number + 1 == lines.len() {
            let row = (position.y - top)
                .max(px(0.))
                .min(height - line_height / 2.);
            let local = point(position.x - bounds.left(), row);
            let found = if closest {
                line.closest_index_for_position(local, line_height)
            } else {
                line.index_for_position(local, line_height)
            };
            return match found {
                Err(end) if !closest && end > 0 && local.x > px(0.) => {
                    block.text.floor_char_boundary(start + end - 1)
                }
                Ok(index) | Err(index) => start + index,
            };
        }
        top += height;
        start += line.len() + 1;
    }
    0
}

fn word(text: &str, index: usize) -> Range<usize> {
    let letter = |character: char| character.is_alphanumeric() || character == '_';
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let class = |place: usize| {
        let (_, character) = chars[place];
        let beside = |other: Option<usize>| {
            other
                .and_then(|other| chars.get(other))
                .is_some_and(|(_, found)| letter(*found))
        };
        let joins = matches!(character, '.' | '\'')
            && beside(place.checked_sub(1))
            && beside(Some(place + 1));
        (letter(character) || joins, character.is_whitespace())
    };
    let place = chars.partition_point(|(start, _)| *start < index);
    if place == chars.len() {
        return index..index;
    }
    let (mut first, mut last) = (place, place);
    if class(place) != (false, false) {
        while first > 0 && class(first - 1) == class(place) {
            first -= 1;
        }
        while last + 1 < chars.len() && class(last + 1) == class(place) {
            last += 1;
        }
    }
    chars[first].0..chars[last].0 + chars[last].1.len_utf8()
}

fn line(text: &str, index: usize) -> Range<usize> {
    let start = text[..index].rfind('\n').map_or(0, |found| found + 1);
    let end = text[index..]
        .find('\n')
        .map_or(text.len(), |found| index + found);
    start..end
}

pub fn selection(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> Entity<Selection> {
    window.use_keyed_state(id, cx, |_, cx| Selection::new(cx))
}

pub fn selection_area(owner: &Entity<Selection>, container: Div, cx: &App) -> Div {
    let focus = owner.read(cx).focus.clone();
    let [press, reset, moved, released] = std::array::from_fn(|_| owner.clone());
    let sensor = canvas(
        move |_, _, cx| reset.update(cx, |selection, _| selection.blocks.clear()),
        move |bounds: Bounds<Pixels>, _, window, _| {
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble && moved.read(cx).dragging.is_some() {
                    let inside = point(
                        event.position.x.max(bounds.left()).min(bounds.right()),
                        event.position.y.max(bounds.top()).min(bounds.bottom()),
                    );
                    moved.update(cx, |selection, cx| {
                        selection.drag(inside);
                        cx.notify();
                    });
                }
            });
            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                    released.update(cx, |selection, _| selection.dragging = None);
                }
            });
        },
    );
    container
        .track_focus(&focus)
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            press.update(cx, |selection, cx| selection.press(event, cx))
        })
        .child(sensor.absolute().inset_0())
}

pub fn autoscroll(
    owner: &Entity<Selection>,
    scroll: &ScrollHandle,
    window: &mut Window,
    cx: &mut App,
) {
    let (pointer, view, offset) = (window.mouse_position(), scroll.bounds(), scroll.offset());
    let push = (view.top() - pointer.y).max(px(0.)) - (pointer.y - view.bottom()).max(px(0.));
    let selection = owner.read(cx);
    if selection.dragging.is_none() || push == px(0.) {
        if selection.pushed.is_some() {
            owner.update(cx, |selection, _| selection.pushed = None);
        }
        return;
    }
    let edge = point(pointer.x, pointer.y.max(view.top()).min(view.bottom()));
    let now = cx.background_executor().now();
    let since = owner.update(cx, |selection, _| {
        selection.drag(edge);
        selection.pushed.replace(now)
    });
    let frames = since.map_or(1., |since| {
        (now - since).as_secs_f32() / PUSH_PACE.as_secs_f32()
    });
    let target = (offset.y + push * frames)
        .max(-scroll.max_offset().y)
        .min(px(0.));
    if target != offset.y {
        scroll.set_offset(point(offset.x, target));
        window.request_animation_frame();
    }
}

pub struct SelectableText {
    owner: Entity<Selection>,
    key: u64,
    text: SharedString,
}

pub fn selectable(
    owner: &Entity<Selection>,
    key: u64,
    text: impl Into<SharedString>,
) -> SelectableText {
    SelectableText {
        owner: owner.clone(),
        key,
        text: text.into(),
    }
}

impl IntoElement for SelectableText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectableText {
    type RequestLayoutState = StyledText;
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let picked = self.owner.read(cx).span(self.key, &self.text);
        let lit = HighlightStyle {
            color: Some(contrast().solid()),
            background_color: Some(SELECTION.solid()),
            ..HighlightStyle::default()
        };
        let marks = (!picked.is_empty()).then_some((picked, lit));
        let mut styled =
            StyledText::new(self.text.clone()).with_default_highlights(&window.text_style(), marks);
        let (layout, ()) = styled.request_layout(None, inspector, window, cx);
        (layout, styled)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        styled: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        styled.prepaint(None, inspector, bounds, &mut (), window, cx);
        bounds
            .intersects(&window.content_mask().bounds)
            .then(|| window.insert_hitbox(bounds, HitboxBehavior::Normal))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        styled: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(hitbox) = hitbox {
            window.set_cursor_style(CursorStyle::IBeam, hitbox);
            styled.paint(None, inspector, bounds, &mut (), &mut (), window, cx);
        }
        let block = Block {
            text: self.text.clone(),
            layout: styled.layout().clone(),
        };
        self.owner.update(cx, |selection, _| {
            selection.blocks.insert(self.key, block);
        });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{Modifiers, StatefulInteractiveElement, TestAppContext, VisualTestContext, div};

    use super::*;
    use crate::ui::button::icon_button;
    use crate::ui::icon::FOLDER;
    use crate::ui::testing::mount;

    fn probe<'a>(
        width: f32,
        blocks: &'static [&'static str],
        cx: &'a mut TestAppContext,
    ) -> (Entity<Selection>, &'a mut VisualTestContext) {
        let stash = Rc::new(RefCell::new(None));
        let keep = stash.clone();
        let cx = mount(cx, move |window, cx| {
            let owner = selection("probe", window, cx);
            *keep.borrow_mut() = Some(owner.clone());
            let frame = div().w(px(width)).text_size(px(10.)).line_height(px(20.));
            selection_area(&owner, frame, cx)
                .children(
                    blocks
                        .iter()
                        .enumerate()
                        .map(|(key, text)| selectable(&owner, key as u64, *text)),
                )
                .into_any_element()
        });
        let owner = stash.borrow_mut().take().expect("owner");
        (owner, cx)
    }

    fn press(cx: &mut VisualTestContext, x: f32, y: f32, click_count: usize, shift: bool) {
        cx.simulate_event(MouseDownEvent {
            position: point(px(x), px(y)),
            modifiers: Modifiers {
                shift,
                ..Default::default()
            },
            button: MouseButton::Left,
            click_count,
            first_mouse: false,
        });
    }

    fn release(cx: &mut VisualTestContext, x: f32, y: f32) {
        cx.simulate_mouse_up(point(px(x), px(y)), MouseButton::Left, Modifiers::default());
    }

    fn drag(cx: &mut VisualTestContext, from: (f32, f32), to: (f32, f32)) {
        press(cx, from.0, from.1, 1, false);
        cx.simulate_mouse_move(
            point(px(to.0), px(to.1)),
            MouseButton::Left,
            Modifiers::default(),
        );
        release(cx, to.0, to.1);
    }

    fn copied(cx: &mut VisualTestContext) -> String {
        cx.simulate_keystrokes("ctrl-c");
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default()
    }

    #[gpui::test]
    fn lookup_follows_wrapped_rows(cx: &mut TestAppContext) {
        let (owner, cx) = probe(60., &["abcdefghijklmnopqrstuvwxy"], cx);
        let at = |x: f32, y: f32, closest: bool| {
            owner.read_with(cx, |selection, _| {
                index_at(&selection.blocks[&0], point(px(x), px(y)), closest)
            })
        };
        assert_eq!(at(16., 10., true), 3);
        assert_eq!(at(16., 30., true), 13);
        assert_eq!(at(16., 30., false), 12);
        assert_eq!(at(100., 10., true), 10);
        assert_eq!(at(100., 10., false), 9);
        assert_eq!(at(100., 50., false), 24);
        assert_eq!(at(-5., 30., false), 10);
        assert_eq!(at(-5., 30., true), 10);
        assert_eq!(at(7., 50., true), 21);
        assert_eq!(at(7., 100., true), 21);
        assert_eq!(at(7., -5., true), 1);
    }

    #[gpui::test]
    fn dragging_selects_between_two_points_on_wrapped_rows(cx: &mut TestAppContext) {
        let (owner, cx) = probe(60., &["abcdefghijklmnopqrstuvwxy"], cx);
        drag(cx, (16., 10.), (19., 50.));
        assert_eq!(copied(cx), "defghijklmnopqrstuvw");
        drag(cx, (19., 50.), (16., 10.));
        assert_eq!(copied(cx), "defghijklmnopqrstuvw");
        press(cx, 16., 10., 1, false);
        release(cx, 16., 10.);
        assert!(owner.read_with(cx, |selection, _| selection.range().is_none()));
        assert_eq!(copied(cx), "defghijklmnopqrstuvw");
    }

    #[gpui::test]
    fn double_click_takes_a_word_and_triple_click_a_line(cx: &mut TestAppContext) {
        let (_, cx) = probe(200., &["one two-three\nfour", "LaunchR6.exe 0.2.2."], cx);
        press(cx, 30., 10., 2, false);
        assert_eq!(copied(cx), "two");
        cx.simulate_mouse_move(
            point(px(62.), px(10.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        release(cx, 62., 10.);
        assert_eq!(copied(cx), "two-three");
        press(cx, 45., 10., 2, false);
        release(cx, 45., 10.);
        assert_eq!(copied(cx), "-");
        press(cx, 21., 10., 2, false);
        release(cx, 21., 10.);
        assert_eq!(copied(cx), " ");
        press(cx, 150., 10., 2, false);
        release(cx, 150., 10.);
        assert_eq!(copied(cx), "three");
        press(cx, 30., 10., 3, false);
        release(cx, 30., 10.);
        assert_eq!(copied(cx), "one two-three");
        press(cx, 150., 30., 3, false);
        release(cx, 150., 30.);
        assert_eq!(copied(cx), "four");
        press(cx, 10., 50., 2, false);
        release(cx, 10., 50.);
        assert_eq!(copied(cx), "LaunchR6.exe");
        press(cx, 93., 50., 2, false);
        release(cx, 93., 50.);
        assert_eq!(copied(cx), "0.2.2");
        press(cx, 111., 50., 2, false);
        release(cx, 111., 50.);
        assert_eq!(copied(cx), ".");
    }

    #[gpui::test]
    fn double_click_beside_a_wrapped_row_stays_on_that_row(cx: &mut TestAppContext) {
        let (owner, cx) = probe(60., &["aaa bbb ccc ddd eee"], cx);
        let wrapped = owner.read_with(cx, |selection, _| {
            selection.blocks[&0].layout.wrapped_text()
        });
        assert_eq!(wrapped, "aaa bbb \nccc ddd \neee");
        let picked = |cx: &mut VisualTestContext, x: f32, y: f32| {
            press(cx, x, y, 2, false);
            release(cx, x, y);
            owner.read_with(cx, |selection, _| selection.range())
        };
        for x in [50., 58.] {
            assert_eq!(picked(cx, x, 10.), Some((0, 7)..(0, 8)));
            assert_eq!(picked(cx, x, 30.), Some((0, 15)..(0, 16)));
            assert_eq!(picked(cx, x, 50.), Some((0, 16)..(0, 19)));
        }
        assert_eq!(picked(cx, 0., 30.), Some((0, 8)..(0, 11)));
        press(cx, 30., 10., 2, false);
        cx.simulate_mouse_move(
            point(px(200.), px(10.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        release(cx, 200., 10.);
        assert_eq!(copied(cx), "bbb ");
        press(cx, 58., 10., 3, false);
        release(cx, 58., 10.);
        assert_eq!(copied(cx), "aaa bbb ccc ddd eee");
    }

    #[gpui::test]
    fn selection_spans_blocks_and_copies_them_line_by_line(cx: &mut TestAppContext) {
        let (owner, cx) = probe(200., &["first block", "second", "third block"], cx);
        drag(cx, (36., 10.), (30., 50.));
        assert_eq!(copied(cx), "block\nsecond\nthird");
        let spans = owner.read_with(cx, |selection, _| {
            [
                selection.span(0, "first block"),
                selection.span(1, "second"),
                selection.span(2, "third block"),
            ]
        });
        assert_eq!(spans, [6..11, 0..6, 0..5]);
        press(cx, 190., 50., 1, true);
        release(cx, 190., 50.);
        assert_eq!(copied(cx), "block\nsecond\nthird block");
        press(cx, 2., 30., 1, true);
        release(cx, 2., 30.);
        assert_eq!(copied(cx), "block\n");
        cx.simulate_keystrokes("ctrl-a");
        assert_eq!(copied(cx), "first block\nsecond\nthird block");
    }

    #[gpui::test]
    fn selection_outlives_a_press_elsewhere_until_another_area_is_pressed(cx: &mut TestAppContext) {
        let cx = mount(cx, |window, cx| {
            let frame = || div().w(px(200.)).text_size(px(10.)).line_height(px(20.));
            let (upper, lower) = (
                selection("upper", window, cx),
                selection("lower", window, cx),
            );
            div()
                .child(selection_area(&upper, frame(), cx).child(selectable(
                    &upper,
                    0,
                    "first area",
                )))
                .child(selection_area(&lower, frame(), cx).child(selectable(
                    &lower,
                    0,
                    "second area",
                )))
                .into_any_element()
        });
        drag(cx, (0., 10.), (190., 10.));
        cx.simulate_click(point(px(400.), px(300.)), Modifiers::default());
        assert_eq!(copied(cx), "first area");
        drag(cx, (42., 30.), (190., 30.));
        assert_eq!(copied(cx), "area");
        cx.simulate_keystrokes("ctrl-a");
        assert_eq!(copied(cx), "second area");
    }

    #[gpui::test]
    fn dragging_past_a_view_shorter_than_the_area_scrolls_it(cx: &mut TestAppContext) {
        let stash = Rc::new(RefCell::new(None));
        let keep = stash.clone();
        let cx = mount(cx, move |window, cx| {
            let owner = selection("probe", window, cx);
            let scroll = window
                .use_keyed_state("view", cx, |_, _| ScrollHandle::new())
                .read(cx)
                .clone();
            autoscroll(&owner, &scroll, window, cx);
            *keep.borrow_mut() = Some(scroll.clone());
            let frame = div().w(px(200.)).text_size(px(10.)).line_height(px(20.));
            div()
                .id("view")
                .h(px(90.))
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .child(
                    selection_area(&owner, frame, cx)
                        .children((0..30).map(|key| selectable(&owner, key, "line"))),
                )
                .into_any_element()
        });
        let scroll = stash.borrow_mut().take().expect("scroll handle");
        press(cx, 9., 50., 1, false);
        for y in [85., 95.] {
            cx.simulate_mouse_move(
                point(px(9.), px(y)),
                MouseButton::Left,
                Modifiers::default(),
            );
        }
        assert_eq!(scroll.offset().y, px(-5.));
        for _ in 0..2 {
            cx.executor().advance_clock(PUSH_PACE);
            cx.update(|window, cx| window.simulate_next_frame(cx));
        }
        assert_eq!(scroll.offset().y, px(-15.));
        release(cx, 9., 95.);
        cx.update(|window, cx| window.simulate_next_frame(cx));
        assert_eq!(scroll.offset().y, px(-15.));
        assert_eq!(copied(cx), "ine\nline\nl");
    }

    #[gpui::test]
    fn tab_moves_on_from_the_pressed_text(cx: &mut TestAppContext) {
        let clicks = Rc::new(RefCell::new(Vec::new()));
        let log = clicks.clone();
        let cx = mount(cx, move |window, cx| {
            let owner = selection("probe", window, cx);
            let (before, after) = (log.clone(), log.clone());
            div()
                .flex()
                .child(
                    icon_button("before", FOLDER, window, cx)
                        .on_click(move |_, _, _| before.borrow_mut().push("before")),
                )
                .child(
                    selection_area(&owner, div().w(px(100.)), cx)
                        .child(selectable(&owner, 0, "text")),
                )
                .child(
                    icon_button("after", FOLDER, window, cx)
                        .on_click(move |_, _, _| after.borrow_mut().push("after")),
                )
                .into_any_element()
        });
        cx.simulate_click(point(px(40.), px(8.)), Modifiers::default());
        crate::ui::testing::press(cx, "tab enter");
        assert_eq!(*clicks.borrow(), ["after"]);
    }
}
