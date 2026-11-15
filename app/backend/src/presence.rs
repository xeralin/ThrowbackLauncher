use std::io;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use chrono::Utc;

use crate::manifest::{Download, resolve_install};
#[cfg(not(test))]
use crate::rpc::Presence;
use crate::rpc::{Activity, build_activity};
#[cfg(test)]
use tests::Presence;

const POLL_INTERVAL: Duration = Duration::from_secs(10);
const RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);

struct Session {
    presence: Presence,
    activity: Option<Activity>,
    sent: bool,
    delay: Duration,
}

impl Session {
    fn new() -> Self {
        Self {
            presence: Presence::new(""),
            activity: None,
            sent: false,
            delay: RECONNECT_DELAY,
        }
    }

    fn show(&mut self, season: Option<(&Download, bool)>, now: i64) {
        let (client_id, activity) = match season {
            Some((download, hm)) => {
                let (client_id, activity) = build_activity(&download.label, hm, now);
                (client_id, Some(activity))
            }
            None => ("", None),
        };
        if client_id != self.presence.client_id() || activity.is_none() {
            self.presence.close();
            self.presence = Presence::new(client_id);
        }
        self.activity = activity;
        self.sent = false;
        self.delay = RECONNECT_DELAY;
    }

    fn step(&mut self) -> Option<Duration> {
        self.presence.drain();
        let activity = self.activity.as_ref()?;
        if !self.presence.connected() {
            if !self.presence.connect() {
                let wait = self.delay;
                self.delay = (wait * 2).min(MAX_RECONNECT_DELAY);
                return Some(wait);
            }
            self.delay = RECONNECT_DELAY;
            self.sent = false;
        }
        if !self.sent {
            self.sent = self.presence.set(activity);
        }
        Some(POLL_INTERVAL)
    }
}

