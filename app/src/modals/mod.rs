mod login;
pub mod steps;

use std::path::PathBuf;
use std::rc::Rc;

use backend::env::IS_WINDOWS;
use gpui::{
    AnyElement, App, Context, Deferred, Div, Entity, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Stateful, StatefulInteractiveElement, Styled,
    Subscription, Window, div, prelude::FluentBuilder,
};

use crate::model::Launcher;
use crate::model::download::{DownloadEvent, LoginKind};
use crate::model::launch::LaunchEvent;
use crate::model::liberator::LiberatorEvent;
use crate::model::settings::Library;
use crate::ui::button::{primary, secondary};
use crate::ui::kit::{busy, disable};
use crate::ui::modal::Modal;
use crate::ui::prose::{BODY, inline, link};
use crate::ui::theme::{FONT_BODY, TEXT_UI, Typeface};
use login::{Form, Submission};
use steps::{archive_steps, exclusion};

const LOADER_URL: &str = "https://github.com/xeralin/ThrowbackLoader";

struct Slot<T> {
    modal: Modal,
    shown: Option<T>,
}

impl<T> Slot<T> {
    fn new(cx: &mut App) -> Self {
        Self {
            modal: Modal::new(cx),
            shown: None,
        }
    }

    fn show(&mut self, content: T, window: &mut Window, cx: &mut App) {
        self.shown = Some(content);
        self.modal.close(window, cx);
        self.modal.open(window, cx);
    }

    fn hide(&mut self, window: &mut Window, cx: &mut App) -> Option<T> {
        self.modal.close(window, cx);
        self.shown.take()
    }
}

struct Missing {
    key: String,
    hm: bool,
    library: Option<SharedString>,
    picking: Option<bool>,
}

pub struct Modals {
    launcher: Entity<Launcher>,
    login: Slot<Form>,
    space: Slot<f64>,
    missing: Slot<Missing>,
    exclusion: Slot<()>,
    _subscriptions: [Subscription; 4],
}

fn layer(
    name: &'static str,
    window: &mut Window,
    build: impl FnOnce(&mut Window) -> Option<Deferred>,
) -> Option<Stateful<Div>> {
    let modal = window.with_element_namespace(name, build)?;
    Some(div().id(name).child(modal))
}

