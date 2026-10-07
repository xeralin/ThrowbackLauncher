#![cfg_attr(windows, windows_subsystem = "windows")]

pub mod relocate;
pub mod scan_build;
pub mod tables;
pub mod tree;

#[cfg(windows)]
pub mod runtime;

#[cfg(windows)]
pub mod shadow;

#[cfg(windows)]
pub mod unlock;

#[cfg(windows)]
fn main() {
    std::process::exit(runtime::main_entry());
}

#[cfg(not(windows))]
fn main() {}
