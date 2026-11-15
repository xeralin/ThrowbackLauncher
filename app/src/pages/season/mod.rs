mod dialogs;
mod info;
mod manage;
mod menu;
mod playtime;

use std::path::PathBuf;

use crate::ui::{bind, px};
use backend::env::IS_WINDOWS;
use backend::manifest::SeasonEntry;
use backend::shears::Scan;
use gpui::{
    App, AppContext, Context, Div, ElementId, Entity, FocusHandle, InteractiveElement, IntoElement,
    ParentElement, Render, RetainAllImageCache, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, div, prelude::FluentBuilder,
};

use crate::assets::HEATED_METAL_ART;
use crate::model::launch::{GameArgs, Installs};
use crate::model::{Edition, Launcher};
use crate::shell;
use crate::ui::Dismiss;
use crate::ui::icon::{CHEVRON_LEFT, icon};
use crate::ui::key_art::key_art;
use crate::ui::kit::{focus_ring_inside, panel};
use crate::ui::log_box::Log;
use crate::ui::modal::Modal;
use crate::ui::motion::{self, FAST};
use crate::ui::tabs::{options, tabs};
use crate::ui::theme::{FONT_DISPLAY_BOLD, REM, TEXT, TEXT_MUTED, Typeface};
use dialogs::Dialog;
pub use menu::SeasonMenu;

const TITLE: f32 = 1.9 * REM;
const ART: f32 = 214.;
const ART_SHORTEST: f32 = 160.;
const CORNER: f32 = 8.;
const SHORTEST: f32 = 30. * REM;
const TABS: [&str; 2] = ["Manage", "Info"];
const EDITIONS: [&str; 2] = ["Throwback", "Heated Metal"];

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Manage,
    Info,
}

#[derive(Clone, PartialEq)]
struct Watched {
    playing: bool,
    running: bool,
    libraries: Vec<PathBuf>,
}

pub struct SeasonPage {
    launcher: Entity<Launcher>,
    season: SeasonEntry,
    hm: bool,
    tab: Tab,
    art: Entity<RetainAllImageCache>,
    back: FocusHandle,
    installs: Installs,
    game_args: GameArgs,
    scans: bool,
    scan: Scan,
    log: Log,
    watched: Option<Watched>,
    renderers: bool,
    split: FocusHandle,
    modal: Modal,
    dialog: Option<Dialog>,
    cut: FocusHandle,
    watch: Vec<Subscription>,
}

impl SeasonPage {
    pub fn new(key: &str, hm: bool, cx: &mut App) -> Option<Self> {
        let launcher = Launcher::handle(cx);
        let seasons = launcher.read(cx).seasons();
        let season = seasons.into_iter().find(|season| season.key == key)?;
        let stop = |cx: &mut App| cx.focus_handle().tab_index(0).tab_stop(true);
        Some(Self {
            launcher,
            hm: hm && season.hm_available,
            season,
            tab: Tab::Manage,
            art: RetainAllImageCache::new(cx),
            back: stop(cx),
            installs: Installs::default(),
            game_args: GameArgs::default(),
            scans: false,
            scan: Scan::default(),
            log: Log::default(),
            watched: None,
            renderers: false,
            split: cx.focus_handle(),
            modal: Modal::new(cx),
            dialog: None,
            cut: stop(cx),
            watch: Vec::new(),
        })
    }

    pub fn key(&self) -> &str {
        &self.season.key
    }

    pub fn edition(&self) -> Edition {
        Edition {
            key: self.season.key.clone(),
            hm: self.hm,
        }
    }

    pub fn title(&self) -> SharedString {
        self.season.title(self.hm).into()
    }

    pub fn set_edition(&mut self, hm: bool, cx: &mut Context<Self>) {
        let hm = hm && self.season.hm_available;
        if hm != self.hm {
            self.hm = hm;
            self.game_args = self.launcher.read(cx).game_args(&self.season.key, hm);
        }
        cx.notify();
    }

    pub fn mount(self, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let page = cx.new(|_| self);
        page.update(cx, |page, cx| {
            window.focus(&page.back, cx);
            page.scans = true;
            page.attach(window, cx);
        });
        page
    }

    fn attach(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let launcher = self.launcher.clone();
        self.watch = vec![
            cx.observe_in(&launcher, window, |page, _, window, cx| {
                page.follow(window, cx)
            }),
            cx.subscribe_in(&launcher, window, Self::download_event),
            cx.subscribe_in(&launcher, window, Self::settings_event),
            cx.subscribe_in(&launcher, window, Self::shears_event),
            cx.subscribe_in(&launcher, window, Self::uninstall_event),
            cx.subscribe_in(&launcher, window, Self::cheat_engine_event),
            cx.observe_window_activation(window, |page, window, cx| {
                if window.is_window_active() {
                    page.revisit(cx);
                }
            }),
            cx.on_focus_lost(window, Self::focus_lost),
        ];
        self.revisit(cx);
        self.follow(window, cx);
    }

