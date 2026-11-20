use std::rc::Rc;
use std::time::Instant;

use crate::ui::px;
use backend::env::{DOWNLOADS_MAX, DOWNLOADS_MIN, IS_WINDOWS, MAX_USERNAME_LENGTH};
use backend::settings::{Flag, clean_username};
use gpui::{
    AnyElement, App, Context, Deferred, Div, ElementId, Entity, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder,
};

use super::{SettingsPage, column, section, sections};
use crate::model::Launcher;
use crate::model::rvpn::State;
use crate::model::settings::{Library, SettingsEvent};
use crate::shell::{self, Page};
use crate::ui::button::{icon_button, primary, secondary};
use crate::ui::hint::hint;
use crate::ui::icon::{BOOKMARK, BOOKMARK_FILLED, FOLDER, REMOVE, icon};
use crate::ui::input::FIELD_PAD;
use crate::ui::kit::{
    chip, disable, field, focus_ring, label, list_row, note, note_external, note_link, row,
    save_check, tip, truncate, version_chip,
};
use crate::ui::modal::{Modal, picker_row};
use crate::ui::motion::{self, FAST};
use crate::ui::scroll::reveal;
use crate::ui::setting::TextSetting;
use crate::ui::stepper::Stepper;
use crate::ui::switch::switch;
use crate::ui::theme::{BORDER, REM, TEXT_MUTED, Theme};

const RADMIN_VPN_URL: &str = "https://radmin-vpn.com/";
const FIELD_WIDTH: f32 = 230.;

#[derive(Clone, Copy)]
enum Dialog {
    Proton,
    Rvpn,
}

pub struct General {
    username: TextSetting,
    username_saved: Option<Instant>,
    downloads: Stepper,
    modal: Modal,
    dialog: Option<Dialog>,
}

impl SettingsPage {
    fn show(&mut self, dialog: Dialog, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(general) = self.general() {
            general.dialog = Some(dialog);
            general.modal.open(window, cx);
            cx.notify();
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(general) = self.general() {
            general.dialog = None;
            general.modal.close(window, cx);
            cx.notify();
        }
    }
}

impl General {
    pub fn new(launcher: &Entity<Launcher>, window: &mut Window, cx: &mut App) -> Self {
        let (name, downloads) = {
            let settings = &launcher.read(cx).settings;
            (settings.username(), settings.max_downloads())
        };
        Self {
            username: TextSetting::new(&name, None, window, cx),
            username_saved: None,
            downloads: Stepper::new(downloads, window, cx),
            modal: Modal::new(cx),
            dialog: None,
        }
    }

    pub fn settings_event(
        &mut self,
        event: &SettingsEvent,
        launcher: &Entity<Launcher>,
        cx: &mut App,
    ) {
        match event {
            SettingsEvent::UsernameSaved => {
                self.username_saved = Some(cx.background_executor().now());
            }
            SettingsEvent::UsernameRejected(_) => {
                let value = launcher.read(cx).settings.username();
                self.username.reset(&value, cx);
            }
            _ => {}
        }
    }

    fn username_field(
        &mut self,
        launcher: &Entity<Launcher>,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let value = launcher.read(cx).settings.username();
        let target = launcher.clone();
        let field = self.username.build(
            &value,
            Some(MAX_USERNAME_LENGTH),
            Some(Rc::new(clean_username)),
            Rc::new(move |draft, _, cx| {
                if draft != target.read(cx).settings.username().as_ref() {
                    target.update(cx, |launcher, cx| launcher.settings.set_username(draft, cx));
                }
            }),
            window,
            cx,
        );
        div()
            .flex_none()
            .w(px(FIELD_WIDTH))
            .child(field.pr(px(2. * REM - FIELD_PAD)))
            .children(save_check(self.username_saved, window, cx))
    }

