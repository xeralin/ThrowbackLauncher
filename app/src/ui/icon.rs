use crate::ui::px;
use gpui::{Styled, Svg, svg};

macro_rules! glyph {
    ($fill:literal, $path:literal) => {
        glyph!($fill, $path, "2")
    };
    ($fill:literal, $path:literal, $stroke:literal) => {
        concat!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill=""##,
            $fill,
            r##"" stroke="#000" stroke-width=""##,
            $stroke,
            r##"" stroke-linecap="round" stroke-linejoin="round"><path d=""##,
            $path,
            r##""/></svg>"##
        )
        .as_bytes()
    };
}

pub const INFO: &[u8] = glyph!(
    "none",
    "M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0zM12 16v-4M12 8h.01"
);
pub const BAR: &[u8] = glyph!("none", "M4 12h16");
pub const CANCEL: &[u8] = glyph!("none", "M7.5 7.5 16.5 16.5M16.5 7.5 7.5 16.5");
pub const CROSS: &[u8] = glyph!("none", "M18 6 6 18M6 6l12 12");
pub const WINDOW_CLOSE: &[u8] = glyph!("none", "M5.5 5.5l13 13M18.5 5.5l-13 13", "1.5");
pub const PLAY: &[u8] = glyph!("none", "M8.5 5.7 18.5 12l-10 6.3Z");
pub const PAUSE: &[u8] = glyph!("none", "M9.5 6v12M14.5 6v12");
pub const EYE: &[u8] = glyph!(
    "none",
    "M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7ZM15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z"
);
pub const EYE_OFF: &[u8] = glyph!(
    "none",
    "M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7ZM15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0ZM4 20 20 4"
);
pub const COPY: &[u8] = glyph!(
    "none",
    "M11 9h9a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-9a2 2 0 0 1-2-2v-9a2 2 0 0 1 2-2ZM5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"
);
pub const PROTON: &[u8] = glyph!("none", "m4 17 6-6-6-6M12 19h8");
pub const CHEVRON_LEFT: &[u8] = glyph!("none", "m15 6-6 6 6 6");
pub const CHEVRON_RIGHT: &[u8] = glyph!("none", "m9 6 6 6-6 6");
pub const CHEVRON_DOWN: &[u8] = glyph!("none", "m6 9 6 6 6-6");
pub const PACKAGE: &[u8] = glyph!(
    "none",
    "M11 21.73a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73zM12 22V12M3.3 7l7.703 4.734a2 2 0 0 0 1.994 0L20.7 7"
);
pub const REFRESH: &[u8] = glyph!(
    "none",
    "M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8M21 3v5h-5M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16M8 16H3v5"
);
pub const GITHUB: &[u8] = glyph!(
    "#000",
    "M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12",
    "0"
);
pub const FOLDER: &[u8] = glyph!(
    "none",
    "M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"
);
pub const BOOKMARK: &[u8] = glyph!(
    "none",
    "M19 20l-7-4.6L5 20V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"
);
pub const BOOKMARK_FILLED: &[u8] = glyph!(
    "#000",
    "M19 20l-7-4.6L5 20V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"
);
pub const REMOVE: &[u8] = glyph!(
    "none",
    "M5 3h14l-1.4 15.2a2 2 0 0 1-2 1.8H8.4a2 2 0 0 1-2-1.8Z"
);
pub const CHECK_BOLD: &[u8] = glyph!("none", "M5 13l4 4L19 7", "2.5");
pub const ERASER: &[u8] = glyph!(
    "none",
    "m7 21-4.3-4.3c-1-1-1-2.5 0-3.4l9.6-9.6c1-1 2.5-1 3.4 0l5.6 5.6c1 1 1 2.5 0 3.4L13 21M22 21H7M5 11l9 9"
);
pub const REVERSE: &[u8] = glyph!("none", "M7 20V4m0 0L4 7m3-3 3 3M17 4v16m0 0 3-3m-3 3-3-3");
pub const PENCIL: &[u8] = glyph!(
    "none",
    "M13 21h8M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z"
);
pub const PENCIL_FILLED: &[u8] = glyph!(
    "#000",
    "M13 21h8M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z"
);
pub const GRIP: &[u8] = glyph!(
    "none",
    "M3.75 20.25H9A11.25 11.25 0 0 0 20.25 9V3.75",
    "7.5"
);
pub const CLOCK: &[u8] = glyph!(
    "none",
    "M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0zM12 6v6l4 2"
);
pub const RING: &[u8] = glyph!("none", "M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0z", "3");
pub const RING_ARC: &[u8] = glyph!("none", "M21 12a9 9 0 0 0-9-9", "3");
pub const UNLOCK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="11" width="13" height="10" rx="1.5" fill="#000"/><path d="M12.5 10V6a3.75 3.75 0 0 1 7.5 0v2" stroke-width="2.5"/></svg>"##;
pub const WARNING: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21.73 18l-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3z"/><path d="M21.73 18l-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3zM11 9a1 1 0 0 1 2 0v4a1 1 0 0 1-2 0zM13 17a1 1 0 0 1-2 0 1 1 0 0 1 2 0z" fill="#000" fill-rule="evenodd" stroke="none"/></svg>"##;

pub fn icon(data: &'static [u8], size: f32) -> Svg {
    svg().data(data).flex_none().size(px(size))
}
