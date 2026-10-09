//! Folders other editors opened lately, to import: Zed's workspaces (in its
//! database), VS Code's and its forks' (Cursor, Windsurf, VSCodium…), and
//! JetBrains IDEs' recent projects.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use crate::sqlite;

/// A recent project found in another editor's history.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    /// One folder, or a Zed workspace's several.
    pub paths: Vec<PathBuf>,
    /// The editors it was found in: "Zed", "VS Code"…
    pub editors: Vec<&'static str>,
    /// Unix seconds it was last opened, where the editor says.
    pub at: Option<u64>,
}

/// The VS Code family's folders under the config directory (`%APPDATA%`,
/// `~/Library/Application Support`, `~/.config`).
const VS_CODES: &[(&str, &str)] = &[
    ("Code", "VS Code"),
    ("Code - Insiders", "VS Code Insiders"),
    ("Cursor", "Cursor"),
    ("Windsurf", "Windsurf"),
    ("VSCodium", "VSCodium"),
];

/// Every recent project the editors on this machine remember whose folders
/// are there, most recent first where that's known, leaving out those
/// `listed` already has (any of their folders, for a workspace: all of them).
pub fn found(listed: &[PathBuf]) -> Vec<Found> {
    let mut found = Vec::new();
    if let Some(dir) = zed_db_dir() {
        for channel in ["0-stable", "0-preview"] {
            found.extend(zed(&dir.join(channel).join("db.sqlite")));
        }
    }
    if let Some(config) = dirs::config_dir() {
        for (folder, name) in VS_CODES {
            let storage = config.join(folder).join("User/globalStorage/storage.json");
            if let Ok(text) = fs::read_to_string(storage) {
                found.extend(vs_code(&text, name));
            }
        }
        found.extend(jetbrains(&config.join("JetBrains"), "JetBrains"));
        found.extend(jetbrains(&config.join("Google"), "Android Studio"));
    }
    keep(found, listed, &not_projects())
}

/// Folders whose insides aren't projects: app data and temp files.
fn not_projects() -> Vec<PathBuf> {
    [
        dirs::config_dir(),
        dirs::data_local_dir(),
        dirs::data_dir(),
        dirs::cache_dir(),
        Some(std::env::temp_dir()),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Where Zed keeps its databases.
fn zed_db_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        dirs::data_local_dir().map(|d| d.join("Zed/db"))
    } else if cfg!(target_os = "macos") {
        dirs::home_dir().map(|d| d.join("Library/Application Support/Zed/db"))
    } else {
        dirs::data_local_dir().map(|d| d.join("zed/db"))
    }
}

/// The folders that are there, merged (one entry per set of folders, with
/// every editor that had it), leaving out the listed ones and places that
/// aren't projects: home, a drive's root, and what's in `skip`.
fn keep(found: Vec<Found>, listed: &[PathBuf], skip: &[PathBuf]) -> Vec<Found> {
    let listed: HashSet<String> = listed.iter().map(|p| same(p)).collect();
    let home = dirs::home_dir();
    let mut kept: Vec<Found> = Vec::new();
    for mut item in found {
        item.paths.dedup();
        let ok = |path: &Path| {
            path.parent().is_some()
                && home.as_deref() != Some(path)
                && !skip.iter().any(|dir| path.starts_with(dir))
                && path.is_dir()
        };
        if item.paths.is_empty() || !item.paths.iter().all(|p| ok(p)) {
            continue;
        }
        if item.paths.iter().all(|p| listed.contains(&same(p))) {
            continue;
        }
        let key = set_key(&item.paths);
        match kept.iter_mut().find(|k| set_key(&k.paths) == key) {
            Some(known) => {
                for editor in item.editors {
                    if !known.editors.contains(&editor) {
                        known.editors.push(editor);
                    }
                }
                known.at = known.at.max(item.at);
            }
            None => kept.push(item),
        }
    }
    // The ones with a time first, the latest first; the rest as found.
    kept.sort_by_key(|f| std::cmp::Reverse(f.at));
    kept
}