    fn proton_field(
        &self,
        launcher: &Entity<Launcher>,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) -> Option<AnyElement> {
        let settings = &launcher.read(cx).settings;
        let protons = settings.protons.as_ref()?;
        if protons.is_empty() {
            return Some(
                note()
                    .child("No Proton installation was found.")
                    .into_any_element(),
            );
        }
        let display = settings.proton_display();
        let id = ElementId::from("proton");
        let hover = motion::hover(&id, FAST, window, cx);
        let edge = BORDER.mix(Theme::get(cx).action_edge, hover.amount);
        Some(
            field()
                .id(id)
                .flex_none()
                .w(px(FIELD_WIDTH))
                .pt(px(1.))
                .px(px(FIELD_PAD))
                .border_color(edge.solid())
                .tab_index(0)
                .focus_visible(focus_ring)
                .map(reveal)
                .map(|element| hover.follow(element))
                .on_click(cx.listener(|page, _, window, cx| {
                    SettingsPage::reload_protons(&page.launcher, cx);
                    page.show(Dialog::Proton, window, cx);
                }))
                .child(truncate(display))
                .into_any_element(),
        )
    }

    fn proton_picker(
        &self,
        launcher: &Entity<Launcher>,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) -> Deferred {
        let (current, protons): (SharedString, Vec<(SharedString, SharedString)>) = {
            let settings = &launcher.read(cx).settings;
            (
                settings.proton.clone(),
                settings
                    .protons
                    .iter()
                    .flatten()
                    .map(|proton| (proton.internal.clone(), proton.display.clone()))
                    .collect(),
            )
        };
        let mut list = div().flex().flex_col().gap(px(8.));
        for (index, (internal, display)) in protons.into_iter().enumerate() {
            let selected = internal == current;
            list = list.child(
                picker_row(("proton-option", index), display, selected, window, cx).on_click(
                    cx.listener(move |page, _, window, cx| {
                        page.launcher.update(cx, |launcher, cx| {
                            launcher.settings.set_proton(&internal, cx)
                        });
                        page.close(window, cx);
                    }),
                ),
            );
        }
        self.modal
            .sheet(list)
            .title("Proton")
            .on_close(window.handler_for(&cx.entity(), SettingsPage::close))
            .show(window, cx)
    }

    fn rvpn_controls(
        &self,
        launcher: &Entity<Launcher>,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) -> Div {
        let (state, installed, step) = {
            let rvpn = &launcher.read(cx).rvpn;
            (rvpn.state(), rvpn.installed(), rvpn.step())
        };
        let (running, building) = (state == State::Running, state == State::Building);
        if !installed && !running && !building {
            let launcher = launcher.clone();
            return note()
                .child(note_external(
                    "rvpn-download",
                    "Download",
                    RADMIN_VPN_URL,
                    window,
                    cx,
                ))
                .child(" the installer and ")
                .child(
                    note_link("rvpn-select", "select it").on_click(move |_, _, cx| {
                        launcher.update(cx, |launcher, cx| launcher.rvpn.select_installer(cx));
                    }),
                )
                .child(".");
        }
        let mut controls = div().flex().min_w_0().items_center().gap(px(12.));
        if building && !step.is_empty() {
            controls = controls.child(chip(step).opacity(motion::pulse(window, cx)));
        }
        if installed {
            let manage = secondary("rvpn-manage", "Manage", window, cx)
                .on_click(cx.listener(|page, _, window, cx| {
                    page.show(Dialog::Rvpn, window, cx);
                }))
                .when(state == State::Uninstalling, disable);
            controls = controls.child(manage);
        }
        if running {
            let launcher = launcher.clone();
            controls = controls.child(
                secondary("rvpn-stop", "Stop", window, cx)
                    .on_click(move |_, _, cx| launcher.read(cx).rvpn.stop()),
            );
        } else if !building {
            let launcher = launcher.clone();
            let run = primary("rvpn-run", "Run", window, cx)
                .on_click(move |_, _, cx| {
                    launcher.update(cx, |launcher, cx| launcher.rvpn.run(cx));
                })
                .when(state != State::Idle, disable);
            controls = controls.child(run);
        }
        controls
    }

