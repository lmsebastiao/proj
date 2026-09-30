//! User-edited settings (`config.toml`).

use serde::{Deserialize, Deserializer};
use std::{fs, io, path::PathBuf};

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
    /// Look for a new release about once a day (installed copies only).
    pub check_for_updates: bool,
    /// Hold-and-tap shortcut for switching between open editor windows.
    /// Unset = alt + the key left of 1; "" = off.
    pub switch_hotkey: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: vec![default_hotkey().into()],
            editor: None,
            editor_args: Vec::new(),
            scan_dirs: Vec::new(),
            scan_depth: 1,
            check_for_updates: true,
            switch_hotkey: None,
        }
    }
}

impl Config {
    /// The shortcuts for display, e.g. "ctrl+alt+space or alt+p".
    pub fn hotkey_label(&self) -> String {
        self.hotkey.join(" or ")
    }

    /// The window switcher's shortcut, or `None` when it's turned off.
    pub fn switch_hotkey(&self) -> Option<String> {
        match self.switch_hotkey.as_deref().map(str::trim) {
            None => Some(format!("alt+{}", crate::platform::key_left_of_1())),
            Some("") => None,
            Some(text) => Some(text.to_string()),
        }
    }
}

// alt+space is the window menu on Windows and PowerToys' default.
fn default_hotkey() -> &'static str {
    if cfg!(target_os = "macos") {
        "alt+space"
    } else {
        "ctrl+alt+space"
    }
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
            let text = CONFIG_TEMPLATE.replace(
                "{hotkey}",
                &toml::Value::String(default_hotkey().into()).to_string(),
            );
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

# Global shortcut that toggles the launcher, e.g. "alt+space", or a list of
# them: ["ctrl+alt+space", "alt+p"].
# A change applies the next time the launcher opens (with the old shortcut or the tray icon).
hotkey = {hotkey}

# Switch between your open editor windows like Alt+Tab: hold alt, tap the key
# left of 1 to move through them, let go to switch. Set another shortcut here,
# e.g. "alt+q", or "" to turn it off.
# switch_hotkey = "alt+q"

# Optional: folders whose sub-folders are all listed as projects,
# e.g. ['C:\Users\me\repos']. Projects can also be added one by one from the launcher.
scan_dirs = []

# 1 = every direct sub-folder of a scan_dir is a project. Higher values descend
# into folders that are not git repositories, up to this depth.
scan_depth = 1

# Look for a new version on GitHub about once a day. When there is one, the tray
# menu offers "Install update"; nothing is installed without asking.
check_for_updates = true

# Program used to open projects; the project path is appended after editor_args.
# Chosen from the launcher (type > and pick "Change the default editor"). "" opens projects in the file manager.
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
