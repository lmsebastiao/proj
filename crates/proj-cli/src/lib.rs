//! Command-line entry point and OS services shared with the UI.

pub mod autostart;
pub mod cli;
pub mod platform;
pub mod update;

pub use proj_core::fuzzy;
pub use proj_projects::{config, open, paths, store};
