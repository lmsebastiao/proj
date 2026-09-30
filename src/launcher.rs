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
/// `hotkey`, `switch_hotkey` or `switch_search_hotkey` in config.toml changed.
struct Hotkeys {
    manager: GlobalHotKeyManager,
    /// The config's `hotkey` list at the last sync.
    wanted: Vec<String>,
    /// The shortcuts that registered, with their text from the config.
    active: Vec<(String, HotKey)>,
    /// Why some of `wanted` couldn't be registered.
    problems: Vec<String>,
    /// The window switcher's shortcut; with shift it goes backwards.
    switch: SwitchShortcut,
    /// The switcher to search in.
    search: SwitchShortcut,
    /// Straight to a window by its number in the switcher.
    numbers: NumberShortcuts,
}

/// One of the switcher's shortcuts, from its text in the config.
#[derive(Default)]
struct SwitchShortcut {
    /// Registered: its text, the key, and the key with shift when that's wanted.
    active: Option<(String, HotKey, Option<HotKey>)>,
    /// Why it couldn't be registered.
    problem: Option<String>,
}

impl SwitchShortcut {
    /// Registers `wanted` (`None` = off) if it changed, or failed last time.
    /// `shifted` also registers it with shift, if that's free. Returns whether
    /// anything changed.
    fn sync(
        &mut self,
        manager: &GlobalHotKeyManager,
        wanted: Option<String>,
        what: &str,
        shifted: bool,
    ) -> bool {
        if self.active.as_ref().map(|(text, ..)| text) == wanted.as_ref() {
            return false;
        }
        if let Some((_, key, shifted)) = self.active.take() {
            manager.unregister(key).ok();
            if let Some(shifted) = shifted {
                manager.unregister(shifted).ok();
            }
        }
        self.problem = None;
        let Some(text) = wanted else {
            return true;
        };
        let key = match HotKey::from_str(&text) {
            Ok(hotkey) => hotkey,
            Err(err) => {
                self.problem = Some(format!("{what} shortcut '{text}' is not valid ({err})"));
                return true;
            }
        };
        match manager.register(key) {
            Ok(()) => {
                let shifted = shifted
                    .then(|| HotKey::new(Some(key.mods | Modifiers::SHIFT), key.key))
                    .filter(|shifted| manager.register(*shifted).is_ok());
                self.active = Some((text, key, shifted));
            }
            Err(global_hotkey::Error::AlreadyRegistered(_)) => {
                self.problem = Some(format!("{what} shortcut '{text}' is taken by another app"));
            }
            Err(err) => self.problem = Some(format!("{what} shortcut '{text}': {err}")),
        }
        true
    }
}

/// The shortcuts that switch to a window by its number: the config's
/// modifiers with 1 to 9.
#[derive(Default)]
struct NumberShortcuts {
    /// Registered: the modifiers' text, and the key for each number (index 0
    /// is 1), `None` where it couldn't be registered.
    active: Option<(String, Vec<Option<HotKey>>)>,
    /// Why some couldn't be registered.
    problem: Option<String>,
}

impl NumberShortcuts {
    /// Registers `wanted` (`None` = off) if it changed, or failed last time.
    /// Returns whether anything changed.
    fn sync(&mut self, manager: &GlobalHotKeyManager, wanted: Option<String>) -> bool {
        if self.active.as_ref().map(|(text, _)| text) == wanted.as_ref() {
            return false;
        }
        if let Some((_, keys)) = self.active.take() {
            for key in keys.into_iter().flatten() {
                manager.unregister(key).ok();
            }
        }
        self.problem = None;
        let Some(mods) = wanted else {
            return true;
        };
        let mut keys = Vec::new();
        let mut failed = Vec::new();
        for n in 1..=9 {
            let key = match HotKey::from_str(&format!("{mods}+{n}")) {
                Ok(key) => key,
                Err(err) => {
                    self.problem = Some(format!(
                        "Switch by number: '{mods}' is not a valid set of modifiers ({err})"
                    ));
                    return true;
                }
            };
            let registered = manager.register(key).is_ok();
            if !registered {
                failed.push(n.to_string());
            }
            keys.push(registered.then_some(key));
        }
        if !failed.is_empty() {
            self.problem = Some(format!(
                "Shortcut {mods}+{} is taken by another app",
                failed.join("/")
            ));
        }
        self.active = Some((mods, keys));
        true
    }

