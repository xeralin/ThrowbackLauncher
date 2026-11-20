use std::collections::HashMap;
use std::f32::consts::{PI, TAU};
use std::mem;
use std::time::{Duration, Instant};

use crate::ui::px;
use backend::update::{Component, Failure, Release};
use chrono::{DateTime, Local};
use gpui::{
    AnyElement, App, Bounds, Context, CursorStyle, Deferred, Div, ElementId, Entity, FocusHandle,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, Rgba, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Subscription, Transformation, Window, canvas, div,
    linear_color_stop, linear_gradient, prelude::FluentBuilder, radians,
};

use crate::model::Launcher;
use crate::model::update::Updater;
use crate::notices::RATE_LIMIT;
use crate::ui::button::{glyph_button, icon_button, primary};
use crate::ui::icon::{CHEVRON_DOWN, GITHUB, PACKAGE, REFRESH, icon};
use crate::ui::kit::{disable, external, icon_box, list_row, ring, truncate, version_chip};
use crate::ui::markdown::{self, Block};
use crate::ui::modal::Modal;
use crate::ui::motion::{self, EASE, EASE_IN_OUT, EASE_OUT_CUBIC, FAST, SLOW, sub};
use crate::ui::prose;
use crate::ui::scroll::reveal;
use crate::ui::theme::{
    BG, BLACK, BORDER, FONT_BODY, FONT_DISPLAY_BOLD, REM, SURFACE, SURFACE_2, TEXT, TEXT_MUTED,
    TEXT_UI, Theme, Typeface, WELL, shadow,
};
use crate::ui::toast;

const UPDATE_TOAST: &str = "update";
const WIDTH: f32 = 600.;
const PAD_X: f32 = 0.6 * REM;
const PAD_Y: f32 = 0.45 * REM;
const SPIN: Duration = Duration::from_millis(500);
const DIMMED: f32 = 0.55;
const FILL_EDGE: f32 = 0.002;
const REVEAL: Duration = Duration::from_millis(350);
const SETTLE: Duration = Duration::from_millis(400);
const STAGGER: Duration = Duration::from_millis(60);
const REVEAL_SHARE: f32 = 0.08;
const REVEAL_MARGIN: f32 = 30.;
const CORNER_MASK: f32 = 3.;
const CAP_SHIFT: f32 = 1.;
const SEGMENT_PAD: f32 = 9.5;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    Ready,
    Blocked,
    Applying(u8),
}

impl Stage {
    fn of(component: Component, update: &Updater, downloading: bool) -> Self {
        if update.applying() == Some(component) {
            Stage::Applying(update.progress())
        } else if update.busy() || update.checking() || downloading {
            Stage::Blocked
        } else {
            Stage::Ready
        }
    }
}

