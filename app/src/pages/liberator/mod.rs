mod modifications;
mod playlist;
mod status;
mod support;
#[cfg(test)]
mod tests;

use std::mem;
use std::rc::Rc;

use crate::ui::px;
use backend::env::IS_WINDOWS;
use backend::settings::Flag;
use gpui::{
    App, AppContext, Context, Deferred, Div, ElementId, Entity, FocusHandle, InteractiveElement,
    Interpolate, IntoElement, ParentElement, Render, Rgba, SharedString, Stateful, StyleRefinement,
    Styled, Subscription, Task, Window, div, prelude::FluentBuilder,
};

use crate::modals::steps::exclusion;
use crate::model::Launcher;
use crate::model::liberator::{Capability, Liberator};
use crate::shell;
use crate::ui::icon::{CHEVRON_RIGHT, icon};
use crate::ui::kit::{focus_ring_inside, roving, truncate};
use crate::ui::modal::Modal;
use crate::ui::motion::{self, EASE_IN_OUT, FAST, sub};
use crate::ui::prose::{BODY, inline};
use crate::ui::scroll::reveal;
use crate::ui::switch::switch;
use crate::ui::tabs::{self, tabs_of};
use crate::ui::theme::{
    BORDER, FONT_BODY_SEMIBOLD, SURFACE_2, TEXT, TEXT_MUTED, TEXT_UI, Theme, Typeface,
};

pub use status::loading_ring;
use status::{status, status_icon};
use support::Support;

const TABS: &str = "liberator-tabs";
const ORDER: [Tab; 3] = [Tab::Playlist, Tab::Modifications, Tab::Support];
const STATUS: f32 = 18.;
const STATUS_GAP: f32 = 10.;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tab {
    Playlist,
    Modifications,
    Support,
}

struct Ready {
    controls: bool,
    playlist: bool,
    mods: bool,
}

fn ready(liberator: &Liberator) -> Ready {
    let state = liberator.state();
    let controls = state.unlocked && state.capabilities.has(Capability::FullFeature);
    Ready {
        controls,
        playlist: controls && liberator.tree().is_some(),
        mods: controls && modifications::offered(&state.capabilities),
    }
}

fn gate(tab: Tab, playlist_was_ready: bool, ready: &Ready) -> Tab {
    if ready.playlist && !playlist_was_ready && tab != Tab::Modifications {
        Tab::Playlist
    } else if (tab == Tab::Playlist && !ready.controls)
        || (tab == Tab::Modifications && !ready.mods)
    {
        Tab::Support
    } else {
        tab
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mark {
    Idle,
    Active,
    Chosen,
}

fn column_item(
    id: ElementId,
    label: SharedString,
    mark: Mark,
    branch: bool,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let hover = motion::hover(&id, FAST, window, cx);
    let mut level = |part: &'static str, on: bool| {
        let target = if on { 1. } else { 0. };
        motion::transition(sub(&id, part), target, FAST, EASE_IN_OUT, window, cx)
    };
    let lit = level("lit", mark != Mark::Idle || hover.over);
    let chosen = level("chosen", mark == Mark::Chosen);
    let (rest, dim) = (SURFACE_2.rgba(lit), Theme::get(cx).action_dim);
    let fill = Rgba::interpolate(rest, dim, chosen);
    let color = TEXT_MUTED.mix(TEXT, lit).solid();
    let caption = div().flex().flex_1().min_w_0().top(px(-1.));
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(6.))
        .border_l_2()
        .border_color(BORDER.alpha(0.))
        .px(px(8.))
        .py(px(4.))
        .bg(fill)
        .face(FONT_BODY_SEMIBOLD, TEXT_UI)
        .line_height(px(17.))
        .text_color(color)
        .tab_index(0)
        .focus_visible(focus_ring_inside)
        .map(reveal)
        .map(|element| hover.follow(element))
        .child(caption.child(truncate(label)))
        .children(branch.then(|| icon(CHEVRON_RIGHT, 14.).text_color(color)))
}

pub struct LiberatorPage {
    launcher: Entity<Launcher>,
    tab: Tab,
    support: Entity<Support>,
    playlist_ready: bool,
    available: bool,
    risen: bool,
    dismissed: bool,
    refocus: bool,
    panel: FocusHandle,
    missing: Modal,
    _rise: Option<Task<()>>,
    _launcher: Subscription,
}

