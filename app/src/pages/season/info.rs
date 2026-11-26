use std::borrow::Cow;

use crate::ui::{DATE_FORMAT, px, units};
use backend::env::IS_WINDOWS;
use chrono::DateTime;
use gpui::{
    AnyElement, App, Bounds, Context, Corners, Div, ImgResourceLoader, IntoElement, ObjectFit,
    ParentElement, Pixels, Resource, SharedString, Styled, StyledImage, Window, canvas, div, img,
    point, prelude::FluentBuilder, size,
};

use super::SeasonPage;
use crate::data::liberator_builds::events;
use crate::data::season_info::{self, Map, Operator};
use crate::ui::key_art::shade;
use crate::ui::kit::{box_text, code_chip, panel, truncate};
use crate::ui::prose::{Prose, inline};
use crate::ui::theme::{
    BORDER, FONT_BODY, FONT_DISPLAY_BOLD, ON_ART, REM, SURFACE_2, TEXT, TEXT_MUTED, TEXT_UI,
    Typeface,
};

const REPOSITORY: &str =
    "R6S **SDK** by [DataCluster0](https://github.com/DataCluster0/HeatedMetal)";
const REDISTRIBUTABLE: &str =
    "Install the latest [Visual C++ Redistributable](https://aka.ms/vc14/vc_redist.x64.exe)";
const ARCHIVE: &str = "Get `Unstable.7z` from [`#indev-releases`](https://discord.gg/7mR9VxBxWd)";
const SIDE: f32 = 260.;
const CARD: f32 = 72.;
const CARD_WIDTH: f32 = 280.;
const CARD_GAP: f32 = 12.;
const NAME: f32 = 0.95 * REM;
const PORTRAIT: f32 = 160. / 96.;
const FACE: f32 = 0.2;

const ROW: Prose = Prose {
    size: 0.78 * REM,
    line: 14.,
    code: 0.68 * REM,
};
const GADGET: Prose = Prose {
    size: TEXT_UI,
    line: 1.375 * TEXT_UI,
    code: 0.85 * TEXT_UI,
};

fn portrait(picture: String) -> impl IntoElement {
    let picture = Resource::Embedded(picture.into());
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, (), window, cx| {
            let Some(Ok(image)) = window.use_asset::<ImgResourceLoader>(&picture, cx) else {
                return;
            };
            let height = bounds.size.width * PORTRAIT;
            let top = bounds.top() + (bounds.size.height - height) * FACE;
            let placed = Bounds::new(point(bounds.left(), top), size(bounds.size.width, height));
            let corners = Corners {
                top_left: px(7.),
                bottom_left: px(7.),
                ..Corners::default()
            };
            window
                .paint_image(bounds, placed, corners, image, 0, false)
                .ok();
        },
    )
    .absolute()
    .size_full()
}

fn name(content: impl IntoElement) -> Div {
    div()
        .face(FONT_DISPLAY_BOLD, NAME)
        .line_height(px(19.))
        .text_color(TEXT.solid())
        .child(content)
}

fn operator(index: usize, operator: &Operator, window: &mut Window, cx: &mut App) -> Div {
    let gadget = format!("**{}** — {}", operator.gadget_name, operator.gadget_desc);
    panel()
        .flex()
        .h(px(CARD))
        .overflow_hidden()
        .child(
            div()
                .flex_none()
                .w(px(48.))
                .h_full()
                .rounded_l(px(7.))
                .border_r_1()
                .border_color(BORDER.solid())
                .bg(SURFACE_2.solid())
                .child(portrait(operator.picture())),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .justify_center()
                .gap(px(4.))
                .px(px(12.))
                .child(name(truncate(operator.name)))
                .child(
                    inline(("gadget", index), &gadget, GADGET, window, cx)
                        .min_h(px(2.75 * TEXT_UI)),
                ),
        )
}

fn map(map: &Map) -> Div {
    div()
        .h(px(CARD))
        .rounded(px(8.))
        .border_1()
        .border_color(BORDER.solid())
        .bg(SURFACE_2.solid())
        .child(
            img(SharedString::from(map.picture()))
                .absolute()
                .size_full()
                .rounded(px(7.))
                .object_fit(ObjectFit::Cover),
        )
        .child(shade(7.))
        .child(
            name(map.name)
                .text_color(ON_ART.solid())
                .absolute()
                .left(px(12.))
                .bottom(px(8.)),
        )
}

