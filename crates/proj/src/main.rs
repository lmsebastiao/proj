// No console window for the background launcher on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        proj_ui::launcher::run();
    } else {
        proj_cli::platform::attach_console();
        std::process::exit(proj_cli::cli::run(&args));
    }
}
