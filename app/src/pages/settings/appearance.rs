use std::rc::Rc;
use std::sync::LazyLock;

use crate::ui::{Bound, Handler, bind, px};
use backend::env::SCALE_LEVELS;
use gpui::{
    App, Context, Div, Entity, ParentElement, StatefulInteractiveElement, Styled, Window, div,
};

use super::{SettingsPage, column, section, sections};
use backend::settings::{Color, Flag};

use crate::model::Launcher;
use crate::ui::accent::{self, HUE_MAX, TRACK_STOPS};
use crate::ui::button::secondary;
use crate::ui::kit::{heading, label, row};
use crate::ui::setting::HexSetting;
use crate::ui::slider::{Range, slider};
use crate::ui::stepper::level_stepper;
use crate::ui::swatches::color_bar;
use crate::ui::switch::switch;
use crate::ui::theme::{PALETTES, Theme, hex};
use crate::ui::transfer::transfer_bar;

const BAR_FILLS: [u32; 5] = [
    Color::BarFill.default(),
    0x6aa5fc,
    0x00bdcf,
    0x3cbf85,
    0xe57db1,
];
const BAR_STRIPES: [u32; 5] = [
    Color::BarStripe.default(),
    0xa8cbfe,
    0x87d9e2,
    0x9adab7,
    0xf2b4d0,
];
const PREVIEW_PROGRESS: f32 = 70.;
const LEVEL_STEP: f64 = 0.01;

static HUE_TRACK: LazyLock<[u32; TRACK_STOPS]> = LazyLock::new(accent::hue_track);

pub struct Appearance {
    fill: HexSetting,
    stripe: HexSetting,
    accent: HexSetting,
    backdrop: HexSetting,
    draft: (u32, f64),
    seen: u32,
    level_track: (u32, [u32; 5]),
}

impl Appearance {
    pub fn new(launcher: &Entity<Launcher>, window: &mut Window, cx: &mut App) -> Self {
        let (fill, stripe, color, backdrop) = {
            let settings = &launcher.read(cx).settings;
            (
                settings.color(Color::BarFill),
                settings.color(Color::BarStripe),
                settings.color(Color::Accent),
                settings.color(Color::Background),
            )
        };
        let draft = accent::parts(color);
        Self {
            fill: HexSetting::new(fill, window, cx),
            stripe: HexSetting::new(stripe, window, cx),
            accent: HexSetting::new(color, window, cx),
            backdrop: HexSetting::new(backdrop, window, cx),
            draft,
            seen: color,
            level_track: (draft.0, accent::level_track(draft.0)),
        }
    }

    fn preview(&mut self, draft: (u32, f64), window: &mut Window, cx: &mut App) {
        self.draft = draft;
        Theme::apply(accent::accent(draft.0, draft.1), window, cx);
    }

    fn commit(&mut self, launcher: &Entity<Launcher>, window: &mut Window, cx: &mut App) {
        let value = launcher.read(cx).settings.color(Color::Accent);
        if self.draft == accent::parts(value) {
            Theme::apply(value, window, cx);
            return;
        }
        let color = accent::accent(self.draft.0, self.draft.1);
        self.seen = color;
        launcher.update(cx, |launcher, cx| {
            launcher.settings.set_color(Color::Accent, color, cx)
        });
    }

    fn sliders(
        &mut self,
        launcher: &Entity<Launcher>,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) -> Div {
        let value = launcher.read(cx).settings.color(Color::Accent);
        if self.seen != value {
            self.seen = value;
            self.draft = accent::parts(value);
        }
        let (hue, level) = self.draft;
        if self.level_track.0 != hue {
            self.level_track = (hue, accent::level_track(hue));
        }
        let done: Handler = {
            let launcher = launcher.clone();
            Rc::new(window.handler_for(&cx.entity(), move |page, window, cx| {
                if let Some(appearance) = page.appearance() {
                    appearance.commit(&launcher, window, cx);
                    cx.notify();
                }
            }))
        };
        let set_hue = bind(cx, |page: &mut SettingsPage, value: f64, window, cx| {
            if let Some(appearance) = page.appearance() {
                let level = appearance.draft.1;
                appearance.preview((HUE_MAX - value as u32, level), window, cx);
                cx.notify();
            }
        });
        let set_level = bind(cx, |page: &mut SettingsPage, value: f64, window, cx| {
            if let Some(appearance) = page.appearance() {
                let hue = appearance.draft.0;
                appearance.preview((hue, value), window, cx);
                cx.notify();
            }
        });
        let line = || div().flex().items_center().w_full().h(px(24.));
        div()
            .flex()
            .flex_col()
            .justify_center()
            .gap(px(24.))
            .size_full()
            .child(line().child(slider(
                "accent-hue",
                Range {
                    min: 0.,
                    max: f64::from(HUE_MAX),
                    step: 1.,
                },
                f64::from(HUE_MAX - hue),
                &*HUE_TRACK,
                (set_hue, done.clone()),
                window,
                cx,
            )))
            .child(line().child(slider(
                "accent-level",
                Range {
                    min: 0.,
                    max: 1.,
                    step: LEVEL_STEP,
                },
                level,
                &self.level_track.1,
                (set_level, done),
                window,
                cx,
            )))
    }

