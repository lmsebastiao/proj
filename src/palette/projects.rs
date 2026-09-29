//! Acting on projects: opening, marking, pinning, removing and adding them, plus `>` commands.

use std::path::PathBuf;

use gpui::{ClipboardItem, Context, Focusable, PathPromptOptions, Window};

use crate::{
    autostart, config, git, open, paths, platform,
    store::{self, Project},
};

use super::{
    Palette,
    items::{Mode, PaletteCommand, Target},
    keymap::{AddProjects, CopyPath, OpenRemote, Remove},
};

impl Palette {
    /// Tab: mark/unmark the selected project for opening together, then move down.
    pub(super) fn toggle_mark(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.selected_project().cloned() else {
            return;
        };
        if project.is_workspace() {
            self.status = Some("Workspaces can't be combined further".into());
            cx.notify();
            return;
        }
        match self.marked.iter().position(|p| p == &project.path) {
            Some(pos) => {
                self.marked.remove(pos);
            }
            None => self.marked.push(project.path.clone()),
        }
        self.select(1, cx);
    }

    /// Saves the marked projects as a workspace (reusing it if it exists) and returns it.
    pub(super) fn marked_workspace(&mut self, cx: &mut Context<Self>) -> Option<Project> {
        let paths = std::mem::take(&mut self.marked);
        let key = store::remember_workspace(&mut self.db, paths);
        self.save(cx);
        self.reload_projects();
        self.projects.iter().find(|p| p.key() == key).cloned()
    }

    /// Enter on an entry: its only editor, the global one, or ask when it has several.
    pub(super) fn open_entry(
        &mut self,
        project: Project,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match project.editors.as_slice() {
            [] => {
                let editor = self.config.editor.clone().unwrap_or_default();
                self.launch(&project, Target::Editor(editor), window, cx);
            }
            [only] => self.launch(&project, Target::Editor(only.clone()), window, cx),
            _ => self.show_open_with(project.key(), cx),
        }
    }

    pub(super) fn show_open_with(&mut self, key: String, cx: &mut Context<Self>) {
        self.open_with = Some(key);
        self.set_mode(Mode::OpenWith, cx);
    }

    pub(super) fn open_selected(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(project) = self.selected_project().cloned() {
            self.launch(&project, target, window, cx);
        }
    }

    pub(super) fn launch(
        &mut self,
        project: &Project,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = match &target {
            Target::Editor(editor) => open::open_with(&self.config, editor, &project.paths()),
            Target::FileManager => open::reveal(&project.path),
            Target::Terminal => open::open_terminal(&project.path),
        };
        match result {
            Ok(()) => {
                self.db.opened.insert(project.key(), store::now());
                self.save(cx);
                window.remove_window();
            }
            Err(err) => {
                let program = match &target {
                    Target::Editor(editor) => self.name_of(editor),
                    Target::FileManager => "the file manager".into(),
                    Target::Terminal => "a terminal".into(),
                };
                self.status = Some(format!("Failed to launch {program}: {err}").into());
                cx.notify();
            }
        }
    }

