use crate::ui::px;
use backend::env::IS_WINDOWS;
use gpui::{
    Anchor, App, AppContext, Bounds, Context, Deferred, DismissEvent, Entity, EventEmitter,
    FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Pixels, Point,
    Render, Size, StatefulInteractiveElement, Styled, Subscription, Window, anchored, deferred,
    div, prelude::FluentBuilder,
};

use super::SeasonPage;
use super::manage::{Act, Flags, Tool, shelf};
use crate::model::download::State;
use crate::model::launch::Installs;
use crate::ui::Dismiss;
use crate::ui::button::icon_button;
use crate::ui::icon::{CLOCK, COPY, CROSS, FOLDER, PROTON, REFRESH, REMOVE};
use crate::ui::kit::{OVERLAY, disable, icon_box};
use crate::ui::theme::{SURFACE, lifted};

const EDGE: f32 = 8.;
const MOST: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    Act(Act),
    Tool(Tool),
}

impl Entry {
    fn glyph(self) -> &'static [u8] {
        match self {
            Entry::Act(Act::Dequeue) => CROSS,
            Entry::Act(Act::Verify) => REFRESH,
            Entry::Act(_) | Entry::Tool(Tool::Uninstall) => REMOVE,
            Entry::Tool(Tool::Clock) => CLOCK,
            Entry::Tool(Tool::CrashLog) => COPY,
            Entry::Tool(Tool::Proton) => PROTON,
            Entry::Tool(Tool::Folder) => FOLDER,
        }
    }
}

fn entries(
    flags: &Flags,
    installs: &Installs,
    hm: bool,
    any_playtime: bool,
    protons: Option<usize>,
) -> Vec<(Entry, bool)> {
    let install = installs.edition(hm);
    let playing = flags.playing_season;
    if flags.transferring() {
        let paused = flags.state == State::Paused;
        return Vec::from_iter(paused.then_some((Entry::Act(Act::Remove), true)));
    }
    if install.installed {
        let first = if flags.queued {
            (Entry::Act(Act::Dequeue), true)
        } else {
            (Entry::Act(Act::Verify), !playing)
        };
        let tools = shelf(flags, installs, hm, 0, any_playtime, protons).tools;
        let tools = tools.into_iter();
        let tools = tools.map(|(tool, enabled)| (Entry::Tool(tool), enabled));
        return std::iter::once(first).chain(tools).collect();
    }
    if install.partial && !flags.queued {
        return vec![
            (Entry::Act(Act::Remove), !playing),
            (Entry::Tool(Tool::Clock), any_playtime),
        ];
    }
    Vec::new()
}

pub struct SeasonMenu {
    season: Entity<SeasonPage>,
    at: Point<Pixels>,
    open: bool,
    greeted: bool,
    scope: FocusHandle,
    stops: [FocusHandle; MOST],
    opener: Option<FocusHandle>,
    _watch: [Subscription; 2],
}

impl EventEmitter<DismissEvent> for SeasonMenu {}

