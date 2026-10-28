//! Conservative, read-only resolution of a Zed window title to local workspace roots.
//!
//! Zed sources: `crates/paths/src/paths.rs` (`database_dir`),
//! `crates/db/src/db.rs` (`db_path`), `crates/workspace/src/persistence.rs`
//! (`workspaces` migration), `crates/util/src/path_list.rs` (serialization),
//! and `crates/workspace/src/workspace.rs` (`project_window_title`).
//! Stable and Preview share the Zed data directory but have distinct DB scopes.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

/// Resolve the *active* local project named in a Zed window title.
///
/// Only the leading, complete project-name segment (before Zed's default ` — `
/// separator) is considered. Zed's project name is the visible worktree names
/// joined by `, `, not the list of workspaces in a multi-workspace window.
/// Custom title formats that move/omit this segment cannot be resolved safely.
/// Returns `None` for an unknown executable, missing DB/paths, remote workspace,
/// unsupported title, or more than one distinct matching local workspace.
/// This does not identify a window by ID: two open windows with the same title
/// but different roots are intentionally ambiguous.
#[cfg(any(windows, target_os = "macos"))]
pub fn zed_paths(title: &str, exe: &Path) -> Option<Vec<PathBuf>> {
    let channel = zed_channel(exe)?;
    let data = {
        #[cfg(windows)]
        {
            dirs::data_local_dir()?.join("Zed")
        }
        #[cfg(target_os = "macos")]
        {
            dirs::home_dir()?.join("Library/Application Support/Zed")
        }
    };
    let db = data
        .join("db")
        .join(format!("0-{channel}"))
        .join("db.sqlite");
    zed_paths_in_db(title, &db)
}

/// Other platforms have no executable-location check in this module.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn zed_paths(_title: &str, _exe: &Path) -> Option<Vec<PathBuf>> {
    None
}

#[cfg(any(windows, target_os = "macos"))]
fn zed_channel(exe: &Path) -> Option<&'static str> {
    #[cfg(windows)]
    {
        let name = exe.file_name()?.to_str()?;
        if !name.eq_ignore_ascii_case("zed.exe") {
            return None;
        }
        let parent = exe.parent()?;
        let install = if parent.file_name()?.to_str()?.eq_ignore_ascii_case("bin") {
            parent.parent()?
        } else {
            parent
        };
        match install.file_name()?.to_str()?.to_ascii_lowercase().as_str() {
            "zed" => Some("stable"),
            "zed preview" => Some("preview"),
            _ => None,
        }
    }
    #[cfg(target_os = "macos")]
    {
        let name = exe.file_name()?.to_str()?;
        if name != "zed" && name != "cli" {
            return None;
        }
        let macos = exe.parent()?;
        if macos.file_name()? != "MacOS" || macos.parent()?.file_name()? != "Contents" {
            return None;
        }
        match macos.parent()?.parent()?.file_name()?.to_str()? {
            "Zed.app" => Some("stable"),
            "Zed Preview.app" => Some("preview"),
            _ => None,
        }
    }
}

fn zed_paths_in_db(title: &str, db: &Path) -> Option<Vec<PathBuf>> {
    // OPEN_READ_ONLY does not create a missing database or migrate its schema.
    // A normal SQLite reader also sees committed WAL transactions (unlike immutable=1).
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    resolve(title, &conn)
}

