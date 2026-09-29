//! Config (user-edited) and project database (app-managed) persistence, plus folder scanning.

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
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
            hotkey: if cfg!(target_os = "macos") {
                "alt+space"
            } else {
                "ctrl+alt+space"
            }
            .into(),
            editor: None,
            editor_args: Vec::new(),
            scan_dirs: Vec::new(),
            scan_depth: 1,
        }
    }
}

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
        std::iter::once(self.path.clone()).chain(self.extra.iter().cloned()).collect()
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
            let text = CONFIG_TEMPLATE.replace(
                "{hotkey}",
                &toml::Value::String(config.hotkey.clone()).to_string(),
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
    let mut push = |paths: Vec<PathBuf>, name: String, manual: bool| {
        let key = entry_key(&paths);
        if !seen.insert(key.clone()) {
            return;
        }
        let mut paths = paths.into_iter();
        let path = paths.next().expect("at least one folder");
        projects.push(Project {
            name,
            branch: git_branch(&path),
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
            let name = workspace.iter().map(|p| folder_name(p)).collect::<Vec<_>>().join(" + ");
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

/// The repository's git directory: `.git`, or where a worktree/submodule's `.git` file points.
fn git_dir(path: &Path) -> Option<PathBuf> {
    let dot_git = path.join(".git");
    if dot_git.is_file() {
        let text = fs::read_to_string(&dot_git).ok()?;
        Some(path.join(text.strip_prefix("gitdir:")?.trim()))
    } else {
        dot_git.is_dir().then_some(dot_git)
    }
}

/// Current branch (or short commit when detached), read straight from `.git/HEAD`.
fn git_branch(path: &Path) -> Option<String> {
    let head = fs::read_to_string(git_dir(path)?.join("HEAD")).ok()?;
    let head = head.trim();
    Some(match head.strip_prefix("ref: ") {
        Some(reference) => reference
            .strip_prefix("refs/heads/")
            .unwrap_or(reference)
            .to_string(),
        None => head.get(..7)?.to_string(),
    })
}

/// Web page of the repository's `origin` remote (or its first remote), e.g.
/// `https://git.tomiworld.com/web/interactive-v2`.
pub fn git_web_url(path: &Path) -> Option<String> {
    let mut git_dir = git_dir(path)?;
    // Worktrees keep the shared config in the main repository.
    if let Ok(common) = fs::read_to_string(git_dir.join("commondir")) {
        git_dir = git_dir.join(common.trim());
    }
    let config = fs::read_to_string(git_dir.join("config")).ok()?;
    remote_web_url(&remote_url(&config)?)
}

/// `url` of `[remote "origin"]`, else of the first remote, from a git config file.
fn remote_url(config: &str) -> Option<String> {
    let mut section = String::new();
    let mut first = None;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line.to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "url" || !section.starts_with("[remote ") {
            continue;
        }
        let url = value.trim().trim_matches('"').to_string();
        if section == "[remote \"origin\"]" {
            return Some(url);
        }
        first.get_or_insert(url);
    }
    first
}

/// Turns a clone URL (ssh, scp-style or http) into the repository's web page.
fn remote_web_url(url: &str) -> Option<String> {
    let (scheme, host, path) = if let Some((scheme, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        match scheme {
            "http" | "https" => (scheme, host.to_string(), path),
            // The ssh port isn't the web port.
            "ssh" | "git" | "git+ssh" => ("https", host.split(':').next()?.to_string(), path),
            _ => return None,
        }
    } else {
        // scp-like: [user@]host:group/repo.git (not a Windows drive like C:\...)
        let (authority, path) = url.split_once(':')?;
        if authority.len() < 2 || path.starts_with('\\') {
            return None;
        }
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        ("https", host.to_string(), path)
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if host.is_empty() || path.is_empty() {
        return None;
    }
    // Azure DevOps ssh remotes: ssh.dev.azure.com:v3/org/project/repo
    if host == "ssh.dev.azure.com"
        && let Some(["v3", org, project, repo]) = Some(path.split('/').collect::<Vec<_>>().as_slice())
    {
        return Some(format!("https://dev.azure.com/{org}/{project}/_git/{repo}"));
    }
    Some(format!("{scheme}://{host}/{path}"))
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
    fn reads_git_branch() {
        let dir = std::env::temp_dir().join(format!("proj-test-{}", std::process::id()));
        let repo = dir.join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join(".git/HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        assert_eq!(git_branch(&repo).as_deref(), Some("feature/x"));

        fs::write(repo.join(".git/HEAD"), "0123456789abcdef\n").unwrap();
        assert_eq!(git_branch(&repo).as_deref(), Some("0123456"));

        // Worktree: .git is a file pointing elsewhere.
        let worktree = dir.join("wt");
        fs::create_dir_all(dir.join("gitdir")).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(dir.join("gitdir/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", dir.join("gitdir").display()),
        )
        .unwrap();
        assert_eq!(git_branch(&worktree).as_deref(), Some("main"));

        assert_eq!(git_branch(&dir), None);
        fs::remove_dir_all(dir).ok();
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
        assert_eq!(remember_workspace(&mut db, vec![app.clone(), sdk.clone()]), key);
        assert_eq!(db.workspaces.len(), 1, "opening the same pair again reuses it");
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

    #[test]
    fn git_remotes_become_web_urls() {
        let cases = [
            ("ssh://git@git.tomiworld.com:222/web/interactive-v2.git", "https://git.tomiworld.com/web/interactive-v2"),
            ("https://github.com/lmsebastiao/proj.git", "https://github.com/lmsebastiao/proj"),
            ("https://user:token@git.tomiworld.com/tomi/shared-sdk.git", "https://git.tomiworld.com/tomi/shared-sdk"),
            ("git@github.com:owner/repo.git", "https://github.com/owner/repo"),
            ("http://gitea.local:3000/team/app/", "http://gitea.local:3000/team/app"),
            ("git@ssh.dev.azure.com:v3/org/project/repo", "https://dev.azure.com/org/project/_git/repo"),
        ];
        for (remote, web) in cases {
            assert_eq!(remote_web_url(remote).as_deref(), Some(web), "{remote}");
        }
        assert_eq!(remote_web_url(r"C:\repos\local-mirror"), None);
        assert_eq!(remote_web_url("/srv/git/repo.git"), None);
    }

    #[test]
    fn prefers_origin_remote() {
        let config = r#"
[core]
    bare = false
[remote "upstream"]
    url = https://github.com/upstream/repo.git
    fetch = +refs/heads/*:refs/remotes/upstream/*
[remote "origin"]
    url = git@github.com:me/repo.git
"#;
        assert_eq!(remote_url(config).as_deref(), Some("git@github.com:me/repo.git"));
        assert_eq!(
            remote_url("[remote \"fork\"]\n\turl = https://x.com/a/b\n").as_deref(),
            Some("https://x.com/a/b")
        );
        assert_eq!(remote_url("[core]\n\tbare = false\n"), None);
    }
}
