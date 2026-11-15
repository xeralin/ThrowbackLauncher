use std::sync::Arc;

use gpui::{
    AnyElement, App, Div, ElementId, Font, FontStyle, Hsla, IntoElement, ParentElement, Pixels,
    SharedString, Stateful, Styled, TextRun, Window, div, prelude::FluentBuilder,
};

use crate::ui::kit::external;
use crate::ui::motion::{self, FAST};
use crate::ui::px;
use crate::ui::theme::{
    BORDER, FONT_BODY, FONT_BODY_SEMIBOLD, FONT_MONO, Face, LINK, REM, SURFACE_2, Srgb, TEXT,
    TEXT_BODY, TEXT_MUTED, Typeface,
};

#[derive(Clone, Copy)]
pub struct Prose {
    pub size: f32,
    pub line: f32,
    pub code: f32,
}

pub const BODY: Prose = Prose {
    size: TEXT_BODY,
    line: 26.,
    code: 0.85 * TEXT_BODY,
};
pub const STEP: Prose = Prose {
    size: 0.78 * REM,
    line: 18.,
    code: 0.85 * 0.78 * REM,
};

pub fn link(
    id: impl Into<ElementId>,
    content: impl IntoElement,
    url: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let id = id.into();
    let hover = motion::hover(&id, FAST, window, cx);
    let face = div()
        .rounded(px(6.))
        .cursor_pointer()
        .text_color(LINK.solid())
        .when(hover.over, |link| link.underline())
        .child(content);
    external(id, face, url, &hover, window, cx)
}

#[derive(Debug, PartialEq)]
struct Span<'a> {
    text: &'a str,
    bold: bool,
    code: bool,
    link: Option<&'a str>,
}

fn anchor(text: &str) -> Option<(&str, &str, &str)> {
    let (label, rest) = text.strip_prefix('[')?.split_once("](")?;
    let (url, rest) = rest.split_once(')')?;
    Some((label, url, rest))
}

fn quoted(text: &str) -> Option<(&str, &str)> {
    text.strip_prefix('`')?.split_once('`')
}

fn spans(text: &str) -> Vec<Span<'_>> {
    let (mut spans, mut rest, mut bold) = (Vec::new(), text, false);
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("**") {
            bold = !bold;
            rest = after;
        } else if let Some((text, after)) = quoted(rest) {
            spans.push(Span {
                text,
                bold,
                code: true,
                link: None,
            });
            rest = after;
        } else if let Some((label, url, after)) = anchor(rest) {
            let code = quoted(label).map(|(code, _)| code);
            spans.push(Span {
                text: code.unwrap_or(label),
                bold,
                code: code.is_some(),
                link: Some(url),
            });
            rest = after;
        } else {
            let mut letters = rest.char_indices().skip(1);
            let end = letters.find(|(_, letter)| matches!(letter, '*' | '`' | '['));
            let end = end.map_or(rest.len(), |(index, _)| index);
            spans.push(Span {
                text: &rest[..end],
                bold,
                code: false,
                link: None,
            });
            rest = &rest[end..];
        }
    }
    spans
}

fn words<'a>(spans: &'a [Span<'a>]) -> Vec<Vec<(&'a str, &'a Span<'a>)>> {
    let (mut words, mut word) = (Vec::new(), Vec::new());
    for span in spans {
        if span.code || span.link.is_some() {
            word.push((span.text, span));
            continue;
        }
        for (index, part) in span.text.split(' ').enumerate() {
            if index > 0 {
                words.push(std::mem::take(&mut word));
            }
            if !part.is_empty() {
                word.push((part, span));
            }
        }
    }
    words.push(word);
    words.retain(|word| !word.is_empty());
    words
}

pub fn code(content: &str, prose: Prose, edge: Srgb, ground: Srgb) -> Div {
    let ascent = (0.885 * prose.code).round();
    let descent = (0.242 * prose.code).round();
    let baseline = (prose.line - 1.2 * prose.size) / 2. + prose.size;
    let chip = div()
        .mt(px(baseline - ascent - 0.18 * prose.code - 1.))
        .rounded(px(6.))
        .border_1()
        .border_color(edge.solid())
        .bg(ground.solid())
        .px(px(0.15 * prose.code))
        .pt(px(0.18 * prose.code))
        .pb(px(0.12 * prose.code))
        .face(FONT_MONO, prose.code)
        .line_height(px(ascent + descent))
        .whitespace_nowrap()
        .child(content.to_owned());
    div().flex_none().h(px(prose.line)).child(chip)
}

