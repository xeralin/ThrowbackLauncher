use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use crate::ui::px;
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, DispatchPhase, Div, Element,
    ElementId, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, GlobalElementId,
    InspectorElementId, InteractiveElement, IntoElement, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, ParentElement, Pixels, Point, Render,
    ShapedLine, SharedString, StatefulInteractiveElement, Style, Styled, Subscription, Task,
    TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div, fill, point,
    prelude::FluentBuilder, relative, size,
};

use crate::ui::button::glyph_button;
use crate::ui::icon::{EYE, EYE_OFF, icon};
use crate::ui::kit::field;
use crate::ui::scroll::reveal;
use crate::ui::theme::{BORDER, REM, SELECTION, TEXT_MUTED, Theme, contrast};

actions!(
    input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        DeleteWordLeft,
        DeleteWordRight,
        Home,
        End,
        SelectHome,
        SelectEnd,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Commit,
        Revert,
    ]
);

const CONTEXT: &str = "TextInput";
const BLINK: Duration = Duration::from_millis(500);
const BULLET: char = '•';
pub const FIELD_PAD: f32 = 0.4 * REM;
const REVEAL: f32 = 40.;

pub fn init(cx: &mut App) {
    let context = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("left", Left, context),
        KeyBinding::new("right", Right, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("ctrl-left", WordLeft, context),
        KeyBinding::new("ctrl-right", WordRight, context),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, context),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, context),
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, context),
        KeyBinding::new("ctrl-delete", DeleteWordRight, context),
        KeyBinding::new("home", Home, context),
        KeyBinding::new("end", End, context),
        KeyBinding::new("shift-home", SelectHome, context),
        KeyBinding::new("shift-end", SelectEnd, context),
        KeyBinding::new("ctrl-a", SelectAll, context),
        KeyBinding::new("ctrl-c", Copy, context),
        KeyBinding::new("ctrl-x", Cut, context),
        KeyBinding::new("ctrl-v", Paste, context),
        KeyBinding::new("enter", Commit, context),
        KeyBinding::new("escape", Revert, context),
    ]);
}

pub type Filter = Rc<dyn Fn(&str) -> String>;
pub type Accept = Rc<dyn Fn(&str, &mut Window, &mut App) -> String>;
pub type Restore = Rc<dyn Fn(&mut TextInput, &mut Context<TextInput>)>;

#[derive(Default)]
pub struct Hooks {
    pub max_length: Option<usize>,
    pub filter: Option<Filter>,
    pub commit: Option<Accept>,
    pub revert: Option<Restore>,
}

pub struct TextInput {
    focus: FocusHandle,
    content: String,
    selection: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    line: Option<(ShapedLine, Pixels)>,
    selecting: bool,
    editing: bool,
    reverted: bool,
    claimed: bool,
    centered: bool,
    masked: bool,
    scroll: Pixels,
    placeholder: Option<SharedString>,
    blink: Option<Task<()>>,
    caret: bool,
    pub hooks: Hooks,
    _subscriptions: [Subscription; 3],
}

impl TextInput {
    pub fn new(value: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle().tab_index(0).tab_stop(true);
        let subscriptions = [
            cx.on_focus(&focus, window, |_, window, cx| {
                cx.defer_in(window, Self::focused)
            }),
            cx.on_blur(&focus, window, |_, window, cx| {
                cx.defer_in(window, Self::blurred)
            }),
            cx.on_release_in(window, |input, window, cx| {
                if input.editing && !input.reverted {
                    input.accepted(window, cx);
                }
            }),
        ];
        Self {
            focus,
            content: value.to_owned(),
            selection: value.len()..value.len(),
            reversed: false,
            marked: None,
            line: None,
            selecting: false,
            editing: false,
            reverted: false,
            claimed: false,
            centered: false,
            masked: false,
            scroll: px(0.),
            placeholder: None,
            blink: None,
            caret: false,
            hooks: Hooks::default(),
            _subscriptions: subscriptions,
        }
    }

