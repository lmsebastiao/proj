//! The background app: global hotkey, tray icon, and showing/hiding the palette window.

use std::{
    str::FromStr,
    time::{Duration, Instant, SystemTime},
};

use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{HotKey, Modifiers},
};
use gpui::{
    App, Application, Bounds, Context, Focusable, Global, Window, WindowBackgroundAppearance,
    WindowBounds, WindowHandle, WindowKind, WindowOptions, div, point, prelude::*, px, size,
};

use crate::{
    autostart, config, editors, open,
    palette::{self, Palette},
    platform, store,
    switcher::{self, EditorWindow},
    tray::{Tray, TrayCommand},
    update::{self, Update},
};

/// The open palette, if any, and when it last closed.
#[derive(Default)]
struct PaletteWindow {
    handle: Option<WindowHandle<Palette>>,
    closed_at: Option<Instant>,
}

impl Global for PaletteWindow {}

/// The registered global shortcuts. Opening the palette re-registers them when
/// `hotkey` or `switch_hotkey` in config.toml has changed.
struct Hotkeys {
    manager: GlobalHotKeyManager,
    /// The config's `hotkey` list at the last sync.
    wanted: Vec<String>,
    /// The shortcuts that registered, with their text from the config.
    active: Vec<(String, HotKey)>,
    /// Why some of `wanted` couldn't be registered.
    problems: Vec<String>,
    /// The window switcher's shortcut: its text, and the key forwards and
    /// with shift (backwards).
    switch: Option<(String, HotKey, HotKey)>,
    /// Why the switcher's shortcut couldn't be registered.
    switch_problem: Option<String>,
}

impl Global for Hotkeys {}

impl Hotkeys {
    /// Registers `wanted` and unregisters shortcuts no longer in it, returning a
    /// message for each one that failed. If none of them work, the current ones
    /// stay so proj can still be opened.
    fn sync(&mut self, wanted: &[String]) -> Vec<String> {
        let problems = self.sync_toggles(wanted);
        self.problems.clone_from(&problems);
        problems
    }

    fn sync_toggles(&mut self, wanted: &[String]) -> Vec<String> {
        self.wanted = wanted.to_vec();
        let mut problems = Vec::new();
        let mut next: Vec<(String, HotKey)> = Vec::new();
        for text in wanted {
            let hotkey = match HotKey::from_str(text) {
                Ok(hotkey) => hotkey,
                Err(err) => {
                    problems.push(format!("Shortcut '{text}' is not valid ({err})"));
                    continue;
                }
            };
            if next.iter().any(|(_, h)| *h == hotkey) {
                continue;
            }
            let registered = self.active.iter().any(|(_, h)| *h == hotkey)
                || match self.manager.register(hotkey) {
                    Ok(()) => true,
                    Err(global_hotkey::Error::AlreadyRegistered(_)) => {
                        problems.push(format!("Shortcut '{text}' is taken by another app"));
                        false
                    }
                    Err(err) => {
                        problems.push(format!("Shortcut '{text}': {err}"));
                        false
                    }
                };
            if registered {
                next.push((text.clone(), hotkey));
            }
        }
        if next.is_empty() && !self.active.is_empty() {
            problems.push(format!("still using {}", self.label()));
            return problems;
        }
        for (_, old) in &self.active {
            if !next.iter().any(|(_, h)| h == old) {
                self.manager.unregister(*old).ok();
            }
        }
        self.active = next;
        problems
    }

    /// The working shortcuts, e.g. "ctrl+alt+space or alt+p".
    fn label(&self) -> String {
        let texts: Vec<&str> = self.active.iter().map(|(text, _)| text.as_str()).collect();
        texts.join(" or ")
    }

