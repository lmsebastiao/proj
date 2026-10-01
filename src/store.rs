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
    paths::{app_dir, display_path, write_atomic},
};

/// App-managed state: manually added projects, hidden scanned ones, and open history.
///
/// `names`, `editors`, `opened`, `tags` and `commands` are keyed by
/// [`Project::key`]: the folder path for a project, or all folder paths joined
/// with `|` for a workspace.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Db {
    pub manual: Vec<PathBuf>,
    pub hidden: BTreeSet<PathBuf>,
    /// Multi-folder workspaces, remembered when projects are opened together.
    pub workspaces: Vec<Vec<PathBuf>>,
    /// Entry -> name given with F2, shown instead of the folder name(s).
    pub names: BTreeMap<String, String>,
    /// Entry -> its own default editor. Missing = the global editor.
    ///
    /// Stored as a one-item list: older versions kept a list of editors here
    /// (and reset the whole file if they can't read it). Only the first is used.
    pub editors: BTreeMap<String, Vec<String>>,
    /// Entry -> unix seconds of last open.
    pub opened: BTreeMap<String, u64>,
    /// Entry -> its tags, lowercase, searched with `#tag`.
    pub tags: BTreeMap<String, BTreeSet<String>>,
    /// Entry -> commands added to its actions menu, run in a terminal there.
    pub commands: BTreeMap<String, Vec<String>>,
}

/// A list entry: a project folder, or a workspace of several folders opened together.
#[derive(Debug, Clone, Default)]
pub struct Project {
    /// Folder name (plus its parent when several projects share it), or
    /// "a + b" for a workspace.
    pub name: String,
    /// The folder, or a workspace's first folder.
    pub path: PathBuf,
    /// A workspace's other folders.
    pub extra: Vec<PathBuf>,
    pub branch: Option<String>,
    /// This entry's own default editor; `None` = the global one.
    pub editor: Option<String>,
    pub manual: bool,
    pub last_opened: u64,
    /// Its folder (or one of a workspace's) is gone: moved, deleted, or on a
    /// drive that isn't connected.
    pub missing: bool,
    /// Sorted, lowercase.
    pub tags: Vec<String>,
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

    /// The folders for display: "~/repos/app", or "~/repos/a + ~/repos/b".
    pub fn location(&self) -> String {
        self.paths()
            .iter()
            .map(|p| display_path(p))
            .collect::<Vec<_>>()
            .join(" + ")
    }

    /// Search bonus so entries opened before win ties.
    pub fn search_boost(&self) -> i32 {
        i32::from(self.last_opened > 0) * 8
    }

    /// Whether it has a tag starting with each of `prefixes` (lowercase).
    pub fn has_tags(&self, prefixes: &[String]) -> bool {
        prefixes
            .iter()
            .all(|prefix| self.tags.iter().any(|tag| tag.starts_with(prefix.as_str())))
    }

    /// The editor Enter uses: the entry's own default, else the global one.
    pub fn default_editor(&self, config: &Config) -> String {
        self.editor
            .as_ref()
            .or(config.editor.as_ref())
            .cloned()
            .unwrap_or_default()
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

/// Builds the project list: scanned + manual, minus hidden, most recently
/// opened first. Added projects whose folder is gone come last, marked missing.
pub fn collect(config: &Config, db: &Db) -> Vec<Project> {
    let mut seen = HashSet::new();
    let mut projects = Vec::new();
    let mut push = |paths: Vec<PathBuf>, name: String, manual: bool| {
        let key = entry_key(&paths);
        if !seen.insert(key.clone()) {
            return;
        }
        let missing = !paths.iter().all(|p| p.is_dir());
        let mut paths = paths.into_iter();
        let path = paths.next().expect("at least one folder");
        projects.push(Project {
            name,
            branch: (!missing).then(|| git::git_branch(&path)).flatten(),
            editor: db.editors.get(&key).and_then(|list| list.first()).cloned(),
            last_opened: db.opened.get(&key).copied().unwrap_or(0),
            tags: db
                .tags
                .get(&key)
                .map(|tags| tags.iter().cloned().collect())
                .unwrap_or_default(),
            path,
            extra: paths.collect(),
            manual,
            missing,
        });
    };
    let folder_name = |path: &Path| {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned())
    };

    let mut folders: Vec<(PathBuf, bool)> = db.manual.iter().map(|p| (p.clone(), true)).collect();
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
        if workspace.len() > 1 {
            let name = workspace
                .iter()
                .map(|p| folder_name(p))
                .collect::<Vec<_>>()
                .join(" + ");
            push(workspace.clone(), name, true);
        }
    }

