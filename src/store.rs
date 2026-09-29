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
    /// Program used to open a project. Empty = system file manager.
    pub editor: String,
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
            editor: String::new(),
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

/// Loads the config, writing a commented default on first run.
pub fn load_config() -> Config {
    let path = config_path();
    match fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
            eprintln!("proj: invalid {}: {err}", path.display());
            Config::default()
        }),
        Err(_) => {
            let config = first_run_config();
            if let Err(err) = write_config(&path, &config) {
                eprintln!("proj: could not write {}: {err}", path.display());
            }
            config
        }
    }
}

fn first_run_config() -> Config {
    let home = dirs::home_dir().unwrap_or_default();
    let candidates = ["repos", "Projects", "projects", "dev", "code", "src", "git"];
    let mut scan_dirs: Vec<PathBuf> = candidates
        .iter()
        .map(|name| home.join(name))
        .filter(|dir| dir.is_dir())
        .collect();
    if scan_dirs.is_empty() {
        scan_dirs.push(home.join("repos"));
    }
    let editor = ["zed", "code", "subl"]
        .into_iter()
        .find(|name| crate::open::which(name).is_some())
        .unwrap_or("")
        .to_string();
    Config {
        editor,
        scan_dirs,
        ..Config::default()
    }
}

fn write_config(path: &Path, config: &Config) -> io::Result<()> {
    let string = |s: &str| toml::Value::String(s.into()).to_string();
    let array = |items: Vec<String>| {
        toml::Value::Array(items.into_iter().map(toml::Value::String).collect()).to_string()
    };
    let text = format!(
        r#"# proj configuration

# Global shortcut that toggles the launcher, e.g. "alt+space", "ctrl+alt+p".
# Changing it requires restarting proj.
hotkey = {hotkey}

# Program used to open a project; the project path is appended after editor_args.
# Leave empty to open projects in the system file manager.
editor = {editor}
editor_args = {editor_args}

# Folders whose sub-folders are listed as projects.
scan_dirs = {scan_dirs}

# 1 = every direct sub-folder is a project. Higher values descend into
# folders that are not git repositories, up to this depth.
scan_depth = {scan_depth}
"#,
        hotkey = string(&config.hotkey),
        editor = string(&config.editor),
        editor_args = array(config.editor_args.clone()),
        scan_dirs = array(
            config
                .scan_dirs
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        ),
        scan_depth = config.scan_depth,
    );
    write_atomic(path, &text)
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
