// No console window for the background launcher on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod autostart;
mod cli;
mod config;
mod editors;
mod fuzzy;
mod git;
mod input;
mod launcher;
mod open;
mod palette;
mod paths;
mod platform;
mod recent;
mod sqlite;
mod store;
mod switcher;
mod tasks;
mod templates;
mod tray;
mod update;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        launcher::run();
    } else {
        platform::attach_console();
        std::process::exit(cli::run(&args));
    }
}
