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

/// Row titles, and the add / clone row.
pub(super) const FONT_SIZE: f32 = 16.;
/// Subtitles, details, banners, the footer and the shortcuts dropdown.
pub(super) const SMALL_FONT_SIZE: f32 = 13.5;
pub(super) const INPUT_FONT_SIZE: f32 = 19.;
pub(super) const ROW_HEIGHT: f32 = 54.;
pub(super) const FOOTER_HEIGHT: f32 = 34.;
