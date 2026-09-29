//! Where proj keeps its files, and path helpers.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub(crate) fn app_dir(base: Option<PathBuf>) -> PathBuf {
    base.unwrap_or_else(|| PathBuf::from(".")).join("proj")
}

pub(crate) fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)
}

/// Expands `~` and makes the path absolute (without resolving symlinks or adding `\\?\`).
pub fn normalize(input: &str) -> Option<PathBuf> {
    let input = input.trim().trim_matches('"');
    if input.is_empty() {
        return None;
    }
    let path = match input.strip_prefix('~') {
        Some(rest) => dirs::home_dir()?.join(rest.trim_start_matches(['/', '\\'])),
        None => PathBuf::from(input),
    };
    let path = std::path::absolute(path).ok()?;
    // Drop trailing separators so "C:\repos\x\" and "C:\repos\x" dedupe.
    Some(path.components().collect())
}

/// Replaces the home directory prefix with `~` for display.
pub fn display_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return Path::new("~").join(rest).to_string_lossy().into_owned();
    }
    path.to_string_lossy().into_owned()
}
