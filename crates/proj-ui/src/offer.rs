//! Offer to save newly opened local Zed projects without interrupting the palette.

use std::{
    collections::VecDeque,
    path::PathBuf,
    time::{Duration, Instant},
};

use gpui::{
    App, Bounds, Context, Render, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, div, point, prelude::*, px, rgb, size,
};

use crate::{
    config::{self, MonitorSetting, ThemeSetting},
    launcher, platform,
    platform::WindowRef,
    store::{self, Db},
};

const POLL: Duration = Duration::from_secs(1);
const TICK: Duration = Duration::from_millis(200);
const LIFETIME: Duration = Duration::from_secs(7);

#[derive(Clone)]
struct Offer {
    window: WindowRef,
    paths: Vec<PathBuf>,
}

enum Message {
    Found(Offer),
    Add(u64),
    Dismiss(u64),
}

struct Popup {
    offer: Offer,
    handle: WindowHandle<Prompt>,
    id: u64,
    until: Instant,
}

struct Offers {
    pending: VecDeque<Offer>,
    // A changed file title is not a new project; ask again only for different roots.
    seen: Vec<(WindowRef, Vec<PathBuf>)>,
    popup: Option<Popup>,
    next_id: u64,
}

impl Offers {
    fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            seen: Vec::new(),
            popup: None,
            next_id: 0,
        }
    }

    fn enqueue(&mut self, offer: Offer) {
        if self
            .seen
            .iter()
            .any(|(window, paths)| *window == offer.window && same_paths(paths, &offer.paths))
        {
            return;
        }
        self.seen.push((offer.window, offer.paths.clone()));
        // The same workspace can be visible in several windows at once.
        if self
            .pending
            .iter()
            .chain(self.popup.iter().map(|popup| &popup.offer))
            .any(|other| same_paths(&other.paths, &offer.paths))
        {
            return;
        }
        self.pending.push_back(offer);
    }

    fn close(&mut self, cx: &mut App) -> Option<Offer> {
        let popup = self.popup.take()?;
        popup
            .handle
            .update(cx, |_, window, _| window.remove_window())
            .ok();
        Some(popup.offer)
    }

    fn message(&mut self, message: Message, cx: &mut App) {
        match message {
            Message::Found(offer) => self.enqueue(offer),
            Message::Dismiss(id) if self.popup.as_ref().is_some_and(|p| p.id == id) => {
                self.close(cx);
            }
            Message::Add(id) if self.popup.as_ref().is_some_and(|p| p.id == id) => {
                if launcher::palette_is_open(cx) {
                    if let Some(offer) = self.close(cx) {
                        self.pending.push_front(offer);
                    }
                    return;
                }
                let paths = self.popup.as_ref().unwrap().offer.paths.clone();
                let mut db = store::load_db();
                let config = config::load_config();
                if !eligible(&paths, &config, &db) {
                    self.close(cx);
                    return;
                }
                if paths.len() == 1 {
                    store::add_manual(&mut db, paths[0].clone());
                } else {
                    let known = store::collect(&config, &db);
                    for path in &paths {
                        if !known
                            .iter()
                            .any(|project| !project.is_workspace() && project.path == *path)
                        {
                            store::add_manual(&mut db, path.clone());
                        }
                    }
                    store::save_group(&mut db, paths);
                }
                match store::save_db(&db) {
                    Ok(()) => {
                        self.close(cx);
                    }
                    Err(err) => {
                        eprintln!("proj: could not save offered project: {err}");
                        if let Some(popup) = &mut self.popup {
                            popup.until = Instant::now() + LIFETIME;
                            popup
                                .handle
                                .update(cx, |prompt, _, cx| {
                                    prompt.error = Some(format!("Could not save: {err}"));
                                    cx.notify();
                                })
                                .ok();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn tick(&mut self, tx: &async_channel::Sender<Message>, cx: &mut App) {
        if launcher::palette_is_open(cx) {
            if let Some(offer) = self.close(cx) {
                self.pending.push_front(offer);
            }
            return;
        }
        if self.popup.as_ref().is_some_and(|popup| {
            Instant::now() >= popup.until
                || !cx
                    .windows()
                    .iter()
                    .any(|window| window.window_id() == popup.handle.window_id())
        }) {
            self.close(cx);
        }
        if self.popup.is_some() {
            return;
        }
        while let Some(offer) = self.pending.pop_front() {
            // A prompt queued while the palette was open may outlive the editor
            // window (or its former workspace). Do not offer stale paths.
            let still_open = platform::top_windows().into_iter().any(|window| {
                window.window == offer.window
                    && crate::discovery::zed_paths(&window.title, &window.exe)
                        .is_some_and(|paths| same_paths(&paths, &offer.paths))
            });
            if !still_open {
                continue;
            }
            let config = config::load_config();
            let db = store::load_db();
            if !eligible(&offer.paths, &config, &db) {
                continue;
            }
            self.next_id += 1;
            if let Some(handle) = show(&offer, self.next_id, tx.clone(), cx) {
                self.popup = Some(Popup {
                    offer,
                    handle,
                    id: self.next_id,
                    until: Instant::now() + LIFETIME,
                });
                break;
            }
        }
    }
}

fn same_paths(a: &[PathBuf], b: &[PathBuf]) -> bool {
    a.len() == b.len() && a.iter().all(|path| b.contains(path))
}

fn eligible(paths: &[PathBuf], config: &config::Config, db: &Db) -> bool {
    if paths.is_empty()
        || paths.iter().any(|p| !p.is_dir() || db.hidden.contains(p))
        || paths
            .iter()
            .enumerate()
            .any(|(i, p)| paths[..i].contains(p))
    {
        return false;
    }
    if paths.len() > 1 {
        return store::saved_group(db, paths).is_none();
    }
    !store::collect(config, db)
        .iter()
        .any(|project| !project.is_workspace() && project.path == paths[0])
}

#[cfg(windows)]
fn monitor(tx: async_channel::Sender<Message>) {
    // First snapshot is a baseline, not an invitation to import everything open at launch.
    let mut resolved: Vec<(WindowRef, String)> = platform::top_windows()
        .into_iter()
        .map(|window| (window.window, window.title))
        .collect();
    loop {
        std::thread::sleep(POLL);
        if tx.is_closed() {
            break;
        }
        let windows = platform::top_windows();
        resolved.retain(|(id, title)| windows.iter().any(|w| w.window == *id && w.title == *title));
        for window in windows {
            if resolved
                .iter()
                .any(|(id, title)| *id == window.window && *title == window.title)
            {
                continue;
            }
            // Unknown and not-yet-written titles are retried on the next poll.
            if !window
                .exe
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("zed.exe"))
            {
                continue;
            }
            if let Some(paths) = crate::discovery::zed_paths(&window.title, &window.exe)
                && !paths.is_empty()
            {
                resolved.push((window.window, window.title));
                if tx
                    .try_send(Message::Found(Offer {
                        window: window.window,
                        paths,
                    }))
                    .is_err()
                {
                    return;
                }
            }
        }
    }
}

/// Start the window monitor and the foreground prompt queue. macOS window IDs
/// are only valid for the most recent window listing, so offers are Windows-only.
pub fn start(cx: &mut App) {
    #[cfg(windows)]
    {
        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn({
            let tx = tx.clone();
            move || monitor(tx)
        });
        cx.spawn(async move |cx| {
            let mut offers = Offers::new();
            loop {
                while let Ok(message) = rx.try_recv() {
                    cx.update(|cx| offers.message(message, cx)).ok();
                }
                if cx.update(|cx| offers.tick(&tx, cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(TICK).await;
            }
        })
        .detach();
    }
    #[cfg(not(windows))]
    let _ = cx;
}

// The same surface and text colours used by the palette; its theme module is private.
#[derive(Clone, Copy)]
struct Colors {
    bg: u32,
    border: u32,
    text: u32,
    muted: u32,
    hover: u32,
    selected: u32,
    accent: u32,
    danger: u32,
}

const DARK: Colors = Colors {
    bg: 0x1f2023,
    border: 0x34363c,
    text: 0xdcdfe4,
    muted: 0x8b8f98,
    hover: 0x27292e,
    selected: 0x2e333c,
    accent: 0x74ade8,
    danger: 0xe5707a,
};
const LIGHT: Colors = Colors {
    bg: 0xfafafb,
    border: 0xdcdee3,
    text: 0x1f2328,
    muted: 0x666b74,
    hover: 0xf0f1f4,
    selected: 0xe3e8f0,
    accent: 0x2468c4,
    danger: 0xc4314b,
};

struct Prompt {
    offer: Offer,
    id: u64,
    tx: async_channel::Sender<Message>,
    error: Option<String>,
    theme: Colors,
}

impl Render for Prompt {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let label = if self.offer.paths.len() == 1 {
            "Add project?"
        } else {
            "Save workspace?"
        };
        let location = self
            .offer
            .paths
            .iter()
            .map(|path| crate::paths::display_path(path))
            .collect::<Vec<_>>()
            .join(" + ");
        let add = self.tx.clone();
        let dismiss = self.tx.clone();
        let id = self.id;
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(9.))
            .p(px(16.))
            .rounded_lg()
            .border_1()
            .border_color(rgb(t.border))
            .bg(rgb(t.bg))
            .text_color(rgb(t.text))
            .text_size(px(13.5))
            .child(div().text_size(px(16.)).child(label))
            .child(div().truncate().text_color(rgb(t.muted)).child(location))
            .when_some(self.error.clone(), |root, error| {
                root.child(div().text_color(rgb(t.danger)).child(error))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("offer-dismiss")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|d| d.bg(rgb(t.hover)))
                            .on_click(move |_, _, _| {
                                dismiss.try_send(Message::Dismiss(id)).ok();
                            })
                            .child("Dismiss"),
                    )
                    .child(
                        div()
                            .id("offer-add")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(t.selected))
                            .text_color(rgb(t.accent))
                            .cursor_pointer()
                            .hover(|d| d.bg(rgb(t.hover)))
                            .on_click(move |_, _, _| {
                                add.try_send(Message::Add(id)).ok();
                            })
                            .child("Add"),
                    ),
            )
    }
}

fn show(
    offer: &Offer,
    id: u64,
    tx: async_channel::Sender<Message>,
    cx: &mut App,
) -> Option<WindowHandle<Prompt>> {
    let display = platform::launcher_display(MonitorSetting::Focused, cx);
    let dimensions = size(px(370.), px(155.));
    let bounds = match &display {
        Some(display) => {
            let screen = display.bounds();
            Bounds::new(
                point(
                    screen.origin.x + screen.size.width - dimensions.width - px(20.),
                    screen.origin.y + screen.size.height - dimensions.height - px(20.),
                ),
                dimensions,
            )
        }
        None => Bounds::centered(None, dimensions, cx),
    };
    let display_id = display.map(|d| d.id());
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id,
        titlebar: None,
        focus: false,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("proj".into()),
        ..Default::default()
    };
    let offer = offer.clone();
    match cx.open_window(options, |window, cx| {
        let theme = match config::load_config().theme {
            ThemeSetting::Light => LIGHT,
            ThemeSetting::Dark => DARK,
            ThemeSetting::System => match window.appearance() {
                gpui::WindowAppearance::Light | gpui::WindowAppearance::VibrantLight => LIGHT,
                gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark => DARK,
            },
        };
        cx.new(|_| Prompt {
            offer,
            id,
            tx,
            error: None,
            theme,
        })
    }) {
        Ok(handle) => {
            handle
                .update(cx, |_, window, _| {
                    #[cfg(windows)]
                    no_activate(window);
                    if let Some(display_id) = display_id {
                        platform::place(window, display_id, bounds);
                    }
                })
                .ok();
            Some(handle)
        }
        Err(err) => {
            eprintln!("proj: could not show project offer: {err}");
            None
        }
    }
}

#[cfg(windows)]
fn no_activate(window: &Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SetWindowLongPtrW, SetWindowPos, WS_EX_NOACTIVATE,
    };

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as windows_sys::Win32::Foundation::HWND;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_NOACTIVATE as isize);
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_duplicate_and_hidden_paths() {
        let config = config::Config::default();
        let mut db = Db::default();
        let path = std::env::temp_dir();
        assert!(!eligible(&[], &config, &db));
        assert!(!eligible(&[path.clone(), path.clone()], &config, &db));
        assert!(!eligible(&[std::env::current_exe().unwrap()], &config, &db));
        db.hidden.insert(path.clone());
        assert!(!eligible(&[path], &config, &db));
    }

    #[test]
    fn a_dismissed_window_and_duplicate_paths_are_not_queued_again() {
        let mut offers = Offers::new();
        let first = Offer {
            window: WindowRef::test(1),
            paths: vec![std::env::temp_dir()],
        };
        offers.enqueue(first.clone());
        offers.enqueue(first.clone());
        offers.enqueue(Offer {
            window: WindowRef::test(2),
            ..first.clone()
        });
        assert_eq!(offers.pending.len(), 1);
        offers.pending.pop_front(); // dismissed
        offers.enqueue(first.clone());
        assert!(offers.pending.is_empty());
        offers.enqueue(first.clone());
        assert!(offers.pending.is_empty());
        offers.enqueue(Offer {
            paths: vec![std::env::temp_dir().parent().unwrap().to_path_buf()],
            ..first
        });
        assert_eq!(offers.pending.len(), 1);
    }

    #[test]
    fn distinguishes_saved_workspaces_from_known_individual_roots() {
        let config = config::Config::default();
        let base = std::env::temp_dir();
        let one = base.clone();
        let other = base.parent().unwrap().to_path_buf();
        if !other.is_dir() || one == other {
            return;
        }
        let mut db = Db::default();
        store::add_manual(&mut db, one.clone());
        assert!(!eligible(std::slice::from_ref(&one), &config, &db));
        assert!(eligible(&[one.clone(), other.clone()], &config, &db));
        store::save_group(&mut db, vec![other.clone(), one.clone()]);
        assert!(!eligible(&[one, other], &config, &db));
    }
}
