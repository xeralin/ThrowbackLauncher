use crate::ui::px;
use gpui::{
    App, Div, Entity, ParentElement, Stateful, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder,
};

use crate::model::Launcher;
use crate::model::liberator::{Capabilities, Capability, Liberator};
use crate::ui::button::secondary;
use crate::ui::kit::{disable, panel};
use crate::ui::scroll::scroll;
use crate::ui::switch::switch;
use crate::ui::theme::{FONT_BODY, TEXT, TEXT_UI, Typeface};

type Mod = (Capability, &'static str);

pub(super) const PLAYERS: &[Mod] = &[
    (Capability::Deathless, "Deathless Players and Hostage"),
    (Capability::UnlimitedEquip, "Unlimited Equipment"),
    (Capability::UnlimitedAmmo, "Unlimited Ammo"),
];
pub(super) const MATCH: &[Mod] = &[
    (Capability::InfiniteTime, "Infinite Time"),
    (Capability::DisableAi, "Brain-Dead AI"),
    (Capability::DisplayBuild, "Display Build Number"),
];
pub(super) const LOADOUT: &[Mod] = &[
    (Capability::DisablePrimary, "Disable Primary Weapon"),
    (Capability::DisableSecondary, "Disable Secondary Weapon"),
    (Capability::DisablePrimaryGadget, "Disable Primary Gadget"),
    (
        Capability::DisableSecondaryGadget,
        "Disable Secondary Gadget",
    ),
];

pub(super) struct Toggle {
    pub capability: Capability,
    pub label: &'static str,
    pub checked: bool,
    pub usable: bool,
}

pub(super) fn offered(capabilities: &Capabilities) -> bool {
    let mods = [PLAYERS, MATCH, LOADOUT].into_iter().flatten();
    let actions = [Capability::Skip, Capability::EndMatch];
    let mut all = mods.map(|(capability, _)| *capability).chain(actions);
    all.any(|capability| capabilities.has(capability))
}

pub(super) fn toggles(mods: &[Mod], liberator: &Liberator) -> Vec<Toggle> {
    let capabilities = &liberator.state().capabilities;
    let reported = mods
        .iter()
        .filter(|(capability, _)| capabilities.reported(*capability));
    let toggle = |&(capability, label): &Mod| Toggle {
        capability,
        label,
        checked: liberator.switched(capability),
        usable: capabilities.has(capability),
    };
    reported.map(toggle).collect()
}

fn group(
    toggles: Vec<Toggle>,
    launcher: &Entity<Launcher>,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let rows = toggles.into_iter().map(|toggle| {
        let (launcher, capability) = (launcher.clone(), toggle.capability);
        let control = switch(
            ("liberator-mod", capability as usize),
            toggle.checked,
            move |value, _, cx| {
                launcher.update(cx, |launcher, cx| {
                    launcher.liberator.set_mod(capability, value, cx)
                });
            },
            window,
            cx,
        );
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .py(px(6.))
            .child(div().flex_1().child(toggle.label))
            .child(control.when(!toggle.usable, disable))
    });
    panel()
        .p(px(8.))
        .face(FONT_BODY, TEXT_UI)
        .text_color(TEXT.solid())
        .children(rows)
}

pub(super) fn modifications(
    launcher: &Entity<Launcher>,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let (groups, skip, end_match) = {
        let liberator = &launcher.read(cx).liberator;
        let capabilities = &liberator.state().capabilities;
        (
            [PLAYERS, MATCH, LOADOUT].map(|mods| toggles(mods, liberator)),
            capabilities.has(Capability::Skip),
            capabilities.has(Capability::EndMatch),
        )
    };
    let [players, rounds, loadout] = groups.map(|toggles| group(toggles, launcher, window, cx));
    let (skipping, ending) = (launcher.clone(), launcher.clone());
    let skip = secondary("liberator-skip", "Skip", window, cx)
        .on_click(move |_, _, cx| skipping.read(cx).liberator.skip())
        .when(!skip, disable);
    let end_match = secondary("liberator-end-match", "End match", window, cx)
        .on_click(move |_, _, cx| ending.read(cx).liberator.end_match())
        .when(!end_match, disable);
    let column = || div().flex().flex_col().gap(px(16.));
    let actions = div().flex().gap(px(8.)).child(skip).child(end_match);
    let grid = div()
        .grid()
        .grid_cols(2)
        .gap(px(16.))
        .max_w(px(720.))
        .child(column().child(players).child(rounds))
        .child(column().child(loadout).child(actions));
    let area = scroll("liberator-mods", window, cx);
    area.keys(area.area(div().size_full().child(grid)))
}
