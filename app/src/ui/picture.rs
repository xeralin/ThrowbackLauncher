use std::io::Cursor;
use std::panic::Location;
use std::sync::Arc;

use backend::github::{fetch_picture, github_media};
use gpui::{
    App, AppContext, Asset, AvailableSpace, Bounds, Corners, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, RenderImage, SharedString, Size, Style,
    Window, size,
};
use image::{DynamicImage, Frame, ImageDecoder, ImageReader};

const AREA: u64 = 16_000_000;

fn decode(bytes: &[u8], area: u64) -> Option<RenderImage> {
    let reader = ImageReader::new(Cursor::new(bytes));
    let mut decoder = reader.with_guessed_format().ok()?.into_decoder().ok()?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > area {
        return None;
    }
    let orientation = decoder.orientation().ok()?;
    let mut picture = DynamicImage::from_decoder(decoder).ok()?;
    picture.apply_orientation(orientation);
    let mut pixels = picture.into_rgba8();
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(RenderImage::new(vec![Frame::new(pixels)]))
}

fn fetch(url: &str) -> Option<Vec<u8>> {
    #[cfg(feature = "qa")]
    if let Some(folder) = std::env::var_os("TB_QA_PICTURES") {
        let name = url.rsplit('/').next()?;
        return std::fs::read(std::path::Path::new(&folder).join(name)).ok();
    }
    fetch_picture(url).ok()
}

struct Still;

impl Asset for Still {
    type Source = SharedString;
    type Output = Option<Arc<RenderImage>>;

    fn load(
        url: SharedString,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        cx.background_spawn(async move { decode(&fetch(&url)?, AREA).map(Arc::new) })
    }
}

pub fn loaded(url: &SharedString, window: &mut Window, cx: &mut App) -> Option<Arc<RenderImage>> {
    if !github_media(url) {
        return None;
    }
    window.use_asset::<Still>(url, cx).flatten()
}

struct View {
    picture: Arc<RenderImage>,
    limit: Pixels,
}

pub fn view(picture: Arc<RenderImage>, limit: Pixels) -> impl IntoElement {
    View { picture, limit }
}

impl View {
    fn ratio(&self) -> f32 {
        let natural = self.picture.size(0);
        natural.height.0 as f32 / natural.width.0 as f32
    }
}

impl IntoElement for View {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for View {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, ()) {
        let (limit, ratio) = (self.limit, self.ratio());
        let measure = move |known: Size<Option<Pixels>>, room: Size<AvailableSpace>| {
            let width = match (known.width, room.width) {
                (Some(width), _) | (None, AvailableSpace::Definite(width)) => width.min(limit),
                (None, AvailableSpace::MinContent) => Pixels::ZERO,
                (None, AvailableSpace::MaxContent) => limit,
            };
            size(width, width * ratio)
        };
        let style = Style::default();
        let layout =
            window.request_measured_layout(style, move |known, room, _, _| measure(known, room));
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) {
        let width = bounds.size.width.min(self.limit);
        let shown = Bounds::new(bounds.origin, size(width, width * self.ratio()));
        let (corners, picture) = (Corners::default(), self.picture.clone());
        let drawn = window.paint_image(shown, shown, corners, picture, 0, false);
        drawn.ok();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{
        DevicePixels, InteractiveElement, Modifiers, ParentElement, StatefulInteractiveElement,
        Styled, TestAppContext, div, point,
    };
    use image::{ImageFormat, RgbaImage};

    use super::*;
    use crate::ui::px;
    use crate::ui::testing::mount;

    #[gpui::test]
    fn a_picture_keeps_its_shape_when_the_room_is_narrower_or_wider(cx: &mut TestAppContext) {
        let hits = Rc::new(Cell::new(0));
        let count = hits.clone();
        let cx = mount(cx, move |_, _| {
            let frame = |id: &'static str, room: f32| {
                let picture =
                    Arc::new(RenderImage::new(vec![Frame::new(RgbaImage::new(400, 100))]));
                let count = count.clone();
                div().w(px(room)).flex().child(
                    div()
                        .id(id)
                        .min_w_0()
                        .on_click(move |_, _, _| count.set(count.get() + 1))
                        .child(view(picture, px(400.))),
                )
            };
            div()
                .flex()
                .flex_col()
                .items_start()
                .child(frame("narrow", 200.))
                .child(frame("wide", 600.))
                .into_any_element()
        });
        let mut hit = |x: f32, y: f32| {
            let before = hits.get();
            cx.simulate_click(point(px(x), px(y)), Modifiers::default());
            hits.get() > before
        };
        assert!(hit(190., 45.) && !hit(210., 45.));
        assert!(hit(100., 55.) && hit(390., 145.) && !hit(410., 145.) && !hit(100., 155.));
    }

    #[test]
    fn a_picture_is_decoded_to_one_frame_unless_it_is_too_large_or_no_picture() {
        let pixels = vec![10, 20, 30, 255, 40, 50, 60, 255];
        let picture = RgbaImage::from_raw(2, 1, pixels).expect("two pixels");
        let mut file = Cursor::new(Vec::new());
        picture.write_to(&mut file, ImageFormat::Png).expect("png");
        let file = file.into_inner();
        let shown = decode(&file, AREA).expect("decoded");
        assert_eq!(shown.frame_count(), 1);
        assert_eq!(shown.size(0), size(DevicePixels(2), DevicePixels(1)));
        assert_eq!(
            shown.as_bytes(0),
            Some(&[30, 20, 10, 255, 60, 50, 40, 255][..])
        );
        assert!(decode(&file, 1).is_none());
        assert!(decode(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>", AREA).is_none());
    }
}