    pub fn render(
        &mut self,
        launcher: &Entity<Launcher>,
        viewport: f32,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) -> Div {
        let (fill, stripe, color, backdrop, reduce_motion, scale) = {
            let settings = &launcher.read(cx).settings;
            (
                settings.color(Color::BarFill),
                settings.color(Color::BarStripe),
                settings.color(Color::Accent),
                settings.color(Color::Background),
                settings.flag(Flag::ReduceMotion),
                settings.scale(),
            )
        };
        let store = |target: Color| -> Bound<u32> {
            let launcher = launcher.clone();
            Rc::new(move |color, _, cx| {
                launcher.update(cx, |launcher, cx| {
                    launcher.settings.set_color(target, color, cx)
                })
            })
        };
        let picks = || {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_end()
                .gap(px(8.))
        };
        let cell = || div().flex().flex_1().min_w_0().items_center().h(px(32.));
        let bar_row = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .child(cell().child(heading("Progress bar")))
                    .child(
                        picks()
                            .child(color_bar(
                                "bar-fill",
                                &BAR_FILLS,
                                fill,
                                store(Color::BarFill),
                                window,
                                cx,
                            ))
                            .child(self.fill.build(fill, store(Color::BarFill), window, cx)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .child(
                        cell().child(
                            transfer_bar(
                                "bar-preview",
                                PREVIEW_PROGRESS,
                                (hex(fill), hex(stripe)),
                                false,
                                window,
                                cx,
                            )
                            .w_full(),
                        ),
                    )
                    .child(
                        picks()
                            .child(color_bar(
                                "bar-stripe",
                                &BAR_STRIPES,
                                stripe,
                                store(Color::BarStripe),
                                window,
                                cx,
                            ))
                            .child(
                                self.stripe
                                    .build(stripe, store(Color::BarStripe), window, cx),
                            ),
                    ),
            );
        let (reduce, zoom) = (launcher.clone(), launcher.clone());
        let motion_row = row().child(label("Reduce motion")).child(switch(
            "reduce-motion",
            reduce_motion,
            move |value, _, cx| {
                reduce.update(cx, |launcher, cx| {
                    launcher.settings.set_flag(Flag::ReduceMotion, value, cx)
                });
            },
            window,
            cx,
        ));
        let scale_row = row().child(label("Scale")).child(level_stepper(
            "scale",
            scale,
            &SCALE_LEVELS,
            Rc::new(move |value, _, cx| {
                zoom.update(cx, |launcher, cx| launcher.settings.set_scale(value, cx));
            }),
            window,
            cx,
        ));
        let reset = launcher.clone();
        let sliders = self.sliders(launcher, window, cx);
        let accent_row = div()
            .flex()
            .flex_row_reverse()
            .gap(px(16.))
            .child(div().flex_1().min_w_0().child(sliders))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(px(16.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h(px(32.))
                            .child(heading("Accent")),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h(px(32.))
                            .gap(px(8.))
                            .child(self.accent.build(color, store(Color::Accent), window, cx))
                            .child(secondary("accent-reset", "Reset", window, cx).on_click(
                                move |_, _, cx| {
                                    reset.update(cx, |launcher, cx| {
                                        launcher.settings.set_color(
                                            Color::Accent,
                                            Color::Accent.default(),
                                            cx,
                                        )
                                    });
                                },
                            )),
                    ),
            );
        let backgrounds = PALETTES.map(|palette| palette.key);
        let tone = store(Color::Background);
        let tones = color_bar(
            "background",
            &backgrounds,
            backdrop,
            tone.clone(),
            window,
            cx,
        );
        let typed = self.backdrop.build(backdrop, tone, window, cx);
        let background_row = div()
            .flex()
            .gap(px(16.))
            .child(cell().child(heading("Background")))
            .child(picks().child(tones).child(typed));
        sections(viewport)
            .child(
                column()
                    .child(section([bar_row]))
                    .child(section([motion_row, scale_row])),
            )
            .child(
                column()
                    .child(section([accent_row]))
                    .child(section([background_row])),
            )
    }
}