fn resolve(title: &str, conn: &Connection) -> Option<Vec<PathBuf>> {
    // Only Zed's default title layout is supported. In particular, a file name
    // later in the title must never be mistaken for a workspace name.
    let project = title.split_once(" — ").map_or(title, |(name, _)| name);
    let project = project.strip_prefix("● ").unwrap_or(project);
    if project.is_empty()
        || project == "empty project"
        || project != project.trim()
        || title.ends_with(" ↙")
        || title.ends_with(" ↗")
    {
        return None;
    }

    let mut stmt = conn
        .prepare(
            "SELECT paths, paths_order FROM workspaces \
         WHERE remote_connection_id IS NULL AND paths IS NOT NULL",
        )
        .ok()?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .ok()?;
    let mut found: Option<Vec<PathBuf>> = None;
    for row in rows {
        let (paths, order) = row.ok()?;
        let Some(candidate) = workspace_paths(&paths, order.as_deref()) else {
            continue;
        };
        let names: Option<Vec<&str>> = candidate
            .iter()
            .map(|path| path.file_name()?.to_str())
            .collect();
        let Some(names) = names else {
            continue;
        };
        if names.join(", ") != project {
            continue;
        }
        if let Some(previous) = &found {
            // Identical paths in duplicate rows are not a different answer.
            let mut a = previous.clone();
            let mut b = candidate.clone();
            a.sort();
            b.sort();
            if a != b {
                return None;
            }
        } else {
            found = Some(candidate);
        }
    }
    found
}

fn workspace_paths(serialized: &str, order: Option<&str>) -> Option<Vec<PathBuf>> {
    if serialized.is_empty() {
        return None;
    }
    let sorted: Vec<PathBuf> = serialized.split('\n').map(PathBuf::from).collect();
    if sorted
        .iter()
        .any(|path| !path.is_absolute() || !path.is_dir() || !local_path(path))
    {
        return None;
    }
    let indices: Vec<usize> = order?
        .split(',')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if indices.len() != sorted.len() {
        return None;
    }
    let mut ordered = vec![None; sorted.len()];
    for (path, index) in sorted.into_iter().zip(indices) {
        if ordered.get_mut(index)?.replace(path).is_some() {
            return None;
        }
    }
    ordered.into_iter().collect()
}

#[cfg(windows)]
fn local_path(path: &Path) -> bool {
    // Network/WSL UNC paths are not local even if the OS reports them as directories.
    use std::path::{Component, Prefix};
    matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_)))
}

