//! System tray icon: left click opens the palette, right click shows a small menu.
//!
//! Windows and macOS only; Linux trays need a GTK main loop, which gpui doesn't run.

/// What the tray asks the launcher to do.
pub enum TrayCommand {
    Toggle,
    ToggleAutostart,
    OpenConfig,
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
        _icon: TrayIcon,
        autostart: CheckMenuItem,
    }

    impl Tray {
        /// Creates the icon. `send` is called from the platform event loop.
        pub fn new(
            tooltip: &str,
            autostart_on: bool,
            send: impl Fn(TrayCommand) + Clone + Send + Sync + 'static,
        ) -> Result<Self, String> {
            let autostart =
                CheckMenuItem::with_id("autostart", "Start on login", true, autostart_on, None);
            let menu = Menu::new();
            menu.append_items(&[
                &MenuItem::with_id("open", "Open proj", true, None),
                &autostart,
                &MenuItem::with_id("config", "Open config file", true, None),
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
                _icon: icon,
                autostart,
            })
        }

        pub fn set_autostart(&self, on: bool) {
            self.autostart.set_checked(on);
        }
    }

    /// A white "P" on a rounded blue square, drawn at 32×32 with 4×4 supersampling.
    fn icon() -> Icon {
        const SIZE: u32 = 32;
        const SAMPLES: u32 = 4;
        let background = |x: f32, y: f32| rounded_rect(x, y, 1.0, 31.0, 7.0);
        let glyph = |x: f32, y: f32| {
            let stem = (10.0..14.5).contains(&x) && (7.0..25.0).contains(&y);
            let bowl_outer = ((10.0..16.0).contains(&x) && (7.0..18.0).contains(&y))
                || (x >= 16.0 && (x - 16.0).powi(2) + (y - 12.5).powi(2) <= 5.5f32.powi(2));
            let bowl_inner = ((14.5..16.0).contains(&x) && (10.5..14.5).contains(&y))
                || (x >= 16.0 && (x - 16.0).powi(2) + (y - 12.5).powi(2) <= 2.0f32.powi(2));
            stem || (bowl_outer && !bowl_inner)
        };
        let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        for py in 0..SIZE {
            for px in 0..SIZE {
                let (mut bg, mut fg) = (0u32, 0u32);
                for sy in 0..SAMPLES {
                    for sx in 0..SAMPLES {
                        let x = px as f32 + (sx as f32 + 0.5) / SAMPLES as f32;
                        let y = py as f32 + (sy as f32 + 0.5) / SAMPLES as f32;
                        if background(x, y) {
                            bg += 1;
                            fg += glyph(x, y) as u32;
                        }
                    }
                }
                let total = (SAMPLES * SAMPLES) as f32;
                // Blend white over the accent colour by glyph coverage.
                let t = if bg == 0 { 0.0 } else { fg as f32 / bg as f32 };
                let mix = |accent: f32| (accent + (255.0 - accent) * t).round() as u8;
                rgba.extend([
                    mix(0x74 as f32),
                    mix(0xad as f32),
                    mix(0xe8 as f32),
                    (bg as f32 / total * 255.0).round() as u8,
                ]);
            }
        }
        Icon::from_rgba(rgba, SIZE, SIZE).expect("valid icon size")
    }

    fn rounded_rect(x: f32, y: f32, min: f32, max: f32, radius: f32) -> bool {
        let cx = x.clamp(min + radius, max - radius);
        let cy = y.clamp(min + radius, max - radius);
        (min..max).contains(&x)
            && (min..max).contains(&y)
            && (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius
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
            _send: impl Fn(TrayCommand) + Clone + Send + Sync + 'static,
        ) -> Result<Self, String> {
            Err("tray icons aren't supported on this platform".into())
        }

        pub fn set_autostart(&self, _on: bool) {}
    }
}
