//! Project configuration, persistence, discovery and related operations.

pub mod config;
pub mod discovery;
pub mod editors;
pub mod open;
pub mod paths;
pub mod store;
pub mod tasks;
pub mod templates;

#[cfg(test)]
use proj_core::fuzzy;
pub use proj_git::git;
