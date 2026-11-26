use std::cell::Cell;

use crate::ui::px;
use backend::settings::Color;
use gpui::{App, BoxShadow, FontWeight, Global, Hsla, Rgba, SharedString, Styled, relative};

#[derive(Clone, Copy, PartialEq)]
pub struct Srgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

#[derive(Clone, Copy)]
struct Oklch {
    l: f32,
    c: f32,
    h: f32,
}

pub const fn hex(value: u32) -> Srgb {
    Srgb {
        r: ((value >> 16) & 0xff) as f32 / 255.,
        g: ((value >> 8) & 0xff) as f32 / 255.,
        b: (value & 0xff) as f32 / 255.,
    }
}

fn to_linear(channel: f32) -> f32 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(channel: f32) -> f32 {
    if channel <= 0.0031308 {
        channel * 12.92
    } else {
        1.055 * channel.powf(1. / 2.4) - 0.055
    }
}

impl Srgb {
    fn to_oklch(self) -> Oklch {
        let (r, g, b) = (to_linear(self.r), to_linear(self.g), to_linear(self.b));
        let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
        let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
        let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
        let lightness = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
        let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
        let b = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
        Oklch {
            l: lightness,
            c: a.hypot(b),
            h: b.atan2(a),
        }
    }

    fn contrast_with_white(self) -> bool {
        let luminance =
            0.2126 * to_linear(self.r) + 0.7152 * to_linear(self.g) + 0.0722 * to_linear(self.b);
        1.05 / (luminance + 0.05) >= (luminance + 0.05) / 0.05
    }

    pub fn rgba(self, alpha: f32) -> Rgba {
        Rgba {
            r: self.r,
            g: self.g,
            b: self.b,
            a: alpha,
        }
    }

    pub fn alpha(self, alpha: f32) -> Hsla {
        self.rgba(alpha).into()
    }

    pub fn solid(self) -> Hsla {
        self.alpha(1.)
    }

    pub fn mix(self, to: impl Into<Srgb>, amount: f32) -> Srgb {
        let to = to.into();
        let channel = |from: f32, to: f32| from + (to - from) * amount;
        Srgb {
            r: channel(self.r, to.r),
            g: channel(self.g, to.g),
            b: channel(self.b, to.b),
        }
    }

    pub fn edge(self) -> Srgb {
        self.to_oklch().fixed(0.25, 0.35).to_srgb()
    }
}