fn advance(word: &str, face: Face, slanted: bool, size: f32, window: &Window) -> Pixels {
    let style = if slanted {
        FontStyle::Italic
    } else {
        FontStyle::Normal
    };
    let run = TextRun {
        len: word.len(),
        font: Font {
            family: face.family(),
            weight: face.weight(),
            style,
            ..Font::default()
        },
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let word = SharedString::new(word);
    let line = window
        .text_system()
        .shape_line(word, px(size), &[run], None);
    line.width
}

pub struct Spacing {
    size: f32,
    space: Pixels,
    pub gap: Pixels,
    over: Pixels,
}

impl Spacing {
    pub fn new(face: Face, size: f32, window: &Window) -> Self {
        let space = advance(" ", face, false, size, window);
        let pixel = gpui::px(1.) / window.scale_factor();
        Self {
            size,
            space,
            gap: pixel * (space / pixel).floor(),
            over: Pixels::ZERO,
        }
    }

    pub fn part(&mut self, part: &str, face: Face, slanted: bool, window: &Window) -> Pixels {
        let width = advance(part, face, slanted, self.size, window);
        let snapped = window.pixel_snap(width);
        self.over = width - snapped;
        snapped
    }

    pub fn rest(&self, window: &Window) -> Pixels {
        window.pixel_snap(self.over + self.space) - self.gap
    }
}

pub fn inline(
    id: impl Into<ElementId>,
    text: &str,
    prose: Prose,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let id = id.into();
    let spans = spans(text);
    let mut spacing = Spacing::new(FONT_BODY, prose.size, window);
    let mut flow = div()
        .flex()
        .flex_wrap()
        .gap_x(spacing.gap)
        .face(FONT_BODY, prose.size)
        .line_height(px(prose.line))
        .text_color(TEXT_MUTED.solid());
    let words = words(&spans);
    let count = words.len();
    for (index, word) in words.into_iter().enumerate() {
        let mut parts = Vec::with_capacity(word.len());
        for (part, span) in word {
            let piece = if span.code {
                code(part, prose, BORDER.srgb(), SURFACE_2.srgb())
            } else {
                let face = if span.bold {
                    FONT_BODY_SEMIBOLD
                } else {
                    FONT_BODY
                };
                div()
                    .flex_none()
                    .w(spacing.part(part, face, false, window))
                    .whitespace_nowrap()
                    .child(part.to_owned())
            };
            let piece = piece.when(span.bold, |piece| {
                piece
                    .face(FONT_BODY_SEMIBOLD, prose.size)
                    .line_height(px(prose.line))
            });
            let Some(url) = span.link else {
                let lit = span.bold || span.code;
                let piece = piece.when(lit, |piece| piece.text_color(TEXT.solid()));
                parts.push(piece.into_any_element());
                continue;
            };
            let url = SharedString::from(url.to_owned());
            let place = ElementId::NamedChild(Arc::new(id.clone()), url.clone());
            parts.push(link(place, piece, url, window, cx).into_any_element());
        }
        let rest = if index + 1 < count {
            spacing.rest(window)
        } else {
            Pixels::ZERO
        };
        flow = flow.child(div().flex().mr(rest).children(parts));
    }
    flow
}

pub fn step_box() -> Div {
    div()
        .rounded(px(8.))
        .border_1()
        .border_color(BORDER.solid())
        .px(px(0.6 * REM))
        .py(px(0.45 * REM))
        .face(FONT_BODY, STEP.size)
        .line_height(px(STEP.line))
        .text_color(TEXT_MUTED.solid())
}

pub fn steps(items: impl IntoIterator<Item = AnyElement>) -> Div {
    let rows = items.into_iter().enumerate().map(|(index, item)| {
        let marker = div()
            .flex()
            .flex_none()
            .justify_end()
            .w(px(1.15 * REM))
            .child(format!("{}. ", index + 1));
        div()
            .flex()
            .child(marker)
            .child(div().min_w_0().child(item))
    });
    div().flex().flex_col().gap(px(0.3 * REM)).children(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> Span<'_> {
        Span {
            text,
            bold: false,
            code: false,
            link: None,
        }
    }

    #[test]
    fn rows_split_into_bold_code_and_links() {
        assert_eq!(
            spans("DP27 light machine gun"),
            [plain("DP27 light machine gun")]
        );
        assert_eq!(
            spans("**Claymore** for attackers, **Impact Grenade** — both"),
            [
                Span {
                    bold: true,
                    ..plain("Claymore")
                },
                plain(" for attackers, "),
                Span {
                    bold: true,
                    ..plain("Impact Grenade")
                },
                plain(" — both"),
            ]
        );
        assert_eq!(
            spans("Get `Unstable.7z` from [`#indev-releases`](https://discord.gg/7mR9VxBxWd)"),
            [
                plain("Get "),
                Span {
                    code: true,
                    ..plain("Unstable.7z")
                },
                plain(" from "),
                Span {
                    code: true,
                    link: Some("https://discord.gg/7mR9VxBxWd"),
                    ..plain("#indev-releases")
                },
            ]
        );
        assert_eq!(
            spans("R6S **SDK** by [DataCluster0](https://github.com/DataCluster0/HeatedMetal)")[3],
            Span {
                link: Some("https://github.com/DataCluster0/HeatedMetal"),
                ..plain("DataCluster0")
            }
        );
        assert_eq!(
            spans("2 * 3 [not a link] `open"),
            [
                plain("2 "),
                plain("* 3 "),
                plain("[not a link] "),
                plain("`open")
            ]
        );
    }

    #[test]
    fn words_keep_links_whole_and_glue_what_touches() {
        let row = spans("See [Visual C++ Redistributable](https://x).");
        let grouped: Vec<Vec<&str>> = words(&row)
            .into_iter()
            .map(|word| word.into_iter().map(|(part, _)| part).collect())
            .collect();
        assert_eq!(
            grouped,
            [vec!["See"], vec!["Visual C++ Redistributable", "."]]
        );
        let row = spans("**Sugar Fright** event");
        assert_eq!(words(&row).len(), 3);
    }
}