    pub(super) fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.selected_project().cloned() else {
            return;
        };
        let key = project.key();
        let pinned = !project.pinned;
        if pinned {
            self.db.pinned.insert(key.clone());
        } else {
            self.db.pinned.remove(&key);
        }
        self.save(cx);
        self.reload_projects();
        self.refilter(cx);
        // Keep the selection on the same entry after re-sorting.
        self.select_where(|this, ix| this.projects[ix].key() == key);
        let verb = if pinned { "Pinned" } else { "Unpinned" };
        self.status = Some(format!("{verb} {}", project.name).into());
    }

    /// Ctrl-G: the repository's web page (GitHub, GitLab, Gitea…), from the origin remote.
    pub(super) fn open_remote(
        &mut self,
        _: &OpenRemote,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.selected_project().cloned() else {
            return;
        };
        // For a workspace, the first folder's repository.
        let Some(url) = git::git_web_url(&project.path) else {
            self.status = Some(format!("{} has no git remote", project.name).into());
            cx.notify();
            return;
        };
        match open::open_url(&url) {
            Ok(()) => window.remove_window(),
            Err(err) => {
                self.status = Some(format!("Could not open {url}: {err}").into());
                cx.notify();
            }
        }
    }

    pub(super) fn copy_path(&mut self, _: &CopyPath, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(project) = self.selected_project() {
            let paths: Vec<String> = project
                .paths()
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            cx.write_to_clipboard(ClipboardItem::new_string(paths.join("\n")));
            window.remove_window();
        }
    }

    pub(super) fn remove(&mut self, _: &Remove, _: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.selected_project().cloned() else {
            return;
        };
        store::forget_entry(&mut self.db, &project);
        self.save(cx);
        let key = project.key();
        self.projects.retain(|p| p.key() != key);
        let selected = self.selected;
        self.refilter(cx);
        self.selected = selected.min(self.matches.len().saturating_sub(1));
        self.status = Some(format!("Removed {}", project.name).into());
    }

    pub(super) fn run_command(
        &mut self,
        command: PaletteCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            PaletteCommand::Autostart => match autostart::set(!self.autostart) {
                Ok(()) => {
                    self.autostart = !self.autostart;
                    let state = if self.autostart { "on" } else { "off" };
                    self.status = Some(format!("Start on login turned {state}").into());
                    cx.notify();
                }
                Err(err) => {
                    self.status = Some(format!("Could not change start on login: {err}").into());
                    cx.notify();
                }
            },
            PaletteCommand::AddProjects => {
                self.set_query("", cx);
                self.add_projects(&AddProjects, window, cx);
            }
            PaletteCommand::ChangeEditor => self.set_mode(Mode::Editors, cx),
            PaletteCommand::OpenConfig => {
                match open::open_project(&self.config, &config::config_path()) {
                    Ok(()) => window.remove_window(),
                    Err(err) => {
                        self.status = Some(format!("Could not open config: {err}").into());
                        cx.notify();
                    }
                }
            }
            PaletteCommand::Quit => cx.quit(),
        }
    }

    pub(super) fn add_projects(
        &mut self,
        _: &AddProjects,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let options = PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Add projects".into()),
        };
        self.pick(options, window, cx, |this, paths, _, cx| {
            this.add_paths(paths, cx)
        });
    }

    pub(super) fn add_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let mut added = Vec::new();
        for path in paths {
            let Some(path) = paths::normalize(&path.to_string_lossy()).filter(|p| p.is_dir())
            else {
                continue;
            };
            self.db.hidden.remove(&path);
            if !self.db.manual.contains(&path) {
                self.db.manual.push(path.clone());
            }
            added.push(path);
        }
        if added.is_empty() {
            return;
        }
        self.save(cx);
        self.reload_projects();
        if self.mode != Mode::Projects {
            self.set_mode(Mode::Projects, cx);
        } else {
            self.set_query("", cx);
        }
        // Put the newly added projects first so they can be opened right away.
        self.projects.sort_by_key(|p| !added.contains(&p.path));
        self.refilter(cx);
        self.status = Some(match added.as_slice() {
            [one] => format!("Added {}", paths::display_path(one)).into(),
            many => format!("Added {} projects", many.len()).into(),
        });
    }

    /// Shows a native file dialog, keeping the palette open while it's up.
    pub(super) fn pick(
        &mut self,
        options: PathPromptOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, Vec<PathBuf>, &mut Window, &mut Context<Self>) + 'static,
    ) {
        self.picking = true;
        let paths = cx.prompt_for_paths(options);
        cx.spawn_in(window, async move |this, cx| {
            let paths = paths
                .await
                .ok()
                .and_then(Result::ok)
                .flatten()
                .unwrap_or_default();
            this.update_in(cx, |this, window, cx| {
                this.picking = false;
                then(this, paths, window, cx);
                window.focus(&this.input.focus_handle(cx));
                platform::raise(window);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
