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

/// Removes only proj's own config and database files; never touches project folders.
pub fn clear_saved_files(config: &Path, database: &Path) -> io::Result<()> {
    for path in [config, database] {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_removes_only_own_files() {
        let root = std::env::temp_dir().join(format!("proj-reset-{}", std::process::id()));
        let project = root.join("my-project");
        fs::create_dir_all(&project).unwrap();
        let config = root.join("config.toml");
        let database = root.join("projects.toml");
        fs::write(&config, "editor = 'zed'").unwrap();
        fs::write(&database, "manual = []").unwrap();
        fs::write(project.join("README.md"), "keep me").unwrap();
        clear_saved_files(&config, &database).unwrap();
        clear_saved_files(&config, &database).unwrap();
        assert!(!config.exists() && !database.exists());
        assert_eq!(
            fs::read_to_string(project.join("README.md")).unwrap(),
            "keep me"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
