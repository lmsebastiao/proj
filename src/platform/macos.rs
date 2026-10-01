//! macOS: app icons from the workspace, and the window switcher through the
//! Accessibility API, which proj needs to be allowed to use (System Settings ›
//! Privacy & Security › Accessibility).

use std::{
    path::{Path, PathBuf},
    ptr::{self, NonNull},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicIsize, Ordering},
    },
};

use objc2_app_kit::{
    NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication,
    NSWorkspace,
};
use objc2_application_services::{
    AXError, AXIsProcessTrusted, AXIsProcessTrustedWithOptions, AXUIElement,
    kAXTrustedCheckOptionPrompt,
};
use objc2_core_foundation::{
    CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect,
    CGSize,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGEventFlags, CGEventSource,
    CGEventSourceStateID, CGImageAlphaInfo, CGImageByteOrderInfo, CGWindowListCopyWindowInfo,
    CGWindowListOption, kCGNullWindowID, kCGWindowLayer, kCGWindowOwnerPID,
};
use objc2_foundation::NSString;

use super::{TopWindow, WindowRef};

/// A window `top_windows` listed: its `WindowRef` id, its app, and the window.
struct Listed {
    id: isize,
    pid: i32,
    window: CFRetained<AXUIElement>,
}

// Accessibility elements are references to another process's objects, safe to
// use from any thread.
unsafe impl Send for Listed {}

/// The windows `top_windows` listed last, which `WindowRef`s point to.
static LISTED: Mutex<Vec<Listed>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicIsize = AtomicIsize::new(1);

/// A program's icon, or its app's for a program inside one: width, height and
/// BGRA pixels with straight alpha.
pub fn app_icon(path: &Path, size: u32) -> Option<(u32, u32, Vec<u8>)> {
    let path = path
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .unwrap_or(path);
    let image =
        NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&path.to_string_lossy()));
    let points = f64::from(size);
    let rect = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(points, points));
    let mut proposed = rect;
    // The image's best representation at that size.
    let icon =
        unsafe { image.CGImageForProposedRect_context_hints(&raw mut proposed, None, None) }?;
    let side_px = usize::try_from(size).ok()?;
    let mut pixels = vec![0u8; side_px * side_px * 4];
    let space = CGColorSpace::new_device_rgb()?;
    // 32-bit ARGB, little-endian: B, G, R, A in memory, as gpui wants them.
    let info = CGImageAlphaInfo::PremultipliedFirst.0 | CGImageByteOrderInfo::Order32Little.0;
    let context = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            side_px,
            side_px,
            8,
            side_px * 4,
            Some(&space),
            info,
        )
    }?;
    CGContext::draw_image(Some(&context), rect, Some(&icon));
    drop(context);
    // Core Graphics draws premultiplied; gpui wants straight alpha.
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u32::from(pixel[3]);
        if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = u8::try_from((u32::from(*channel) * 255 + alpha / 2) / alpha)
                    .unwrap_or(u8::MAX);
            }
        }
    }
    Some((size, size, pixels))
}

/// Whether proj may use the Accessibility API. The first time it may not,
/// macOS is asked to show where to allow it.
fn trusted() -> bool {
    static ASKED: AtomicBool = AtomicBool::new(false);
    if unsafe { AXIsProcessTrusted() } {
        return true;
    }
    if !ASKED.swap(true, Ordering::Relaxed) {
        let prompt: &CFString = unsafe { kAXTrustedCheckOptionPrompt };
        let options = CFDictionary::from_slices(&[prompt], &[CFBoolean::new(true)]);
        unsafe { AXIsProcessTrustedWithOptions(Some(options.as_opaque())) };
    }
    false
}

/// Why the switcher can't list windows, if it can't.
pub fn window_access_hint() -> Option<&'static str> {
    (!unsafe { AXIsProcessTrusted() }).then_some(
        "Allow proj in System Settings › Privacy & Security › Accessibility to list windows",
    )
}

/// Apps' standard windows, front to back as far as macOS tells: the apps in
/// the order their windows are on screen, each app's windows in its own order.
pub fn top_windows() -> Vec<TopWindow> {
    let mut listed = Vec::new();
    let mut windows = Vec::new();
    if trusted() {
        for pid in apps_front_to_back() {
            list_app_windows(pid, &mut windows, &mut listed);
        }
    }
    *LISTED.lock().unwrap_or_else(|e| e.into_inner()) = listed;
    windows
}

