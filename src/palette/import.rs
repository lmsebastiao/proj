//! Importing the projects other editors opened lately (`recent`): a page
//! listing them, all ticked, to untick the ones that aren't projects and add
//! the rest. Shown on first run, after choosing the editor, and by the `>`
//! command.

use gpui::{Context, Window};

use crate::{
    recent::{self, Found},
    store,
};

use super::{
    Palette,
    items::{List, Mode},
    keymap::AddProjects,
};

/// The import page's list, while it's open.
#[derive(Default)]
pub(super) struct ImportPage {
    /// `None` while the editors' histories are being read.
    pub(super) found: Option<Vec<Found>>,
    /// Whether each of `found` is ticked.
    pub(super) ticked: Vec<bool>,
    /// First run: with nothing found, go on to picking folders instead.
    first_run: bool,
}

impl Palette {
    /// The import page: reads the editors' histories in the background, then
    /// lists what isn't listed yet, all ticked.
    pub(super) fn import_page(&mut self, first_run: bool, window: &Window, cx: &mut Context<Self>) {
        self.import = Some(ImportPage {
            first_run,
            ..ImportPage::default()
        });
        self.set_mode(Mode::Import, cx);
        let listed = store::listed_folders(&self.config, &self.db);
        let read = cx
            .background_executor()
            .spawn(async move { recent::found(&listed) });
        cx.spawn_in(window, async move |this, cx| {
            let found = read.await;
            this.update_in(cx, |this, window, cx| {
                let Some(page) = this.import.as_mut().filter(|_| this.mode == Mode::Import) else {
                    return;
                };
                if found.is_empty() && page.first_run {
                    this.back_to_projects(cx);
                    return this.add_projects(&AddProjects, window, cx);
                }
                page.ticked = vec![true; found.len()];
                page.found = Some(found);
                this.refilter(cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn found_projects(&self) -> &[Found] {
        self.import
            .as_ref()
            .and_then(|page| page.found.as_deref())
            .unwrap_or_default()
    }

    pub(super) fn is_ticked(&self, ix: usize) -> bool {
        self.import
            .as_ref()
            .is_some_and(|page| page.ticked.get(ix).copied().unwrap_or(false))
    }

    /// Tab or space: ticks or unticks the highlighted one, then moves by
    /// `delta` (tab: down; space: stays).
    pub(super) fn toggle_import(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(ix) = self.matches.get(self.selected).map(|m| m.ix) else {
            return;
        };
        if let Some(tick) = self.import.as_mut().and_then(|p| p.ticked.get_mut(ix)) {
            *tick = !*tick;
        }
        if delta != 0 {
            self.select(delta, cx);
        }
        cx.notify();
    }

    /// Ctrl-enter: ticks them all, or unticks them all when they all are.
    pub(super) fn toggle_all_imports(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.import.as_mut() {
            let all = page.ticked.iter().all(|t| *t);
            page.ticked.iter_mut().for_each(|t| *t = !all);
        }
        cx.notify();
    }

    /// Enter: adds the ticked ones, a Zed workspace's folders as a group too.
    pub(super) fn confirm_import(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.import.take() else {
            return;
        };
        let chosen: Vec<Found> = page
            .found
            .unwrap_or_default()
            .into_iter()
            .zip(page.ticked)
            .filter_map(|(found, ticked)| ticked.then_some(found))
            .collect();
        if chosen.is_empty() {
            self.back_to_projects(cx);
            return self.notice("Nothing imported", cx);
        }
        let count = chosen.len();
        for found in chosen {
            store::add_found(&mut self.db, found.paths);
        }
        self.save(cx);
        self.reload_projects();
        self.set_mode(Mode::Projects, cx);
        let projects = if count == 1 { "project" } else { "projects" };
        self.notice(format!("Imported {count} {projects}"), cx);
    }

    /// What the footer says on the import page.
    pub(super) fn import_summary(&self) -> Option<String> {
        let page = self.import.as_ref()?;
        let found = page.found.as_ref()?;
        let ticked = page.ticked.iter().filter(|t| **t).count();
        Some(format!("{ticked} of {} ticked", found.len()))
    }

    /// Whether the import page is still reading the editors' histories.
    pub(super) fn import_loading(&self) -> bool {
        self.list() == List::Import && self.import.as_ref().is_some_and(|p| p.found.is_none())
    }
}