    /// The window (0 = the first) that the shortcut `id` switches to.
    fn window_for(&self, id: u32) -> Option<usize> {
        let (_, keys) = self.active.as_ref()?;
        keys.iter()
            .position(|key| key.is_some_and(|key| key.id() == id))
    }
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

    /// Registers the switcher's shortcuts from the config if they changed, or
    /// failed last time. Returns whether anything changed.
    fn sync_switcher(&mut self, config: &config::Config) -> bool {
        let switch = self
            .switch
            .sync(&self.manager, config.switch_hotkey(), "Switcher", true);
        let search = self.search.sync(
            &self.manager,
            config.switch_search_hotkey(),
            "Switcher search",
            false,
        );
        let numbers = self
            .numbers
            .sync(&self.manager, config.switch_number_modifiers());
        switch || search || numbers
    }

    /// What the palette's footer says about shortcuts that don't work.
    fn notice(&self) -> palette::ShortcutNotice {
        let problems: Vec<&str> = self
            .problems
            .iter()
            .chain(&self.switch.problem)
            .chain(&self.search.problem)
            .chain(&self.numbers.problem)
            .map(String::as_str)
            .collect();
        palette::ShortcutNotice((!problems.is_empty()).then(|| problems.join("; ").into()))
    }
}

#[derive(Default)]
struct Switching {
    /// The switcher's order: windows in the order they were first listed. It
    /// doesn't change when you switch, so each window keeps its place.
    order: Vec<platform::WindowRef>,
}

impl Global for Switching {}

impl Switching {
    /// `windows` (front to back) in the switcher's order, and which one to start
    /// on. Forwards (`delta` 1): the editor window used before the one in front,
    /// so a quick tap goes back to it, like Alt+Tab. Backwards: the one above the
    /// window in front.
    fn arrange(
        &mut self,
        mut windows: Vec<EditorWindow>,
        delta: isize,
    ) -> (Vec<EditorWindow>, usize) {
        let front = windows
            .first()
            .map(|w| w.window)
            .filter(|w| Some(*w) == platform::foreground_window());
        let previous = windows.get(usize::from(front.is_some())).map(|w| w.window);
        // Forget closed windows; new ones go at the end.
        self.order
            .retain(|known| windows.iter().any(|w| w.window == *known));
        for window in &windows {
            if !self.order.contains(&window.window) {
                self.order.push(window.window);
            }
        }
        windows.sort_by_key(|w| self.order.iter().position(|known| *known == w.window));
        let position = |window: Option<platform::WindowRef>| {
            windows.iter().position(|w| Some(w.window) == window)
        };
        let selected = if delta < 0 {
            let len = windows.len();
            position(front).map_or(len.saturating_sub(1), |at| (at + len - 1) % len)
        } else {
            position(previous).unwrap_or(0)
        };
        (windows, selected)
    }
}

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

