use std::mem;
use std::time::Duration;

use crate::ui::{px, units};
use backend::manifest::edition_folder;
use gpui::{
    Animation, AnimationExt, AnyElement, AnyView, App, AppContext, Context, Div, ElementId, Entity,
    FocusHandle, Global, HighlightStyle, Hsla, InteractiveElement, IntoElement, ParentElement,
    Pixels, Render, RetainAllImageCache, ScrollHandle, SharedString, Size, Stateful,
    StatefulInteractiveElement, Styled, StyledText, Subscription, Transformation, WeakEntity,
    Window, div, point, prelude::FluentBuilder, radians, size,
};

use crate::activity::activity;
use crate::modals::Modals;
use crate::model::{Edition, Launcher};
use crate::notices;
use crate::pages::download::DownloadPage;
use crate::pages::home::HomePage;
use crate::pages::liberator::{LiberatorPage, loading_ring};
use crate::pages::season::SeasonPage;
use crate::pages::settings::SettingsPage;
use crate::pages::updates::UpdatesPage;
use crate::ui::frame;
use crate::ui::icon::{BAR, icon};
use crate::ui::kit::{NAV_BREAKPOINT, external, focus_ring_inside};
use crate::ui::motion::{self, EASE, EASE_IN_OUT, EASE_OUT_CUBIC, FAST, SLOW};
use crate::ui::scroll::scroll;
use crate::ui::theme::{
    BG, BORDER, FONT_BODY, FONT_BODY_MEDIUM, FONT_DISPLAY_BOLD, FONT_MONO_4, FONT_MONO_20, REM,
    SURFACE, SURFACE_2, TEXT, TEXT_LABEL, TEXT_MICRO, TEXT_MUTED, Theme, Typeface, blend, shadow,
};
use crate::ui::{Dismiss, FocusNext, FocusPrevious, modal, toast};

const FAQ_URL: &str = "https://xeralin.github.io/ThrowbackFAQ/";
const BLINK: Duration = Duration::from_secs(1);
const TOPBAR: f32 = 52.;
const SLIDE: Duration = Duration::from_millis(320);
const HOP: Duration = Duration::from_millis(450);
const HOP_HEIGHT: f32 = 6.;
pub const ENTER: Duration = Duration::from_millis(350);

type Tools = Box<dyn Fn(&mut Window, &mut App) -> Option<AnyElement>>;
type Stops = Box<dyn Fn(&str, &App) -> Option<FocusHandle>>;

fn hop(elapsed: Duration) -> f32 {
    let phase = (elapsed.as_secs_f32() / HOP.as_secs_f32()).fract();
    HOP_HEIGHT * 4. * phase * (1. - phase)
}

#[derive(Clone, Copy, Default, PartialEq)]
pub enum Page {
    #[default]
    Home,
    Download,
    Liberator,
    Settings,
    Updates,
}

impl Page {
    fn label(self) -> &'static str {
        match self {
            Page::Home => "Home",
            Page::Download => "Download",
            Page::Liberator => "Liberator",
            Page::Settings => "Settings",
            Page::Updates => "Updates",
        }
    }
}

#[derive(Clone, Default, PartialEq)]
pub struct Place {
    page: Page,
    season: Option<Edition>,
}

impl From<Page> for Place {
    fn from(page: Page) -> Self {
        Self { page, season: None }
    }
}

const NAV: [(&str, &[Option<Page>]); 3] = [
    (
        "Library",
        &[
            Some(Page::Home),
            Some(Page::Download),
            Some(Page::Liberator),
        ],
    ),
    ("System", &[Some(Page::Settings), Some(Page::Updates)]),
    ("Help", &[None]),
];

#[derive(Default)]
struct Members {
    label: Option<SharedString>,
    requested: bool,
    shown: bool,
}

struct Detail {
    season: Entity<SeasonPage>,
    list_top: Pixels,
    origin: String,
}

#[derive(Default)]
struct Slide {
    serial: usize,
    direction: f32,
}

pub struct Shell {
    page: Page,
    view: AnyView,
    tools: Option<Tools>,
    stops: Option<Stops>,
    detail: Option<Detail>,
    slide: Slide,
    restore: Option<Pixels>,
    route: usize,
    drawer: bool,
    art: Entity<RetainAllImageCache>,
    members: Members,
    scroll: ScrollHandle,
    root_focus: FocusHandle,
    modals: Entity<Modals>,
    _subscriptions: [Subscription; 3],
    _notices: [Subscription; 7],
}

struct ShellHandle(WeakEntity<Shell>);

impl Global for ShellHandle {}

