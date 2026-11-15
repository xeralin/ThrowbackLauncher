use backend::env::{APP_NAME, DIR_NAME};
use gpui::{
    AppContext, Bounds, Context, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    px, size,
};

use crate::ui::button::primary;
use crate::ui::theme::{BG, FONT_BODY, REM, TEXT, Typeface};

struct Failure(SharedString);

impl Render for Failure {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(16.))
            .p(px(24.))
            .bg(BG.solid())
            .face(FONT_BODY, REM)
            .text_color(TEXT.solid())
            .child(div().flex_1().child(self.0.clone()))
            .child(
                div().flex().justify_end().child(
                    primary("failure-close", "OK", window, cx).on_click(|_, _, cx| cx.quit()),
                ),
            )
    }
}

pub fn show(message: String) -> ! {
    crate::run(move |cx| {
        let bounds = Bounds::centered(None, size(px(480.), px(200.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            is_resizable: false,
            titlebar: Some(TitlebarOptions {
                title: Some(APP_NAME.into()),
                ..Default::default()
            }),
            app_id: Some(DIR_NAME.into()),
            ..Default::default()
        };
        let message = SharedString::from(message);
        let _ = cx.open_window(options, |_, cx| cx.new(|_| Failure(message)));
        cx.activate(true);
    });
    std::process::exit(1)
}
