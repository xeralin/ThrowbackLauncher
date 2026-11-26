use std::collections::HashMap;
use std::rc::Rc;

use crate::ui::{frame, px, units};
use backend::manifest::SeasonEntry;
use gpui::{
    AnyElement, AppContext, Context, Div, ElementId, Entity, FocusHandle, InteractiveElement,
    IntoElement, KeystrokeEvent, ParentElement, Render, RetainAllImageCache, SharedString,
    Stateful, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder,
};

use crate::model::Launcher;
use crate::shell;
use crate::ui::input::{Hooks, TextInput, text_field, typing};
use crate::ui::key_art::key_art;
use crate::ui::modal::covering;
use crate::ui::motion::{self, Bezier, EASE_IN_OUT, FAST, SLOW, sub};
use crate::ui::scroll::reveal;
use crate::ui::theme::{
    BLACK, BORDER, FONT_BODY, FONT_BODY_SEMIBOLD, FONT_DISPLAY_BOLD, ON_ART, REM, TEXT, TEXT_MUTED,
    Typeface,
};

const BANNER: f32 = 210.;
const TITLE: f32 = 1.9 * REM;
const TITLE_TOP: f32 = 89.;
const NOTE: f32 = 0.78 * REM;
const SEARCH_WIDTH: f32 = 200.;
const SEARCH_LENGTH: usize = 25;
const ZOOM: f32 = 0.04;
const ZOOM_EASE: Bezier = Bezier(0., 0., 0.58, 1.);
const VEIL: (f32, f32) = (0.32, 0.12);

pub struct DownloadPage {
    seasons: Vec<(SeasonEntry, String)>,
    stops: HashMap<String, FocusHandle>,
    search: Entity<TextInput>,
    art: Entity<RetainAllImageCache>,
    _subscriptions: [Subscription; 2],
}

fn title(content: impl Into<SharedString>) -> Div {
    div()
        .face(FONT_DISPLAY_BOLD, TITLE)
        .line_height(px(TITLE))
        .text_color(ON_ART.solid())
        .child(content.into())
}