fn list_app_windows(pid: i32, windows: &mut Vec<TopWindow>, listed: &mut Vec<Listed>) {
    let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
        return;
    };
    // Not menu bar extras and background helpers.
    if app.activationPolicy() != NSApplicationActivationPolicy::Regular {
        return;
    }
    let Some(exe) = app
        .executableURL()
        .and_then(|url| url.path())
        .map(|path| PathBuf::from(path.to_string()))
    else {
        return;
    };
    let element = unsafe { AXUIElement::new_application(pid) };
    let Some(list) = attribute(&element, "AXWindows").and_then(|v| v.downcast::<CFArray>().ok())
    else {
        return;
    };
    // SAFETY: AXWindows is an array of windows' elements.
    let list: &CFArray<AXUIElement> = unsafe { list.cast_unchecked() };
    for window in list {
        // Not palettes, sheets or dialogs.
        if string_attribute(&window, "AXSubrole").as_deref() != Some("AXStandardWindow") {
            continue;
        }
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        windows.push(TopWindow {
            window: WindowRef(id),
            title: string_attribute(&window, "AXTitle").unwrap_or_default(),
            exe: exe.clone(),
        });
        listed.push(Listed { id, pid, window });
    }
}

/// The ids of the processes with normal windows on screen, front to back.
/// Apps with only minimized windows, or only windows on other Spaces, aren't
/// on screen.
fn apps_front_to_back() -> Vec<i32> {
    let options =
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    let Some(info) = CGWindowListCopyWindowInfo(options, kCGNullWindowID) else {
        return Vec::new();
    };
    // SAFETY: the window list is an array of dictionaries with string keys.
    let info: &CFArray<CFDictionary<CFString, CFType>> = unsafe { info.cast_unchecked() };
    let (layer_key, pid_key): (&CFString, &CFString) =
        unsafe { (kCGWindowLayer, kCGWindowOwnerPID) };
    let mut pids = Vec::new();
    for window in info {
        let number = |key: &CFString| {
            window
                .get(key)
                .and_then(|value| value.downcast::<CFNumber>().ok())
                .and_then(|n| n.as_i32())
        };
        // Layer 0 holds apps' windows; the menu bar and the Dock are above it.
        if number(layer_key) != Some(0) {
            continue;
        }
        if let Some(pid) = number(pid_key)
            && !pids.contains(&pid)
        {
            pids.push(pid);
        }
    }
    pids
}

/// The focused window of the app in front, if `top_windows` listed it.
pub fn foreground_window() -> Option<WindowRef> {
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let element = unsafe { AXUIElement::new_application(app.processIdentifier()) };
    let focused = attribute(&element, "AXFocusedWindow")?
        .downcast::<AXUIElement>()
        .ok()?;
    let listed = LISTED.lock().unwrap_or_else(|e| e.into_inner());
    listed
        .iter()
        .find(|l| *l.window == *focused)
        .map(|l| WindowRef(l.id))
}

fn listed(window: WindowRef) -> Option<(i32, CFRetained<AXUIElement>)> {
    let listed = LISTED.lock().unwrap_or_else(|e| e.into_inner());
    listed
        .iter()
        .find(|l| l.id == window.0)
        .map(|l| (l.pid, l.window.clone()))
}

/// Brings the window to the front, and its app with it.
pub fn focus_window(window: WindowRef) {
    let Some((pid, element)) = listed(window) else {
        return;
    };
    if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
        #[allow(deprecated)]
        app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
    }
    unsafe {
        element.set_attribute_value(&CFString::from_static_str("AXMain"), CFBoolean::new(true));
        element.perform_action(&CFString::from_static_str("AXRaise"));
    }
}

/// Presses the window's close button.
pub fn close_window(window: WindowRef) {
    let Some((_, element)) = listed(window) else {
        return;
    };
    if let Some(button) =
        attribute(&element, "AXCloseButton").and_then(|b| b.downcast::<AXUIElement>().ok())
    {
        unsafe { button.perform_action(&CFString::from_static_str("AXPress")) };
    }
}

/// Whether these modifiers are all held down right now.
pub fn modifiers_held(alt: bool, control: bool, shift: bool, command: bool) -> bool {
    let flags = CGEventSource::flags_state(CGEventSourceStateID::CombinedSessionState);
    (!alt || flags.contains(CGEventFlags::MaskAlternate))
        && (!control || flags.contains(CGEventFlags::MaskControl))
        && (!shift || flags.contains(CGEventFlags::MaskShift))
        && (!command || flags.contains(CGEventFlags::MaskCommand))
}

/// An accessibility attribute's value, e.g. "AXTitle".
fn attribute(element: &AXUIElement, name: &'static str) -> Option<CFRetained<CFType>> {
    let mut value: *const CFType = ptr::null();
    let error = unsafe {
        element.copy_attribute_value(&CFString::from_static_str(name), NonNull::from(&mut value))
    };
    if error != AXError::Success {
        return None;
    }
    // SAFETY: a "Copy" function: the value is ours to release.
    NonNull::new(value.cast_mut()).map(|value| unsafe { CFRetained::from_raw(value) })
}

fn string_attribute(element: &AXUIElement, name: &'static str) -> Option<String> {
    Some(
        attribute(element, name)?
            .downcast::<CFString>()
            .ok()?
            .to_string(),
    )
}
