//! Config (user-edited) and project database (app-managed) persistence, plus folder scanning.

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Global shortcut that toggles the launcher, e.g. "alt+space".
    pub hotkey: String,
    /// Program used to open a project. `None` = not chosen yet, "" = file manager.
    pub editor: Option<String>,
    /// Extra arguments passed before the project path.
    pub editor_args: Vec<String>,
    /// Folders whose sub-folders are listed as projects.
    pub scan_dirs: Vec<PathBuf>,
    /// 1 = every direct sub-folder is a project. Higher values descend into
    /// folders that are not git repositories, up to this depth.
    pub scan_depth: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // alt+space is the window menu on Windows and PowerToys' default.
            hotkey: if cfg!(target_os = "macos") { "alt+space" } else { "ctrl+alt+space" }.into(),
            editor: None,
            editor_args: Vec::new(),
            scan_dirs: Vec::new(),
            scan_depth: 1,
        }
    }
}

/// App-managed state: manually added projects, hidden scanned ones, and open history.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Db {
    pub manual: Vec<PathBuf>,
    pub hidden: BTreeSet<PathBuf>,
    /// Project path -> unix seconds of last open.
    pub opened: BTreeMap<String, u64>,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub name: String,
    pub path: PathBuf,
    pub manual: bool,
    pub last_opened: u64,
}

fn app_dir(base: Option<PathBuf>) -> PathBuf {
    base.unwrap_or_else(|| PathBuf::from(".")).join("proj")
}

pub fn config_path() -> PathBuf {
    app_dir(dirs::config_dir()).join("config.toml")
}

pub fn db_path() -> PathBuf {
    app_dir(dirs::data_local_dir()).join("projects.toml")
}

/// Loads the config, writing a commented template on first run.
pub fn load_config() -> Config {
    let path = config_path();
    match fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
            eprintln!("proj: invalid {}: {err}", path.display());
            Config::default()
        }),
        Err(_) => {
            let config = Config::default();
            let text = CONFIG_TEMPLATE.replace("{hotkey}", &toml::Value::String(config.hotkey.clone()).to_string());
            if let Err(err) = write_atomic(&path, &text) {
                eprintln!("proj: could not write {}: {err}", path.display());
            }
            config
        }
    }
}

// `editor` is left unset so the launcher asks for it; `set_editor` appends it
// at the end, right below its comment.
const CONFIG_TEMPLATE: &str = r#"# proj configuration

# Global shortcut that toggles the launcher, e.g. "alt+space", "ctrl+alt+p".
# Changing it requires restarting proj.
hotkey = {hotkey}

# Optional: folders whose sub-folders are all listed as projects,
# e.g. ['C:\Users\me\repos']. Projects can also be added one by one from the launcher.
scan_dirs = []

# 1 = every direct sub-folder of a scan_dir is a project. Higher values descend
# into folders that are not git repositories, up to this depth.
scan_depth = 1

# Program used to open projects; the project path is appended after editor_args.
# Chosen from the launcher (ctrl-e). "" opens projects in the file manager.
# editor_args = ["--new-window"]
"#;

/// Sets `editor` in config.toml, preserving the rest of the file.
pub fn set_editor(command: &str) -> io::Result<()> {
    let path = config_path();
    let text = fs::read_to_string(&path).unwrap_or_default();
    write_atomic(&path, &with_editor(&text, command)?)
}

fn with_editor(text: &str, command: &str) -> io::Result<String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(io::Error::other)?;
    let is_new = !doc.contains_key("editor");
    doc["editor"] = toml_edit::value(command);
    if is_new {
        // The template ends with the editor docs, which toml_edit keeps as trailing
        // text after the new key; move them above it instead.
        let trailing = doc.trailing().as_str().unwrap_or_default().to_string();
        doc.set_trailing("");
        if let Some(mut key) = doc.as_table_mut().key_mut("editor") {
            key.leaf_decor_mut().set_prefix(trailing);
        }
    }
    Ok(doc.to_string())
}

pub fn load_db() -> Db {
    fs::read_to_string(db_path())
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_db(db: &Db) -> io::Result<()> {
    let text = toml::to_string(db).map_err(io::Error::other)?;
    write_atomic(&db_path(), &text)
}

fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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

/// Builds the project list: scanned + manual, minus hidden, most recently opened first.
pub fn collect(config: &Config, db: &Db) -> Vec<Project> {
    let mut seen = HashSet::new();
    let mut projects = Vec::new();
    let mut push = |path: PathBuf, manual: bool| {
        if db.hidden.contains(&path) || !seen.insert(path.clone()) {
            return;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let last_opened = db
            .opened
            .get(path.to_string_lossy().as_ref())
            .copied()
            .unwrap_or(0);
        projects.push(Project {
            name,
            path,
            manual,
            last_opened,
        });
    };

    for path in &db.manual {
        if path.is_dir() {
            push(path.clone(), true);
        }
    }
    let mut scanned = Vec::new();
    for dir in &config.scan_dirs {
        scan(dir, config.scan_depth.max(1), &mut scanned);
    }
    for path in scanned {
        push(path, false);
    }

    projects.sort_by(|a, b| {
        b.last_opened
            .cmp(&a.last_opened)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    projects
}

fn scan(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = match entry.file_type() {
            Ok(ft) if ft.is_symlink() => path.is_dir(),
            Ok(ft) => ft.is_dir(),
            Err(_) => false,
        };
        if !is_dir || entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if depth <= 1 || path.join(".git").exists() {
            out.push(path);
        } else {
            scan(&path, depth - 1, out);
        }
    }
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
    fn editor_is_added_below_its_docs_and_replaced_in_place() {
        let template = CONFIG_TEMPLATE.replace("{hotkey}", "\"ctrl+alt+space\"");
        let added = with_editor(&template, "zed").unwrap();
        assert!(added.ends_with("# editor_args = [\"--new-window\"]\neditor = \"zed\"\n"), "{added}");
        assert_eq!(toml::from_str::<Config>(&added).unwrap().editor.as_deref(), Some("zed"));

        let replaced = with_editor(&added, "code").unwrap();
        assert_eq!(replaced, added.replace("\"zed\"", "\"code\""));
    }
}
