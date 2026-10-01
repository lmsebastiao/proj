//! Colours (a dark and a light theme) and sizes.

use gpui::WindowAppearance;

use crate::config::ThemeSetting;

#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: u32,
    pub border: u32,
    pub text: u32,
    pub muted: u32,
    pub placeholder: u32,
    pub branch: u32,
    pub selected: u32,
    pub accent: u32,
    /// Warnings, e.g. the remove icon waiting for its second click.
    pub danger: u32,
}

pub const DARK: Theme = Theme {
    bg: 0x1f2023,
    border: 0x34363c,
    text: 0xdcdfe4,
    muted: 0x8b8f98,
    placeholder: 0x6b6f78,
    branch: 0xb4a0e0,
    selected: 0x2c3038,
    accent: 0x74ade8,
    danger: 0xe5707a,
};

pub const LIGHT: Theme = Theme {
    bg: 0xfafafb,
    border: 0xdcdee3,
    text: 0x1f2328,
    muted: 0x666b74,
    placeholder: 0x9a9fa8,
    branch: 0x7353c4,
    selected: 0xe7eaf0,
    accent: 0x2468c4,
    danger: 0xc4314b,
};

impl Theme {
    /// The theme for a setting, where "system" follows the window's appearance
    /// (the Windows light/dark app setting).
    pub fn for_setting(setting: ThemeSetting, appearance: WindowAppearance) -> Self {
        match setting {
            ThemeSetting::Light => LIGHT,
            ThemeSetting::Dark => DARK,
            ThemeSetting::System => match appearance {
                WindowAppearance::Light | WindowAppearance::VibrantLight => LIGHT,
                WindowAppearance::Dark | WindowAppearance::VibrantDark => DARK,
            },
        }
    }

    pub fn is_dark(&self) -> bool {
        self.bg == DARK.bg
    }
}

/// Icons for the row buttons. Windows 10 and 11 ship the Segoe MDL2 Assets icon
/// font; elsewhere plain symbols stand in.
pub(super) mod icons {
    #[cfg(windows)]
    mod glyphs {
        pub const FONT: &str = "Segoe MDL2 Assets";
        pub const PIN: &str = "\u{E718}";
        pub const PINNED: &str = "\u{E840}";
        pub const RENAME: &str = "\u{E8AC}";
        pub const REMOVE: &str = "\u{E74D}";
        pub const MORE: &str = "\u{E712}";
        pub const CLOSE: &str = "\u{E8BB}";
        // The actions menu.
        pub const OPEN_WITH: &str = "\u{E8A7}";
        pub const FOLDER: &str = "\u{E8B7}";
        pub const TERMINAL: &str = "\u{E756}";
        pub const RUN: &str = "\u{E768}";
        pub const ADD: &str = "\u{E710}";
        pub const TAG: &str = "\u{E8EC}";
        pub const COPY: &str = "\u{E8C8}";
        pub const GLOBE: &str = "\u{E774}";
        pub const PULL_REQUESTS: &str = "\u{E8AB}";
        pub const CI: &str = "\u{E73E}";
        pub const LINK: &str = "\u{E71B}";
        pub const NEW_PROJECT: &str = "\u{E8F4}";
    }
    #[cfg(not(windows))]
    mod glyphs {
        pub const FONT: &str = "";
        pub const PIN: &str = "☆";
        pub const PINNED: &str = "★";
        pub const RENAME: &str = "✎";
        pub const REMOVE: &str = "✕";
        pub const MORE: &str = "⋯";
        pub const CLOSE: &str = "✕";
        pub const OPEN_WITH: &str = "↗";
        pub const FOLDER: &str = "▤";
        pub const TERMINAL: &str = "›";
        pub const RUN: &str = "▶";
        pub const ADD: &str = "+";
        pub const TAG: &str = "#";
        pub const COPY: &str = "⧉";
        pub const GLOBE: &str = "◍";
        pub const PULL_REQUESTS: &str = "⇄";
        pub const CI: &str = "✓";
        pub const LINK: &str = "⛓";
        pub const NEW_PROJECT: &str = "✚";
    }
    pub use glyphs::*;
}

/// Row titles, and the add / clone row.
pub(super) const FONT_SIZE: f32 = 16.;
/// Subtitles, details, banners, the footer and the shortcuts dropdown.
pub(super) const SMALL_FONT_SIZE: f32 = 13.5;
pub(super) const INPUT_FONT_SIZE: f32 = 19.;
pub(super) const ROW_HEIGHT: f32 = 54.;
pub(super) const FOOTER_HEIGHT: f32 = 34.;
