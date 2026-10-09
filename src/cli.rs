//! Command-line interface: `proj add`, `proj list`, …

use std::path::PathBuf;

use crate::{
    autostart, config, fuzzy, open, paths, platform, recent,
    store::{self, Project},
    update,
};

const USAGE: &str = "\
proj - project launcher

usage:
  proj                     run the launcher in the background
  proj open QUERY          open the project that best matches QUERY
  proj add [PATH]          add a project (defaults to the current directory)
  proj remove PATH         remove / hide a project
  proj list                list known projects
  proj recent [add]        list the projects other editors opened lately (add: add them)
  proj paths               show config and database locations
  proj autostart [on|off]  start proj when you log in
  proj path [add|remove]   put proj's folder on your PATH (Windows)
  proj update              install the latest release, if it is newer
  proj version             show the installed version";

pub fn run(args: &[String]) -> i32 {
    let config = config::load_config();
    let mut db = store::load_db();
    let path_arg = |arg: Option<&String>| -> Option<PathBuf> {
        match arg {
            Some(arg) => paths::normalize(arg),
            None => std::env::current_dir().ok(),
        }
    };
    match args[0].as_str() {
        "open" => {
            let query = args[1..].join(" ");
            if query.trim().is_empty() {
                eprintln!("usage: proj open QUERY");
                return 2;
            }
            let projects = store::collect(&config, &db);
            let Some(project) = best_match(&query, &projects) else {
                eprintln!("proj: no project matches '{query}'");
                return 1;
            };
            let editor = project.default_editor(&config);
            if let Err(err) = open::open_with(&config, &editor, &project.paths()) {
                eprintln!("proj: could not open {}: {err}", project.name);
                return 1;
            }
            store::record_open(&mut db, project.key());
            println!("opened {}", project.name);
        }
        "add" => {
            let Some(path) = path_arg(args.get(1)).filter(|p| p.is_dir()) else {
                eprintln!("proj: not a directory");
                return 1;
            };
            println!("added {}", path.display());
            store::add_manual(&mut db, path);
        }
        "remove" | "rm" => {
            let Some(path) = args.get(1).and_then(|a| paths::normalize(a)) else {
                eprintln!("usage: proj remove PATH");
                return 1;
            };
            if db.manual.contains(&path) {
                db.manual.retain(|p| p != &path);
            } else {
                db.hidden.insert(path.clone());
            }
            let key = path.to_string_lossy();
            db.names.remove(key.as_ref());
            db.editors.remove(key.as_ref());
            db.opened.remove(key.as_ref());
            db.tags.remove(key.as_ref());
            db.commands.remove(key.as_ref());
            println!("removed {}", path.display());
        }
        "list" | "ls" => {
            for project in store::collect(&config, &db) {
                let missing = if project.missing { "  (missing)" } else { "" };
                println!("{:<32} {}{missing}", project.name, project.path.display());
            }
            return 0;
        }
        "recent" => {
            let add = match args.get(1).map(String::as_str) {
                None => false,
                Some("add") => true,
                Some(_) => {
                    eprintln!("usage: proj recent [add]");
                    return 2;
                }
            };
            let found = recent::found(&store::listed_folders(&config, &db));
            if found.is_empty() {
                println!("no recent projects that aren't listed yet");
                return 0;
            }
            for item in &found {
                let folders: Vec<String> =
                    item.paths.iter().map(|p| paths::display_path(p)).collect();
                println!("{:<56} {}", folders.join(" + "), item.editors.join(", "));
            }
            if !add {
                println!("\nproj recent add  adds them");
                return 0;
            }
            for item in found {
                store::add_found(&mut db, item.paths);
            }
        }
        "autostart" => {
            let result = match args.get(1).map(String::as_str) {
                Some("on") => autostart::set(true),
                Some("off") => autostart::set(false),
                None => Ok(()),
                Some(_) => {
                    eprintln!("usage: proj autostart [on|off]");
                    return 2;
                }
            };
            if let Err(err) = result {
                eprintln!("proj: {err}");
                return 1;
            }
            let state = if autostart::is_enabled() { "on" } else { "off" };
            println!("start on login: {state}");
            return 0;
        }
        "path" => {
            let result = match args.get(1).map(String::as_str) {
                Some("add") => platform::set_exe_dir_on_path(true).map(|_| ()),
                Some("remove") => platform::set_exe_dir_on_path(false).map(|_| ()),
                None => Ok(()),
                Some(_) => {
                    eprintln!("usage: proj path [add|remove]");
                    return 2;
                }
            };
            match result.and_then(|()| platform::exe_dir_on_path()) {
                Ok(on) => println!("on PATH: {}", if on { "yes" } else { "no" }),
                Err(err) => {
                    eprintln!("proj: {err}");
                    return 1;
                }
            }
            return 0;
        }
        "update" => {
            let found = match update::check() {
                Ok(Some(found)) => found,
                Ok(None) => {
                    println!("proj {} is up to date", update::CURRENT);
                    return 0;
                }
                Err(err) => {
                    eprintln!("proj: {err}");
                    return 1;
                }
            };
            if !update::is_installed() {
                println!(
                    "proj {} is available (this is {}); download it from {}",
                    found.version,
                    update::CURRENT,
                    update::releases_url()
                );
                return 0;
            }
            println!("updating proj {} to {}…", update::CURRENT, found.version);
            if let Err(err) = update::install(&found) {
                eprintln!("proj: {err}");
                return 1;
            }
            println!("the installer is running; proj starts again when it's done");
            return 0;
        }
        "version" | "-V" | "--version" => {
            println!("proj {}", update::CURRENT);
            return 0;
        }
        "paths" => {
            println!("config:   {}", config::config_path().display());
            println!("projects: {}", store::db_path().display());
            return 0;
        }
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            return 0;
        }
        other => {
            eprintln!("proj: unknown command '{other}'\n\n{USAGE}");
            return 2;
        }
    }
    if let Err(err) = store::save_db(&db) {
        eprintln!("proj: could not save: {err}");
        return 1;
    }
    0
}

