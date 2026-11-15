use crate::ui::px;
use gpui::{
    Context, Div, InteractiveElement, ParentElement, SharedString, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder,
};

use super::{LiberatorPage, Mark, column_item};
use crate::model::liberator::Node;
use crate::ui::kit::panel;
use crate::ui::scroll::scroll;
use crate::ui::theme::BORDER;

struct Entry {
    text: SharedString,
    id: SharedString,
    branch: bool,
}

pub(super) fn active_path(roots: &[Node], path: &[usize]) -> Vec<usize> {
    if path.is_empty() && roots.first().is_some_and(|root| !root.children.is_empty()) {
        vec![0]
    } else {
        path.to_vec()
    }
}

pub(super) fn columns<'a>(roots: &'a [Node], active: &[usize]) -> Vec<&'a [Node]> {
    let mut columns = vec![roots];
    let mut current = roots;
    for index in active {
        match current.get(*index) {
            Some(next) if !next.children.is_empty() => {
                current = &next.children;
                columns.push(current);
            }
            _ => break,
        }
    }
    columns
}

impl LiberatorPage {
    pub(super) fn playlist(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<Div> {
        let (lists, active, picked) = {
            let liberator = &self.launcher.read(cx).liberator;
            let roots = liberator.tree()?;
            let active = active_path(roots, liberator.path());
            let entry = |node: &Node| Entry {
                text: node.text.clone(),
                id: node.id.clone(),
                branch: !node.children.is_empty(),
            };
            let lists: Vec<Vec<Entry>> = columns(roots, &active)
                .into_iter()
                .map(|nodes| nodes.iter().map(entry).collect())
                .collect();
            (lists, active, liberator.picked().clone())
        };
        let last = lists.len() - 1;
        let mut row = div()
            .id("playlist-columns")
            .flex()
            .h_full()
            .overflow_x_scroll();
        for (col, entries) in lists.into_iter().enumerate() {
            let name = ("playlist-column", col);
            let items = window.with_element_namespace(name, |window| {
                let item = |(index, entry): (usize, Entry)| {
                    let Entry { text, id, branch } = entry;
                    let mark = if !branch && id == picked {
                        Mark::Chosen
                    } else if branch && active.get(col) == Some(&index) {
                        Mark::Active
                    } else {
                        Mark::Idle
                    };
                    let (launcher, mut path) = (self.launcher.clone(), active[..col].to_vec());
                    if branch {
                        path.push(index);
                    }
                    column_item(
                        ("playlist-item", index).into(),
                        text,
                        mark,
                        branch,
                        window,
                        cx,
                    )
                    .on_click(move |_, _, cx| {
                        launcher.update(cx, |launcher, cx| {
                            launcher.liberator.open(path.clone(), cx);
                            if !branch {
                                launcher.liberator.set_playlist(&id, cx);
                            }
                        });
                    })
                };
                entries
                    .into_iter()
                    .enumerate()
                    .map(item)
                    .collect::<Vec<_>>()
            });
            let list = scroll(name, window, cx);
            let column = div()
                .flex_1()
                .min_w(px(150.))
                .p(px(8.))
                .when(col < last, |column| {
                    column.border_r_1().border_color(BORDER.solid())
                })
                .children(items);
            row = row.child(list.keys(list.area(column)));
        }
        Some(panel().h_full().overflow_hidden().child(row))
    }
}
