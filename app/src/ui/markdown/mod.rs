mod parse;

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use crate::ui::px;
use gpui::{
    App, Div, ElementId, InteractiveElement, IntoElement, ParentElement, Pixels, Stateful,
    StatefulInteractiveElement, Styled, Transformation, Window, div, prelude::FluentBuilder,
    radians, relative,
};

use parse::{Alert, Align, Inline, Item, Style};
pub use parse::{Block, parse};

use crate::ui::icon::{CHECK_BOLD, CHEVRON_RIGHT, icon};
use crate::ui::kit::link_preview;
use crate::ui::motion::{self, EASE_OUT};
use crate::ui::picture;
use crate::ui::prose::{Prose, Spacing, code};
use crate::ui::theme::{
    BORDER, BRAND, ERROR_BG, ERROR_LINK, ERROR_TEXT, FONT_BODY, FONT_BODY_SEMIBOLD,
    FONT_DISPLAY_BOLD, FONT_MONO, FONT_MONO_20, Face, LINK, NOTICE_BG, NOTICE_EDGE, NOTICE_LINK,
    NOTICE_TEXT, REM, SURFACE_2, Srgb, TEXT, TEXT_LABEL, TEXT_MICRO, TEXT_MUTED, TEXT_UI, Theme,
    Tone, Typeface, contrast,
};

const LINE: f32 = 20.;
const HEADINGS: [f32; 6] = [2., 1.5, 1.25, 1., 0.875, 0.85];
const HEADING_LINE: f32 = 1.25;
const GAP: f32 = 8.;
const HEADING_GAP: f32 = 14.;
const NOTES_GAP: f32 = 12.;
const ITEM_GAP: f32 = 2.;
const INDENT: f32 = 16.;
const BULLET: f32 = 5.;
const CHECK: f32 = 14.;
const PRE: f32 = 0.78 * REM;
const PRE_LINE: f32 = 1.45;
const TABLE: f32 = 0.78 * REM;
const STRUCK: f32 = 0.7;
const LONG: usize = 32;
const BRIDGE: &str = "  ";
const FOLD: Duration = Duration::from_millis(200);
const BODY: Prose = Prose {
    size: TEXT_UI,
    line: LINE,
    code: 0.85 * TEXT_UI,
};
const CELL: Prose = Prose {
    size: TABLE,
    line: 14.,
    code: 11.,
};
const NOTE: Prose = Prose {
    size: TEXT_LABEL,
    line: 1.5 * TEXT_LABEL,
    code: 0.85 * TEXT_LABEL,
};

#[derive(Clone, Copy)]
struct Ink {
    link: Srgb,
    strong: Option<Srgb>,
    code: [Srgb; 3],
    lined: bool,
}

