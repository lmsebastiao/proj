//! Choosing editors: the global default, and per-project "open with" lists.

use gpui::{Context, PathPromptOptions, Window};

use crate::{config, store};

use super::{
    Palette,
    items::{EditorOption, Mode, Target},
    keymap::AddProjects,
};

impl Palette {
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
                // A program picked for this project joins its list.
                self.pick(options, window, cx, move |this, paths, window, cx| {
                    if let Some(path) = paths.into_iter().next() {
                        let command = path.to_string_lossy().into_owned();
                        this.edit_project_editors(&project.key(), |list| {
                            list.push(command.clone())
                        });
                        this.launch(&project, Target::Editor(command), window, cx);
                    }
                });
            }
            EditorOption::FileManager => {}
        }
    }

    /// Ctrl-P in Open-with: add/remove the selected editor for this entry.
    pub(super) fn toggle_project_editor(&mut self, cx: &mut Context<Self>) {
        let (Some(project), Some(editor)) = (
            self.open_with_project().cloned(),
            self.selected_editor().cloned(),
        ) else {
            return;
        };
        let global = self.config.editor.clone();
        let removing = project.editors.contains(&editor.command);
        self.edit_project_editors(&project.key(), |list| {
            if removing {
                store::remove_editor(list, &editor.command, global.as_deref());
            } else {
                store::offer_editor(list, &editor.command, global.as_deref());
            }
        });
        let status = if removing {
            format!("Removed {} from {}", editor.name, project.name)
        } else {
            format!(
                "{} now offers {}",
                project.name,
                self.editor_list(&project.key())
            )
        };
        self.refresh_open_with(&editor.command, status, cx);
    }

    /// Ctrl-Enter in Open-with: make the selected editor this entry's default, or,
    /// on the one that already is, stop using it so the entry falls back to the
    /// global default (or the next editor in its list).
    pub(super) fn toggle_project_default(&mut self, cx: &mut Context<Self>) {
        let (Some(project), Some(editor)) = (
            self.open_with_project().cloned(),
            self.selected_editor().cloned(),
        ) else {
            return;
        };
        let key = project.key();
        let global = self.config.editor.clone();
        let undo = project.editors.first() == Some(&editor.command);
        self.edit_project_editors(&key, |list| {
            if undo {
                store::remove_editor(list, &editor.command, global.as_deref());
            } else {
                store::make_default_editor(list, &editor.command);
            }
        });
        let Some(project) = self.projects.iter().find(|p| p.key() == key) else {
            return;
        };
        let status = if undo {
            let fallback = self.name_of(&project.default_editor(&self.config));
            format!("{} opens in {fallback} again", project.name)
        } else if project.editors.len() > 1 {
            format!(
                "{} is {}'s default; ↵ on it still asks between {}",
                editor.name,
                project.name,
                self.editor_list(&key)
            )
        } else {
            format!("{} now always opens in {}", project.name, editor.name)
        };
        self.refresh_open_with(&editor.command, status, cx);
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

    pub(super) fn edit_project_editors(&mut self, key: &str, edit: impl FnOnce(&mut Vec<String>)) {
        let list = self.db.editors.entry(key.to_string()).or_default();
        edit(list);
        if list.is_empty() {
            self.db.editors.remove(key);
        }
        if let Err(err) = store::save_db(&self.db) {
            self.status = Some(format!("Could not save: {err}").into());
        }
        self.reload_projects();
    }

    pub(super) fn editor_list(&self, key: &str) -> String {
        self.projects
            .iter()
            .find(|p| p.key() == key)
            .map(|p| {
                p.editors
                    .iter()
                    .map(|c| self.name_of(c))
                    .collect::<Vec<_>>()
                    .join(" / ")
            })
            .unwrap_or_default()
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
        self.status = Some(status.into());
        cx.notify();
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
            self.status = Some(format!("Could not save config: {err}").into());
            cx.notify();
            return;
        }
        self.config.editor = Some(command.clone());
        self.reload_projects();
        let label = self.name_of(&command);
        self.back_to_projects(cx);
        let own = self
            .projects
            .iter()
            .filter(|p| !p.editors.is_empty())
            .count();
        self.status = Some(match own {
            0 => format!("All projects now open in {label}").into(),
            1 => format!("Projects now open in {label}, except 1 with its own editor").into(),
            n => format!("Projects now open in {label}, except {n} with their own editor").into(),
        });
        // First run: go straight on to picking projects.
        if self.projects.is_empty() {
            self.add_projects(&AddProjects, window, cx);
        }
    }
}