impl SeasonMenu {
    pub fn open(
        key: &str,
        hm: bool,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Entity<Self>> {
        let page = SeasonPage::new(key, hm, cx)?;
        let opener = window.focused(cx);
        let season = cx.new(|_| page);
        season.update(cx, |page, cx| page.attach(window, cx));
        Some(cx.new(|cx: &mut Context<Self>| {
            let scope = cx.focus_handle();
            let watch = [
                cx.observe_in(&season, window, |menu, _, window, cx| {
                    menu.settle(window, cx)
                }),
                cx.on_focus_out(&scope, window, |menu, _, window, cx| menu.close(window, cx)),
            ];
            cx.defer_in(window, Self::settle);
            Self {
                season,
                at,
                open: true,
                greeted: false,
                scope,
                stops: std::array::from_fn(|_| cx.focus_handle().tab_index(0).tab_stop(true)),
                opener,
                _watch: watch,
            }
        }))
    }

    fn entries(&self, cx: &App) -> Option<Vec<(Entry, bool)>> {
        let page = self.season.read(cx);
        let launcher = page.launcher.read(cx);
        let protons = match &launcher.settings.protons {
            _ if IS_WINDOWS => None,
            Some(protons) => Some(protons.len()),
            None => return None,
        };
        let any_playtime = !launcher.playtime().is_empty();
        let flags = page.flags(launcher);
        Some(entries(
            &flags,
            &page.installs,
            page.hm,
            any_playtime,
            protons,
        ))
    }

    fn enabled(&self, cx: &App) -> Vec<FocusHandle> {
        let entries = self.entries(cx).unwrap_or_default();
        let stops = entries.iter().zip(&self.stops);
        stops
            .filter(|((_, enabled), _)| *enabled)
            .map(|(_, stop)| stop.clone())
            .collect()
    }

    fn settle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        if self.open && self.entries(cx).is_some() {
            match self.enabled(cx).first() {
                Some(first) if !self.greeted => {
                    self.greeted = true;
                    window.focus(first, cx);
                }
                Some(_) => {}
                None => self.open = false,
            }
        }
        if !self.open && self.season.read(cx).dialog.is_none() {
            self.release(window, cx);
            cx.emit(DismissEvent);
        }
    }

    fn release(&self, window: &mut Window, cx: &mut App) {
        let held = |stop: &FocusHandle| stop.is_focused(window);
        if held(&self.scope) || self.stops.iter().any(held) {
            match &self.opener {
                Some(opener) => window.focus(opener, cx),
                None => window.blur(cx),
            }
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.open) {
            self.settle(window, cx);
        }
    }

    fn pick(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.release(window, cx);
        self.season.update(cx, |page, cx| match entry {
            Entry::Act(act) => page.act(act, window, cx),
            Entry::Tool(tool) => page.use_tool(tool, window, cx),
        });
        self.settle(window, cx);
    }

    fn step(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stops = self.enabled(cx);
        let Some(last) = stops.len().checked_sub(1) else {
            return;
        };
        let current = stops.iter().position(|stop| stop.is_focused(window));
        let next = match event.keystroke.key.as_str() {
            "home" => 0,
            "end" => last,
            "down" | "right" => current.map_or(0, |index| (index + 1) % stops.len()),
            "up" | "left" => current.map_or(last, |index| (index + last) % stops.len()),
            _ => return,
        };
        window.focus(&stops[next], cx);
        cx.stop_propagation();
    }

    fn bar(
        &self,
        entries: Vec<(Entry, bool)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Deferred {
        let mut bar = icon_box()
            .id("season-menu")
            .occlude()
            .bg(SURFACE.solid())
            .shadow(lifted())
            .track_focus(&self.scope)
            .on_action(cx.listener(|menu, _: &Dismiss, window, cx| menu.close(window, cx)))
            .on_key_down(cx.listener(Self::step))
            .on_mouse_down_out(cx.listener(|menu, _, window, cx| menu.close(window, cx)));
        for (index, ((entry, enabled), stop)) in entries.into_iter().zip(&self.stops).enumerate() {
            bar = bar.child(
                icon_button(("season-menu-tool", index), entry.glyph(), window, cx)
                    .track_focus(stop)
                    .on_click(cx.listener(move |menu, _, window, cx| menu.pick(entry, window, cx)))
                    .when(!enabled, disable),
            );
        }
        let placed = anchored()
            .attach(Bounds::new(self.at, Size::default()), Anchor::BottomLeft)
            .snap_to_window_with_margin(px(EDGE));
        deferred(placed.child(bar)).with_priority(OVERLAY)
    }
}

impl Render for SeasonMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dialog = self.season.update(cx, |page, cx| page.dialog(window, cx));
        let entries = self.entries(cx).filter(|_| self.open);
        div()
            .children(entries.map(|entries| self.bar(entries, window, cx)))
            .children(dialog)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::launch::Install;

