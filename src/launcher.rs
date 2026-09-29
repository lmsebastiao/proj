//! The background app: global hotkey, and showing/hiding the palette window.

use std::{str::FromStr, time::Duration};

use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use gpui::{
    App, Application, Bounds, Context, Focusable, Global, Window, WindowBackgroundAppearance,
    WindowBounds, WindowHandle, WindowKind, WindowOptions, div, point, prelude::*, px, size,
};

use crate::{
    config, editors,
    palette::{self, Palette},
    platform,
};

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

pub fn run() {
    Application::new().run(|cx: &mut App| {
        let config = config::load_config();
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
            cx.spawn(async |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                platform::trim_memory();
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

        // First run (or editor never chosen): show the setup right away.
        if config.editor.is_none() {
            toggle_palette(cx);
        } else {
            platform::trim_memory();
        }
    });
}

fn toggle_palette(cx: &mut App) {
    if let Some(handle) = open_palette(cx) {
        handle
            .update(cx, |_, window, _| window.remove_window())
            .ok();
        cx.global_mut::<PaletteWindow>().0 = None;
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
    platform::attach_console();
    eprintln!("proj: {message}");
    cx.quit();
}
