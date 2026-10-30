//! Every shortcut of the current list. The footer has buttons for the main
//! two; F1 (or the footer's ? button) lists them all in a dropdown, where
//! clicking one runs it.

use gpui::Action;

use super::{
    Palette,
    actions::MenuKind,
    items::{List, Mode, ProjectAction},
    keymap::*,
    secondary,
};

pub(super) struct Shortcut {
    /// All keys that do it; the footer shows the first.
    pub(super) keys: Vec<String>,
    /// What it does, in the dropdown.
    pub(super) action: &'static str,
    /// Its button's label when the footer has a button for it.
    pub(super) footer: Option<&'static str>,
    /// Run when its row is clicked. `None` for keys like → that only make sense typed.
    pub(super) run: Option<Box<dyn Action>>,
}

impl Shortcut {
    fn new(keys: &[&str], action: &'static str) -> Self {
        let m = secondary();
        Self {
            keys: keys.iter().map(|k| k.replace("mod", m)).collect(),
            action,
            footer: None,
            run: None,
        }
    }

    fn footer(mut self, label: &'static str) -> Self {
        self.footer = Some(label);
        self
    }

    /// Also lists the alt form of its ctrl key ("ctrl-e", then "alt-e"),
    /// for the project actions that have one.
    fn or_alt(mut self) -> Self {
        let ctrl = format!("{}-", secondary());
        if ALT_ACTIONS && let Some(at) = self.keys.iter().position(|k| k.starts_with(&ctrl)) {
            let alt = alt_form(&self.keys[at]);
            self.keys.insert(at + 1, alt);
        }
        self
    }

    fn run(mut self, action: impl Action) -> Self {
        self.run = Some(Box::new(action));
        self
    }
}

/// "ctrl-e" as "alt-e", "ctrl-shift-p" as "alt-p".
fn alt_form(key: &str) -> String {
    let rest = key.rsplit('-').next().unwrap_or(key);
    format!("alt-{rest}")
}

/// "Show in Explorer" and the footer's short name for it.
pub(super) fn file_manager() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("Show in Explorer", "Explorer")
    } else if cfg!(target_os = "macos") {
        ("Show in Finder", "Finder")
    } else {
        ("Show in the file manager", "Folder")
    }
}

