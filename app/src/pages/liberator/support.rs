use crate::ui::px;
use gpui::{
    Context, Div, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled, Window,
    div, prelude::FluentBuilder,
};

use super::{Mark, column_item};
use crate::data::liberator_builds::{SeasonRow, years};
use crate::ui::kit::{box_text, code_chip, panel};
use crate::ui::prose::{STEP, inline, step_box, steps};
use crate::ui::scroll::scroll;
use crate::ui::theme::{BORDER, FONT_BODY, REM, TEXT_MUTED, Typeface};

const GUIDE: [&str; 3] = [
    "Create a local custom game",
    "Select the game mode in the **Playlist** tab",
    "For any PvE mode, make sure all players are on the **blue team**, then start the match",
];
const GAP: f32 = 16.;
const BELOW: f32 = 20.;
const ROW: f32 = 32.;
const EVENT: f32 = 0.78 * REM;

fn season_rows(rows: &'static [SeasonRow]) -> Div {
    let events = rows.iter().any(|row| !row.events.is_empty());
    let mut grid = div().grid().grid_cols_max_content(2 + u16::from(events));
    for (index, row) in rows.iter().enumerate() {
        let cell = || {
            div()
                .flex()
                .items_center()
                .h(px(ROW))
                .px(px(0.6 * REM))
                .border_color(BORDER.solid())
                .when(index > 0, |cell| cell.border_t_1())
        };
        let name = format!("{} {}", row.season, row.operation);
        grid = grid
            .child(cell().border_r_1().child(box_text(name)))
            .when(events, |grid| {
                let names = div()
                    .face(FONT_BODY, EVENT)
                    .text_color(TEXT_MUTED.solid())
                    .whitespace_nowrap()
                    .child(row.events.join(", "));
                grid.child(cell().border_r_1().child(names))
            })
            .child(cell().child(code_chip(row.build)));
    }
    panel().mb(px(BELOW)).child(grid)
}

#[derive(Default)]
pub(super) struct Support {
    pub(super) year: usize,
}

impl Render for Support {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = panel()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(150.))
            .mb(px(BELOW))
            .p(px(8.));
        let mut tables = div().grid();
        for (index, (year, rows)) in years().enumerate() {
            let shown = index == self.year;
            let mark = if shown { Mark::Active } else { Mark::Idle };
            let id = ("support-year", index).into();
            let label = format!("Year {year}").into();
            let item = column_item(id, label, mark, true, window, cx);
            list = list.child(item.on_click(cx.listener(move |support, _, _, cx| {
                support.year = index;
                cx.notify();
            })));
            let place = div().col_start(1).row_start(1).flex().items_start();
            let seasons = season_rows(rows);
            tables = tables.child(place.when(!shown, Div::invisible).child(seasons));
        }
        let guide = GUIDE.into_iter().enumerate();
        let guide: Vec<_> = guide
            .map(|(index, step)| {
                inline(("guide", index), step, STEP, window, cx).into_any_element()
            })
            .collect();
        let content = div()
            .flex()
            .flex_wrap()
            .items_start()
            .justify_between()
            .gap_x(px(GAP))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_start()
                    .gap_x(px(GAP))
                    .child(list)
                    .child(tables),
            )
            .child(step_box().max_w(px(280.)).mb(px(BELOW)).child(steps(guide)));
        let area = scroll("support", window, cx);
        area.keys(area.area(div().size_full().child(content)))
    }
}
