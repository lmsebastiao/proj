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

/// Visible top-level windows as Alt+Tab would list them, front to back: no
/// owned, tool or cloaked (other virtual desktop) windows, and none from proj.
pub fn top_windows() -> Vec<super::TopWindow> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute},
        System::Threading::{
            OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GW_OWNER, GWL_EXSTYLE, GetWindow, GetWindowLongW, GetWindowTextW,
            GetWindowThreadProcessId, IsWindowVisible, WS_EX_TOOLWINDOW,
        },
    };

    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> i32 {
        unsafe { (*(data as *mut Vec<HWND>)).push(hwnd) };
        1
    }
    let mut handles: Vec<HWND> = Vec::new();
    unsafe { EnumWindows(Some(collect), &mut handles as *mut _ as LPARAM) };

    let own = std::process::id();
    let mut exes: std::collections::HashMap<u32, Option<std::path::PathBuf>> = Default::default();
    let mut windows = Vec::new();
    for hwnd in handles {
        let listed = unsafe {
            IsWindowVisible(hwnd) != 0
                && GetWindow(hwnd, GW_OWNER).is_null()
                && GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW == 0
        };
        if !listed {
            continue;
        }
        let mut cloaked = 0u32;
        unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED as u32,
                (&mut cloaked as *mut u32).cast(),
                size_of::<u32>() as u32,
            )
        };
        if cloaked != 0 {
            continue;
        }
        let mut buffer = [0u16; 512];
        let len = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
        if len <= 0 {
            continue;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid == own {
            continue;
        }
        let exe = exes.entry(pid).or_insert_with(|| unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let mut path = [0u16; 1024];
            let mut size = path.len() as u32;
            let ok = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                path.as_mut_ptr(),
                &mut size,
            );
            CloseHandle(process);
            (ok != 0).then(|| String::from_utf16_lossy(&path[..size as usize]).into())
        });
        let Some(exe) = exe.clone() else {
            continue;
        };
        windows.push(super::TopWindow {
            window: super::WindowRef(hwnd as isize),
            title: String::from_utf16_lossy(&buffer[..len as usize]),
            exe,
        });
    }
    windows
}

pub fn foreground_window() -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    unsafe { GetForegroundWindow() as isize }
}

/// Brings a window to the front, restoring it if minimized.
pub fn focus_window(hwnd: isize) {
    use windows_sys::Win32::{
        System::Threading::{AttachThreadInput, GetCurrentThreadId},
        UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId, IsIconic, SW_RESTORE, ShowWindow,
        },
    };
    let hwnd = hwnd as HWND;
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
        if GetForegroundWindow() == hwnd {
            return;
        }
        // Windows only lets the app that got the last input take the foreground;
        // after a quick shortcut tap that is the app in front (it got the key
        // release). Sharing its input state for a moment lifts that.
        let front = GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut());
        let ours = GetCurrentThreadId();
        if front != 0 && front != ours && AttachThreadInput(ours, front, 1) != 0 {
            SetForegroundWindow(hwnd);
            AttachThreadInput(ours, front, 0);
        }
        if GetForegroundWindow() != hwnd {
            // What Windows' own task switching uses; not bound by that rule.
            windows_sys::Win32::UI::WindowsAndMessaging::SwitchToThisWindow(hwnd, 1);
        }
    }
}

/// Whether every one of these modifiers is physically held down right now.
pub fn modifiers_held(mods: global_hotkey::hotkey::Modifiers) -> bool {
    use global_hotkey::hotkey::Modifiers;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    let down = |vk: u16| unsafe { GetAsyncKeyState(i32::from(vk)) } < 0;
    (!mods.contains(Modifiers::ALT) || down(VK_MENU))
        && (!mods.contains(Modifiers::CONTROL) || down(VK_CONTROL))
        && (!mods.contains(Modifiers::SHIFT) || down(VK_SHIFT))
        && (!mods.contains(Modifiers::SUPER) || down(VK_LWIN) || down(VK_RWIN))
}

/// A program's icon at about `size` pixels square: width, height and BGRA
/// pixels with straight alpha. `None` if it has none.
pub fn app_icon(path: &std::path::Path, size: u32) -> Option<(u32, u32, Vec<u8>)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Graphics::Gdi::DeleteObject,
        UI::{
            Shell::SHDefExtractIconW,
            WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO},
        },
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut icon: HICON = std::ptr::null_mut();
    // S_OK only: S_FALSE means the file has no icon.
    let found =
        unsafe { SHDefExtractIconW(wide.as_ptr(), 0, 0, &mut icon, std::ptr::null_mut(), size) }
            == 0;
    if !found || icon.is_null() {
        return None;
    }
    let mut info = ICONINFO::default();
    let pixels = (unsafe { GetIconInfo(icon, &mut info) } != 0).then(|| {
        let (width, height, mut bgra) = bitmap_pixels(info.hbmColor)?;
        // Old icons without alpha get it from their mask (black: opaque).
        if bgra.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
            let (_, _, mask) = bitmap_pixels(info.hbmMask)?;
            if mask.len() == bgra.len() {
                let pixels = bgra.as_chunks_mut::<4>().0.iter_mut();
                for (p, m) in pixels.zip(mask.as_chunks::<4>().0) {
                    p[3] = if m[0] == 0 { 255 } else { 0 };
                }
            }
        }
        Some((width, height, bgra))
    });
    unsafe {
        DeleteObject(info.hbmColor);
        DeleteObject(info.hbmMask);
        DestroyIcon(icon);
    }
    pixels.flatten()
}

/// A bitmap's pixels as top-down 32-bit BGRA.
fn bitmap_pixels(
    bitmap: windows_sys::Win32::Graphics::Gdi::HBITMAP,
) -> Option<(u32, u32, Vec<u8>)> {
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
        GetDIBits, GetObjectW,
    };
    if bitmap.is_null() {
        return None;
    }
    let mut bm = BITMAP::default();
    let got = unsafe { GetObjectW(bitmap, size_of::<BITMAP>() as i32, (&raw mut bm).cast()) };
    if got == 0 {
        return None;
    }
    let (width, height) = (bm.bmWidth, bm.bmHeight.abs());
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Negative: top-down rows.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    };
    let (width, height) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    let lines = unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            height,
            pixels.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        );
        DeleteDC(dc);
        lines
    };
    (u32::try_from(lines).ok() == Some(height)).then_some((width, height, pixels))
}
