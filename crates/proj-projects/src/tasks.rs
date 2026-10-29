//! Commands a project's actions menu runs in a terminal: the ones added to it,
//! then its `package.json` scripts.

use std::{fs, path::Path};

use crate::store::{Db, Project};

#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    /// As typed in a terminal, e.g. "npm run dev".
    pub command: String,
    /// Added from the actions menu (and so removable), not found in the project.
    pub added: bool,
}

/// The entry's tasks: added ones first, then the scripts of the (first)
/// folder's `package.json` that aren't added too.
pub fn tasks(project: &Project, db: &Db) -> Vec<Task> {
    let mut tasks: Vec<Task> = db
        .commands
        .get(&project.key())
        .into_iter()
        .flatten()
        .map(|command| Task {
            command: command.clone(),
            added: true,
        })
        .collect();
    for command in package_scripts(&project.path) {
        if !tasks.iter().any(|t| t.command == command) {
            tasks.push(Task {
                command,
                added: false,
            });
        }
    }
    tasks
}

/// "npm run dev" for each script, with the package manager the lock file says.
fn package_scripts(dir: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(dir.join("package.json")) else {
        return Vec::new();
    };
    let manager = package_manager(dir);
    script_names(&text)
        .into_iter()
        .map(|name| {
            if name.contains(char::is_whitespace) {
                format!("{manager} run \"{name}\"")
            } else {
                format!("{manager} run {name}")
            }
        })
        .collect()
}

fn package_manager(dir: &Path) -> &'static str {
    [
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lock", "bun"),
        ("bun.lockb", "bun"),
    ]
    .into_iter()
    .find(|(lock, _)| dir.join(lock).is_file())
    .map_or("npm", |(_, manager)| manager)
}

/// The keys of `scripts`, in the file's order.
fn script_names(package_json: &str) -> Vec<String> {
    let Ok(serde_json::Value::Object(package)) = serde_json::from_str(package_json) else {
        return Vec::new();
    };
    match package.get("scripts") {
        Some(serde_json::Value::Object(scripts)) => scripts.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn added_commands_then_package_scripts() {
        let dir = std::env::temp_dir().join(format!("proj-tasks-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("package.json"),
            r#"{ "name": "web", "scripts": { "dev": "vite", "build": "vite build", "type check": "tsc" } }"#,
        )
        .unwrap();
        let project = Project {
            path: dir.clone(),
            ..Project::default()
        };
        let mut db = Db::default();
        crate::store::add_command(&mut db, &project.key(), "cargo watch");
        crate::store::add_command(&mut db, &project.key(), "npm run dev");
        let found: Vec<(String, bool)> = tasks(&project, &db)
            .into_iter()
            .map(|t| (t.command, t.added))
            .collect();
        let expect = |command: &str, added| (command.to_string(), added);
        assert_eq!(
            found,
            [
                expect("cargo watch", true),
                expect("npm run dev", true),
                expect("npm run build", false),
                expect("npm run \"type check\"", false),
            ]
        );

        fs::write(dir.join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(package_scripts(&dir)[0], "pnpm run dev");
        assert!(script_names("not json").is_empty());
        fs::remove_dir_all(dir).ok();
    }
}