impl Modals {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let launcher = Launcher::handle(cx);
        let (login, shortfall) = {
            let download = &launcher.read(cx).download;
            (download.login_kind(), download.disk_shortfall())
        };
        let mut modals = Self {
            login: Slot::new(cx),
            space: Slot::new(cx),
            missing: Slot::new(cx),
            exclusion: Slot::new(cx),
            _subscriptions: [
                cx.subscribe_in(&launcher, window, Self::download_event),
                cx.subscribe_in(&launcher, window, Self::launch_event),
                cx.subscribe_in(&launcher, window, Self::liberator_event),
                cx.on_release(|modals, cx| {
                    modals
                        .launcher
                        .update(cx, |launcher, _| launcher.stop_qr_login());
                }),
            ],
            launcher,
        };
        if let Some(shortfall) = shortfall {
            modals.space.show(shortfall, window, cx);
        }
        modals.ask_login(login, window, cx);
        modals
    }

    fn download_event(
        &mut self,
        _: &Entity<Launcher>,
        event: &DownloadEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            DownloadEvent::LoginRequired(kind) => self.ask_login(*kind, window, cx),
            DownloadEvent::Done { .. } => self.ask_login(None, window, cx),
            DownloadEvent::DiskSpaceRequired(shortfall) => {
                self.space.show(*shortfall, window, cx);
                cx.notify();
            }
            DownloadEvent::FilesMissing { key, library } => {
                self.report_missing(key, true, library.clone(), window, cx)
            }
            DownloadEvent::HmArchivePicked(picked) => self.archive_picked(*picked, window, cx),
            _ => {}
        }
    }

    fn launch_event(
        &mut self,
        _: &Entity<Launcher>,
        event: &LaunchEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let LaunchEvent::FilesMissing { edition, library } = event {
            self.report_missing(&edition.key, edition.hm, library.clone(), window, cx);
        }
    }

    fn liberator_event(
        &mut self,
        _: &Entity<Launcher>,
        event: &LiberatorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            LiberatorEvent::Exclusion => self.exclusion.show((), window, cx),
            LiberatorEvent::Restored | LiberatorEvent::Error(_) => {
                self.exclusion.hide(window, cx);
            }
        }
        cx.notify();
    }

    fn exclusion_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Deferred> {
        self.exclusion.shown?;
        let risk = "Windows Security may remove `Liberator.exe` and other files.";
        let advice = inline("exclusion-risk", risk, BODY, window, cx);
        let body = exclusion(advice, None, window, cx);
        let quit =
            secondary("exclusion-cancel", "Cancel", window, cx).on_click(|_, _, cx| cx.quit());
        let download = primary("exclusion-download", "Download", window, cx);
        let (quit, download) = if self.launcher.read(cx).liberator.busy() {
            (disable(quit), busy(download, window, cx))
        } else {
            let launcher = self.launcher.clone();
            let download = download.on_click(move |_, _, cx| {
                launcher.update(cx, |launcher, cx| launcher.liberator.restore(cx));
            });
            (quit, download)
        };
        let sheet = self.exclusion.modal.sheet(body);
        let sheet = sheet.title("Antivirus exclusion");
        Some(sheet.footer([quit, download]).show(window, cx))
    }

    fn ask_login(&mut self, kind: Option<LoginKind>, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.login.shown.as_ref().map(|form| form.kind);
        if kind == shown {
            return;
        }
        match kind {
            Some(kind) => {
                let form = Form::new(kind, window, cx);
                let first = form.first(cx);
                self.login.show(form, window, cx);
                if let Some(first) = first {
                    window.focus(&first, cx);
                }
                if kind == LoginKind::Account {
                    self.launcher
                        .update(cx, |launcher, cx| launcher.start_qr_login(cx));
                }
            }
            None => {
                self.login.hide(window, cx);
            }
        }
        cx.notify();
    }

    fn submit_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let form = self.login.shown.as_ref();
        let Some(submission) = form.and_then(|form| form.submission(cx)) else {
            return;
        };
        self.ask_login(None, window, cx);
        self.launcher.update(cx, |launcher, cx| match submission {
            Submission::Account { account, password } => {
                launcher.submit_account_login(&account, &password, cx)
            }
            Submission::Text(text) => launcher.submit_login(&text, cx),
        });
    }

    fn cancel_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ask_login(None, window, cx);
        self.launcher
            .update(cx, |launcher, cx| launcher.cancel_download(cx));
    }

    fn answer_space(&mut self, proceed: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.space.hide(window, cx);
        cx.notify();
        self.launcher.update(cx, |launcher, cx| {
            if proceed {
                launcher.confirm_disk_space(cx);
            } else {
                launcher.cancel_download(cx);
            }
        });
    }

    fn report_missing(
        &mut self,
        key: &str,
        hm: bool,
        library: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = library.to_string_lossy();
        let launcher = self.launcher.clone();
        launcher.update(cx, |launcher, cx| launcher.survey_libraries(cx));
        let libraries = launcher.read(cx).settings.libraries().iter();
        let listed = |entry: &&Library| !entry.fixed && entry.exists && entry.path.as_ref() == path;
        let missing = Missing {
            key: key.to_owned(),
            hm,
            library: libraries
                .filter(listed)
                .map(|entry| entry.path.clone())
                .next(),
            picking: None,
        };
        self.missing.show(missing, window, cx);
        cx.notify();
    }

    fn close_missing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picking = self
            .missing
            .shown
            .as_ref()
            .and_then(|missing| missing.picking);
        if picking != Some(true) {
            self.missing.hide(window, cx);
            cx.notify();
        }
    }

    fn verify(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut missing) = self.missing.hide(window, cx) else {
            return;
        };
        cx.notify();
        let launcher = self.launcher.read(cx);
        let mut seasons = launcher.seasons().into_iter();
        let unstable =
            missing.hm && seasons.any(|season| season.key == missing.key && season.hm_beta);
        if unstable && !launcher.hm_archive_cached() {
            missing.picking = Some(false);
            return self.missing.show(missing, window, cx);
        }
        self.restore(missing, cx);
    }

    fn restore(&mut self, missing: Missing, cx: &mut Context<Self>) {
        let Missing { key, hm, .. } = missing;
        self.launcher.update(cx, |launcher, cx| {
            if !hm {
                launcher.verify(&key, false, cx);
            } else if launcher.installs(&key).hm.installed {
                launcher.restore_hm(&key, cx);
            } else {
                launcher.enqueue(&key, true, "", cx);
            }
        });
    }

    fn select_archive(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(missing) = &mut self.missing.shown
            && missing.picking == Some(false)
        {
            missing.picking = Some(true);
            cx.notify();
            self.launcher
                .update(cx, |launcher, cx| launcher.pick_hm_archive(cx));
        }
    }

    fn archive_picked(&mut self, picked: bool, window: &mut Window, cx: &mut Context<Self>) {
        let shown = &mut self.missing.shown;
        let Some(missing) = shown
            .as_mut()
            .filter(|missing| missing.picking == Some(true))
        else {
            return;
        };
        missing.picking = Some(false);
        cx.notify();
        if picked && let Some(missing) = self.missing.hide(window, cx) {
            self.restore(missing, cx);
        }
    }

    fn login_modal(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<Deferred> {
        let form = self.login.shown.as_ref()?;
        let cancel = Rc::new(window.handler_for(&cx.entity(), Self::cancel_login));
        let submit = Rc::new(window.handler_for(&cx.entity(), Self::submit_login));
        let ready = form.submission(cx).is_some();
        let guide = (form.kind == LoginKind::Account).then(|| {
            link(
                "login-guide",
                "Why does the Launcher need my Steam login?",
                login::FAQ_URL,
                window,
                cx,
            )
            .mr_auto()
            .min_w_0()
            .face(FONT_BODY, TEXT_UI)
        });
        let dismiss = cancel.clone();
        let back = secondary("login-cancel", "Cancel", window, cx)
            .on_click(move |_, window, cx| dismiss(window, cx));
        let guide = guide.map(IntoElement::into_any_element);
        let mut keys: Vec<AnyElement> = guide.into_iter().collect();
        keys.push(back.into_any_element());
        let asks = form.asks();
        if asks {
            let send = submit.clone();
            let log_in = primary("login-submit", "Log in", window, cx)
                .on_click(move |_, window, cx| send(window, cx))
                .when(!ready, disable);
            keys.push(log_in.into_any_element());
        }
        let body = form.body(window, cx);
        let sheet = self
            .login
            .modal
            .sheet(body)
            .title(form.title())
            .footer(keys);
        let sheet = sheet.form().on_close(move |window, cx| cancel(window, cx));
        let sheet = match asks {
            true => sheet.on_confirm(move |window, cx| submit(window, cx)),
            false => sheet,
        };
        let code = self.launcher.read(cx).download.qr_code();
        let sheet = match (form.kind, code) {
            (LoginKind::Account, Some(code)) => sheet.side(login::qr_panel(code)),
            _ => sheet,
        };
        Some(sheet.show(window, cx))
    }

    fn space_modal(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<Deferred> {
        let shortfall = self.space.shown?;
        let this = cx.entity();
        let cancel = window.handler_for(&this, |this, window, cx| {
            this.answer_space(false, window, cx)
        });
        let proceed = window.handler_for(&this, |this, window, cx| {
            this.answer_space(true, window, cx)
        });
        let ask = format!("Free up `{shortfall} GB` with Shears or pick a different library.");
        let body = inline("space-ask", &ask, BODY, window, cx);
        let sheet = self.space.modal.sheet(body).title("Not enough disk space");
        let sheet = sheet.confirm("Download anyway", false, proceed, cancel);
        Some(sheet.without_enter().show(window, cx))
    }

    fn missing_modal(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<Deferred> {
        let missing = self.missing.shown.as_ref()?;
        let modal = &self.missing.modal;
        let cancel = window.handler_for(&cx.entity(), Self::close_missing);
        let Some(picking) = missing.picking else {
            let verify = window.handler_for(&cx.entity(), Self::verify);
            let body = Self::missing_body(missing, window, cx);
            let sheet = modal.sheet(body).title("Files are missing");
            return Some(
                sheet
                    .confirm("Verify", false, verify, cancel)
                    .show(window, cx),
            );
        };
        let select = window.handler_for(&cx.entity(), Self::select_archive);
        let sheet = modal.sheet(archive_steps(window, cx)).title("Heated Metal");
        let sheet = sheet.confirm("Select HM", picking, select, cancel);
        Some(sheet.show(window, cx))
    }

    fn missing_body(missing: &Missing, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let loader = format!("[ThrowbackLoader]({LOADER_URL})");
        let sentence = match (IS_WINDOWS, missing.hm) {
            (false, true) => "Heated Metal files were not found.".to_owned(),
            (false, false) => format!("{loader} files were not found."),
            (true, true) => "Windows Security may have removed Heated Metal files.".to_owned(),
            (true, false) => format!("Windows Security may have removed {loader} files."),
        };
        let sentence = inline("missing-files", &sentence, BODY, window, cx);
        if !IS_WINDOWS {
            return sentence;
        }
        exclusion(sentence, missing.library.clone(), window, cx)
    }
}

impl Render for Modals {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let step = match &self.missing.shown {
            Some(Missing {
                picking: Some(_), ..
            }) => "select",
            _ => "missing",
        };
        div()
            .absolute()
            .children(layer(step, window, |window| self.missing_modal(window, cx)))
            .children(layer("space", window, |window| {
                self.space_modal(window, cx)
            }))
            .children(layer("login", window, |window| {
                self.login_modal(window, cx)
            }))
            .children(layer("exclusion", window, |window| {
                self.exclusion_modal(window, cx)
            }))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::sync::{Arc, Mutex};

    use backend::env::{Paths, TL_LAUNCHER};
    use backend::manifest;
    use backend::settings::Settings as Store;
    use gpui::{Modifiers, TestAppContext, VisualTestContext, point};
    use tempfile::TempDir;

    use super::*;
    use crate::model::Edition;
    use crate::model::download::Downloader;
    use crate::ui::px;
    use crate::ui::testing::press;

    const VANILLA: &str = "Y1S0_Vanilla";
    const BLOOD: &str = "Y9S2_NewBlood";

    type Typed = Arc<Mutex<Vec<String>>>;

    struct Bench {
        _dir: TempDir,
        launcher: Entity<Launcher>,
        modals: Entity<Modals>,
        errors: Rc<RefCell<Vec<String>>>,
    }

    #[derive(Debug, PartialEq)]
    struct Shown {
        login: Option<LoginKind>,
        space: Option<f64>,
        missing: Option<(String, bool, Option<bool>)>,
    }

    const NOTHING: Shown = Shown {
        login: None,
        space: None,
        missing: None,
    };

    fn bench(cx: &mut TestAppContext, download: Downloader) -> (Bench, &mut VisualTestContext) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        let library = dir.path().join("lib");
        fs::create_dir_all(&paths.data_root).unwrap();
        fs::create_dir_all(&library).unwrap();
        let library = library.display();
        let settings = format!("[settings]\ndiscord_rpc = false\nlibraries = ['{library}']\n");
        fs::write(paths.settings_file(), settings).unwrap();
        let errors = Rc::new(RefCell::new(Vec::new()));
        let launcher = cx.update(|cx| {
            crate::init(cx);
            let store = Store::load(&paths).unwrap();
            let mut launcher = Launcher::new(paths.clone(), store, manifest::downloads());
            launcher.download = download;
            launcher.init(cx);
            let launcher = Launcher::handle(cx);
            let log = errors.clone();
            cx.subscribe(&launcher, move |_, event: &DownloadEvent, _| {
                if let DownloadEvent::Error(message) = event {
                    log.borrow_mut().push(message.to_string());
                }
            })
            .detach();
            launcher
        });
        let (modals, cx) = cx.add_window_view(Modals::new);
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        let bench = Bench {
            _dir: dir,
            launcher,
            modals,
            errors,
        };
        (bench, cx)
    }

    fn shown(bench: &Bench, cx: &mut VisualTestContext) -> Shown {
        bench.modals.read_with(cx, |modals, _| Shown {
            login: modals.login.shown.as_ref().map(|form| form.kind),
            space: modals.space.shown,
            missing: modals
                .missing
                .shown
                .as_ref()
                .map(|missing| (missing.key.clone(), missing.hm, missing.picking)),
        })
    }

    fn login(kind: LoginKind) -> Shown {
        Shown {
            login: Some(kind),
            ..NOTHING
        }
    }

    fn submission(bench: &Bench, cx: &mut VisualTestContext) -> Option<Submission> {
        bench.modals.read_with(cx, |modals, cx| {
            let form = modals.login.shown.as_ref();
            form.and_then(|form| form.submission(cx))
        })
    }

    fn account(bench: &Bench, cx: &mut VisualTestContext) -> String {
        bench
            .launcher
            .read_with(cx, |launcher, _| launcher.settings.steam_account().into())
    }

    fn asking(kind: LoginKind) -> (Downloader, Typed) {
        let typed = Typed::default();
        (Downloader::asking(kind, typed.clone()), typed)
    }

    fn report(bench: &Bench, key: &str, hm: bool, cx: &mut VisualTestContext) {
        let edition = Edition {
            key: key.to_owned(),
            hm,
        };
        let library = "/games".into();
        bench.launcher.update(cx, |_, cx| {
            cx.emit(LaunchEvent::FilesMissing { edition, library })
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn closing_the_window_ends_the_qr_login(cx: &mut TestAppContext) {
        let typed = Typed::default();
        let (Bench { modals, .. }, cx) = bench(cx, Downloader::signing_in(typed.clone()));
        assert!(typed.lock().unwrap().is_empty());
        drop(modals);
        cx.update(|window, _| window.remove_window());
        cx.run_until_parked();
        assert_eq!(*typed.lock().unwrap(), ["killed"]);
    }

    #[gpui::test]
    fn each_login_kind_opens_its_modal(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, Downloader::default());
        assert_eq!(shown(&bench, cx), NOTHING);
        let title = |cx: &mut VisualTestContext| {
            bench.modals.read_with(cx, |modals, _| {
                let form = modals.login.shown.as_ref().unwrap();
                (form.title(), form.asks())
            })
        };
        for (kind, look) in [
            (LoginKind::Account, ("Steam login", true)),
            (LoginKind::Password, ("Steam login", true)),
            (LoginKind::Guard, ("Steam Guard", true)),
            (LoginKind::GuardEmail, ("Steam Guard", true)),
            (LoginKind::GuardMobile, ("Steam Guard", false)),
        ] {
            let asked = DownloadEvent::LoginRequired(Some(kind));
            bench.launcher.update(cx, |_, cx| cx.emit(asked));
            cx.run_until_parked();
            assert_eq!(shown(&bench, cx), login(kind));
            assert_eq!(title(cx), look);
            assert_eq!(submission(&bench, cx), None);
        }
        bench
            .launcher
            .update(cx, |_, cx| cx.emit(DownloadEvent::LoginRequired(None)));
        cx.run_until_parked();
        assert_eq!(shown(&bench, cx), NOTHING);
    }

    #[gpui::test]
    fn the_account_form_sends_the_trimmed_name_and_the_password_as_typed(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, Downloader::default());
        bench
            .launcher
            .update(cx, |launcher, cx| launcher.enqueue(VANILLA, false, "", cx));
        cx.run_until_parked();
        assert_eq!(shown(&bench, cx), login(LoginKind::Account));
        cx.simulate_input(" ash ");
        press(cx, "enter");
        assert_eq!(shown(&bench, cx), login(LoginKind::Account));
        cx.simulate_keystrokes("ctrl-a ctrl-c");
        press(cx, "tab");
        cx.simulate_input(" pass word ");
        cx.simulate_keystrokes("ctrl-a ctrl-c");
        let copied = cx.read_from_clipboard().and_then(|item| item.text());
        assert_eq!(copied.as_deref(), Some(" ash "));
        let filled = Submission::Account {
            account: "ash".into(),
            password: " pass word ".into(),
        };
        assert_eq!(submission(&bench, cx), Some(filled));
        press(cx, "enter");
        assert_eq!(shown(&bench, cx), NOTHING);
        assert_eq!(account(&bench, cx), "ash");
        assert_eq!(*bench.errors.borrow(), ["DepotDownloader failed to start"]);
    }

    #[gpui::test]
    fn a_rejected_account_name_asks_again_with_empty_fields(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, Downloader::default());
        bench
            .launcher
            .update(cx, |launcher, cx| launcher.enqueue(VANILLA, false, "", cx));
        cx.run_until_parked();
        cx.simulate_input("not a name");
        press(cx, "tab");
        cx.simulate_input("secret");
        press(cx, "tab tab tab tab enter");
        assert_eq!(*bench.errors.borrow(), ["Invalid Steam account"]);
        assert_eq!(shown(&bench, cx), login(LoginKind::Account));
        assert_eq!(submission(&bench, cx), None);
        assert_eq!(account(&bench, cx), "");
        cx.simulate_input("ash");
        assert_eq!(submission(&bench, cx), None);
    }

    #[gpui::test]
    fn a_code_or_password_goes_to_the_downloader_on_enter(cx: &mut TestAppContext) {
        for kind in [LoginKind::Password, LoginKind::Guard, LoginKind::GuardEmail] {
            let (download, typed) = asking(kind);
            let (bench, cx) = bench(cx, download);
            assert_eq!(shown(&bench, cx), login(kind));
            press(cx, "enter");
            assert_eq!(shown(&bench, cx), login(kind));
            cx.simulate_input(" 5k2pq");
            press(cx, "enter");
            assert_eq!(shown(&bench, cx), NOTHING);
            assert_eq!(*typed.lock().unwrap(), [" 5k2pq"]);
        }
    }

    #[gpui::test]
    fn the_mobile_confirmation_only_cancels(cx: &mut TestAppContext) {
        let (download, typed) = asking(LoginKind::GuardMobile);
        let (bench, cx) = bench(cx, download);
        press(cx, "enter");
        assert_eq!(shown(&bench, cx), login(LoginKind::GuardMobile));
        press(cx, "tab enter");
        assert_eq!(shown(&bench, cx), NOTHING);
        assert_eq!(*typed.lock().unwrap(), ["killed"]);
    }

    #[gpui::test]
    fn escape_in_a_field_and_the_backdrop_cancel_the_download(cx: &mut TestAppContext) {
        let (download, typed) = asking(LoginKind::Guard);
        let (bench, cx) = bench(cx, download);
        cx.simulate_input("12");
        press(cx, "escape");
        assert_eq!(shown(&bench, cx), NOTHING);
        assert_eq!(*typed.lock().unwrap(), ["killed"]);
        let asked = DownloadEvent::LoginRequired(Some(LoginKind::Guard));
        bench.launcher.update(cx, |_, cx| cx.emit(asked));
        cx.run_until_parked();
        assert_eq!(submission(&bench, cx), None);
        cx.simulate_click(point(px(2.), px(2.)), Modifiers::default());
        assert_eq!(shown(&bench, cx), NOTHING);
        assert_eq!(*typed.lock().unwrap(), ["killed", "killed"]);
    }

    #[gpui::test]
    fn the_end_of_a_download_closes_the_login(cx: &mut TestAppContext) {
        let (download, typed) = asking(LoginKind::Password);
        let (bench, cx) = bench(cx, download);
        cx.simulate_input("secret");
        let done = DownloadEvent::Done {
            key: VANILLA.into(),
            outcome: crate::model::download::Outcome::Failed,
        };
        bench.launcher.update(cx, |_, cx| cx.emit(done));
        cx.run_until_parked();
        assert_eq!(shown(&bench, cx), NOTHING);
        assert!(typed.lock().unwrap().is_empty());
    }

    #[gpui::test]
    fn download_anyway_goes_on_and_everything_else_drops_the_download(cx: &mut TestAppContext) {
        let pending = |bench: &Bench, cx: &mut VisualTestContext| {
            bench
                .launcher
                .read_with(cx, |launcher, _| launcher.download.disk_shortfall())
        };
        let (bench, cx) = bench(cx, Downloader::lacking(VANILLA, 12.5));
        let asked = Shown {
            space: Some(12.5),
            ..NOTHING
        };
        assert_eq!(shown(&bench, cx), asked);
        press(cx, "enter");
        assert_eq!(shown(&bench, cx), asked);
        press(cx, "tab tab enter");
        assert_eq!(shown(&bench, cx), login(LoginKind::Account));
        assert_eq!(pending(&bench, cx), None);

        press(cx, "escape");
        let short = DownloadEvent::DiskSpaceRequired(3.);
        bench.launcher.update(cx, |launcher, cx| {
            launcher.download = Downloader::lacking(VANILLA, 3.);
            cx.emit(short);
        });
        cx.run_until_parked();
        press(cx, "escape");
        assert_eq!(shown(&bench, cx), NOTHING);
        assert_eq!(pending(&bench, cx), None);
    }

    #[gpui::test]
    fn verify_restores_throwback_files_through_a_normal_verify(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, Downloader::default());
        report(&bench, VANILLA, false, cx);
        let missing = |key: &str, hm, picking| Shown {
            missing: Some((key.to_owned(), hm, picking)),
            ..NOTHING
        };
        assert_eq!(shown(&bench, cx), missing(VANILLA, false, None));
        press(cx, "escape");
        assert_eq!(shown(&bench, cx), NOTHING);
        let library = bench
            .launcher
            .read_with(cx, |launcher, _| launcher.settings.roots()[0].join(VANILLA));
        fs::create_dir_all(&library).unwrap();
        fs::write(library.join(TL_LAUNCHER), "").unwrap();
        report(&bench, VANILLA, false, cx);
        press(cx, "enter");
        assert_eq!(shown(&bench, cx), login(LoginKind::Account));
        assert_eq!(*bench.errors.borrow(), Vec::<String>::new());
    }

    #[gpui::test]
    fn the_unstable_build_asks_for_its_archive_before_it_verifies(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, Downloader::default());
        let missing = |picking| Shown {
            missing: Some((BLOOD.to_owned(), true, picking)),
            ..NOTHING
        };
        let found = DownloadEvent::FilesMissing {
            key: BLOOD.into(),
            library: "/games".into(),
        };
        bench.launcher.update(cx, |_, cx| cx.emit(found));
        cx.run_until_parked();
        assert_eq!(shown(&bench, cx), missing(None));
        press(cx, "tab tab enter");
        assert_eq!(shown(&bench, cx), missing(Some(false)));
        press(cx, "enter");
        assert_eq!(shown(&bench, cx), missing(Some(true)));
        press(cx, "escape");
        cx.simulate_click(point(px(2.), px(2.)), Modifiers::default());
        assert_eq!(shown(&bench, cx), missing(Some(true)));
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        assert_eq!(shown(&bench, cx), missing(Some(false)));
        press(cx, "escape");
        assert_eq!(shown(&bench, cx), NOTHING);
    }

    #[gpui::test]
    fn heated_metal_without_an_installation_goes_back_to_the_download(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, Downloader::default());
        report(&bench, "Y5S3_ShadowLegacy", true, cx);
        press(cx, "enter");
        assert_eq!(shown(&bench, cx), login(LoginKind::Account));
        let waits = bench
            .launcher
            .read_with(cx, |launcher, _| launcher.download.queue().count());
        assert_eq!(waits, 0);
    }

    #[gpui::test]
    fn the_exclusion_modal_stays_open_until_the_download_ends(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, Downloader::default());
        let open = |cx: &mut VisualTestContext| {
            bench
                .modals
                .read_with(cx, |modals, _| modals.exclusion.shown.is_some())
        };
        let ask = |cx: &mut VisualTestContext| {
            bench
                .launcher
                .update(cx, |_, cx| cx.emit(LiberatorEvent::Exclusion));
            cx.run_until_parked();
        };
        bench
            .launcher
            .update(cx, |launcher, _| launcher.liberator.stage(""));
        ask(cx);
        press(cx, "escape");
        cx.simulate_click(point(px(2.), px(2.)), Modifiers::default());
        assert!(open(cx));
        bench.launcher.update(cx, |launcher, cx| {
            launcher.liberator.restore(cx);
            assert!(launcher.liberator.busy());
        });
        assert!(open(cx));
        cx.run_until_parked();
        assert!(!open(cx));

        ask(cx);
        let failed = LiberatorEvent::Error("Liberator download failed".into());
        bench.launcher.update(cx, |_, cx| cx.emit(failed));
        cx.run_until_parked();
        assert!(!open(cx));
    }
}
