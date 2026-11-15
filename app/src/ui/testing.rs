use gpui::{
    AnyElement, App, Context, FocusHandle, InteractiveElement, IntoElement, KeyUpEvent, Keystroke,
    ParentElement, Render, Styled, Subscription, TestAppContext, VisualTestContext, Window, div,
};

use crate::ui::{FocusNext, FocusPrevious};

type Build = Box<dyn Fn(&mut Window, &mut App) -> AnyElement>;

struct Probe {
    ground: FocusHandle,
    build: Build,
    _focus_lost: Subscription,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .track_focus(&self.ground)
            .on_action(|_: &FocusNext, window, cx| window.focus_next(cx))
            .on_action(|_: &FocusPrevious, window, cx| window.focus_prev(cx))
            .child((self.build)(window, cx))
    }
}

pub fn mount(
    cx: &mut TestAppContext,
    build: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
) -> &mut VisualTestContext {
    cx.update(crate::init);
    let (_, cx) = cx.add_window_view(|window, cx| {
        let ground = cx.focus_handle();
        window.focus(&ground, cx);
        Probe {
            ground,
            build: Box::new(build),
            _focus_lost: cx.on_focus_lost(window, |probe: &mut Probe, window, cx| {
                window.focus(&probe.ground, cx)
            }),
        }
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    cx
}

pub fn press(cx: &mut VisualTestContext, keys: &'static str) {
    for key in keys.split(' ') {
        cx.simulate_keystrokes(key);
        cx.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse(key).expect("keystroke"),
        });
    }
}