struct Piece {
    range: Range<usize>,
    style: Style,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Space {
    None,
    Free,
    Bound,
}

enum Bit {
    Word(Space, Vec<Piece>),
    Break,
}

fn bits(content: &Inline) -> Vec<Bit> {
    let mut bits = Vec::new();
    let mut word = Vec::new();
    let mut end = 0;
    for (length, style) in &content.spans {
        let start = end;
        end += length;
        if style.code || style.image.is_some() {
            word.push(Piece {
                range: start..end,
                style: *style,
            });
            continue;
        }
        let bound = style.link.is_some() || style.strike || style.inserted;
        let mut from = start;
        for (offset, letter) in content.text[start..end].char_indices() {
            if letter != ' ' && letter != '\n' {
                continue;
            }
            let here = start + offset;
            if here > from {
                word.push(Piece {
                    range: from..here,
                    style: *style,
                });
            }
            if !word.is_empty() {
                let space = match letter {
                    '\n' => Space::None,
                    _ if bound && here > start && here + 1 < end => Space::Bound,
                    _ => Space::Free,
                };
                bits.push(Bit::Word(space, std::mem::take(&mut word)));
            }
            if letter == '\n' {
                bits.push(Bit::Break);
            }
            from = here + 1;
        }
        if end > from {
            word.push(Piece {
                range: from..end,
                style: *style,
            });
        }
    }
    if !word.is_empty() {
        bits.push(Bit::Word(Space::None, word));
    }
    bits
}

fn chrome(kind: Alert) -> [Srgb; 4] {
    let tones = |kind: usize| [0, 1, 2, 3].map(|index| Tone::Alert(kind, index).srgb());
    match kind {
        Alert::Note => tones(0),
        Alert::Tip => tones(1),
        Alert::Important => tones(2),
        Alert::Warning => [NOTICE_BG, NOTICE_TEXT, NOTICE_LINK, NOTICE_EDGE].map(Tone::srgb),
        Alert::Caution => [ERROR_BG.srgb(), ERROR_TEXT.srgb(), ERROR_LINK.srgb(), BRAND],
    }
}

fn heading_size(level: u8) -> f32 {
    HEADINGS[usize::from(level.clamp(1, 6)) - 1] * TEXT_UI
}

fn heading_line(level: u8) -> f32 {
    (HEADING_LINE * heading_size(level)).round()
}

fn first_line(blocks: &[Block]) -> f32 {
    match blocks.first() {
        Some(Block::Heading(level, _)) => heading_line(*level),
        _ => LINE,
    }
}

fn roman(mut number: usize) -> String {
    const PARTS: [(usize, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut out = String::new();
    for (value, letters) in PARTS {
        while number >= value {
            out.push_str(letters);
            number -= value;
        }
    }
    out
}

fn letters(mut number: usize) -> String {
    let mut out = Vec::new();
    while number > 0 {
        number -= 1;
        out.push(b'a' + (number % 26) as u8);
        number /= 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

fn ordinal(number: usize, depth: usize) -> String {
    match depth {
        0 => number.to_string(),
        1 => roman(number),
        _ => letters(number),
    }
}

fn marker(start: Option<u32>, index: usize, depth: usize, line: f32) -> Div {
    if let Some(first) = start {
        return div()
            .absolute()
            .top_0()
            .right(relative(1.))
            .pr(px(0.25 * TEXT_UI))
            .whitespace_nowrap()
            .child(format!("{}.", ordinal(first as usize + index, depth)));
    }
    let color = TEXT_MUTED.solid();
    let dot = div()
        .absolute()
        .left(px(-INDENT))
        .top(px((line - BULLET) / 2.))
        .size(px(BULLET));
    match depth {
        0 => dot.rounded_full().bg(color),
        1 => dot.rounded_full().border_1().border_color(color),
        _ => dot.bg(color),
    }
}

fn check(done: bool, line: f32, cx: &App) -> Div {
    let theme = Theme::get(cx);
    let frame = div()
        .absolute()
        .left(px(-INDENT))
        .top(px((line - CHECK) / 2.))
        .size(px(CHECK))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1();
    if done {
        frame
            .border_color(theme.action.solid())
            .bg(theme.action.solid())
            .child(icon(CHECK_BOLD, 12.).text_color(theme.action_text.solid()))
    } else {
        frame.border_color(BORDER.solid()).bg(SURFACE_2.solid())
    }
}

struct Flow {
    id: ElementId,
    parts: usize,
    ink: Ink,
    prose: Prose,
}

impl Flow {
    fn part(&mut self, kind: &'static str) -> ElementId {
        self.parts += 1;
        ElementId::from((kind, self.parts))
    }

    fn within<T>(&mut self, prose: Prose, draw: impl FnOnce(&mut Self) -> T) -> T {
        let outer = std::mem::replace(&mut self.prose, prose);
        let drawn = draw(self);
        self.prose = outer;
        drawn
    }

    fn text(
        &mut self,
        content: &Inline,
        base: Face,
        strong: Face,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let (ink, prose) = (self.ink, self.prose);
        let hovered = (!content.links.is_empty()).then(|| {
            let id = self.part("links");
            window.with_element_namespace(self.id.clone(), |window| {
                window.use_keyed_state(id, cx, |_, _| None::<(usize, ElementId)>)
            })
        });
        let over = hovered
            .as_ref()
            .and_then(|hovered| hovered.read(cx).as_ref().map(|(link, _)| *link));
        let mut spacing = Spacing::new(base, prose.size, window);
        let mut flow = div().flex().flex_wrap().items_end().gap_x(spacing.gap);
        for bit in bits(content) {
            let (space, pieces) = match bit {
                Bit::Word(space, pieces) => (space, pieces),
                Bit::Break => {
                    flow = flow.child(div().w_full());
                    continue;
                }
            };
            let count = pieces.len();
            let mut parts = Vec::with_capacity(count);
            for (index, piece) in pieces.into_iter().enumerate() {
                let style = piece.style;
                let part = &content.text[piece.range];
                let face = if style.strong { strong } else { base };
                let shown = style.image.and_then(|image| {
                    let image = &content.images[image];
                    let loaded = picture::loaded(&image.source, window, cx)?;
                    Some((loaded, image.width))
                });
                let piece = if let Some((loaded, width)) = shown {
                    let limit = width.unwrap_or(loaded.size(0).width.0 as f32);
                    div().min_w_0().child(picture::view(loaded, px(limit)))
                } else if style.code {
                    let [edge, ground, lit] = ink.code;
                    code(part, prose, edge, ground).text_color(lit.solid())
                } else {
                    let width = spacing.part(part, face, style.emphasis, window);
                    let piece = if space == Space::Bound && index + 1 == count {
                        let room = width + spacing.gap + spacing.rest(window);
                        let bridged = format!("{part}{BRIDGE}");
                        div().w(room).overflow_hidden().child(bridged)
                    } else {
                        div().w(width).child(part.to_owned())
                    };
                    if part.chars().count() > LONG {
                        piece.min_w_0()
                    } else {
                        piece.flex_none().whitespace_nowrap()
                    }
                };
                let followed = style.link.is_some() && (ink.lined || style.link == over);
                let piece = piece
                    .when(style.strong && !style.code, |piece| {
                        piece
                            .face(strong, prose.size)
                            .line_height(px(prose.line))
                            .when_some(ink.strong, |piece, color| piece.text_color(color.solid()))
                    })
                    .when(style.emphasis, |piece| piece.italic())
                    .when(style.strike, |piece| piece.line_through().opacity(STRUCK))
                    .when(style.inserted || followed, |piece| piece.underline());
                let (Some(link), Some(hovered)) = (style.link, hovered.clone()) else {
                    parts.push(piece.into_any_element());
                    continue;
                };
                let url = content.links[link].clone();
                let id = self.part("piece");
                let mine = id.clone();
                let piece = piece
                    .when(!style.code, |piece| piece.text_color(ink.link.solid()))
                    .id(id)
                    .cursor_pointer()
                    .on_hover(move |inside, _, cx| {
                        let inside = *inside;
                        hovered.update(cx, |over, cx| {
                            if hover(over, link, &mine, inside) {
                                cx.notify();
                            }
                        });
                    })
                    .on_click(move |_, _, cx| cx.open_url(&url));
                parts.push(piece.into_any_element());
            }
            let margin = match space {
                Space::None => Pixels::ZERO,
                Space::Free => spacing.rest(window),
                Space::Bound => -spacing.gap,
            };
            flow = flow.child(div().flex().max_w_full().mr(margin).children(parts));
        }
        let preview = over
            .and_then(|link| content.links.get(link))
            .and_then(|url| link_preview(url.clone(), window));
        flow.children(preview)
    }

    fn column(
        &mut self,
        blocks: &[Block],
        depth: usize,
        root: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let mut column = div().flex().flex_col();
        for (index, block) in blocks.iter().enumerate() {
            let gap = match block {
                _ if index == 0 => 0.,
                Block::Heading(..) if root => HEADING_GAP,
                Block::Footnotes(_) => NOTES_GAP,
                _ => GAP,
            };
            column = column.child(self.block(block, depth, window, cx).mt(px(gap)));
        }
        column
    }

    fn list(
        &mut self,
        start: Option<u32>,
        tight: bool,
        items: &[Item],
        depth: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let mut list = div().flex().flex_col().pl(px(INDENT));
        for (index, item) in items.iter().enumerate() {
            let line = first_line(&item.blocks);
            let sign = match item.checked {
                Some(done) => check(done, line, cx),
                None => marker(start, index, depth, line),
            };
            let mut row = div()
                .flex()
                .flex_col()
                .mt(px(if index > 0 { ITEM_GAP } else { 0. }))
                .when(item.checked.is_some(), |row| row.pl(px(4.)))
                .child(sign);
            let mut elements = 0;
            for block in &item.blocks {
                let bare = tight && matches!(block, Block::Paragraph(_));
                let gap = if bare || elements == 0 { 0. } else { ITEM_GAP };
                elements += usize::from(!bare);
                row = row.child(self.block(block, depth + 1, window, cx).mt(px(gap)));
            }
            list = list.child(row);
        }
        list
    }

    fn table(
        &mut self,
        align: &[Align],
        rows: &[Vec<Inline>],
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let last = align.len().saturating_sub(1);
        let mut grid = div()
            .grid()
            .grid_cols_max_content(align.len().max(1) as u16)
            .face(FONT_BODY, TABLE)
            .line_height(px(14.))
            .text_color(TEXT_MUTED.solid());
        for (index, row) in rows.iter().enumerate() {
            for (column, side) in align.iter().enumerate() {
                let cell = div()
                    .flex()
                    .items_center()
                    .pl(px(10.))
                    .pr(px(9.))
                    .py(px(3.))
                    .border_color(BORDER.solid())
                    .when(column < last, |cell| cell.border_r_1())
                    .when(index > 0 && index + 1 < rows.len(), |cell| {
                        cell.border_b_1()
                    });
                let cell = match side {
                    Align::Start => cell,
                    Align::Center => cell.justify_center(),
                    Align::End => cell.justify_end(),
                };
                let Some(content) = row.get(column) else {
                    grid = grid.child(cell);
                    continue;
                };
                let cell = if index == 0 {
                    let head = Inline {
                        text: content.text.to_ascii_uppercase(),
                        spans: content.spans.clone(),
                        links: content.links.clone(),
                        images: content.images.clone(),
                    };
                    let prose = Prose {
                        size: TEXT_MICRO,
                        line: 12.,
                        code: 0.85 * TEXT_MICRO,
                    };
                    cell.bg(SURFACE_2.solid())
                        .when(column == 0, |cell| cell.rounded_tl(px(7.)))
                        .when(column == last, |cell| cell.rounded_tr(px(7.)))
                        .pt(px(4.))
                        .face(FONT_MONO_20, TEXT_MICRO)
                        .line_height(px(12.))
                        .child(self.within(prose, |flow| {
                            flow.text(&head, FONT_MONO_20, FONT_MONO_20, window, cx)
                        }))
                } else if column == 0 {
                    cell.face(FONT_MONO, TABLE).text_color(TEXT.solid()).child(
                        self.within(CELL, |flow| {
                            flow.text(content, FONT_MONO, FONT_MONO, window, cx)
                        }),
                    )
                } else {
                    cell.child(self.within(CELL, |flow| {
                        flow.text(content, FONT_BODY, FONT_BODY_SEMIBOLD, window, cx)
                    }))
                };
                grid = grid.child(cell);
            }
        }
        let frame = div()
            .id(self.part("table"))
            .max_w_full()
            .overflow_x_scroll()
            .rounded(px(8.))
            .border_1()
            .border_color(BORDER.solid())
            .flex()
            .child(grid.flex_none());
        div().flex().child(frame)
    }

    fn details(
        &mut self,
        summary: &Inline,
        open: bool,
        blocks: &[Block],
        depth: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let id = self.part("details");
        let inside = format!("details-{}", self.parts);
        let shown = window.with_element_namespace(self.id.clone(), |window| {
            window.use_keyed_state(id.clone(), cx, |_, _| open)
        });
        let unfolded = *shown.read(cx);
        let target = if unfolded { 1. } else { 0. };
        let turn = window.with_element_namespace(self.id.clone(), |window| {
            motion::transition(id.clone(), target, FOLD, EASE_OUT, window, cx)
        });
        let pose = Transformation::rotate(radians(turn * std::f32::consts::FRAC_PI_2));
        let title = if summary.text.is_empty() {
            div().child("Details")
        } else {
            self.text(summary, FONT_BODY, FONT_BODY_SEMIBOLD, window, cx)
        };
        let head = div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(6.))
            .cursor_pointer()
            .text_color(TEXT.solid())
            .on_click(move |_, _, cx| {
                shown.update(cx, |shown, cx| {
                    *shown = !*shown;
                    cx.notify();
                });
            })
            .child(
                icon(CHEVRON_RIGHT, 14.)
                    .text_color(TEXT.solid())
                    .with_transformation(pose),
            )
            .child(title);
        let body = unfolded.then(|| {
            let mut inner = Self {
                id: ElementId::NamedChild(Arc::new(self.id.clone()), inside.into()),
                parts: 0,
                ink: self.ink,
                prose: self.prose,
            };
            let column = inner.column(blocks, depth, false, window, cx);
            column.id(inner.id).ml(px(20.)).mt(px(GAP))
        });
        div().flex().flex_col().child(head).children(body)
    }

    fn block(&mut self, block: &Block, depth: usize, window: &mut Window, cx: &mut App) -> Div {
        match block {
            Block::Heading(level, content) => {
                let size = heading_size(*level);
                let color = if *level < 6 { TEXT } else { TEXT_MUTED };
                let prose = Prose {
                    size,
                    line: heading_line(*level),
                    code: 0.85 * size,
                };
                div()
                    .face(FONT_DISPLAY_BOLD, size)
                    .line_height(px(prose.line))
                    .text_color(color.solid())
                    .child(self.within(prose, |flow| {
                        flow.text(content, FONT_DISPLAY_BOLD, FONT_DISPLAY_BOLD, window, cx)
                    }))
                    .when(*level <= 2, |heading| {
                        heading
                            .pb(px(0.3 * size))
                            .border_b_1()
                            .border_color(BORDER.solid())
                    })
            }
            Block::Paragraph(content) => {
                self.text(content, FONT_BODY, FONT_BODY_SEMIBOLD, window, cx)
            }
            Block::List {
                start,
                tight,
                items,
            } => self.list(*start, *tight, items, depth, window, cx),
            Block::Quote(blocks) => self
                .column(blocks, depth, false, window, cx)
                .border_l_3()
                .border_color(BORDER.solid())
                .pl(px(12.)),
            Block::Alert(kind, blocks) => {
                let [ground, ink, link, edge] = chrome(*kind);
                let look = Ink {
                    link,
                    strong: None,
                    code: [
                        ground.mix(ink, 0.3),
                        ground.mix(ink, 0.15),
                        ink.mix(contrast(), 0.55),
                    ],
                    lined: true,
                };
                let outer = std::mem::replace(&mut self.ink, look);
                let body = self.column(blocks, depth, false, window, cx);
                self.ink = outer;
                let note = body
                    .rounded_r(px(4.))
                    .border_l_3()
                    .border_color(edge.solid())
                    .bg(ground.solid())
                    .px(px(0.4 * REM))
                    .py(px(0.35 * REM))
                    .text_color(ink.solid());
                div().flex().child(note)
            }
            Block::Code(source) => div()
                .rounded(px(6.))
                .border_1()
                .border_color(BORDER.solid())
                .bg(SURFACE_2.solid())
                .px(px(12.))
                .py(px(8.))
                .face(FONT_MONO, PRE)
                .line_height(px(PRE_LINE * PRE))
                .text_color(TEXT.solid())
                .child(source.clone()),
            Block::Rule => div().h(px(1.)).bg(BORDER.solid()),
            Block::Table { align, rows } => self.table(align, rows, window, cx),
            Block::Details {
                summary,
                open,
                blocks,
            } => self.details(summary, *open, blocks, depth, window, cx),
            Block::Footnotes(notes) => {
                let mut list = div()
                    .flex()
                    .flex_col()
                    .border_t_1()
                    .border_color(BORDER.solid())
                    .pt(px(GAP))
                    .pl(px(INDENT))
                    .text_size(px(NOTE.size))
                    .line_height(px(NOTE.line));
                for (index, note) in notes.iter().enumerate() {
                    let body =
                        self.within(NOTE, |flow| flow.column(note, depth + 1, false, window, cx));
                    let row = div()
                        .mt(px(if index > 0 { ITEM_GAP } else { 0. }))
                        .child(marker(Some(1), index, 0, NOTE.line))
                        .child(body);
                    list = list.child(row);
                }
                list
            }
        }
    }
}

pub fn notes(
    id: impl Into<ElementId>,
    blocks: &[Block],
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let mut flow = Flow {
        id: id.into(),
        parts: 0,
        ink: Ink {
            link: LINK.srgb(),
            strong: Some(TEXT.srgb()),
            code: [BORDER.srgb(), SURFACE_2.srgb(), TEXT.srgb()],
            lined: false,
        },
        prose: BODY,
    };
    flow.column(blocks, 0, true, window, cx)
        .id(flow.id)
        .face(FONT_BODY, TEXT_UI)
        .line_height(px(LINE))
        .text_color(TEXT_MUTED.solid())
}

fn hover(
    over: &mut Option<(usize, ElementId)>,
    link: usize,
    piece: &ElementId,
    inside: bool,
) -> bool {
    let left = !inside && over.as_ref().is_some_and(|(_, at)| at == piece);
    if inside {
        *over = Some((link, piece.clone()));
    } else if left {
        *over = None;
    }
    inside || left
}

#[cfg(test)]
mod tests {
    use gpui::{Modifiers, TestAppContext, point};

    use super::*;
    use crate::ui::testing::mount;

    const SOURCE: &str = "see [docs](https://example.com/docs) or #7";
    const FOLDED: &str = concat!(
        "<details><summary>First</summary>\n\nsee #7\n\n</details>\n\n",
        "<details><summary>Second</summary>\n\n[docs](https://example.com/docs)\n\n</details>",
    );

    #[gpui::test]
    fn an_unfolded_block_stays_open_when_an_earlier_one_unfolds(cx: &mut TestAppContext) {
        let cx = mount(cx, |window, cx| {
            let blocks = parse(FOLDED, "owner/repo");
            div()
                .w(px(400.))
                .child(notes("probe", &blocks, window, cx))
                .into_any_element()
        });
        let row = LINE + GAP;
        let places = [
            (30., row + LINE / 2.),
            (30., LINE / 2.),
            (20. + 0.9 * TEXT_UI, 3. * row + LINE / 2.),
        ];
        for (x, y) in places {
            cx.simulate_click(point(px(x), px(y)), Modifiers::default());
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/docs"));
    }

    #[gpui::test]
    fn links_in_notes_open_where_they_point(cx: &mut TestAppContext) {
        let cx = mount(cx, |window, cx| {
            let blocks = parse(&format!("- one\n- {SOURCE}"), "owner/repo");
            div()
                .w(px(400.))
                .child(notes("probe", &blocks, window, cx))
                .into_any_element()
        });
        let char = 0.6 * TEXT_UI;
        let second = px(LINE + ITEM_GAP + LINE / 2.);
        let at = |index: f32| point(px(INDENT + index * char), second);
        cx.simulate_click(at(1.5), Modifiers::default());
        assert_eq!(cx.opened_url(), None);
        cx.simulate_click(at(5.5), Modifiers::default());
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/docs"));
        cx.simulate_click(at(12.5), Modifiers::default());
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("https://github.com/owner/repo/issues/7")
        );
    }

    #[test]
    fn a_link_stays_hovered_while_the_pointer_moves_across_its_words() {
        let (first, second) = (
            ElementId::from(("piece", 1usize)),
            ElementId::from(("piece", 2usize)),
        );
        let mut over = None;
        hover(&mut over, 0, &first, true);
        hover(&mut over, 0, &second, true);
        hover(&mut over, 0, &first, false);
        assert_eq!(over, Some((0, second.clone())));
        hover(&mut over, 0, &second, false);
        hover(&mut over, 0, &first, true);
        assert_eq!(over, Some((0, first.clone())));
        hover(&mut over, 0, &first, false);
        assert_eq!(over, None);
    }

    #[test]
    fn a_space_inside_a_link_stays_with_the_word_before_it() {
        let blocks = parse(
            "first\\\nsee [the full guide](https://x.com) now",
            "owner/repo",
        );
        let [Block::Paragraph(content)] = blocks.as_slice() else {
            panic!("paragraph expected, got {blocks:?}");
        };
        let spaces = bits(content).into_iter().filter_map(|bit| match bit {
            Bit::Word(space, _) => Some(space),
            Bit::Break => None,
        });
        assert_eq!(
            spaces.collect::<Vec<_>>(),
            [
                Space::None,
                Space::Free,
                Space::Bound,
                Space::Bound,
                Space::Free,
                Space::None
            ]
        );
    }

    #[test]
    fn nested_numbers_count_in_roman_numerals_and_then_in_letters() {
        let counted = |depth: usize| [1, 4, 9, 14, 27].map(|number| ordinal(number, depth));
        assert_eq!(counted(0), ["1", "4", "9", "14", "27"]);
        assert_eq!(counted(1), ["i", "iv", "ix", "xiv", "xxvii"]);
        assert_eq!(counted(2), ["a", "d", "i", "n", "aa"]);
    }
}
