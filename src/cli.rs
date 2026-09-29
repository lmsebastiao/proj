//! Command-line interface: `proj add`, `proj list`, …

use std::path::PathBuf;

use crate::{autostart, config, paths, platform, store};

const USAGE: &str = "\
proj - project launcher

usage:
  proj                     run the launcher in the background
  proj add [PATH]          add a project (defaults to the current directory)
  proj remove PATH         remove / hide a project
  proj list                list known projects
  proj paths               show config and database locations
  proj autostart [on|off]  start proj when you log in
  proj path [add|remove]   put proj's folder on your PATH (Windows)";

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
        "add" => {
            let Some(path) = path_arg(args.get(1)).filter(|p| p.is_dir()) else {
                eprintln!("proj: not a directory");
                return 1;
            };
            db.hidden.remove(&path);
            if !db.manual.contains(&path) {
                db.manual.push(path.clone());
            }
            println!("added {}", path.display());
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
            db.pinned.remove(key.as_ref());
            db.editors.remove(key.as_ref());
            db.opened.remove(key.as_ref());
            println!("removed {}", path.display());
        }
        "list" | "ls" => {
            for project in store::collect(&config, &db) {
                println!("{:<32} {}", project.name, project.path.display());
            }
            return 0;
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
