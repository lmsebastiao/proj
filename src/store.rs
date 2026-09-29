//! The project database (`projects.toml`) and the project list built from it.

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    config::Config,
    git,
    paths::{app_dir, write_atomic},
};

/// App-managed state: manually added projects, hidden scanned ones, and open history.
///
/// `pinned`, `editors` and `opened` are keyed by [`Project::key`]: the folder path for a
/// project, or all folder paths joined with `|` for a workspace.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Db {
    pub manual: Vec<PathBuf>,
    pub hidden: BTreeSet<PathBuf>,
    /// Multi-folder workspaces, remembered when projects are opened together.
    pub workspaces: Vec<Vec<PathBuf>>,
    /// Always listed first.
    pub pinned: BTreeSet<String>,
    /// Entry -> editors offered for it; the first is its default.
    /// Missing = the global editor.
    pub editors: BTreeMap<String, Vec<String>>,
    /// Entry -> unix seconds of last open.
    pub opened: BTreeMap<String, u64>,
}

/// A list entry: a project folder, or a workspace of several folders opened together.
#[derive(Debug, Clone)]
pub struct Project {
    /// Folder name (plus its parent when several projects share it), or
    /// "a + b" for a workspace.
    pub name: String,
    /// The folder, or a workspace's first folder.
    pub path: PathBuf,
    /// A workspace's other folders.
    pub extra: Vec<PathBuf>,
    pub branch: Option<String>,
    pub pinned: bool,
    /// Editors offered for this entry (first = default); empty = the global editor.
    pub editors: Vec<String>,
    pub manual: bool,
    pub last_opened: u64,
}

impl Project {
    pub fn is_workspace(&self) -> bool {
        !self.extra.is_empty()
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        std::iter::once(self.path.clone())
            .chain(self.extra.iter().cloned())
            .collect()
    }

    pub fn key(&self) -> String {
        entry_key(&self.paths())
    }
}

pub fn entry_key(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| p.to_string_lossy())
        .collect::<Vec<_>>()
        .join("|")
}

pub fn db_path() -> PathBuf {
    app_dir(dirs::data_local_dir()).join("projects.toml")
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

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Builds the project list: scanned + manual, minus hidden, most recently opened first.
pub fn collect(config: &Config, db: &Db) -> Vec<Project> {
    let mut seen = HashSet::new();
    let mut projects = Vec::new();
    let mut push = |paths: Vec<PathBuf>, name: String, manual: bool| {
        let key = entry_key(&paths);
        if !seen.insert(key.clone()) {
            return;
        }
        let mut paths = paths.into_iter();
        let path = paths.next().expect("at least one folder");
        projects.push(Project {
            name,
            branch: git::git_branch(&path),
            pinned: db.pinned.contains(&key),
            editors: db.editors.get(&key).cloned().unwrap_or_default(),
            last_opened: db.opened.get(&key).copied().unwrap_or(0),
            path,
            extra: paths.collect(),
            manual,
        });
    };
    let folder_name = |path: &Path| {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned())
    };

    let mut folders: Vec<(PathBuf, bool)> = db
        .manual
        .iter()
        .filter(|p| p.is_dir())
        .map(|p| (p.clone(), true))
        .collect();
    let mut scanned = Vec::new();
    for dir in &config.scan_dirs {
        scan(dir, config.scan_depth.max(1), &mut scanned);
    }
    folders.extend(scanned.into_iter().map(|p| (p, false)));
    for (path, manual) in folders {
        if !db.hidden.contains(&path) {
            push(vec![path.clone()], folder_name(&path), manual);
        }
    }
    for workspace in &db.workspaces {
        if workspace.len() > 1 && workspace.iter().all(|p| p.is_dir()) {
            let name = workspace
                .iter()
                .map(|p| folder_name(p))
                .collect::<Vec<_>>()
                .join(" + ");
            push(workspace.clone(), name, true);
        }
    }

    disambiguate(&mut projects);
    projects.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| b.last_opened.cmp(&a.last_opened))
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

/// Appends the parent folder to names shared by several projects: "app (client)".
fn disambiguate(projects: &mut [Project]) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for project in projects.iter().filter(|p| !p.is_workspace()) {
        *counts.entry(project.name.to_lowercase()).or_default() += 1;
    }
    for project in projects.iter_mut().filter(|p| !p.is_workspace()) {
        if counts[&project.name.to_lowercase()] > 1
            && let Some(parent) = project.path.parent().and_then(Path::file_name)
        {
            project.name = format!("{} ({})", project.name, parent.to_string_lossy());
        }
    }
}

/// Saves a set of folders opened together as a workspace, returning its key.
pub fn remember_workspace(db: &mut Db, paths: Vec<PathBuf>) -> String {
    let key = entry_key(&paths);
    if !db.workspaces.contains(&paths) {
        db.workspaces.push(paths);
    }
    key
}

