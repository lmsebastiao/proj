//! OS-specific behaviour behind small cross-platform functions.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use std::rc::Rc;

use gpui::{App, PlatformDisplay, Window};

use crate::config::MonitorSetting;

/// Where to show the launcher: the display `monitor` asks for where that's
/// supported (Windows), else the primary display.
pub fn launcher_display(monitor: MonitorSetting, cx: &App) -> Option<Rc<dyn PlatformDisplay>> {
    #[cfg(windows)]
    {
        let display = match monitor {
            MonitorSetting::Cursor => windows::display_under_cursor(cx),
            MonitorSetting::Focused => windows::display_of_foreground(cx),
            MonitorSetting::Primary => None,
        };
        if let Some(display) = display {
            return Some(display);
        }
    }
    #[cfg(not(windows))]
    let _ = monitor;
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

/// Whether `app_icon` can find programs' icons here.
pub const HAS_APP_ICONS: bool = cfg!(any(windows, target_os = "macos"));

/// A program's icon at about `size` pixels square: width, height and BGRA
/// pixels with straight alpha (Windows and macOS).
pub fn app_icon(path: &std::path::Path, size: u32) -> Option<(u32, u32, Vec<u8>)> {
    #[cfg(windows)]
    return windows::app_icon(path, size);
    #[cfg(target_os = "macos")]
    return macos::app_icon(path, size);
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (path, size);
        None
    }
}

/// Why the switcher can't list other apps' windows, when it can't: on macOS,
/// until proj is allowed to use the Accessibility API.
pub fn window_access_hint() -> Option<&'static str> {
    #[cfg(target_os = "macos")]
    return macos::window_access_hint();
    #[cfg(not(target_os = "macos"))]
    None
}

/// Another app's top-level window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WindowRef(isize);

#[cfg(test)]
impl WindowRef {
    pub fn test(id: isize) -> Self {
        Self(id)
    }
}

pub struct TopWindow {
    pub window: WindowRef,
    pub title: String,
    /// The program that owns it.
    pub exe: std::path::PathBuf,
}

/// Other apps' windows as Alt+Tab lists them, front to back (Windows and
/// macOS). On macOS, `WindowRef`s point into the last list made.
pub fn top_windows() -> Vec<TopWindow> {
    #[cfg(windows)]
    return windows::top_windows();
    #[cfg(target_os = "macos")]
    return macos::top_windows();
    #[cfg(not(any(windows, target_os = "macos")))]
    Vec::new()
}

/// The window in front. On macOS, only one of the last `top_windows` list.
pub fn foreground_window() -> Option<WindowRef> {
    #[cfg(windows)]
    return Some(WindowRef(windows::foreground_window()));
    #[cfg(target_os = "macos")]
    return macos::foreground_window();
    #[cfg(not(any(windows, target_os = "macos")))]
    None
}

pub fn focus_window(window: WindowRef) {
    #[cfg(windows)]
    windows::focus_window(window.0);
    #[cfg(target_os = "macos")]
    macos::focus_window(window);
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = window;
}

/// Asks a window to close, as its close button does: the app can still ask
/// about unsaved changes first.
pub fn close_window(window: WindowRef) {
    #[cfg(windows)]
    windows::close_window(window.0);
    #[cfg(target_os = "macos")]
    macos::close_window(window);
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = window;
}

/// Whether all of `mods` are still held down, e.g. while cycling the switcher.
pub fn modifiers_held(mods: global_hotkey::hotkey::Modifiers) -> bool {
    #[cfg(windows)]
    return windows::modifiers_held(mods);
    #[cfg(target_os = "macos")]
    {
        use global_hotkey::hotkey::Modifiers;
        macos::modifiers_held(
            mods.contains(Modifiers::ALT),
            mods.contains(Modifiers::CONTROL),
            mods.contains(Modifiers::SHIFT),
            mods.intersects(Modifiers::SUPER | Modifiers::META),
        )
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = mods;
        false
    }
}

/// Whether the folder containing proj.exe is on the user's PATH.
pub fn exe_dir_on_path() -> std::io::Result<bool> {
    #[cfg(windows)]
    {
        let dir = exe_dir()?;
        let (path, _) = windows::read_user_path()?;
        Ok(edit_path_list(&path, &dir, true).is_none())
    }
    #[cfg(not(windows))]
    Err(unsupported())
}

/// Adds (`on`) or removes the folder containing proj.exe on the user's PATH.
/// Returns whether anything changed. Used by the Windows installer.
pub fn set_exe_dir_on_path(on: bool) -> std::io::Result<bool> {
    #[cfg(windows)]
    {
        let dir = exe_dir()?;
        let (path, kind) = windows::read_user_path()?;
        match edit_path_list(&path, &dir, on) {
            Some(new) => windows::write_user_path(&new, kind).map(|()| true),
            None => Ok(false),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = on;
        Err(unsupported())
    }
}

#[cfg(windows)]
fn exe_dir() -> std::io::Result<String> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| std::io::Error::other("proj.exe has no parent folder"))?;
    Ok(dir.to_string_lossy().into_owned())
}

#[cfg(not(windows))]
fn unsupported() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "only supported on Windows; add proj's folder to PATH in your shell profile",
    )
}

/// Adds (`present`) or removes `dir` in a `;`-separated PATH value, leaving the
/// other entries as they are. Returns `None` when nothing needs to change.
/// Entries match case-insensitively and ignoring a trailing slash.
#[cfg_attr(not(windows), allow(dead_code))]
fn edit_path_list(current: &str, dir: &str, present: bool) -> Option<String> {
    let normalize = |entry: &str| entry.trim().trim_end_matches(['\\', '/']).to_lowercase();
    let target = normalize(dir);
    let entries: Vec<&str> = current
        .split(';')
        .filter(|e| !e.trim().is_empty())
        .collect();
    let found = entries.iter().any(|e| normalize(e) == target);
    match (present, found) {
        (true, false) => Some(
            entries
                .into_iter()
                .chain([dir])
                .collect::<Vec<_>>()
                .join(";"),
        ),
        (false, true) => Some(
            entries
                .into_iter()
                .filter(|e| normalize(e) != target)
                .collect::<Vec<_>>()
                .join(";"),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_path_lists() {
        let dir = r"C:\Users\me\AppData\Local\Programs\proj";
        assert_eq!(edit_path_list("", dir, true).as_deref(), Some(dir));
        assert_eq!(
            edit_path_list(r"%USERPROFILE%\bin;C:\tools;", dir, true).as_deref(),
            Some(r"%USERPROFILE%\bin;C:\tools;C:\Users\me\AppData\Local\Programs\proj"),
            "other entries (and their %VARIABLES%) stay as they were"
        );
        // Already there, in another case or with a trailing slash: nothing to do.
        let with = r"C:\tools;c:\users\me\appdata\local\programs\PROJ\";
        assert_eq!(edit_path_list(with, dir, true), None);
        assert_eq!(
            edit_path_list(with, dir, false).as_deref(),
            Some(r"C:\tools")
        );
        assert_eq!(edit_path_list(r"C:\tools", dir, false), None);
    }
}
