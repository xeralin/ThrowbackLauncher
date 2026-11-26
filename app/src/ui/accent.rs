const LIGHTNESS_BASE: f64 = 0.4;
const CHROMA_FLOOR: f64 = 0.25;
const LEVEL_STEPS: u32 = 100;
const DEFAULT_HUE: u32 = 23;
pub const HUE_MAX: u32 = 359;
pub const TRACK_STOPS: usize = 13;

fn gamma(value: f64) -> f64 {
    if value <= 0.0031308 {
        12.92 * value
    } else {
        1.055 * value.powf(1. / 2.4) - 0.055
    }
}

fn linear(channel: u32) -> f64 {
    let value = f64::from(channel) / 255.;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn to_rgb(lightness: f64, hue: f64, chroma: f64) -> [f64; 3] {
    let a = chroma * hue.to_radians().cos();
    let b = chroma * hue.to_radians().sin();
    let l = (lightness + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m = (lightness - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s = (lightness - 0.0894841775 * a - 1.291485548 * b).powi(3);
    [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
    ]
    .map(gamma)
}

fn max_chroma(lightness: f64, hue: f64) -> f64 {
    let (mut low, mut high) = (0., 0.4);
    for _ in 0..24 {
        let mid = (low + high) / 2.;
        if to_rgb(lightness, hue, mid)
            .iter()
            .all(|value| (-0.0005..=1.0005).contains(value))
        {
            low = mid;
        } else {
            high = mid;
        }
    }
    low
}

fn vivid_lightness(hue: u32) -> f64 {
    let hue = f64::from(hue);
    let (mut low, mut high) = (0.517, 0.99);
    for _ in 0..20 {
        let third = (high - low) / 3.;
        if max_chroma(low + third, hue) < max_chroma(high - third, hue) {
            low += third;
        } else {
            high -= third;
        }
    }
    (low + high) / 2.
}

fn lightness_for(vivid: f64, level: f64) -> f64 {
    LIGHTNESS_BASE + (vivid - LIGHTNESS_BASE) * level
}

fn chroma_for(lightness: f64, hue: u32, level: f64) -> f64 {
    (CHROMA_FLOOR + (1. - CHROMA_FLOOR) * level) * max_chroma(lightness, f64::from(hue))
}

pub fn accent(hue: u32, level: f64) -> u32 {
    let lightness = lightness_for(vivid_lightness(hue), level);
    to_rgb(lightness, f64::from(hue), chroma_for(lightness, hue, level))
        .iter()
        .fold(0, |packed, value| {
            (packed << 8) | (value.clamp(0., 1.) * 255.).round() as u32
        })
}

pub fn parts(color: u32) -> (u32, f64) {
    let r = linear((color >> 16) & 255);
    let g = linear((color >> 8) & 255);
    let b = linear(color & 255);
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let ax = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
    let bx = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
    let angle = bx.atan2(ax).to_degrees();
    let hue = if ax * ax + bx * bx < 1e-9 {
        DEFAULT_HUE
    } else {
        (if angle < 0. { angle + 360. } else { angle }).round() as u32 % 360
    };
    let lightness = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
    let radians = f64::from(hue).to_radians();
    let vivid = vivid_lightness(hue);
    let mut best = 0.;
    let mut closest = f64::INFINITY;
    for step in 0..=LEVEL_STEPS {
        let level = f64::from(step) / f64::from(LEVEL_STEPS);
        let light = lightness_for(vivid, level);
        let chroma = chroma_for(light, hue, level);
        let distance = (light - lightness).powi(2)
            + (chroma * radians.cos() - ax).powi(2)
            + (chroma * radians.sin() - bx).powi(2);
        if distance < closest {
            closest = distance;
            best = level;
        }
    }
    (hue, best)
}

pub fn hue_track() -> [u32; TRACK_STOPS] {
    std::array::from_fn(|index| accent(360 - 30 * index as u32, 1.))
}

pub fn level_track(hue: u32) -> [u32; 5] {
    std::array::from_fn(|index| accent(hue, index as f64 / 4.))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_reference_values() {
        let color = |text: &str| u32::from_str_radix(text, 16).expect("color");
        let mut checked = 0;
        for line in include_str!("accent_reference.txt").lines() {
            let fields: Vec<&str> = line.split(' ').collect();
            match fields[0] {
                "hex" => {
                    let hue = fields[1].parse().expect("hue");
                    let level = fields[2].parse().expect("level");
                    assert_eq!(accent(hue, level), color(fields[3]), "{line}");
                }
                "parts" => {
                    let (hue, level) = parts(color(fields[1]));
                    assert_eq!(hue, fields[2].parse::<u32>().expect("hue"), "{line}");
                    assert_eq!(level, fields[3].parse::<f64>().expect("level"), "{line}");
                }
                _ => {
                    let track: Vec<u32> = fields[1..].iter().map(|text| color(text)).collect();
                    assert_eq!(hue_track().to_vec(), track);
                }
            }
            checked += 1;
        }
        assert_eq!(checked, 377);
    }
}
