//! Commands a project's actions menu runs in a terminal: saved commands first,
//! followed by tasks found in the project's manifest files.

use std::{fs, path::Path};

use crate::store::{Db, Project};

#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    /// As typed in a terminal, e.g. "npm run dev".
    pub command: String,
    /// Added from the actions menu (and so removable), not found in the project.
    pub added: bool,
    /// Where the command came from (e.g. `Cargo.toml` or `Makefile`).
    pub origin: &'static str,
}

/// The entry's tasks: saved commands first, then tasks in the first folder's
/// manifests, in their file order. A command is shown only once.
pub fn tasks(project: &Project, db: &Db) -> Vec<Task> {
    let mut tasks = Vec::new();
    for command in db.commands.get(&project.key()).into_iter().flatten() {
        if !tasks.iter().any(|task: &Task| &task.command == command) {
            tasks.push(Task {
                command: command.clone(),
                added: true,
                origin: "Saved",
            });
        }
    }
    let dir = &project.path;
    let (make_origin, make_commands) = make_tasks(dir);
    let (just_origin, just_commands) = just_tasks(dir);
    for (origin, commands) in [
        ("package.json", package_scripts(dir)),
        ("Cargo.toml", cargo_tasks(dir)),
        (make_origin, make_commands),
        (just_origin, just_commands),
    ] {
        for command in commands {
            if !tasks.iter().any(|t| t.command == command) {
                tasks.push(Task {
                    command,
                    added: false,
                    origin,
                });
            }
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
        .filter_map(|name| shell_argument(&name).map(|arg| format!("{manager} run {arg}")))
        .collect()
}

/// Only emit names which cannot break out of a shell argument in either cmd.exe
/// or a POSIX shell. Never interpolate untrusted manifest text as shell syntax.
fn shell_argument(name: &str) -> Option<String> {
    if name.is_empty()
        || name.starts_with(['-', ' '])
        || !name.bytes().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'/' | b':' | b' ')
        })
    {
        return None;
    }
    Some(if name.contains(' ') {
        format!("\"{name}\"")
    } else {
        name.to_string()
    })
}

/// The conventional Cargo commands, plus targets explicitly named in the manifest.
fn cargo_tasks(dir: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(dir.join("Cargo.toml")) else {
        return Vec::new();
    };
    let Ok(manifest) = text.parse::<toml_edit::DocumentMut>() else {
        return Vec::new();
    };
    if manifest
        .get("package")
        .and_then(toml_edit::Item::as_table)
        .is_none()
        && manifest
            .get("workspace")
            .and_then(toml_edit::Item::as_table)
            .is_none()
    {
        return Vec::new();
    }
    let mut commands = vec![
        "cargo build".into(),
        "cargo test".into(),
        "cargo check".into(),
    ];
    for (kind, item) in manifest.iter() {
        let verb = match kind {
            "bin" | "example" => "run",
            "test" => "test",
            "bench" => "bench",
            _ => continue,
        };
        if let Some(targets) = item.as_array_of_tables() {
            for target in targets {
                if let Some(name) = target.get("name").and_then(toml_edit::Item::as_str)
                    && let Some(arg) = shell_argument(name)
                {
                    commands.push(format!("cargo {verb} --{kind} {arg}"));
                }
            }
        }
    }
    commands
}

