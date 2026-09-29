//! OS-specific behaviour behind small cross-platform functions.

#[cfg(windows)]
mod windows;

use std::rc::Rc;

use gpui::{App, PlatformDisplay, Window};

/// Where to show the launcher: the display under the mouse cursor where
/// supported, else the primary display.
pub fn launcher_display(cx: &App) -> Option<Rc<dyn PlatformDisplay>> {
    #[cfg(windows)]
    if let Some(display) = windows::display_under_cursor(cx) {
        return Some(display);
    }
    cx.primary_display()
}

/// Brings `window` to the front with keyboard focus (and, on Windows, above
/// fullscreen apps).
pub fn raise(window: &Window) {
    #[cfg(windows)]
    windows::raise(window);
    #[cfg(not(windows))]
    window.activate_window();
}

/// Lets `proj add` etc. print to the terminal (the Windows build has no console).
pub fn attach_console() {
    #[cfg(windows)]
    windows::attach_console();
}

/// Returns idle memory to the OS while the launcher sits in the background.
pub fn trim_memory() {
    #[cfg(windows)]
    windows::trim_memory();
}
