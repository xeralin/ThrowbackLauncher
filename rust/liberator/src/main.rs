#![windows_subsystem = "windows"]

mod relocate;
mod runtime;
mod scan_build;
mod shadow;
mod tables;
mod tree;
mod unlock;

fn main() {
    std::process::exit(runtime::main_entry());
}