    fn rvpn_manage(
        &self,
        launcher: &Entity<Launcher>,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) -> Deferred {
        let (idle, version, autorun) = {
            let launcher = launcher.read(cx);
            (
                launcher.rvpn.state() == State::Idle,
                launcher.rvpn.version(),
                launcher.settings.flag(Flag::RvpnAutorun),
            )
        };
        let title = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child("Radmin VPN")
            .when(!version.is_empty(), |title| {
                title.child(version_chip(&version))
            });
        let toggle = switch(
            "rvpn-autorun",
            autorun,
            {
                let launcher = launcher.clone();
                move |value, _, cx| {
                    launcher.update(cx, |launcher, cx| {
                        launcher.settings.set_flag(Flag::RvpnAutorun, value, cx)
                    });
                }
            },
            window,
            cx,
        );
        let remove = icon_button("rvpn-uninstall", REMOVE, window, cx)
            .on_click(cx.listener(|page, _, window, cx| {
                page.close(window, cx);
                page.launcher
                    .update(cx, |launcher, cx| launcher.rvpn.uninstall(cx));
            }))
            .when(!idle, disable);
        let autorun = div()
            .flex()
            .flex_grow(1.)
            .min_w_0()
            .items_center()
            .gap(px(6.))
            .child("Autorun")
            .child(hint(
                "rvpn-autorun-hint",
                "Run Radmin VPN automatically when you open the Launcher.",
                window,
                cx,
            ));
        let rows = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                list_row().child(autorun).child(
                    div()
                        .flex()
                        .flex_none()
                        .h(px(24.))
                        .items_center()
                        .child(toggle),
                ),
            )
            .child(
                list_row().child(truncate("Uninstall")).child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .mr(px(-4.))
                        .child(remove),
                ),
            );
        self.modal
            .sheet(rows)
            .title(title)
            .on_close(window.handler_for(&cx.entity(), SettingsPage::close))
            .show(window, cx)
    }

    fn library_rows(
        &self,
        launcher: &Entity<Launcher>,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) -> [Div; 2] {
        let (libraries, usage, downloading) = {
            let launcher = launcher.read(cx);
            (
                launcher.settings.libraries().to_vec(),
                launcher.info.disk_usage(),
                launcher.download.running(),
            )
        };
        let add = secondary("library-add", "Add library", window, cx)
            .on_click({
                let launcher = launcher.clone();
                move |_, _, cx| {
                    launcher.update(cx, |launcher, cx| launcher.add_library(cx));
                }
            })
            .when(downloading, disable);
        let head = row().child(label("Libraries")).child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .children(usage.map(|size| chip(format!("{size} GB"))))
                .child(add),
        );
        let mut list = div().flex().flex_col().gap(px(8.));
        for (index, library) in libraries.into_iter().enumerate() {
            let Library {
                path,
                display,
                exists,
                default,
                fixed,
            } = library;
            let (opener, folder, removed) = (launcher.clone(), path.clone(), path.clone());
            let shown = path.clone();
            let mut actions = div().flex().flex_none().items_center().mr(px(-4.)).child(
                icon_button(("library-open", index), FOLDER, window, cx)
                    .on_click(move |_, _, cx| {
                        opener.update(cx, |launcher, cx| launcher.open_library(&folder, cx));
                    })
                    .when(!exists, disable),
            );
            actions = if default {
                actions.child(
                    div()
                        .flex_none()
                        .p(px(4.))
                        .child(icon(BOOKMARK_FILLED, 16.).text_color(TEXT_MUTED.solid())),
                )
            } else {
                let launcher = launcher.clone();
                actions.child(
                    icon_button(("library-default", index), BOOKMARK, window, cx)
                        .on_click(move |_, _, cx| {
                            launcher
                                .update(cx, |launcher, cx| launcher.set_default_library(&path, cx));
                        })
                        .when(!exists, disable),
                )
            };
            if !default && !fixed {
                let launcher = launcher.clone();
                actions = actions.child(
                    icon_button(("library-remove", index), REMOVE, window, cx).on_click(
                        move |_, _, cx| {
                            launcher
                                .update(cx, |launcher, cx| launcher.remove_library(&removed, cx));
                        },
                    ),
                );
            }
            list = list.child(
                list_row()
                    .when(!exists, |row| {
                        row.text_color(TEXT_MUTED.solid()).line_through()
                    })
                    .child(
                        div()
                            .id(("library-path", index))
                            .flex_1()
                            .min_w_0()
                            .child(truncate(display))
                            .tooltip(tip(shown)),
                    )
                    .child(actions),
            );
        }
        [head, list]
    }

    fn dialog(
        &self,
        launcher: &Entity<Launcher>,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) -> Option<Deferred> {
        if !self.modal.is_open() {
            return None;
        }
        Some(match self.dialog? {
            Dialog::Proton => self.proton_picker(launcher, window, cx),
            Dialog::Rvpn => self.rvpn_manage(launcher, window, cx),
        })
    }

    pub fn render(
        &mut self,
        launcher: &Entity<Launcher>,
        viewport: f32,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) -> Div {
        let (discord_rpc, tray, close_to_tray, account, downloads) = {
            let launcher = launcher.read(cx);
            (
                launcher.settings.flag(Flag::DiscordRpc),
                launcher.settings.tray_available,
                launcher.settings.flag(Flag::CloseToTray),
                launcher.settings.steam_account(),
                launcher.settings.max_downloads(),
            )
        };
        let session = if account.is_empty() {
            note()
                .child(
                    note_link("steam-download", "Download")
                        .on_click(|_, window, cx| shell::navigate(Page::Download, window, cx)),
                )
                .child(" a season to log in.")
        } else {
            let launcher = launcher.clone();
            div()
                .flex()
                .min_w_0()
                .items_center()
                .gap(px(12.))
                .child(chip(account))
                .child(secondary("steam-logout", "Log out", window, cx).on_click(
                    move |_, _, cx| {
                        launcher.update(cx, |launcher, cx| launcher.logout(cx));
                    },
                ))
        };
        let (presence, closing, parallel) = (launcher.clone(), launcher.clone(), launcher.clone());
        let profile = [
            row()
                .child(label("Username"))
                .child(self.username_field(launcher, window, cx)),
            row().child(label("Steam session")).child(session),
        ];
        let presence_row = row()
            .child(label("Discord presence").child(hint(
                "discord-hint",
                "Display the season you are playing as a Discord activity.",
                window,
                cx,
            )))
            .child(switch(
                "discord-presence",
                discord_rpc,
                move |value, _, cx| {
                    presence.update(cx, |launcher, cx| {
                        launcher.settings.set_flag(Flag::DiscordRpc, value, cx)
                    });
                },
                window,
                cx,
            ));
        let tray_row = tray.then(|| {
            row()
                .child(label("Close to tray").child(hint(
                    "tray-hint",
                    "Keep the Launcher running in the tray when the window is closed.",
                    window,
                    cx,
                )))
                .child(switch(
                    "close-to-tray",
                    close_to_tray,
                    move |value, _, cx| {
                        closing.update(cx, |launcher, cx| {
                            launcher.settings.set_flag(Flag::CloseToTray, value, cx)
                        });
                    },
                    window,
                    cx,
                ))
        });
        let parallel_row = row()
            .child(label("Parallel downloads").child(hint(
                "downloads-hint",
                "How many file chunks are downloaded at the same time. A higher value can be faster, but uses more bandwidth and system resources.",
                window,
                cx,
            )))
            .child(self.downloads.build(
                "downloads",
                downloads,
                (DOWNLOADS_MIN, DOWNLOADS_MAX),
                Rc::new(move |next, _, cx| {
                    parallel.update(cx, |launcher, cx| {
                        launcher.settings.set_max_downloads(next, cx)
                    });
                }),
                window,
                cx,
            ));
        let cache_row = row().child(label("App cache")).child(
            secondary("cache-clear", "Clear", window, cx).on_click({
                let launcher = launcher.clone();
                move |_, _, cx| {
                    launcher.update(cx, |launcher, cx| launcher.clear_cache(cx));
                }
            }),
        );
        let rvpn_row = (!IS_WINDOWS).then(|| {
            row()
                .child(label("Radmin VPN"))
                .child(self.rvpn_controls(launcher, window, cx))
        });
        let proton_row = (!IS_WINDOWS).then(|| {
            row()
                .child(label("Proton").child(hint(
                    "proton-hint",
                    "Proton is a compatibility layer that runs Windows games on Linux.",
                    window,
                    cx,
                )))
                .children(self.proton_field(launcher, window, cx))
        });
        let left = column()
            .child(section(profile.into_iter().chain(rvpn_row)))
            .child(section([presence_row].into_iter().chain(tray_row)));
        let right = column()
            .child(section(
                [parallel_row, cache_row].into_iter().chain(proton_row),
            ))
            .child(section(self.library_rows(launcher, window, cx)));
        div()
            .child(sections(viewport).child(left).child(right))
            .children(self.dialog(launcher, window, cx))
    }
}