/// The entry the dialog would put first for `query`.
fn best_match<'a>(query: &str, projects: &'a [Project]) -> Option<&'a Project> {
    let mut best: Option<(fuzzy::Rank, &Project)> = None;
    for project in projects.iter().filter(|p| !p.missing) {
        let ranked = fuzzy::rank_item(
            query,
            &project.name,
            &project.location(),
            project.search_boost(),
        );
        // Strictly greater, so ties go to the earlier (recent) entry.
        if let Some((rank, _)) = ranked.filter(|(rank, _)| best.is_none_or(|(b, _)| *rank > b)) {
            best = Some((rank, project));
        }
    }
    best.map(|(_, project)| project)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str, path: &str) -> Project {
        Project {
            name: name.into(),
            path: path.into(),
            manual: true,
            ..Project::default()
        }
    }

    #[test]
    fn open_picks_the_best_match() {
        let mut projects = vec![
            project("reviews", "/r/reviews"),
            project("api", "/r/api"),
            project("Client", "/r/example-v2"),
        ];
        assert_eq!(best_match("api", &projects).unwrap().name, "api");
        assert_eq!(
            best_match("exam", &projects).unwrap().name,
            "Client",
            "a renamed entry is still found by its folder"
        );
        assert!(best_match("zzz", &projects).is_none());
        projects.push(Project {
            missing: true,
            ..project("vanished", "/gone/vanished")
        });
        assert!(
            best_match("vanished", &projects).is_none(),
            "a missing one can't be opened"
        );

        // Equal scores: the earlier entry (the list is sorted recent first) wins.
        projects.insert(0, project("api", "/other/api"));
        assert_eq!(
            best_match("api", &projects).unwrap().path,
            PathBuf::from("/other/api")
        );
    }
}