/// Find a file by its actual spelling: Windows would otherwise open `Makefile`
/// when asked for `makefile`, giving the task an incorrect origin label.
fn preferred_file(dir: &Path, names: &[&'static str]) -> Option<(&'static str, String)> {
    let entries: Vec<_> = fs::read_dir(dir).ok()?.filter_map(Result::ok).collect();
    names.iter().find_map(|&name| {
        entries
            .iter()
            .find(|entry| entry.file_name() == name)
            .and_then(|entry| {
                fs::read_to_string(entry.path())
                    .ok()
                    .map(|text| (name, text))
            })
    })
}

/// Read one Makefile according to make's filename precedence, never invoking make.
fn make_tasks(dir: &Path) -> (&'static str, Vec<String>) {
    let Some((origin, text)) = preferred_file(dir, &["GNUmakefile", "makefile", "Makefile"]) else {
        return ("Makefile", Vec::new());
    };
    let commands = text
        .lines()
        .scan(false, |continued, line| {
            let was_continued = *continued;
            *continued = line.trim_end().ends_with('\\');
            Some((was_continued, line))
        })
        .filter(|(continued, line)| !continued && !line.starts_with(['\t', ' ']))
        .filter_map(|(_, line)| {
            let (names, rest) = line.split_once(':')?;
            // Reject variable assignments (:=), pattern/special targets and
            // dynamic names; only literal targets can be run safely.
            if rest.starts_with('=') || names.contains(['=', '$', '%', '#', '&', '\\']) {
                return None;
            }
            Some(names.split_whitespace().filter_map(|name| {
                if name.starts_with('.') {
                    return None;
                }
                shell_argument(name).map(|arg| format!("make {arg}"))
            }))
        })
        .flatten()
        .collect();
    (origin, commands)
}

/// Just recipe headers without parameters (which would require user input).
fn just_tasks(dir: &Path) -> (&'static str, Vec<String>) {
    let Some((origin, text)) = preferred_file(dir, &["justfile", "Justfile", ".justfile"]) else {
        return ("justfile", Vec::new());
    };
    let commands = text
        .lines()
        .filter(|line| !line.starts_with(['\t', ' ']))
        .filter_map(|line| {
            let header = line.strip_prefix('@').unwrap_or(line);
            let name = if let Some(quoted) = header.strip_prefix('"') {
                let (name, rest) = quoted.split_once('"')?;
                rest.strip_prefix(':')?;
                name
            } else {
                let (name, _) = header.split_once(':')?;
                name
            };
            // Whitespace in an unquoted header indicates recipe parameters.
            if !header.starts_with('"') && name.contains(char::is_whitespace) {
                return None;
            }
            shell_argument(name).map(|arg| format!("just {arg}"))
        })
        .collect();
    (origin, commands)
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    struct Fixture(std::path::PathBuf);

    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "proj-tasks-{}-{}",
                std::process::id(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, name: &str, contents: &str) {
            fs::write(self.0.join(name), contents).unwrap();
        }

        fn project(&self) -> Project {
            Project {
                path: self.0.clone(),
                ..Project::default()
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn added_commands_then_package_scripts() {
        let fixture = Fixture::new();
        fixture.write(
            "package.json",
            r#"{ "name": "web", "scripts": { "dev": "vite", "build": "vite build", "type check": "tsc" } }"#,
        );
        let project = fixture.project();
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

        fixture.write("pnpm-lock.yaml", "");
        assert_eq!(package_scripts(&fixture.0)[0], "pnpm run dev");
        assert!(script_names("not json").is_empty());
    }

    #[test]
    fn discovers_manifests_in_order_and_keeps_saved_commands() {
        let fixture = Fixture::new();
        fixture.write("package.json", r#"{"scripts":{"test":"node test"}}"#);
        fixture.write(
            "Cargo.toml",
            r#"[package]
name = "app"
version = "0.1.0"
[[example]]
name = "demo"
[[bin]]
name = "worker"
[[test]]
name = "integration"
[[bench]]
name = "speed"
"#,
        );
        fixture.write(
            "Makefile",
            "build test: deps\n\t@echo hello\nbuild: other\n.PHONY: build\n",
        );
        fixture.write(
            "justfile",
            "build:\n  echo ok\n@\"type check\":\n  echo ok\n",
        );
        let project = fixture.project();
        let mut db = Db::default();
        crate::store::add_command(&mut db, &project.key(), "make build");
        crate::store::add_command(&mut db, &project.key(), "cargo test");
        db.commands
            .get_mut(&project.key())
            .unwrap()
            .push("cargo test".into());
        let found: Vec<_> = tasks(&project, &db)
            .into_iter()
            .map(|t| (t.command, t.added, t.origin))
            .collect();
        assert_eq!(
            found,
            [
                ("make build".into(), true, "Saved"),
                ("cargo test".into(), true, "Saved"),
                ("npm run test".into(), false, "package.json"),
                ("cargo build".into(), false, "Cargo.toml"),
                ("cargo check".into(), false, "Cargo.toml"),
                ("cargo run --example demo".into(), false, "Cargo.toml"),
                ("cargo run --bin worker".into(), false, "Cargo.toml"),
                ("cargo test --test integration".into(), false, "Cargo.toml"),
                ("cargo bench --bench speed".into(), false, "Cargo.toml"),
                ("make test".into(), false, "Makefile"),
                ("just build".into(), false, "justfile"),
                ("just \"type check\"".into(), false, "justfile"),
            ]
        );
    }

    #[test]
    fn rejects_shell_syntax_and_non_executable_definitions() {
        let fixture = Fixture::new();
        fixture.write(
            "package.json",
            r#"{"scripts":{"lint:fix":"ok","safe name":"ok","bad; echo pwned":"ok","x\" & whoami":"ok","-option":"ok"}}"#,
        );
        fixture.write(
            "Cargo.toml",
            "[package]\nname = 'app'\nversion = '0.1.0'\n[[bin]]\nname = 'bad; echo pwned'\n",
        );
        fixture.write(
            "Makefile",
            "GOOD: dep \\\nnot-a-target: still-a-dependency\nVAR := value\n.PHONY: GOOD\n%.o: %.c\nbad;echo: x\nbad\\ name: x\n\tfoo: bar\n",
        );
        fixture.write("justfile", "safe:\n  echo ok\n\"two words\":\n  echo ok\nrequired arg:\n  echo ok\nunsafe;touch:\n  echo ok\nvalue := 'hi'\n");
        let found: Vec<_> = tasks(&fixture.project(), &Db::default())
            .into_iter()
            .map(|task| task.command)
            .collect();
        assert_eq!(
            found,
            [
                "npm run lint:fix",
                "npm run \"safe name\"",
                "cargo build",
                "cargo test",
                "cargo check",
                "make GOOD",
                "just safe",
                "just \"two words\"",
            ]
        );
    }

    #[test]
    fn invalid_and_missing_manifests_are_ignored() {
        let fixture = Fixture::new();
        assert!(tasks(&fixture.project(), &Db::default()).is_empty());
        fixture.write("Cargo.toml", "[package\n");
        fixture.write("package.json", "invalid");
        fixture.write("Makefile", "one:\n");
        fixture.write("GNUmakefile", "preferred:\n");
        assert_eq!(
            tasks(&fixture.project(), &Db::default())
                .into_iter()
                .map(|task| (task.command, task.origin))
                .collect::<Vec<_>>(),
            [("make preferred".into(), "GNUmakefile")]
        );
    }
}
