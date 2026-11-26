use std::rc::Rc;
use std::time::Instant;

use crate::ui::{frame, px};
use backend::cheatengine::CheatEngineStatus;
use backend::shears::{Kind, Scan};
use gpui::{
    App, Context, Deferred, Entity, InteractiveElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder,
};

use super::SeasonPage;
use super::playtime::overview;
use crate::modals::steps::archive_steps;
use crate::model::Launcher;
use crate::model::cheatengine::CheatEngineEvent;
use crate::model::download::DownloadEvent;
use crate::model::settings::{Library, SettingsEvent};
use crate::model::shears::ShearsEvent;
use crate::model::uninstall::UninstallEvent;
use crate::shell::{self, Page};
use crate::ui::button::{icon_button, primary, secondary};
use crate::ui::icon::{BOOKMARK_FILLED, REMOVE, icon};
use crate::ui::input::FIELD_PAD;
use crate::ui::kit::{disable, label_chip, list_row, note, save_check, tip, truncate};
use crate::ui::modal::{modal_title, picker_row};
use crate::ui::motion;
use crate::ui::prose::{BODY, inline};
use crate::ui::setting::TextSetting;
use crate::ui::switch::switch;
use crate::ui::theme::{REM, TEXT_MUTED, Theme};
use crate::ui::toast;

pub(super) enum Pending {
    Download { library: SharedString },
    Verify,
    SwitchToHm,
}

pub(super) enum Dialog {
    Download {
        library: SharedString,
    },
    RemoveDownload,
    OperatorsLocked,
    SelectHm {
        then: Pending,
        busy: bool,
    },
    Proton {
        status: Option<CheatEngineStatus>,
        args: TextSetting,
        saved: Option<Instant>,
    },
    Playtime,
    ResetPlaytime,
    Shears,
    Uninstall {
        busy: bool,
    },
}

pub(super) struct Cut {
    pub label: SharedString,
    pub size: u64,
    pub kind: Kind,
}

pub(super) fn cuts(scan: &Scan) -> Vec<Cut> {
    let whole = |label: &'static str, size: u64, kind: Kind| {
        (size > 0).then(|| Cut {
            label: label.into(),
            size,
            kind,
        })
    };
    let videos = whole("Videos", scan.videos, Kind::Videos);
    let events = whole("Events", scan.events, Kind::Events);
    let lower = scan.tiers.len().saturating_sub(1);
    let textures = scan.tiers.iter().take(lower).enumerate();
    let textures = textures.map(|(index, tier)| Cut {
        label: format!("Textures above {}", tier.quality).into(),
        size: scan.tiers[index + 1..].iter().map(|tier| tier.size).sum(),
        kind: Kind::Textures { above: tier.level },
    });
    videos.into_iter().chain(events).chain(textures).collect()
}

pub(super) fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let (mut value, mut unit) = (bytes as f64, 0);
    while value.round() >= 1024. && unit < UNITS.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    let tenths = (value * 10.).round();
    if unit > 0 && tenths < 1000. {
        format!("{:.1} {}", tenths / 10., UNITS[unit])
    } else {
        format!("{} {}", value.round(), UNITS[unit])
    }
}

impl SeasonPage {
    pub(super) fn ready(&self, cx: &App) -> bool {
        let listed = self.launcher.read(cx).settings.protons.is_some();
        listed || !matches!(self.dialog, Some(Dialog::Proton { .. }))
    }

    pub(super) fn show(&mut self, dialog: Dialog, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = Some(dialog);
        if self.ready(cx) {
            self.modal.open(window, cx);
        }
        cx.notify();
    }