/// Where updating is at, shown by the tray item and the palette's `>` command.
#[derive(Clone)]
pub enum UpdateState {
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

/// Updating, for installed copies. The palette observes this to redraw its
/// update command.
pub struct Updates {
    pub state: UpdateState,
    /// For the check and install threads to report back.
    commands: async_channel::Sender<Command>,
}

impl Global for Updates {}

/// The palette's update command: check for an update, or install the one found,
/// like the tray item (which it keeps in step).
pub fn run_update(cx: &App) {
    if let Some(updates) = cx.try_global::<Updates>() {
        updates
            .commands
            .try_send(Command::Tray(TrayCommand::Update))
            .ok();
    }
}

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
            switch: SwitchShortcut::default(),
            search: SwitchShortcut::default(),
            numbers: NumberShortcuts::default(),
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
        hotkeys.sync_switcher(&config);
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
            let hotkeys = cx.global::<Hotkeys>();
            if let Some(ix) = hotkeys.numbers.window_for(id) {
                return switch_to_number(ix, cx);
            }
            // The switcher stays up while its own modifiers (without shift) are held.
            let switch = hotkeys
                .switch
                .active
                .as_ref()
                .map(|(_, key, back)| (key.id(), back.map(|b| b.id()), key.mods));
            let search = hotkeys.search.active.as_ref().map(|(_, key, _)| key.id());
            match switch {
                Some((next, _, mods)) if id == next => switch_windows(1, mods, cx),
                Some((_, back, mods)) if Some(id) == back => switch_windows(-1, mods, cx),
                _ if Some(id) == search => search_windows(cx),
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

/// Applies changed shortcuts from config.toml, like the palette does with the
/// rest of the config each time it opens.
fn reload_hotkeys(tray: Option<&Tray>, cx: &mut App) {
    let config = config::load_config();
    let hotkeys = cx.global_mut::<Hotkeys>();
    let mut changed = hotkeys.sync_switcher(&config);
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
/// shows the list straight away, like Alt+Tab, and letting go switches, however
/// quick the tap; each further press moves the selection.
fn switch_windows(delta: isize, mods: Modifiers, cx: &mut App) {
    if let Some(handle) = open_palette(cx) {
        let cycled = handle
            .update(cx, |palette, _, cx| palette.cycle(delta, cx))
            .unwrap_or(false);
        if cycled {
            return;
        }
        close_palette(handle, cx);
    }
    let (windows, selected) = arranged_windows(delta, cx);
    show_palette(cx, move |window, cx| {
        Palette::switcher(window, cx, windows, selected, Some(mods))
    });
}

/// The open editor windows in the switcher's order, and the one to start on
/// (see `Switching::arrange`).
fn arranged_windows(delta: isize, cx: &mut App) -> (Vec<EditorWindow>, usize) {
    let windows = switcher::editor_windows(&config::load_config(), &store::load_db());
    cx.global_mut::<Switching>().arrange(windows, delta)
}

/// A switch-by-number shortcut: straight to the `ix`th window (0 = the first)
/// in the switcher's order, without showing the list.
fn switch_to_number(ix: usize, cx: &mut App) {
    let (windows, _) = arranged_windows(1, cx);
    let Some(target) = windows.get(ix) else {
        return;
    };
    // Before closing an open palette, while proj is still in front and allowed
    // to hand over focus.
    platform::focus_window(target.window);
    if let Some(handle) = open_palette(cx) {
        close_palette(handle, cx);
    }
    // Recently used, like switching from the list.
    let config = config::load_config();
    let mut db = store::load_db();
    let projects = store::collect(&config, &db);
    if let Some(project) = target.project(&projects) {
        db.opened.insert(projects[project].key(), store::now());
        if let Err(err) = store::save_db(&db) {
            eprintln!("proj: could not save: {err}");
        }
    }
}

/// The switcher's search shortcut: the same list, but it stays
/// open to type in until enter or esc. From a switcher already showing, it
/// just stops waiting for the modifier to be let go.
fn search_windows(cx: &mut App) {
    if let Some(handle) = open_palette(cx) {
        let searching = handle
            .update(cx, |palette, _, cx| palette.start_search(cx))
            .unwrap_or(false);
        if searching {
            return;
        }
        close_palette(handle, cx);
    }
    let (windows, selected) = arranged_windows(1, cx);
    show_palette(cx, move |window, cx| {
        Palette::switcher(window, cx, windows, selected, None)
    });
}

fn close_palette(handle: WindowHandle<Palette>, cx: &mut App) {
    handle
        .update(cx, |_, window, _| window.remove_window())
        .ok();
    cx.global_mut::<PaletteWindow>().handle = None;
}

fn toggle_palette(tray: Option<&Tray>, cx: &mut App) {
    if let Some(handle) = open_palette(cx) {
        close_palette(handle, cx);
        return;
    }
    reload_hotkeys(tray, cx);
    // In the switcher's order, for the `@` list and its numbers.
    let (windows, _) = arranged_windows(1, cx);
    show_palette(cx, move |window, cx| Palette::new(window, cx, windows));
}

/// Opens the palette window in front, with `build` making its contents.
fn show_palette(
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut Context<Palette>) -> Palette + 'static,
) {
    let display = platform::launcher_display(cx);

    let window_size = size(px(720.), px(500.));
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
