use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::ui::{px, units};
use gpui::{
    App, Bounds, Corners, Div, Global, Hsla, Image, ImageFormat, InteractiveElement, ParentElement,
    Pixels, RenderImage, Rgba, Stateful, StatefulInteractiveElement, Styled, Window, canvas, div,
    fill, linear_color_stop, linear_gradient, pattern_slash, point, relative, size, white,
};

use crate::model::Launcher;
use crate::ui::kit::external;
use crate::ui::motion::{self, sub};
use crate::ui::theme::{BORDER, Srgb, WELL, hex, shadow};

const HEIGHT: f32 = 15.;
const STRIPE: f32 = 40.;
const CYCLE: Duration = Duration::from_millis(1800);
const GLOW: f32 = 0.27;
const CORNER: f32 = 4.;
const SHORTEST: f32 = 12.;
const SPRITE: &[u8] = include_bytes!("../../assets/nyan-stars.webp");
const ROWS: usize = 13;
const TWINKLE: Duration = Duration::from_millis(1200);
const RAINBOW: [(Srgb, f32); 6] = [
    (hex(0xff0000), 0.165),
    (hex(0xff9900), 0.33),
    (hex(0xffff00), 0.5),
    (hex(0x33ff00), 0.66),
    (hex(0x0099ff), 0.835),
    (hex(0x6633ff), 1.),
];
const EDGE: f32 = 0.001;
const CAT: (f32, f32) = (34., 21.);
const CAT_LIFT: f32 = 4.;
const CAT_REST: f32 = 0.89;
const CAT_GROW: Duration = Duration::from_millis(100);
const TAIL: f32 = 10.;
const HOME: &str = "https://www.nyan.cat/";

#[derive(Default)]
struct Stripes {
    ran: Duration,
    since: Option<Instant>,
}

struct Sky {
    color: Hsla,
    width: usize,
    runs: Rc<[Vec<(usize, usize)>]>,
}

impl Global for Sky {}

impl Sky {
    fn read(cx: &mut App) -> &Self {
        if !cx.has_global::<Self>() {
            let sprite = Image::from_bytes(ImageFormat::Webp, SPRITE.to_vec());
            let sprite = sprite.to_image_data(cx.svg_renderer());
            let sprite = sprite.expect("bundled sprite");
            let width = sprite.size(0).width.0 as usize;
            let pixels: Vec<&[u8]> = sprite.as_bytes(0).unwrap_or_default().chunks(4).collect();
            let sky = pixels.first().copied().unwrap_or_default();
            let runs = pixels.chunks(width).map(|row| {
                let mut runs: Vec<(usize, usize)> = Vec::new();
                for (x, _) in row.iter().enumerate().filter(|(_, pixel)| **pixel != sky) {
                    match runs.last_mut() {
                        Some((start, length)) if *start + *length == x => *length += 1,
                        _ => runs.push((x, 1)),
                    }
                }
                runs
            });
            let color = Rgba {
                r: f32::from(sky[2]) / 255.,
                g: f32::from(sky[1]) / 255.,
                b: f32::from(sky[0]) / 255.,
                a: 1.,
            };
            cx.set_global(Self {
                color: color.into(),
                width,
                runs: runs.collect(),
            });
        }
        cx.global()
    }
}

fn stars(ran: Duration, cx: &mut App) -> Div {
    let sky = Sky::read(cx);
    let (width, runs) = (sky.width, sky.runs.clone());
    let frames = runs.len() / ROWS;
    let frame =
        (ran.as_millis() % TWINKLE.as_millis()) as usize * frames / TWINKLE.as_millis() as usize;
    let twinkle = canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window, _| {
            let radius = units(bounds.size.height) / 2.;
            let span = units(bounds.size.width);
            for (row, runs) in runs[frame * ROWS..][..ROWS].iter().enumerate() {
                let height = row as f32 + 0.5 - radius;
                let inset = radius - (radius * radius - height * height).max(0.).sqrt();
                let mut tile = 0.;
                while tile < span {
                    for (start, length) in runs {
                        let from = (tile + *start as f32).max(inset);
                        let to = (tile + (start + length) as f32).min(span - inset);
                        if from < to {
                            let origin = bounds.origin + point(px(from), px(row as f32));
                            let star = Bounds::new(origin, size(px(to - from), px(1.)));
                            window.paint_quad(fill(star, white()));
                        }
                    }
                    tile += width as f32;
                }
            }
        },
    );
    div()
        .absolute()
        .inset_0()
        .rounded(px(HEIGHT / 2.))
        .bg(sky.color)
        .child(twinkle.size_full())
}

