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
    switcher,
};

use super::{
    JobResult, LastSearch, Palette, Retry,
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
        let choices =
            switcher::windows_for_project(&project, &self.projects, &self.window_projects);
        match choices.as_slice() {
            [] => self.launch(
                &project,
                Target::Editor(project.default_editor(&self.config)),
                window,
                cx,
            ),
            [ix] => self.switch_to(*ix, window, cx),
            _ => {
                self.choosing_window = Some(project.key());
                self.window_choices = choices;
                self.set_mode(Mode::ProjectWindows, cx);
            }
        }
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
        if matches!(self.list(), List::Browse | List::Files) {
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
        self.finish_launch(project, &target, None, result, window, cx);
    }

    /// Records the open and closes the palette, or shows why launching failed.
    pub(super) fn finish_launch(
        &mut self,
        project: &Project,
        target: &Target,
        file: Option<(PathBuf, bool)>,
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
                        store::record_open(&mut self.db, store::entry_key(&[path]));
                    }
                } else {
                    store::record_open(&mut self.db, key);
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
                let message = format!(
                    "Failed to launch {program} for {}: {err}",
                    paths::display_path(&project.path)
                );
                if let Target::Editor(editor) = target {
                    super::record_job_result(JobResult {
                        message,
                        failed: true,
                        retry: Some(Retry::Editor {
                            project: Some(project.clone()),
                            editor: editor.clone(),
                            file,
                        }),
                    });
                    self.sync_job_results(cx);
                } else {
                    self.problem(message, cx);
                }
            }
        }
    }

    /// Type a new name for the selected entry in the search box.
    pub(super) fn rename(&mut self, cx: &mut Context<Self>) {
        self.edit_selected(Mode::Rename, cx);
    }

    /// Type something about the selected entry in the search box: its name
    /// (`Mode::Rename`) or a command.
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

    /// Clone into the selected parent, after showing the resulting path.
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
            Some(folder) => {
                let scanned = self.config.scan_dirs.contains(&folder);
                self.start_clone(target.url, folder.join(&target.name), scanned, window, cx);
            }
            None => self.choose_destination_for(Mode::Projects, window, cx),
        }
    }

    pub(super) fn retry_job(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(retry) = self
            .job_history
            .get(ix)
            .and_then(|entry| entry.retry.clone())
        else {
            return;
        };
        match retry {
            Retry::Clone { url, dest, scanned } => {
                self.start_clone(url, dest, scanned, window, cx);
            }
            Retry::Template {
                template,
                dest,
                scanned,
            } => {
                let retry = Retry::Template {
                    template: template.clone(),
                    dest: dest.clone(),
                    scanned,
                };
                self.start_making(
                    dest,
                    scanned,
                    retry,
                    move |dest| template.create(dest),
                    window,
                    cx,
                );
            }
            Retry::Editor {
                project,
                editor,
                file,
            } => {
                if project.as_ref().is_some_and(|p| self.say_if_missing(p, cx)) {
                    return;
                }
                let folders = project.as_ref().map(Project::paths).unwrap_or_default();
                let result = match &file {
                    Some((path, true)) => {
                        open::open_with(&self.config, &editor, std::slice::from_ref(path))
                    }
                    Some((path, false)) => open::open_file(&self.config, &editor, &folders, path),
                    None => open::open_with(&self.config, &editor, &folders),
                };
                if let Some(project) = project {
                    self.finish_launch(&project, &Target::Editor(editor), file, result, window, cx);
                } else if let Err(err) = result {
                    if let Some((path, is_dir)) = file {
                        self.unlisted_launch_failure(editor, path, is_dir, &err, cx);
                    }
                } else {
                    window.remove_window();
                }
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
        let retry = Retry::Clone {
            url: url.clone(),
            dest: dest.clone(),
            scanned,
        };
        let job = move |dest: &Path| git::clone(&url, dest);
        self.start_making(dest, scanned, retry, job, window, cx);
    }

    /// Enter on the new project's name: make it in the displayed destination.
    pub(super) fn finish_new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(template) = self.new_from.clone() else {
            return;
        };
        let name = self.query.trim().to_string();
        if let Err(problem) = check_folder_name(&name) {
            return self.problem(problem, cx);
        }
        match self.destination.clone() {
            Some(folder) => {
                let scanned = self.config.scan_dirs.contains(&folder);
                let dest = folder.join(&name);
                let retry = Retry::Template {
                    template: template.clone(),
                    dest: dest.clone(),
                    scanned,
                };
                let job = move |dest: &Path| template.create(dest);
                self.start_making(dest, scanned, retry, job, window, cx);
            }
            None => self.choose_destination_for(Mode::NewProject, window, cx),
        }
    }

    /// Runs `job` (a clone or a copy) to make `dest` in the background, then
    /// lists and opens it. `scanned`: `dest` is in a scan folder.
    fn start_making(
        &mut self,
        dest: PathBuf,
        scanned: bool,
        retry: Retry,
        job: impl FnOnce(&Path) -> io::Result<()> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cloning {
            return;
        }
        let making = Making::of(&retry);
        if dest.exists() {
            return self.problem(existing_destination(&dest), cx);
        }
        self.cloning = true;
        self.say(
            format!("{} {}…", making.doing(), paths::display_path(&dest)),
            cx,
        );
        let work = cx.background_executor().spawn({
            let dest = dest.clone();
            async move {
                if dest.exists() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        existing_destination(&dest),
                    ));
                }
                job(&dest)
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let made = result.is_ok();
            let error = result.as_ref().err().map(ToString::to_string);
            let finished = this.update_in(cx, |this, window, cx| {
                this.finish_making(dest.clone(), scanned, retry.clone(), result, window, cx);
            });
            // Closed with esc meanwhile: still list the new project, and keep
            // its result for the next time the palette opens.
            if finished.is_err() {
                if made {
                    let mut db = store::load_db();
                    list_clone(&mut db, dest.clone(), scanned);
                    match store::save_db(&db) {
                        Ok(()) => super::record_job_result(JobResult {
                            message: format!("{} {}", making.done(), paths::display_path(&dest)),
                            failed: false,
                            retry: None,
                        }),
                        Err(err) => super::record_job_result(JobResult {
                            message: format!(
                                "{} {}, but could not save it to the project list: {err}",
                                making.done(),
                                paths::display_path(&dest)
                            ),
                            failed: true,
                            retry: None,
                        }),
                    }
                } else if let Some(err) = error {
                    super::record_job_result(creation_failure(making, &dest, &err, retry));
                }
            }
        })
        .detach();
    }

    fn finish_making(
        &mut self,
        dest: PathBuf,
        scanned: bool,
        retry: Retry,
        result: io::Result<()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let making = Making::of(&retry);
        self.cloning = false;
        // The job's result replaces the in-progress footer, including on failure.
        self.status = None;
        if let Err(err) = result {
            super::record_job_result(creation_failure(making, &dest, &err.to_string(), retry));
            self.sync_job_results(cx);
            return;
        }
        list_clone(&mut self.db, dest.clone(), scanned);
        self.undo = None;
        if let Err(err) = store::save_db(&self.db) {
            let message = format!(
                "{} {}, but could not save it to the project list: {err}",
                making.done(),
                paths::display_path(&dest)
            );
            self.problem(message.clone(), cx);
            super::record_job_result(JobResult {
                message,
                failed: true,
                retry: None,
            });
            self.sync_job_results(cx);
        }
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
        let paths: Vec<PathBuf> = match (self.selected_file(), self.selected_project()) {
            (Some((_, path, _)), _) => vec![path],
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
    /// name and history.
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
            PaletteCommand::DiscoverProjects => {
                self.set_query("", cx);
                self.discover_projects(window, cx);
            }
            PaletteCommand::ManageWatchedRoots => self.set_mode(Mode::WatchedRoots, cx),
            PaletteCommand::NewFromTemplate | PaletteCommand::ManageTemplates => {
                self.set_mode(Mode::Templates, cx)
            }
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
            PaletteCommand::Reset => self.set_mode(Mode::Reset, cx),
            PaletteCommand::Quit => cx.quit(),
        }
    }

    /// Reset requires an exact typed confirmation because it discards all saved
    /// launcher preferences and project metadata, not the projects on disk.
    pub(super) fn finish_reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.query != "RESET" {
            return self.problem(
                "Type RESET exactly to confirm; no data has been cleared",
                cx,
            );
        }
        if self.autostart {
            if let Err(err) = autostart::set(false) {
                return self.problem(format!("Could not turn off start on login: {err}"), cx);
            }
            self.autostart = false;
        }
        if let Err(err) = paths::clear_saved_files(&config::config_path(), &store::db_path()) {
            return self.problem(format!("Could not clear saved data: {err}"), cx);
        }
        self.config = config::load_config();
        self.db = Db::default();
        self.undo = None;
        self.marked.clear();
        self.git_status.clear();
        self.files = Default::default();
        cx.default_global::<LastSearch>().0 = None;
        self.reload_projects();
        self.set_mode(Mode::Editors, cx);
        self.apply_theme(window, cx);
        self.notice("proj reset; choose your editor to start fresh", cx);
    }

    /// Pick roots once; new immediate subfolders are discovered on each open.
    pub(super) fn discover_projects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let options = PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Watch folders for projects".into()),
        };
        self.pick(options, window, cx, |this, paths, _, cx| {
            if !paths.is_empty() {
                this.watch_folders(&paths, cx);
            }
        });
    }

    pub(super) fn discover_suggested(&mut self, cx: &mut Context<Self>) {
        self.watch_folders(&config::suggested_scan_dirs(), cx);
    }

    fn watch_folders(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        let added = match config::add_scan_dirs(paths) {
            Ok(added) => added,
            Err(err) => return self.problem(format!("Could not save scan folders: {err}"), cx),
        };
        if added.is_empty() {
            return self.notice("Those folders are already watched", cx);
        }
        self.config.scan_dirs.extend(added.iter().cloned());
        self.reload_projects();
        if self.mode == Mode::WatchedRoots {
            self.refilter(cx);
        } else {
            self.set_mode(Mode::Projects, cx);
        }
        let count = self.projects.len();
        self.notice(
            format!("Watching {} folders · {count} projects found", added.len()),
            cx,
        );
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
                self.finish_launch(
                    &project,
                    &Target::Editor(editor),
                    Some((path.clone(), path.is_dir())),
                    result,
                    window,
                    cx,
                );
            }
            (None, Ok(())) => window.remove_window(),
            (None, Err(err)) => {
                self.unlisted_launch_failure(editor, path.clone(), path.is_dir(), &err, cx)
            }
        }
    }

    fn unlisted_launch_failure(
        &mut self,
        editor: String,
        path: PathBuf,
        is_dir: bool,
        error: &io::Error,
        cx: &mut Context<Self>,
    ) {
        super::record_job_result(JobResult {
            message: format!(
                "Failed to launch {} for {}: {error}",
                self.name_of(&editor),
                paths::display_path(&path)
            ),
            failed: true,
            retry: Some(Retry::Editor {
                project: None,
                editor,
                file: Some((path, is_dir)),
            }),
        });
        self.sync_job_results(cx);
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

    pub(super) fn edit_scan_rule(&mut self, ix: usize, mode: Mode, cx: &mut Context<Self>) {
        let Some(root) = self.config.scan_dirs.get(ix).cloned() else {
            return;
        };
        self.editing_root = Some(root);
        self.set_mode(mode, cx);
    }

    pub(super) fn finish_scan_rule(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.editing_root.clone() else {
            return;
        };
        let mut rule = self.config.scan_rule(&root);
        match self.mode {
            Mode::ScanDepth => {
                rule.depth = if self.query.trim().is_empty() {
                    None
                } else {
                    match self.query.trim().parse::<u8>().ok().filter(|d| *d > 0) {
                        Some(depth) => Some(depth),
                        None => {
                            return self
                                .problem("Depth must be from 1 to 255, or empty to inherit", cx);
                        }
                    }
                };
            }
            Mode::ScanExclusions => {
                rule.exclude = self
                    .query
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            _ => return,
        }
        if let Err(err) = config::set_scan_rule(&root, &rule) {
            return self.problem(format!("Could not save scan rule: {err}"), cx);
        }
        let key = root.to_string_lossy().into_owned();
        self.config.scan_rules.retain(|path, _| {
            std::path::Path::new(path) != root && paths::normalize(path).as_deref() != Some(&root)
        });
        if rule.depth.is_some() || !rule.exclude.is_empty() {
            self.config.scan_rules.insert(key, rule);
        }
        self.reload_projects();
        self.set_mode(Mode::WatchedRoots, cx);
        self.select_where(|this, ix| this.config.scan_dirs.get(ix) == Some(&root));
        self.notice(
            format!("Updated scanning for {}", paths::display_path(&root)),
            cx,
        );
    }

    pub(super) fn remove_watched_root(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self
            .matches
            .get(self.selected)
            .and_then(|m| self.config.scan_dirs.get(m.ix))
            .cloned()
        else {
            return;
        };
        match config::remove_scan_dir(&root) {
            Ok(true) => {
                self.config.scan_dirs.retain(|dir| dir != &root);
                self.config.scan_rules.retain(|path, _| {
                    std::path::Path::new(path) != root
                        && paths::normalize(path).as_deref() != Some(&root)
                });
                self.reload_projects();
                self.refilter(cx);
                self.notice(
                    format!(
                        "Stopped watching {} · folder kept",
                        paths::display_path(&root)
                    ),
                    cx,
                );
            }
            Ok(false) => self.problem("That folder is no longer watched", cx),
            Err(err) => self.problem(format!("Could not stop watching: {err}"), cx),
        }
    }

    pub(super) fn choose_template(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(template) = self.templates.get(ix).cloned() {
            self.new_from = Some(template);
            self.destination = self.config.scan_dirs.first().cloned();
            self.set_mode(Mode::NewProject, cx);
        } else if ix == self.templates.len() {
            let options = PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("Add a template folder".into()),
            };
            self.pick(options, window, cx, |this, paths, _, cx| {
                if let Some(path) = paths.into_iter().next() {
                    this.save_template(&path.to_string_lossy(), cx);
                }
            });
        } else {
            self.set_mode(Mode::TemplateUrl, cx);
        }
    }

    pub(super) fn finish_template_url(&mut self, cx: &mut Context<Self>) {
        let url = self.query.trim().to_string();
        if !matches!(
            crate::templates::Template::parse(&url),
            Some(crate::templates::Template::Git(_))
        ) {
            return self.problem("Enter a valid git repository URL", cx);
        }
        self.save_template(&url, cx);
    }

    fn save_template(&mut self, source: &str, cx: &mut Context<Self>) {
        let Some(template) = crate::templates::Template::parse(source) else {
            return self.problem("Not a valid template folder or git URL", cx);
        };
        if let crate::templates::Template::Folder(ref folder) = template
            && !folder.is_dir()
        {
            return self.problem("Template folder does not exist", cx);
        }
        if self.templates.contains(&template) {
            return self.problem("That template is already saved", cx);
        }
        match config::add_template(source) {
            Ok(true) => {
                self.config.templates.push(source.to_string());
                self.set_mode(Mode::Templates, cx);
                self.select_where(|this, ix| this.templates.get(ix) == Some(&template));
                self.notice(format!("Added template {}", template.name()), cx);
            }
            Ok(false) => self.problem("That template is already saved", cx),
            Err(err) => self.problem(format!("Could not save template: {err}"), cx),
        }
    }

    pub(super) fn remove_selected_template(&mut self, cx: &mut Context<Self>) {
        let Some(template) = self
            .matches
            .get(self.selected)
            .and_then(|m| self.templates.get(m.ix))
            .cloned()
        else {
            return;
        };
        let Some(source) = self
            .config
            .templates
            .iter()
            .find(|s| crate::templates::Template::parse(s) == Some(template.clone()))
            .cloned()
        else {
            return self.problem("Template is no longer in config", cx);
        };
        match config::remove_template(&source) {
            Ok(true) => {
                self.config.templates.retain(|s| s != &source);
                self.templates.retain(|t| t != &template);
                self.refilter(cx);
                self.notice(
                    format!("Removed template {} · source kept", template.name()),
                    cx,
                );
            }
            Ok(false) => self.problem("Template is no longer in config", cx),
            Err(err) => self.problem(format!("Could not remove template: {err}"), cx),
        }
    }

    pub(super) fn choose_destination_for(
        &mut self,
        mode: Mode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.destination_for = mode;
        self.destination_query = self.query.clone();
        if self.config.scan_dirs.is_empty() {
            self.pick_destination(window, cx);
        } else {
            self.destination = if mode == Mode::Projects {
                self.clone_candidate.as_ref().and_then(|c| c.into.clone())
            } else {
                self.destination.clone()
            };
            self.set_mode(Mode::Destinations, cx);
            let chosen = self.destination.clone();
            self.select_where(|this, ix| this.config.scan_dirs.get(ix) == chosen.as_ref());
        }
    }

    pub(super) fn choose_destination(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(root) = self.config.scan_dirs.get(ix).cloned() {
            self.destination = Some(root);
            self.return_from_destination(cx);
        } else {
            self.pick_destination(window, cx);
        }
    }

    fn pick_destination(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let options = PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose a parent folder for the new project".into()),
        };
        self.pick(options, window, cx, |this, paths, _, cx| {
            if let Some(folder) = paths.into_iter().next() {
                this.destination = Some(folder);
                this.return_from_destination(cx);
            }
        });
    }

    pub(super) fn return_from_destination(&mut self, cx: &mut Context<Self>) {
        let query = self.destination_query.clone();
        let into = self.destination.clone();
        self.set_mode(self.destination_for, cx);
        self.set_query(&query, cx);
        if self.destination_for == Mode::Projects
            && let Some(target) = self.clone_candidate.as_mut()
        {
            target.into = into;
        }
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

fn existing_destination(dest: &Path) -> String {
    format!(
        "{} already exists. Choose a new destination, or manually inspect/remove the partial folder before retrying; proj will not delete it",
        paths::display_path(dest)
    )
}

fn creation_failure(making: Making, dest: &Path, error: &str, retry: Retry) -> JobResult {
    let source = match &retry {
        Retry::Clone { url, .. }
        | Retry::Template {
            template: crate::templates::Template::Git(url),
            ..
        } => Some(url.as_str()),
        _ => None,
    };
    let error = source.map_or_else(
        || error.to_string(),
        |url| error.replace(url, "[repository URL]"),
    );
    JobResult {
        message: format!(
            "{} in {}: {error}. Retry below; if the folder exists, choose a new destination or manually inspect/remove the partial folder",
            making.failed(),
            paths::display_path(dest)
        ),
        failed: true,
        retry: Some(retry),
    }
}

/// What `start_making` makes a project with, for what the footer says.
#[derive(Clone, Copy)]
enum Making {
    Clone,
    Template,
}

impl Making {
    fn of(retry: &Retry) -> Self {
        match retry {
            Retry::Clone { .. } => Self::Clone,
            Retry::Template { .. } => Self::Template,
            Retry::Editor { .. } => unreachable!("editor launches are not creation jobs"),
        }
    }

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

/// A folder under a watched root is not necessarily discovered: it may be
/// excluded or deeper than that root's scan rule. List it manually in that case.
fn list_clone(db: &mut Db, dest: PathBuf, scanned: bool) {
    if scanned && store::is_scanned(&config::load_config(), &dest) {
        db.hidden.remove(&dest);
    } else {
        store::add_manual(db, dest);
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;

    #[test]
    fn clone_failure_keeps_retry_inputs_without_leaking_credentials() {
        let dest = PathBuf::from("projects/new-app");
        let url = "https://user:secret@example.com/org/new-app.git";
        let result = creation_failure(
            Making::Clone,
            &dest,
            &format!("fatal: repository '{url}' not found"),
            Retry::Clone {
                url: url.into(),
                dest: dest.clone(),
                scanned: true,
            },
        );
        assert!(result.message.contains(&paths::display_path(&dest)));
        assert!(
            result
                .message
                .contains("repository '[repository URL]' not found")
        );
        assert!(!result.message.contains("secret"));
        assert!(result.message.contains("Retry"));
        match result.retry.unwrap() {
            Retry::Clone {
                url: saved,
                dest: saved_dest,
                scanned,
            } => {
                assert_eq!(saved, url);
                assert_eq!(saved_dest, dest);
                assert!(scanned);
            }
            _ => panic!("expected clone retry"),
        }
    }

    #[test]
    fn existing_folder_hint_never_suggests_overwriting() {
        let message = existing_destination(Path::new("projects/partial"));
        assert!(message.contains("projects/partial"));
        assert!(message.contains("Choose a new destination"));
        assert!(message.contains("manually inspect/remove"));
        assert!(message.contains("will not delete"));
    }

    #[test]
    fn template_failure_keeps_source_and_destination_for_retry() {
        let dest = PathBuf::from("projects/new-app");
        let template = crate::templates::Template::Folder(PathBuf::from("templates/starter"));
        let result = creation_failure(
            Making::Template,
            &dest,
            "permission denied",
            Retry::Template {
                template: template.clone(),
                dest: dest.clone(),
                scanned: false,
            },
        );
        assert!(result.message.contains("permission denied"));
        assert!(result.message.contains(&paths::display_path(&dest)));
        match result.retry.unwrap() {
            Retry::Template {
                template: saved,
                dest: saved_dest,
                scanned,
            } => {
                assert_eq!(saved, template);
                assert_eq!(saved_dest, dest);
                assert!(!scanned);
            }
            _ => panic!("expected template retry"),
        }
    }
}