fn run(feed: &Receiver<Option<String>>, downloads: &[Download], folder: Option<String>) {
    let mut session = Session::new();
    let mut change = Some(folder);
    loop {
        if let Some(folder) = change.take() {
            let season = folder.and_then(|folder| resolve_install(&folder, downloads));
            session.show(season, Utc::now().timestamp());
        }
        let next = match session.step() {
            Some(wait) => feed.recv_timeout(wait),
            None => feed.recv().map_err(RecvTimeoutError::from),
        };
        match next {
            Ok(folder) => change = Some(folder),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    session.presence.close();
}

pub struct PresenceWorker {
    downloads: Arc<[Download]>,
    folder: Option<String>,
    feed: Option<Sender<Option<String>>>,
}

impl PresenceWorker {
    pub fn new(downloads: impl Into<Arc<[Download]>>) -> Self {
        Self {
            downloads: downloads.into(),
            folder: None,
            feed: None,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) -> io::Result<()> {
        if !enabled {
            self.feed = None;
        } else if self.feed.is_none() {
            let (feed, folders) = mpsc::channel();
            let (downloads, folder) = (Arc::clone(&self.downloads), self.folder.clone());
            thread::Builder::new()
                .name("discord-rpc".to_owned())
                .spawn(move || run(&folders, &downloads, folder))?;
            self.feed = Some(feed);
        }
        Ok(())
    }

    pub fn set_folder(&mut self, folder: Option<&str>) {
        if folder == self.folder.as_deref() {
            return;
        }
        self.folder = folder.map(str::to_owned);
        if let Some(feed) = &self.feed {
            let _ = feed.send(self.folder.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::time::Instant;

    use super::*;

    const THROWBACK: &str = "1544119665394192525";
    const HEATED_METAL: &str = "1544119720662278215";

    thread_local! {
        static CONNECTS: RefCell<VecDeque<bool>> = const { RefCell::new(VecDeque::new()) };
        static SENDS: RefCell<VecDeque<bool>> = const { RefCell::new(VecDeque::new()) };
        static EVENTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) struct Presence {
        client_id: String,
        connected: bool,
    }

    impl Presence {
        pub(super) fn new(client_id: &str) -> Self {
            Self {
                client_id: client_id.to_owned(),
                connected: false,
            }
        }

        fn note(&self, event: &str) {
            let who = self.client_id.replace(THROWBACK, "tb");
            let line = format!("{event} {}", who.replace(HEATED_METAL, "hm"));
            EVENTS.with_borrow_mut(|events| events.push(line.trim_end().to_owned()));
        }

        pub(super) fn client_id(&self) -> &str {
            &self.client_id
        }

        pub(super) fn connected(&self) -> bool {
            self.connected
        }

        pub(super) fn connect(&mut self) -> bool {
            self.connected =
                CONNECTS.with_borrow_mut(|results| results.pop_front().unwrap_or(true));
            self.note(if self.connected { "connect" } else { "refused" });
            self.connected
        }

        pub(super) fn set(&mut self, activity: &Activity) -> bool {
            self.connected = SENDS.with_borrow_mut(|results| results.pop_front().unwrap_or(true));
            let body = serde_json::to_value(activity).unwrap();
            let shown = format!("{} at {}", body["details"], body["timestamps"]["start"]);
            let outcome = if self.connected { "set" } else { "lost" };
            self.note(&format!("{outcome} {shown}"));
            self.connected
        }

        pub(super) fn drain(&mut self) {}

        pub(super) fn close(&mut self) {
            if std::mem::take(&mut self.connected) {
                self.note("close");
            }
        }
    }

    fn events() -> Vec<String> {
        EVENTS.take()
    }

    fn download(key: &str, label: &str, hm: bool) -> Download {
        Download {
            key: key.to_owned(),
            label: label.to_owned(),
            build: String::new(),
            app: 359550,
            depot_content: 359551,
            depot_ww: 377237,
            manifest_content: String::new(),
            manifest_ww: String::new(),
            size_gb: 1.0,
            hm_version: hm.then(|| "latest".to_owned()),
        }
    }

    fn downloads() -> Vec<Download> {
        vec![
            download("Y1S0_Vanilla", "Y1S0 Vanilla", false),
            download("Y2S1_VelvetShell", "Y2S1 Velvet Shell", false),
            download("Y5S3_ShadowLegacy", "Y5S3 Shadow Legacy", true),
        ]
    }

    #[test]
    fn nothing_is_shown_without_a_known_running_season() {
        let games = downloads();
        let mut session = Session::new();
        assert_eq!(session.step(), None);
        session.show(resolve_install("Y3S1_Chimera", &games), 5);
        assert_eq!(session.step(), None);
        assert!(events().is_empty());
    }

    #[test]
    fn a_missing_discord_is_retried_with_doubling_delays() {
        let games = downloads();
        let mut session = Session::new();
        session.show(resolve_install("Y1S0_Vanilla", &games), 100);
        CONNECTS.with_borrow_mut(|results| results.extend([false; 7]));
        let waits: Vec<u64> = (0..7).map(|_| session.step().unwrap().as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 30, 30]);
        assert_eq!(events(), ["refused tb"; 7]);
        assert_eq!(session.step(), Some(POLL_INTERVAL));
        assert_eq!(session.step(), Some(POLL_INTERVAL));
        assert_eq!(events(), ["connect tb", "set \"Y1S0 Vanilla\" at 100 tb"]);
        session.presence.connected = false;
        CONNECTS.with_borrow_mut(|results| results.push_back(false));
        assert_eq!(session.step(), Some(RECONNECT_DELAY));
        assert_eq!(session.step(), Some(POLL_INTERVAL));
        assert_eq!(
            events(),
            ["refused tb", "connect tb", "set \"Y1S0 Vanilla\" at 100 tb"]
        );
    }

    #[test]
    fn a_lost_send_is_repeated_after_the_next_connect() {
        let games = downloads();
        let mut session = Session::new();
        session.show(resolve_install("Y1S0_Vanilla", &games), 1);
        SENDS.with_borrow_mut(|results| results.push_back(false));
        assert_eq!(session.step(), Some(POLL_INTERVAL));
        assert_eq!(session.step(), Some(POLL_INTERVAL));
        assert_eq!(
            events(),
            [
                "connect tb",
                "lost \"Y1S0 Vanilla\" at 1 tb",
                "connect tb",
                "set \"Y1S0 Vanilla\" at 1 tb"
            ]
        );
    }

    #[test]
    fn a_season_change_keeps_the_connection_unless_the_edition_changes() {
        let games = downloads();
        let mut session = Session::new();
        session.show(resolve_install("Y1S0_Vanilla", &games), 10);
        session.step();
        session.show(resolve_install("Y2S1_VelvetShell", &games), 20);
        session.step();
        session.show(resolve_install("Y5S3_HeatedMetal", &games), 30);
        session.step();
        session.show(None, 40);
        assert_eq!(session.step(), None);
        assert_eq!(
            events(),
            [
                "connect tb",
                "set \"Y1S0 Vanilla\" at 10 tb",
                "set \"Y2S1 Velvet Shell\" at 20 tb",
                "close tb",
                "connect hm",
                "set \"Y5S3 Shadow Legacy\" at 30 hm",
                "close hm"
            ]
        );
    }

    #[test]
    fn the_worker_follows_the_feed_and_closes_when_it_ends() {
        let (feed, folders) = mpsc::channel();
        feed.send(Some("Y5S3_HeatedMetal".to_owned())).unwrap();
        feed.send(None).unwrap();
        drop(feed);
        let started = Instant::now();
        run(&folders, &downloads(), Some("Y1S0_Vanilla".to_owned()));
        assert!(started.elapsed() < POLL_INTERVAL);
        let seen = events();
        assert_eq!(seen.len(), 6, "{seen:?}");
        assert_eq!(seen[0], "connect tb");
        assert!(seen[1].starts_with("set \"Y1S0 Vanilla\" at "));
        assert_eq!(seen[2..4], ["close tb", "connect hm"]);
        assert!(seen[4].starts_with("set \"Y5S3 Shadow Legacy\" at "));
        assert_eq!(seen[5], "close hm");
    }

    #[test]
    fn the_thread_runs_only_while_enabled() {
        let mut worker = PresenceWorker::new(downloads());
        let threads = |worker: &PresenceWorker| Arc::strong_count(&worker.downloads) - 1;
        let settled = |worker: &PresenceWorker, wanted: usize| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while threads(worker) != wanted && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            threads(worker) == wanted
        };
        worker.set_folder(Some("Y1S0_Vanilla"));
        assert_eq!(threads(&worker), 0);
        worker.set_enabled(true).unwrap();
        worker.set_enabled(true).unwrap();
        worker.set_folder(Some("Y1S0_Vanilla"));
        worker.set_folder(None);
        assert_eq!(threads(&worker), 1);
        worker.set_enabled(false).unwrap();
        assert!(settled(&worker, 0));
        worker.set_enabled(true).unwrap();
        assert_eq!(threads(&worker), 1);
        let downloads = Arc::clone(&worker.downloads);
        drop(worker);
        let deadline = Instant::now() + Duration::from_secs(20);
        while Arc::strong_count(&downloads) > 1 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(Arc::strong_count(&downloads), 1);
    }
}