    fn revisit(&mut self, cx: &mut Context<Self>) {
        self.refresh(cx);
        if !IS_WINDOWS {
            self.launcher
                .update(cx, |launcher, cx| launcher.refresh_protons(cx));
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let launcher = self.launcher.read(cx);
        self.installs = launcher.installs(&self.season.key);
        self.game_args = launcher.game_args(&self.season.key, self.hm);
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.reload(cx);
        if self.scans {
            let key = self.season.key.clone();
            self.launcher
                .update(cx, |launcher, cx| launcher.scan_season(&key, cx));
        }
    }

    fn survey(&mut self, cx: &mut Context<Self>) {
        let launcher = self.launcher.read(cx);
        let flags = self.flags(launcher);
        let watched = Watched {
            playing: flags.playing_season,
            running: flags.running_season,
            libraries: launcher.settings.roots().to_vec(),
        };
        let Some(before) = self.watched.replace(watched.clone()) else {
            return;
        };
        if before == watched {
            return;
        }
        let mut roots = watched.libraries.iter();
        let added = roots.find(|root| !before.libraries.contains(root));
        if let (Some(Dialog::Download { library }), Some(added)) = (&mut self.dialog, added) {
            *library = added.to_string_lossy().into_owned().into();
        }
        self.refresh(cx);
    }

    fn follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.survey(cx);
        let launcher = self.launcher.read(cx);
        let flags = self.flags(launcher);
        let download = &launcher.download;
        if download
            .active()
            .is_some_and(|active| active.key == self.season.key)
        {
            self.log.follow(download.history());
        }
        let held = flags.downloading_season || flags.playing_season;
        let changing = matches!(self.dialog, Some(Dialog::Shears | Dialog::Uninstall { .. }));
        if held && changing {
            self.close(window, cx);
        }
        if self.dialog.is_some() && !self.modal.is_open() && self.ready(cx) {
            self.modal.open(window, cx);
        }
        cx.notify();
    }

    fn focus_lost(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.modal.is_open() {
            return;
        }
        let cutting = matches!(self.dialog, Some(Dialog::Shears));
        if cutting && !dialogs::cuts(&self.scan).is_empty() {
            window.focus(&self.cut, cx);
        } else {
            self.modal.focus(window, cx);
        }
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.renderers) {
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn heading(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let id = ElementId::from("season-back");
        let hover = motion::hover(&id, FAST, window, cx);
        div().flex().flex_none().mt(px(-1.)).mb(px(17.)).child(
            div()
                .id(id)
                .flex()
                .items_center()
                .rounded(px(6.))
                .face(FONT_DISPLAY_BOLD, TITLE)
                .line_height(px(TITLE))
                .track_focus(&self.back)
                .focus_visible(focus_ring_inside)
                .map(|element| hover.follow(element))
                .on_click(|_, window, cx| shell::close_season(window, cx))
                .child(
                    icon(CHEVRON_LEFT, 24.)
                        .flex_none()
                        .ml(px(-8.))
                        .mt(px(-3.))
                        .text_color(TEXT_MUTED.mix(TEXT, hover.amount).solid()),
                )
                .child(self.title()),
        )
    }

    fn tab_row(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let active = self.tab as usize;
        let select = bind(cx, |page, index, _, cx| {
            page.tab = if index == 0 { Tab::Manage } else { Tab::Info };
            cx.notify();
        });
        let row = tabs("season-tabs", &TABS, active, select, window, cx);
        if !self.season.hm_available {
            return row;
        }
        let pick = bind(cx, |page, index, _, cx| page.set_edition(index == 1, cx));
        let edition = self.hm as usize;
        row.child(options(
            "season-editions",
            &EDITIONS,
            edition,
            pick,
            window,
            cx,
        ))
    }
}

impl Render for SeasonPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.renderers && !self.split.contains_focused(window, cx) {
            self.renderers = false;
        }
        let room = shell::page_size(window);
        let managing = self.tab == Tab::Manage;
        let name = if self.hm {
            HEATED_METAL_ART
        } else {
            &self.season.key
        };
        let body = if managing {
            self.manage(window, cx)
        } else {
            self.info(room.width, window, cx)
        };
        div()
            .flex()
            .flex_col()
            .when(managing, |page| page.h(room.height.max(px(SHORTEST))))
            .on_action(cx.listener(Self::dismiss))
            .child(self.heading(window, cx))
            .child(
                panel()
                    .mb(px(12.))
                    .h(px(ART))
                    .min_h(px(ART_SHORTEST))
                    .child(key_art(name, !self.hm, CORNER, 1., &self.art)),
            )
            .child(self.tab_row(window, cx).flex_none())
            .child(body)
            .children(self.dialog(window, cx))
    }
}
