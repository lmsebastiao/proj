//! Starting a new project from a template: a folder (a project of the list,
//! or one from `templates` in config.toml) or a git URL.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::{git, paths};

#[derive(Clone, Debug, PartialEq)]
pub enum Template {
    Folder(PathBuf),
    Git(String),
}

/// Folders the copy of a folder that isn't a git repository leaves out: build
/// output and installed packages. Repositories copy what git would commit.
pub(crate) const SKIPPED: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
];

impl Template {
    /// A `templates` entry: a git URL, or a folder (with `~` for home).
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if git::clone_name(text).is_some() {
            return Some(Self::Git(text.to_string()));
        }
        paths::normalize(text).map(Self::Folder)
    }

    /// The folder's or repository's name.
    pub fn name(&self) -> String {
        match self {
            Self::Folder(path) => path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
            Self::Git(url) => git::clone_name(url).unwrap_or_else(|| url.clone()),
        }
    }

    /// Where it comes from, for display.
    pub fn source(&self) -> String {
        match self {
            Self::Folder(path) => paths::display_path(path),
            Self::Git(url) => url.clone(),
        }
    }

    /// Makes `dest` a copy of the template, with a git history of its own.
    /// Takes as long as a copy or clone does: call it off the main thread.
    pub fn create(&self, dest: &Path) -> io::Result<()> {
        if dest.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} already exists", paths::display_path(dest)),
            ));
        }
        match self {
            Self::Folder(source) => copy_project(source, dest)?,
            Self::Git(url) => {
                git::clone_latest(url, dest)?;
                remove_dir_all_force(&dest.join(".git"))?;
            }
        }
        // Without git, the copy is still there to use.
        git::init(dest).ok();
        Ok(())
    }
}

/// Copies what git would commit from a repository; anything else whole, but
/// for the `SKIPPED` folders.
fn copy_project(source: &Path, dest: &Path) -> io::Result<()> {
    if !source.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not a folder", paths::display_path(source)),
        ));
    }
    fs::create_dir_all(dest)?;
    match git::listed_files(source) {
        Some(files) => {
            for file in files {
                let from = source.join(&file);
                // Listed but deleted, or a submodule's folder.
                if !from.is_file() {
                    continue;
                }
                let to = dest.join(&file);
                if let Some(parent) = to.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&from, &to)?;
            }
            Ok(())
        }
        None => copy_dir(source, dest),
    }
}

fn copy_dir(source: &Path, dest: &Path) -> io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        let to = dest.join(&name);
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if SKIPPED.iter().any(|s| name.eq_ignore_ascii_case(s)) {
                continue;
            }
            fs::create_dir_all(&to)?;
            copy_dir(&entry.path(), &to)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// `fs::remove_dir_all`, also for git's read-only object files, which
/// Windows won't delete as they are.
fn remove_dir_all_force(dir: &Path) -> io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    fn make_writable(dir: &Path) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                make_writable(&path)?;
            } else {
                let mut permissions = fs::metadata(&path)?.permissions();
                if permissions.readonly() {
                    #[allow(clippy::permissions_set_readonly_false)]
                    permissions.set_readonly(false);
                    fs::set_permissions(&path, permissions)?;
                }
            }
        }
        Ok(())
    }
    make_writable(dir)?;
    fs::remove_dir_all(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_from_config() {
        assert_eq!(
            Template::parse("https://github.com/me/web-starter.git"),
            Some(Template::Git(
                "https://github.com/me/web-starter.git".into()
            ))
        );
        let git = Template::parse("git@github.com:me/cli.git").unwrap();
        assert_eq!(git.name(), "cli");
        let folder = Template::parse("/srv/templates/rust-cli/").unwrap();
        assert!(matches!(&folder, Template::Folder(path) if path.ends_with("templates/rust-cli")));
        assert_eq!(folder.name(), "rust-cli");
    }

    #[test]
    fn copies_a_folder_without_build_output() {
        let root = std::env::temp_dir().join(format!("proj-template-{}", std::process::id()));
        let source = root.join("starter");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::create_dir_all(source.join("node_modules/left-pad")).unwrap();
        fs::write(source.join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(source.join("node_modules/left-pad/index.js"), "").unwrap();
        let dest = root.join("new-app");
        copy_project(&source, &dest).unwrap();
        assert!(dest.join("src/main.rs").is_file());
        assert!(!dest.join("node_modules").exists());
        assert!(
            Template::Folder(source.clone()).create(&dest).is_err(),
            "not over an existing folder"
        );

        // A read-only file, like git's objects.
        let objects = root.join("repo/.git/objects");
        fs::create_dir_all(&objects).unwrap();
        let object = objects.join("pack.idx");
        fs::write(&object, "").unwrap();
        let mut permissions = fs::metadata(&object).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&object, permissions).unwrap();
        remove_dir_all_force(&root.join("repo/.git")).unwrap();
        assert!(!root.join("repo/.git").exists());
        fs::remove_dir_all(root).ok();
    }
}
