use std::rc::Rc;

use crate::ui::{Bound, px};
use backend::settings::{color_text, parse_color};
use gpui::{App, AppContext, Div, Entity, SharedString, Styled, Window};

use crate::ui::input::{FIELD_PAD, Filter, Hooks, TextInput, text_field};
use crate::ui::theme::CH;

pub type TextCommit = Rc<dyn Fn(&str, &mut Window, &mut App)>;

const HEX_DIGITS: usize = 6;

pub struct TextSetting {
    input: Entity<TextInput>,
    seen: SharedString,
}

impl TextSetting {
    pub fn new(
        value: &SharedString,
        placeholder: Option<&'static str>,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let input = cx.new(|cx| {
            let input = TextInput::new(value, window, cx);
            match placeholder {
                Some(text) => input.placeholder(text),
                None => input,
            }
        });
        Self {
            input,
            seen: value.clone(),
        }
    }

    pub fn reset(&self, value: &str, cx: &mut App) {
        self.input.update(cx, |input, cx| input.set_text(value, cx));
    }

    pub fn build(
        &mut self,
        value: &SharedString,
        max_length: Option<usize>,
        filter: Option<Filter>,
        on_commit: TextCommit,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let adopt = self.seen != *value;
        self.seen = value.clone();
        let original = value.clone();
        self.input.update(cx, |input, cx| {
            if adopt {
                input.set_text(&original, cx);
            }
            input.hooks = Hooks {
                max_length,
                filter,
                commit: Some(Rc::new(move |draft, window, cx| {
                    on_commit(draft, window, cx);
                    draft.to_owned()
                })),
                revert: Some(Rc::new(move |input, cx| input.set_text(&original, cx))),
            };
        });
        text_field(&self.input, window, cx)
    }
}

pub struct HexSetting {
    input: Entity<TextInput>,
    seen: u32,
}

fn clean_hex(raw: &str) -> String {
    std::iter::once('#')
        .chain(
            raw.chars()
                .filter(char::is_ascii_hexdigit)
                .map(|digit| digit.to_ascii_lowercase())
                .take(HEX_DIGITS),
        )
        .collect()
}

impl HexSetting {
    pub fn new(value: u32, window: &mut Window, cx: &mut App) -> Self {
        Self {
            input: cx.new(|cx| TextInput::new(&color_text(value), window, cx)),
            seen: value,
        }
    }

    pub fn build(
        &mut self,
        value: u32,
        on_commit: Bound<u32>,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let adopt = self.seen != value;
        self.seen = value;
        self.input.update(cx, |input, cx| {
            if adopt {
                input.set_text(&color_text(value), cx);
            }
            input.hooks = Hooks {
                max_length: None,
                filter: Some(Rc::new(clean_hex)),
                commit: Some(Rc::new(move |draft, window, cx| {
                    let Some(color) = parse_color(draft) else {
                        return color_text(value);
                    };
                    if color != value {
                        on_commit(color, window, cx);
                    }
                    draft.to_owned()
                })),
                revert: Some(Rc::new(move |input, cx| {
                    input.set_text(&color_text(value), cx)
                })),
            };
        });
        text_field(&self.input, window, cx)
            .flex_none()
            .w(px(7. * CH + 2. * FIELD_PAD + 2.))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui::{IntoElement, TestAppContext};

    use super::*;
    use crate::ui::testing::mount;

    #[test]
    fn hex_drafts_keep_six_lowercase_digits() {
        assert_eq!(clean_hex("#C388E3"), "#c388e3");
        assert_eq!(clean_hex("c3 88-e3ff"), "#c388e3");
        assert_eq!(clean_hex("xyz"), "#");
        assert_eq!(clean_hex(""), "#");
    }

    #[gpui::test]
    fn hex_commits_valid_colors_and_restores_broken_ones(cx: &mut TestAppContext) {
        let commits = Rc::new(RefCell::new(Vec::new()));
        let setting = Rc::new(RefCell::new(None::<HexSetting>));
        let (log, slot) = (commits.clone(), setting.clone());
        let cx = mount(cx, move |window, cx| {
            let mut slot = slot.borrow_mut();
            let setting = slot.get_or_insert_with(|| HexSetting::new(0xc0152a, window, cx));
            let log = log.clone();
            setting
                .build(
                    0xc0152a,
                    Rc::new(move |color, _, _| log.borrow_mut().push(color)),
                    window,
                    cx,
                )
                .into_any_element()
        });
        let draft = |cx: &mut gpui::VisualTestContext| {
            let input = setting.borrow().as_ref().expect("setting").input.clone();
            input.read_with(cx, |input, _| input.text().to_owned())
        };
        cx.simulate_keystrokes("tab");
        cx.simulate_input("FF00zz88");
        assert_eq!(draft(cx), "#ff0088");
        cx.simulate_keystrokes("enter");
        assert_eq!(*commits.borrow(), [0xff0088]);
        cx.simulate_keystrokes("tab");
        cx.simulate_input("12");
        assert_eq!(draft(cx), "#12");
        cx.simulate_keystrokes("enter");
        assert_eq!(draft(cx), "#c0152a");
        assert_eq!(*commits.borrow(), [0xff0088]);
    }

    #[gpui::test]
    fn a_commit_on_blur_reaches_observers_of_the_shown_value(cx: &mut TestAppContext) {
        let color = cx.new(|_| 0xc0152a_u32);
        let observed = Rc::new(RefCell::new(Vec::new()));
        let log = observed.clone();
        cx.update(|cx| {
            cx.observe(&color, move |color, cx| {
                log.borrow_mut().push(*color.read(cx))
            })
            .detach();
        });
        let setting = Rc::new(RefCell::new(None::<HexSetting>));
        let cx = mount(cx, move |window, cx| {
            let value = *color.read(cx);
            let store = color.clone();
            setting
                .borrow_mut()
                .get_or_insert_with(|| HexSetting::new(value, window, cx))
                .build(
                    value,
                    Rc::new(move |next, _, cx| {
                        store.update(cx, |color, cx| {
                            *color = next;
                            cx.notify();
                        })
                    }),
                    window,
                    cx,
                )
                .into_any_element()
        });
        cx.simulate_keystrokes("tab");
        cx.simulate_input("00bdcf");
        cx.simulate_keystrokes("enter");
        assert_eq!(*observed.borrow(), [0x00bdcf]);
    }
}