    pub(super) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = None;
        self.modal.close(window, cx);
        cx.notify();
    }

    fn cheat_engine(&self) -> bool {
        !self.hm && self.installs.tb.installed
    }

    fn partial(&self) -> bool {
        self.installs.edition(self.hm).partial
    }

    pub(super) fn ensure_hm(&mut self, then: Pending, window: &mut Window, cx: &mut Context<Self>) {
        let asked = self.hm && self.season.hm_beta;
        if asked && !self.launcher.read(cx).hm_archive_cached() {
            self.show(Dialog::SelectHm { then, busy: false }, window, cx);
        } else {
            self.run(then, cx);
        }
    }

    fn run(&mut self, pending: Pending, cx: &mut Context<Self>) {
        let (key, hm, partial) = (self.season.key.clone(), self.hm, self.partial());
        self.launcher.update(cx, |launcher, cx| match pending {
            Pending::Download { library } => {
                let library = if partial { "" } else { library.as_ref() };
                launcher.enqueue(&key, hm, library, cx)
            }
            Pending::Verify => launcher.verify(&key, hm, cx),
            Pending::SwitchToHm => launcher.switch_to_hm(&key, cx),
        });
    }

    fn start_download(
        &mut self,
        library: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close(window, cx);
        self.ensure_hm(Pending::Download { library }, window, cx);
    }

    pub(super) fn prompt_download(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.partial() {
            return self.start_download(SharedString::default(), window, cx);
        }
        let survey = |launcher: &mut Launcher, cx: &mut Context<Launcher>| {
            launcher.survey_libraries(cx);
            launcher.settings.libraries().to_vec()
        };
        let libraries = self.launcher.update(cx, survey);
        let usable = |library: &&Library| library.exists;
        let preferred = libraries
            .iter()
            .find(|library| library.default && library.exists);
        let library = preferred.or_else(|| libraries.iter().find(usable));
        let library = library.map(|library| library.path.clone());
        let library = library.unwrap_or_default();
        let own = libraries
            .iter()
            .find(|library| library.default && !library.fixed);
        if libraries.len() > 1 && !own.is_some_and(|library| library.exists) {
            self.show(Dialog::Download { library }, window, cx);
        } else {
            self.start_download(library, window, cx);
        }
    }

    pub(super) fn open_proton(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.season.key.clone();
        let watching = self.cheat_engine();
        let (status, stored) = self.launcher.update(cx, |launcher, cx| {
            launcher.refresh_protons(cx);
            let status = watching.then(|| launcher.cheat_engine_status(&key));
            let stored = SharedString::from(launcher.settings.launch_args(&key).to_owned());
            (status, stored)
        });
        let dialog = Dialog::Proton {
            status,
            args: TextSetting::new(&stored, Some("%command%"), window, cx),
            saved: None,
        };
        self.show(dialog, window, cx);
    }

    pub(super) fn copy_crash_log(&mut self, cx: &mut Context<Self>) -> bool {
        let key = self.season.key.clone();
        let launcher = self.launcher.clone();
        let copied = launcher.update(cx, |launcher, _| launcher.copy_crash_log(&key));
        if let Err(message) = &copied {
            toast::show(message.clone(), None, cx);
            self.refresh(cx);
        }
        copied.is_ok()
    }

    fn reread_cheat_engine(&mut self, cx: &mut Context<Self>) {
        let watching = self.cheat_engine();
        if let Some(Dialog::Proton { status, .. }) = &mut self.dialog {
            let launcher = self.launcher.read(cx);
            *status = watching.then(|| launcher.cheat_engine_status(&self.season.key));
            cx.notify();
        }
    }

    fn toggle_cheat_engine(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(Dialog::Proton {
            status: Some(status),
            ..
        }) = &self.dialog
        else {
            return;
        };
        let (key, present) = (self.season.key.clone(), status.present);
        self.launcher.update(cx, |launcher, cx| {
            if !on {
                launcher.remove_cheat_engine(&key, cx)
            } else if present {
                launcher.add_cheat_engine(&key, cx)
            } else {
                launcher.install_cheat_engine(&key, cx)
            }
        });
        self.reread_cheat_engine(cx);
    }

    pub(super) fn download_event(
        &mut self,
        _: &Entity<Launcher>,
        event: &DownloadEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            DownloadEvent::Done { key, .. } | DownloadEvent::PartialDeleted { key, .. } => {
                if *key == self.season.key {
                    self.refresh(cx);
                }
            }
            DownloadEvent::HmArchivePicked(picked) => {
                let Some(Dialog::SelectHm { busy, .. }) = &mut self.dialog else {
                    return;
                };
                if !std::mem::take(busy) {
                    return;
                }
                cx.notify();
                if *picked && let Some(Dialog::SelectHm { then, .. }) = self.dialog.take() {
                    self.close(window, cx);
                    self.run(then, cx);
                }
            }
            _ => {}
        }
    }

    pub(super) fn settings_event(
        &mut self,
        _: &Entity<Launcher>,
        event: &SettingsEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SettingsEvent::LaunchArgsSaved => {
                if let Some(Dialog::Proton { saved, .. }) = &mut self.dialog {
                    *saved = Some(cx.background_executor().now());
                    cx.notify();
                }
            }
            SettingsEvent::PrefixRemoved => {
                self.reload(cx);
                self.reread_cheat_engine(cx);
            }
            _ => {}
        }
    }

    pub(super) fn shears_event(
        &mut self,
        _: &Entity<Launcher>,
        event: &ShearsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (ShearsEvent::Scanned { key, outcome } | ShearsEvent::Cut { key, outcome }) = event;
        if *key != self.season.key {
            return;
        }
        let cut = matches!(event, ShearsEvent::Cut { .. });
        match outcome {
            Ok(scan) => {
                self.scan = scan.clone();
                let shown = matches!(self.dialog, Some(Dialog::Shears));
                if cut && shown && cuts(scan).is_empty() {
                    self.close(window, cx);
                }
            }
            Err(message) => {
                toast::show(message.clone(), None, cx);
                if cut {
                    self.refresh(cx);
                }
            }
        }
        cx.notify();
    }

    pub(super) fn uninstall_event(
        &mut self,
        _: &Entity<Launcher>,
        event: &UninstallEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let UninstallEvent::Done(outcome) = event;
        if !matches!(self.dialog, Some(Dialog::Uninstall { .. })) {
            return;
        }
        if let Err(message) = outcome {
            toast::show(message.clone(), None, cx);
        }
        let other = self.installs.edition(!self.hm).installed;
        self.refresh(cx);
        self.close(window, cx);
        let editions = self.launcher.read(cx).home.editions();
        let others = editions.is_some_and(|editions| {
            let mut editions = editions.iter();
            editions.any(|edition| edition.key != self.season.key)
        });
        if outcome.is_ok() && !other && others && shell::page(cx) == Some(Page::Home) {
            shell::close_season(window, cx);
        }
    }

    pub(super) fn cheat_engine_event(
        &mut self,
        _: &Entity<Launcher>,
        event: &CheatEngineEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            CheatEngineEvent::Done => self.reread_cheat_engine(cx),
            CheatEngineEvent::Error(_) => {}
        }
    }

    fn download_dialog(
        &self,
        chosen: &SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Deferred {
        let (libraries, downloading) = {
            let launcher = self.launcher.read(cx);
            (
                launcher.settings.libraries().to_vec(),
                launcher.download.running(),
            )
        };
        let mut list = div().flex().flex_col().gap(px(8.));
        for (index, library) in libraries.into_iter().enumerate() {
            let pick = library.path.clone();
            let selected = library.path == *chosen;
            let row = picker_row(
                ("season-library", index),
                library.display,
                selected,
                window,
                cx,
            )
            .tooltip(tip(library.path.clone()))
            .on_click(cx.listener(move |page, _, _, cx| {
                if let Some(Dialog::Download { library }) = &mut page.dialog {
                    *library = pick.clone();
                }
                cx.notify();
            }))
            .when(library.default, |row| {
                row.child(
                    div()
                        .flex_none()
                        .p(px(4.))
                        .child(icon(BOOKMARK_FILLED, 16.).text_color(TEXT_MUTED.solid())),
                )
            })
            .when(!library.exists, |row| {
                disable(row.text_color(TEXT_MUTED.solid()).line_through())
            });
            list = list.child(row);
        }
        let title = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child("Download")
            .child(label_chip(format!("{} GB", self.season.size_gb)));
        let start = Rc::new(window.handler_for(&cx.entity(), |page, window, cx| {
            if let Some(Dialog::Download { library }) = &page.dialog {
                page.start_download(library.clone(), window, cx);
            }
        }));
        let confirm = start.clone();
        let launcher = self.launcher.clone();
        let label = if downloading {
            "Queue download"
        } else {
            "Download"
        };
        let buttons = [
            secondary("season-library-cancel", "Cancel", window, cx)
                .on_click(cx.listener(|page, _, window, cx| page.close(window, cx))),
            secondary("season-library-add", "Add library", window, cx)
                .on_click(move |_, _, cx| {
                    launcher.update(cx, |launcher, cx| launcher.add_library(cx));
                })
                .when(downloading, disable),
            primary("season-library-start", label, window, cx)
                .on_click(move |_, window, cx| start(window, cx)),
        ];
        self.modal
            .sheet(list)
            .title(title)
            .footer(buttons)
            .on_close(window.handler_for(&cx.entity(), Self::close))
            .on_confirm(move |window, cx| confirm(window, cx))
            .show(window, cx)
    }

    fn remove_dialog(&self, window: &mut Window, cx: &mut Context<Self>) -> Deferred {
        let remove = window.handler_for(&cx.entity(), |page, window, cx| {
            page.close(window, cx);
            let (key, hm) = (page.season.key.clone(), page.hm);
            page.launcher
                .update(cx, |launcher, cx| launcher.delete_partial(&key, hm, cx));
        });
        self.modal
            .sheet(inline(
                "remove-ask",
                "This permanently removes the files downloaded so far.",
                BODY,
                window,
                cx,
            ))
            .title("Remove download")
            .confirm(
                "Remove",
                false,
                remove,
                window.handler_for(&cx.entity(), Self::close),
            )
            .without_enter()
            .show(window, cx)
    }

    fn locked_dialog(&self, window: &mut Window, cx: &mut Context<Self>) -> Deferred {
        let label = if self.partial() { "Verify" } else { "Download" };
        let download = window.handler_for(&cx.entity(), |page, window, cx| {
            page.prompt_download(window, cx)
        });
        self.modal
            .sheet(inline(
                "locked-ask",
                "Pick Striker and Sentry once you complete the tutorial.",
                BODY,
                window,
                cx,
            ))
            .title("Operators are locked")
            .note(note().child("There is currently no fix."))
            .confirm(
                label,
                false,
                download,
                window.handler_for(&cx.entity(), Self::close),
            )
            .show(window, cx)
    }

    fn uninstall_dialog(
        &self,
        busy: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Deferred {
        let name = if self.hm {
            "Heated Metal"
        } else {
            &self.season.name
        };
        let uninstall = window.handler_for(&cx.entity(), |page, _, cx| {
            page.dialog = Some(Dialog::Uninstall { busy: true });
            let (key, hm) = (page.season.key.clone(), page.hm);
            page.launcher
                .update(cx, |launcher, cx| launcher.uninstall_season(&key, hm, cx));
            cx.notify();
        });
        self.modal
            .sheet(inline(
                "uninstall-ask",
                "This permanently removes the installed files.",
                BODY,
                window,
                cx,
            ))
            .title(format!("Uninstall {name}"))
            .confirm(
                "Uninstall",
                busy,
                uninstall,
                window.handler_for(&cx.entity(), Self::close),
            )
            .without_enter()
            .show(window, cx)
    }

    fn select_hm_dialog(
        &self,
        busy: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Deferred {
        let select = window.handler_for(&cx.entity(), |page, _, cx| {
            if let Some(Dialog::SelectHm { busy, .. }) = &mut page.dialog {
                *busy = true;
            }
            page.launcher
                .update(cx, |launcher, cx| launcher.pick_hm_archive(cx));
            cx.notify();
        });
        let body = archive_steps(window, cx);
        self.modal
            .sheet(body)
            .title("Heated Metal")
            .confirm(
                "Select HM",
                busy,
                select,
                window.handler_for(&cx.entity(), Self::close),
            )
            .show(window, cx)
    }

    fn shears_dialog(&self, window: &mut Window, cx: &mut Context<Self>) -> Deferred {
        let mut list = div().flex().flex_col().gap(px(8.));
        for (index, cut) in cuts(&self.scan).into_iter().enumerate() {
            let kind = cut.kind;
            let remove = icon_button(("season-cut", index), REMOVE, window, cx)
                .when(index == 0, |remove| remove.track_focus(&self.cut))
                .on_click(cx.listener(move |page, _, _, cx| {
                    let key = page.season.key.clone();
                    page.launcher
                        .update(cx, |launcher, cx| launcher.cut_season(&key, kind, cx));
                }));
            list = list.child(
                list_row().child(truncate(cut.label)).child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(8.))
                        .mr(px(-4.))
                        .child(label_chip(format_bytes(cut.size)))
                        .child(remove),
                ),
            );
        }
        self.modal
            .sheet(list)
            .title("Shears")
            .on_close(window.handler_for(&cx.entity(), Self::close))
            .show(window, cx)
    }

    fn playtime_dialog(&self, window: &mut Window, cx: &mut Context<Self>) -> Deferred {
        let (entries, seasons, playing) = {
            let launcher = self.launcher.read(cx);
            let playing = !launcher.launch.running().is_empty();
            (launcher.playtime(), launcher.seasons(), playing)
        };
        let viewport = frame::area(window).size.width;
        let reset = secondary("playtime-reset", "Reset", window, cx)
            .on_click(cx.listener(|page, _, window, cx| {
                page.show(Dialog::ResetPlaytime, window, cx);
            }))
            .when(playing, disable);
        let body = overview(&entries, &seasons, viewport, Theme::get(cx).action, reset);
        self.modal
            .sheet(body)
            .wide()
            .on_close(window.handler_for(&cx.entity(), Self::close))
            .show(window, cx)
    }

    fn reset_dialog(&self, window: &mut Window, cx: &mut Context<Self>) -> Deferred {
        let reset = window.handler_for(&cx.entity(), |page, window, cx| {
            page.close(window, cx);
            let reset = page
                .launcher
                .update(cx, |launcher, cx| launcher.reset_playtime(cx));
            match reset {
                Ok(()) => toast::show("Play time reset", None, cx),
                Err(message) => toast::show(message, None, cx),
            }
        });
        let back = window.handler_for(&cx.entity(), |page, window, cx| {
            page.show(Dialog::Playtime, window, cx);
        });
        self.modal
            .sheet(inline(
                "playtime-reset-ask",
                "This permanently removes the play time of all seasons.",
                BODY,
                window,
                cx,
            ))
            .title("Reset play time")
            .confirm("Reset", false, reset, back)
            .without_enter()
            .show(window, cx)
    }

    fn proton_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Deferred> {
        let key = self.season.key.clone();
        let (default, pinned, stored, working, playing, protons) = {
            let launcher = self.launcher.read(cx);
            let settings = &launcher.settings;
            let protons = settings.protons.as_ref()?;
            let protons = protons.iter();
            let protons = protons.map(|proton| (proton.internal.clone(), proton.display.clone()));
            (
                settings.proton.clone(),
                SharedString::from(settings.season_proton(&key).to_owned()),
                SharedString::from(settings.launch_args(&key).to_owned()),
                launcher.cheat_engine.busy(),
                self.flags(launcher).playing_season,
                protons.collect::<Vec<_>>(),
            )
        };
        let offered = protons.iter().any(|(internal, _)| *internal == pinned);
        let active = if offered { pinned } else { default.clone() };
        let mut list = div().flex().flex_col().gap(px(8.));
        for (index, (internal, display)) in protons.into_iter().enumerate() {
            let usual = internal == default;
            let selected = internal == active;
            let row = picker_row(("season-proton", index), display, selected, window, cx)
                .on_click(cx.listener(move |page, _, window, cx| {
                    let key = page.season.key.clone();
                    let choice = if usual { "" } else { internal.as_ref() };
                    page.launcher.update(cx, |launcher, cx| {
                        launcher.settings.set_season_proton(&key, choice, cx)
                    });
                    page.close(window, cx);
                }))
                .when(usual, |row| row.child(label_chip("Default")));
            list = list.child(row);
        }
        let prefix = self.installs.prefix;
        let (launcher, season) = (self.launcher.clone(), key);
        let Some(Dialog::Proton {
            status,
            args,
            saved,
        }) = &mut self.dialog
        else {
            return None;
        };
        let status = *status;
        let before = stored.clone();
        let field = args.build(
            &stored,
            None,
            None,
            Rc::new(move |draft, _, cx| {
                let draft = draft.trim();
                if draft != before.as_ref() {
                    launcher.update(cx, |launcher, cx| {
                        launcher.settings.set_launch_args(&season, draft, cx)
                    });
                }
            }),
            window,
            cx,
        );
        let mut options = div().flex().flex_col().gap(px(8.)).child(
            div()
                .child(field.w_full().pr(px(2. * REM - FIELD_PAD)))
                .children(save_check(*saved, window, cx)),
        );
        if let Some(status) = status {
            let toggle = switch(
                "season-cheat-engine",
                status.enabled && status.present,
                {
                    let page = cx.weak_entity();
                    move |on, _, cx| {
                        page.update(cx, |page, cx| page.toggle_cheat_engine(on, cx))
                            .ok();
                    }
                },
                window,
                cx,
            );
            let toggle = if working {
                div()
                    .flex()
                    .opacity(motion::pulse(window, cx))
                    .child(disable(toggle))
            } else {
                div().flex().child(toggle)
            };
            options = options.child(list_row().child(truncate("Cheat Engine")).child(toggle));
        }
        if prefix {
            let remove = icon_button("season-prefix", REMOVE, window, cx)
                .on_click(cx.listener(|page, _, _, cx| {
                    let key = page.season.key.clone();
                    page.launcher
                        .update(cx, |launcher, cx| launcher.remove_prefix(&key, cx));
                }))
                .when(playing || working, disable);
            options = options.child(
                list_row().child(truncate("Prefix")).child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .mr(px(-4.))
                        .child(remove),
                ),
            );
        }
        let aside = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(modal_title("Launch options"))
            .child(options);
        Some(
            self.modal
                .sheet(list)
                .title("Proton")
                .aside(aside)
                .on_close(window.handler_for(&cx.entity(), Self::close))
                .show(window, cx),
        )
    }

    pub(super) fn dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Deferred> {
        if !self.modal.is_open() {
            return None;
        }
        match self.dialog.as_ref()? {
            Dialog::Download { library } => Some(self.download_dialog(library, window, cx)),
            Dialog::RemoveDownload => Some(self.remove_dialog(window, cx)),
            Dialog::OperatorsLocked => Some(self.locked_dialog(window, cx)),
            Dialog::SelectHm { busy, .. } => Some(self.select_hm_dialog(*busy, window, cx)),
            Dialog::Proton { .. } => self.proton_dialog(window, cx),
            Dialog::Playtime => Some(self.playtime_dialog(window, cx)),
            Dialog::ResetPlaytime => Some(self.reset_dialog(window, cx)),
            Dialog::Shears => Some(self.shears_dialog(window, cx)),
            Dialog::Uninstall { busy } => Some(self.uninstall_dialog(*busy, window, cx)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use backend::env::Paths;
    use backend::manifest;
    use backend::settings::Settings as Store;
    use backend::shears::Tier;
    use gpui::TestAppContext;

    use super::super::manage::Act;
    use super::*;
    use crate::ui::testing::press;

    #[test]
    fn cuts_list_videos_events_and_every_texture_tier_but_the_last() {
        let tier = |level, quality, size| Tier {
            level,
            quality,
            size,
        };
        let scan = Scan {
            tiers: vec![
                tier(0, "Low", 10),
                tier(2, "High", 30),
                tier(4, "Ultra", 50),
            ],
            videos: 7,
            events: 0,
        };
        let listed: Vec<_> = cuts(&scan)
            .into_iter()
            .map(|cut| (cut.label.to_string(), cut.size, cut.kind))
            .collect();
        assert_eq!(
            listed,
            [
                ("Videos".to_owned(), 7, Kind::Videos),
                (
                    "Textures above Low".to_owned(),
                    80,
                    Kind::Textures { above: 0 }
                ),
                (
                    "Textures above High".to_owned(),
                    50,
                    Kind::Textures { above: 2 }
                ),
            ]
        );
        let events = Scan {
            events: 3,
            ..Scan::default()
        };
        assert_eq!(cuts(&events).len(), 1);
        assert!(cuts(&Scan::default()).is_empty());
    }

    #[test]
    fn sizes_show_one_decimal_below_a_hundred_units() {
        for (bytes, text) in [
            (0, "0 B"),
            (1, "1 B"),
            (1023, "1023 B"),
            (1024, "1.0 KB"),
            (1280, "1.3 KB"),
            (1536, "1.5 KB"),
            (102_297, "99.9 KB"),
            (102_349, "100 KB"),
            (102_400, "100 KB"),
            (1_047_552, "1023 KB"),
            (1_048_064, "1.0 MB"),
            (1_048_576, "1.0 MB"),
            (5 << 30, "5.0 GB"),
            (1 << 40, "1.0 TB"),
            (2000 << 40, "2000 TB"),
        ] {
            assert_eq!(format_bytes(bytes), text, "{bytes}");
        }
    }

    #[gpui::test]
    fn the_download_of_a_season_the_liberator_cannot_unlock_first_warns(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        fs::create_dir_all(&paths.data_root).unwrap();
        fs::write(paths.settings_file(), "[settings]\ndiscord_rpc = false\n").unwrap();
        cx.update(|cx| {
            crate::init(cx);
            let store = Store::load(&paths).unwrap();
            Launcher::new(paths.clone(), store, manifest::downloads()).init(cx);
        });
        let (page, cx) =
            cx.add_window_view(|_, cx| SeasonPage::new("Y11S3_SplitFire", false, cx).unwrap());
        page.update_in(cx, |page, window, cx| page.act(Act::Download, window, cx));
        let warned = page.read_with(cx, |page, cx| {
            let queued = page.launcher.read(cx).download.queue().count();
            (matches!(page.dialog, Some(Dialog::OperatorsLocked)), queued)
        });
        assert_eq!(warned, (true, 0));
    }

    #[gpui::test]
    fn escape_in_the_reset_question_leads_back_to_the_play_time(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        fs::create_dir_all(&paths.data_root).unwrap();
        fs::write(paths.settings_file(), "[settings]\ndiscord_rpc = false\n").unwrap();
        cx.update(|cx| {
            crate::init(cx);
            let store = Store::load(&paths).unwrap();
            Launcher::new(paths.clone(), store, manifest::downloads()).init(cx);
        });
        let (page, cx) =
            cx.add_window_view(|_, cx| SeasonPage::new("Y11S3_SplitFire", false, cx).unwrap());
        let draw = |cx: &mut gpui::VisualTestContext| {
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
        };
        cx.update(|window, _| window.activate_window());
        page.update_in(cx, |page, window, cx| {
            page.attach(window, cx);
            page.show(Dialog::Playtime, window, cx);
        });
        draw(cx);
        let shown = |cx: &mut gpui::VisualTestContext| {
            page.read_with(cx, |page, _| {
                let asking = matches!(page.dialog, Some(Dialog::ResetPlaytime));
                let listing = matches!(page.dialog, Some(Dialog::Playtime));
                (asking, listing, page.modal.is_open())
            })
        };
        press(cx, "tab enter");
        draw(cx);
        assert_eq!(shown(cx), (true, false, true));
        press(cx, "escape");
        draw(cx);
        assert_eq!(shown(cx), (false, true, true));
    }
}
