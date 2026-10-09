//! Choosing editors: the global default, and each project's own default.

use gpui::{Context, PathPromptOptions, Window};

use crate::{
    config,
    store::{self, Project},
};

use super::{
    Palette,
    items::{EditorOption, Mode, Target},
};

impl Palette {
    /// Enter in Open-with: open the entry with the chosen editor, just this once.
    pub(super) fn choose_open_with(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.open_with_project().cloned() else {
            return;
        };
        match &self.editors[ix] {
            EditorOption::Detected(editor) => {
                let command = editor.command.clone();
                self.launch(&project, Target::Editor(command), window, cx);
            }
            EditorOption::Browse => {
                let options = PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some("Open with".into()),
                };
                self.pick(options, window, cx, move |this, paths, window, cx| {
                    if let Some(path) = paths.into_iter().next() {
                        let command = path.to_string_lossy().into_owned();
                        this.launch(&project, Target::Editor(command), window, cx);
                    }
                });
            }
            EditorOption::FileManager => {}
        }
    }

    /// Ctrl-Enter in Open-with: make the highlighted editor this entry's default,
    /// or, on the one that already is, go back to the global default. On
    /// "Other…", pick a program to be its default.
    pub(super) fn toggle_project_default(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.open_with_project().cloned() else {
            return;
        };
        match self.matches.get(self.selected).map(|m| &self.editors[m.ix]) {
            Some(EditorOption::Detected(editor)) => {
                let command = editor.command.clone();
                let undo = project.editor.as_ref() == Some(&command);
                self.set_project_editor(&project, (!undo).then_some(command), cx);
            }
            Some(EditorOption::Browse) => {
                let options = PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some(format!("Open {} with", project.name).into()),
                };
                self.pick(options, window, cx, move |this, paths, _, cx| {
                    if let Some(path) = paths.into_iter().next() {
                        let command = path.to_string_lossy().into_owned();
                        this.set_project_editor(&project, Some(command), cx);
                    }
                });
            }
            Some(EditorOption::FileManager) | None => {}
        }
    }

    /// Sets an entry's own default editor (`None`: back to the global one) and
    /// says so, keeping the editor that changed highlighted.
    fn set_project_editor(
        &mut self,
        project: &Project,
        editor: Option<String>,
        cx: &mut Context<Self>,
    ) {
        // Projects opened together get an editor of their own as a group.
        self.keep_unsaved(&project.key());
        store::set_editor(&mut self.db, &project.key(), editor.clone());
        self.undo = None;
        if let Err(err) = store::save_db(&self.db) {
            return self.problem(format!("Could not save: {err}"), cx);
        }
        self.reload_projects();
        let status = match &editor {
            Some(command) => format!("{} now opens in {}", project.name, self.name_of(command)),
            None => format!(
                "{} opens in the default editor again, {}",
                project.name,
                self.name_of(self.config.editor.as_deref().unwrap_or_default())
            ),
        };
        let highlight = editor
            .or_else(|| project.editor.clone())
            .unwrap_or_default();
        self.refresh_open_with(&highlight, status, cx);
    }

    /// The "Change the default editor" command: the editor for every project.
    pub(super) fn choose_default_editor(&mut self, cx: &mut Context<Self>) {
        self.set_mode(Mode::Editors, cx);
        // Start on the current default, so a stray enter changes nothing.
        let current = self.config.editor.clone();
        self.select_where(|this, ix| match &this.editors[ix] {
            EditorOption::Detected(e) => current.as_deref() == Some(e.command.as_str()),
            EditorOption::FileManager => current.as_deref() == Some(""),
            EditorOption::Browse => false,
        });
    }

    /// Rebuilds the Open-with list after an edit, keeping `command` selected.
    pub(super) fn refresh_open_with(
        &mut self,
        command: &str,
        status: String,
        cx: &mut Context<Self>,
    ) {
        self.set_mode(Mode::OpenWith, cx);
        self.select_where(|this, ix| {
            matches!(&this.editors[ix], EditorOption::Detected(e) if e.command == command)
        });
        self.notice(status, cx);
    }

    pub(super) fn choose_editor(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        match &self.editors[ix] {
            EditorOption::Detected(editor) => {
                let command = editor.command.clone();
                self.set_editor(command, window, cx);
            }
            EditorOption::FileManager => self.set_editor(String::new(), window, cx),
            EditorOption::Browse => {
                let options = PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some("Use as editor".into()),
                };
                self.pick(options, window, cx, |this, paths, window, cx| {
                    if let Some(path) = paths.into_iter().next() {
                        this.set_editor(path.to_string_lossy().into_owned(), window, cx);
                    }
                });
            }
        }
    }

    pub(super) fn set_editor(
        &mut self,
        command: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(err) = config::set_editor(&command) {
            return self.problem(format!("Could not save config: {err}"), cx);
        }
        self.config.editor = Some(command.clone());
        self.reload_projects();
        let label = self.name_of(&command);
        self.back_to_projects(cx);
        let own = self.projects.iter().filter(|p| p.editor.is_some()).count();
        let status = match own {
            0 => format!("All projects now open in {label}"),
            1 => format!("Projects now open in {label}, except 1 with its own editor"),
            n => format!("Projects now open in {label}, except {n} with their own editor"),
        };
        self.notice(status, cx);
        // First run: go straight on to importing the projects other editors
        // opened, or picking folders if there are none.
        if self.projects.is_empty() {
            self.import_page(true, window, cx);
        }
    }
}
