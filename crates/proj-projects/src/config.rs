//! User-edited settings (`config.toml`).

use serde::{Deserialize, Deserializer};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use crate::paths::{app_dir, write_atomic};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Global shortcuts that toggle the launcher, e.g. "alt+space". The file
    /// accepts a single string or a list.
    #[serde(deserialize_with = "one_or_many")]
    pub hotkey: Vec<String>,
    /// Program used to open a project. `None` = not chosen yet, "" = file manager.
    pub editor: Option<String>,
    /// Extra arguments passed before the project path.
    pub editor_args: Vec<String>,
    /// Folders whose sub-folders are listed as projects.
    pub scan_dirs: Vec<PathBuf>,
    /// 1 = every direct sub-folder is a project. Higher values descend into
    /// folders that are not git repositories, up to this depth.
    pub scan_depth: u8,
    /// Optional overrides keyed by a watched folder's path. Other roots inherit scan_depth.
    pub scan_rules: BTreeMap<String, ScanRule>,
    /// Look for a new release about once a day (installed copies only).
    pub check_for_updates: bool,
    /// Hold-and-tap shortcut for switching between open editor windows (with
    /// shift: backwards). Unset = alt+\; "" = off.
    pub switch_hotkey: Option<String>,
    /// Light or dark colours, or follow the system setting.
    #[serde(deserialize_with = "theme_or_system")]
    pub theme: ThemeSetting,
    /// Which screen the launcher opens on.
    #[serde(deserialize_with = "monitor_or_cursor")]
    pub monitor: MonitorSetting,
    /// Git hosts and the software they run ("github", "gitlab", "gitea",
    /// "forgejo", "bitbucket", "azure"), for sites whose name doesn't say.
    pub forges: BTreeMap<String, String>,
    /// Folders and git URLs to start new projects from.
    pub templates: Vec<String>,
}

/// Rules for one watched root. Exclusions match folder components at any level:
/// `build` matches that name exactly, `vendor*` matches names starting with vendor.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ScanRule {
    pub depth: Option<u8>,
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum ThemeSetting {
    /// Follow Windows' light/dark app setting.
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeSetting {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// The next one, for the palette command that goes through them.
    pub fn next(self) -> Self {
        match self {
            Self::System => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::System,
        }
    }
}

/// Which screen the launcher opens on.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum MonitorSetting {
    /// The one with the mouse pointer.
    #[default]
    Cursor,
    /// The one with the window in front (the one being typed in).
    Focused,
    Primary,
}

/// A misspelt monitor falls back to "cursor", like the theme.
fn monitor_or_cursor<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<MonitorSetting, D::Error> {
    Ok(match String::deserialize(deserializer)?.trim() {
        "focused" => MonitorSetting::Focused,
        "primary" => MonitorSetting::Primary,
        _ => MonitorSetting::Cursor,
    })
}

/// A misspelt theme falls back to "system" instead of failing the whole file.
fn theme_or_system<'de, D: Deserializer<'de>>(deserializer: D) -> Result<ThemeSetting, D::Error> {
    Ok(match String::deserialize(deserializer)?.trim() {
        "light" => ThemeSetting::Light,
        "dark" => ThemeSetting::Dark,
        _ => ThemeSetting::System,
    })
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: vec![default_hotkey().into()],
            editor: None,
            editor_args: Vec::new(),
            scan_dirs: Vec::new(),
            scan_depth: 1,
            scan_rules: BTreeMap::new(),
            check_for_updates: true,
            switch_hotkey: None,
            theme: ThemeSetting::System,
            monitor: MonitorSetting::Cursor,
            forges: BTreeMap::new(),
            templates: Vec::new(),
        }
    }
}

impl Config {
    /// Per-root overrides also work when the config uses a different spelling
    /// of the same path (e.g. `~` or a relative path).
    pub fn scan_rule(&self, root: &Path) -> ScanRule {
        self.scan_rules
            .iter()
            .find(|(key, _)| {
                Path::new(key.as_str()) == root
                    || crate::paths::normalize(key).as_deref() == Some(root)
            })
            .map(|(_, rule)| rule.clone())
            .unwrap_or_default()
    }

    pub fn effective_scan_depth(&self, root: &Path) -> u8 {
        self.scan_rule(root).depth.unwrap_or(self.scan_depth).max(1)
    }

