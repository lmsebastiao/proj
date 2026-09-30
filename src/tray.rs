//! System tray icon: left click opens the palette, right click shows a small menu.
//!
//! Windows and macOS only; Linux trays need a GTK main loop, which gpui doesn't run.

/// What the tray asks the launcher to do.
pub enum TrayCommand {
    Toggle,
    ToggleAutostart,
    OpenConfig,
    /// Check for an update, or install the one found.
    Update,
    Quit,
    /// The mouse is over the icon: refresh menu state (e.g. the start-on-login
    /// check, which the palette can also change) before the menu opens.
    Refresh,
}

pub use imp::Tray;

#[cfg(any(windows, target_os = "macos"))]
mod imp {
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
        menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    };

    use super::TrayCommand;

    pub struct Tray {
        icon: TrayIcon,
        autostart: CheckMenuItem,
        update: Option<MenuItem>,
    }

    impl Tray {
        /// Creates the icon. `send` is called from the platform event loop.
        /// `updates` adds the "Check for updates" item.
        pub fn new(
            tooltip: &str,
            autostart_on: bool,
            updates: bool,
            send: impl Fn(TrayCommand) + Clone + Send + Sync + 'static,
        ) -> Result<Self, String> {
            let autostart =
                CheckMenuItem::with_id("autostart", "Start on login", true, autostart_on, None);
            let update =
                updates.then(|| MenuItem::with_id("update", "Check for updates", true, None));
            let menu = Menu::new();
            menu.append_items(&[
                &MenuItem::with_id("open", "Open proj", true, None),
                &autostart,
                &MenuItem::with_id("config", "Open config file", true, None),
            ])
            .map_err(|err| err.to_string())?;
            if let Some(update) = &update {
                menu.append(update).map_err(|err| err.to_string())?;
            }
            menu.append_items(&[
                &PredefinedMenuItem::separator(),
                &MenuItem::with_id("quit", "Quit", true, None),
            ])
            .map_err(|err| err.to_string())?;

            let icon = TrayIconBuilder::new()
                .with_icon(icon())
                .with_tooltip(tooltip)
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .build()
                .map_err(|err| err.to_string())?;

            let on_menu = send.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let command = match event.id.as_ref() {
                    "open" => TrayCommand::Toggle,
                    "autostart" => TrayCommand::ToggleAutostart,
                    "config" => TrayCommand::OpenConfig,
                    "update" => TrayCommand::Update,
                    "quit" => TrayCommand::Quit,
                    _ => return,
                };
                on_menu(command);
            }));
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| match event {
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } => send(TrayCommand::Toggle),
                TrayIconEvent::Enter { .. } => send(TrayCommand::Refresh),
                _ => {}
            }));

            Ok(Self {
                icon,
                autostart,
                update,
            })
        }

        pub fn set_autostart(&self, on: bool) {
            self.autostart.set_checked(on);
        }

        pub fn set_update(&self, label: &str, enabled: bool) {
            if let Some(update) = &self.update {
                update.set_text(label);
                update.set_enabled(enabled);
            }
        }

        pub fn set_tooltip(&self, tooltip: &str) {
            self.icon.set_tooltip(Some(tooltip)).ok();
        }
    }

    /// The exe's icon (embedded by build.rs) at the tray's size for the current
    /// DPI, so Windows picks the matching image from the .ico instead of scaling one.
    #[cfg(windows)]
    fn icon() -> Icon {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, SM_CXSMICON, SM_CYSMICON,
        };
        let size = unsafe { (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON)) };
        Icon::from_resource(1, Some((size.0 as u32, size.1 as u32))).expect("icon resource")
    }

    /// The icon at 32×32, as written by scripts/make-icon.
    #[cfg(target_os = "macos")]
    fn icon() -> Icon {
        let rgba = include_bytes!("../assets/icon-32.rgba");
        Icon::from_rgba(rgba.to_vec(), 32, 32).expect("valid icon size")
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::TrayCommand;

    pub struct Tray;

    impl Tray {
        pub fn new(
            _tooltip: &str,
            _autostart_on: bool,
            _updates: bool,
            _send: impl Fn(TrayCommand) + Clone + Send + Sync + 'static,
        ) -> Result<Self, String> {
            Err("tray icons aren't supported on this platform".into())
        }

        pub fn set_autostart(&self, _on: bool) {}

        pub fn set_update(&self, _label: &str, _enabled: bool) {}

        pub fn set_tooltip(&self, _tooltip: &str) {}
    }
}
