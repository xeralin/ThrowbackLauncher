use backend::settings::Color;
use gpui::{Background, Rgba, linear_color_stop, linear_gradient};

use crate::model::settings::Settings;
use crate::ui::theme::{MUTED, Srgb, Theme, hex};

const MUTED_EDGE: Srgb = hex(0x34343f);
const TINT: f32 = 0.13;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Highlight {
    Accent,
    Transfer,
    Muted,
}

pub struct Glow {
    pub line: Background,
    pub edge: Srgb,
    pub halo: Rgba,
}

impl Highlight {
    pub fn glow(self, settings: &Settings, theme: &Theme) -> Glow {
        match self {
            Highlight::Accent => Glow {
                line: theme.action.solid().into(),
                edge: theme.action_edge,
                halo: theme.action_glow,
            },
            Highlight::Transfer => {
                let fill = hex(settings.color(Color::BarFill));
                let stripe = hex(settings.color(Color::BarStripe));
                let blend = fill.mix(stripe, 0.5);
                Glow {
                    line: linear_gradient(
                        90.,
                        linear_color_stop(fill.solid(), 0.),
                        linear_color_stop(stripe.solid(), 1.),
                    ),
                    edge: blend.edge(),
                    halo: blend.rgba(TINT),
                }
            }
            Highlight::Muted => Glow {
                line: MUTED.solid().into(),
                edge: MUTED_EDGE,
                halo: MUTED.rgba(TINT),
            },
        }
    }
}
