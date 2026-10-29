//! Background launcher, palette and window-switching UI.

pub mod input;
pub mod launcher;
pub mod offer;
pub mod palette;
pub mod switcher;
pub mod tray;

pub use proj_cli::{autostart, platform, update};
pub use proj_core::fuzzy;
pub use proj_git::git;
pub use proj_projects::{config, discovery, editors, open, paths, store, tasks, templates};
