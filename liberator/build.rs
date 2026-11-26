use std::fs;

fn main() {
    println!("cargo:rerun-if-changed=../app/VERSION");
    let version = fs::read_to_string("../app/VERSION").expect("app/VERSION");
    let version = version.trim();
    let mut parts = version
        .split('.')
        .map(|part| part.parse::<u64>().expect("version part"));
    let mut number = || parts.next().unwrap_or(0);
    let packed = number() << 48 | number() << 32 | number() << 16 | number();
    winresource::WindowsResource::new()
        .set("FileVersion", version)
        .set("ProductVersion", version)
        .set_version_info(winresource::VersionInfo::FILEVERSION, packed)
        .set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed)
        .compile()
        .expect("windows resource");
}
