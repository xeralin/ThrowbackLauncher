use crate::ui::{px, units};
use backend::manifest::SeasonEntry;
use chrono::{Local, TimeZone};
use gpui::{Div, IntoElement, ParentElement, Pixels, Styled, div, prelude::FluentBuilder};

use super::manage::format_playtime;
use crate::model::launch::PlaytimeEntry;
use crate::ui::theme::{
    BORDER, FONT_DISPLAY_BOLD, FONT_MONO, REM, SURFACE_2, Srgb, TEXT, TEXT_LABEL, TEXT_MUTED,
    Typeface,
};

const COMPACT: f32 = 768.;
const FAINT: f32 = 0.15;
const DEPTH: f32 = 0.35;

fn label(code: &str, hm: bool) -> String {
    if hm {
        format!("{code} HM")
    } else {
        code.to_owned()
    }
}

fn year(season: &SeasonEntry) -> &str {
    season.code.split('S').next().unwrap_or_default()
}

fn rows(seasons: &[SeasonEntry]) -> Vec<Vec<(&SeasonEntry, bool)>> {
    let years = seasons.chunk_by(|one, next| year(one) == year(next));
    let plain = |season| (season, false);
    let mut rows: Vec<Vec<_>> = years.map(|year| year.iter().map(plain).collect()).collect();
    let heated = seasons.iter().filter(|season| season.hm_available);
    rows.push(heated.map(|season| (season, true)).collect());
    rows
}

fn corner_free<T>(rows: &[Vec<T>], columns: u16) -> bool {
    let lines = rows.iter().flat_map(|row| row.chunks(columns.into()));
    lines.rev().take(2).all(|line| line.len() < columns.into())
}

fn since(seconds: i64) -> String {
    let day = Local.timestamp_opt(seconds, 0).single();
    day.map(|day| day.format("%b %-d, %Y").to_string())
        .unwrap_or_default()
}

fn stat(name: &'static str, value: String) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .h(px(4.125 * REM))
        .rounded(px(6.))
        .border_1()
        .border_color(BORDER.solid())
        .bg(SURFACE_2.solid())
        .px(px(8.))
        .pt(px(7.))
        .child(
            div()
                .face(FONT_DISPLAY_BOLD, 1.6 * REM)
                .line_height(px(1.625 * REM))
                .text_color(TEXT.solid())
                .whitespace_nowrap()
                .child(value),
        )
        .child(
            div()
                .face(FONT_MONO, TEXT_LABEL)
                .line_height(px(TEXT_LABEL))
                .text_color(TEXT_MUTED.solid())
                .child(name),
        )
}

pub fn overview(
    entries: &[PlaytimeEntry],
    seasons: &[SeasonEntry],
    viewport: Pixels,
    action: Srgb,
    corner: impl IntoElement,
) -> Div {
    let compact = units(viewport) < COMPACT;
    let columns = if compact { 4 } else { 5 };
    let total: u64 = entries.iter().map(|entry| entry.minutes).sum();
    let sessions: u64 = entries.iter().map(|entry| entry.sessions).sum();
    let named = |entry: &PlaytimeEntry| label(&entry.code, entry.edition.hm);
    let last = entries.iter().max_by_key(|entry| entry.last_played);
    let first = entries.iter().map(|entry| entry.first_played).min();
    let stats = div()
        .grid()
        .grid_cols(if compact { 2 } else { 4 })
        .gap(px(8.))
        .child(stat("All time", format_playtime(total)))
        .child(stat("Sessions", sessions.to_string()))
        .child(stat("Last played", last.map(named).unwrap_or_default()))
        .child(stat("Playing since", first.map(since).unwrap_or_default()));
    let peak = entries.iter().map(|entry| entry.minutes).max();
    let peak = peak.unwrap_or_default().max(1);
    let rows = rows(seasons);
    let free = corner_free(&rows, columns);
    let timeline = rows.into_iter().map(|row| {
        let cells = row.into_iter().map(|(season, hm)| {
            let mut played = entries.iter();
            let played = played.find(|entry| entry.edition.is(&season.key, hm));
            let played = played.map_or(0, |entry| entry.minutes);
            let share = played as f32 / peak as f32;
            let (fill, ink) = if played > 0 {
                (SURFACE_2.mix(action, FAINT + DEPTH * share), TEXT)
            } else {
                (SURFACE_2.srgb(), TEXT_MUTED)
            };
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(4.))
                .h(px(20.))
                .rounded(px(2.))
                .bg(fill.solid())
                .px(px(6.))
                .face(FONT_MONO, TEXT_LABEL)
                .line_height(px(TEXT_LABEL))
                .text_color(ink.solid())
                .child(label(&season.code, hm))
                .when(played > 0 && !compact, |cell| {
                    cell.child(div().whitespace_nowrap().child(format_playtime(played)))
                })
        });
        div().grid().grid_cols(columns).gap(px(2.)).children(cells)
    });
    let timeline = div().flex().flex_col().gap(px(4.)).children(timeline);
    let corner = div().flex().justify_end().child(corner);
    let timeline = if free {
        timeline.child(corner.absolute().right_0().bottom_0())
    } else {
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(timeline)
            .child(corner)
    };
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(stats)
        .child(timeline)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn season(code: &str, hm_available: bool) -> SeasonEntry {
        SeasonEntry {
            key: format!("{code}_Name"),
            id: format!("{code}_Name"),
            hm: false,
            code: code.to_owned(),
            name: "Name".to_owned(),
            size_gb: 0.,
            build: String::new(),
            hm_available,
            hm_beta: false,
            partial: false,
        }
    }

    #[test]
    fn each_year_is_a_row_and_heated_metal_editions_follow() {
        let seasons = [
            season("Y1S0", false),
            season("Y1S1", false),
            season("Y5S3", true),
            season("Y10S1", false),
            season("Y9S2", true),
        ];
        let rows = rows(&seasons).into_iter();
        let named = |row: Vec<(&SeasonEntry, bool)>| -> Vec<String> {
            let cells = row.into_iter();
            cells.map(|(season, hm)| label(&season.code, hm)).collect()
        };
        assert_eq!(
            rows.map(named).collect::<Vec<_>>(),
            [
                vec!["Y1S0", "Y1S1"],
                vec!["Y5S3"],
                vec!["Y10S1"],
                vec!["Y9S2"],
                vec!["Y5S3 HM", "Y9S2 HM"],
            ]
        );
    }

    #[test]
    fn the_corner_is_free_while_the_last_two_lines_leave_their_last_column() {
        let row = |cells: usize| vec![String::new(); cells];
        let today = [row(5), row(4), row(2), row(3)];
        assert!(corner_free(&today, 5) && corner_free(&today, 4));
        let grown = [row(5), row(4), row(4), row(3)];
        assert!(corner_free(&grown, 5) && !corner_free(&grown, 4));
        assert!(!corner_free(&[row(4), row(5)], 5));
        assert!(!corner_free(&[row(4), row(7)], 5));
        assert!(corner_free(&[row(3)], 5) && corner_free::<String>(&[], 5));
    }

    #[test]
    fn a_heated_metal_edition_is_named_with_its_suffix() {
        assert_eq!(label("Y9S2", false), "Y9S2");
        assert_eq!(label("Y9S2", true), "Y9S2 HM");
    }
}
