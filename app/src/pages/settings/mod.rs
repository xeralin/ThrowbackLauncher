mod appearance;
mod general;

use crate::ui::{bind, frame, px, units};
use gpui::{
    App, Context, Div, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window,
    div, prelude::FluentBuilder,
};

use backend::env::IS_WINDOWS;

use crate::model::Launcher;
use crate::ui::kit::panel;
use crate::ui::tabs::tabs;
use crate::ui::theme::BORDER;

use appearance::Appearance;
use general::General;

const TABS: [&str; 2] = ["General", "Appearance"];

enum Body {
    General(General),
    Appearance(Appearance),
}

pub struct SettingsPage {
    launcher: Entity<Launcher>,
    body: Body,
    _subscriptions: [Subscription; 3],
}

impl SettingsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let launcher = Launcher::handle(cx);
        Self::reload(&launcher, cx);
        let subscriptions = [
            cx.observe(&launcher, |_, _, cx| cx.notify()),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    Self::reload(&this.launcher, cx);
                }
            }),
            cx.subscribe(&launcher, |this, launcher, event, cx| {
                if let Body::General(general) = &mut this.body {
                    general.settings_event(event, &launcher, cx);
                    cx.notify();
                }
            }),
        ];
        let body = Body::General(General::new(&launcher, window, cx));
        Self {
            launcher,
            body,
            _subscriptions: subscriptions,
        }
    }

    fn reload(launcher: &Entity<Launcher>, cx: &mut App) {
        launcher.update(cx, |launcher, cx| launcher.survey_libraries(cx));
        Self::reload_protons(launcher, cx);
    }

    fn reload_protons(launcher: &Entity<Launcher>, cx: &mut App) {
        if !IS_WINDOWS {
            launcher.update(cx, |launcher, cx| launcher.refresh_protons(cx));
        }
    }

    fn general(&mut self) -> Option<&mut General> {
        match &mut self.body {
            Body::General(general) => Some(general),
            _ => None,
        }
    }

    fn appearance(&mut self) -> Option<&mut Appearance> {
        match &mut self.body {
            Body::Appearance(appearance) => Some(appearance),
            _ => None,
        }
    }

    fn tab(&self) -> usize {
        match self.body {
            Body::General(_) => 0,
            Body::Appearance(_) => 1,
        }
    }

    fn select(&mut self, tab: usize, window: &mut Window, cx: &mut Context<Self>) {
        if tab == self.tab() {
            return;
        }
        self.body = match tab {
            0 => Body::General(General::new(&self.launcher, window, cx)),
            _ => Body::Appearance(Appearance::new(&self.launcher, window, cx)),
        };
        cx.notify();
    }
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = units(frame::area(window).size.width);
        let select = bind(cx, |this, tab, window, cx| this.select(tab, window, cx));
        let active = self.tab();
        let body = match &mut self.body {
            Body::General(general) => general.render(&self.launcher, viewport, window, cx),
            Body::Appearance(appearance) => appearance.render(&self.launcher, viewport, window, cx),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(tabs("settings-tabs", &TABS, active, select, window, cx))
            .child(body)
    }
}

fn sections(viewport: f32) -> Div {
    const BREAKPOINT: f32 = 1200.;
    div()
        .grid()
        .grid_cols(if viewport >= BREAKPOINT { 2 } else { 1 })
        .gap(px(16.))
        .max_w(px(1160.))
        .items_start()
}

fn column() -> Div {
    div().flex().flex_col().gap(px(16.))
}

fn section(rows: impl IntoIterator<Item = Div>) -> Div {
    let mut list = panel();
    for (index, row) in rows.into_iter().enumerate() {
        list = list.child(
            div()
                .px(px(16.))
                .py(px(8.))
                .when(index > 0, |cell| {
                    cell.border_t_1().border_color(BORDER.solid())
                })
                .child(row),
        );
    }
    list
}