#[derive(Debug, PartialEq)]
enum Report {
    Show(SharedString, &'static str),
    Dismiss(&'static str),
}

fn report(failure: Option<&Failure>, found: bool, manual: bool) -> Option<Report> {
    match failure {
        Some(Failure::RateLimit(detail)) => Some(Report::Show(detail.clone().into(), RATE_LIMIT)),
        Some(Failure::Error) => Some(Report::Show("Update check failed".into(), UPDATE_TOAST)),
        None if found => Some(Report::Dismiss(UPDATE_TOAST)),
        None if manual => Some(Report::Show(
            "Everything is up to date".into(),
            UPDATE_TOAST,
        )),
        None => None,
    }
}

fn release_date(date: &str) -> Option<SharedString> {
    let moment = DateTime::parse_from_rfc3339(date).ok()?;
    let local = moment.with_timezone(&Local);
    Some(local.format("%b %-d, %Y").to_string().into())
}

fn spin(serial: usize, window: &mut Window, cx: &mut App) -> f32 {
    if serial == 0 {
        return 0.;
    }
    let elapsed = motion::clock(("updates-spin", serial), window, cx);
    let progress = elapsed.as_secs_f32() / SPIN.as_secs_f32();
    if progress >= 1. {
        return 0.;
    }
    window.request_animation_frame();
    TAU * EASE_OUT_CUBIC.at(progress)
}

fn in_view(bounds: Bounds<Pixels>, window: &Window) -> bool {
    let floor = window.viewport_size().height - px(REVEAL_MARGIN);
    let seen = bounds.bottom().min(floor) - bounds.top().max(px(0.));
    seen >= bounds.size.height * REVEAL_SHARE
}

fn boxed() -> Div {
    div()
        .rounded(px(8.))
        .border_1()
        .border_color(BORDER.solid())
        .bg(WELL.solid())
}

fn caption(content: impl Into<SharedString>) -> Div {
    div()
        .top(px(CAP_SHIFT))
        .face(FONT_DISPLAY_BOLD, TEXT_UI)
        .whitespace_nowrap()
        .child(content.into())
}

fn row() -> Div {
    div()
        .border_t_1()
        .border_color(BORDER.solid())
        .px(px(PAD_X))
        .py(px(PAD_Y))
}

struct Offer {
    component: Component,
    target: String,
    url: SharedString,
    date: Option<SharedString>,
    stage: Stage,
}

struct Row {
    date: Option<SharedString>,
    open: bool,
    notes: Option<Vec<Block>>,
    shown: Option<Instant>,
}

pub struct UpdatesPage {
    launcher: Entity<Launcher>,
    history: Vec<(Component, Vec<Release>)>,
    picked: Option<Component>,
    rows: Vec<Row>,
    parsed: HashMap<&'static str, (String, Vec<Block>)>,
    installed: Vec<(Component, String)>,
    modal: Modal,
    refresh: FocusHandle,
    spins: usize,
    manual: bool,
    checking: bool,
    boxes: usize,
    shrunk: bool,
    settled: Instant,
    next_reveal: Instant,
    _subscriptions: [Subscription; 2],
}

impl UpdatesPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let launcher = Launcher::handle(cx);
        let subscriptions = [
            cx.observe(&launcher, Self::changed),
            cx.on_focus_lost(window, |this, window, cx| {
                if mem::take(&mut this.shrunk) {
                    window.focus(&this.refresh, cx);
                }
            }),
        ];
        let now = cx.background_executor().now();
        let update = &launcher.read(cx).update;
        let mut page = Self {
            checking: update.checking(),
            boxes: update.pending().len(),
            history: update.history().to_vec(),
            launcher,
            picked: None,
            rows: Vec::new(),
            parsed: HashMap::new(),
            installed: Vec::new(),
            modal: Modal::new(cx),
            refresh: cx.focus_handle().tab_index(0).tab_stop(true),
            spins: 0,
            manual: false,
            shrunk: false,
            settled: now + SETTLE,
            next_reveal: now,
            _subscriptions: subscriptions,
        };
        page.fresh_rows();
        page
    }

    fn shown(&self) -> Option<&(Component, Vec<Release>)> {
        let mut listed = self.history.iter().filter(|entry| !entry.1.is_empty());
        let first = listed.clone().next();
        listed.find(|entry| Some(entry.0) == self.picked).or(first)
    }

    fn fresh_rows(&mut self) {
        let releases = self.shown().map_or(&[][..], |shown| &shown.1);
        let row = |release: &Release| Row {
            date: release_date(&release.notes.date),
            open: false,
            notes: None,
            shown: None,
        };
        self.rows = releases.iter().map(row).collect();
    }

    fn pick(&mut self, component: Component, cx: &mut Context<Self>) {
        if self.shown().is_some_and(|shown| shown.0 != component) {
            self.picked = Some(component);
            self.fresh_rows();
            cx.notify();
        }
    }

    fn toggle(&mut self, index: usize, cx: &mut Context<Self>) {
        let notes = match (self.shown(), self.rows.get(index)) {
            (Some((component, releases)), Some(row)) if row.notes.is_none() => releases
                .get(index)
                .map(|release| markdown::parse(&release.notes.body, component.repository())),
            _ => None,
        };
        if let Some(row) = self.rows.get_mut(index) {
            row.open = !row.open;
            row.notes = row.notes.take().or(notes);
            cx.notify();
        }
    }

    fn reveal(&mut self, index: usize, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let instant = cx.reduce_motion() || now < self.settled;
        let done = if instant {
            now
        } else {
            let start = self.next_reveal.max(now);
            self.next_reveal = start + STAGGER;
            start + REVEAL
        };
        if let Some(row) = self.rows.get_mut(index) {
            row.shown = Some(done);
        }
    }

    fn recheck(&mut self, cx: &mut Context<Self>) {
        self.manual = true;
        self.spins += 1;
        self.launcher
            .update(cx, |launcher, cx| launcher.check_updates(true, cx));
        cx.notify();
    }