/// A path to compare: on Windows, whatever its casing.
fn same(path: &Path) -> String {
    let text = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

fn set_key(paths: &[PathBuf]) -> Vec<String> {
    let mut key: Vec<String> = paths.iter().map(|p| same(p)).collect();
    key.sort();
    key
}

/// Zed's workspaces: the `paths` of each (one folder per line) and when it
/// was last used, from its database. Remote ones (`remote_connection_id`)
/// aren't folders here.
fn zed(db: &Path) -> Vec<Found> {
    let Ok(mut database) = sqlite::Database::open(db) else {
        return Vec::new();
    };
    let Ok(Some(table)) = database.table("workspaces") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for row in &table.rows {
        let remote = table
            .get(row, "remote_connection_id")
            .is_some_and(|v| *v != sqlite::Value::Null);
        let Some(paths) = table.get(row, "paths").and_then(sqlite::Value::as_text) else {
            continue;
        };
        if remote {
            continue;
        }
        let paths: Vec<PathBuf> = paths
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .collect();
        let at = table
            .get(row, "timestamp")
            .and_then(sqlite::Value::as_text)
            .and_then(unix_time);
        found.push(Found {
            paths,
            editors: vec!["Zed"],
            at,
        });
    }
    found
}

/// "2026-10-09 14:12:05" (UTC, as SQLite's CURRENT_TIMESTAMP) in unix seconds.
fn unix_time(text: &str) -> Option<u64> {
    let mut numbers = text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<i64>());
    let mut next = || numbers.next()?.ok();
    let (y, m, d) = (next()?, next()?, next()?);
    let (hh, mm, ss) = (
        next().unwrap_or(0),
        next().unwrap_or(0),
        next().unwrap_or(0),
    );
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3600 + mm * 60 + ss).ok()
}

/// The folders in a VS Code-like editor's `storage.json`: the ones it keeps
/// a profile for (every folder opened lately), and its open windows'.
fn vs_code(text: &str, editor: &'static str) -> Vec<Found> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let mut uris: Vec<&str> = Vec::new();
    if let Some(workspaces) = json["profileAssociations"]["workspaces"].as_object() {
        uris.extend(workspaces.keys().map(String::as_str));
    }
    if let Some(folders) = json["backupWorkspaces"]["folders"].as_array() {
        uris.extend(folders.iter().filter_map(|f| f["folderUri"].as_str()));
    }
    let windows = &json["windowsState"];
    uris.extend(windows["lastActiveWindow"]["folder"].as_str());
    if let Some(open) = windows["openedWindows"].as_array() {
        uris.extend(open.iter().filter_map(|w| w["folder"].as_str()));
    }
    uris.into_iter()
        .filter_map(file_uri_path)
        .map(|path| Found {
            paths: vec![path],
            editors: vec![editor],
            at: None,
        })
        .collect()
}

/// The path of a `file://` URI: `file:///c%3A/repos/app` is `C:\repos\app`,
/// `file://server/share/app` the network path `\\server\share\app`.
pub(crate) fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let decoded = percent_decode(rest)?;
    let path = match decoded.strip_prefix('/') {
        // file:///c:/x
        Some(local) if cfg!(windows) => {
            let bytes = local.as_bytes();
            if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
                let drive = local[..1].to_ascii_uppercase();
                format!("{drive}{}", &local[1..]).replace('/', "\\")
            } else {
                return None;
            }
        }
        Some(_) => decoded.clone(),
        // file://host/share
        None if cfg!(windows) => format!(r"\\{}", decoded.replace('/', "\\")),
        None => return None,
    };
    let path = path.trim_end_matches(['/', '\\']);
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// `%xx` escapes to bytes, read as UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The recent projects of every JetBrains IDE under `dir` (one folder per
/// IDE and version, e.g. `IntelliJIdea2025.2`): their
/// `options/recentProjects.xml`.
fn jetbrains(dir: &Path, editor: &'static str) -> Vec<Found> {
    let mut found = Vec::new();
    for ide in fs::read_dir(dir).into_iter().flatten().flatten() {
        let file = ide.path().join("options").join("recentProjects.xml");
        if let Ok(text) = fs::read_to_string(file) {
            found.extend(recent_projects_xml(&text, editor));
        }
    }
    found
}

/// The `<entry key="$USER_HOME$/repos/app">` of a `recentProjects.xml`, each
/// with its `projectOpenTimestamp` (milliseconds).
fn recent_projects_xml(text: &str, editor: &'static str) -> Vec<Found> {
    let home = dirs::home_dir().map(|h| h.to_string_lossy().into_owned());
    let mut found = Vec::new();
    for entry in text.split("<entry key=\"").skip(1) {
        let Some((key, rest)) = entry.split_once('"') else {
            continue;
        };
        let key = xml_unescape(key);
        let path = match (key.strip_prefix("$USER_HOME$"), &home) {
            (Some(rest), Some(home)) => format!("{home}{rest}"),
            (Some(_), None) => continue,
            (None, _) => key,
        };
        let path: PathBuf = if cfg!(windows) {
            path.replace('/', "\\").into()
        } else {
            path.into()
        };
        // Its own info runs to the next entry.
        let own = rest.split("<entry key=\"").next().unwrap_or(rest);
        let at = own
            .split_once("name=\"projectOpenTimestamp\" value=\"")
            .and_then(|(_, v)| v.split('"').next())
            .and_then(|ms| ms.parse::<u64>().ok())
            .map(|ms| ms / 1000);
        found.push(Found {
            paths: vec![path],
            editors: vec![editor],
            at,
        });
    }
    found
}