    pub fn centered(mut self) -> Self {
        self.centered = true;
        self
    }

    pub fn placeholder(mut self, text: &'static str) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    pub fn masked(mut self) -> Self {
        self.masked = true;
        self
    }

    pub fn toggle_mask(&mut self, cx: &mut Context<Self>) {
        self.masked = !self.masked;
        cx.notify();
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn set_text(&mut self, value: &str, cx: &mut Context<Self>) {
        if self.content != value {
            self.content = value.to_owned();
            self.marked = None;
            self.move_to(value.len(), cx);
        }
    }

    pub fn append(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let end = self.content.len();
        self.replace(end..end, text, cx);
        self.claim(window, cx);
    }

    pub fn pop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let end = self.content.len();
        self.replace(self.previous(end)..end, "", cx);
        self.claim(window, cx);
    }

    fn claim(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.claimed = !self.focus.is_focused(window);
        window.focus(&self.focus, cx);
    }

    fn focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = true;
        if !std::mem::take(&mut self.claimed) && window.last_input_was_keyboard() {
            self.select_all(&SelectAll, window, cx);
        }
        self.show_caret(cx);
    }

    fn accepted(&self, window: &mut Window, cx: &mut App) -> Option<String> {
        let accept = self.hooks.commit.clone()?;
        Some(accept(&self.content, window, cx))
    }

    fn blurred(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.blink = None;
        self.selecting = false;
        self.editing = false;
        self.marked = None;
        if !std::mem::take(&mut self.reverted)
            && let Some(shown) = self.accepted(window, cx)
        {
            self.set_text(&shown, cx);
        }
        cx.notify();
    }

    fn show_caret(&mut self, cx: &mut Context<Self>) {
        self.caret = true;
        self.blink = self.editing.then(|| {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(BLINK).await;
                    let flipped = this.update(cx, |input, cx| {
                        input.caret = !input.caret;
                        cx.notify();
                    });
                    if flipped.is_err() {
                        break;
                    }
                }
            })
        });
        cx.notify();
    }

    fn cursor(&self) -> usize {
        if self.reversed {
            self.selection.start
        } else {
            self.selection.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selection = offset..offset;
        self.reversed = false;
        self.show_caret(cx);
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.reversed {
            self.selection.start = offset;
        } else {
            self.selection.end = offset;
        }
        if self.selection.end < self.selection.start {
            self.reversed = !self.reversed;
            self.selection = self.selection.end..self.selection.start;
        }
        cx.notify();
    }

    fn previous(&self, offset: usize) -> usize {
        self.content[..offset]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next(&self, offset: usize) -> usize {
        self.content[offset..]
            .chars()
            .next()
            .map_or(self.content.len(), |character| {
                offset + character.len_utf8()
            })
    }

    fn word_start(&self, offset: usize) -> usize {
        if self.masked {
            return 0;
        }
        let head = self.content[..offset].trim_end();
        let gap = head
            .char_indices()
            .rfind(|(_, letter)| letter.is_whitespace());
        gap.map_or(0, |(index, space)| index + space.len_utf8())
    }

    fn word_end(&self, offset: usize) -> usize {
        if self.masked {
            return self.content.len();
        }
        let tail = &self.content[offset..];
        let word = tail.trim_start();
        let length = word.find(char::is_whitespace).unwrap_or(word.len());
        self.content.len() - word.len() + length
    }

    fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        let mut text = text.replace(['\r', '\n'], " ");
        if let Some(limit) = self.hooks.max_length {
            let kept = self.content.chars().count() - self.content[range.clone()].chars().count();
            text = text.chars().take(limit.saturating_sub(kept)).collect();
        }
        let head = format!("{}{text}", &self.content[..range.start]);
        let raw = format!("{head}{}", &self.content[range.end..]);
        let (content, caret) = match &self.hooks.filter {
            Some(filter) => (filter(&raw), filter(&head).len()),
            None => (raw, head.len()),
        };
        let caret = content.floor_char_boundary(caret);
        self.content = content;
        self.marked = None;
        self.move_to(caret, cx);
    }

    fn delete_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            self.select_to(offset, cx);
        }
        self.replace(self.selection.clone(), "", cx);
    }

    fn shown(&self) -> SharedString {
        if self.masked {
            let bullets = self.content.chars().map(|_| BULLET);
            bullets.collect::<String>().into()
        } else {
            self.content.clone().into()
        }
    }

    fn shown_index(&self, index: usize) -> usize {
        if self.masked {
            self.content[..index].chars().count() * BULLET.len_utf8()
        } else {
            index
        }
    }

    fn content_index(&self, index: usize) -> usize {
        if self.masked {
            let mut starts = self.content.char_indices().map(|(start, _)| start);
            starts
                .nth(index / BULLET.len_utf8())
                .unwrap_or(self.content.len())
        } else {
            index
        }
    }

    fn index_at(&self, position: Point<Pixels>) -> usize {
        let Some((line, origin)) = &self.line else {
            return 0;
        };
        let x = position.x - *origin;
        let end = self.content.len();
        let last = line.x_for_index(self.shown_index(self.previous(end)));
        if x <= last {
            self.content_index(line.closest_index_for_x(x))
        } else if x - last < line.width - x {
            self.previous(end)
        } else {
            end
        }
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_to(self.previous(self.cursor()), cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_to(self.next(self.cursor()), cx);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let target = if self.selection.is_empty() {
            self.previous(self.cursor())
        } else {
            self.selection.start
        };
        self.move_to(target, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let target = if self.selection.is_empty() {
            self.next(self.cursor())
        } else {
            self.selection.end
        };
        self.move_to(target, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous(self.cursor()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next(self.cursor()), cx);
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.word_start(self.cursor()), cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.word_end(self.cursor()), cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.word_start(self.cursor()), cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.word_end(self.cursor()), cx);
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_to(self.word_start(self.cursor()), cx);
    }

    fn delete_word_right(&mut self, _: &DeleteWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_to(self.word_end(self.cursor()), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.content.len(), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = 0..self.content.len();
        self.reversed = false;
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.masked && !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selection.clone()].to_owned(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.masked {
            self.copy(&Copy, window, cx);
            self.replace(self.selection.clone(), "", cx);
        }
    }

    fn insert_clipboard(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace(self.selection.clone(), &text, cx);
        }
    }

    fn commit(&mut self, _: &Commit, window: &mut Window, cx: &mut Context<Self>) {
        window.blur(cx);
    }

    fn revert(&mut self, _: &Revert, window: &mut Window, cx: &mut Context<Self>) {
        self.reverted = true;
        if let Some(revert) = self.hooks.revert.clone() {
            revert(self, cx);
        }
        window.blur(cx);
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let index = self.index_at(event.position);
        if event.click_count > 1 {
            let start = self.word_start(self.next(index));
            self.selection = match event.click_count {
                2 => start..self.word_end(start),
                _ => 0..self.content.len(),
            };
            self.reversed = false;
            cx.notify();
        } else if event.modifiers.shift {
            self.select_to(index, cx);
        } else {
            self.selecting = true;
            self.move_to(index, cx);
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut units = 0;
        for (index, character) in self.content.char_indices() {
            if units >= offset {
                return index;
            }
            units += character.len_utf16();
        }
        self.content.len()
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        let units = |end: usize| self.content[..end].encode_utf16().count();
        units(range.start)..units(range.end)
    }

    fn target_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked.clone())
            .unwrap_or(self.selection.clone())
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selection),
            reversed: self.reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace(self.target_range(range_utf16), text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        selected_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace(self.target_range(range_utf16), text, cx);
        let end = self.cursor();
        let start = end.checked_sub(text.len());
        let Some(start) = start.filter(|start| self.content.get(*start..end) == Some(text)) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        let marked = start..end;
        if let Some(selected) = selected_utf16 {
            let start = self.range_to_utf16(&(0..marked.start)).end;
            let length = text.encode_utf16().count();
            self.selection = self.range_from_utf16(
                &(start + selected.start.min(length)..start + selected.end.min(length)),
            );
        }
        self.marked = Some(marked);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let (line, origin) = self.line.as_ref()?;
        let x_at = |index: usize| *origin + line.x_for_index(self.shown_index(index));
        Some(Bounds::from_corners(
            point(x_at(range.start), bounds.top()),
            point(x_at(range.end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.range_to_utf16(&(0..self.index_at(point))).end)
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::insert_clipboard))
            .on_action(cx.listener(Self::commit))
            .on_action(cx.listener(Self::revert))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .size_full()
            .flex()
            .items_center()
            .when(!self.centered, |input| input.px(px(FIELD_PAD)).pt(px(1.)))
            .child(TextElement { input: cx.entity() })
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct TextLayout {
    line: ShapedLine,
    hint: Option<ShapedLine>,
    origin: Point<Pixels>,
    scroll: Pixels,
    selection: Option<PaintQuad>,
    caret: Option<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = TextLayout;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let style = window.text_style();
        let focused = input.focus.is_focused(window);
        let selected = if focused {
            input.selection.clone()
        } else {
            0..0
        };
        let marked = input.marked.clone().unwrap_or(0..0);
        let mut edges = vec![
            0,
            selected.start,
            selected.end,
            marked.start,
            marked.end,
            input.content.len(),
        ];
        edges.sort_unstable();
        edges.dedup();
        let runs: Vec<TextRun> = edges
            .windows(2)
            .map(|edge| TextRun {
                len: input.shown_index(edge[1]) - input.shown_index(edge[0]),
                font: style.font(),
                color: if selected.contains(&edge[0]) {
                    contrast().solid()
                } else {
                    style.color
                },
                background_color: None,
                underline: marked.contains(&edge[0]).then(|| UnderlineStyle {
                    color: Some(style.color),
                    thickness: px(1.),
                    wavy: false,
                }),
                strikethrough: None,
            })
            .collect();
        let line = window.text_system().shape_line(
            input.shown(),
            style.font_size.to_pixels(window.rem_size()),
            &runs,
            None,
        );
        let hint = input
            .placeholder
            .clone()
            .filter(|_| input.content.is_empty() && input.marked.is_none())
            .map(|text| {
                let run = TextRun {
                    len: text.len(),
                    font: style.font(),
                    color: TEXT_MUTED.solid(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let size = style.font_size.to_pixels(window.rem_size());
                window.text_system().shape_line(text, size, &[run], None)
            });
        let room = bounds.size.width;
        let slack = if input.centered {
            (room - line.width) / 2.
        } else {
            px(0.)
        };
        let tip = line.x_for_index(input.shown_index(input.cursor()));
        let hidden = (line.width + px(1.) - room).max(px(0.));
        let scroll = input.scroll.min(tip).max(tip + px(1.) - room).min(hidden);
        let origin = point(bounds.left() + slack - scroll, bounds.top());
        let x_at = |index: usize| origin.x + line.x_for_index(input.shown_index(index));
        let selection = (!selected.is_empty()).then(|| {
            fill(
                Bounds::from_corners(
                    point(x_at(selected.start), bounds.top()),
                    point(x_at(selected.end), bounds.bottom()),
                ),
                SELECTION.solid(),
            )
        });
        let caret = (focused && selected.is_empty() && input.caret).then(|| {
            fill(
                Bounds::new(
                    point(x_at(input.cursor()), bounds.top()),
                    size(px(1.), bounds.size.height),
                ),
                style.color,
            )
        });
        TextLayout {
            line,
            hint,
            origin,
            scroll,
            selection,
            caret,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        layout: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let input = self.input.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && input.read(cx).selecting {
                input.update(cx, |input, cx| {
                    input.select_to(input.index_at(event.position), cx);
                });
            }
        });
        let height = window.line_height();
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(selection) = layout.selection.take() {
                window.paint_quad(selection);
            }
            for line in layout.hint.iter().chain([&layout.line]) {
                line.paint(layout.origin, height, TextAlign::Left, None, window, cx)
                    .ok();
            }
            if let Some(caret) = layout.caret.take() {
                window.paint_quad(caret);
            }
        });
        let painted = (layout.line.clone(), layout.origin.x);
        self.input.update(cx, |input, _| {
            input.line = Some(painted);
            input.scroll = layout.scroll;
        });
    }
}

pub fn typing(window: &Window) -> bool {
    window
        .context_stack()
        .iter()
        .any(|context| context.contains(CONTEXT))
}

pub fn text_field(input: &Entity<TextInput>, window: &Window, cx: &App) -> Div {
    let edge = if input.read(cx).focus.is_focused(window) {
        Theme::get(cx).action
    } else {
        BORDER.srgb()
    };
    field()
        .min_w_0()
        .border_color(edge.solid())
        .child(input.clone())
        .map(reveal)
}

pub fn password_field(input: &Entity<TextInput>, window: &mut Window, cx: &mut App) -> Div {
    let glyph = if input.read(cx).masked { EYE_OFF } else { EYE };
    let toggle = input.clone();
    div()
        .child(text_field(input, window, cx).pr(px(REVEAL - FIELD_PAD)))
        .child(
            glyph_button(("reveal", input.entity_id()), icon(glyph, 16.), window, cx)
                .absolute()
                .top_0()
                .bottom_0()
                .right_0()
                .px(px(12.))
                .on_click(move |_, _, cx| toggle.update(cx, |input, cx| input.toggle_mask(cx))),
        )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui::{AppContext, Modifiers, TestAppContext, VisualTestContext};

    use super::*;
    use backend::settings::clean_username;

    struct Probe {
        input: Entity<TextInput>,
    }

    impl Render for Probe {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(text_field(&self.input, window, cx).w(px(200.)))
        }
    }

    fn probe<'a>(
        value: &str,
        cx: &'a mut TestAppContext,
    ) -> (Entity<TextInput>, &'a mut VisualTestContext) {
        cx.update(crate::init);
        let (view, cx) = cx.add_window_view(|window, cx| Probe {
            input: cx.new(|cx| TextInput::new(value, window, cx)),
        });
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        let input = view.read_with(cx, |probe, _| probe.input.clone());
        (input, cx)
    }

    fn enter(input: &Entity<TextInput>, cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            let handle = input.read(cx).focus_handle().clone();
            window.focus(&handle, cx);
        });
        cx.run_until_parked();
    }

    fn state(input: &Entity<TextInput>, cx: &mut VisualTestContext) -> (String, Range<usize>) {
        input.read_with(cx, |input, _| {
            (input.content.clone(), input.selection.clone())
        })
    }

    #[gpui::test]
    fn typing_inserts_at_the_caret(cx: &mut TestAppContext) {
        let (input, cx) = probe("Throwback", cx);
        enter(&input, cx);
        cx.simulate_keystrokes("home right right");
        cx.simulate_input("XY");
        assert_eq!(state(&input, cx), ("ThXYrowback".to_owned(), 4..4));
        cx.simulate_keystrokes("backspace delete");
        assert_eq!(state(&input, cx), ("ThXowback".to_owned(), 3..3));
    }

    #[gpui::test]
    fn the_control_key_moves_selects_and_deletes_by_words(cx: &mut TestAppContext) {
        let (input, cx) = probe("-windowed  /vulkan --fps=120", cx);
        enter(&input, cx);
        cx.simulate_keystrokes("ctrl-left");
        assert_eq!(state(&input, cx).1, 19..19);
        cx.simulate_keystrokes("ctrl-left ctrl-left ctrl-left");
        assert_eq!(state(&input, cx).1, 0..0);
        cx.simulate_keystrokes("ctrl-right");
        assert_eq!(state(&input, cx).1, 9..9);
        cx.simulate_keystrokes("ctrl-shift-right");
        assert_eq!(state(&input, cx).1, 9..18);
        cx.simulate_keystrokes("ctrl-delete");
        assert_eq!(state(&input, cx), ("-windowed --fps=120".to_owned(), 9..9));
        cx.simulate_keystrokes("end ctrl-backspace");
        assert_eq!(state(&input, cx), ("-windowed ".to_owned(), 10..10));
        cx.simulate_keystrokes("ctrl-shift-left");
        assert_eq!(state(&input, cx).1, 0..10);

        input.update(cx, |input, cx| {
            input.set_text("secret words", cx);
            input.masked = true;
        });
        cx.simulate_keystrokes("ctrl-left");
        assert_eq!(state(&input, cx).1, 0..0);
        cx.simulate_keystrokes("ctrl-right");
        assert_eq!(state(&input, cx).1, 12..12);
    }

    #[gpui::test]
    fn filtered_characters_leave_the_caret_in_place(cx: &mut TestAppContext) {
        let (input, cx) = probe("abc", cx);
        input.update(cx, |input, _| {
            input.hooks.filter = Some(Rc::new(clean_username));
        });
        enter(&input, cx);
        cx.simulate_keystrokes("home");
        cx.simulate_input("x");
        assert_eq!(state(&input, cx), ("xabc".to_owned(), 1..1));
        cx.simulate_input("!");
        assert_eq!(state(&input, cx), ("xabc".to_owned(), 1..1));
    }

    #[gpui::test]
    fn composing_in_a_field_whose_text_the_filter_shortens_marks_nothing(cx: &mut TestAppContext) {
        let (input, cx) = probe("A sh", cx);
        input.update(cx, |input, _| {
            input.hooks.filter = Some(Rc::new(clean_username));
        });
        enter(&input, cx);
        cx.simulate_keystrokes("end");
        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "^", None, window, cx);
            });
        });
        assert_eq!(state(&input, cx), ("Ash".to_owned(), 3..3));
        assert_eq!(input.read_with(cx, |input, _| input.marked.clone()), None);
        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "e", None, window, cx);
            });
        });
        assert_eq!(state(&input, cx).0, "Ashe");
        assert_eq!(
            input.read_with(cx, |input, _| input.marked.clone()),
            Some(3..4)
        );
    }

    #[gpui::test]
    fn length_limit_counts_what_stays(cx: &mut TestAppContext) {
        let (input, cx) = probe("0123456789", cx);
        input.update(cx, |input, _| input.hooks.max_length = Some(12));
        enter(&input, cx);
        cx.simulate_keystrokes("end");
        cx.simulate_input("abcdef");
        assert_eq!(state(&input, cx).0, "0123456789ab");
        cx.simulate_keystrokes("ctrl-a");
        cx.simulate_input("line\nbreak and more text");
        assert_eq!(state(&input, cx).0, "line break a");
    }

    #[gpui::test]
    fn selection_grows_and_collapses(cx: &mut TestAppContext) {
        let (input, cx) = probe("abcdef", cx);
        enter(&input, cx);
        cx.simulate_keystrokes("end shift-left shift-left");
        assert_eq!(state(&input, cx).1, 4..6);
        cx.simulate_keystrokes("left");
        assert_eq!(state(&input, cx).1, 4..4);
        cx.simulate_keystrokes("shift-home");
        assert_eq!(state(&input, cx).1, 0..4);
        cx.simulate_keystrokes("right");
        assert_eq!(state(&input, cx).1, 4..4);
        cx.simulate_keystrokes("ctrl-a");
        cx.simulate_input("z");
        assert_eq!(state(&input, cx), ("z".to_owned(), 1..1));
    }

    #[gpui::test]
    fn clipboard_cuts_and_pastes_the_selection(cx: &mut TestAppContext) {
        let (input, cx) = probe("abcdef", cx);
        enter(&input, cx);
        cx.simulate_keystrokes("home shift-right shift-right ctrl-x");
        assert_eq!(state(&input, cx), ("cdef".to_owned(), 0..0));
        cx.simulate_keystrokes("end ctrl-v");
        assert_eq!(state(&input, cx), ("cdefab".to_owned(), 6..6));
    }

    #[gpui::test]
    fn keyboard_focus_selects_everything(cx: &mut TestAppContext) {
        let (input, cx) = probe("abc", cx);
        cx.simulate_keystrokes("tab");
        enter(&input, cx);
        assert_eq!(state(&input, cx).1, 0..3);
    }

    #[gpui::test]
    fn pointer_focus_keeps_the_caret(cx: &mut TestAppContext) {
        let (input, cx) = probe("abc", cx);
        enter(&input, cx);
        assert_eq!(state(&input, cx).1, 3..3);
    }

    #[gpui::test]
    fn a_drag_keeps_selecting_outside_the_field(cx: &mut TestAppContext) {
        let (input, cx) = probe("Throwback", cx);
        let (left, none) = (MouseButton::Left, Modifiers::default());
        let outside = point(px(400.), px(300.));
        cx.simulate_mouse_down(point(px(4.), px(16.)), left, none);
        cx.simulate_mouse_move(outside, Some(left), none);
        assert_eq!(state(&input, cx).1, 0..9);
        cx.simulate_mouse_up(outside, left, none);
        cx.simulate_mouse_move(point(px(4.), px(16.)), None, none);
        assert_eq!(state(&input, cx).1, 0..9);
    }

    #[gpui::test]
    fn a_field_that_goes_away_while_edited_commits_its_draft(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (view, cx) = cx.add_window_view(|window, cx| Probe {
            input: cx.new(|cx| TextInput::new("", window, cx)),
        });
        cx.update(|window, _| window.activate_window());
        let commits = Rc::new(RefCell::new(Vec::<String>::new()));
        let renew = |cx: &mut VisualTestContext| {
            let log = commits.clone();
            view.update_in(cx, |probe, window, cx| {
                probe.input = cx.new(|cx| TextInput::new("old", window, cx));
                let handle = probe.input.update(cx, |input, _| {
                    input.hooks.commit = Some(Rc::new(move |draft, _, _| {
                        log.borrow_mut().push(draft.to_owned());
                        draft.to_owned()
                    }));
                    input.focus_handle().clone()
                });
                window.focus(&handle, cx);
                cx.notify();
            });
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.run_until_parked();
        };
        renew(cx);
        cx.simulate_input("er");
        renew(cx);
        assert_eq!(*commits.borrow(), ["older"]);
        cx.simulate_input("!!");
        cx.simulate_keystrokes("escape");
        renew(cx);
        assert_eq!(*commits.borrow(), ["older"]);
    }

    #[gpui::test]
    fn enter_commits_and_escape_reverts(cx: &mut TestAppContext) {
        let (input, cx) = probe("old", cx);
        let commits = Rc::new(RefCell::new(Vec::<String>::new()));
        let log = commits.clone();
        input.update(cx, |input, _| {
            input.hooks.commit = Some(Rc::new(move |draft, _, _| {
                log.borrow_mut().push(draft.to_owned());
                draft.to_owned()
            }));
            input.hooks.revert = Some(Rc::new(|input, cx| input.set_text("old", cx)));
        });
        enter(&input, cx);
        cx.simulate_keystrokes("end");
        cx.simulate_input("er");
        cx.simulate_keystrokes("enter");
        assert_eq!(*commits.borrow(), ["older"]);
        enter(&input, cx);
        cx.simulate_keystrokes("end");
        cx.simulate_input("!!");
        cx.simulate_keystrokes("escape");
        assert_eq!(state(&input, cx).0, "old");
        assert_eq!(*commits.borrow(), ["older"]);
    }
}
