use backend::env::IS_WINDOWS;
use gpui::{
    App, ClipboardItem, Div, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div,
};

use crate::model::Launcher;
use crate::ui::button::copy_button;
use crate::ui::icon::{PROTON, icon};
use crate::ui::kit::{list_row, tip, truncate};
use crate::ui::prose::{BODY, STEP, inline, step_box, steps};
use crate::ui::px;
use crate::ui::theme::{FONT_BODY_SEMIBOLD, REM, TEXT, Typeface};

const EXCLUSION: [&str; 3] = [
    "Search for **Virus & threat protection** in the Windows Start menu",
    "Click **Manage settings** under Virus & threat protection settings",
    "Scroll down to Exclusions and click **Add or remove exclusions**",
];
const ARCHIVE: &str =
    "Download the latest `Unstable.7z` from [`#indev-releases`](https://discord.gg/7mR9VxBxWd).";
const PROTON_STEPS: [&str; 2] = [
    "Download the Proton build from [`#indev-releases`](https://discord.gg/7mR9VxBxWd)",
    "Extract it into `~/.local/share/ThrowbackLauncher/bin/proton`",
];

fn exclusion_steps(paths: Vec<SharedString>, window: &mut Window, cx: &mut App) -> Div {
    let paste = if paths.len() > 1 {
        "Click **Add an exclusion** > **Folder** and paste the paths below"
    } else {
        "Click **Add an exclusion** > **Folder** and paste the path below"
    };
    let list = EXCLUSION.into_iter().chain([paste]).enumerate();
    let list: Vec<_> = list
        .map(|(index, step)| {
            inline(("exclusion", index), step, STEP, window, cx).into_any_element()
        })
        .collect();
    let rows = paths.into_iter().enumerate().map(|(index, path)| {
        let text = path.clone();
        let copy = move |_: &mut Window, cx: &mut App| {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
            true
        };
        let copy = copy_button(("copy-path", index), copy, window, cx);
        let action = div().flex().flex_none().items_center().mr(px(-4.));
        let label = div().id(("exclusion-path", index)).flex_1().min_w_0();
        let label = label.child(truncate(path.clone())).tooltip(tip(path));
        list_row().child(label).child(action.child(copy))
    });
    let rows = div().flex().flex_col().gap(px(8.)).children(rows);
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .max_w_full()
        .child(step_box().child(steps(list)))
        .child(rows)
}

pub fn exclusion(
    sentence: Div,
    library: Option<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let folder = Launcher::handle(cx)
        .read(cx)
        .paths
        .data_root
        .to_string_lossy();
    let folder = SharedString::from(folder.into_owned());
    let paths = std::iter::once(folder).chain(library).collect();
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(sentence)
        .child(exclusion_steps(paths, window, cx))
}

fn proton_steps(window: &mut Window, cx: &mut App) -> Div {
    let intro = "**Y9S2 Heated Metal** only runs on a specific Proton build.";
    let mut list: Vec<_> = PROTON_STEPS
        .into_iter()
        .enumerate()
        .map(|(index, step)| {
            inline(("proton-step", index), step, STEP, window, cx).into_any_element()
        })
        .collect();
    let mark = div()
        .flex()
        .items_center()
        .gap(px(0.2 * STEP.size))
        .face(FONT_BODY_SEMIBOLD, STEP.size)
        .line_height(px(STEP.line))
        .text_color(TEXT.solid())
        .child(icon(PROTON, 1.15 * STEP.size).text_color(TEXT.solid()))
        .child("Proton");
    let pick = div()
        .flex()
        .gap(px(0.2 * STEP.size))
        .child(inline("proton-pick", "Pick it under", STEP, window, cx))
        .child(mark)
        .child(inline(
            "proton-tab",
            "in the **Manage** tab",
            STEP,
            window,
            cx,
        ));
    list.push(pick.into_any_element());
    step_box()
        .child(inline("proton-intro", intro, STEP, window, cx).mb(px(0.3 * REM)))
        .child(steps(list))
}

pub fn archive_steps(window: &mut Window, cx: &mut App) -> Div {
    let body = div().flex().flex_col().gap(px(12.)).child(inline(
        "select-archive",
        ARCHIVE,
        BODY,
        window,
        cx,
    ));
    if IS_WINDOWS {
        body
    } else {
        body.child(proton_steps(window, cx))
    }
}
