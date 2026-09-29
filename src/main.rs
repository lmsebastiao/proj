// No console window for the background launcher on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod fuzzy;
mod input;
mod open;
mod palette;
mod store;
#[cfg(windows)]
mod win;

use std::{path::PathBuf, str::FromStr, time::Duration};

use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use gpui::{
    App, Application, Bounds, Context, Focusable, Global, Window, WindowBackgroundAppearance,
    WindowBounds, WindowHandle, WindowKind, WindowOptions, div, point, prelude::*, px, size,
};

use palette::Palette;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        run_launcher();
    } else {
        attach_console();
        std::process::exit(run_cli(&args));
    }
}

const USAGE: &str = "\
proj - project launcher

usage:
  proj                 run the launcher in the background
  proj add [PATH]      add a project (defaults to the current directory)
  proj remove PATH     remove / hide a project
  proj list            list known projects
  proj paths           show config and database locations";

fn run_cli(args: &[String]) -> i32 {
    let config = store::load_config();
    let mut db = store::load_db();
    let path_arg = |arg: Option<&String>| -> Option<PathBuf> {
        match arg {
            Some(arg) => store::normalize(arg),
            None => std::env::current_dir().ok(),
        }
    };
    match args[0].as_str() {
        "add" => {
            let Some(path) = path_arg(args.get(1)).filter(|p| p.is_dir()) else {
                eprintln!("proj: not a directory");
                return 1;
            };
            db.hidden.remove(&path);
            if !db.manual.contains(&path) {
                db.manual.push(path.clone());
            }
            println!("added {}", path.display());
        }
        "remove" | "rm" => {
            let Some(path) = args.get(1).and_then(|a| store::normalize(a)) else {
                eprintln!("usage: proj remove PATH");
                return 1;
            };
            if db.manual.contains(&path) {
                db.manual.retain(|p| p != &path);
            } else {
                db.hidden.insert(path.clone());
            }
            db.opened.remove(path.to_string_lossy().as_ref());
            println!("removed {}", path.display());
        }
        "list" | "ls" => {
            for project in store::collect(&config, &db) {
                println!("{:<32} {}", project.name, project.path.display());
            }
            return 0;
        }
        "paths" => {
            println!("config:   {}", store::config_path().display());
            println!("projects: {}", store::db_path().display());
            return 0;
        }
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            return 0;
        }
        other => {
            eprintln!("proj: unknown command '{other}'\n\n{USAGE}");
            return 2;
        }
    }
    if let Err(err) = store::save_db(&db) {
        eprintln!("proj: could not save: {err}");
        return 1;
    }
    0
}

/// Handle to the open palette, if any.
#[derive(Default)]
struct PaletteWindow(Option<WindowHandle<Palette>>);

impl Global for PaletteWindow {}

/// Invisible window that keeps the app alive: gpui quits when the last window
/// closes, and the palette window is destroyed each time it is dismissed so
/// its GPU resources are released while idle.
struct Anchor;

impl Render for Anchor {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn run_launcher() {
    Application::new().run(|cx: &mut App| {
        let config = store::load_config();
        let hotkey = match HotKey::from_str(&config.hotkey) {
            Ok(hotkey) => hotkey,
            Err(err) => return fatal(cx, &format!("invalid hotkey '{}': {err}", config.hotkey)),
        };
        let manager = match GlobalHotKeyManager::new() {
            Ok(manager) => manager,
            Err(err) => return fatal(cx, &format!("could not start hotkey listener: {err}")),
        };
        if let Err(err) = manager.register(hotkey) {
            // Also acts as a single-instance guard.
            return fatal(
                cx,
                &format!(
                    "could not register '{}' ({err}). Is proj already running?",
                    config.hotkey
                ),
            );
        }

        palette::bind_keys(cx);
        cx.set_global(PaletteWindow::default());

        let anchor = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(1.), px(1.)),
                ))),
                titlebar: None,
                focus: false,
                show: false,
                kind: WindowKind::PopUp,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                ..Default::default()
            },
            |_, cx| cx.new(|_| Anchor),
        );
        if let Err(err) = anchor {
            return fatal(cx, &format!("could not create window: {err}"));
        }

        cx.on_window_closed(|cx| {
            cx.spawn(async |cx| {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                trim_memory();
            })
            .detach();
        })
        .detach();

        // The hotkey callback runs on the platform event loop; forward presses
        // to a foreground task instead of polling.
        let (tx, rx) = async_channel::bounded::<()>(1);
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state == HotKeyState::Pressed {
                tx.try_send(()).ok();
            }
        }));
        cx.spawn(async move |cx| {
            let _manager = manager; // unregisters the hotkey when dropped
            while rx.recv().await.is_ok() {
                cx.update(toggle_palette).ok();
            }
        })
        .detach();

        trim_memory();
    });
}

fn toggle_palette(cx: &mut App) {
    if let Some(handle) = open_palette(cx) {
        handle.update(cx, |_, window, _| window.remove_window()).ok();
        cx.global_mut::<PaletteWindow>().0 = None;
        return;
    }

    #[cfg(windows)]
    let display = win::display_under_cursor(cx).or_else(|| cx.primary_display());
    #[cfg(not(windows))]
    let display = cx.primary_display();

    let window_size = size(px(680.), px(440.));
    let bounds = match &display {
        Some(display) => {
            let screen = display.bounds();
            Bounds::new(
                point(
                    screen.origin.x + (screen.size.width - window_size.width) * 0.5,
                    screen.origin.y + screen.size.height * 0.2,
                ),
                window_size,
            )
        }
        None => Bounds::centered(None, window_size, cx),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id: display.map(|d| d.id()),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("proj".into()),
        ..Default::default()
    };
    match cx.open_window(options, |window, cx| cx.new(|cx| Palette::new(window, cx))) {
        Ok(handle) => {
            handle
                .update(cx, |palette, window, cx| {
                    window.focus(&palette.focus_handle(cx));
                    #[cfg(windows)]
                    win::raise(window);
                    #[cfg(not(windows))]
                    window.activate_window();
                    cx.activate(true);
                })
                .ok();
            cx.global_mut::<PaletteWindow>().0 = Some(handle);
        }
        Err(err) => eprintln!("proj: could not open window: {err}"),
    }
}

/// The palette closes itself (blur, escape, open), so the stored handle may be stale.
fn open_palette(cx: &App) -> Option<WindowHandle<Palette>> {
    let handle = cx.global::<PaletteWindow>().0?;
    cx.windows()
        .iter()
        .any(|w| w.window_id() == handle.window_id())
        .then_some(handle)
}

fn fatal(cx: &mut App, message: &str) {
    attach_console();
    eprintln!("proj: {message}");
    cx.quit();
}

/// Lets `proj add` etc. print to the terminal despite the GUI subsystem.
fn attach_console() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// Returns idle pages to the OS while the launcher sits in the background.
fn trim_memory() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::{ProcessStatus::K32EmptyWorkingSet, Threading::GetCurrentProcess};
        K32EmptyWorkingSet(GetCurrentProcess());
    }
}
