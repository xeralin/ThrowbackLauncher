use async_channel::Sender;
use backend::env::{APP_NAME, DIR_NAME};
use backend::log;
use ksni::menu::StandardItem;
use ksni::{MenuItem, ToolTip, Tray, TrayMethods};
use zbus::fdo::PropertiesProxy;

use super::{ENTRIES, Event};

include!(concat!(env!("OUT_DIR"), "/tray.rs"));

const WATCHER: &str = "org.kde.StatusNotifierWatcher";

pub async fn available() -> bool {
    let hosted = async {
        let bus = zbus::Connection::session().await?;
        let watcher = PropertiesProxy::new(&bus, WATCHER, "/StatusNotifierWatcher").await?;
        let answer = watcher.get(WATCHER.try_into()?, "IsStatusNotifierHostRegistered");
        zbus::Result::Ok(answer.await?.try_into()?)
    };
    hosted.await.unwrap_or(false)
}

pub struct Icon(ksni::Handle<Item>);

impl Drop for Icon {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

pub async fn show(events: Sender<Event>) -> Option<Icon> {
    let shown = Item(events).spawn().await;
    shown
        .map(Icon)
        .map_err(|error| log::fail("Tray icon failed", error))
        .ok()
}

struct Item(Sender<Event>);

impl Item {
    fn send(&self, event: Event) {
        let _ = self.0.try_send(event);
    }
}

impl Tray for Item {
    fn id(&self) -> String {
        DIR_NAME.into()
    }

    fn title(&self) -> String {
        APP_NAME.into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let pixmap = |&(side, data): &(i32, &[u8])| ksni::Icon {
            width: side,
            height: side,
            data: data.to_vec(),
        };
        PIXMAPS.iter().map(pixmap).collect()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: APP_NAME.into(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Event::Open);
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.send(Event::Open);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let entry = |(label, event): (&str, Event)| {
            MenuItem::from(StandardItem {
                label: label.into(),
                activate: Box::new(move |item: &mut Self| item.send(event)),
                ..Default::default()
            })
        };
        ENTRIES.map(entry).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_opens_and_the_menu_opens_and_quits() {
        let (sender, events) = async_channel::unbounded();
        let mut item = Item(sender);
        item.activate(0, 0);
        item.secondary_activate(0, 0);
        let menu = item.menu();
        let mut labels = Vec::new();
        for entry in &menu {
            let MenuItem::Standard(entry) = entry else {
                panic!("{entry:?}");
            };
            labels.push(entry.label.as_str());
            (entry.activate)(&mut item);
        }
        assert_eq!(labels, ["Open", "Quit"]);
        let sent: Vec<Event> = std::iter::from_fn(|| events.try_recv().ok()).collect();
        assert!(matches!(
            sent[..],
            [Event::Open, Event::Open, Event::Open, Event::Quit]
        ));
        assert_eq!(item.tool_tip().title, "Throwback Launcher");
    }

    #[test]
    fn the_pixmaps_are_squares_in_argb() {
        let pixmaps = Item(async_channel::unbounded().0).icon_pixmap();
        let sides: Vec<i32> = pixmaps.iter().map(|pixmap| pixmap.width).collect();
        assert_eq!(sides, [22, 64]);
        for pixmap in pixmaps {
            let side = pixmap.width as usize;
            assert_eq!(pixmap.height, pixmap.width);
            assert_eq!(pixmap.data.len(), 4 * side * side);
            let centre = 4 * (side / 2) * (side + 1);
            assert_eq!(pixmap.data[0], 0);
            assert_eq!(pixmap.data[centre], 255);
        }
    }
}
