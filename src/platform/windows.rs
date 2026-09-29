//! Win32 window and process behaviour gpui doesn't expose.

use gpui::{App, PlatformDisplay, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::rc::Rc;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, POINT, RECT},
    Graphics::Gdi::{
        EnumDisplayMonitors, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MonitorFromPoint,
    },
    UI::WindowsAndMessaging::{
        GetCursorPos, HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow, SetWindowPos,
    },
};

/// The display under the mouse cursor. gpui's Windows `DisplayId` is the
/// monitor's index in `EnumDisplayMonitors` order, so we match on that.
pub fn display_under_cursor(cx: &App) -> Option<Rc<dyn PlatformDisplay>> {
    let target = unsafe {
        let mut cursor = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut cursor) == 0 {
            return None;
        }
        MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST)
    };
    let mut monitors: Vec<HMONITOR> = Vec::new();
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> i32 {
        unsafe { (*(data as *mut Vec<HMONITOR>)).push(monitor) };
        1
    }
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect),
            &mut monitors as *mut _ as LPARAM,
        )
    };
    let index = monitors.iter().position(|&m| m == target)? as u32;
    cx.displays()
        .into_iter()
        .find(|display| u32::from(display.id()) == index)
}

/// Keeps the launcher above fullscreen/topmost apps and takes keyboard focus.
/// Allowed because we are handling the hotkey, the most recent input event.
pub fn raise(window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    if let RawWindowHandle::Win32(handle) = handle.as_raw() {
        let hwnd = handle.hwnd.get() as HWND;
        unsafe {
            SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
            SetForegroundWindow(hwnd);
        }
    }
}

/// Lets CLI output reach the terminal despite the GUI subsystem.
pub fn attach_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

/// Moves idle pages out of the working set.
pub fn trim_memory() {
    use windows_sys::Win32::System::{
        ProcessStatus::K32EmptyWorkingSet, Threading::GetCurrentProcess,
    };
    unsafe { K32EmptyWorkingSet(GetCurrentProcess()) };
}

/// The user's PATH (`HKCU\Environment\Path`), unexpanded, with its registry type.
pub fn read_user_path() -> std::io::Result<(String, u32)> {
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{KEY_READ, REG_EXPAND_SZ, RegQueryValueExW},
    };
    let key = open_environment_key(KEY_READ)?;
    let name = wide("Path");
    let (mut kind, mut size) = (0u32, 0u32);
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            std::ptr::null(),
            &mut kind,
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok((String::new(), REG_EXPAND_SZ));
    }
    if status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status as i32));
    }
    let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            std::ptr::null(),
            &mut kind,
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status as i32));
    }
    buffer.truncate(size as usize / 2);
    while buffer.last() == Some(&0) {
        buffer.pop();
    }
    Ok((String::from_utf16_lossy(&buffer), kind))
}

/// Writes the user's PATH (keeping `kind`) and tells running programs, such as
/// Explorer, so new terminals pick it up without logging out.
pub fn write_user_path(value: &str, kind: u32) -> std::io::Result<()> {
    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS,
        System::Registry::{KEY_SET_VALUE, RegSetValueExW},
        UI::WindowsAndMessaging::{
            HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
        },
    };
    let key = open_environment_key(KEY_SET_VALUE)?;
    let data = wide(value);
    let status = unsafe {
        RegSetValueExW(
            key.0,
            wide("Path").as_ptr(),
            0,
            kind,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status as i32));
    }
    let environment = wide("Environment");
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            environment.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5000,
            std::ptr::null_mut(),
        )
    };
    Ok(())
}

/// An open registry key, closed on drop.
struct Key(windows_sys::Win32::System::Registry::HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::System::Registry::RegCloseKey(self.0) };
    }
}

fn open_environment_key(access: u32) -> std::io::Result<Key> {
    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS,
        System::Registry::{HKEY_CURRENT_USER, RegOpenKeyExW},
    };
    let mut key = std::ptr::null_mut();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            wide("Environment").as_ptr(),
            0,
            access,
            &mut key,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status as i32));
    }
    Ok(Key(key))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