fn xml_unescape(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(unix_time("1970-01-01 00:00:00"), Some(0));
        // 2026-09-29 14:12 UTC, as in store's tests.
        assert_eq!(unix_time("2026-09-29 14:12:00"), Some(1_790_691_120));
        assert_eq!(unix_time("nonsense"), None);
    }

    #[test]
    fn file_uris() {
        if cfg!(windows) {
            assert_eq!(
                file_uri_path("file:///c%3A/Users/LucasSebasti%C3%A3o/repos/app"),
                Some(PathBuf::from(r"C:\Users\LucasSebastião\repos\app"))
            );
            assert_eq!(
                file_uri_path("file://wsl.localhost/Ubuntu/home/me/app/"),
                Some(PathBuf::from(r"\\wsl.localhost\Ubuntu\home\me\app"))
            );
        } else {
            assert_eq!(
                file_uri_path("file:///home/me/my%20app"),
                Some(PathBuf::from("/home/me/my app"))
            );
        }
        assert_eq!(file_uri_path("vscode-remote://ssh-remote%2Bbox/home"), None);
        assert_eq!(file_uri_path("file:///bad%zz"), None);
    }

    #[test]
    fn vs_code_storage() {
        let text = r#"{
            "profileAssociations": { "workspaces": {
                "file:///c%3A/repos/app": "__default__profile__",
                "vscode-remote://ssh-remote%2Bbox/home/me/x": "__default__profile__"
            } },
            "backupWorkspaces": { "folders": [ { "folderUri": "file:///c%3A/repos/web" } ] },
            "windowsState": { "lastActiveWindow": { "folder": "file:///c%3A/repos/app" } }
        }"#;
        let found = vs_code(text, "VS Code");
        let paths: Vec<String> = found
            .iter()
            .map(|f| f.paths[0].to_string_lossy().replace('\\', "/"))
            .collect();
        if cfg!(windows) {
            assert_eq!(paths, ["C:/repos/app", "C:/repos/web", "C:/repos/app"]);
        }
        assert!(found.iter().all(|f| f.editors == ["VS Code"]));
        assert!(vs_code("not json", "VS Code").is_empty());
    }

    #[test]
    fn jetbrains_recent_projects() {
        let xml = r#"<application>
  <component name="RecentProjectsManager">
    <option name="additionalInfo">
      <map>
        <entry key="$USER_HOME$/repos/api">
          <value><RecentProjectMetaInfo>
            <option name="projectOpenTimestamp" value="1790691120000" />
          </RecentProjectMetaInfo></value>
        </entry>
        <entry key="/srv/a&amp;b">
          <value><RecentProjectMetaInfo /></value>
        </entry>
      </map>
    </option>
  </component>
</application>"#;
        let found = recent_projects_xml(xml, "JetBrains");
        assert_eq!(found.len(), 2);
        let home = dirs::home_dir().unwrap();
        assert_eq!(found[0].paths[0], home.join("repos").join("api"));
        assert_eq!(found[0].at, Some(1_790_691_120));
        assert!(found[1].paths[0].to_string_lossy().ends_with("a&b"));
        assert_eq!(found[1].at, None);
    }

    #[test]
    fn kept_once_with_every_editor_latest_first() {
        let root = std::env::temp_dir().join(format!("proj-recent-{}", std::process::id()));
        let (app, web, gone) = (root.join("app"), root.join("web"), root.join("gone"));
        fs::create_dir_all(&app).unwrap();
        fs::create_dir_all(&web).unwrap();
        let item = |paths: &[&PathBuf], editor, at| Found {
            paths: paths.iter().map(|p| (*p).clone()).collect(),
            editors: vec![editor],
            at,
        };
        let kept = keep(
            vec![
                item(&[&app], "VS Code", None),
                item(&[&web], "Zed", Some(5)),
                item(&[&app], "Zed", Some(9)),
                item(&[&gone], "Zed", Some(10)),
                item(&[&app, &web], "Zed", Some(1)),
            ],
            &[],
            &[],
        );
        let summary: Vec<(usize, Vec<&str>, Option<u64>)> = kept
            .iter()
            .map(|f| (f.paths.len(), f.editors.clone(), f.at))
            .collect();
        assert_eq!(
            summary,
            [
                (1, vec!["VS Code", "Zed"], Some(9)),
                (1, vec!["Zed"], Some(5)),
                (2, vec!["Zed"], Some(1)),
            ]
        );
        // Already listed: left out; a workspace only when all its folders are.
        let kept = keep(
            vec![item(&[&app], "Zed", None), item(&[&app, &web], "Zed", None)],
            std::slice::from_ref(&app),
            &[],
        );
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].paths.len(), 2);
        // Inside app data or temp files: not a project.
        let kept = keep(
            vec![item(&[&app], "Zed", None)],
            &[],
            std::slice::from_ref(&root),
        );
        assert!(kept.is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