    fn changed(&mut self, launcher: Entity<Launcher>, cx: &mut Context<Self>) {
        let update = &launcher.read(cx).update;
        let checking = update.checking();
        let done = mem::replace(&mut self.checking, checking) && !checking;
        let report = if done {
            let found = !update.pending().is_empty();
            report(update.failure(), found, mem::take(&mut self.manual))
        } else {
            None
        };
        if done && update.history() != self.history {
            self.history = update.history().to_vec();
            self.fresh_rows();
        }
        match report {
            Some(Report::Show(text, key)) => toast::show(text, Some(key), cx),
            Some(Report::Dismiss(key)) => toast::dismiss(key, cx),
            None => {}
        }
        cx.notify();
    }

    fn list_installed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let versions = self.launcher.read(cx).installed_versions(cx);
        cx.spawn_in(window, async move |this, cx| {
            let versions = versions.await;
            this.update_in(cx, |this, window, cx| {
                this.installed = versions;
                this.modal.open(window, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn tools(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let busy = self.launcher.read(cx).update.busy();
        let turn = Transformation::rotate(radians(spin(self.spins, window, cx)));
        let arrows = icon(REFRESH, 16.).with_transformation(turn);
        let refresh = glyph_button("updates-refresh", arrows, window, cx)
            .p(px(4.))
            .track_focus(&self.refresh.clone().tab_stop(!busy));
        let refresh = if busy {
            disable(refresh)
        } else {
            refresh.on_click(cx.listener(|this, _, _, cx| this.recheck(cx)))
        };
        let installed = icon_button("updates-installed", PACKAGE, window, cx)
            .on_click(cx.listener(|this, _, window, cx| this.list_installed(window, cx)));
        let tools = icon_box()
            .bg(SURFACE_2.solid())
            .child(installed)
            .child(refresh);
        Some(tools.into_any_element())
    }

    fn update_button(
        &self,
        component: Component,
        stage: Stage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = ElementId::from(("update", component as usize));
        let applying = matches!(stage, Stage::Applying(_));
        let dim = if applying { 1. } else { 0. };
        let dim = motion::transition(sub(&id, "dim"), dim, SLOW, EASE_IN_OUT, window, cx);
        let button = primary(id.clone(), "Update", window, cx);
        let progress = match stage {
            Stage::Applying(progress) => progress,
            Stage::Blocked => return disable(button),
            Stage::Ready => {
                let launcher = self.launcher.clone();
                return button.on_click(move |_, _, cx| {
                    launcher.update(cx, |launcher, cx| launcher.apply_update(component, cx));
                });
            }
        };
        let lit = motion::hover(&id, SLOW, window, cx).amount;
        let share = f32::from(progress) / 100.;
        let fill = motion::transition(sub(&id, "fill"), share, SLOW, EASE_IN_OUT, window, cx);
        let opacity = if progress > 0 && progress < 100 {
            1.
        } else {
            motion::pulse(window, cx)
        };
        let theme = Theme::get(cx);
        let rest = theme.action.mix(theme.action_deep, lit);
        let dark = rest.mix(theme.action.mix(BLACK, DIMMED), dim);
        let glow = Rgba {
            a: theme.action_glow.a * lit * (1. - dim),
            ..theme.action_glow
        };
        button
            .bg(linear_gradient(
                90.,
                linear_color_stop(theme.action.solid(), fill),
                linear_color_stop(dark.solid(), fill + FILL_EDGE),
            ))
            .shadow(vec![shadow(0., 2., 14., 0., glow.into())])
            .cursor(CursorStyle::OperationNotAllowed)
            .opacity(opacity)
    }

    fn offer(&self, offer: &Offer, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let component = offer.component;
        let key = component as usize;
        let memo = self.parsed.get(component.name());
        let notes = memo.map_or(&[][..], |(_, notes)| notes);
        let footer = (!offer.url.is_empty()).then(|| {
            let id = ("release-page", key);
            let page = prose::link(id, "View on GitHub", offer.url.clone(), window, cx);
            row()
                .flex()
                .items_center()
                .gap(px(16.))
                .face(FONT_BODY, TEXT_UI)
                .text_color(TEXT_MUTED.solid())
                .children(offer.date.clone())
                .child(page.ml_auto())
        });
        let name = div()
            .face(FONT_DISPLAY_BOLD, 1.05 * REM)
            .text_color(TEXT.solid())
            .truncate()
            .child(component.name());
        let title = div()
            .flex()
            .min_w_0()
            .items_center()
            .gap(px(10.))
            .child(name)
            .child(version_chip(&offer.target));
        let button = self.update_button(component, offer.stage, window, cx);
        let head = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(px(16.))
            .gap_y(px(8.))
            .py(px(PAD_Y))
            .pl(px(PAD_X))
            .pr(px(PAD_Y))
            .child(title)
            .child(button.ml_auto());
        boxed()
            .child(head)
            .when(!notes.is_empty(), |card| {
                card.child(row().child(markdown::notes(("offer", key), notes, window, cx)))
            })
            .children(footer)
    }

    fn segment(
        &self,
        component: Component,
        listed: bool,
        active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = ElementId::from(("history", component as usize));
        let hover = motion::hover(&id, FAST, window, cx);
        let lit = if active { 1. } else { 0. };
        let lit = motion::transition(sub(&id, "active"), lit, FAST, EASE_IN_OUT, window, cx);
        let color = if listed {
            TEXT_MUTED.mix(TEXT, lit.max(hover.amount)).solid()
        } else {
            TEXT_MUTED.alpha(0.4)
        };
        let segment = div()
            .id(id)
            .flex()
            .flex_grow_1()
            .items_center()
            .justify_center()
            .h(px(30.))
            .px(px(SEGMENT_PAD))
            .bg(SURFACE.mix(SURFACE_2, lit).solid())
            .text_color(color)
            .child(caption(component.name()));
        if !listed {
            return segment.cursor(CursorStyle::OperationNotAllowed);
        }
        let edge = |x: f32| shadow(x, 0., 0., 0., TEXT.solid());
        segment
            .tab_index(0)
            .focus_visible(move |style| style.shadow(vec![ring().inset(), edge(1.), edge(-1.)]))
            .map(reveal)
            .map(|element| hover.follow(element))
            .on_click(cx.listener(move |this, _, _, cx| this.pick(component, cx)))
    }

    fn entry(
        &self,
        id: ElementId,
        index: usize,
        release: &Release,
        state: &Row,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let hover = motion::hover(&id, FAST, window, cx);
        let open = if state.open { 1. } else { 0. };
        let turn = motion::transition(sub(&id, "turn"), open, SLOW, EASE_IN_OUT, window, cx);
        let grow = motion::transition(sub(&id, "grow"), open, SLOW, EASE_OUT_CUBIC, window, cx);
        let height = window.use_keyed_state(sub(&id, "height"), cx, |_, _| px(0.));
        let focus = window
            .use_keyed_state(sub(&id, "focus"), cx, |_, cx| {
                cx.focus_handle().tab_index(0).tab_stop(true)
            })
            .read(cx)
            .clone();
        let ringed = focus.is_focused(window) && window.last_input_was_keyboard();
        let edge = BORDER.mix(Theme::get(cx).action_edge, hover.amount);
        let now = cx.background_executor().now();
        let fade = state.shown.map_or(0., |done| {
            let left = done.saturating_duration_since(now).as_secs_f32();
            EASE.at((1. - left / REVEAL.as_secs_f32()).max(0.))
        });
        if state.shown.is_some() && fade < 1. {
            window.request_animation_frame();
        }
        let arrow = icon(CHEVRON_DOWN, 14.)
            .text_color(TEXT_MUTED.solid())
            .with_transformation(Transformation::rotate(radians(PI * turn)));
        let header = div()
            .id(id.clone())
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(30.))
            .px(px(PAD_X))
            .rounded(px(6.))
            .track_focus(&focus)
            .map(reveal)
            .map(|element| hover.follow(element))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(index, cx)))
            .children(
                state
                    .date
                    .clone()
                    .map(|date| caption(date).text_color(TEXT.solid())),
            )
            .child(div().flex_1())
            .child(version_chip(&release.version))
            .child(arrow);
        let notes = state.notes.as_deref().filter(|notes| !notes.is_empty());
        let body = notes.filter(|_| state.open || grow > 0.).map(|notes| {
            let natural = *height.read(cx);
            div()
                .overflow_hidden()
                .when(grow < 1., |body| body.h(natural * grow))
                .on_children_prepainted(move |bounds, window, cx| {
                    let measured = bounds.first().map_or(px(0.), |bounds| bounds.size.height);
                    if *height.read(cx) != measured {
                        height.update(cx, |height, _| *height = measured);
                        window.request_animation_frame();
                    }
                })
                .child(row().child(markdown::notes(sub(&id, "notes"), notes, window, cx)))
        });
        let sensor = state.shown.is_none().then(|| {
            let page = cx.weak_entity();
            let sense = move |bounds, window: &mut Window, cx: &mut App| {
                if in_view(bounds, window) {
                    page.update(cx, |page, cx| page.reveal(index, cx)).ok();
                    window.request_animation_frame();
                }
            };
            canvas(sense, |_, (), _, _| {}).absolute().size_full()
        });
        div()
            .opacity(fade)
            .child(
                boxed()
                    .border_color(edge.solid())
                    .child(header)
                    .children(body),
            )
            .children(sensor)
            .when(ringed, |entry| {
                entry.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(32.))
                        .rounded(px(7.))
                        .border_2()
                        .border_color(TEXT.solid()),
                )
            })
    }