    fn installed(hm: bool, crash_log: bool) -> Installs {
        let present = Install {
            installed: true,
            ..Install::default()
        };
        Installs {
            tb: if hm { Install::default() } else { present },
            hm: Install {
                installed: hm,
                crash_log,
                ..Install::default()
            },
            ..Installs::default()
        }
    }

    fn partial() -> Installs {
        Installs {
            tb: Install {
                partial: true,
                ..Install::default()
            },
            ..Installs::default()
        }
    }

    fn act(act: Act, enabled: bool) -> (Entry, bool) {
        (Entry::Act(act), enabled)
    }

    fn tool(tool: Tool, enabled: bool) -> (Entry, bool) {
        (Entry::Tool(tool), enabled)
    }

    #[test]
    fn an_installed_edition_offers_verify_play_time_proton_folder_and_uninstall() {
        let idle = Flags::default();
        assert_eq!(
            entries(&idle, &installed(false, false), false, true, Some(2)),
            [
                act(Act::Verify, true),
                tool(Tool::Clock, true),
                tool(Tool::Proton, true),
                tool(Tool::Folder, true),
                tool(Tool::Uninstall, true),
            ]
        );
        assert_eq!(
            entries(&idle, &installed(false, false), false, false, None),
            [
                act(Act::Verify, true),
                tool(Tool::Clock, false),
                tool(Tool::Folder, true),
                tool(Tool::Uninstall, true),
            ]
        );
    }

    #[test]
    fn heated_metal_adds_the_crash_log_and_a_running_game_blocks_changes() {
        let playing = Flags {
            playing_season: true,
            ..Flags::default()
        };
        assert_eq!(
            entries(&playing, &installed(true, true), true, true, Some(0)),
            [
                act(Act::Verify, false),
                tool(Tool::Clock, true),
                tool(Tool::CrashLog, true),
                tool(Tool::Proton, false),
                tool(Tool::Folder, true),
                tool(Tool::Uninstall, false),
            ]
        );
        let other = Flags {
            downloading: true,
            downloading_season: true,
            ..Flags::default()
        };
        let listed = entries(&other, &installed(true, false), true, false, None);
        assert_eq!(listed[2], tool(Tool::CrashLog, false));
        assert_eq!(listed[4], tool(Tool::Uninstall, false));
    }

    #[test]
    fn a_queued_edition_leaves_the_queue_instead_of_verifying() {
        let queued = Flags {
            queued: true,
            ..Flags::default()
        };
        let listed = entries(&queued, &installed(false, false), false, false, None);
        assert_eq!(listed[0], act(Act::Dequeue, true));
        assert!(entries(&queued, &partial(), false, true, None).is_empty());
    }

    #[test]
    fn a_transfer_offers_only_removing_a_paused_download() {
        let running = Flags {
            downloading: true,
            downloading_season: true,
            downloading_edition: true,
            state: State::Downloading,
            ..Flags::default()
        };
        assert!(entries(&running, &partial(), false, true, Some(1)).is_empty());
        let paused = Flags {
            state: State::Paused,
            ..Flags::default()
        };
        assert_eq!(
            entries(&paused, &partial(), false, true, Some(1)),
            [act(Act::Remove, true)]
        );
    }

    #[test]
    fn a_partial_download_offers_removal_and_play_time() {
        let idle = Flags::default();
        assert_eq!(
            entries(&idle, &partial(), false, false, Some(1)),
            [act(Act::Remove, true), tool(Tool::Clock, false)]
        );
        let playing = Flags {
            playing_season: true,
            ..Flags::default()
        };
        assert_eq!(
            entries(&playing, &partial(), false, true, Some(1))[0],
            act(Act::Remove, false)
        );
        assert!(entries(&idle, &Installs::default(), false, true, Some(1)).is_empty());
    }
}