pub fn navigate(page: Page, window: &mut Window, cx: &mut App) {
    let shell = cx.global::<ShellHandle>().0.clone();
    shell
        .update(cx, |shell, cx| shell.open(page, window, cx))
        .ok();
}

pub fn open_season(key: &str, hm: bool, window: &mut Window, cx: &mut App) {
    let shell = cx.global::<ShellHandle>().0.clone();
    shell
        .update(cx, |shell, cx| shell.open_season(key, hm, window, cx))
        .ok();
}

pub fn close_season(window: &mut Window, cx: &mut App) {
    let shell = cx.global::<ShellHandle>().0.clone();
    shell
        .update(cx, |shell, cx| shell.close_season(window, cx))
        .ok();
}

fn sidebar_width(viewport: f32) -> f32 {
    (0.18 * viewport).clamp(210., 280.)
}

fn column_width(viewport: f32) -> f32 {
    if viewport < NAV_BREAKPOINT {
        viewport
    } else {
        viewport - sidebar_width(viewport)
    }
}

pub fn page_pad(viewport: f32) -> f32 {
    (14. + 0.0078 * viewport).clamp(20., 40.)
}

pub fn page_top(cx: &App) -> Pixels {
    let shell = cx.try_global::<ShellHandle>();
    let shell = shell.and_then(|shell| shell.0.upgrade());
    shell.map_or(px(0.), |shell| -shell.read(cx).scroll.offset().y)
}

pub fn season_open(cx: &App) -> bool {
    let shell = cx.try_global::<ShellHandle>();
    let shell = shell.and_then(|shell| shell.0.upgrade());
    shell.is_some_and(|shell| shell.read(cx).detail.is_some())
}

pub fn page(cx: &App) -> Option<Page> {
    let shell = cx.try_global::<ShellHandle>()?.0.upgrade()?;
    Some(shell.read(cx).page)
}

pub fn page_size(window: &Window) -> Size<Pixels> {
    let viewport = frame::area(window).size;
    let width = units(viewport.width);
    let pad = 2. * page_pad(width);
    size(
        px(column_width(width) - pad),
        viewport.height - px(TOPBAR + pad),
    )
}

fn grouped(count: u64) -> String {
    let digits = count.to_string();
    let mut text = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push('.');
        }
        text.push(digit);
    }
    text
}

fn micro(content: &str, color: Hsla) -> Div {
    div()
        .face(FONT_MONO_20, TEXT_MICRO)
        .text_color(color)
        .child(content.to_uppercase())
}

fn cursor(id: &'static str, window: &Window) -> AnyElement {
    let mark = div().ml(px(1.)).child("_");
    if !window.is_window_active() {
        return mark.into_any_element();
    }
    mark.with_animation(
        id,
        Animation::new(BLINK).repeat().with_max_fps(2.),
        |mark, progress| mark.opacity(if progress < 0.5 { 1. } else { 0. }),
    )
    .into_any_element()
}

impl Shell {
    pub fn new(place: Place, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let root_focus = cx.focus_handle();
        window.focus(&root_focus, cx);
        let handle = cx.weak_entity();
        cx.set_global(ShellHandle(handle));
        let (view, tools, stops) = Self::view(place.page, window, cx);
        let detail = place.season.and_then(|edition| {
            let season = SeasonPage::new(&edition.key, edition.hm, cx)?;
            Some(Detail {
                season: season.mount(window, cx),
                list_top: px(0.),
                origin: edition_folder(&edition.key, edition.hm),
            })
        });
        Self {
            page: place.page,
            view,
            tools,
            stops,
            detail,
            slide: Slide::default(),
            restore: None,
            route: 0,
            drawer: false,
            art: RetainAllImageCache::new(cx),
            members: Members::default(),
            scroll: ScrollHandle::new(),
            root_focus,
            modals: cx.new(|cx| Modals::new(window, cx)),
            _subscriptions: [
                cx.observe(&Launcher::handle(cx), |_, _, cx| cx.notify()),
                cx.observe_window_activation(window, |_, _, cx| cx.notify()),
                cx.on_focus_lost(window, |this, window, cx| {
                    window.focus(&this.root_focus, cx)
                }),
            ],
            _notices: notices::watch(&Launcher::handle(cx), cx),
        }
    }

    pub fn place(&self, cx: &App) -> Place {
        let detail = self.detail.as_ref();
        Place {
            page: self.page,
            season: detail.map(|detail| detail.season.read(cx).edition()),
        }
    }

