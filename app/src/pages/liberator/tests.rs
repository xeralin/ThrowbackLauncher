use std::cell::{Cell, RefCell};
use std::fs;

use backend::env::Paths;
use backend::settings::Settings as Store;
use gpui::{AppContext, TestAppContext, VisualTestContext};
use tempfile::TempDir;

use super::modifications::{Toggle, toggles};
use super::playlist::{active_path, columns};
use super::status::Status;
use super::*;
use crate::model::liberator::{Node, State};
use crate::ui::testing::{mount, press};

const LOADING: &str = r#"{"event":"state","attached":true}"#;
const DETACHED: &str = r#"{"event":"state"}"#;
const NO_TREE: &str = r#"{"event":"tree","tree":null}"#;
const TREE: &str = r#"{"event":"tree","tree":[{"text":"PvP","id":"","children":[{"text":"Bomb","id":"bomb","children":[]},{"text":"Hostage","id":"hostage","children":[]}]},{"text":"Tutorial","id":"tutorial","children":[]},{"text":"PvE","id":"","children":[{"text":"Hunt","id":"","children":[{"text":"Bank","id":"hunt:bank","children":[]}]}]}]}"#;
const SOME_MODS: &str =
    r#","deathless":true,"unlimitedAmmo":false,"disablePrimary":true,"skip":true,"endMatch":false"#;

fn unlocked(capabilities: &str) -> String {
    format!(
        r#"{{"event":"state","attached":true,"unlocked":true,"capabilities":{{"fullFeature":true{capabilities}}}}}"#
    )
}

fn staged(lines: &str) -> Liberator {
    let mut liberator = Liberator::new(Paths::new("asset".into(), "data".into()));
    liberator.stage(lines);
    liberator
}

struct Bench {
    _dir: TempDir,
    paths: Paths,
    launcher: Entity<Launcher>,
    page: Entity<LiberatorPage>,
}

impl Bench {
    fn say(&self, cx: &mut VisualTestContext, lines: &str) {
        self.launcher.update(cx, |launcher, cx| {
            launcher.liberator.stage(lines);
            cx.notify();
        });
        cx.run_until_parked();
    }

    fn tab(&self, cx: &mut VisualTestContext) -> Tab {
        self.page.read_with(cx, |page, _| page.tab)
    }

    fn read<R>(&self, cx: &mut VisualTestContext, read: impl FnOnce(&Liberator) -> R) -> R {
        self.launcher
            .read_with(cx, |launcher, _| read(&launcher.liberator))
    }

    fn missing(&self, cx: &mut VisualTestContext) -> bool {
        self.page.read_with(cx, |page, _| page.missing.is_open())
    }
}

fn bench<'a>(cx: &'a mut TestAppContext, lines: &str) -> (Bench, &'a mut VisualTestContext) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
    fs::create_dir_all(&paths.data_root).unwrap();
    fs::write(paths.settings_file(), "[settings]\ndiscord_rpc = false\n").unwrap();
    let launcher = cx.update(|cx| {
        crate::init(cx);
        let store = Store::load(&paths).unwrap();
        let mut launcher = Launcher::new(paths.clone(), store, Vec::new());
        launcher.liberator.stage(lines);
        launcher.init(cx);
        Launcher::handle(cx)
    });
    let slot = Rc::new(RefCell::new(None::<Entity<LiberatorPage>>));
    let made = slot.clone();
    let cx = mount(cx, move |window, cx| {
        let mut made = made.borrow_mut();
        let page = made.get_or_insert_with(|| cx.new(|cx| LiberatorPage::new(window, cx)));
        page.clone().into_any_element()
    });
    let page = slot.borrow().clone().unwrap();
    let bench = Bench {
        _dir: dir,
        paths,
        launcher,
        page,
    };
    (bench, cx)
}

#[test]
fn the_guard_sends_locked_tabs_to_support() {
    let ready = |controls, playlist, mods| Ready {
        controls,
        playlist,
        mods,
    };
    let off = ready(false, false, false);
    for tab in ORDER {
        assert_eq!(gate(tab, false, &off), Tab::Support);
        assert_eq!(gate(tab, true, &off), Tab::Support);
    }
    let treeless = ready(true, false, true);
    assert_eq!(gate(Tab::Playlist, true, &treeless), Tab::Playlist);
    assert_eq!(
        gate(Tab::Modifications, true, &treeless),
        Tab::Modifications
    );
    let bare = ready(true, true, false);
    assert_eq!(gate(Tab::Modifications, true, &bare), Tab::Support);
    assert_eq!(gate(Tab::Playlist, true, &bare), Tab::Playlist);
}

