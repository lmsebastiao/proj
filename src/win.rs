//! Windows-specific window behaviour gpui doesn't expose.

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