    fn view(
        page: Page,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (AnyView, Option<Tools>, Option<Stops>) {
        match page {
            Page::Home => {
                let home = cx.new(|cx| HomePage::new(window, cx));
                let (view, cards) = (home.clone().into(), home.clone());
                let tools: Tools =
                    Box::new(move |window, cx| home.update(cx, |home, cx| home.tools(window, cx)));
                let stops: Stops = Box::new(move |id, cx| cards.read(cx).stop(id));
                (view, Some(tools), Some(stops))
            }
            Page::Download => {
                let download = cx.new(|cx| DownloadPage::new(window, cx));
                let (view, banners) = (download.clone().into(), download.clone());
                let tools: Tools = Box::new(move |window, cx| {
                    download.update(cx, |download, cx| download.tools(window, cx))
                });
                let stops: Stops = Box::new(move |id, cx| banners.read(cx).stop(id));
                (view, Some(tools), Some(stops))
            }
            Page::Liberator => {
                let liberator = cx.new(|cx| LiberatorPage::new(window, cx));
                (liberator.into(), None, None)
            }
            Page::Settings => {
                let settings = cx.new(|cx| SettingsPage::new(window, cx));
                (settings.into(), None, None)
            }
            Page::Updates => {
                let updates = cx.new(|cx| UpdatesPage::new(window, cx));
                let view = updates.clone().into();
                let tools: Tools = Box::new(move |window, cx| {
                    updates.update(cx, |updates, cx| updates.tools(window, cx))
                });
                (view, Some(tools), None)
            }
        }
    }

    fn open(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if page != self.page {
            self.page = page;
            (self.view, self.tools, self.stops) = Self::view(page, window, cx);
            self.detail = None;
            self.slide.direction = 0.;
            self.route += 1;
            cx.notify();
        } else {
            self.close_season(window, cx);
        }
    }

    fn slide(&mut self, direction: f32) {
        self.slide = Slide {
            serial: self.slide.serial + 1,
            direction,
        };
    }

    fn open_season(&mut self, key: &str, hm: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.drawer = false;
        if let Some(detail) = &self.detail
            && detail.season.read(cx).key() == key
        {
            detail
                .season
                .update(cx, |season, cx| season.set_edition(hm, cx));
            return cx.notify();
        }
        let Some(season) = SeasonPage::new(key, hm, cx) else {
            return;
        };
        let home = &Launcher::handle(cx).read(cx).home;
        let installed = home
            .editions()
            .is_some_and(|editions| editions.iter().any(|edition| edition.key == key));
        let target = if self.page != Page::Download && installed {
            Page::Home
        } else {
            Page::Download
        };
        let list_top = if target != self.page {
            self.page = target;
            (self.view, self.tools, self.stops) = Self::view(target, window, cx);
            px(0.)
        } else {
            let shown = -self.scroll.offset().y;
            self.detail.take().map_or(shown, |detail| detail.list_top)
        };
        let edition = season.edition();
        self.detail = Some(Detail {
            season: season.mount(window, cx),
            list_top,
            origin: edition_folder(&edition.key, edition.hm),
        });
        self.slide(1.);
        cx.notify();
    }

    fn close_season(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.take() else {
            return;
        };
        let stops = self.stops.as_ref();
        if let Some(stop) = stops.and_then(|stops| stops(&detail.origin, cx)) {
            window.focus(&stop, cx);
        }
        self.restore = Some(detail.list_top);
        self.slide(-1.);
        if self.page == Page::Home {
            Launcher::handle(cx).update(cx, |launcher, cx| launcher.refresh_home(cx));
        }
        cx.notify();
    }

    fn show_members(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.members.shown = shown;
        cx.notify();
        if !shown || mem::replace(&mut self.members.requested, true) {
            return;
        }
        let count = cx.background_spawn(async { backend::discord::member_count() });
        cx.spawn(async move |this, cx| {
            let Some(count) = count.await else { return };
            this.update(cx, |this, cx| {
                this.members.label = Some(format!("// {} MEMBERS", grouped(count)).into());
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn nav_link(
        &self,
        index: usize,
        target: Option<Page>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = ElementId::from(("nav", index));
        let hover = motion::hover(&id, SLOW, window, cx);
        let ring = match target {
            Some(Page::Liberator) => loading_ring(window, cx),
            _ => None,
        };
        let updates = match target {
            Some(Page::Updates) => Launcher::handle(cx).read(cx).update.pending().len(),
            _ => 0,
        };
        let hopping = updates > 0
            && self.page != Page::Updates
            && window.is_window_active()
            && !cx.reduce_motion();
        let lift = if hopping {
            window.request_animation_frame();
            hop(motion::clock("updates-hop", window, cx))
        } else {
            0.
        };
        let shift = if cx.reduce_motion() { 0. } else { 4. };
        let current = target == Some(self.page);
        let chosen = if current { 1. } else { 0. };
        let chosen = motion::transition(
            motion::sub(&id, "current"),
            chosen,
            SLOW,
            EASE_IN_OUT,
            window,
            cx,
        );
        let theme = Theme::get(cx);
        let lit = hover.amount;
        let idle = SURFACE_2.rgba(lit);
        let link = div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(12.))
            .py(px(0.55 * REM))
            .rounded(px(6.))
            .cursor_pointer()
            .face(FONT_BODY_MEDIUM, 0.9 * REM)
            .child(target.map_or("FAQ", Page::label))
            .when(updates > 0, |link| {
                link.child(
                    div()
                        .top(px(-lift))
                        .face(FONT_DISPLAY_BOLD, 0.8 * REM)
                        .line_height(px(0.8 * REM))
                        .text_color(TEXT.solid())
                        .child(updates.to_string()),
                )
            })
            .children(ring)
            .left(px(shift * lit * (1. - chosen)))
            .bg(blend(idle, theme.action_dim, chosen))
            .text_color(
                TEXT_MUTED
                    .mix(TEXT, 1. - (1. - lit) * (1. - chosen))
                    .solid(),
            );
        let link = link.when(current, |link| {
            link.border_l_2()
                .border_color(theme.action.solid())
                .shadow(vec![
                    shadow(0., 0., 18., -5., theme.action_glow_soft).inset(),
                ])
        });
        match target {
            Some(page) => link
                .id(id)
                .tab_index(0)
                .focus_visible(focus_ring_inside)
                .map(|element| hover.follow(element))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.drawer = false;
                    this.open(page, window, cx);
                })),
            None => external(id, link, FAQ_URL, &hover, window, cx),
        }
    }

    fn hamburger(&self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let open = if self.drawer { 1. } else { 0. };
        let cross = motion::transition("hamburger", open, SLOW, EASE_IN_OUT, window, cx);
        let bar = |top: f32, turn: f32| {
            let pose = Transformation::rotate(radians(turn * cross * std::f32::consts::FRAC_PI_4))
                .with_translation(point(px(0.), px(turn * 3. * cross)));
            icon(BAR, 24.)
                .absolute()
                .left(px(5.))
                .top(px(top))
                .text_color(TEXT.solid())
                .with_transformation(pose)
        };
        div()
            .id("hamburger")
            .occlude()
            .absolute()
            .left(px(12.))
            .top(px(12.))
            .w(px(34.))
            .h(px(28.))
            .rounded(px(6.))
            .tab_index(0)
            .focus_visible(focus_ring_inside)
            .on_click(cx.listener(|this, _, _, cx| {
                this.drawer = !this.drawer;
                cx.notify();
            }))
            .child(bar(-1., 1.))
            .child(bar(5., -1.))
    }

    fn sidebar(&self, narrow: bool, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let action = Theme::get(cx).action.solid();
        let width = sidebar_width(units(frame::area(window).size.width));
        let title = "Throwback Launcher";
        let brand = match &self.members.label {
            Some(label) if self.members.shown => label,
            _ => "// R6S COMMUNITY",
        };
        let head = div()
            .flex()
            .flex_col()
            .items_start()
            .border_b_1()
            .border_color(BORDER.solid())
            .px(px(20.))
            .pt(px(if narrow { 56. } else { 24. }))
            .pb(px(16.));
        let corner = frame::radius(window);
        let mut sidebar = div()
            .flex_none()
            .w(px(width))
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .when(!narrow, |bar| {
                bar.bg(SURFACE.solid())
                    .rounded_tl(corner)
                    .rounded_bl(corner)
                    .border_color(BORDER.solid())
            })
            .child(
                frame::grip("sidebar-head", head, window, cx)
                    .child(
                        micro(brand, action)
                            .id("brand")
                            .flex()
                            .mb(px(0.3 * REM))
                            .on_hover(cx.listener(|this, shown: &bool, _, cx| {
                                this.show_members(*shown, cx)
                            }))
                            .child(cursor("blink-brand", window)),
                    )
                    .child(
                        div()
                            .face(FONT_DISPLAY_BOLD, 1.2 * REM)
                            .line_height(px(1.2 * 1.2 * REM))
                            .child(StyledText::new(title).with_highlights([(
                                0..title.len() - " Launcher".len(),
                                HighlightStyle::color(action),
                            )])),
                    ),
            );
        let mut index = 0;
        for (label, items) in NAV {
            let color = if items.contains(&Some(self.page)) {
                action
            } else {
                TEXT_MUTED.solid()
            };
            let mut section = div()
                .px(px(12.))
                .pt(px(1.2 * REM))
                .pb(px(8.))
                .child(micro(label, color).px(px(8.)).mb(px(0.4 * REM)));
            for item in items {
                section = section.child(self.nav_link(index, *item, window, cx));
                index += 1;
            }
            sidebar = sidebar.child(section);
        }
        let sidebar = sidebar
            .child(div().flex_1())
            .children(activity(&self.art, width, window, cx));
        scroll("sidebar", window, cx).area(sidebar)
    }

    fn topbar(
        &self,
        narrow: bool,
        pad: (f32, f32),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let home = ElementId::from("crumb-home");
        let hover = motion::hover(&home, FAST, window, cx);
        let detail = self.detail.as_ref();
        let tools = match detail {
            Some(_) => None,
            None => self.tools.as_ref().and_then(|tools| tools(window, cx)),
        };
        let list = ElementId::from("crumb-list");
        let list_hover = motion::hover(&list, FAST, window, cx);
        let place = match detail {
            Some(detail) => div()
                .flex()
                .child(
                    div()
                        .id(list)
                        .occlude()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .text_color(TEXT_MUTED.mix(TEXT, list_hover.amount).solid())
                        .tab_index(0)
                        .focus_visible(focus_ring_inside)
                        .map(|element| list_hover.follow(element))
                        .on_click(cx.listener(|this, _, window, cx| this.close_season(window, cx)))
                        .child(self.page.label()),
                )
                .child(" / ")
                .child(
                    div()
                        .text_color(TEXT.solid())
                        .child(detail.season.read(cx).title()),
                ),
            None => div().text_color(TEXT.solid()).child(self.page.label()),
        };
        let controls = frame::controls(window, cx);
        let corner = frame::radius(window);
        let rule = div().absolute().left_0().bottom_0().w_full().h(px(1.));
        let rule = rule.bg(BORDER.solid());
        let bar = div()
            .flex_none()
            .h(px(TOPBAR))
            .flex()
            .items_center()
            .bg(SURFACE.solid())
            .rounded_tr(corner)
            .when(narrow, |bar| bar.rounded_tl(corner))
            .pl(px(pad.0))
            .pr(px(if controls.is_some() { 0. } else { pad.1 }))
            .face(FONT_MONO_4, TEXT_LABEL)
            .text_color(TEXT_MUTED.solid())
            .whitespace_nowrap();
        frame::grip("topbar", bar, window, cx)
            .child(
                div()
                    .id(home)
                    .occlude()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .text_color(TEXT_MUTED.mix(TEXT, hover.amount).solid())
                    .tab_index(0)
                    .focus_visible(focus_ring_inside)
                    .map(|element| hover.follow(element))
                    .on_click(cx.listener(|this, _, window, cx| this.open(Page::Home, window, cx)))
                    .child("Launcher"),
            )
            .child(" / ")
            .child(place)
            .child(cursor("blink-crumb", window))
            .child(div().flex_1())
            .children(tools.map(|tools| div().occlude().child(tools)))
            .children(controls.map(|controls| controls.ml(px(12.))))
            .child(rule)
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = units(frame::area(window).size.width);
        let pad = page_pad(viewport);
        let detail = self.detail.as_ref();
        let shown = match detail {
            Some(detail) => AnyView::from(detail.season.clone()),
            None => self.view.clone(),
        };
        let bleed = self.page == Page::Download && detail.is_none();
        let page = scroll(("page", shown.entity_id()), window, cx);
        if let Some(top) = self.restore.take() {
            page.handle().set_offset(point(px(0.), -top));
        }
        self.scroll = page.handle().clone();
        let narrow = viewport < NAV_BREAKPOINT;
        self.drawer &= narrow;
        let open = if self.drawer { 1. } else { 0. };
        let drawer = motion::transition("drawer", open, SLIDE, EASE_OUT_CUBIC, window, cx);
        let bar_pad = if narrow { (56., 16.) } else { (pad, pad) };
        let fade = motion::entrance(("route", self.route), ENTER, EASE, window, cx);
        let lift = motion::rise(("route-rise", self.route), window, cx);
        let slid = match self.slide.direction {
            0. => 1.,
            _ => motion::entrance(
                ("slide", self.slide.serial),
                SLIDE,
                EASE_OUT_CUBIC,
                window,
                cx,
            ),
        };
        let run = column_width(viewport) - if bleed { 0. } else { 2. * pad };
        let shift = self.slide.direction * run * (1. - slid);
        let corner = frame::radius(window);
        let shell = div()
            .id("shell")
            .size_full()
            .flex()
            .bg(BG.solid())
            .rounded(corner)
            .face(FONT_BODY, REM)
            .text_color(TEXT.solid())
            .track_focus(&self.root_focus)
            .on_action(cx.listener(|_, _: &FocusNext, window, cx| window.focus_next(cx)))
            .on_action(cx.listener(|_, _: &FocusPrevious, window, cx| window.focus_prev(cx)))
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                if mem::take(&mut this.drawer) {
                    cx.notify();
                } else if this.detail.is_some() {
                    this.close_season(window, cx);
                } else {
                    this.open(Page::Home, window, cx);
                }
            }))
            .map(|shell| page.wheel(page.keys(shell)))
            .when(!narrow, |shell| {
                shell.child(self.sidebar(narrow, window, cx))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(self.topbar(narrow, bar_pad, window, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .child(
                                page.area(
                                    div().flex_1().overflow_x_hidden().child(
                                        div()
                                            .left(px(shift))
                                            .top(px(lift))
                                            .when(!bleed, |content| content.p(px(pad)))
                                            .child(shown),
                                    ),
                                ),
                            )
                            .when(fade < 1., |page| {
                                let veil = div().absolute().inset_0().bg(BG.alpha(1. - fade));
                                let veil = veil.rounded_br(corner);
                                page.child(veil.when(narrow, |veil| veil.rounded_bl(corner)))
                            }),
                    ),
            )
            .when(narrow && drawer > 0., |shell| {
                let width = px(sidebar_width(viewport));
                let left = width * (drawer - 1.);
                let pane = |fill: Hsla| {
                    let pane = div().absolute().top_0().w(width).h_full().bg(fill);
                    pane.rounded_tl(corner).rounded_bl(corner)
                };
                let line = div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .w(gpui::px(1.))
                    .h_full()
                    .overflow_hidden()
                    .child(pane(BORDER.solid()).left(gpui::px(1.) - width - left));
                shell.child(
                    div()
                        .absolute()
                        .top_0()
                        .left(left)
                        .w(width)
                        .h_full()
                        .overflow_hidden()
                        .occlude()
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.drawer = false;
                            cx.notify();
                        }))
                        .child(pane(SURFACE.solid()).left(-left))
                        .child(self.sidebar(narrow, window, cx))
                        .child(line),
                )
            })
            .when(narrow, |shell| shell.child(self.hamburger(window, cx)))
            .child(self.modals.clone())
            .child(toast::stack(cx));
        let dim = modal::dim(window, cx);
        #[cfg(windows)]
        frame::border(dim, window, cx);
        frame::outline(dim, window)
            .child(shell)
            .children(frame::edges(window))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use backend::env::Paths;
    use backend::manifest;
    use backend::settings::{Settings, USERNAME_EMPTY};
    use gpui::{Modifiers, MouseButton, TestAppContext, VisualTestContext, point};
    use tempfile::TempDir;

    use super::*;
    use crate::model::Edition;
    use crate::model::download::{Downloader, LoginKind};

    const QUEUE: &str = "queue_paused = true\nqueue = [\
        { key = 'Y1S0_Vanilla', hm = false, library = '', verify = false }, \
        { key = 'Y2S1_VelvetShell', hm = false, library = '', verify = false }, \
        { key = 'Y3S1_Chimera', hm = false, library = '', verify = false }, \
        { key = 'Y4S2_PhantomSight', hm = false, library = '', verify = false }]";

    #[test]
    fn the_update_count_keeps_hopping() {
        let near = |elapsed, height: f32| (hop(elapsed) - height).abs() < 1e-3;
        assert!(near(Duration::ZERO, 0.) && near(HOP / 2, HOP_HEIGHT));
        assert!(near(HOP + HOP / 4, 0.75 * HOP_HEIGHT));
        assert!(near(7 * HOP + HOP / 2, HOP_HEIGHT));
    }

    struct Bench {
        _dir: TempDir,
        paths: Paths,
        launcher: Entity<Launcher>,
        shell: Entity<Shell>,
    }

    fn bench<'a>(
        cx: &'a mut TestAppContext,
        settings: &str,
        download: Option<Downloader>,
    ) -> (Bench, &'a mut VisualTestContext) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        std::fs::create_dir_all(&paths.data_root).unwrap();
        let content = format!("[settings]\ndiscord_rpc = false\n{settings}\n");
        std::fs::write(paths.settings_file(), content).unwrap();
        let launcher = cx.update(|cx| {
            crate::init(cx);
            let store = Settings::load(&paths).unwrap();
            let mut launcher = Launcher::new(paths.clone(), store, manifest::downloads());
            if let Some(download) = download {
                launcher.download = download;
            }
            launcher.init(cx);
            Launcher::handle(cx)
        });
        let (shell, cx) =
            cx.add_window_view(|window, cx| Shell::new(Page::Home.into(), window, cx));
        cx.simulate_resize(size(px(1280.), px(720.)));
        cx.update(|window, _| window.activate_window());
        draw(cx);
        let bench = Bench {
            _dir: dir,
            paths,
            launcher,
            shell,
        };
        (bench, cx)
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn waiting(bench: &Bench, cx: &mut VisualTestContext) -> Vec<String> {
        bench.launcher.read_with(cx, |launcher, _| {
            let queue = launcher.download.queue();
            queue.map(|edition| edition.key).collect()
        })
    }

    fn opened(bench: &Bench, cx: &mut VisualTestContext) -> Option<String> {
        bench.shell.read_with(cx, |shell, cx| {
            let detail = shell.detail.as_ref()?;
            Some(detail.season.read(cx).key().to_owned())
        })
    }

    #[gpui::test]
    fn queue_rows_open_their_season_and_move_by_keys_and_by_dragging(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, QUEUE, None);
        let (velvet, chimera, sight) = ("Y2S1_VelvetShell", "Y3S1_Chimera", "Y4S2_PhantomSight");
        assert_eq!(waiting(&bench, cx), [velvet, chimera, sight]);
        cx.simulate_click(point(px(100.), px(643.)), Modifiers::default());
        assert_eq!(opened(&bench, cx).as_deref(), Some(velvet));
        assert!(
            bench
                .shell
                .read_with(cx, |shell, _| shell.page == Page::Download)
        );

        cx.simulate_click(point(px(100.), px(643.)), Modifiers::default());
        cx.simulate_keystrokes("alt-up");
        assert_eq!(waiting(&bench, cx), [chimera, velvet, sight]);
        cx.simulate_keystrokes("alt-up alt-up");
        assert_eq!(waiting(&bench, cx), [chimera, sight, velvet]);
        cx.simulate_keystrokes("alt-down");
        assert_eq!(waiting(&bench, cx), [chimera, velvet, sight]);
        let stored = Settings::load(&bench.paths).unwrap().queue();
        let stored: Vec<&str> = stored.iter().map(|entry| entry.key.as_str()).collect();
        assert_eq!(stored, ["Y1S0_Vanilla", chimera, velvet, sight]);

        draw(cx);
        let (left, none) = (MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_down(point(px(100.), px(643.)), left, none);
        cx.simulate_mouse_move(point(px(104.), px(639.)), left, none);
        draw(cx);
        assert_eq!(waiting(&bench, cx), [chimera, velvet, sight]);
        cx.simulate_mouse_move(point(px(104.), px(595.)), left, none);
        draw(cx);
        assert_eq!(waiting(&bench, cx), [velvet, sight, chimera]);
        cx.simulate_mouse_move(point(px(104.), px(596.)), left, none);
        cx.simulate_mouse_up(point(px(104.), px(596.)), left, none);
        draw(cx);
        assert_eq!(waiting(&bench, cx), [velvet, sight, chimera]);
        assert_eq!(opened(&bench, cx).as_deref(), Some(velvet));
        assert!(!cx.update(|_, cx| cx.has_active_drag()));
    }

    #[gpui::test]
    fn the_download_card_pauses_cancels_continues_and_opens_its_season(cx: &mut TestAppContext) {
        let toggle = point(px(204.), px(688.));
        let paused = |bench: &Bench| Settings::load(&bench.paths).unwrap().queue_paused();
        for (verifying, pauses) in [(false, true), (true, false)] {
            let typed = Arc::new(Mutex::new(Vec::new()));
            let download = Downloader::loading(verifying, typed.clone());
            let (bench, cx) = bench(cx, "", Some(download));
            cx.simulate_click(toggle, Modifiers::default());
            assert_eq!(*typed.lock().unwrap(), ["killed"]);
            assert_eq!(paused(&bench), pauses);
            assert_eq!(opened(&bench, cx), None);
        }
        let (bench, cx) = bench(cx, QUEUE, None);
        cx.simulate_click(point(px(100.), px(688.)), Modifiers::default());
        assert_eq!(opened(&bench, cx).as_deref(), Some("Y1S0_Vanilla"));
        cx.simulate_click(toggle, Modifiers::default());
        let asked = |launcher: &Launcher, _: &App| launcher.download.login_kind();
        assert_eq!(
            bench.launcher.read_with(cx, asked),
            Some(LoginKind::Account)
        );
        let active = bench
            .launcher
            .read_with(cx, |launcher, _| launcher.download.active().cloned());
        let vanilla = Edition {
            key: "Y1S0_Vanilla".into(),
            hm: false,
        };
        assert_eq!(active, Some(vanilla));
    }

    #[gpui::test]
    fn closing_a_season_returns_the_focus_to_its_place_in_the_list(cx: &mut TestAppContext) {
        const KEY: &str = "Y1S0_Vanilla";
        let (bench, cx) = bench(cx, "", None);
        cx.update(|window, cx| {
            navigate(Page::Download, window, cx);
            open_season(KEY, false, window, cx);
        });
        draw(cx);
        assert!(bench.shell.read_with(cx, |shell, _| shell.detail.is_some()));
        cx.simulate_keystrokes("escape");
        draw(cx);
        let stop = bench.shell.read_with(cx, |shell, cx| {
            let stops = shell.stops.as_ref();
            stops.and_then(|stops| stops(KEY, cx))
        });
        assert!(stop.is_some());
        assert!(cx.update(|window, cx| window.focused(cx)) == stop);
        assert!(bench.shell.read_with(cx, |shell, _| shell.detail.is_none()));
    }

    #[gpui::test]
    fn typing_searches_the_download_list_only_while_it_is_shown(cx: &mut TestAppContext) {
        let (bench, cx) = bench(cx, "", None);
        cx.update(|window, cx| navigate(Page::Download, window, cx));
        draw(cx);
        let searched = |cx: &mut VisualTestContext| {
            bench.shell.read_with(cx, |shell, cx| {
                let list = shell.view.clone().downcast::<DownloadPage>();
                list.unwrap().read(cx).searched(cx)
            })
        };
        let open = |cx: &mut VisualTestContext| {
            bench.shell.read_with(cx, |shell, _| shell.detail.is_some())
        };
        cx.simulate_keystrokes("v");
        assert_eq!(searched(cx), "v");

        cx.update(|window, cx| open_season("Y1S0_Vanilla", false, window, cx));
        draw(cx);
        cx.simulate_keystrokes("x");
        assert_eq!(searched(cx), "v");
        cx.simulate_keystrokes("escape");
        draw(cx);
        assert!(!open(cx));
        assert_eq!(searched(cx), "v");

        cx.simulate_keystrokes("escape");
        assert_eq!(searched(cx), "");
        assert!(
            bench
                .shell
                .read_with(cx, |shell, _| shell.page == Page::Download)
        );
    }

    #[gpui::test]
    fn escape_opens_home(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        std::fs::create_dir_all(&paths.data_root).unwrap();
        std::fs::write(paths.settings_file(), "[settings]\ndiscord_rpc = false\n").unwrap();
        cx.update(|cx| {
            crate::init(cx);
            let store = Settings::load(&paths).unwrap();
            Launcher::new(paths.clone(), store, Vec::new()).init(cx);
        });
        let (shell, cx) =
            cx.add_window_view(|window, cx| Shell::new(Page::Settings.into(), window, cx));
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        cx.simulate_keystrokes("escape");
        assert!(shell.read_with(cx, |shell, _| shell.page == Page::Home));
    }

    #[gpui::test]
    fn rejected_settings_input_appears_as_a_toast_above_the_page(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        std::fs::create_dir_all(&paths.data_root).unwrap();
        std::fs::write(
            paths.settings_file(),
            "[settings]\nreduce_motion = true\ndiscord_rpc = false\n",
        )
        .unwrap();
        let store = Settings::load(&paths).unwrap();
        cx.update(|cx| {
            crate::init(cx);
            Launcher::new(paths, store, Vec::new()).init(cx);
        });
        let (_, cx) =
            cx.add_window_view(|window, cx| Shell::new(Page::Settings.into(), window, cx));
        cx.update(|_, cx| {
            Launcher::handle(cx).update(cx, |launcher, cx| launcher.settings.set_username("", cx));
        });
        let corner = cx.update(|window, _| window.viewport_size());
        cx.simulate_click(
            point(corner.width - px(25.), corner.height - px(25.)),
            Modifiers::default(),
        );
        let copied = cx.read_from_clipboard().and_then(|item| item.text());
        assert_eq!(copied.as_deref(), Some(USERNAME_EMPTY));
    }
}
