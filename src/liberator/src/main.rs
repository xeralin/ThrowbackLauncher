#![cfg_attr(windows, windows_subsystem = "windows")]

pub mod buildscan;
pub mod scanrip;
pub mod tables;
pub mod tree;

#[cfg(windows)]
pub mod badges;

#[cfg(windows)]
pub mod onboarding;

#[cfg(windows)]
pub mod ownership;

#[cfg(windows)]
pub mod shadow;

#[cfg(windows)]
pub mod win;

#[cfg(windows)]
fn main() {
    std::process::exit(win::main_entry());
}

#[cfg(not(windows))]
fn main() {}