    disambiguate(&mut projects);
    for project in &mut projects {
        if let Some(name) = db.names.get(&project.key()) {
            project.name.clone_from(name);
        }
    }
    projects.sort_by(|a, b| {
        a.missing
            .cmp(&b.missing)
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

/// Lists `path` as a manually added project, un-hiding it if it was hidden.
pub fn add_manual(db: &mut Db, path: PathBuf) {
    db.hidden.remove(&path);
    if !db.manual.contains(&path) {
        db.manual.push(path);
    }
}

/// Names an entry; an empty name goes back to the folder name(s).
pub fn rename(db: &mut Db, key: &str, name: &str) {
    let name = name.trim();
    if name.is_empty() {
        db.names.remove(key);
    } else {
        db.names.insert(key.to_string(), name.to_string());
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
    db.names.remove(&key);
    db.editors.remove(&key);
    db.opened.remove(&key);
    db.tags.remove(&key);
    db.commands.remove(&key);
}

/// Sets an entry's tags from text like "work, #oss client-x": lowercase, without
/// the `#`. Empty text removes them all.
pub fn set_tags(db: &mut Db, key: &str, text: &str) {
    let tags: BTreeSet<String> = text
        .split(|c: char| c.is_whitespace() || c == ',')
        .map(|tag| tag.trim_start_matches('#').to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect();
    if tags.is_empty() {
        db.tags.remove(key);
    } else {
        db.tags.insert(key.to_string(), tags);
    }
}

/// Adds a command to an entry's actions menu, unless it's there already.
pub fn add_command(db: &mut Db, key: &str, command: &str) {
    let command = command.trim();
    if command.is_empty() {
        return;
    }
    let commands = db.commands.entry(key.to_string()).or_default();
    if !commands.iter().any(|c| c == command) {
        commands.push(command.to_string());
    }
}

pub fn remove_command(db: &mut Db, key: &str, command: &str) {
    if let Some(commands) = db.commands.get_mut(key) {
        commands.retain(|c| c != command);
        if commands.is_empty() {
            db.commands.remove(key);
        }
    }
}

/// Sets an entry's own default editor, or with `None` goes back to the global one.
pub fn set_editor(db: &mut Db, key: &str, editor: Option<String>) {
    match editor {
        Some(editor) => db.editors.insert(key.to_string(), vec![editor]),
        None => db.editors.remove(key),
    };
}

/// "Tue 29 Sep, 14:12" (with the year when it isn't this one), in local time
/// where the system says what that is, else UTC.
pub fn date_of(then: u64) -> String {
    use time::{OffsetDateTime, UtcOffset};
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let local = |secs: u64| {
        let secs = i64::try_from(secs).ok()?;
        Some(
            OffsetDateTime::from_unix_timestamp(secs)
                .ok()?
                .to_offset(offset),
        )
    };
    let (Some(date), Some(today)) = (local(then), local(now())) else {
        return String::new();
    };
    format_date(date, today.year())
}

fn format_date(date: time::OffsetDateTime, this_year: i32) -> String {
    let weekday = &format!("{}", date.weekday())[..3];
    let month = &format!("{}", date.month())[..3];
    let year = if date.year() == this_year {
        String::new()
    } else {
        format!(" {}", date.year())
    };
    format!(
        "{weekday} {} {month}{year}, {:02}:{:02}",
        date.day(),
        date.hour(),
        date.minute()
    )
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
    fn dates() {
        // 2026-09-29 14:12 UTC, a Tuesday.
        let date = time::OffsetDateTime::from_unix_timestamp(1_790_691_120).unwrap();
        assert_eq!(format_date(date, 2026), "Tue 29 Sep, 14:12");
        assert_eq!(format_date(date, 2027), "Tue 29 Sep 2026, 14:12");
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
            manual: true,
            ..Project::default()
        }
    }

    #[test]
    fn gone_folders_stay_listed_as_missing() {
        let dir = std::env::temp_dir().join(format!("proj-missing-{}", std::process::id()));
        let (app, gone) = (dir.join("app"), dir.join("gone"));
        fs::create_dir_all(&app).unwrap();
        let mut db = Db {
            manual: vec![gone.clone(), app.clone()],
            ..Db::default()
        };
        db.opened.insert(entry_key(std::slice::from_ref(&gone)), 99);
        remember_workspace(&mut db, vec![app.clone(), gone.clone()]);
        let projects = collect(&Config::default(), &db);
        let listed: Vec<(&Path, bool, bool)> = projects
            .iter()
            .map(|p| (p.path.as_path(), p.is_workspace(), p.missing))
            .collect();
        // Last, even though it was opened most recently.
        assert_eq!(
            listed,
            [
                (app.as_path(), false, false),
                (gone.as_path(), false, true),
                (app.as_path(), true, true),
            ]
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn tags_and_commands() {
        let mut db = Db::default();
        set_tags(&mut db, "k", "Work, #oss  client-x #work");
        let tags: Vec<&str> = db.tags["k"].iter().map(String::as_str).collect();
        assert_eq!(tags, ["client-x", "oss", "work"]);
        let tagged = Project {
            tags: tags.iter().map(|t| (*t).to_string()).collect(),
            ..project("/a/app")
        };
        assert!(tagged.has_tags(&["wo".into(), "cl".into()]));
        assert!(!tagged.has_tags(&["web".into()]));
        set_tags(&mut db, "k", "  ");
        assert!(db.tags.is_empty(), "no tags left");

        add_command(&mut db, "k", " npm run dev ");
        add_command(&mut db, "k", "npm run dev");
        add_command(&mut db, "k", "cargo watch");
        assert_eq!(db.commands["k"], ["npm run dev", "cargo watch"]);
        remove_command(&mut db, "k", "npm run dev");
        remove_command(&mut db, "k", "cargo watch");
        assert!(db.commands.is_empty());
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
    fn project_default_editors() {
        let config = Config {
            editor: Some("zed".into()),
            ..Config::default()
        };
        let dir = std::env::temp_dir().join(format!("proj-editors-{}", std::process::id()));
        let (app, web) = (dir.join("app"), dir.join("web"));
        fs::create_dir_all(&app).unwrap();
        fs::create_dir_all(&web).unwrap();
        let key = |path: &PathBuf| entry_key(std::slice::from_ref(path));
        let (app_key, web_key) = (key(&app), key(&web));
        let mut db = Db {
            manual: vec![app.clone(), web.clone()],
            ..Db::default()
        };
        // Older versions kept a list to pick from: the first one is the default.
        db.editors
            .insert(app_key.clone(), vec!["devenv".into(), "code".into()]);
        let editor = |db: &Db, path: &Path| {
            let projects = collect(&config, db);
            let project = projects.iter().find(|p| p.path == path).unwrap();
            (project.editor.clone(), project.default_editor(&config))
        };
        assert_eq!(editor(&db, &app), (Some("devenv".into()), "devenv".into()));
        assert_eq!(editor(&db, &web), (None, "zed".into()), "the global one");

        set_editor(&mut db, &web_key, Some("code".into()));
        assert_eq!(editor(&db, &web).1, "code");
        set_editor(&mut db, &app_key, None);
        assert_eq!(editor(&db, &app), (None, "zed".into()));
        // Still a list on disk, so older versions can read the file.
        assert!(toml::to_string(&db).unwrap().contains(r#"= ["code"]"#));
        fs::remove_dir_all(&dir).ok();
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

    #[test]
    fn renamed_entries_keep_their_name() {
        let dir = std::env::temp_dir().join(format!("proj-name-{}", std::process::id()));
        let (client, server) = (dir.join("client/app"), dir.join("server/app"));
        fs::create_dir_all(&client).unwrap();
        fs::create_dir_all(&server).unwrap();
        let mut db = Db {
            manual: vec![client.clone(), server.clone()],
            ..Db::default()
        };
        let names = |db: &Db| {
            let mut names: Vec<_> = collect(&Config::default(), db)
                .into_iter()
                .map(|p| p.name)
                .collect();
            names.sort();
            names
        };
        let key = entry_key(std::slice::from_ref(&client));
        rename(&mut db, &key, "  Client  ");
        assert_eq!(names(&db), ["Client", "app (server)"]);

        rename(&mut db, &key, "");
        assert!(db.names.is_empty(), "an empty name resets it");
        assert_eq!(names(&db), ["app (client)", "app (server)"]);

        rename(&mut db, &key, "Client");
        let listed = collect(&Config::default(), &db);
        let renamed = listed.iter().find(|p| p.key() == key).unwrap();
        forget_entry(&mut db, renamed);
        assert!(db.names.is_empty(), "removing forgets the name");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn forgetting_a_project() {
        let mut db = Db {
            manual: vec![PathBuf::from("/a/app"), PathBuf::from("/a/web")],
            ..Db::default()
        };
        let mut app = project("/a/app");
        db.opened.insert(app.key(), 5);
        forget_entry(&mut db, &app);
        assert_eq!(db.manual, [PathBuf::from("/a/web")]);
        assert!(db.opened.is_empty());

        // Scanned projects are hidden instead, so a rescan doesn't bring them back.
        app.manual = false;
        forget_entry(&mut db, &app);
        assert!(db.hidden.contains(&PathBuf::from("/a/app")));
    }
}