impl DownloadPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let seasons = Launcher::handle(cx).read(cx).seasons();
        let seasons = seasons.into_iter().map(|season| {
            let edition = if season.hm_available {
                "heated metal hm"
            } else {
                ""
            };
            let words = format!("{} {} {edition}", season.code, season.name).to_lowercase();
            (season, words)
        });
        let search = cx.new(|cx| {
            let mut input = TextInput::new("", window, cx).placeholder("Search");
            input.hooks = Hooks {
                max_length: Some(SEARCH_LENGTH),
                revert: Some(Rc::new(|input, cx| input.set_text("", cx))),
                ..Hooks::default()
            };
            input
        });
        let page = cx.weak_entity();
        let subscriptions = [
            cx.observe(&search, |_, _, cx| cx.notify()),
            cx.intercept_keystrokes(move |event, window, cx| {
                page.update(cx, |page, cx| page.typed(event, window, cx))
                    .ok();
            }),
        ];
        let seasons: Vec<(SeasonEntry, String)> = seasons.collect();
        let stop = |(season, _): &(SeasonEntry, String)| {
            let stop = cx.focus_handle().tab_index(0).tab_stop(true);
            (season.id.clone(), stop)
        };
        Self {
            stops: seasons.iter().map(stop).collect(),
            seasons,
            search,
            art: RetainAllImageCache::new(cx),
            _subscriptions: subscriptions,
        }
    }

    pub fn stop(&self, id: &str) -> Option<FocusHandle> {
        self.stops.get(id).cloned()
    }

    #[cfg(test)]
    pub fn searched(&self, cx: &gpui::App) -> String {
        self.search.read(cx).text().to_owned()
    }

    fn typed(&mut self, event: &KeystrokeEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let chord = modifiers.control || modifiers.alt || modifiers.platform;
        if chord || typing(window) || covering(window) || shell::season_open(cx) {
            return;
        }
        let filled = !self.search.read(cx).text().is_empty();
        let letter = keystroke.key_char.as_deref().filter(|text| {
            let mut letters = text.chars();
            letters
                .next()
                .is_some_and(|letter| !letter.is_control() && letter != ' ')
                && letters.next().is_none()
        });
        let handled = self.search.update(cx, |search, cx| {
            match (keystroke.key.as_str(), letter) {
                ("escape", _) if filled => search.set_text("", cx),
                ("backspace", _) if filled => search.pop(window, cx),
                (_, Some(letter)) => search.append(letter, window, cx),
                _ => return false,
            }
            true
        });
        if handled {
            cx.stop_propagation();
        }
    }

    pub fn tools(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let field = text_field(&self.search, window, cx).w(px(SEARCH_WIDTH));
        Some(field.into_any_element())
    }

    fn banner(
        &self,
        season: &SeasonEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = ElementId::Name(season.id.clone().into());
        let hover = motion::hover(&id, SLOW, window, cx);
        let focus = &self.stops[&season.id];
        let ringed = focus.is_focused(window) && window.last_input_was_keyboard();
        let lit = if hover.over || ringed { 1. } else { 0. };
        let veil = motion::transition(sub(&id, "veil"), lit, SLOW, EASE_IN_OUT, window, cx);
        let zoom = motion::transition(sub(&id, "zoom"), lit, FAST, ZOOM_EASE, window, cx);
        let zoom = if cx.reduce_motion() { 0. } else { zoom };
        div()
            .id(id)
            .absolute()
            .left_0()
            .right_0()
            .h(px(BANNER))
            .overflow_hidden()
            .track_focus(focus)
            .map(reveal)
            .map(|element| hover.follow(element))
            .on_click({
                let key = season.key.clone();
                move |_, window, cx| shell::open_season(&key, false, window, cx)
            })
            .child(key_art(&season.key, false, 0., 1. + ZOOM * zoom, &self.art))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(BLACK.alpha(VEIL.0 + (VEIL.1 - VEIL.0) * veil)),
            )
            .child(
                title(season.code.clone())
                    .absolute()
                    .top(px(TITLE_TOP))
                    .left(px(40.)),
            )
            .child(
                div()
                    .absolute()
                    .top(px(TITLE_TOP))
                    .left(px(112.))
                    .right(px(112.))
                    .flex()
                    .justify_center()
                    .child(title(season.name.clone())),
            )
            .when(ringed, |banner| {
                banner.child(
                    div()
                        .absolute()
                        .inset_0()
                        .border_1()
                        .border_color(TEXT.solid()),
                )
            })
    }

    fn nothing(query: &str) -> Div {
        div().flex().child(
            div()
                .flex()
                .max_w(px(640.))
                .rounded(px(8.))
                .border_1()
                .border_color(BORDER.solid())
                .px(px(0.6 * REM))
                .py(px(0.45 * REM))
                .face(FONT_BODY, NOTE)
                .line_height(px(1.45 * NOTE))
                .text_color(TEXT_MUTED.solid())
                .whitespace_nowrap()
                .child("No results for ")
                .child(
                    div()
                        .face(FONT_BODY_SEMIBOLD, NOTE)
                        .line_height(px(1.45 * NOTE))
                        .text_color(TEXT.solid())
                        .child(query.to_owned()),
                )
                .child("."),
        )
    }
}

impl Render for DownloadPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).text().trim().to_lowercase();
        let shown: Vec<&SeasonEntry> = self
            .seasons
            .iter()
            .filter(|(_, words)| words.contains(&query))
            .map(|(season, _)| season)
            .collect();
        let pad = shell::page_pad(units(frame::area(window).size.width));
        if shown.is_empty() {
            return Self::nothing(self.search.read(cx).text().trim()).p(px(pad));
        }
        let top = shell::page_top(cx);
        let bottom = top + shell::page_size(window).height + px(2. * pad);
        let first = ((top / px(BANNER)) as usize).saturating_sub(1);
        let last = (bottom / px(BANNER)).ceil() as usize + 1;
        let height = BANNER * shown.len() as f32;
        let banners: Vec<_> = shown
            .into_iter()
            .enumerate()
            .skip(first)
            .take(last - first)
            .map(|(index, season)| {
                self.banner(season, window, cx)
                    .top(px(BANNER * index as f32))
            })
            .collect();
        div().h(px(height)).children(banners)
    }
}
