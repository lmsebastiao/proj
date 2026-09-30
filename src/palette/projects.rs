//! Acting on projects: opening, marking, pinning, renaming, removing, adding and
//! cloning them, plus `>` commands.

use std::{io, path::PathBuf};

use gpui::{ClipboardItem, Context, Focusable, PathPromptOptions, Window};

use crate::{
    autostart, config, git,
    launcher::{self, UpdateState},
    open, paths, platform,
    store::{self, Db, Project},
};

use super::{
    Palette,
    items::{CloneTarget, List, Mode, PaletteCommand, Target},
    keymap::{AddProjects, CopyPath, OpenRemote, Remove, Rename},
};

impl Palette {
    /// Tab / shift-tab: mark/unmark the selected project for opening together,
    /// then move down (`delta` 1) or up (-1). Other lists just move.
    pub(super) fn toggle_mark(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.list() != List::Projects {
            return self.select(delta, cx);
        }
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
        self.select(delta, cx);
    }

    /// Saves the marked projects as a workspace (reusing it if it exists) and returns it.
    pub(super) fn marked_workspace(&mut self, cx: &mut Context<Self>) -> Option<Project> {
        let paths = std::mem::take(&mut self.marked);
        let key = store::remember_workspace(&mut self.db, paths);
        self.save(cx);
        self.reload_projects();
        self.projects.iter().find(|p| p.key() == key).cloned()
    }

    /// Enter on an entry: its own default editor, else the global one.
    pub(super) fn open_entry(
        &mut self,
        project: Project,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = project.default_editor(&self.config);
        self.launch(&project, Target::Editor(editor), window, cx);
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
        if self.list() == List::Browse {
            self.launch_entry(target, window, cx);
        } else if let Some(project) = self.selected_project().cloned() {
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
        self.finish_launch(project, &target, result, window, cx);
    }

    /// Records the open and closes the palette, or shows why launching failed.
    pub(super) fn finish_launch(
        &mut self,
        project: &Project,
        target: &Target,
        result: io::Result<()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(()) => {
                self.db.opened.insert(project.key(), store::now());
                self.save(cx);
                window.remove_window();
            }
            Err(err) => {
                let program = match target {
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

    /// F2: type a new name for the selected entry in the search box.
    pub(super) fn rename(&mut self, _: &Rename, _: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.selected_project().map(Project::key) else {
            return;
        };
        self.renaming = Some(key);
        self.set_mode(Mode::Rename, cx);
    }

    pub(super) fn finish_rename(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.renaming.clone() else {
            return;
        };
        // Unchanged: don't freeze a generated name like "app (client)".
        if self.renamed_project().is_some_and(|p| p.name == self.query) {
            self.set_mode(Mode::Projects, cx);
            self.select_where(|this, ix| this.projects[ix].key() == key);
            return;
        }
        store::rename(&mut self.db, &key, &self.query);
        self.save(cx);
        self.reload_projects();
        self.set_mode(Mode::Projects, cx);
        self.select_where(|this, ix| this.projects[ix].key() == key);
        let verb = if self.db.names.contains_key(&key) {
            "Renamed to"
        } else {
            "Back to"
        };
        let name = self
            .projects
            .iter()
            .find(|p| p.key() == key)
            .map(|p| &p.name);
        self.status = name.map(|name| format!("{verb} {name}").into());
    }

    /// Enter on a pasted git URL: clone it into the first scan folder (or one
    /// picked now), then list and open it.
    pub(super) fn clone_repo(
        &mut self,
        target: CloneTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cloning {
            return;
        }
        match target.into {
            // Folders cloned straight into a scan folder are listed by the scan.
            Some(folder) => {
                self.start_clone(target.url, folder.join(&target.name), true, window, cx)
            }
            None => {
                let options = PathPromptOptions {
                    files: false,
                    directories: true,
                    multiple: false,
                    prompt: Some(format!("Clone {} into", target.name).into()),
                };
                self.pick(options, window, cx, move |this, paths, window, cx| {
                    if let Some(folder) = paths.into_iter().next() {
                        let dest = folder.join(&target.name);
                        this.start_clone(target.url, dest, false, window, cx);
                    }
                });
            }
        }
    }

    fn start_clone(
        &mut self,
        url: String,
        dest: PathBuf,
        scanned: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if dest.exists() {
            self.status = Some(format!("{} already exists", paths::display_path(&dest)).into());
            cx.notify();
            return;
        }
        self.cloning = true;
        self.status = Some(format!("Cloning into {}…", paths::display_path(&dest)).into());
        cx.notify();
        let clone = cx.background_executor().spawn({
            let dest = dest.clone();
            async move { git::clone(&url, &dest) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = clone.await;
            let cloned = result.is_ok();
            let finished = this.update_in(cx, |this, window, cx| {
                this.finish_clone(dest.clone(), scanned, result, window, cx);
            });
            // Closed with esc while cloning: still list the new project.
            if finished.is_err() && cloned {
                let mut db = store::load_db();
                list_clone(&mut db, dest, scanned);
                store::save_db(&db).ok();
            }
        })
        .detach();
    }

    fn finish_clone(
        &mut self,
        dest: PathBuf,
        scanned: bool,
        result: io::Result<()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cloning = false;
        if let Err(err) = result {
            self.status = Some(format!("Clone failed: {err}").into());
            cx.notify();
            return;
        }
        list_clone(&mut self.db, dest.clone(), scanned);
        self.save(cx);
        self.reload_projects();
        let project = self
            .projects
            .iter()
            .find(|p| !p.is_workspace() && p.path == dest)
            .cloned();
        match project {
            Some(project) => self.open_entry(project, window, cx),
            None => {
                self.set_query("", cx);
                self.status = Some(format!("Cloned into {}", paths::display_path(&dest)).into());
            }
        }
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
        let paths: Vec<PathBuf> = match (self.selected_entry(), self.selected_project()) {
            (Some(entry), _) => vec![entry.path.clone()],
            (None, Some(project)) => project.paths(),
            (None, None) => return,
        };
        let text: Vec<String> = paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        cx.write_to_clipboard(ClipboardItem::new_string(text.join("\n")));
        window.remove_window();
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
            PaletteCommand::ChangeEditor => self.choose_default_editor(cx),
            PaletteCommand::OpenConfig => {
                match open::open_project(&self.config, &config::config_path()) {
                    Ok(()) => window.remove_window(),
                    Err(err) => {
                        self.status = Some(format!("Could not open config: {err}").into());
                        cx.notify();
                    }
                }
            }
            // The palette stays open and the row follows along; installing
            // restarts proj.
            PaletteCommand::Update => {
                if !matches!(self.update, UpdateState::Checking | UpdateState::Installing) {
                    launcher::run_update(cx);
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
            store::add_manual(&mut self.db, path.clone());
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

/// Lists a freshly cloned folder: the scan already finds it inside a scan
/// folder (unless an old folder there was hidden); elsewhere it's added by hand.
fn list_clone(db: &mut Db, dest: PathBuf, scanned: bool) {
    if scanned {
        db.hidden.remove(&dest);
    } else {
        store::add_manual(db, dest);
    }
}