    fn history(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<Div> {
        let (shown, releases) = self.shown()?;
        let mut selector = div()
            .self_start()
            .flex()
            .flex_wrap()
            .gap(px(1.))
            .rounded(px(8.))
            .border_1()
            .border_color(BORDER.solid())
            .bg(BORDER.solid());
        for (component, listed) in &self.history {
            let active = component == shown;
            let segment = self.segment(*component, !listed.is_empty(), active, window, cx);
            selector = selector.child(segment);
        }
        let mask = div()
            .absolute()
            .inset(px(-1. - CORNER_MASK))
            .rounded(px(8. + CORNER_MASK))
            .border(px(CORNER_MASK))
            .border_color(BG.solid());
        let mut list = div().flex().flex_col().gap(px(8.));
        for (index, (release, state)) in releases.iter().zip(&self.rows).enumerate() {
            let id = ElementId::from((SharedString::new_static(shown.name()), index));
            list = list.child(self.entry(id, index, release, state, window, cx));
        }
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(16.))
                .max_w(px(WIDTH))
                .child(selector.child(mask))
                .child(list),
        )
    }

    fn versions(&self, window: &mut Window, cx: &mut Context<Self>) -> Deferred {
        let mut list = div().flex().flex_col().gap(px(8.));
        for (component, version) in &self.installed {
            let id = ElementId::from(("repository", *component as usize));
            let hover = motion::hover(&id, FAST, window, cx);
            let logo = icon(GITHUB, 16.).text_color(TEXT_MUTED.mix(TEXT, hover.amount).solid());
            let link = div()
                .flex()
                .flex_none()
                .rounded(px(6.))
                .cursor_pointer()
                .child(logo);
            let url = format!("https://github.com/{}", component.repository());
            list = list.child(
                list_row()
                    .child(truncate(component.name()))
                    .child(version_chip(version))
                    .child(external(id, link, url, &hover, window, cx)),
            );
        }
        let page = cx.weak_entity();
        let close = move |window: &mut Window, cx: &mut App| {
            page.update(cx, |page, cx| {
                page.modal.close(window, cx);
                cx.notify();
            })
            .ok();
        };
        let sheet = self.modal.sheet(list).title("Installed versions");
        sheet.on_close(close).show(window, cx)
    }
}

