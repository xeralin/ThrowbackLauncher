use std::borrow::Cow;

use gpui::{App, AssetSource, Result, SharedString};

include!(concat!(env!("OUT_DIR"), "/pictures.rs"));

pub const HEATED_METAL_ART: &str = "hm";

const FONTS: [&[u8]; 10] = [
    include_bytes!("../assets/fonts/Barlow-Regular.ttf"),
    include_bytes!("../assets/fonts/Barlow-Italic.ttf"),
    include_bytes!("../assets/fonts/Barlow-Medium.ttf"),
    include_bytes!("../assets/fonts/Barlow-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Rajdhani-Bold.ttf"),
    include_bytes!("../assets/fonts/ShareTechMono-Regular.ttf"),
    include_bytes!("../assets/fonts/ShareTechMono-4.ttf"),
    include_bytes!("../assets/fonts/ShareTechMono-8.ttf"),
    include_bytes!("../assets/fonts/ShareTechMono-12.ttf"),
    include_bytes!("../assets/fonts/ShareTechMono-20.ttf"),
];

pub fn load_fonts(cx: &App) {
    cx.text_system()
        .add_fonts(FONTS.iter().map(|font| Cow::Borrowed(*font)).collect())
        .expect("bundled fonts");
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let asset = PICTURES.iter().find(|(name, _)| *name == path);
        Ok(asset.map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let names = PICTURES.iter().map(|(name, _)| *name);
        Ok(names
            .filter(|name| name.starts_with(path))
            .map(SharedString::from)
            .collect())
    }
}

pub fn key_art(name: &str, device_width: f32) -> SharedString {
    let [.., widest] = KEY_ART_WIDTHS;
    let covering = KEY_ART_WIDTHS
        .into_iter()
        .find(|width| *width as f32 >= device_width);
    format!("keyart/{}/{name}.webp", covering.unwrap_or(widest)).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_season_and_heated_metal_have_key_art_in_each_width() {
        let seasons = backend::manifest::downloads();
        let names = seasons.iter().map(|season| season.key.as_str());
        for name in names.chain([HEATED_METAL_ART]) {
            for width in KEY_ART_WIDTHS {
                let path = format!("keyart/{width}/{name}.webp");
                assert!(Assets.load(&path).unwrap().is_some(), "{path}");
            }
        }
        assert_eq!(Assets.list("keyart/640/").unwrap().len(), seasons.len() + 1);
    }

    #[test]
    fn key_art_takes_the_smallest_variant_that_covers_the_width() {
        for (width, variant) in [
            (1., 640),
            (640., 640),
            (640.5, 1280),
            (1280., 1280),
            (1900., 1920),
            (1921., 2560),
            (5000., 2560),
        ] {
            assert_eq!(
                key_art("Y1S0_Vanilla", width),
                format!("keyart/{variant}/Y1S0_Vanilla.webp")
            );
        }
    }
}
