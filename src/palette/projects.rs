//! Acting on projects: opening, marking, renaming, removing, adding and
//! cloning them, plus `>` commands.

use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};

use gpui::{ClipboardItem, Context, Focusable, Keystroke, PathPromptOptions, Window};

use crate::{
    autostart, config, git,
    launcher::{self, UpdateState},
    open, paths, platform,
    store::{self, Db, Project},
};

use super::{
    Palette,
    items::{CloneTarget, List, Mode, PaletteCommand, PastedPath, Target},
    keymap::{AddProjects, CopyPath, OpenRemote, UndoRemove},
};

impl Palette {
    /// Tab / shift-tab: mark/unmark the selected project for opening together,
    /// then move down (`delta` 1) or up (-1). Other lists just move.
    pub(super) fn toggle_mark(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.list() == List::Group {
            return self.toggle_tick(delta, cx);
        }
        if self.list() != List::Projects {
            return self.select(delta, cx);
        }
        let Some(project) = self.selected_project().cloned() else {
            return;
        };
        if project.is_workspace() {
            return self.problem("A group can't go in another group", cx);
        }
        match self.marked.iter().position(|p| p == &project.path) {
            Some(pos) => {
                self.marked.remove(pos);
            }
            None => self.marked.push(project.path.clone()),
        }
        self.select(delta, cx);
    }

