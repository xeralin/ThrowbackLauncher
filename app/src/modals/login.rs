use crate::ui::px;
use backend::depot::QrCode;
use gpui::{
    AnyElement, App, AppContext, Bounds, Context, Div, Entity, FocusHandle, IntoElement,
    ParentElement, Pixels, Styled, Subscription, Window, black, canvas, div, fill, point, size,
    white,
};

use super::Modals;
use crate::model::download::LoginKind;
use crate::ui::input::{TextInput, password_field, text_field};
use crate::ui::kit::panel;
use crate::ui::prose::{BODY, inline};

pub const FAQ_URL: &str = "https://xeralin.github.io/ThrowbackFAQ/getting-started/#steam-login";

const MODULE: f32 = 6.;
const QUIET_ZONE: f32 = 2.;

pub fn qr_panel(code: QrCode) -> Div {
    panel()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .child(qr_picture(code))
}

fn qr_picture(code: QrCode) -> impl IntoElement {
    let side = px(MODULE * (code.size() as f32 + 2. * QUIET_ZONE));
    let paint = move |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
        window.paint_quad(fill(bounds, white()).corner_radii(px(6.)));
        let cell = size(px(MODULE), px(MODULE));
        let at = |module: usize| px(MODULE * (module as f32 + QUIET_ZONE));
        for y in 0..code.size() {
            for x in (0..code.size()).filter(|&x| code.dark(x, y)) {
                let offset = point(at(x), at(y));
                window.paint_quad(fill(Bounds::new(bounds.origin + offset, cell), black()));
            }
        }
    };
    canvas(|_, _, _| (), paint).size(side)
}

#[derive(Debug, PartialEq)]
pub enum Submission {
    Account { account: String, password: String },
    Text(String),
}

enum Fields {
    Account {
        account: Entity<TextInput>,
        password: Entity<TextInput>,
    },
    Password(Entity<TextInput>),
    Code(Entity<TextInput>),
    Mobile,
}

pub struct Form {
    pub kind: LoginKind,
    fields: Fields,
    _edits: Vec<Subscription>,
}

impl Form {
    pub fn new(kind: LoginKind, window: &mut Window, cx: &mut Context<Modals>) -> Self {
        let mut edits = Vec::new();
        let mut field = |placeholder: &'static str, masked: bool| {
            let input = cx.new(|cx| {
                let input = TextInput::new("", window, cx).placeholder(placeholder);
                if masked { input.masked() } else { input }
            });
            edits.push(cx.observe(&input, |_, _, cx| cx.notify()));
            input
        };
        let fields = match kind {
            LoginKind::Account => Fields::Account {
                account: field("Account name", false),
                password: field("Password", true),
            },
            LoginKind::Password => Fields::Password(field("Password", true)),
            LoginKind::Guard => Fields::Code(field("Code", false)),
            LoginKind::GuardEmail => Fields::Code(field("Email code", false)),
            LoginKind::GuardMobile => Fields::Mobile,
        };
        Self {
            kind,
            fields,
            _edits: edits,
        }
    }

    pub fn title(&self) -> &'static str {
        match self.kind {
            LoginKind::Account | LoginKind::Password => "Steam login",
            LoginKind::Guard | LoginKind::GuardEmail | LoginKind::GuardMobile => "Steam Guard",
        }
    }

    pub fn first(&self, cx: &App) -> Option<FocusHandle> {
        let input = match &self.fields {
            Fields::Account { account, .. } => account,
            Fields::Password(input) | Fields::Code(input) => input,
            Fields::Mobile => return None,
        };
        Some(input.read(cx).focus_handle().clone())
    }

    pub fn asks(&self) -> bool {
        !matches!(self.fields, Fields::Mobile)
    }

    pub fn submission(&self, cx: &App) -> Option<Submission> {
        let typed = |input: &Entity<TextInput>| input.read(cx).text().to_owned();
        match &self.fields {
            Fields::Account { account, password } => {
                let (account, password) = (typed(account).trim().to_owned(), typed(password));
                let filled = !account.is_empty() && !password.is_empty();
                filled.then_some(Submission::Account { account, password })
            }
            Fields::Password(input) | Fields::Code(input) => Some(typed(input))
                .filter(|text| !text.is_empty())
                .map(Submission::Text),
            Fields::Mobile => None,
        }
    }

    pub fn body(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let stacked = |upper: Div, lower: Div| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(upper)
                .child(lower)
        };
        match &self.fields {
            Fields::Account { account, password } => {
                let account = text_field(account, window, cx);
                stacked(account, password_field(password, window, cx)).into_any_element()
            }
            Fields::Password(input) => password_field(input, window, cx).into_any_element(),
            Fields::Code(input) => text_field(input, window, cx).into_any_element(),
            Fields::Mobile => {
                let ask = "Confirm the login with the Steam Mobile App.";
                inline("login-mobile", ask, BODY, window, cx).into_any_element()
            }
        }
    }
}