impl Palette {
    /// The current list's keys. At most two have a `footer` label: those are
    /// the footer's buttons, the same two whatever is highlighted, so they
    /// don't move about.
    pub(super) fn shortcuts(&self) -> Vec<Shortcut> {
        let s = Shortcut::new;
        let (reveal, reveal_short) = file_manager();
        if self.menu_open() {
            let added = match self.selected_action() {
                Some(ProjectAction::Run(i)) => self.tasks.get(i).is_some_and(|t| t.added),
                _ => false,
            };
            let mut keys = vec![s(&["↵"], "Run it").footer("Run").run(Confirm)];
            let own_key = match self.menu_kind {
                MenuKind::Actions => "mod-k",
                MenuKind::Commands => "mod-r",
            };
            if added {
                keys.push(s(&["shift-del"], "Remove the added command").run(RemoveItem));
            }
            keys.extend([
                s(&["↑ ↓"], "Move the selection"),
                s(&["esc", own_key], "Close the menu")
                    .or_alt()
                    .footer("Close")
                    .run(Dismiss),
            ]);
            return keys;
        }
        match self.list() {
            List::Projects => {
                let marking = !self.marked.is_empty();
                let window_count = self.selected_project().map_or(0, |p| {
                    crate::switcher::windows_for_project(p, &self.projects, &self.window_projects)
                        .len()
                });
                let (open, open_short) = if self.marked.len() > 1 {
                    ("Open the marked projects in one window", "Open together")
                } else if window_count > 1 {
                    ("Choose an open window or open a new one", "Choose window")
                } else if window_count == 1 {
                    ("Switch to its open editor window", "Switch to window")
                } else {
                    ("Open in the editor", "Open")
                };
                let esc = if marking { "Clear the marks" } else { "Close" };
                vec![
                    s(&["↵"], open).footer(open_short).run(Confirm),
                    s(
                        &["mod-k", "shift-f10", "right-click"],
                        "Actions: rename, open with, the repository's pages, remove…",
                    )
                    .or_alt()
                    .footer("Actions")
                    .run(ShowActions),
                    s(
                        &["mod-r"],
                        "Commands: tasks from project files and your own",
                    )
                    .or_alt()
                    .run(ShowCommands),
                    s(
                        &["mod-↵", "alt-↵"],
                        "Open with another editor, or set its default…",
                    )
                    .run(OpenWithMenu),
                    s(&["→"], "Browse its files and folders"),
                    s(
                        &["tab", "shift-tab"],
                        "Mark to open several in one window (not saved)",
                    )
                    .run(ToggleMark),
                    s(&["f2"], "Rename… (search still finds it by its folder)").run(RenameItem),
                    s(&["shift-del"], "Remove from the list (the folder stays)").run(RemoveItem),
                    s(&["mod-z"], "Put back the project just removed").run(UndoRemove),
                    s(&["mod-e"], reveal).or_alt().run(ShowInFileManager),
                    s(&["mod-t"], "Open a terminal there")
                        .or_alt()
                        .run(OpenTerminal),
                    s(&["mod-g"], "Open the repository web page")
                        .or_alt()
                        .run(OpenRemote),
                    s(&["mod-c"], "Copy the path").or_alt().run(CopyPath),
                    s(&["mod-o"], "Add projects…").run(AddProjects),
                    s(
                        &["mod-↵"],
                        "On a pasted git URL, choose the clone destination",
                    )
                    .run(ConfirmSecondary),
                    s(
                        &["mod-1…9"],
                        "Open the project in that place (numbered while mod is held)",
                    )
                    .or_alt(),
                    s(&[">"], "Commands: default editor, updates, start on login…"),
                    s(
                        &["paste"],
                        "A folder path to add it, or a git URL to clone it",
                    ),
                    s(&["↑ ↓"], "Move the selection"),
                    s(&["pgup", "pgdn"], "Move a page at a time").run(SelectPageDown),
                    s(&["home", "end"], "The first or last, with nothing typed"),
                    s(&["mod-,"], "Open the config file").run(OpenConfig),
                    s(&["esc"], esc).run(Dismiss),
                    s(&["mod-q"], "Quit proj").run(QuitApp),
                ]
            }
            List::Browse => vec![
                s(&["↵"], "Open (files open in the project's window)")
                    .footer("Open")
                    .run(Confirm),
                s(&["mod-e"], reveal)
                    .footer(reveal_short)
                    .run(ShowInFileManager),
                s(&["→"], "Go into the folder"),
                s(&["←", "backspace"], "Back up a folder"),
                s(&["mod-t"], "Open a terminal there").run(OpenTerminal),
                s(&["mod-c"], "Copy the path").run(CopyPath),
                s(&["↑ ↓"], "Move the selection"),
                s(&["esc"], "Back to the projects").run(Dismiss),
            ],
            List::ProjectWindows => vec![
                s(&["↵"], "Switch to this window or open a new one")
                    .footer("Choose")
                    .run(Confirm),
                s(&["↑ ↓"], "Move the selection"),
                s(&["esc", "backspace"], "Back to the projects")
                    .footer("Back")
                    .run(Dismiss),
            ],
            List::OpenWith => {
                let own = self.open_with_project().and_then(|p| p.editor.as_ref());
                let selected = self.selected_editor().map(|e| &e.command);
                let (default, default_short) = if selected.is_some() && own == selected {
                    ("Go back to the default for all projects", "Undo default")
                } else {
                    ("Make it this project's default", "Make default")
                };
                vec![
                    s(&["↵"], "Open with it just this once")
                        .footer("Open once")
                        .run(Confirm),
                    s(&["mod-↵"], default)
                        .footer(default_short)
                        .run(ConfirmSecondary),
                    s(&["esc", "backspace"], "Back").run(Dismiss),
                ]
            }
            List::Templates => vec![
                s(&["↵"], "Use the template, or add a folder / git URL")
                    .footer("Choose")
                    .run(Confirm),
                s(
                    &["shift-del"],
                    "Remove the selected template (keep its source)",
                )
                .run(RemoveItem),
                s(&["↑ ↓"], "Move the selection"),
                s(&["esc", "backspace"], "Back to the projects")
                    .footer("Back")
                    .run(Dismiss),
            ],
            List::WatchedRoots => vec![
                s(
                    &["↵"],
                    "Edit this root's scan depth (or watch another folder)",
                )
                .footer(
                    if self
                        .matches
                        .get(self.selected)
                        .is_some_and(|m| m.ix < self.config.scan_dirs.len())
                    {
                        "Set depth"
                    } else {
                        "Add"
                    },
                )
                .run(Confirm),
                s(&["mod-↵"], "Edit excluded folder names or prefixes")
                    .footer("Exclusions")
                    .run(ConfirmSecondary),
                s(
                    &["shift-del"],
                    "Stop watching the selected root; never delete its folder",
                )
                .run(RemoveItem),
                s(&["↑ ↓"], "Move the selection"),
                s(&["esc", "backspace"], "Back to the projects")
                    .footer("Back")
                    .run(Dismiss),
            ],
            List::Destinations => vec![
                s(&["↵"], "Use this parent folder, then confirm the action")
                    .footer("Choose")
                    .run(Confirm),
                s(&["↑ ↓"], "Move the selection"),
                s(&["esc", "backspace"], "Keep the previous destination")
                    .footer("Back")
                    .run(Dismiss),
            ],
            List::Forges => vec![
                s(&["↵"], "It runs this: open the page, and remember it")
                    .footer("Use it")
                    .run(Confirm),
                s(&["↑ ↓"], "Move the selection"),
                s(&["esc", "backspace"], "Back to the projects")
                    .footer("Back")
                    .run(Dismiss),
            ],
            List::Group
                if self
                    .group_page
                    .as_ref()
                    .is_some_and(|p| p.editing.is_some()) =>
            {
                vec![
                    s(&["↵"], "Save the group's folders")
                        .footer("Save")
                        .run(Confirm),
                    s(&["tab", "space"], "Tick or untick a folder").run(ToggleMark),
                    s(
                        &["alt-↑", "alt-↓"],
                        "Move it earlier or later in the order they open in",
                    ),
                    s(&["esc", "backspace"], "Back, changing nothing")
                        .footer("Back")
                        .run(Dismiss),
                ]
            }
            List::Group => vec![
                s(&["↵"], "Open the ticked projects in one editor window")
                    .footer("Open together")
                    .run(Confirm),
                s(&["mod-↵"], "Save them as a group, to find in the list")
                    .footer("Save as group")
                    .run(ConfirmSecondary),
                s(&["tab", "space"], "Tick or untick a project").run(ToggleMark),
                s(
                    &["alt-↑", "alt-↓"],
                    "Move it earlier or later in the order they open in",
                ),
                s(&["esc", "backspace"], "Back to the projects").run(Dismiss),
            ],
            List::Files => vec![
                s(&["↵"], "Open in the project's editor (files in its window)")
                    .footer("Open")
                    .run(Confirm),
                s(&["mod-e"], reveal)
                    .footer(reveal_short)
                    .run(ShowInFileManager),
                s(&["mod-t"], "Open a terminal there").run(OpenTerminal),
                s(&["mod-c"], "Copy the path").run(CopyPath),
                s(&["↑ ↓"], "Move the selection"),
                s(&["esc"], "Back to the projects").run(Dismiss),
            ],
            List::Commands => vec![
                s(&["↵"], "Run").footer("Run").run(Confirm),
                s(&["esc"], "Back to the projects")
                    .footer("Back")
                    .run(Dismiss),
            ],
            List::Editors => {
                let (esc, esc_short) = if self.config.editor.is_some() {
                    ("Back", "Back")
                } else {
                    ("Close", "Close")
                };
                vec![
                    s(&["↵"], "Use it for all projects")
                        .footer(if self.config.editor.is_some() {
                            "Set default"
                        } else {
                            "Select"
                        })
                        .run(Confirm),
                    s(&["esc"], esc).footer(esc_short).run(Dismiss),
                ]
            }
            List::Switch => vec![
                s(&["alt-\\", "alt-shift-\\"], "Cycle editor windows"),
                s(&["mod-w", "alt-mod-w"], "Close the selected window").run(CloseWindow),
                s(&["alt-esc"], "Cancel").run(Dismiss),
            ],
            List::Text => {
                let save = match self.mode {
                    Mode::AddCommand => "Add the command",
                    Mode::NewProject => "Make the project and open it",
                    Mode::TemplateUrl => "Save the git template URL",
                    Mode::ScanDepth | Mode::ScanExclusions => "Save this root's scan settings",
                    Mode::Reset => "Reset only after typing RESET",
                    _ => "Save the name",
                };
                let short = match self.mode {
                    Mode::NewProject => "Make",
                    Mode::TemplateUrl => "Add",
                    Mode::Reset => "Reset",
                    Mode::AddCommand => "Add",
                    Mode::ScanDepth | Mode::ScanExclusions => "Save",
                    _ => "Save",
                };
                let mut keys = vec![s(&["↵"], save).footer(short).run(Confirm)];
                if self.mode == Mode::NewProject {
                    keys.push(s(&["mod-↵"], "Change the destination folder").run(ConfirmSecondary));
                }
                keys.push(s(&["esc"], "Cancel").footer("Cancel").run(Dismiss));
                keys
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alt_forms() {
        assert_eq!(alt_form("ctrl-e"), "alt-e");
        assert_eq!(alt_form("ctrl-shift-p"), "alt-p");
        assert_eq!(alt_form("ctrl-1…9"), "alt-1…9");
    }
}
