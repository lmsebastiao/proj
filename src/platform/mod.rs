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

/// Another app's top-level window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WindowRef(isize);

pub struct TopWindow {
    pub window: WindowRef,
    pub title: String,
    /// The program that owns it.
    pub exe: std::path::PathBuf,
}

/// Other apps' windows as Alt+Tab lists them, front to back (Windows only).
pub fn top_windows() -> Vec<TopWindow> {
    #[cfg(windows)]
    return windows::top_windows();
    #[cfg(not(windows))]
    Vec::new()
}

pub fn foreground_window() -> Option<WindowRef> {
    #[cfg(windows)]
    return Some(WindowRef(windows::foreground_window()));
    #[cfg(not(windows))]
    None
}

pub fn focus_window(window: WindowRef) {
    #[cfg(windows)]
    windows::focus_window(window.0);
    #[cfg(not(windows))]
    let _ = window;
}

/// Whether all of `mods` are still held down, e.g. while cycling the switcher.
pub fn modifiers_held(mods: global_hotkey::hotkey::Modifiers) -> bool {
    #[cfg(windows)]
    return windows::modifiers_held(mods);
    #[cfg(not(windows))]
    {
        let _ = mods;
        false
    }
}

/// The key left of 1 (under Esc) as a shortcut key name, which depends on
/// the keyboard layout: "Backquote" on US keyboards, "Backslash" on Portuguese…
pub fn key_left_of_1() -> &'static str {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            VK_OEM_1, VK_OEM_2, VK_OEM_4, VK_OEM_5, VK_OEM_6, VK_OEM_7, VK_OEM_MINUS, VK_OEM_PLUS,
        };
        match windows::key_left_of_1() {
            VK_OEM_5 => "Backslash",
            VK_OEM_7 => "Quote",
            VK_OEM_1 => "Semicolon",
            VK_OEM_2 => "Slash",
            VK_OEM_4 => "BracketLeft",
            VK_OEM_6 => "BracketRight",
            VK_OEM_MINUS => "Minus",
            VK_OEM_PLUS => "Equal",
            // VK_OEM_3, and layouts whose key there has no name here.
            _ => "Backquote",
        }
    }
    #[cfg(not(windows))]
    "Backquote"
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