    /// Ctrl+1…9 in the project list: opens the project in that place, as
    /// numbered while ctrl is held. Returns whether the key was one of those.
    pub(super) fn open_number(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let mods = keystroke.modifiers;
        // Ctrl or alt alone, not both: that's AltGr on Windows.
        let one = (mods.secondary() && !mods.alt)
            || (super::keymap::ALT_ACTIONS && mods.alt && !mods.secondary());
        if self.list() != List::Projects || !one || mods.shift {
            return false;
        }
        let Some(n) = keystroke
            .key
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=9).contains(n))
        else {
            return false;
        };
        if let Some(project) = self.matches.get(n - 1).map(|m| self.projects[m.ix].clone()) {
            self.numbers_shown = false;
            self.open_entry(project, window, cx);
        }
        true
    }

    /// Enter on an entry: to its editor window if one is open, else open it in
    /// its own default editor, else the global one.
    pub(super) fn open_entry(
        &mut self,
        project: Project,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.say_if_missing(&project, cx) {
            return;
        }
        let editor = project.default_editor(&self.config);
        if let Some(ix) = self.open_window_of(&project, &editor) {
            return self.switch_to(ix, window, cx);
        }
        self.launch(&project, Target::Editor(editor), window, cx);
    }

    /// Whether `project`'s folder is gone, saying so if it is.
    pub(super) fn say_if_missing(&mut self, project: &Project, cx: &mut Context<Self>) -> bool {
        if !project.missing {
            return false;
        }
        let gone = project
            .paths()
            .into_iter()
            .find(|p| !p.is_dir())
            .unwrap_or_else(|| project.path.clone());
        self.problem(
            format!(
                "{} isn't there anymore. Reconnect its drive, or remove it with shift-del",
                paths::display_path(&gone),
            ),
            cx,
        );
        true
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
        if self.say_if_missing(project, cx) {
            return;
        }
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
                let key = project.key();
                if self.is_unsaved(&key) {
                    // Not a group: each of its projects was opened.
                    for path in project.paths() {
                        self.db
                            .opened
                            .insert(store::entry_key(&[path]), store::now());
                    }
                } else {
                    self.db.opened.insert(key, store::now());
                }
                self.save(cx);
                window.remove_window();
            }
            Err(err) => {
                let program = match target {
                    Target::Editor(editor) => self.name_of(editor),
                    Target::FileManager => "the file manager".into(),
                    Target::Terminal => "a terminal".into(),
                };
                self.problem(format!("Failed to launch {program}: {err}"), cx);
            }
        }
    }

    /// Type a new name for the selected entry in the search box.
    pub(super) fn rename(&mut self, cx: &mut Context<Self>) {
        self.edit_selected(Mode::Rename, cx);
    }

    /// Type something about the selected entry in the search box: its name
    /// (`Mode::Rename`), tags or a command.
    pub(super) fn edit_selected(&mut self, mode: Mode, cx: &mut Context<Self>) {
        let Some(key) = self.selected_project().map(Project::key) else {
            return;
        };
        self.editing = Some(key);
        self.set_mode(mode, cx);
    }

    /// Back to the project list after typing, with `key` selected and `status` said.
    fn done_editing(&mut self, key: &str, status: Option<String>, cx: &mut Context<Self>) {
        self.save(cx);
        self.reload_projects();
        self.set_mode(Mode::Projects, cx);
        self.select_where(|this, ix| this.projects[ix].key() == key);
        if let Some(status) = status {
            self.notice(status, cx);
        }
    }

    pub(super) fn finish_tags(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.editing.clone() else {
            return;
        };
        store::set_tags(&mut self.db, &key, &self.query);
        let status = match self.db.tags.get(&key) {
            Some(tags) => {
                let tags: Vec<String> = tags.iter().map(|t| format!("#{t}")).collect();
                format!("Tagged {}", tags.join(" "))
            }
            None => "No tags".into(),
        };
        self.done_editing(&key, Some(status), cx);
    }

    pub(super) fn finish_add_command(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.editing.clone() else {
            return;
        };
        let command = self.query.trim().to_string();
        if command.is_empty() {
            return self.back_to_projects(cx);
        }
        store::add_command(&mut self.db, &key, &command);
        let status = format!(
            "Added \"{command}\" to its commands ({}-r)",
            super::secondary()
        );
        self.done_editing(&key, Some(status), cx);
    }

    pub(super) fn finish_rename(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.editing.clone() else {
            return;
        };
        // Unchanged: don't freeze a generated name like "app (client)".
        if self.edited_project().is_some_and(|p| p.name == self.query) {
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
            .map(|p| p.name.clone());
        if let Some(name) = name {
            self.notice(format!("{verb} {name}"), cx);
        }
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
        let job = move |dest: &Path| git::clone(&url, dest);
        self.start_making(dest, scanned, Making::Clone, job, window, cx);
    }

    /// Enter on the new project's name: make it from `new_from` in the first
    /// scan folder (or one picked now), then list and open it.
    pub(super) fn finish_new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(template) = self.new_from.clone() else {
            return;
        };
        let name = self.query.trim().to_string();
        if let Err(problem) = check_folder_name(&name) {
            return self.problem(problem, cx);
        }
        let job = move |dest: &Path| template.create(dest);
        match self.config.scan_dirs.first().cloned() {
            Some(folder) => {
                self.start_making(folder.join(&name), true, Making::Template, job, window, cx);
            }
            None => {
                let options = PathPromptOptions {
                    files: false,
                    directories: true,
                    multiple: false,
                    prompt: Some(format!("Make {name} in").into()),
                };
                self.pick(options, window, cx, move |this, paths, window, cx| {
                    if let Some(folder) = paths.into_iter().next() {
                        let dest = folder.join(&name);
                        this.start_making(dest, false, Making::Template, job, window, cx);
                    }
                });
            }
        }
    }

    /// Runs `job` (a clone or a copy) to make `dest` in the background, then
    /// lists and opens it. `scanned`: `dest` is in a scan folder.
    fn start_making(
        &mut self,
        dest: PathBuf,
        scanned: bool,
        making: Making,
        job: impl FnOnce(&Path) -> io::Result<()> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cloning {
            return;
        }
        if dest.exists() {
            return self.problem(format!("{} already exists", paths::display_path(&dest)), cx);
        }
        self.cloning = true;
        self.say(
            format!("{} {}…", making.doing(), paths::display_path(&dest)),
            cx,
        );
        let work = cx.background_executor().spawn({
            let dest = dest.clone();
            async move { job(&dest) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let made = result.is_ok();
            let finished = this.update_in(cx, |this, window, cx| {
                this.finish_making(dest.clone(), scanned, making, result, window, cx);
            });
            // Closed with esc meanwhile: still list the new project.
            if finished.is_err() && made {
                let mut db = store::load_db();
                list_clone(&mut db, dest, scanned);
                store::save_db(&db).ok();
            }
        })
        .detach();
    }

    fn finish_making(
        &mut self,
        dest: PathBuf,
        scanned: bool,
        making: Making,
        result: io::Result<()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cloning = false;
        if let Err(err) = result {
            return self.problem(format!("{}: {err}", making.failed()), cx);
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
                self.set_mode(Mode::Projects, cx);
                self.say(
                    format!("{} {}", making.done(), paths::display_path(&dest)),
                    cx,
                );
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
            return self.problem(format!("{} has no git remote", project.name), cx);
        };
        match open::open_url(&url) {
            Ok(()) => window.remove_window(),
            Err(err) => self.problem(format!("Could not open {url}: {err}"), cx),
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
        let what = if text.len() == 1 {
            "the path"
        } else {
            "the paths"
        };
        self.copy_and_close(text.join("\n"), what, window, cx);
    }

    /// Copies `text`, says so for a moment, then closes, so the copy is
    /// seen to have happened.
    pub(super) fn copy_and_close(
        &mut self,
        text: String,
        what: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.close_menu(cx);
        self.closing = true;
        self.say(format!("Copied {what}"), cx);
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(600))
                .await;
            this.update_in(cx, |_, window, _| window.remove_window())
                .ok();
        })
        .detach();
    }

    pub(super) fn remove(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.selected_project().cloned() else {
            return;
        };
        let before = self.db.clone();
        store::forget_entry(&mut self.db, &project);
        self.save(cx);
        self.undo = Some((before, project.name.clone()));
        let key = project.key();
        self.projects.retain(|p| p.key() != key);
        let selected = self.selected;
        self.refilter(cx);
        self.selected = selected.min(self.matches.len().saturating_sub(1));
        self.say_removed(&project.name, cx);
    }

    /// "Removed proj", with an Undo button (and ctrl-z), up for longer than
    /// other notices.
    fn say_removed(&mut self, what: &str, cx: &mut Context<Self>) {
        let text = format!("Removed {what}");
        self.show_status(text, false, true, Some(Duration::from_secs(8)), cx);
    }

    /// Ctrl-Z after a remove: puts back what it took off the list, with its
    /// name, tags and history.
    pub(super) fn undo_remove(&mut self, _: &UndoRemove, _: &mut Window, cx: &mut Context<Self>) {
        self.undo_last_remove(cx);
    }

    /// Ctrl-z, or the Undo button after a remove.
    pub(super) fn undo_last_remove(&mut self, cx: &mut Context<Self>) {
        let Some((db, what)) = self.undo.take() else {
            return;
        };
        self.db = db;
        self.save(cx);
        self.reload_projects();
        if self.mode == Mode::Projects {
            self.refilter(cx);
        }
        self.notice(format!("Put back {what}"), cx);
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
                    self.notice(format!("Start on login turned {state}"), cx);
                }
                Err(err) => self.problem(format!("Could not change start on login: {err}"), cx),
            },
            PaletteCommand::AddProjects => {
                self.set_query("", cx);
                self.add_projects(&AddProjects, window, cx);
            }
            PaletteCommand::NewFromTemplate => self.set_mode(Mode::Templates, cx),
            PaletteCommand::RemoveMissing => {
                let missing: Vec<Project> = self
                    .projects
                    .iter()
                    .filter(|p| p.missing)
                    .cloned()
                    .collect();
                let before = self.db.clone();
                for project in &missing {
                    store::forget_entry(&mut self.db, project);
                }
                self.save(cx);
                let projects = if missing.len() == 1 {
                    "project"
                } else {
                    "projects"
                };
                let what = format!("{} missing {projects}", missing.len());
                self.undo = Some((before, what.clone()));
                self.reload_projects();
                self.set_query("", cx);
                self.say_removed(&what, cx);
            }
            PaletteCommand::ChangeEditor => self.choose_default_editor(cx),
            // Saved to config.toml and applied right away; the palette stays open
            // so it can be pressed again.
            PaletteCommand::Theme => {
                let theme = self.config.theme.next();
                if let Err(err) = config::set_theme(theme) {
                    return self.problem(format!("Could not save config: {err}"), cx);
                }
                self.config.theme = theme;
                self.apply_theme(window, cx);
            }
            PaletteCommand::OpenConfig => {
                match open::open_project(&self.config, &config::config_path()) {
                    Ok(()) => window.remove_window(),
                    Err(err) => self.problem(format!("Could not open config: {err}"), cx),
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

    /// Enter (`add`) or ctrl-enter on a pasted path. A folder opens in the
    /// default editor, listed first unless it's ctrl-enter; a file opens
    /// with the listed project it's in, if any, so it lands in its window.
    pub(super) fn open_pasted(&mut self, add: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pasted) = self.pasted.clone() else {
            return;
        };
        let (path, folders, project) = match pasted {
            PastedPath::Folder(path) if add => {
                store::add_manual(&mut self.db, path.clone());
                self.save(cx);
                self.reload_projects();
                self.refilter(cx);
                let listed = self
                    .projects
                    .iter()
                    .find(|p| !p.is_workspace() && p.path == path)
                    .cloned();
                if let Some(project) = listed {
                    self.open_entry(project, window, cx);
                }
                return;
            }
            PastedPath::Folder(path) => (path, Vec::new(), None),
            PastedPath::File(path) => {
                // The innermost, if projects are inside one another.
                let project = self
                    .projects
                    .iter()
                    .filter(|p| !p.is_workspace() && path.starts_with(&p.path))
                    .max_by_key(|p| p.path.components().count())
                    .cloned();
                let folders = project.as_ref().map(Project::paths).unwrap_or_default();
                (path, folders, project)
            }
        };
        let editor = match &project {
            Some(project) => project.default_editor(&self.config),
            None => self.config.editor.clone().unwrap_or_default(),
        };
        let result = if path.is_dir() {
            open::open_with(&self.config, &editor, std::slice::from_ref(&path))
        } else {
            open::open_file(&self.config, &editor, &folders, &path)
        };
        match (project, result) {
            (Some(project), result) => {
                self.finish_launch(&project, &Target::Editor(editor), result, window, cx);
            }
            (None, Ok(())) => window.remove_window(),
            (None, Err(err)) => {
                let program = self.name_of(&editor);
                self.problem(format!("Failed to launch {program}: {err}"), cx);
            }
        }
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
        // `window_projects` points into the old order.
        self.match_windows();
        self.refilter(cx);
        let added = match added.as_slice() {
            [one] => format!("Added {}", paths::display_path(one)),
            many => format!("Added {} projects", many.len()),
        };
        self.notice(added, cx);
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

/// What `start_making` makes a project with, for what the footer says.
#[derive(Clone, Copy)]
enum Making {
    Clone,
    Template,
}

impl Making {
    fn doing(self) -> &'static str {
        match self {
            Self::Clone => "Cloning into",
            Self::Template => "Making",
        }
    }

    fn failed(self) -> &'static str {
        match self {
            Self::Clone => "Clone failed",
            Self::Template => "Could not make the project",
        }
    }

    fn done(self) -> &'static str {
        match self {
            Self::Clone => "Cloned into",
            Self::Template => "Made",
        }
    }
}

/// A name that works as a folder name everywhere (Windows is the pickiest).
fn check_folder_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("Type a name for the new project");
    }
    if name == "." || name == ".." || name.ends_with(['.', ' ']) {
        return Err("A folder name can't be that, or end with a dot or a space");
    }
    if name.contains(['<', '>', ':', '"', '/', '\\', '|', '?', '*'])
        || name.contains(char::is_control)
    {
        return Err("A folder name can't have any of < > : \" / \\ | ? *");
    }
    Ok(())
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
