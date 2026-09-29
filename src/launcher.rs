//! The background app: global hotkey, tray icon, and showing/hiding the palette window.

use std::{
    str::FromStr,
    time::{Duration, Instant},
};

use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use gpui::{
    App, Application, Bounds, Context, Focusable, Global, Window, WindowBackgroundAppearance,
    WindowBounds, WindowHandle, WindowKind, WindowOptions, div, point, prelude::*, px, size,
};

use crate::{
    autostart, config, editors, open,
    palette::{self, Palette},
    platform,
    tray::{Tray, TrayCommand},
};

/// The open palette, if any, and when it last closed.
#[derive(Default)]
struct PaletteWindow {
    handle: Option<WindowHandle<Palette>>,
    closed_at: Option<Instant>,
}

impl Global for PaletteWindow {}

/// Events from the hotkey and tray callbacks, handled on the main thread.
enum Command {
    Hotkey,
    Tray(TrayCommand),
}

/// Invisible window that keeps the app alive: gpui quits when the last window
/// closes, and the palette window is destroyed each time it is dismissed so
/// its GPU resources are released while idle.
struct Anchor;

impl Render for Anchor {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

pub fn run() {
    Application::new().run(|cx: &mut App| {
        let config = config::load_config();
        let manager = match GlobalHotKeyManager::new() {
            Ok(manager) => manager,
            Err(err) => return fatal(cx, &format!("could not start hotkey listener: {err}")),
        };
        let problems = register_hotkeys(&manager, &config.hotkey);
        if problems.len() == config.hotkey.len() {
            // Nothing registered. Also acts as a single-instance guard.
            return fatal(
                cx,
                &format!(
                    "no shortcut could be registered ({}). Is proj already running?",
                    problems.join("; ")
                ),
            );
        }
        if !problems.is_empty() {
            // Some shortcuts work; say which don't where the user will see it.
            let notice = problems.join("; ");
            cx.set_global(palette::StartupNotice(notice.into()));
        }

        palette::bind_keys(cx);
        // Finds installed editors off the main thread so the first open is instant.
        std::thread::spawn(|| editors::detected_editors(false));
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
            cx.global_mut::<PaletteWindow>().closed_at = Some(Instant::now());
            cx.spawn(async |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                platform::trim_memory();
            })
            .detach();
        })
        .detach();

        // Hotkey and tray callbacks run on the platform event loop; forward them
        // to a foreground task instead of polling.
        let (tx, rx) = async_channel::unbounded::<Command>();
        let hotkey_tx = tx.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state == HotKeyState::Pressed {
                hotkey_tx.try_send(Command::Hotkey).ok();
            }
        }));
        let tooltip = format!("proj ({})", config.hotkey_label());
        let tray = Tray::new(&tooltip, autostart::is_enabled(), move |command| {
            tx.try_send(Command::Tray(command)).ok();
        })
        .map_err(|err| eprintln!("proj: no tray icon: {err}"))
        .ok();
        cx.spawn(async move |cx| {
            // Both unregister when dropped, so they live as long as this task.
            let _manager = manager;
            let tray = tray;
            while let Ok(command) = rx.recv().await {
                cx.update(|cx| handle(command, tray.as_ref(), cx)).ok();
            }
        })
        .detach();

        // First run (or editor never chosen): show the setup right away.
        if config.editor.is_none() {
            toggle_palette(cx);
        } else {
            platform::trim_memory();
        }
    });
}

fn handle(command: Command, tray: Option<&Tray>, cx: &mut App) {
    let sync_tray = |tray: Option<&Tray>| {
        if let Some(tray) = tray {
            tray.set_autostart(autostart::is_enabled());
        }
    };
    match command {
        Command::Hotkey => toggle_palette(cx),
        Command::Tray(TrayCommand::Toggle) => {
            // Clicking the tray icon takes focus from the palette, which closes it
            // just before the click arrives; treat that click as "close".
            let closed_at = cx.global::<PaletteWindow>().closed_at;
            if !closed_at.is_some_and(|at| at.elapsed() < Duration::from_millis(400)) {
                toggle_palette(cx);
            }
        }
        Command::Tray(TrayCommand::ToggleAutostart) => {
            if let Err(err) = autostart::set(!autostart::is_enabled()) {
                eprintln!("proj: could not change start on login: {err}");
            }
            sync_tray(tray);
        }
        Command::Tray(TrayCommand::Refresh) => sync_tray(tray),
        Command::Tray(TrayCommand::OpenConfig) => {
            let config = config::load_config();
            if let Err(err) = open::open_project(&config, &config::config_path()) {
                eprintln!("proj: could not open config: {err}");
            }
        }
        Command::Tray(TrayCommand::Quit) => cx.quit(),
    }
}

/// Registers every shortcut, returning a message for each one that failed.
fn register_hotkeys(manager: &GlobalHotKeyManager, hotkeys: &[String]) -> Vec<String> {
    hotkeys
        .iter()
        .filter_map(|text| {
            let hotkey = match HotKey::from_str(text) {
                Ok(hotkey) => hotkey,
                Err(err) => return Some(format!("Shortcut '{text}' is not valid ({err})")),
            };
            match manager.register(hotkey) {
                Ok(()) => None,
                Err(global_hotkey::Error::AlreadyRegistered(_)) => {
                    Some(format!("Shortcut '{text}' is taken by another app"))
                }
                Err(err) => Some(format!("Shortcut '{text}': {err}")),
            }
        })
        .collect()
}

fn toggle_palette(cx: &mut App) {
    if let Some(handle) = open_palette(cx) {
        handle
            .update(cx, |_, window, _| window.remove_window())
            .ok();
        cx.global_mut::<PaletteWindow>().handle = None;
        return;
    }

    let display = platform::launcher_display(cx);

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
                    platform::raise(window);
                    cx.activate(true);
                })
                .ok();
            cx.global_mut::<PaletteWindow>().handle = Some(handle);
        }
        Err(err) => eprintln!("proj: could not open window: {err}"),
    }
}

/// The palette closes itself (blur, escape, open), so the stored handle may be stale.
fn open_palette(cx: &App) -> Option<WindowHandle<Palette>> {
    let handle = cx.global::<PaletteWindow>().handle?;
    cx.windows()
        .iter()
        .any(|w| w.window_id() == handle.window_id())
        .then_some(handle)
}

fn fatal(cx: &mut App, message: &str) {
    platform::attach_console();
    eprintln!("proj: {message}");
    cx.quit();
}
