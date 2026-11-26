pub mod accent;
pub mod button;
pub mod frame;
pub mod glow;
pub mod hint;
pub mod icon;
pub mod input;
pub mod key_art;
pub mod kit;
pub mod log_box;
pub mod markdown;
pub mod modal;
pub mod motion;
pub mod picture;
pub mod prose;
pub mod scroll;
pub mod selectable;
pub mod setting;
pub mod slider;
pub mod stepper;
pub mod swatches;
pub mod switch;
pub mod tabs;
#[cfg(test)]
pub mod testing;
pub mod theme;
pub mod toast;
pub mod transfer;

use std::cell::Cell;
use std::rc::Rc;

use gpui::{App, Context, KeyBinding, Pixels, Window, actions};

actions!(ui, [FocusNext, FocusPrevious, Dismiss]);

pub const DATE_FORMAT: &str = "%b %-d, %Y";

pub type Handler = Rc<dyn Fn(&mut Window, &mut App)>;
pub type Bound<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

pub fn bind<V: 'static, T: 'static>(
    cx: &Context<V>,
    apply: impl Fn(&mut V, T, &mut Window, &mut Context<V>) + 'static,
) -> Bound<T> {
    let view = cx.weak_entity();
    Rc::new(move |value, window, cx| {
        view.update(cx, |view, cx| apply(view, value, window, cx))
            .ok();
    })
}

thread_local! {
    static SCALE: Cell<f32> = const { Cell::new(1.) };
}

pub fn set_scale(percent: u32) -> bool {
    let scale = percent as f32 / 100.;
    SCALE.replace(scale) != scale
}

pub fn scale() -> f32 {
    SCALE.get()
}

pub fn px(length: f32) -> Pixels {
    gpui::px(length * scale())
}

pub fn units(length: Pixels) -> f32 {
    f32::from(length) / scale()
}

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", FocusNext, None),
        KeyBinding::new("shift-tab", FocusPrevious, None),
        KeyBinding::new("escape", Dismiss, None),
    ]);
    input::init(cx);
    toast::init(cx);
    selectable::init(cx);
}