impl LiberatorPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let launcher = Launcher::handle(cx);
        let risen = cx.reduce_motion();
        let rise = (!risen).then(|| {
            cx.spawn_in(window, async move |page, cx| {
                cx.background_executor().timer(shell::ENTER).await;
                page.update_in(cx, |page, window, cx| {
                    page.risen = true;
                    page.settle(window, cx);
                })
                .ok();
            })
        });
        let watch = cx.observe_in(&launcher, window, |page, _, window, cx| {
            page.settle(window, cx)
        });
        let mut page = Self {
            launcher,
            tab: Tab::Support,
            support: cx.new(|_| Support::default()),
            playlist_ready: false,
            available: true,
            risen,
            dismissed: false,
            refocus: false,
            panel: cx.focus_handle(),
            missing: Modal::new(cx),
            _rise: rise,
            _launcher: watch,
        };
        page.settle(window, cx);
        page
    }

    fn settle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let launcher = self.launcher.read(cx);
        let liberator = &launcher.liberator;
        let enabled = launcher.settings.flag(Flag::LiberatorEnabled);
        let ready = ready(liberator);
        self.available = liberator.state().attached || liberator.available();
        let tab = gate(self.tab, self.playlist_ready, &ready);
        self.playlist_ready = ready.playlist;
        if tab != self.tab {
            self.refocus = self.panel.contains_focused(window, cx);
            self.turn(tab, cx);
        }
        if self.risen && enabled && !self.available && !self.dismissed {
            self.missing.open(window, cx);
        } else {
            self.missing.close(window, cx);
        }
        cx.notify();
    }

    fn turn(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.support = cx.new(|_| Support::default());
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.tab != ORDER[index] {
            self.turn(ORDER[index], cx);
            cx.notify();
        }
    }

    fn missing_modal(&self, window: &mut Window, cx: &mut Context<Self>) -> Deferred {
        let launcher = self.launcher.clone();
        let busy = launcher.read(cx).liberator.busy();
        let body = if IS_WINDOWS {
            let removed = "Windows Security may have removed `Liberator.exe`.";
            let removed = inline("liberator-removed", removed, BODY, window, cx);
            exclusion(removed, None, window, cx)
        } else {
            inline(
                "liberator-missing",
                "`Liberator.exe` was not found.",
                BODY,
                window,
                cx,
            )
        };
        let page = cx.weak_entity();
        self.missing
            .sheet(body)
            .title("Liberator is missing")
            .confirm(
                "Download",
                busy,
                move |_, cx| launcher.update(cx, |launcher, cx| launcher.liberator.restore(cx)),
                move |window, cx| {
                    page.update(cx, |page, cx| {
                        page.dismissed = true;
                        page.settle(window, cx);
                    })
                    .ok();
                },
            )
            .show(window, cx)
    }
}

impl Render for LiberatorPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let room = shell::page_size(window);
        if mem::take(&mut self.refocus) && !self.missing.is_open() {
            let stops = roving(&TABS.into(), ORDER.len(), window, cx);
            window.focus(&stops[self.tab as usize], cx);
        }
        let (ready, enabled, status) = {
            let launcher = self.launcher.read(cx);
            let enabled = launcher.settings.flag(Flag::LiberatorEnabled);
            let liberator = &launcher.liberator;
            let status = status(liberator.state(), enabled, self.available);
            (ready(liberator), enabled, status)
        };
        let entry = |label, disabled| tabs::Tab { label, disabled };
        let entries = [
            entry("Playlist", !ready.playlist),
            entry("Modifications", !ready.mods),
            entry("Support", false),
        ];
        let page = cx.weak_entity();
        let select = Rc::new(move |index: usize, _: &mut Window, cx: &mut App| {
            page.update(cx, |page, cx| page.select(index, cx)).ok();
        });
        let launcher = self.launcher.clone();
        let toggle = switch(
            "liberator-enabled",
            enabled,
            move |value, _, cx| {
                launcher.update(cx, |launcher, cx| launcher.set_liberator_enabled(value, cx));
            },
            window,
            cx,
        );
        let sign = status.map(|status| status_icon(status, STATUS, window, cx));
        let trailing = div()
            .flex()
            .items_center()
            .gap(px(STATUS_GAP))
            .children(sign)
            .child(toggle);
        let active = self.tab as usize;
        let row = tabs_of(TABS, &entries, active, select, window, cx);
        let body = match self.tab {
            Tab::Playlist => self.playlist(window, cx).map(IntoElement::into_any_element),
            Tab::Modifications => {
                let mods = modifications::modifications(&self.launcher, window, cx);
                Some(mods.into_any_element())
            }
            Tab::Support => {
                let whole = StyleRefinement::default().size_full();
                Some(self.support.clone().cached(whole).into_any_element())
            }
        };
        let missing = self.missing.is_open();
        div()
            .flex()
            .flex_col()
            .h(room.height)
            .child(row.child(trailing))
            .child(
                div()
                    .mt(px(16.))
                    .min_h_0()
                    .flex_1()
                    .track_focus(&self.panel)
                    .children(body),
            )
            .children(missing.then(|| self.missing_modal(window, cx)))
    }
}
