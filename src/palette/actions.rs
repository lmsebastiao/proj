//! A project's actions: the ctrl-k menu, and the icons on the highlighted row.

use gpui::{Context, Window};

use crate::store::Project;

use super::{
    Palette,
    items::{List, Mode, ProjectAction, Target},
    keymap::{CopyPath, OpenRemote},
};

/// The icons on a project row.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum RowIcon {
    Pin,
    Rename,
    Remove,
    /// Opens the actions menu.
    More,
}

/// Left to right. Any order works: rows that aren't highlighted still show a
/// pinned project's pin, at the end.
pub(super) const ROW_ICONS: [RowIcon; 4] = [
    RowIcon::Rename,
    RowIcon::Remove,
    RowIcon::More,
    RowIcon::Pin,
];

impl RowIcon {
    /// For the icon's element id, with the row number.
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Pin => "pin",
            Self::Rename => "rename",
            Self::Remove => "remove",
            Self::More => "more",
        }
    }
}

impl Palette {
    /// Ctrl-K: the actions menu for the selected project.
    pub(super) fn show_actions(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.selected_project().map(Project::key) else {
            return;
        };
        self.actions_for = Some(key);
        self.set_mode(Mode::Actions, cx);
    }

    pub(super) fn actions_project(&self) -> Option<&Project> {
        let key = self.actions_for.as_ref()?;
        self.projects.iter().find(|p| &p.key() == key)
    }

    /// Runs a menu action. The actions work on the selected project, so this
    /// goes back to the project list with it selected first.
    pub(super) fn run_action(
        &mut self,
        action: ProjectAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.actions_for.clone() else {
            return;
        };
        self.back_to_projects(cx);
        if self.selected_project().map(Project::key) != Some(key.clone()) {
            return;
        }
        match action {
            ProjectAction::OpenWith => self.show_open_with(key, cx),
            ProjectAction::ShowInFileManager => {
                self.open_selected(Target::FileManager, window, cx);
            }
            ProjectAction::Terminal => self.open_selected(Target::Terminal, window, cx),
            ProjectAction::TogglePin => self.toggle_pin(cx),
            ProjectAction::Rename => self.rename(cx),
            ProjectAction::CopyPath => self.copy_path(&CopyPath, window, cx),
            ProjectAction::RepoPage => self.open_remote(&OpenRemote, window, cx),
            ProjectAction::Remove => self.remove(cx),
        }
    }

    /// A click on one of a project row's icons. Remove asks for a second click,
    /// as a stray click is easier than a stray key.
    pub(super) fn click_row_icon(&mut self, row: usize, icon: RowIcon, cx: &mut Context<Self>) {
        if self.list() != List::Projects {
            return;
        }
        let key = self.matches.get(row).map(|m| self.projects[m.ix].key());
        if row != self.selected {
            self.selected = row;
            self.confirm_remove = None;
        }
        match icon {
            RowIcon::Pin => self.toggle_pin(cx),
            RowIcon::Rename => self.rename(cx),
            RowIcon::More => self.show_actions(cx),
            RowIcon::Remove if self.confirm_remove.is_some() && self.confirm_remove == key => {
                self.remove(cx);
            }
            RowIcon::Remove => {
                let name = self
                    .selected_project()
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                self.status = Some(format!("Click the bin again to remove {name}").into());
                self.confirm_remove = key;
                cx.notify();
            }
        }
    }
}
