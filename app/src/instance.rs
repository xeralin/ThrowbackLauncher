use backend::instance::Listener;
use gpui::App;

pub fn watch(key: &str, cx: &mut App) {
    let (sender, wakes) = async_channel::unbounded();
    let Ok(listener) = Listener::start(key, move || {
        let _ = sender.try_send(());
    }) else {
        return;
    };
    let mut listener = Some(listener);
    cx.on_app_quit(move |_| {
        listener.take();
        async {}
    })
    .detach();
    cx.spawn(async move |cx| {
        while wakes.recv().await.is_ok() {
            cx.update(crate::show_window);
        }
    })
    .detach();
}
