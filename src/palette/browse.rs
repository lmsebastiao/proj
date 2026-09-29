//! Browsing inside a project with →/←, and opening the files and folders in it.
//! Nothing here is added to the project list.

use std::{
    fs,
    path::{Path, PathBuf},
};

use gpui::{Context, Window};

use crate::{open, store::Project};

use super::{
    Palette,
    items::{List, Mode, Target},
};

/// The project being browsed and where we are in it.
pub(super) struct Browse {
    pub(super) project: Project,
    /// Folders entered so far. For a workspace, empty means its member folders.
    stack: Vec<PathBuf>,
    pub(super) entries: Vec<Entry>,
}

pub(super) struct Entry {
    pub(super) name: String,
    pub(super) path: PathBuf,
    pub(super) is_dir: bool,
}

impl Browse {
    fn new(project: Project) -> Self {
        let stack = if project.is_workspace() {
            Vec::new()
        } else {
            vec![project.path.clone()]
        };
        let mut browse = Self {
            project,
            stack,
            entries: Vec::new(),
        };
        browse.load();
        browse
    }

    /// Going above this depth leaves the project.
    fn top_depth(&self) -> usize {
        if self.project.is_workspace() { 0 } else { 1 }
    }

    fn load(&mut self) {
        self.entries = match self.stack.last() {
            Some(dir) => list_dir(dir),
            None => self
                .project
                .paths()
                .into_iter()
                .map(|path| Entry {
                    name: file_name(&path),
                    path,
                    is_dir: true,
                })
                .collect(),
        };
    }

    /// Where we are, e.g. "interactive-v2 › src › components".
    pub(super) fn breadcrumb(&self) -> String {
        match self.stack.last() {
            Some(dir) => self.relative(dir).replace(['\\', '/'], " › "),
            None => self.project.name.clone(),
        }
    }

    /// `path` relative to the folder that holds the project, e.g. "proj\src\main.rs".
    pub(super) fn relative(&self, path: &Path) -> String {
        self.project
            .path
            .parent()
            .and_then(|root| path.strip_prefix(root).ok())
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }
}

/// Folders first, then files, alphabetically; `.git` is skipped.
fn list_dir(dir: &Path) -> Vec<Entry> {
    let mut entries: Vec<Entry> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_name() != ".git")
        .map(|entry| {
            let path = entry.path();
            Entry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: path.is_dir(),
                path,
            }
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

impl Palette {
    /// →: enter the selected project, or the selected folder while browsing.
    /// Returns false when there's nothing to enter, so the key moves the cursor.
    pub(super) fn enter(&mut self, cx: &mut Context<Self>) -> bool {
        match self.list() {
            List::Projects => {
                let Some(project) = self.selected_project().cloned() else {
                    return false;
                };
                self.browse = Some(Browse::new(project));
                self.set_mode(Mode::Browse, cx);
                true
            }
            List::Browse => {
                let Some(dir) = self
                    .selected_entry()
                    .filter(|e| e.is_dir)
                    .map(|e| e.path.clone())
                else {
                    return false;
                };
                if let Some(browse) = self.browse.as_mut() {
                    browse.stack.push(dir);
                    browse.load();
                }
                self.set_mode(Mode::Browse, cx);
                true
            }
            _ => false,
        }
    }

    /// ←: up one folder; from the project's top level, back to the project list.
    pub(super) fn leave(&mut self, cx: &mut Context<Self>) -> bool {
        if self.mode != Mode::Browse {
            return false;
        }
        let Some(browse) = self.browse.as_mut() else {
            return false;
        };
        if browse.stack.len() <= browse.top_depth() {
            self.exit_browse(cx);
            return true;
        }
        let left = browse.stack.pop();
        browse.load();
        self.set_mode(Mode::Browse, cx);
        // Land on the folder we just came out of.
        self.select_where(|this, ix| {
            this.browse.as_ref().map(|b| &b.entries[ix].path) == left.as_ref()
        });
        true
    }

    /// Back to the project list, with the browsed project selected.
    pub(super) fn exit_browse(&mut self, cx: &mut Context<Self>) {
        let key = self.browse.take().map(|b| b.project.key());
        self.set_mode(Mode::Projects, cx);
        self.select_where(|this, ix| Some(this.projects[ix].key()) == key);
    }

    pub(super) fn selected_entry(&self) -> Option<&Entry> {
        if self.list() != List::Browse {
            return None;
        }
        let browse = self.browse.as_ref()?;
        self.matches
            .get(self.selected)
            .map(|m| &browse.entries[m.ix])
    }

    /// The editor Enter uses for `project`: its own default, else the global one.
    pub(super) fn default_editor(&self, project: &Project) -> String {
        project
            .editors
            .first()
            .or(self.config.editor.as_ref())
            .cloned()
            .unwrap_or_default()
    }

    /// Opens the selected entry: files in the project's window, folders as their
    /// own workspace; the file manager and terminal use the entry's folder.
    pub(super) fn launch_entry(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(browse), Some(entry)) = (self.browse.as_ref(), self.selected_entry()) else {
            return;
        };
        let project = browse.project.clone();
        let (path, is_dir) = (entry.path.clone(), entry.is_dir);
        let folder = if is_dir {
            path.as_path()
        } else {
            path.parent().unwrap_or(&path)
        };
        let result = match &target {
            Target::Editor(editor) if is_dir => {
                open::open_with(&self.config, editor, std::slice::from_ref(&path))
            }
            Target::Editor(editor) => {
                open::open_file(&self.config, editor, &project.paths(), &path)
            }
            Target::FileManager => open::reveal(folder),
            Target::Terminal => open::open_terminal(folder),
        };
        self.finish_launch(&project, &target, result, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(path: &Path, extra: Vec<PathBuf>) -> Project {
        Project {
            name: file_name(path),
            path: path.to_path_buf(),
            extra,
            branch: None,
            pinned: false,
            editors: Vec::new(),
            manual: true,
            last_opened: 0,
        }
    }

    #[test]
    fn lists_folders_first_and_navigates() {
        let root = std::env::temp_dir().join(format!("proj-browse-{}", std::process::id()));
        let app = root.join("app");
        fs::create_dir_all(app.join("src")).unwrap();
        fs::create_dir_all(app.join(".git")).unwrap();
        fs::create_dir_all(app.join("Docs")).unwrap();
        fs::write(app.join("README.md"), "").unwrap();
        fs::write(app.join("build.rs"), "").unwrap();

        let mut browse = Browse::new(project(&app, Vec::new()));
        let names: Vec<_> = browse.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Docs", "src", "build.rs", "README.md"]);
        assert_eq!(browse.top_depth(), 1);
        assert_eq!(browse.breadcrumb(), "app");

        browse.stack.push(app.join("src"));
        browse.load();
        assert_eq!(browse.breadcrumb(), "app › src");
        assert_eq!(
            browse.relative(&app.join("src").join("main.rs")),
            format!(
                "app{}src{}main.rs",
                std::path::MAIN_SEPARATOR,
                std::path::MAIN_SEPARATOR
            )
        );

        // A workspace starts at its member folders.
        let sdk = root.join("sdk");
        fs::create_dir_all(&sdk).unwrap();
        let workspace = Browse::new(project(&app, vec![sdk.clone()]));
        assert_eq!(workspace.top_depth(), 0);
        assert_eq!(
            workspace
                .entries
                .iter()
                .map(|e| e.path.clone())
                .collect::<Vec<_>>(),
            [app, sdk]
        );
        fs::remove_dir_all(root).ok();
    }
}
