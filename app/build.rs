use std::env;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;

use image::DynamicImage;
use image::imageops::FilterType;

const WIDTHS: [u32; 4] = [640, 1280, 1920, 2560];
const QUALITY: f32 = 90.;
const NAME: &str = "Throwback Launcher";
const ICON: &str = "../.github/icon.ico";
const TRAY_SIZES: [u32; 2] = [22, 64];

fn main() {
    let folder = env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let assets = PathBuf::from(folder).join("assets");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let files = pictures(&assets.join("keyart"));

    let workers = thread::available_parallelism().map_or(1, usize::from);
    thread::scope(|scope| {
        for batch in files.chunks(files.len().div_ceil(workers).max(1)) {
            scope.spawn(|| batch.iter().for_each(|file| variants(file, &out)));
        }
    });

    let mut table = format!("pub const KEY_ART_WIDTHS: [u32; 4] = {WIDTHS:?};\n");
    table.push_str("pub static PICTURES: &[(&str, &[u8])] = &[\n");
    for file in &files {
        let name = name(file);
        for width in WIDTHS {
            let bytes =
                format!("include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{name}-{width}.webp\"))");
            writeln!(table, "    (\"keyart/{width}/{name}.webp\", {bytes}),").expect("table");
        }
    }
    for folder in ["ops", "maps"] {
        for file in pictures(&assets.join("info").join(folder)) {
            let name = name(&file);
            let bytes = format!("include_bytes!({file:?})");
            writeln!(table, "    (\"info/{folder}/{name}.webp\", {bytes}),").expect("table");
        }
    }
    table.push_str("];\n");
    fs::write(out.join("pictures.rs"), table).expect("picture table");

    println!("cargo:rerun-if-changed={ICON}");
    match env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => windows_resource(),
        Ok("linux") => tray_icon(&out),
        _ => {}
    }
}

fn pictures(folder: &Path) -> Vec<PathBuf> {
    println!("cargo:rerun-if-changed={}", folder.display());
    let mut files: Vec<PathBuf> = fs::read_dir(folder)
        .unwrap_or_else(|error| panic!("{}: {error}", folder.display()))
        .map(|entry| entry.expect("picture entry").path())
        .filter(|path| path.extension().is_some_and(|kind| kind == "webp"))
        .collect();
    files.sort();
    files
}

fn windows_resource() {
    println!("cargo:rerun-if-changed=VERSION");
    let version = fs::read_to_string("VERSION").expect("VERSION");
    let version = version.trim();
    let mut parts = version
        .split('.')
        .map(|part| part.parse::<u64>().expect("version part"));
    let mut number = || parts.next().unwrap_or(0);
    let packed = number() << 48 | number() << 32 | number() << 16 | number();
    winresource::WindowsResource::new()
        .set_icon(ICON)
        .set("ProductName", NAME)
        .set("FileDescription", NAME)
        .set("FileVersion", version)
        .set("ProductVersion", version)
        .set_version_info(winresource::VersionInfo::FILEVERSION, packed)
        .set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed)
        .compile()
        .expect("windows resource");
}

fn tray_icon(out: &Path) {
    let icon = image::open(ICON).unwrap_or_else(|error| panic!("{ICON}: {error}"));
    let mut table = String::from("static PIXMAPS: &[(i32, &[u8])] = &[\n");
    for size in TRAY_SIZES {
        let scaled = icon.resize_exact(size, size, FilterType::Lanczos3);
        let mut pixels = scaled.into_rgba8();
        for pixel in pixels.pixels_mut() {
            pixel.0.rotate_right(1);
        }
        fs::write(out.join(format!("tray-{size}.argb")), pixels.as_raw()).expect("tray pixmap");
        let bytes = format!("include_bytes!(concat!(env!(\"OUT_DIR\"), \"/tray-{size}.argb\"))");
        writeln!(table, "    ({size}, {bytes}),").expect("table");
    }
    table.push_str("];\n");
    fs::write(out.join("tray.rs"), table).expect("tray pixmap table");
}

fn name(file: &Path) -> String {
    file.file_stem()
        .expect("picture name")
        .to_string_lossy()
        .into_owned()
}

fn variants(file: &Path, out: &Path) {
    let master = image::open(file).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
    for width in WIDTHS {
        let scaled = if master.width() > width {
            master.resize(width, u32::MAX, FilterType::Lanczos3)
        } else {
            master.clone()
        };
        let pixels = if scaled.color().has_alpha() {
            DynamicImage::ImageRgba8(scaled.into_rgba8())
        } else {
            DynamicImage::ImageRgb8(scaled.into_rgb8())
        };
        let encoded = webp::Encoder::from_image(&pixels)
            .expect("pixel format")
            .encode(QUALITY);
        let target = out.join(format!("{}-{width}.webp", name(file)));
        fs::write(target, &*encoded).expect("key art variant");
    }
}