#[cfg(not(windows))]
fn local_path(_path: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "proj-discovery-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }
        fn folder(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::create_dir(&path).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE workspaces (paths TEXT, paths_order TEXT, remote_connection_id INTEGER)",
        )
        .unwrap();
        conn
    }

    fn insert(conn: &Connection, paths: &[&Path], order: &str, remote: bool) {
        conn.execute(
            "INSERT INTO workspaces VALUES (?1, ?2, ?3)",
            rusqlite::params![
                paths
                    .iter()
                    .map(|p| p.to_str().unwrap())
                    .collect::<Vec<_>>()
                    .join("\n"),
                order,
                remote.then_some(1)
            ],
        )
        .unwrap();
    }

    #[test]
    fn exact_title_and_unique_folder() {
        let f = Fixture::new();
        let app = f.folder("app");
        let other = f.folder("application");
        let conn = db();
        insert(&conn, &[&app], "0", false);
        insert(&conn, &[&other], "0", false);
        assert_eq!(resolve("app — main.rs", &conn), Some(vec![app.clone()]));
        assert_eq!(resolve("● app — main.rs", &conn), Some(vec![app.clone()]));
        assert_eq!(resolve("app", &conn), Some(vec![app.clone()]));
        assert_eq!(resolve("applic — app", &conn), None);
        assert_eq!(resolve("other — app", &conn), None);
        assert_eq!(resolve("empty project", &conn), None);
        assert_eq!(resolve("app — main.rs ↙", &conn), None);
    }

    #[test]
    fn ambiguous_names_and_missing_paths() {
        let a = Fixture::new();
        let b = Fixture::new();
        let first = a.folder("app");
        let second = b.folder("app");
        let conn = db();
        insert(&conn, &[&first], "0", false);
        insert(&conn, &[&second], "0", false);
        assert_eq!(resolve("app — src.rs", &conn), None);
        fs::remove_dir(&second).unwrap();
        assert_eq!(resolve("app", &conn), Some(vec![first.clone()]));
        insert(&conn, &[&first], "0", false);
        assert_eq!(resolve("app", &conn), Some(vec![first]));
    }

    #[test]
    fn workspace_requires_complete_ordered_folder_list() {
        let f = Fixture::new();
        let app = f.folder("app");
        let sdk = f.folder("sample-sdk");
        let conn = db();
        // `paths` sorted by Zed, `paths_order` maps sorted entries back to display order.
        insert(&conn, &[&app, &sdk], "1,0", false);
        assert_eq!(
            resolve("sample-sdk, app — lib.rs", &conn),
            Some(vec![sdk.clone(), app.clone()])
        );
        assert_eq!(resolve("app, sample-sdk", &conn), None);
        assert_eq!(resolve("app", &conn), None);
        insert(&conn, &[&app], "0", false);
        assert_eq!(resolve("app", &conn), Some(vec![app.clone()]));
        assert_eq!(resolve("sample-sdk, app", &conn), Some(vec![sdk, app]));
    }

    #[test]
    fn skip_remote_and_invalid_workspaces() {
        let f = Fixture::new();
        let app = f.folder("app");
        let sdk = f.folder("sdk");
        let conn = db();
        insert(&conn, &[&app], "0", true);
        assert_eq!(resolve("app", &conn), None);
        insert(&conn, &[&app, &sdk], "0,0", false);
        assert_eq!(resolve("app, sdk", &conn), None);
        insert(&conn, &[&app, &sdk], "0,1", true);
        assert_eq!(resolve("app, sdk", &conn), None);
        insert(&conn, &[&app], "1", false);
        assert_eq!(resolve("app", &conn), None);
        insert(&conn, &[&app, &sdk], "0", false);
        assert_eq!(resolve("app, sdk", &conn), None);
    }

    #[test]
    fn ambiguous_multi_folder_workspaces() {
        let a = Fixture::new();
        let b = Fixture::new();
        let first = [a.folder("app"), a.folder("sdk")];
        let second = [b.folder("app"), b.folder("sdk")];
        let conn = db();
        insert(&conn, &[&first[0], &first[1]], "0,1", false);
        insert(&conn, &[&second[0], &second[1]], "0,1", false);
        assert_eq!(resolve("app, sdk — lib.rs", &conn), None);
    }

    #[test]
    fn reads_existing_database_without_writing() {
        let f = Fixture::new();
        let app = f.folder("app");
        let db_path = f.0.join("workspaces.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE workspaces (paths TEXT, paths_order TEXT, remote_connection_id INTEGER)",
        )
        .unwrap();
        insert(&conn, &[&app], "0", false);
        drop(conn);
        assert_eq!(zed_paths_in_db("app — main.rs", &db_path), Some(vec![app]));
    }

    #[test]
    fn read_only_open_does_not_create_missing_db() {
        let f = Fixture::new();
        let db = f.0.join("missing.sqlite");
        assert_eq!(zed_paths_in_db("app", &db), None);
        assert!(!db.exists());
    }

    #[cfg(windows)]
    #[test]
    fn executable_channel_is_install_specific() {
        assert_eq!(
            zed_channel(Path::new(r"C:\Users\me\Programs\Zed\Zed.exe")),
            Some("stable")
        );
        assert_eq!(
            zed_channel(Path::new(r"C:\Users\me\Programs\Zed Preview\bin\zed.exe")),
            Some("preview")
        );
        assert_eq!(zed_channel(Path::new(r"C:\Other\zed.exe")), None);
        assert_eq!(zed_channel(Path::new(r"C:\Programs\Zed\not-zed.exe")), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn executable_channel_is_bundle_specific() {
        assert_eq!(
            zed_channel(Path::new("/Applications/Zed.app/Contents/MacOS/zed")),
            Some("stable")
        );
        assert_eq!(
            zed_channel(Path::new(
                "/Applications/Zed Preview.app/Contents/MacOS/cli"
            )),
            Some("preview")
        );
        assert_eq!(zed_channel(Path::new("/tmp/zed")), None);
    }
}