    /// The shortcuts for display, e.g. "ctrl+alt+space or alt+p".
    pub fn hotkey_label(&self) -> String {
        self.hotkey.join(" or ")
    }

    /// The window switcher's shortcut, or `None` when it's turned off.
    pub fn switch_hotkey(&self) -> Option<String> {
        shortcut_or(self.switch_hotkey.as_deref(), || "alt+Backslash".into())
    }
}

/// A shortcut from the config: unset = `default`, "" = off.
fn shortcut_or(text: Option<&str>, default: impl FnOnce() -> String) -> Option<String> {
    match text.map(str::trim) {
        None => Some(default()),
        Some("") => None,
        Some(text) => Some(text.to_string()),
    }
}

// The same on every platform. On Windows it takes over the window menu's key,
// and PowerToys Run uses it too; the footer says so when it's taken.
fn default_hotkey() -> &'static str {
    "alt+space"
}

/// `"a"` or `["a", "b"]`. An empty list falls back to the default so the
/// launcher always has a way to be opened.
fn one_or_many<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    let mut list = match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(one) => vec![one],
        OneOrMany::Many(many) => many,
    };
    list.retain(|h| !h.trim().is_empty());
    list.dedup();
    if list.is_empty() {
        list.push(default_hotkey().into());
    }
    Ok(list)
}

pub fn config_path() -> PathBuf {
    app_dir(dirs::config_dir()).join("config.toml")
}

/// Loads the config, writing a commented template on first run. The defaults
/// if it can't be read.
pub fn load_config() -> Config {
    let (config, problem) = load_config_checked();
    if let Some(problem) = problem {
        eprintln!("proj: {problem}");
    }
    config
}

/// The config, and what's wrong with it if it can't be read (then the
/// defaults), for the palette to show: without a console, nobody sees stderr.
pub fn load_config_checked() -> (Config, Option<String>) {
    let path = config_path();
    match fs::read_to_string(&path) {
        Ok(text) => match toml::from_str(&text) {
            Ok(config) => (config, None),
            Err(err) => (salvage(&text), Some(toml_problem(&path, &text, &err))),
        },
        Err(_) => {
            let config = Config::default();
            let text = CONFIG_TEMPLATE.replace(
                "{hotkey}",
                &toml::Value::String(default_hotkey().into()).to_string(),
            );
            if let Err(err) = write_atomic(&path, &text) {
                eprintln!("proj: could not write {}: {err}", path.display());
            }
            (config, None)
        }
    }
}

/// The defaults, but for the editor when the file is TOML with a wrong value
/// elsewhere (`scan_depth = "2"`): projects, and the config file to fix, still
/// open in it.
fn salvage(text: &str) -> Config {
    let mut config = Config::default();
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return config;
    };
    if let Some(editor) = doc.get("editor").and_then(|v| v.as_str()) {
        config.editor = Some(editor.to_string());
        if let Some(args) = doc.get("editor_args").and_then(|v| v.as_array()) {
            config.editor_args = args
                .iter()
                .filter_map(|arg| arg.as_str().map(String::from))
                .collect();
        }
    }
    config
}

/// A TOML error in one line, for the footer: "config.toml, line 12: invalid
/// type: string "2", expected u8".
pub fn toml_problem(path: &Path, text: &str, err: &toml::de::Error) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let message = err
        .message()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    match err.span() {
        Some(span) => {
            let line = text[..span.start.min(text.len())].matches('\n').count() + 1;
            format!("{name}, line {line}: {message}")
        }
        None => format!("{name}: {message}"),
    }
}

// `editor` is left unset so the launcher asks for it; `set_editor` appends it
// at the end, right below its comment.
const CONFIG_TEMPLATE: &str = r#"# proj configuration

# Global shortcut that toggles the launcher, e.g. "alt+space", or a list of
# them: ["ctrl+alt+space", "alt+p"].
# A change applies the next time the launcher opens (with the old shortcut or the tray icon).
hotkey = {hotkey}

# Switch between open editor windows like Alt+Tab: hold alt and tap \ to open
# cards in recent-window order (front to back). Tap again to cycle, hold shift
# to cycle backwards, release alt to switch, or press esc to cancel. While
# holding alt, ctrl+w (or ctrl+alt+w) closes the selected window.
# Set another shortcut here, e.g. "alt+q", or "" to turn it off.
# switch_hotkey = "alt+q"