impl Render for UpdatesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let offers: Vec<Offer> = {
            let launcher = self.launcher.read(cx);
            let downloading = launcher.download.running();
            let pending = launcher.update.pending().iter();
            pending
                .map(|pending| {
                    let component = pending.component;
                    let memo = self.parsed.entry(component.name()).or_default();
                    if memo.0 != pending.notes.body {
                        let notes = markdown::parse(&pending.notes.body, component.repository());
                        *memo = (pending.notes.body.clone(), notes);
                    }
                    Offer {
                        component,
                        target: pending.target.clone(),
                        url: pending.notes.url.clone().into(),
                        date: release_date(&pending.notes.date),
                        stage: Stage::of(component, &launcher.update, downloading),
                    }
                })
                .collect()
        };
        self.shrunk = offers.len() < mem::replace(&mut self.boxes, offers.len());
        let boxes = (!offers.is_empty()).then(|| {
            let mut boxes = div()
                .flex()
                .flex_col()
                .gap(px(16.))
                .mb(px(24.))
                .max_w(px(WIDTH));
            for offer in &offers {
                boxes = boxes.child(self.offer(offer, window, cx));
            }
            boxes
        });
        let history = self.history(window, cx);
        let versions = self.modal.is_open().then(|| self.versions(window, cx));
        div().children(boxes).children(history).children(versions)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use backend::env::{Paths, SEVENZ_ASSET, TL_LAUNCHER, VERSION};
    use backend::manifest;
    use backend::settings::Settings as Store;
    use backend::update::{Check, Install};
    use gpui::{AppContext, TestAppContext, VisualTestContext};
    use tempfile::TempDir;

    use super::*;
    use crate::model::download::{Downloader, State};
    use crate::model::update::Work;
    use crate::ui::testing::{mount, press};

    const LOADER: Component = Component::ThrowbackLoader;
    const METAL: Component = Component::HeatedMetal;
    const ZIP: Component = Component::SevenZip;

    struct Bench {
        _dir: TempDir,
        paths: Paths,
        launcher: Entity<Launcher>,
        page: Entity<UpdatesPage>,
    }

    fn release(tag: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","prerelease":false,"body":"- Fixed **one** thing","html_url":"https://github.com/owner/repo/releases/tag/{tag}","published_at":"2026-11-06T12:00:00Z","assets":[{{"name":"HeatedMetal.7z","url":""}},{{"name":"7z-{SEVENZ_ASSET}","url":""}}]}}"#
        )
    }

    fn answers(paths: &Paths, lists: [&[&str]; 5], newest: &[(Component, &str)]) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let stamped = |url: String, data: String| {
            format!(
                r#""{url}":{{"fetched":{{"secs_since_epoch":{},"nanos_since_epoch":0}},"data":{data}}}"#,
                now.as_secs()
            )
        };
        let base = |component: Component| {
            format!(
                "https://api.github.com/repos/{}/releases",
                component.repository()
            )
        };
        let listed = Component::ALL
            .into_iter()
            .zip(lists)
            .map(|(component, tags)| {
                let releases: Vec<String> = tags.iter().map(|tag| release(tag)).collect();
                stamped(base(component), format!("[{}]", releases.join(",")))
            });
        let latest = newest
            .iter()
            .map(|(component, tag)| stamped(base(*component) + "/latest", release(tag)));
        let entries: Vec<String> = listed.chain(latest).collect();
        fs::create_dir_all(paths.bin_dir()).unwrap();
        fs::write(paths.api_cache_file(), format!("{{{}}}", entries.join(","))).unwrap();
    }

    fn bench(
        cx: &mut TestAppContext,
        prepare: impl FnOnce(&Paths),
    ) -> (Bench, &mut VisualTestContext) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        let library = dir.path().join("lib");
        fs::create_dir_all(&paths.data_root).unwrap();
        fs::write(
            paths.settings_file(),
            format!(
                "[settings]\nlibraries = ['{}']\ndiscord_rpc = false\n",
                library.display()
            ),
        )
        .unwrap();
        let lists: [&[&str]; 5] = [
            &["v0.2.1", "v0.2.0"],
            &[],
            &["26.06"],
            &[],
            &["0.4.2", "0.4.1"],
        ];
        answers(&paths, lists, &[(ZIP, "26.06"), (METAL, "0.4.2")]);
        prepare(&paths);
        let launcher = cx.update(|cx| {
            crate::init(cx);
            let store = Store::load(&paths).unwrap();
            Launcher::new(paths.clone(), store, manifest::downloads()).init(cx);
            let launcher = Launcher::handle(cx);
            launcher.update(cx, |launcher, cx| launcher.check_updates(false, cx));
            launcher
        });
        let slot = Rc::new(RefCell::new(None::<Entity<UpdatesPage>>));
        let made = slot.clone();
        let cx = mount(cx, move |window, cx| {
            let page = made
                .borrow_mut()
                .get_or_insert_with(|| cx.new(|cx| UpdatesPage::new(window, cx)))
                .clone();
            let tools = page.update(cx, |page, cx| page.tools(window, cx));
            div().children(tools).child(page).into_any_element()
        });
        draw(cx);
        let page = slot.borrow().clone().unwrap();
        let bench = Bench {
            _dir: dir,
            paths,
            launcher,
            page,
        };
        (bench, cx)
    }

    fn outdated(paths: &Paths) {
        let metal = paths.data_root.join("../lib/Y5S4_HeatedMetal");
        fs::create_dir_all(metal.join("HeatedMetal")).unwrap();
        fs::write(metal.join(TL_LAUNCHER), "").unwrap();
        fs::write(metal.join("HeatedMetal/.version"), "0.4.1").unwrap();
        fs::write(paths.bin_dir().join("HeatedMetal-0.4.2.7z"), "").unwrap();
        fs::write(paths.sevenz_bin(), "").unwrap();
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn shown(bench: &Bench, cx: &mut VisualTestContext) -> Option<Component> {
        bench
            .page
            .read_with(cx, |page, _| page.shown().map(|shown| shown.0))
    }

    fn stages(bench: &Bench, cx: &mut VisualTestContext) -> Vec<(&'static str, Stage)> {
        bench.launcher.read_with(cx, |launcher, _| {
            let downloading = launcher.download.running();
            let pending = launcher.update.pending().iter();
            pending
                .map(|pending| {
                    let stage = Stage::of(pending.component, &launcher.update, downloading);
                    (pending.component.name(), stage)
                })
                .collect()
        })
    }

    #[gpui::test]
    fn the_history_opens_on_the_first_listed_component_and_skips_empty_ones(
        cx: &mut TestAppContext,
    ) {
        let (bench, cx) = bench(cx, |_| {});
        assert_eq!(shown(&bench, cx), Some(Component::Launcher));
        let rows = |cx: &mut VisualTestContext| bench.page.read_with(cx, |page, _| page.rows.len());
        assert_eq!(rows(cx), 2);

        press(cx, "tab tab tab tab enter");
        assert_eq!(shown(&bench, cx), Some(ZIP));
        assert_eq!(rows(cx), 1);
        press(cx, "tab enter");
        assert_eq!(shown(&bench, cx), Some(METAL));

        bench.page.update(cx, |page, cx| page.pick(LOADER, cx));
        assert_eq!(shown(&bench, cx), Some(Component::Launcher));
        assert_eq!(rows(cx), 2);
    }

    #[gpui::test]
    fn an_entry_opens_with_the_keyboard_and_keeps_its_notes(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, |_| {});
        let rows = |cx: &mut VisualTestContext| {
            bench.page.read_with(cx, |page, _| {
                let state = |row: &Row| (row.open, row.notes.as_ref().map(Vec::len));
                page.rows.iter().map(state).collect::<Vec<_>>()
            })
        };
        assert_eq!(rows(cx), [(false, None), (false, None)]);
        press(cx, "tab tab tab tab tab tab enter");
        assert_eq!(rows(cx), [(true, Some(1)), (false, None)]);
        press(cx, "enter tab enter");
        assert_eq!(rows(cx), [(false, Some(1)), (true, Some(1))]);

        press(cx, "shift-tab shift-tab enter");
        assert_eq!(shown(&bench, cx), Some(METAL));
        assert_eq!(rows(cx), [(false, None), (false, None)]);
    }

    #[gpui::test]
    fn entries_appear_at_once_on_arrival_and_one_after_another_later(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, |_| {});
        let ends = |cx: &mut VisualTestContext| {
            draw(cx);
            let now = cx.executor().now();
            bench.page.read_with(cx, |page, _| {
                let left = |row: &Row| row.shown.map(|done| done.saturating_duration_since(now));
                page.rows.iter().map(left).collect::<Vec<_>>()
            })
        };
        assert_eq!(ends(cx), [Some(Duration::ZERO), Some(Duration::ZERO)]);

        cx.executor().advance_clock(SETTLE);
        bench.page.update(cx, |page, cx| page.pick(ZIP, cx));
        assert_eq!(ends(cx), [Some(REVEAL)]);
        bench
            .page
            .update(cx, |page, cx| page.pick(Component::Launcher, cx));
        assert_eq!(
            ends(cx),
            [Some(REVEAL + STAGGER), Some(REVEAL + 2 * STAGGER)]
        );

        cx.update(|_, cx| cx.set_reduce_motion(true));
        bench.page.update(cx, |page, cx| page.pick(METAL, cx));
        assert_eq!(ends(cx), [Some(Duration::ZERO), Some(Duration::ZERO)]);
    }

    #[gpui::test]
    fn the_package_icon_lists_what_is_installed(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, |_| {});
        let listed = |cx: &mut VisualTestContext| {
            bench.page.read_with(cx, |page, _| {
                page.modal.is_open().then(|| page.installed.clone())
            })
        };
        press(cx, "tab enter");
        cx.run_until_parked();
        assert_eq!(
            listed(cx),
            Some(vec![(Component::Launcher, VERSION.to_owned())])
        );
        press(cx, "escape");
        assert_eq!(listed(cx), None);
        press(cx, "enter");
        cx.run_until_parked();
        assert!(listed(cx).is_some());
    }

    #[derive(Default)]
    struct Asked(Mutex<Vec<bool>>);

    impl Work for Asked {
        fn available(&self, _install: &Install, force: bool) -> Check {
            self.0.lock().unwrap().push(force);
            Check::default()
        }

        fn apply(&self, _: &Install, _: Component, _: &dyn Fn(f64)) -> Result<(), String> {
            unreachable!("refresh never applies an update")
        }

        fn relaunch(&self, _paths: &Paths) {}
    }

    #[gpui::test]
    fn refresh_checks_again_and_marks_the_check_as_asked_for(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, |_| {});
        let asked = Arc::new(Asked::default());
        let work = asked.clone();
        bench.launcher.update(cx, |launcher, _| {
            launcher.update = Updater::with_work(work);
        });
        let state = |cx: &mut VisualTestContext| {
            bench
                .page
                .read_with(cx, |page, _| (page.spins, page.manual, page.checking))
        };
        let forced = || asked.0.lock().unwrap().clone();
        assert_eq!(state(cx), (0, false, false));
        press(cx, "tab tab enter");
        assert_eq!(state(cx), (1, false, false));
        assert_eq!(forced(), [true]);
        bench.page.update(cx, |page, cx| page.recheck(cx));
        assert_eq!(state(cx), (2, true, true));
        cx.run_until_parked();
        assert_eq!(state(cx), (2, false, false));
        assert_eq!(forced(), [true, true]);
    }

    #[test]
    fn a_check_ends_with_the_message_that_fits_its_outcome() {
        let limit = Failure::RateLimit("GitHub rate limit reached, try again later".to_owned());
        let shown = |text: &'static str, key| Some(Report::Show(text.into(), key));
        for manual in [false, true] {
            assert_eq!(
                report(Some(&limit), true, manual),
                shown("GitHub rate limit reached, try again later", "rate-limit")
            );
            assert_eq!(
                report(Some(&Failure::Error), false, manual),
                shown("Update check failed", "update")
            );
            assert_eq!(report(None, true, manual), Some(Report::Dismiss("update")));
        }
        assert_eq!(
            report(None, false, true),
            shown("Everything is up to date", "update")
        );
        assert_eq!(report(None, false, false), None);
    }

    #[gpui::test]
    fn an_update_can_start_only_while_nothing_else_runs(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, outdated);
        let ready = [("7z", Stage::Ready), ("Heated Metal", Stage::Ready)];
        let blocked = [("7z", Stage::Blocked), ("Heated Metal", Stage::Blocked)];
        assert_eq!(stages(&bench, cx), ready);

        bench
            .launcher
            .update(cx, |launcher, cx| launcher.check_updates(false, cx));
        assert_eq!(stages(&bench, cx), blocked);
        cx.run_until_parked();
        assert_eq!(stages(&bench, cx), ready);

        bench.launcher.update(cx, |launcher, cx| {
            launcher.download = Downloader::with(State::Downloading, None, false, Vec::new());
            cx.notify();
        });
        assert_eq!(stages(&bench, cx), blocked);
        bench.launcher.update(cx, |launcher, cx| {
            launcher.download = Downloader::default();
            launcher.apply_update(METAL, cx);
        });
        assert_eq!(
            stages(&bench, cx),
            [("7z", Stage::Blocked), ("Heated Metal", Stage::Applying(0))]
        );
        cx.run_until_parked();
        assert_eq!(stages(&bench, cx), ready);
    }

    #[gpui::test]
    fn focus_moves_to_refresh_when_the_focused_update_goes_away(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, outdated);
        let check = |cx: &mut VisualTestContext| {
            bench
                .launcher
                .update(cx, |launcher, cx| launcher.check_updates(false, cx));
            draw(cx);
            let listed = stages(&bench, cx).len();
            let refresh = bench.page.read_with(cx, |page, _| page.refresh.clone());
            (listed, cx.update(|window, _| refresh.is_focused(window)))
        };
        press(cx, "tab tab tab");
        assert_eq!(check(cx), (2, false));
        fs::remove_file(bench.paths.sevenz_bin()).unwrap();
        assert_eq!(check(cx), (1, true));

        press(cx, "shift-tab");
        fs::write(bench.paths.sevenz_bin(), "").unwrap();
        assert_eq!(check(cx), (2, false));
        fs::remove_file(bench.paths.sevenz_bin()).unwrap();
        assert_eq!(check(cx), (1, false));
        press(cx, "enter");
        cx.run_until_parked();
        assert!(bench.page.read_with(cx, |page, _| page.modal.is_open()));
    }

    #[test]
    fn release_dates_read_as_month_day_and_year() {
        let date = |text: &str| release_date(text).map(String::from);
        assert_eq!(date("2026-11-06T12:00:00Z").as_deref(), Some("Nov 6, 2026"));
        assert_eq!(
            date("2025-05-09T12:43:45+00:00").as_deref(),
            Some("May 9, 2025")
        );
        assert_eq!(date(""), None);
    }
}