    /// Registers the switcher's shortcut (`None` = off) if it changed, or failed
    /// last time. Returns whether anything changed.
    fn sync_switch(&mut self, wanted: Option<String>) -> bool {
        if self.switch.as_ref().map(|(text, ..)| text) == wanted.as_ref() {
            return false;
        }
        if let Some((_, next, back)) = self.switch.take() {
            self.manager.unregister(next).ok();
            self.manager.unregister(back).ok();
        }
        self.switch_problem = None;
        let Some(text) = wanted else {
            return true;
        };
        let next = match HotKey::from_str(&text) {
            Ok(hotkey) => hotkey,
            Err(err) => {
                self.switch_problem =
                    Some(format!("Switcher shortcut '{text}' is not valid ({err})"));
                return true;
            }
        };
        match self.manager.register(next) {
            Ok(()) => {
                // Nice to have, like shift+alt+tab; the switcher works without it.
                let back = HotKey::new(Some(next.mods | Modifiers::SHIFT), next.key);
                self.manager.register(back).ok();
                self.switch = Some((text, next, back));
            }
            Err(global_hotkey::Error::AlreadyRegistered(_)) => {
                self.switch_problem = Some(format!(
                    "Switcher shortcut '{text}' is taken by another app"
                ));
            }
            Err(err) => self.switch_problem = Some(format!("Switcher shortcut '{text}': {err}")),
        }
        true
    }

    /// What the palette's footer says about shortcuts that don't work.
    fn notice(&self) -> palette::ShortcutNotice {
        let problems: Vec<&str> = self
            .problems
            .iter()
            .chain(&self.switch_problem)
            .map(String::as_str)
            .collect();
        palette::ShortcutNotice((!problems.is_empty()).then(|| problems.join("; ").into()))
    }
}

/// A switch started with the switcher's shortcut, before its list is shown: a
/// quick tap switches straight away, like Alt+Tab.
struct PendingSwitch {
    windows: Vec<EditorWindow>,
    selected: usize,
}

#[derive(Default)]
struct Switching(Option<PendingSwitch>);

impl Global for Switching {}

/// How long the switcher's modifier must be held before its list shows.
const SHOW_SWITCHER_AFTER: Duration = Duration::from_millis(150);

/// Events from the hotkey and tray callbacks and the update threads, handled on
/// the main thread.
enum Command {
    /// A global shortcut, by its id.
    Hotkey(u32),
    Tray(TrayCommand),
    /// A finished update check; `manual` when it was asked for from the tray.
    UpdateChecked {
        result: Result<Option<Update>, String>,
        manual: bool,
    },
    /// The installer is running (and about to stop proj), or it couldn't start.
    UpdateStarted(Result<(), String>),
}

/// Where the tray's update item is at.
enum UpdateState {
    Unchecked,
    Checking,
    UpToDate,
    Available(Update),
    Installing,
    Failed,
}

impl UpdateState {
    /// The tray item's label and whether it can be clicked.
    fn label(&self) -> (String, bool) {
        match self {
            Self::Unchecked => ("Check for updates".into(), true),
            Self::Checking => ("Checking for updates…".into(), false),
            Self::UpToDate => (format!("proj {} is up to date", update::CURRENT), true),
            Self::Available(update) => (format!("Install update {}", update.version), true),
            Self::Installing => ("Downloading update…".into(), false),
            Self::Failed => ("Update failed, click to retry".into(), true),
        }
    }
}

struct Updates {
    state: UpdateState,
    /// For the check and install threads to report back.
    commands: async_channel::Sender<Command>,
}