# Folders whose sub-folders are listed as projects. Choose folders to watch
# from the launcher with > Discover projects in folders, or edit this list.
# You can still add individual projects with ctrl-o.
scan_dirs = []

# 1 = every direct sub-folder of a scan_dir is a project. Higher values descend
# into folders that are not git repositories, up to this depth.
scan_depth = 1

# Optional per-root overrides, keyed by a path from scan_dirs. A missing depth
# inherits scan_depth. Exclusions match folder names at ANY depth: "build" is
# exact, "vendor*" matches names starting with vendor. Hidden folders are always
# skipped. Manually added projects are not affected. Edit via > Manage watched folders.
# [scan_rules.'C:\Users\me\repos']
# depth = 3
# exclude = ["node_modules", "target", "tmp*"]

# Folders and git URLs to start new projects from: type > and pick "New project
# from a template". Copies leave out what git ignores, and start a new git history.
# templates = ['C:\Users\me\templates\rust-cli', "https://github.com/me/web-starter"]

# The pull request and CI actions know GitHub, GitLab, Gitea, Forgejo, Bitbucket
# and Azure DevOps by their site names. Name the software of other hosts here:
# "github", "gitlab", "gitea", "forgejo", "bitbucket" or "azure".
# forges = { "git.example.com" = "gitlab" }

# Look for a new version on GitHub about once a day. When there is one, the tray
# menu offers "Install update"; nothing is installed without asking.
check_for_updates = true

# Colours: "system" follows Windows' light/dark app setting, or "light" / "dark".
# Also changed from the launcher: type > and pick "Theme".
theme = "system"

# The screen the launcher opens on: "cursor" (the one with the mouse pointer),
# "focused" (the one with the window you're typing in) or "primary". Windows
# only: elsewhere it opens on the primary screen.
# monitor = "focused"

# Program used to open projects; the project path is appended after editor_args.
# Chosen from the launcher (type > and pick "Change the default editor").
# "" opens projects in the file manager.
# editor_args = ["--new-window"]
"#;

/// Sets `theme` in config.toml, preserving the rest of the file.
pub fn set_theme(theme: ThemeSetting) -> io::Result<()> {
    let path = config_path();
    let text = fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(io::Error::other)?;
    doc["theme"] = toml_edit::value(theme.as_str());
    write_atomic(&path, &doc.to_string())
}

/// Adds `host = kind` to `forges` in config.toml, preserving the rest of the file.
pub fn set_forge(host: &str, kind: &str) -> io::Result<()> {
    let path = config_path();
    let text = fs::read_to_string(&path).unwrap_or_default();
    write_atomic(&path, &with_forge(&text, host, kind)?)
}

fn with_forge(text: &str, host: &str, kind: &str) -> io::Result<String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(io::Error::other)?;
    match doc.get_mut("forges") {
        // As the template shows it: forges = { "git.example.com" = "gitlab" }
        Some(item) if item.is_inline_table() => {
            if let Some(table) = item.as_inline_table_mut() {
                table.insert(host, kind.into());
            }
        }
        // Or a [forges] table.
        Some(item) if item.is_table() => {
            if let Some(table) = item.as_table_mut() {
                table.insert(host, toml_edit::value(kind));
            }
        }
        Some(_) => return Err(io::Error::other("forges in config.toml isn't a table")),
        None => {
            let mut table = toml_edit::InlineTable::new();
            table.insert(host, kind.into());
            // Under the template's commented-out example, where its docs are.
            let mut line = toml_edit::DocumentMut::new();
            line.insert("forges", toml_edit::value(table.clone()));
            let example = text.find("\n# forges =").and_then(|at| {
                let start = at + 1;
                text[start..].find('\n').map(|end| start + end + 1)
            });
            if let Some(at) = example {
                let with = format!("{}{}{}", &text[..at], line, &text[at..]);
                if with.parse::<toml_edit::DocumentMut>().is_ok() {
                    return Ok(with);
                }
            }
            doc.insert("forges", toml_edit::value(table));
        }
    }
    Ok(doc.to_string())
}