fn list(
    id: &'static str,
    rows: &[Cow<'static, str>],
    window: &mut Window,
    cx: &mut App,
) -> Option<Div> {
    if rows.is_empty() {
        return None;
    }
    let mut list = panel();
    for (index, row) in rows.iter().enumerate() {
        list = list.child(
            inline((id, index), row, ROW, window, cx)
                .px(px(0.6 * REM))
                .py(px(8.))
                .when(index > 0, |row| {
                    row.border_t_1().border_color(BORDER.solid())
                }),
        );
    }
    Some(list)
}

fn cell(content: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .px(px(0.6 * REM))
        .border_r_1()
        .border_color(BORDER.solid())
        .child(content)
}

fn built(stamp: u32) -> Option<String> {
    let built = DateTime::from_timestamp(i64::from(stamp), 0)?;
    Some(built.format(DATE_FORMAT).to_string())
}

fn occasions(names: &[&str]) -> Option<Cow<'static, str>> {
    let noun = match names.len() {
        0 => return None,
        1 => "event",
        _ => "events",
    };
    let names: Vec<String> = names.iter().map(|name| format!("**{name}**")).collect();
    Some(format!("{} {noun}", names.join(" and ")).into())
}

impl SeasonPage {
    pub(super) fn info(&self, room: Pixels, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let Some(entry) = season_info::entry(&self.season.key) else {
            return div();
        };
        let season = &self.season;
        let mut cards: Vec<AnyElement> = Vec::new();
        type Rows = Vec<Cow<'static, str>>;
        let (highlights, requirements, release): (Rows, Rows, SharedString) = if self.hm {
            let scripting = if season.hm_beta {
                "**Lua** scripting language and in-game map editor"
            } else {
                "**Quarrel** scripting language and in-game map editor"
            };
            let overlays = if IS_WINDOWS {
                "Disable external overlays like **Overwolf**"
            } else {
                "Disable external overlays"
            };
            let requirements = [
                IS_WINDOWS.then_some(REDISTRIBUTABLE),
                (season.key == "Y5S3_ShadowLegacy")
                    .then_some("Set in-game textures to Medium or above"),
                Some(overlays),
                season.hm_beta.then_some(ARCHIVE),
            ];
            let stamp = self.installs.hm.built.filter(|stamp| *stamp > 0);
            let date = stamp.filter(|_| season.hm_beta).and_then(built);
            (
                vec![
                    REPOSITORY.into(),
                    scripting.into(),
                    "Unrestricted **Unlock All** and weapon inspection".into(),
                ],
                requirements.into_iter().flatten().map(Cow::from).collect(),
                date.map_or(entry.release.into(), SharedString::from),
            )
        } else {
            for (index, card) in entry.operators.iter().enumerate() {
                cards.push(operator(index, card, window, cx).into_any_element());
            }
            cards.extend(entry.maps.iter().map(|card| map(card).into_any_element()));
            let rows = entry.highlights.iter().map(|row| Cow::from(*row));
            (
                rows.chain(occasions(events(&season.build))).collect(),
                Vec::new(),
                entry.release.into(),
            )
        };
        let space = units(room) - SIDE - 16.;
        let columns = ((space + CARD_GAP) / (CARD_WIDTH + CARD_GAP))
            .floor()
            .max(1.);
        let cards = div()
            .grid()
            .grid_cols(columns as u16)
            .gap(px(CARD_GAP))
            .min_w_0()
            .children(cards);
        let size = div()
            .face(FONT_BODY, ROW.size)
            .line_height(px(ROW.line))
            .text_color(TEXT_MUTED.solid())
            .whitespace_nowrap()
            .child(format!("{} GB", season.size_gb));
        let facts = panel()
            .flex()
            .flex_none()
            .h(px(32.))
            .child(cell(box_text(release)))
            .child(cell(size).when(season.build.is_empty(), |cell| cell.border_r_0()))
            .when(!season.build.is_empty(), |facts| {
                facts.child(
                    div()
                        .flex()
                        .items_center()
                        .px(px(0.6 * REM))
                        .child(code_chip(season.build.clone())),
                )
            });
        let side = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .children(list("highlight", &highlights, window, cx))
            .children(list("requirement", &requirements, window, cx))
            .child(div().flex().justify_end().child(facts));
        div()
            .flex()
            .items_start()
            .gap(px(16.))
            .mt(px(16.))
            .child(cards.flex_1())
            .child(side.flex_none().w(px(SIDE)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_close_the_highlights_as_one_row() {
        assert_eq!(occasions(&[]), None);
        assert_eq!(
            occasions(&["Sugar Fright"]).as_deref(),
            Some("**Sugar Fright** event")
        );
        assert_eq!(
            occasions(&["Showdown", "Doktor's Curse"]).as_deref(),
            Some("**Showdown** and **Doktor's Curse** events")
        );
    }

    #[test]
    fn a_heated_metal_build_is_dated_in_utc() {
        assert_eq!(built(1_599_696_000).as_deref(), Some("Sep 10, 2020"));
        assert_eq!(built(1_599_782_399).as_deref(), Some("Sep 10, 2020"));
        assert_eq!(built(1_704_067_200).as_deref(), Some("Jan 1, 2024"));
    }
}