impl Global for Updates {}

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
        let mut hotkeys = Hotkeys {
            manager,
            wanted: Vec::new(),
            active: Vec::new(),
            problems: Vec::new(),
            switch: None,
            switch_problem: None,
        };
        let problems = hotkeys.sync(&config.hotkey);
        if hotkeys.active.is_empty() {
            // Nothing registered. Also acts as a single-instance guard.
            return fatal(
                cx,
                &format!(
                    "no shortcut could be registered ({}). Is proj already running?",
                    problems.join("; ")
                ),
            );
        }
        hotkeys.sync_switch(config.switch_hotkey());
        // Some shortcuts may still fail; say which where the user will see it.
        cx.set_global(hotkeys.notice());
        let tooltip = format!("proj ({})", hotkeys.label());
        cx.set_global(hotkeys);
        cx.set_global(Switching::default());

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
                hotkey_tx.try_send(Command::Hotkey(event.id)).ok();
            }
        }));
        let updates = update::is_installed();
        if updates {
            let update_tx = tx.clone();
            std::thread::spawn(move || check_for_updates_daily(&update_tx));
        }
        cx.set_global(Updates {
            state: UpdateState::Unchecked,
            commands: tx.clone(),
        });
        let tray = Tray::new(&tooltip, autostart::is_enabled(), updates, move |command| {
            tx.try_send(Command::Tray(command)).ok();
        })
        .map_err(|err| eprintln!("proj: no tray icon: {err}"))
        .ok();
        cx.spawn(async move |cx| {
            // The icon goes away when dropped, so it lives as long as this task.
            let tray = tray;
            while let Ok(command) = rx.recv().await {
                cx.update(|cx| handle(command, tray.as_ref(), cx)).ok();
            }
        })
        .detach();

        // First run (or editor never chosen): show the setup right away.
        if config.editor.is_none() {
            toggle_palette(None, cx);
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
        Command::Hotkey(id) => {
            let switch = cx.global::<Hotkeys>().switch.as_ref();
            match switch.map(|(_, next, back)| (next.id(), back.id(), next.mods)) {
                Some((next, _, mods)) if id == next => switch_windows(1, mods, cx),
                Some((_, back, mods)) if id == back => switch_windows(-1, mods, cx),
                _ => toggle_palette(tray, cx),
            }
        }
        Command::Tray(TrayCommand::Toggle) => {
            // Clicking the tray icon takes focus from the palette, which closes it
            // just before the click arrives; treat that click as "close".
            let closed_at = cx.global::<PaletteWindow>().closed_at;
            if !closed_at.is_some_and(|at| at.elapsed() < Duration::from_millis(400)) {
                toggle_palette(tray, cx);
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
        Command::Tray(TrayCommand::Update) => {
            let updates = cx.global_mut::<Updates>();
            let tx = updates.commands.clone();
            match &updates.state {
                UpdateState::Checking | UpdateState::Installing => return,
                UpdateState::Available(found) => {
                    let found = found.clone();
                    std::thread::spawn(move || {
                        tx.try_send(Command::UpdateStarted(update::install(&found)))
                            .ok();
                    });
                    updates.state = UpdateState::Installing;
                }
                _ => {
                    std::thread::spawn(move || {
                        let result = update::check();
                        tx.try_send(Command::UpdateChecked {
                            result,
                            manual: true,
                        })
                        .ok();
                    });
                    updates.state = UpdateState::Checking;
                }
            }
            sync_update_item(tray, cx);
        }
        Command::UpdateChecked { result, manual } => {
            let updates = cx.global_mut::<Updates>();
            if matches!(updates.state, UpdateState::Installing) {
                return;
            }
            updates.state = match result {
                Ok(Some(found)) => UpdateState::Available(found),
                Ok(None) => UpdateState::UpToDate,
                Err(err) => {
                    eprintln!("proj: {err}");
                    // Being offline for a background check isn't worth a warning.
                    if !manual {
                        return;
                    }
                    UpdateState::Failed
                }
            };
            sync_update_item(tray, cx);
        }
        Command::UpdateStarted(Ok(())) => cx.quit(),
        Command::UpdateStarted(Err(err)) => {
            eprintln!("proj: {err}");
            cx.global_mut::<Updates>().state = UpdateState::Failed;
            sync_update_item(tray, cx);
        }
    }
}

fn sync_update_item(tray: Option<&Tray>, cx: &App) {
    if let Some(tray) = tray {
        let (label, enabled) = cx.global::<Updates>().state.label();
        tray.set_update(&label, enabled);
    }
}

/// Checks a minute after startup (the network may not be up yet at login), then
/// about once a day, unless `check_for_updates` is off in config.toml. Runs on
/// its own thread; polls hourly rather than sleeping a day, which a laptop's
/// sleep would stretch.
fn check_for_updates_daily(tx: &async_channel::Sender<Command>) {
    const DAY: Duration = Duration::from_secs(24 * 60 * 60);
    std::thread::sleep(Duration::from_secs(60));
    let mut last_check: Option<SystemTime> = None;
    loop {
        // A clock set back (elapsed() fails) counts as due.
        let due = last_check.is_none_or(|at| !at.elapsed().is_ok_and(|elapsed| elapsed < DAY));
        if due && config::load_config().check_for_updates {
            last_check = Some(SystemTime::now());
            let result = update::check();
            if tx
                .try_send(Command::UpdateChecked {
                    result,
                    manual: false,
                })
                .is_err()
            {
                return;
            }
        }
        std::thread::sleep(Duration::from_secs(60 * 60));
    }
}

/// Applies a changed `hotkey` or `switch_hotkey` from config.toml, like the
/// palette does with the rest of the config each time it opens.
fn reload_hotkeys(tray: Option<&Tray>, cx: &mut App) {
    let config = config::load_config();
    let hotkeys = cx.global_mut::<Hotkeys>();
    let mut changed = hotkeys.sync_switch(config.switch_hotkey());
    if hotkeys.wanted != config.hotkey {
        hotkeys.sync(&config.hotkey);
        if let Some(tray) = tray {
            tray.set_tooltip(&format!("proj ({})", hotkeys.label()));
        }
        changed = true;
    }
    if changed {
        let notice = hotkeys.notice();
        cx.set_global(notice);
    }
}

/// The switcher's shortcut: `delta` 1 forwards, -1 with shift. The first press
/// waits a moment: let go quickly and it switches to the previous window
/// straight away; keep holding and the list shows, and each press moves on.
fn switch_windows(delta: isize, mods: Modifiers, cx: &mut App) {
    if let Some(handle) = open_palette(cx) {
        let cycled = handle
            .update(cx, |palette, _, cx| palette.cycle(delta, cx))
            .unwrap_or(false);
        if cycled {
            return;
        }
        // The launcher is open: swap it for the switcher.
        handle
            .update(cx, |_, window, _| window.remove_window())
            .ok();
        cx.global_mut::<PaletteWindow>().handle = None;
    }
    if let Some(pending) = &mut cx.global_mut::<Switching>().0 {
        let len = pending.windows.len().max(1) as isize;
        pending.selected = (pending.selected as isize + delta).rem_euclid(len) as usize;
        return;
    }
    let windows = switcher::editor_windows(&config::load_config(), &store::load_db());
    // Like Alt+Tab, start on the window behind the one you're in.
    let in_front = windows.len() > 1 && Some(windows[0].window) == platform::foreground_window();
    let selected = if delta < 0 {
        windows.len().saturating_sub(1)
    } else {
        usize::from(in_front)
    };
    cx.global_mut::<Switching>().0 = Some(PendingSwitch { windows, selected });

    cx.spawn(async move |cx| {
        let started = Instant::now();
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(15))
                .await;
            let released = !platform::modifiers_held(mods);
            if !released && started.elapsed() < SHOW_SWITCHER_AFTER {
                continue;
            }
            cx.update(|cx| {
                let Some(pending) = cx.global_mut::<Switching>().0.take() else {
                    return;
                };
                if released {
                    if let Some(target) = pending.windows.get(pending.selected) {
                        platform::focus_window(target.window);
                    }
                } else {
                    let PendingSwitch { windows, selected } = pending;
                    show_palette(cx, move |window, cx| {
                        Palette::switcher(window, cx, windows, selected, mods)
                    });
                }
            })
            .ok();
            break;
        }
    })
    .detach();
}

fn toggle_palette(tray: Option<&Tray>, cx: &mut App) {
    if let Some(handle) = open_palette(cx) {
        handle
            .update(cx, |_, window, _| window.remove_window())
            .ok();
        cx.global_mut::<PaletteWindow>().handle = None;
        return;
    }
    reload_hotkeys(tray, cx);
    show_palette(cx, Palette::new);
}

/// Opens the palette window in front, with `build` making its contents.
fn show_palette(
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut Context<Palette>) -> Palette + 'static,
) {
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
    match cx.open_window(options, |window, cx| cx.new(|cx| build(window, cx))) {
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
