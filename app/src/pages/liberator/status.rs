use crate::ui::px;
use backend::settings::Flag;
use gpui::{App, Div, ParentElement, Styled, Transformation, Window, div, percentage};

use crate::model::Launcher;
use crate::model::liberator::State;
use crate::ui::icon::{RING, RING_ARC, UNLOCK, WARNING, icon};
use crate::ui::motion;
use crate::ui::theme::{BORDER, NOTICE_EDGE, SUCCESS, TEXT};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Status {
    Loading,
    Unlocked,
    Problem,
}

pub(super) fn status(state: &State, enabled: bool, available: bool) -> Option<Status> {
    if !enabled || (available && !state.attached) {
        return None;
    }
    Some(if !available || state.unsupported {
        Status::Problem
    } else if state.unlocked {
        Status::Unlocked
    } else {
        Status::Loading
    })
}

pub(super) fn status_icon(status: Status, size: f32, window: &mut Window, cx: &mut App) -> Div {
    let holder = div().flex().flex_none();
    match status {
        Status::Problem => holder.child(icon(WARNING, size).text_color(NOTICE_EDGE.solid())),
        Status::Unlocked => holder.child(icon(UNLOCK, size).text_color(SUCCESS.solid())),
        Status::Loading => {
            let turn = motion::clock("liberator-ring", window, cx)
                .as_secs_f32()
                .fract();
            window.request_animation_frame();
            let arc = icon(RING_ARC, size)
                .absolute()
                .text_color(TEXT.solid())
                .with_transformation(Transformation::rotate(percentage(turn)));
            holder
                .size(px(size))
                .child(icon(RING, size).absolute().text_color(BORDER.solid()))
                .child(arc)
        }
    }
}

pub fn loading_ring(window: &mut Window, cx: &mut App) -> Option<Div> {
    let launcher = Launcher::handle(cx);
    let launcher = launcher.read(cx);
    let enabled = launcher.settings.flag(Flag::LiberatorEnabled);
    let loading = status(launcher.liberator.state(), enabled, true) == Some(Status::Loading);
    loading.then(|| status_icon(Status::Loading, 16., window, cx))
}
