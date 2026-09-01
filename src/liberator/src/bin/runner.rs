#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    std::process::exit(liberator::win::main_entry());
}

#[cfg(not(windows))]
fn main() {}