/// Existing conventional project roots, suggested only when they have subfolders.
/// Discovery is local and shallow; watching one still requires a user action.
pub fn suggested_scan_dirs() -> Vec<PathBuf> {
    dirs::home_dir().map_or_else(Vec::new, |home| suggested_scan_dirs_in(&home))
}

fn suggested_scan_dirs_in(home: &Path) -> Vec<PathBuf> {
    [
        "repos",
        "projects",
        "code",
        "dev",
        "src",
        "Documents/GitHub",
    ]
    .into_iter()
    .map(|name| home.join(name))
    .filter(|path| {
        fs::read_dir(path).is_ok_and(|mut entries| {
            entries.any(|entry| entry.is_ok_and(|entry| entry.path().is_dir()))
        })
    })
    .collect()
}

/// Adds folders to the watched scan roots, preserving the rest of config.toml.
/// Already watched folders (including those listed with a different spelling)
/// are not duplicated.
pub fn add_scan_dirs(dirs: &[PathBuf]) -> io::Result<Vec<PathBuf>> {
    let path = config_path();
    let text = fs::read_to_string(&path)?;
    let (updated, added) = with_scan_dirs(&text, dirs)?;
    if !added.is_empty() {
        write_atomic(&path, &updated)?;
    }
    Ok(added)
}

fn with_scan_dirs(text: &str, dirs: &[PathBuf]) -> io::Result<(String, Vec<PathBuf>)> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(io::Error::other)?;
    if !doc.contains_key("scan_dirs") {
        doc["scan_dirs"] = toml_edit::value(toml_edit::Array::new());
    }
    let array = doc["scan_dirs"]
        .as_array_mut()
        .ok_or_else(|| io::Error::other("scan_dirs in config.toml isn't a list of folders"))?;
    let mut seen = std::collections::HashSet::new();
    for value in array.iter() {
        let path = value
            .as_str()
            .ok_or_else(|| io::Error::other("scan_dirs in config.toml isn't a list of folders"))?;
        seen.insert(crate::paths::normalize(path).unwrap_or_else(|| PathBuf::from(path)));
    }
    let mut added = Vec::new();
    for dir in dirs {
        let Some(path) = crate::paths::normalize(&dir.to_string_lossy()).filter(|p| p.is_dir())
        else {
            continue;
        };
        if seen.insert(path.clone()) {
            array.push(path.to_string_lossy().as_ref());
            added.push(path);
        }
    }
    Ok((doc.to_string(), added))
}

/// Stops watching a folder without touching the folder or its contents.
pub fn remove_scan_dir(dir: &Path) -> io::Result<bool> {
    let path = config_path();
    let text = fs::read_to_string(&path)?;
    let (updated, removed) = without_scan_dir(&text, dir)?;
    if removed {
        write_atomic(&path, &updated)?;
    }
    Ok(removed)
}

fn without_scan_dir(text: &str, dir: &Path) -> io::Result<(String, bool)> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(io::Error::other)?;
    let Some(array) = doc
        .get_mut("scan_dirs")
        .and_then(toml_edit::Item::as_array_mut)
    else {
        return Err(io::Error::other(
            "scan_dirs in config.toml isn't a list of folders",
        ));
    };
    let target =
        crate::paths::normalize(&dir.to_string_lossy()).unwrap_or_else(|| dir.to_path_buf());
    let mut removed = false;
    for index in (0..array.len()).rev() {
        let entry = array
            .get(index)
            .and_then(toml_edit::Value::as_str)
            .ok_or_else(|| io::Error::other("scan_dirs in config.toml isn't a list of folders"))?;
        let entry = crate::paths::normalize(entry).unwrap_or_else(|| PathBuf::from(entry));
        if entry == target {
            array.remove(index);
            removed = true;
        }
    }
    if removed
        && let Some(rules) = doc
            .get_mut("scan_rules")
            .and_then(toml_edit::Item::as_table_mut)
    {
        let keys: Vec<String> = rules
            .iter()
            .filter(|(key, _)| {
                crate::paths::normalize(key).unwrap_or_else(|| PathBuf::from(key)) == target
            })
            .map(|(key, _)| key.to_string())
            .collect();
        for key in keys {
            rules.remove(&key);
        }
    }
    Ok((doc.to_string(), removed))
}