impl Oklch {
    fn to_srgb(self) -> Srgb {
        let a = self.c * self.h.cos();
        let b = self.c * self.h.sin();
        let l = (self.l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
        let m = (self.l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
        let s = (self.l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
        let channel = |value: f32| from_linear(value).clamp(0., 1.);
        Srgb {
            r: channel(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s),
            g: channel(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s),
            b: channel(-0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s),
        }
    }

    fn shifted(self, lightness: f32, chroma: f32) -> Oklch {
        Oklch {
            l: self.l + lightness,
            c: (self.c + chroma).max(0.),
            h: self.h,
        }
    }

    fn fixed(self, lightness: f32, chroma: f32) -> Oklch {
        Oklch {
            l: lightness,
            c: self.c * chroma,
            h: self.h,
        }
    }
}

pub const BRAND: Srgb = hex(Color::Accent.default());
pub const WHITE: Srgb = hex(0xffffff);
pub const BLACK: Srgb = hex(0x000000);
pub const INK: Srgb = hex(0x0d0d0f);
pub const ON_ART: Srgb = hex(0xe8e0d5);

#[derive(Clone, Copy, PartialEq)]
pub struct Palette {
    pub key: u32,
    pub dark: bool,
    surfaces: [Srgb; 5],
    texts: [Srgb; 6],
    notice: [Srgb; 5],
    error: [Srgb; 3],
    alerts: [[Srgb; 4]; 3],
}

const DARK_TEXTS: [Srgb; 6] = [
    hex(0xe8e0d5),
    hex(0x7a7890),
    hex(0xa2a4ba),
    hex(0x5b9bd4),
    hex(0x6abf6a),
    hex(0x0539a5),
];
const DARK_NOTICE: [Srgb; 5] = [
    hex(0x0d0d00),
    hex(0xc8b060),
    hex(0xf0c040),
    hex(0xffd966),
    hex(0xf0a500),
];
const DARK_ERROR: [Srgb; 3] = [hex(0x0d0000), hex(0xc86060), hex(0xf06060)];
const DARK_ALERTS: [[Srgb; 4]; 3] = [
    [hex(0x00060d), hex(0x7aa6cc), hex(0x8cc4f0), hex(0x5b9bd4)],
    [hex(0x000d00), hex(0x7fb87f), hex(0x8ee08e), hex(0x6abf6a)],
    [hex(0x0a000d), hex(0xb08cc8), hex(0xd0a0f0), hex(0xb47ad4)],
];
const LIGHT_ALERTS: [[Srgb; 4]; 3] = [
    [hex(0xe3f0fc), hex(0x1f4f7a), hex(0x1b63a8), hex(0x1f6fb8)],
    [hex(0xe2f5e2), hex(0x1f5a26), hex(0x1e7a2b), hex(0x2e8b3a)],
    [hex(0xf1e6fa), hex(0x5a2f7a), hex(0x7438a8), hex(0x8a4fc0)],
];
const LIGHT_NOTICE: [Srgb; 5] = [
    hex(0xfff6d6),
    hex(0x7a5a00),
    hex(0x946200),
    hex(0x6e4a00),
    hex(0xe09a00),
];
const LIGHT_ERROR: [Srgb; 3] = [hex(0xfff0f0), hex(0xa12a2a), hex(0xb02222)];

const fn dark(surfaces: [u32; 5]) -> Palette {
    Palette {
        key: surfaces[0],
        dark: true,
        surfaces: tones(surfaces),
        texts: DARK_TEXTS,
        notice: DARK_NOTICE,
        error: DARK_ERROR,
        alerts: DARK_ALERTS,
    }
}

const fn light(surfaces: [u32; 5], texts: [u32; 3]) -> Palette {
    Palette {
        key: surfaces[0],
        dark: false,
        surfaces: tones(surfaces),
        texts: [
            hex(texts[0]),
            hex(texts[1]),
            hex(texts[2]),
            hex(0x1f6fb8),
            hex(0x2e8b3a),
            hex(0xb9d2ff),
        ],
        notice: LIGHT_NOTICE,
        error: LIGHT_ERROR,
        alerts: LIGHT_ALERTS,
    }
}

const fn tones(values: [u32; 5]) -> [Srgb; 5] {
    [
        hex(values[0]),
        hex(values[1]),
        hex(values[2]),
        hex(values[3]),
        hex(values[4]),
    ]
}

pub const PALETTES: [Palette; 4] = [
    dark([0x0d0d0f, 0x13131a, 0x1a1a24, 0x2a2a38, 0x0c0c0f]),
    dark([0x000000, 0x08080a, 0x111116, 0x22222c, 0x000000]),
    light(
        [0xffffff, 0xf4f4f6, 0xeaeaee, 0xd5d5dc, 0xededf1],
        [0x1b1b20, 0x62626e, 0x4a4a55],
    ),
    light(
        [0xfaf6ee, 0xf2ecdf, 0xe9e1d1, 0xd6cbb6, 0xece5d6],
        [0x2a251d, 0x6e6557, 0x54493b],
    ),
];

thread_local! {
    static PALETTE: Cell<Palette> = const { Cell::new(PALETTES[0]) };
}

pub fn palette() -> Palette {
    PALETTE.with(Cell::get)
}

pub fn contrast() -> Srgb {
    if palette().dark { WHITE } else { BLACK }
}

fn around(key: u32) -> Palette {
    let ground = hex(key);
    let dark = ground.contrast_with_white();
    let like = PALETTES.into_iter().find(|palette| palette.dark == dark);
    let like = like.unwrap_or(PALETTES[0]);
    let origin = like.surfaces[0];
    let (r, g, b) = (
        ground.r - origin.r,
        ground.g - origin.g,
        ground.b - origin.b,
    );
    let moved = |tone: Srgb| Srgb {
        r: (tone.r + r).clamp(0., 1.),
        g: (tone.g + g).clamp(0., 1.),
        b: (tone.b + b).clamp(0., 1.),
    };
    Palette {
        key,
        surfaces: like.surfaces.map(moved),
        ..like
    }
}

pub fn set_palette(key: u32) -> bool {
    let preset = PALETTES.into_iter().find(|palette| palette.key == key);
    let chosen = preset.unwrap_or_else(|| around(key));
    PALETTE.with(|palette| palette.replace(chosen) != chosen)
}

#[derive(Clone, Copy, PartialEq)]
pub enum Tone {
    Surface(usize),
    Text(usize),
    Notice(usize),
    Error(usize),
    Alert(usize, usize),
}

pub const BG: Tone = Tone::Surface(0);
pub const SURFACE: Tone = Tone::Surface(1);
pub const SURFACE_2: Tone = Tone::Surface(2);
pub const BORDER: Tone = Tone::Surface(3);
pub const WELL: Tone = Tone::Surface(4);
pub const TEXT: Tone = Tone::Text(0);
pub const TEXT_MUTED: Tone = Tone::Text(1);
pub const MUTED: Tone = Tone::Text(2);
pub const LINK: Tone = Tone::Text(3);
pub const SUCCESS: Tone = Tone::Text(4);
pub const SELECTION: Tone = Tone::Text(5);
pub const NOTICE_BG: Tone = Tone::Notice(0);
pub const NOTICE_TEXT: Tone = Tone::Notice(1);
pub const NOTICE_LINK: Tone = Tone::Notice(2);
pub const NOTICE_LINK_HOVER: Tone = Tone::Notice(3);
pub const NOTICE_EDGE: Tone = Tone::Notice(4);
pub const ERROR_BG: Tone = Tone::Error(0);
pub const ERROR_TEXT: Tone = Tone::Error(1);
pub const ERROR_LINK: Tone = Tone::Error(2);

impl Tone {
    pub fn srgb(self) -> Srgb {
        let palette = palette();
        match self {
            Tone::Surface(index) => palette.surfaces[index],
            Tone::Text(index) => palette.texts[index],
            Tone::Notice(index) => palette.notice[index],
            Tone::Error(index) => palette.error[index],
            Tone::Alert(kind, index) => palette.alerts[kind][index],
        }
    }

    pub fn rgba(self, alpha: f32) -> Rgba {
        self.srgb().rgba(alpha)
    }

    pub fn alpha(self, alpha: f32) -> Hsla {
        self.srgb().alpha(alpha)
    }

    pub fn solid(self) -> Hsla {
        self.srgb().solid()
    }

    pub fn mix(self, to: impl Into<Srgb>, amount: f32) -> Srgb {
        self.srgb().mix(to, amount)
    }
}

impl From<Tone> for Srgb {
    fn from(tone: Tone) -> Self {
        tone.srgb()
    }
}

const DEFAULT_EDGE: Srgb = hex(0x3a1020);

#[derive(Clone, Copy)]
pub struct Face {
    family: &'static str,
    weight: f32,
    leading: f32,
}

const fn face(family: &'static str, weight: f32, leading: f32) -> Face {
    Face {
        family,
        weight,
        leading,
    }
}

impl Face {
    pub fn family(self) -> SharedString {
        self.family.into()
    }

    pub fn weight(self) -> FontWeight {
        FontWeight(self.weight)
    }
}

const LEADING_DISPLAY: f32 = 1.276;
const LEADING_MONO: f32 = 1.127;
const LEADING_BODY: f32 = 1.2;

pub const FONT_DISPLAY_BOLD: Face = face("Rajdhani", 700., LEADING_DISPLAY);
pub const FONT_MONO: Face = face("Share Tech Mono", 400., LEADING_MONO);
pub const FONT_MONO_4: Face = face("Share Tech Mono 4", 400., LEADING_MONO);
pub const FONT_MONO_8: Face = face("Share Tech Mono 8", 400., LEADING_MONO);
pub const FONT_MONO_12: Face = face("Share Tech Mono 12", 400., LEADING_MONO);
pub const FONT_MONO_20: Face = face("Share Tech Mono 20", 400., LEADING_MONO);
pub const FONT_BODY: Face = face("Barlow", 400., LEADING_BODY);
pub const FONT_BODY_MEDIUM: Face = face("Barlow", 500., LEADING_BODY);
pub const FONT_BODY_SEMIBOLD: Face = face("Barlow", 600., LEADING_BODY);

pub trait Typeface: Styled {
    fn face(self, face: Face, size: f32) -> Self {
        self.font_family(face.family)
            .font_weight(FontWeight(face.weight))
            .text_size(px(size))
            .line_height(relative(face.leading))
    }
}

impl<T: Styled> Typeface for T {}

pub fn blend(from: Rgba, to: Rgba, amount: f32) -> Rgba {
    let alpha = from.a + (to.a - from.a) * amount;
    if alpha <= 0. {
        return Rgba { a: 0., ..to };
    }
    let channel = |from_value: f32, to_value: f32| {
        let start = from_value * from.a;
        (start + (to_value * to.a - start) * amount) / alpha
    };
    Rgba {
        r: channel(from.r, to.r),
        g: channel(from.g, to.g),
        b: channel(from.b, to.b),
        a: alpha,
    }
}

pub fn shadow(x: f32, y: f32, blur: f32, spread: f32, color: Hsla) -> BoxShadow {
    BoxShadow::new(px(x), px(y), color)
        .blur_radius(px(blur / 2.))
        .spread_radius(px(spread))
}

pub fn lifted() -> Vec<BoxShadow> {
    let dark = BLACK.alpha(0.1);
    vec![
        shadow(0., 10., 15., -3., dark),
        shadow(0., 4., 6., -4., dark),
    ]
}

pub const REM: f32 = 16.;
pub const TEXT_MICRO: f32 = 0.65 * REM;
pub const TEXT_LABEL: f32 = 0.75 * REM;
pub const TEXT_UI: f32 = 0.85 * REM;
pub const TEXT_BODY: f32 = 0.92 * REM;
pub const CH: f32 = 0.54 * TEXT_UI;

pub struct Theme {
    pub action: Srgb,
    pub action_text: Srgb,
    pub action_deep: Srgb,
    pub action_edge: Srgb,
    pub action_dim: Rgba,
    pub action_glow: Rgba,
    pub action_glow_soft: Hsla,
}

impl Global for Theme {}

impl Theme {
    pub fn for_action(action: Srgb) -> Self {
        let base = action.to_oklch();
        let default = action == BRAND;
        Self {
            action,
            action_text: if action.contrast_with_white() {
                WHITE
            } else {
                INK
            },
            action_deep: base.shifted(-0.066, -0.025).to_srgb(),
            action_edge: if default { DEFAULT_EDGE } else { action.edge() },
            action_dim: base.shifted(-0.129, -0.055).to_srgb().rgba(0.4),
            action_glow: action.rgba(0.27),
            action_glow_soft: action.alpha(0.3),
        }
    }

    pub fn get(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    pub fn apply(color: u32, window: &mut gpui::Window, cx: &mut App) {
        let action = hex(color);
        if Self::get(cx).action != action {
            cx.set_global(Self::for_action(action));
            window.refresh();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: Srgb, r: f32, g: f32, b: f32) {
        for (have, want) in [(actual.r, r), (actual.g, g), (actual.b, b)] {
            assert!((have - want).abs() < 0.0005, "{have} != {want}");
        }
    }

    #[test]
    fn blending_from_a_transparent_color_keeps_the_target_color() {
        let clear = Rgba {
            r: 0.,
            g: 0.,
            b: 0.,
            a: 0.,
        };
        let red = Rgba {
            r: 0.8,
            g: 0.1,
            b: 0.2,
            a: 1.,
        };
        let half = blend(clear, red, 0.5);
        close(
            Srgb {
                r: half.r,
                g: half.g,
                b: half.b,
            },
            0.8,
            0.1,
            0.2,
        );
        assert!((half.a - 0.5).abs() < 0.0005);
        assert_eq!(blend(clear, red, 1.), red);
        assert_eq!(blend(red, clear, 1.).a, 0.);
        let grey = Rgba {
            r: 0.2,
            g: 0.2,
            b: 0.2,
            a: 0.5,
        };
        let mixed = blend(grey, red, 0.5);
        close(
            Srgb {
                r: mixed.r,
                g: mixed.g,
                b: mixed.b,
            },
            0.6,
            0.133_333,
            0.2,
        );
        assert!((mixed.a - 0.75).abs() < 0.0005);
    }

    #[test]
    fn a_free_background_keeps_the_steps_of_the_nearest_preset() {
        let night = around(0x102030);
        assert!(night.dark);
        close(night.surfaces[0], 16. / 255., 32. / 255., 48. / 255.);
        close(night.surfaces[1], 22. / 255., 38. / 255., 59. / 255.);
        close(night.surfaces[3], 45. / 255., 61. / 255., 89. / 255.);
        assert!(night.texts == PALETTES[0].texts);
        let day = around(0xf0f0e0);
        assert!(!day.dark);
        close(day.surfaces[1], 229. / 255., 229. / 255., 215. / 255.);
        assert!(day.texts == PALETTES[2].texts);
        close(around(0xfffffe).surfaces[0], 1., 1., 254. / 255.);
        close(around(0x000001).surfaces[4], 0., 0., 1. / 255.);
        assert!(around(PALETTES[0].key) == PALETTES[0]);
        assert!(around(PALETTES[2].key) == PALETTES[2]);
    }

    #[test]
    fn the_default_accent_dims_to_its_reference_shade() {
        let Rgba { r, g, b, .. } = Theme::for_action(BRAND).action_dim;
        close(Srgb { r, g, b }, 0.502022, 0.0652304, 0.106526);
    }
}