#[test]
fn a_loaded_playlist_takes_over_except_from_modifications() {
    let loaded = Ready {
        controls: true,
        playlist: true,
        mods: true,
    };
    assert_eq!(gate(Tab::Support, false, &loaded), Tab::Playlist);
    assert_eq!(gate(Tab::Support, true, &loaded), Tab::Support);
    assert_eq!(gate(Tab::Modifications, false, &loaded), Tab::Modifications);
    let bare = Ready {
        mods: false,
        ..loaded
    };
    assert_eq!(gate(Tab::Modifications, false, &bare), Tab::Support);
    assert_eq!(gate(Tab::Support, false, &bare), Tab::Playlist);
}

#[test]
fn the_tabs_unlock_with_the_runtime() {
    let flags = |lines: &str| {
        let ready = ready(&staged(lines));
        (ready.controls, ready.playlist, ready.mods)
    };
    assert_eq!(flags(""), (false, false, false));
    assert_eq!(flags(&[LOADING, TREE].join("\n")), (false, false, false));
    let locked =
        r#"{"event":"state","attached":true,"capabilities":{"fullFeature":true,"skip":true}}"#;
    assert_eq!(flags(&[locked, TREE].join("\n")), (false, false, false));
    let partial =
        r#"{"event":"state","attached":true,"unlocked":true,"capabilities":{"skip":true}}"#;
    assert_eq!(flags(&[partial, TREE].join("\n")), (false, false, false));
    assert_eq!(flags(&unlocked("")), (true, false, false));
    assert_eq!(
        flags(&[&unlocked(""), TREE].join("\n")),
        (true, true, false)
    );
    let reported = r#","deathless":false,"skip":false,"endMatch":false"#;
    assert_eq!(flags(&unlocked(reported)), (true, false, false));
    for usable in [
        "deathless",
        "displayBuild",
        "disableSecondaryGadget",
        "skip",
        "endMatch",
    ] {
        let capability = format!(r#","{usable}":true"#);
        assert_eq!(
            flags(&unlocked(&capability)),
            (true, false, true),
            "{usable}"
        );
    }
}

#[test]
fn modifications_are_listed_as_reported_and_greyed_without_the_capability() {
    let liberator = staged(&unlocked(SOME_MODS));
    let listed = |mods| {
        let toggles: Vec<Toggle> = toggles(mods, &liberator);
        let row = |toggle: &Toggle| (toggle.label, toggle.usable, toggle.checked);
        toggles.iter().map(row).collect::<Vec<_>>()
    };
    assert_eq!(
        listed(modifications::PLAYERS),
        [
            ("Deathless Players and Hostage", true, false),
            ("Unlimited Ammo", false, false)
        ]
    );
    assert_eq!(listed(modifications::MATCH), []);
    assert_eq!(
        listed(modifications::LOADOUT),
        [("Disable Primary Weapon", true, false)]
    );
    let every = [
        "deathless",
        "unlimitedEquip",
        "unlimitedAmmo",
        "infiniteTime",
        "disableAI",
        "displayBuild",
        "disablePrimary",
        "disableSecondary",
        "disablePrimaryGadget",
        "disableSecondaryGadget",
    ];
    let all: String = every.map(|key| format!(r#","{key}":true"#)).concat();
    let liberator = staged(&unlocked(&all));
    let groups = [
        modifications::PLAYERS,
        modifications::MATCH,
        modifications::LOADOUT,
    ];
    let keys = groups.map(|mods| {
        let toggles = toggles(mods, &liberator);
        assert!(toggles.iter().all(|toggle| toggle.usable));
        let keys = toggles.iter().map(|toggle| toggle.capability.key());
        keys.collect::<Vec<_>>()
    });
    assert_eq!(keys.concat(), every);
}

#[test]
fn columns_follow_the_open_path() {
    let liberator = staged(TREE);
    let roots = liberator.tree().unwrap();
    assert_eq!(active_path(roots, &[]), [0]);
    assert_eq!(active_path(roots, &[2, 0]), [2, 0]);
    assert_eq!(active_path(&roots[1..], &[]), [0; 0]);
    assert_eq!(active_path(&[], &[]), [0; 0]);
    fn texts(nodes: &[Node]) -> Vec<&str> {
        nodes.iter().map(|node| node.text.as_ref()).collect()
    }
    let shown = |path: &[usize]| {
        let columns = columns(roots, path).into_iter();
        columns.map(texts).collect::<Vec<_>>()
    };
    let top = ["PvP", "Tutorial", "PvE"];
    let (modes, hunts) = (["Bomb", "Hostage"], ["Hunt"]);
    assert_eq!(shown(&[]), [top]);
    assert_eq!(shown(&[0]), [&top[..], &modes]);
    assert_eq!(shown(&[0, 1]), [&top[..], &modes]);
    assert_eq!(shown(&[1]), [top]);
    assert_eq!(shown(&[1, 0]), [top]);
    assert_eq!(shown(&[7]), [top]);
    assert_eq!(shown(&[2]), [&top[..], &hunts]);
    assert_eq!(shown(&[2, 0]), [&top[..], &hunts, &["Bank"]]);
    assert_eq!(shown(&[2, 0, 0]), [&top[..], &hunts, &["Bank"]]);
    assert_eq!(shown(&[2, 4]), [&top[..], &hunts]);
}

#[test]
fn one_icon_tells_the_state_of_the_liberator() {
    let state = |attached, unlocked, unsupported| State {
        attached,
        unlocked,
        unsupported,
        ..State::default()
    };
    let (idle, loading) = (state(false, false, false), state(true, false, false));
    let (open, foreign) = (state(true, true, false), state(true, false, true));
    for state in [&idle, &loading, &open, &foreign] {
        assert_eq!(status(state, false, true), None);
        assert_eq!(status(state, false, false), None);
        assert_eq!(status(state, true, false), Some(Status::Problem));
    }
    assert_eq!(status(&idle, true, true), None);
    assert_eq!(status(&loading, true, true), Some(Status::Loading));
    assert_eq!(status(&open, true, true), Some(Status::Unlocked));
    assert_eq!(status(&foreign, true, true), Some(Status::Problem));
    let both = state(true, true, true);
    assert_eq!(status(&both, true, true), Some(Status::Problem));
}

#[gpui::test]
fn the_navigation_ring_turns_only_while_the_liberator_loads(cx: &mut TestAppContext) {
    let (bench, cx) = bench(cx, "");
    let shown = Rc::new(Cell::new(false));
    let seen = shown.clone();
    let cx = mount(cx, move |window, cx| {
        let ring = loading_ring(window, cx);
        seen.set(ring.is_some());
        div().children(ring).into_any_element()
    });
    let ring = |cx: &mut VisualTestContext, lines: &str| {
        bench.say(cx, lines);
        cx.update(|window, _| window.refresh());
        shown.get()
    };
    assert!(!ring(cx, ""));
    assert!(ring(cx, LOADING));
    let foreign = r#"{"event":"state","attached":true,"unsupported":true}"#;
    assert!(!ring(cx, foreign));
    assert!(!ring(cx, &unlocked("")));
    assert!(!ring(cx, DETACHED));
    assert!(ring(cx, LOADING));
    bench.launcher.update(cx, |launcher, cx| {
        let settings = &mut launcher.settings;
        settings.set_flag(Flag::LiberatorEnabled, false, cx)
    });
    assert!(!ring(cx, LOADING));
}

#[gpui::test]
fn the_page_follows_the_runtime(cx: &mut TestAppContext) {
    let (bench, cx) = bench(cx, "");
    assert_eq!(bench.tab(cx), Tab::Support);
    bench.say(cx, LOADING);
    bench.say(cx, TREE);
    assert_eq!(bench.tab(cx), Tab::Support);
    bench.say(cx, &unlocked(SOME_MODS));
    assert_eq!(bench.tab(cx), Tab::Playlist);

    press(cx, "tab right");
    assert_eq!(bench.tab(cx), Tab::Modifications);
    bench.say(cx, NO_TREE);
    bench.say(cx, TREE);
    assert_eq!(bench.tab(cx), Tab::Modifications);
    press(cx, "right");
    assert_eq!(bench.tab(cx), Tab::Support);
    bench.say(cx, NO_TREE);
    press(cx, "right");
    assert_eq!(bench.tab(cx), Tab::Modifications);
    press(cx, "left");
    assert_eq!(bench.tab(cx), Tab::Support);
    bench.say(cx, TREE);
    assert_eq!(bench.tab(cx), Tab::Playlist);

    bench.say(cx, NO_TREE);
    assert_eq!(bench.tab(cx), Tab::Playlist);
    bench.say(cx, TREE);
    press(cx, "right");
    bench.say(cx, &unlocked(""));
    assert_eq!(bench.tab(cx), Tab::Support);
    press(cx, "left");
    assert_eq!(bench.tab(cx), Tab::Playlist);
    bench.say(cx, DETACHED);
    assert_eq!(bench.tab(cx), Tab::Support);
    press(cx, "left right");
    assert_eq!(bench.tab(cx), Tab::Support);
}

#[gpui::test]
fn a_loaded_playlist_is_shown_when_the_page_opens(cx: &mut TestAppContext) {
    let lines = [&unlocked(SOME_MODS), TREE].join("\n");
    let (bench, cx) = bench(cx, &lines);
    assert_eq!(bench.tab(cx), Tab::Playlist);
}

#[gpui::test]
fn the_switch_stops_and_starts_the_liberator(cx: &mut TestAppContext) {
    let (bench, cx) = bench(cx, LOADING);
    let enabled = |cx: &mut VisualTestContext| {
        bench.launcher.read_with(cx, |launcher, _| {
            launcher.settings.flag(Flag::LiberatorEnabled)
        })
    };
    assert!(enabled(cx));
    press(cx, "tab tab enter");
    assert!(!enabled(cx));
    assert!(bench.read(cx, |liberator| !liberator.state().attached));
    bench.launcher.update(cx, |launcher, _| {
        let running = ("Y5S3_ShadowLegacy".to_owned(), vec![1]);
        launcher.watchdog.set_folders([running].into());
    });
    press(cx, "space");
    assert!(enabled(cx));
    assert!(bench.paths.liberator_bin().is_file());
}

#[gpui::test]
fn the_playlist_opens_branches_and_picks_leaves(cx: &mut TestAppContext) {
    let lines = [&unlocked(""), TREE].join("\n");
    let (bench, cx) = bench(cx, &lines);
    let chosen = |cx: &mut VisualTestContext| {
        bench.read(cx, |liberator| {
            (liberator.path().to_vec(), liberator.picked().to_string())
        })
    };
    assert_eq!(chosen(cx), (vec![], String::new()));
    press(cx, "tab tab tab tab tab tab tab enter");
    assert_eq!(chosen(cx), (vec![0], "hostage".to_owned()));
    press(cx, "shift-tab shift-tab enter");
    assert_eq!(chosen(cx), (vec![2], "hostage".to_owned()));
    press(cx, "tab enter");
    assert_eq!(chosen(cx), (vec![2, 0], "hostage".to_owned()));
    press(cx, "tab enter");
    assert_eq!(chosen(cx), (vec![2, 0], "hunt:bank".to_owned()));
    press(cx, "shift-tab shift-tab shift-tab enter");
    assert_eq!(chosen(cx), (vec![], "tutorial".to_owned()));
    press(cx, "tab tab enter");
    assert_eq!(chosen(cx), (vec![0], "bomb".to_owned()));

    bench.say(cx, DETACHED);
    assert_eq!(chosen(cx), (vec![], String::new()));
}

#[gpui::test]
fn modifications_switch_what_the_runtime_allows(cx: &mut TestAppContext) {
    let (bench, cx) = bench(cx, &unlocked(SOME_MODS));
    let switched = |cx: &mut VisualTestContext| {
        bench.read(cx, |liberator| {
            let mods = [
                Capability::Deathless,
                Capability::UnlimitedAmmo,
                Capability::DisablePrimary,
            ];
            mods.map(|capability| liberator.switched(capability))
        })
    };
    press(cx, "tab left");
    assert_eq!(bench.tab(cx), Tab::Modifications);
    press(cx, "tab tab enter");
    assert_eq!(switched(cx), [true, false, false]);
    assert!(bench.read(cx, |liberator| {
        toggles(modifications::PLAYERS, liberator)[0].checked
    }));
    press(cx, "tab enter");
    assert_eq!(switched(cx), [true, false, true]);
    press(cx, "tab enter tab enter");
    assert_eq!(switched(cx), [true, false, true]);
    press(cx, "shift-tab shift-tab shift-tab enter");
    assert_eq!(switched(cx), [false, false, true]);

    bench.say(cx, LOADING);
    assert_eq!(switched(cx), [false, false, true]);
    bench.say(cx, DETACHED);
    assert_eq!(switched(cx), [false, false, false]);
}

#[gpui::test]
fn the_year_starts_over_after_a_tab_change(cx: &mut TestAppContext) {
    let (bench, cx) = bench(cx, &unlocked(SOME_MODS));
    let year = |cx: &mut VisualTestContext| {
        bench
            .page
            .read_with(cx, |page, cx| page.support.read(cx).year)
    };
    press(cx, "tab tab tab tab tab enter");
    assert_eq!(year(cx), 2);
    bench.say(cx, LOADING);
    assert_eq!(year(cx), 2);
    bench.say(cx, &unlocked(SOME_MODS));
    press(cx, "shift-tab shift-tab shift-tab shift-tab left");
    assert_eq!((bench.tab(cx), year(cx)), (Tab::Modifications, 0));
    press(cx, "right tab tab tab enter");
    assert_eq!((bench.tab(cx), year(cx)), (Tab::Support, 1));
    bench.say(cx, TREE);
    assert_eq!((bench.tab(cx), year(cx)), (Tab::Playlist, 0));
}

#[gpui::test]
fn focus_returns_to_the_tabs_when_its_place_goes_away(cx: &mut TestAppContext) {
    let (bench, cx) = bench(cx, &unlocked(SOME_MODS));
    press(cx, "tab tab tab");
    bench.say(cx, TREE);
    assert_eq!(bench.tab(cx), Tab::Playlist);
    press(cx, "right");
    assert_eq!(bench.tab(cx), Tab::Modifications);

    bench.say(cx, &unlocked(""));
    assert_eq!(bench.tab(cx), Tab::Support);
    press(cx, "left");
    assert_eq!(bench.tab(cx), Tab::Playlist);

    press(cx, "tab tab");
    bench.say(cx, &[DETACHED, NO_TREE].join("\n"));
    assert_eq!(bench.tab(cx), Tab::Support);
    bench.say(cx, &unlocked(SOME_MODS));
    press(cx, "left");
    assert_eq!(bench.tab(cx), Tab::Modifications);

    press(cx, "tab");
    bench.say(cx, &unlocked(""));
    assert_eq!(bench.tab(cx), Tab::Support);
    bench.say(cx, &unlocked(SOME_MODS));
    press(cx, "left");
    assert_eq!(bench.tab(cx), Tab::Support);
}

#[gpui::test]
fn the_missing_modal_waits_for_the_page_and_stays_away_after_cancel(cx: &mut TestAppContext) {
    let (bench, cx) = bench(cx, "tried");
    assert!(!bench.missing(cx));
    cx.executor().advance_clock(shell::ENTER);
    cx.run_until_parked();
    assert!(bench.missing(cx));
    press(cx, "escape");
    assert!(!bench.missing(cx));
    bench.say(cx, "");
    assert!(!bench.missing(cx));
    assert!(bench.read(cx, |liberator| !liberator.available()));
}

#[gpui::test]
fn the_missing_modal_downloads_the_file_and_leaves(cx: &mut TestAppContext) {
    let (bench, cx) = bench(cx, "tried");
    cx.executor().advance_clock(shell::ENTER);
    cx.run_until_parked();
    assert!(bench.missing(cx));
    press(cx, "enter");
    assert!(bench.paths.liberator_bin().is_file());
    assert!(!bench.missing(cx));
    assert!(bench.page.read_with(cx, |page, _| !page.dismissed));
}

#[gpui::test]
fn a_running_download_keeps_the_missing_modal_open(cx: &mut TestAppContext) {
    let (bench, cx) = bench(cx, "tried\nbusy");
    cx.executor().advance_clock(shell::ENTER);
    cx.run_until_parked();
    assert!(bench.missing(cx));
    press(cx, "escape enter tab enter tab enter");
    assert!(bench.missing(cx));
    assert!(!bench.paths.liberator_bin().exists());
}

#[gpui::test]
fn the_missing_modal_keeps_the_focus_when_the_tab_changes_under_it(cx: &mut TestAppContext) {
    let (bench, cx) = bench(cx, &unlocked(SOME_MODS));
    cx.executor().advance_clock(shell::ENTER);
    cx.run_until_parked();
    press(cx, "tab left tab tab");
    bench.say(cx, &["tried", DETACHED].join("\n"));
    assert_eq!(bench.tab(cx), Tab::Support);
    assert!(bench.missing(cx));
    press(cx, "escape");
    assert!(!bench.missing(cx));
}

#[gpui::test]
fn the_missing_modal_needs_the_liberator_switched_on(cx: &mut TestAppContext) {
    let (bench, cx) = bench(cx, "tried");
    bench.launcher.update(cx, |launcher, cx| {
        launcher.set_liberator_enabled(false, cx);
    });
    cx.executor().advance_clock(shell::ENTER);
    cx.run_until_parked();
    assert!(!bench.missing(cx));
    bench.launcher.update(cx, |launcher, cx| {
        launcher.set_liberator_enabled(true, cx);
    });
    cx.run_until_parked();
    assert!(bench.missing(cx));
    bench.say(cx, LOADING);
    assert!(!bench.missing(cx));
}