/// Persist one root's overrides; an empty rule restores the global defaults.
/// Folder exclusions are exact names or nonempty prefixes followed by `*`.
pub fn set_scan_rule(root: &Path, rule: &ScanRule) -> io::Result<()> {
    let path = config_path();
    let text = fs::read_to_string(&path)?;
    let updated = with_scan_rule(&text, root, rule)?;
    write_atomic(&path, &updated)
}

fn with_scan_rule(text: &str, root: &Path, rule: &ScanRule) -> io::Result<String> {
    if rule.depth == Some(0) {
        return Err(io::Error::other("scan depth must be between 1 and 255"));
    }
    for pattern in &rule.exclude {
        let prefix = pattern.strip_suffix('*').unwrap_or(pattern);
        if prefix.is_empty() || prefix.contains(['/', '\\', '*']) || prefix.starts_with('.') {
            return Err(io::Error::other(format!(
                "Invalid excluded folder: {pattern}"
            )));
        }
    }
    let config: Config = toml::from_str(text).map_err(io::Error::other)?;
    if !config.scan_dirs.iter().any(|dir| dir == root) {
        return Err(io::Error::other("folder is not watched"));
    }
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(io::Error::other)?;
    if !doc.contains_key("scan_rules") {
        doc["scan_rules"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let rules = doc["scan_rules"]
        .as_table_mut()
        .ok_or_else(|| io::Error::other("scan_rules in config.toml isn't a table"))?;
    let key = rules
        .iter()
        .find_map(|(key, _)| {
            (Path::new(key) == root || crate::paths::normalize(key).as_deref() == Some(root))
                .then(|| key.to_string())
        })
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    if rule.depth.is_none() && rule.exclude.is_empty() {
        rules.remove(&key);
    } else if let Some(table) = rules.get_mut(&key).and_then(toml_edit::Item::as_table_mut) {
        if let Some(depth) = rule.depth {
            replace_preserving_decor(table, "depth", toml_edit::value(i64::from(depth)));
        } else {
            table.remove("depth");
        }
        if rule.exclude.is_empty() {
            table.remove("exclude");
        } else {
            let mut array = toml_edit::Array::new();
            for pattern in &rule.exclude {
                array.push(pattern.as_str());
            }
            replace_preserving_decor(table, "exclude", toml_edit::value(array));
        }
    } else {
        let mut inline = rules
            .get(&key)
            .and_then(toml_edit::Item::as_inline_table)
            .cloned()
            .unwrap_or_default();
        if let Some(depth) = rule.depth {
            inline.insert("depth", i64::from(depth).into());
        } else {
            inline.remove("depth");
        }
        if rule.exclude.is_empty() {
            inline.remove("exclude");
        } else {
            let mut array = toml_edit::Array::new();
            for pattern in &rule.exclude {
                array.push(pattern.as_str());
            }
            inline.insert("exclude", array.into());
        }
        replace_preserving_decor(rules, &key, toml_edit::value(inline));
    }
    Ok(doc.to_string())
}

fn replace_preserving_decor(table: &mut toml_edit::Table, key: &str, mut item: toml_edit::Item) {
    if let (Some(old), Some(new)) = (
        table.get(key).and_then(toml_edit::Item::as_value),
        item.as_value_mut(),
    ) {
        *new.decor_mut() = old.decor().clone();
    }
    table.insert(key, item);
}

/// Saves a valid folder or git URL in `templates`, preserving the other settings.
pub fn add_template(source: &str) -> io::Result<bool> {
    let path = config_path();
    let text = fs::read_to_string(&path)?;
    let (updated, added) = with_template(&text, source, false)?;
    if added {
        write_atomic(&path, &updated)?;
    }
    Ok(added)
}

/// Forgets a template without deleting its source folder or repository.
pub fn remove_template(source: &str) -> io::Result<bool> {
    let path = config_path();
    let text = fs::read_to_string(&path)?;
    let (updated, removed) = with_template(&text, source, true)?;
    if removed {
        write_atomic(&path, &updated)?;
    }
    Ok(removed)
}

fn with_template(text: &str, source: &str, remove: bool) -> io::Result<(String, bool)> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(io::Error::other)?;
    let template = crate::templates::Template::parse(source)
        .ok_or_else(|| io::Error::other("Not a valid template folder or git URL"))?;
    if !remove && matches!(&template, crate::templates::Template::Folder(dir) if !dir.is_dir()) {
        return Err(io::Error::other("Template folder does not exist"));
    }
    if !doc.contains_key("templates") {
        if remove {
            return Ok((text.to_string(), false));
        }
        doc["templates"] = toml_edit::value(toml_edit::Array::new());
    }
    let array = doc["templates"]
        .as_array_mut()
        .ok_or_else(|| io::Error::other("templates in config.toml isn't a list"))?;
    let mut matching = None;
    for (index, entry) in array.iter().enumerate() {
        let entry = entry
            .as_str()
            .ok_or_else(|| io::Error::other("templates in config.toml isn't a list"))?;
        if crate::templates::Template::parse(entry) == Some(template.clone()) {
            matching = Some(index);
            break;
        }
    }
    if remove {
        if let Some(index) = matching {
            array.remove(index);
        }
    } else if matching.is_none() {
        array.push(source.trim());
    }
    let changed = if remove {
        matching.is_some()
    } else {
        matching.is_none()
    };
    Ok((doc.to_string(), changed))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_scan_dirs_inherit_global_depth() {
        let config: Config = toml::from_str("scan_dirs = ['repos']\nscan_depth = 4\n").unwrap();
        assert!(config.scan_rules.is_empty());
        assert_eq!(config.effective_scan_depth(Path::new("repos")), 4);
        assert!(config.scan_rule(Path::new("repos")).exclude.is_empty());
    }

    #[test]
    fn scan_rules_round_trip_and_keep_unrelated_comments() {
        let root = std::env::temp_dir().join(format!("proj-rule-config-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let template = CONFIG_TEMPLATE.replace("{hotkey}", "\"alt+space\"");
        let (watched, _) = with_scan_dirs(&template, std::slice::from_ref(&root)).unwrap();
        let rule = ScanRule {
            depth: Some(4),
            exclude: vec!["target".into(), "cache*".into()],
        };
        let edited = with_scan_rule(&watched, &root, &rule).unwrap();
        assert!(edited.contains("# You can still add individual projects with ctrl-o."));
        assert_eq!(
            toml::from_str::<Config>(&edited).unwrap().scan_rule(&root),
            rule
        );
        let with_editor = with_editor(&edited, "zed").unwrap();
        let parsed: Config = toml::from_str(&with_editor).unwrap();
        assert_eq!(parsed.editor.as_deref(), Some("zed"));
        assert_eq!(parsed.scan_rule(&root), rule);
        let (edited, removed) = without_scan_dir(&edited, &root).unwrap();
        assert!(removed);
        assert!(
            toml::from_str::<Config>(&edited)
                .unwrap()
                .scan_rules
                .is_empty()
        );
        let regular = format!(
            "scan_dirs = ['{}']\n# keep me\n[scan_rules.'{}']\ndepth = 2 # depth note\nexclude = ['old']\n",
            root.display(),
            root.display()
        );
        let changed = with_scan_rule(&regular, &root, &rule).unwrap();
        assert!(changed.contains("# keep me"));
        assert!(changed.contains("# depth note"));
        assert_eq!(
            toml::from_str::<Config>(&changed).unwrap().scan_rule(&root),
            rule
        );
        let inline = format!(
            "scan_dirs = ['{}']\n[scan_rules]\n'{}' = {{ depth = 2 }} # note\n",
            root.display(),
            root.display()
        );
        let changed = with_scan_rule(&inline, &root, &rule).unwrap();
        assert!(changed.contains("# note"));
        assert_eq!(
            toml::from_str::<Config>(&changed).unwrap().scan_rule(&root),
            rule
        );
        assert!(
            with_scan_rule(
                &watched,
                &root,
                &ScanRule {
                    depth: Some(0),
                    ..rule.clone()
                }
            )
            .is_err()
        );
        assert!(
            with_scan_rule(
                &watched,
                &root,
                &ScanRule {
                    exclude: vec!["a/b".into()],
                    ..rule
                }
            )
            .is_err()
        );
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn problems_fit_on_one_line() {
        let text = "hotkey = \"f8\"\n\nscan_depth = \"2\"\n";
        let err = toml::from_str::<Config>(text).unwrap_err();
        let problem = toml_problem(Path::new("/x/config.toml"), text, &err);
        assert!(problem.starts_with("config.toml, line 3: "), "{problem}");
        assert!(!problem.contains('\n'), "{problem}");
    }

    #[test]
    fn salvages_the_editor() {
        let config = salvage("scan_depth = \"2\"\neditor = \"zed\"\neditor_args = [\"-n\"]\n");
        assert_eq!(config.editor.as_deref(), Some("zed"));
        assert_eq!(config.editor_args, ["-n"]);
        // Not TOML at all: nothing to go on.
        assert_eq!(salvage("editor = \"zed").editor, None);
    }

    #[test]
    fn editor_is_added_below_its_docs_and_replaced_in_place() {
        let template = CONFIG_TEMPLATE.replace("{hotkey}", "\"ctrl+alt+space\"");
        let added = with_editor(&template, "zed").unwrap();
        assert!(
            added.ends_with("# editor_args = [\"--new-window\"]\neditor = \"zed\"\n"),
            "{added}"
        );
        assert_eq!(
            toml::from_str::<Config>(&added).unwrap().editor.as_deref(),
            Some("zed")
        );

        let replaced = with_editor(&added, "code").unwrap();
        assert_eq!(replaced, added.replace("\"zed\"", "\"code\""));
    }

    #[test]
    fn suggests_only_common_roots_with_subfolders() {
        let home = std::env::temp_dir().join(format!("proj-scan-suggest-{}", std::process::id()));
        let repos = home.join("repos");
        fs::create_dir_all(repos.join("app")).unwrap();
        fs::create_dir(home.join("code")).unwrap();
        assert_eq!(suggested_scan_dirs_in(&home), [repos]);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn scan_dirs_keep_existing_settings_and_skip_duplicates() {
        let root = std::env::temp_dir().join(format!("proj-scan-config-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let template = CONFIG_TEMPLATE.replace("{hotkey}", "\"alt+space\"");
        let (updated, added) = with_scan_dirs(&template, std::slice::from_ref(&root)).unwrap();
        assert_eq!(added, std::slice::from_ref(&root));
        assert!(updated.contains("# You can still add individual projects with ctrl-o."));
        assert_eq!(
            toml::from_str::<Config>(&updated).unwrap().scan_dirs,
            std::slice::from_ref(&root)
        );
        let (same, added) = with_scan_dirs(&updated, &[root.clone(), root.clone()]).unwrap();
        assert!(added.is_empty());
        assert_eq!(same, updated);
        assert!(with_scan_dirs("scan_dirs = \"oops\"", std::slice::from_ref(&root)).is_err());
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn removing_a_watched_root_keeps_other_settings() {
        let root = std::env::temp_dir().join(format!("proj-remove-root-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let template = CONFIG_TEMPLATE.replace("{hotkey}", "\"alt+space\"");
        let (watched, _) = with_scan_dirs(&template, std::slice::from_ref(&root)).unwrap();
        let (updated, removed) = without_scan_dir(&watched, &root).unwrap();
        assert!(removed);
        assert!(root.is_dir());
        assert!(updated.contains("# You can still add individual projects with ctrl-o."));
        assert!(
            toml::from_str::<Config>(&updated)
                .unwrap()
                .scan_dirs
                .is_empty()
        );
        assert!(!without_scan_dir(&updated, &root).unwrap().1);
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn template_edits_preserve_settings_and_reject_duplicates() {
        let root =
            std::env::temp_dir().join(format!("proj-config-template-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let template = CONFIG_TEMPLATE.replace("{hotkey}", "\"alt+space\"");
        let folder = root.to_string_lossy();
        let (added, changed) = with_template(&template, &folder, false).unwrap();
        assert!(changed);
        assert_eq!(
            toml::from_str::<Config>(&added).unwrap().templates,
            [folder.as_ref()]
        );
        assert!(added.contains("check_for_updates = true"));
        assert!(!with_template(&added, &folder, false).unwrap().1);
        let (both, changed) =
            with_template(&added, "https://github.com/me/starter.git", false).unwrap();
        assert!(changed);
        let (removed, changed) = with_template(&both, &folder, true).unwrap();
        assert!(changed);
        assert_eq!(
            toml::from_str::<Config>(&removed).unwrap().templates,
            ["https://github.com/me/starter.git"]
        );
        assert!(root.is_dir());
        assert!(!with_template(&removed, &folder, true).unwrap().1);
        assert!(with_template(&template, "not-a-valid-folder", false).is_err());
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn themes() {
        let parse = |text: &str| toml::from_str::<Config>(text).unwrap().theme;
        assert_eq!(parse(""), ThemeSetting::System);
        assert_eq!(parse(r#"theme = "light""#), ThemeSetting::Light);
        assert_eq!(parse(r#"theme = "dark""#), ThemeSetting::Dark);
        assert_eq!(
            parse(r#"theme = "blue""#),
            ThemeSetting::System,
            "not an error"
        );
        let template = CONFIG_TEMPLATE.replace("{hotkey}", "\"ctrl+alt+space\"");
        assert_eq!(parse(&template), ThemeSetting::System);
    }

    #[test]
    fn forges_are_added_to_the_file() {
        let forges = |text: &str| toml::from_str::<Config>(text).unwrap().forges;
        let template = CONFIG_TEMPLATE.replace("{hotkey}", "\"ctrl+alt+space\"");
        let added = with_forge(&template, "git.example.com", "gitlab").unwrap();
        let line = "forges = { \"git.example.com\" = \"gitlab\" }\n";
        let example = "# forges = { \"git.example.com\" = \"gitlab\" }\n";
        assert!(
            added.contains(&format!("{example}{line}")),
            "under the example:\n{added}"
        );
        assert_eq!(
            added.replacen(&format!("{example}{line}"), example, 1),
            template,
            "the rest stays as it was"
        );
        assert_eq!(forges(&added)["git.example.com"], "gitlab");

        let both = with_forge(&added, "code.example.org", "gitea").unwrap();
        let both = forges(&both);
        assert_eq!(both.len(), 2);
        assert_eq!(both["code.example.org"], "gitea");

        let table = "[forges]\n\"a.example\" = \"gitlab\"\n";
        let added = with_forge(table, "b.example", "forgejo").unwrap();
        assert_eq!(forges(&added)["b.example"], "forgejo");
        assert_eq!(forges(&added)["a.example"], "gitlab");
    }

    #[test]
    fn monitors() {
        let parse = |text: &str| toml::from_str::<Config>(text).unwrap().monitor;
        assert_eq!(parse(""), MonitorSetting::Cursor);
        assert_eq!(parse(r#"monitor = "focused""#), MonitorSetting::Focused);
        assert_eq!(parse(r#"monitor = "primary""#), MonitorSetting::Primary);
        assert_eq!(parse(r#"monitor = "left""#), MonitorSetting::Cursor);
    }

    #[test]
    fn hotkey_accepts_one_or_many() {
        let parse = |text: &str| toml::from_str::<Config>(text).unwrap().hotkey;
        assert_eq!(parse(r#"hotkey = "alt+p""#), ["alt+p"]);
        assert_eq!(
            parse(r#"hotkey = ["ctrl+alt+space", "alt+p", "alt+p"]"#),
            ["ctrl+alt+space", "alt+p"]
        );
        // Missing or empty: the default, so the launcher can always be opened.
        assert_eq!(parse(""), [default_hotkey()]);
        assert_eq!(parse("hotkey = []"), [default_hotkey()]);
        let config = Config {
            hotkey: vec!["a+b".into(), "c+d".into()],
            ..Config::default()
        };
        assert_eq!(config.hotkey_label(), "a+b or c+d");
    }

    #[test]
    fn forges_and_templates_as_the_template_shows_them() {
        let config: Config = toml::from_str(
            r#"
templates = ['C:\Users\me\templates\rust-cli', "https://github.com/me/web-starter"]
forges = { "git.example.com" = "gitlab" }
"#,
        )
        .unwrap();
        assert_eq!(config.templates.len(), 2);
        assert_eq!(config.forges["git.example.com"], "gitlab");
        // Uncommenting them leaves `editor` added later at the top level.
        let template = CONFIG_TEMPLATE
            .replace("{hotkey}", "\"alt+p\"")
            .replace("# forges =", "forges =");
        let added = with_editor(&template, "zed").unwrap();
        let config: Config = toml::from_str(&added).unwrap();
        assert_eq!(config.editor.as_deref(), Some("zed"));
        assert_eq!(config.forges.len(), 1);
    }
}
