//! Groups: projects opened together in one editor window. Marked with tab in
//! the list, they open together once without being saved; the "Open
//! together" page (ctrl-k) ticks them, and saves them as a group with
//! ctrl-enter; "Folders…" changes a saved group's folders and their order.

use std::path::PathBuf;

use gpui::{Context, Window};

use crate::store::{self, Project};

use super::{
    Palette,
    items::{List, Mode},
};

/// The "Open together" page, or a saved group's "Folders…".
pub(super) struct GroupPage {
    /// The ticked folders, in the order they open in.
    pub(super) ticked: Vec<PathBuf>,
    /// The saved group being changed, as its folders; `None` on "Open together".
    pub(super) editing: Option<Vec<PathBuf>>,
    /// Key of the entry it was opened from, highlighted again on going back.
    pub(super) from: String,
}

impl Palette {
    /// The marked projects as one entry: the saved group with those folders,
    /// else an unsaved one, which is in the list only until it goes back to it.
    pub(super) fn marked_group(&mut self) -> Option<Project> {
        self.group_of(self.marked.clone())
    }

    fn group_of(&mut self, paths: Vec<PathBuf>) -> Option<Project> {
        if paths.len() < 2 {
            return None;
        }
        if let Some(saved) = store::saved_group(&self.db, &paths) {
            let key = store::entry_key(saved);
            if let Some(group) = self.projects.iter().find(|p| p.key() == key) {
                return Some(group.clone());
            }
        }
        let group = store::unsaved_group(paths, &self.db);
        if !self.projects.iter().any(|p| p.key() == group.key()) {
            self.projects.push(group.clone());
            self.match_windows();
        }
        self.unsaved = Some(group.clone());
        Some(group)
    }

    /// Whether `key` is the unsaved group of the marked or ticked projects.
    pub(super) fn is_unsaved(&self, key: &str) -> bool {
        self.unsaved.as_ref().is_some_and(|g| g.key() == key)
    }

    /// Saves the unsaved group `key`, if that's what it is: before giving it
    /// an editor of its own, which is kept with the group.
    pub(super) fn keep_unsaved(&mut self, key: &str) {
        if let Some(group) = self.unsaved.take_if(|g| g.key() == key) {
            store::save_group(&mut self.db, group.paths());
        }
    }

    /// Ctrl-k → "Open together with…" on a project: the page to tick others.
    pub(super) fn open_together_page(&mut self, project: &Project, cx: &mut Context<Self>) {
        self.group_page = Some(GroupPage {
            ticked: vec![project.path.clone()],
            editing: None,
            from: project.key(),
        });
        self.set_mode(Mode::Group, cx);
    }

    /// Ctrl-k → "Folders…" on a saved group: the same page, to change them.
    pub(super) fn edit_group_page(&mut self, group: &Project, cx: &mut Context<Self>) {
        self.group_page = Some(GroupPage {
            ticked: group.paths(),
            editing: Some(group.paths()),
            from: group.key(),
        });
        self.set_mode(Mode::Group, cx);
    }

    /// The highlighted project on the page.
    fn group_candidate(&self) -> Option<&Project> {
        if self.list() != List::Group {
            return None;
        }
        self.matches
            .get(self.selected)
            .map(|m| &self.projects[m.ix])
    }

    /// Tab or space: ticks the highlighted project, or unticks it, then moves
    /// by `delta` (tab: down; space: stays).
    pub(super) fn toggle_tick(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(path) = self.group_candidate().map(|p| p.path.clone()) else {
            return;
        };
        let Some(page) = self.group_page.as_mut() else {
            return;
        };
        match page.ticked.iter().position(|p| p == &path) {
            Some(at) => {
                page.ticked.remove(at);
            }
            None => page.ticked.push(path),
        }
        if delta != 0 {
            self.select(delta, cx);
        }
        cx.notify();
    }

    /// Alt-↑/↓: moves the highlighted ticked folder earlier or later in the
    /// order they open in.
    pub(super) fn move_tick(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(path) = self.group_candidate().map(|p| p.path.clone()) else {
            return;
        };
        let Some(page) = self.group_page.as_mut() else {
            return;
        };
        let Some(at) = page.ticked.iter().position(|p| p == &path) else {
            return self.problem("Tick it first (tab or space) to give it a place", cx);
        };
        let to = at as isize + delta;
        if to >= 0 && (to as usize) < page.ticked.len() {
            page.ticked.swap(at, to as usize);
            cx.notify();
        }
    }

    /// Enter on the page: opens the ticked projects together (without saving
    /// them), or saves a group's changed folders.
    pub(super) fn confirm_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.group_page.as_ref() else {
            return;
        };
        let ticked = page.ticked.clone();
        if let Some(old) = page.editing.clone() {
            return self.save_group_folders(old, ticked, cx);
        }
        if ticked.len() < 2 {
            return self.problem(
                "Tick two projects or more (tab or space) to open them together",
                cx,
            );
        }
        if let Some(group) = self.group_of(ticked) {
            self.open_entry(group, window, cx);
        }
    }

    /// Ctrl-enter on "Open together": saves the ticked projects as a group,
    /// to find in the list next time.
    pub(super) fn save_ticked(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.group_page.as_ref().filter(|p| p.editing.is_none()) else {
            return;
        };
        let ticked = page.ticked.clone();
        if ticked.len() < 2 {
            return self.problem(
                "Tick two projects or more (tab or space) to save a group",
                cx,
            );
        }
        let key = store::save_group(&mut self.db, ticked);
        self.done_with_group(&key, "Saved the group", cx);
    }

    fn save_group_folders(&mut self, old: Vec<PathBuf>, new: Vec<PathBuf>, cx: &mut Context<Self>) {
        if new.len() < 2 {
            return self.problem(
                "A group needs two folders or more. To take it off the list, shift-del on it",
                cx,
            );
        }
        let key = store::change_group(&mut self.db, &old, new);
        self.done_with_group(&key, "Saved the folders of", cx);
    }

    /// Back to the list after saving, with the group `key` highlighted.
    fn done_with_group(&mut self, key: &str, verb: &str, cx: &mut Context<Self>) {
        self.save(cx);
        self.reload_projects();
        self.set_mode(Mode::Projects, cx);
        self.select_where(|this, ix| this.projects[ix].key() == key);
        let name = self
            .projects
            .iter()
            .find(|p| p.key() == key)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        self.notice(format!("{verb} {name}"), cx);
    }

    /// The ticked folders' names in order, for the footer.
    pub(super) fn ticked_names(&self) -> Option<String> {
        let page = self.group_page.as_ref()?;
        let names: Vec<String> = page
            .ticked
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();
        Some(match names.len() {
            0 => "Nothing ticked yet".into(),
            n => format!("{n} ticked · {}", names.join(" + ")),
        })
    }

    /// A group's folders as listed projects (`None` for one that isn't
    /// listed), for their branches and open windows on the group's row.
    pub(super) fn members(&self, group: &Project) -> Vec<(PathBuf, Option<&Project>)> {
        group
            .paths()
            .into_iter()
            .map(|path| {
                let listed = self
                    .projects
                    .iter()
                    .find(|p| !p.is_workspace() && p.path == path);
                (path, listed)
            })
            .collect()
    }
}