/// Forgets a workspace and everything keyed by it.
pub fn forget_entry(db: &mut Db, project: &Project) {
    let key = project.key();
    if project.is_workspace() {
        db.workspaces.retain(|w| entry_key(w) != key);
    } else if project.manual {
        db.manual.retain(|p| p != &project.path);
    } else {
        db.hidden.insert(project.path.clone());
    }
    db.pinned.remove(&key);
    db.editors.remove(&key);
    db.opened.remove(&key);
}

/// Adds `editor` to a project's editor list. Starting a list keeps the global
/// editor as the first choice, so the project offers both instead of silently
/// switching away from it.
pub fn offer_editor(list: &mut Vec<String>, editor: &str, global: Option<&str>) {
    if list.iter().any(|c| c == editor) {
        return;
    }
    if list.is_empty()
        && let Some(global) = global.filter(|g| !g.trim().is_empty() && *g != editor)
    {
        list.push(global.to_string());
    }
    list.push(editor.to_string());
}

/// Moves (or inserts) `editor` to the front: the project's default.
pub fn make_default_editor(list: &mut Vec<String>, editor: &str) {
    list.retain(|c| c != editor);
    list.insert(0, editor.to_string());
}

/// "just now", "5m ago", "3h ago", "2d ago", "3w ago", "4mo ago", "1y ago".
pub fn ago(then: u64, now: u64) -> String {
    let secs = now.saturating_sub(then);
    let (value, unit) = match secs {
        0..60 => return "just now".into(),
        60..3_600 => (secs / 60, "m"),
        3_600..86_400 => (secs / 3_600, "h"),
        86_400..604_800 => (secs / 86_400, "d"),
        604_800..2_592_000 => (secs / 604_800, "w"),
        2_592_000..31_536_000 => (secs / 2_592_000, "mo"),
        _ => (secs / 31_536_000, "y"),
    };
    format!("{value}{unit} ago")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_time() {
        assert_eq!(ago(100, 130), "just now");
        assert_eq!(ago(0, 5 * 60), "5m ago");
        assert_eq!(ago(0, 3 * 3_600), "3h ago");
        assert_eq!(ago(0, 2 * 86_400), "2d ago");
        assert_eq!(ago(0, 400 * 86_400), "1y ago");
        assert_eq!(ago(200, 100), "just now");
    }

    fn project(path: &str) -> Project {
        let path = PathBuf::from(path);
        Project {
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            path,
            extra: Vec::new(),
            branch: None,
            pinned: false,
            editors: Vec::new(),
            manual: true,
            last_opened: 0,
        }
    }

    #[test]
    fn duplicate_names_get_parent_folder() {
        let mut projects = [
            project("/a/client/app"),
            project("/a/server/App"),
            project("/a/web"),
        ];
        disambiguate(&mut projects);
        let names: Vec<_> = projects.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["app (client)", "App (server)", "web"]);
    }

    #[test]
    fn project_editor_lists() {
        let mut list = Vec::new();
        offer_editor(&mut list, "devenv", Some("zed"));
        assert_eq!(
            list,
            ["zed", "devenv"],
            "first added editor keeps the global one as default"
        );
        offer_editor(&mut list, "devenv", Some("zed"));
        assert_eq!(list, ["zed", "devenv"], "no duplicates");

        let mut list = Vec::new();
        offer_editor(&mut list, "zed", Some("zed"));
        assert_eq!(list, ["zed"]);

        let mut list = Vec::new();
        make_default_editor(&mut list, "code");
        assert_eq!(
            list,
            ["code"],
            "make default on an empty list is a plain override"
        );
        let mut list = vec!["zed".to_string(), "code".to_string()];
        make_default_editor(&mut list, "code");
        assert_eq!(list, ["code", "zed"]);
    }

    #[test]
    fn workspaces_are_listed_remembered_and_forgotten() {
        let dir = std::env::temp_dir().join(format!("proj-ws-{}", std::process::id()));
        let (app, sdk) = (dir.join("interactive-v2"), dir.join("shared-sdk"));
        fs::create_dir_all(&app).unwrap();
        fs::create_dir_all(&sdk).unwrap();
        let config = Config::default();
        let mut db = Db {
            manual: vec![app.clone(), sdk.clone()],
            ..Db::default()
        };

        let key = remember_workspace(&mut db, vec![app.clone(), sdk.clone()]);
        assert_eq!(
            remember_workspace(&mut db, vec![app.clone(), sdk.clone()]),
            key
        );
        assert_eq!(
            db.workspaces.len(),
            1,
            "opening the same pair again reuses it"
        );
        db.opened.insert(key.clone(), 10);

        let projects = collect(&config, &db);
        assert_eq!(projects.len(), 3);
        let workspace = &projects[0];
        assert_eq!(workspace.name, "interactive-v2 + shared-sdk");
        assert_eq!(workspace.paths(), [app.clone(), sdk.clone()]);
        assert_eq!(workspace.key(), key);
        assert!(workspace.is_workspace());

        // Removing the workspace leaves both projects alone.
        forget_entry(&mut db, workspace);
        assert!(db.workspaces.is_empty() && !db.opened.contains_key(&key));
        assert_eq!(collect(&config, &db).len(), 2);
        fs::remove_dir_all(dir).ok();
    }
}