fn rainbow() -> impl Iterator<Item = Div> {
    RAINBOW.into_iter().rev().map(|(color, end)| {
        let band = div()
            .absolute()
            .left_0()
            .top_0()
            .bottom_0()
            .right(px(CAT.0 - TAIL))
            .rounded_l(px(HEIGHT / 2.));
        if end < 1. {
            band.bg(linear_gradient(
                180.,
                linear_color_stop(color.solid(), end),
                linear_color_stop(color.alpha(0.), end + EDGE),
            ))
        } else {
            band.bg(color.solid())
        }
    })
}

fn pose(cat: &RenderImage, ran: Duration) -> usize {
    let pace = |frame| Duration::from(cat.delay(frame)).as_millis();
    let cycle: u128 = (0..cat.frame_count()).map(pace).sum();
    let mut rest = ran.as_millis() % cycle.max(1);
    let reached = |frame: &usize| {
        let stays = rest < pace(*frame);
        rest = rest.saturating_sub(pace(*frame));
        stays
    };
    (0..cat.frame_count()).find(reached).unwrap_or_default()
}

fn cat(
    id: &'static str,
    cat: Arc<RenderImage>,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let id = sub(&id.into(), "cat");
    let hover = motion::hover(&id, CAT_GROW, window, cx);
    let scale = CAT_REST + (1. - CAT_REST) * hover.amount;
    let frame = pose(&cat, motion::clock(sub(&id, "run"), window, cx));
    window.request_animation_frame();
    let picture = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let drawn = window.paint_image(bounds, bounds, Corners::default(), cat, frame, false);
            drawn.ok();
        },
    );
    let frame = div()
        .absolute()
        .top(px(-CAT_LIFT))
        .right_0()
        .w(px(CAT.0))
        .h(px(CAT.1))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .child(picture.w(px(CAT.0 * scale)).h(px(CAT.1 * scale)));
    external(id, frame, HOME, &hover, window, cx).on_click(|_, _, cx| cx.stop_propagation())
}

pub fn transfer_bar(
    id: &'static str,
    progress: f32,
    (base, stripe): (Srgb, Srgb),
    paused: bool,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let now = cx.background_executor().now();
    let stripes = window.use_keyed_state(id, cx, |_, _| Stripes::default());
    let ran = stripes.update(cx, |stripes, _| {
        if let Some(since) = stripes.since.take() {
            stripes.ran += now.duration_since(since);
        }
        stripes.since = (!paused).then_some(now);
        stripes.ran
    });
    if !paused {
        window.request_animation_frame();
    }
    let launcher = Launcher::handle(cx);
    let nyan = launcher.read(cx).nyan();
    let track = div()
        .id(id)
        .h(px(HEIGHT))
        .rounded(px(HEIGHT / 2.))
        .border_1()
        .on_click(move |event, _, cx| {
            if event.click_count().is_multiple_of(3) {
                launcher.update(cx, |launcher, cx| launcher.toggle_nyan(cx));
            }
        });
    let filled = div().h_full().w(relative(progress / 100.));
    if let Some(nyan) = nyan {
        return track
            .border_color(BORDER.alpha(0.))
            .child(stars(ran, cx))
            .child(
                filled
                    .min_w(px(CAT.0))
                    .children(rainbow())
                    .child(cat(id, nyan, window, cx)),
            );
    }
    let phase = (ran.as_secs_f32() % CYCLE.as_secs_f32()) / CYCLE.as_secs_f32() * STRIPE;
    track.border_color(BORDER.solid()).bg(WELL.solid()).child(
        filled
            .min_w(px(SHORTEST))
            .rounded(px(HEIGHT / 2.))
            .overflow_hidden()
            .bg(base.solid())
            .shadow(vec![shadow(0., 2., 14., 0., base.alpha(GLOW))])
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(phase - STRIPE))
                    .right_0()
                    .bg(pattern_slash(stripe.solid(), STRIPE / 2., STRIPE / 2.)),
            )
            .child(
                div()
                    .absolute()
                    .inset(px(-CORNER))
                    .rounded(px(HEIGHT / 2. + CORNER))
                    .border(px(CORNER))
                    .border_color(WELL.solid()),
            ),
    )
}
