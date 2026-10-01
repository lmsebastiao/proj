//! Every shortcut of the current list. The footer has buttons for the main
//! two; F1 (or the footer's ? button) lists them all in a dropdown, where
//! clicking one runs it.

use gpui::Action;

use super::{
    Palette,
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

    fn run(mut self, action: impl Action) -> Self {
        self.run = Some(Box::new(action));
        self
    }
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
            if added {
                keys.push(s(&["shift-del"], "Remove the added command").run(RemoveItem));
            }
            keys.extend([
                s(&["↑ ↓"], "Move the selection"),
                s(&["esc", "mod-k", "alt-k"], "Close the menu")
                    .footer("Close")
                    .run(Dismiss),
            ]);
            return keys;
        }
        match self.list() {
            List::Projects => {
                let marking = !self.marked.is_empty();
                let has_window = self
                    .selected_project()
                    .is_some_and(|p| self.open_keys.contains(&p.key()));
                let (open, open_short) = if self.marked.len() > 1 {
                    ("Open the marked projects in one window", "Open together")
                } else if has_window {
                    ("Switch to its open editor window", "Switch")
                } else {
                    ("Open in the editor", "Open")
                };
                let esc = if marking { "Clear the marks" } else { "Close" };
                vec![
                    s(&["↵"], open).footer(open_short).run(Confirm),
                    s(
                        &["mod-k", "alt-k", "shift-f10", "right-click"],
                        "Actions: pin, rename, tags, commands to run, remove…",
                    )
                    .footer("Actions")
                    .run(ShowActions),
                    s(
                        &["mod-↵", "alt-↵"],
                        "Open with another editor, or set its default…",
                    )
                    .run(OpenWithMenu),
                    s(&["→"], "Browse its files and folders"),
                    s(&["tab", "shift-tab"], "Mark to open several in one window").run(ToggleMark),
                    s(&["f2"], "Rename… (search still finds it by its folder)").run(RenameItem),
                    s(&["mod-shift-p"], "Pin to the top, or unpin").run(TogglePin),
                    s(&["shift-del"], "Remove from the list (the folder stays)").run(RemoveItem),
                    s(&["mod-z"], "Put back the project just removed").run(UndoRemove),
                    s(&["mod-e"], reveal).run(ShowInFileManager),
                    s(&["mod-t"], "Open a terminal there").run(OpenTerminal),
                    s(&["mod-g"], "Open the repository web page").run(OpenRemote),
                    s(&["mod-c"], "Copy the path").run(CopyPath),
                    s(&["mod-o"], "Add projects…").run(AddProjects),
                    s(
                        &["mod-1…9"],
                        "Open the project in that place (numbered while mod is held)",
                    ),
                    s(&[">"], "Commands: default editor, updates, start on login…"),
                    s(&["@"], "Open editor windows, to search and switch to"),
                    s(&["#tag"], "Just the projects with that tag (set in mod-k)"),
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
                s(&["↵"], "Make a new project from it")
                    .footer("Pick")
                    .run(Confirm),
                s(&["↑ ↓"], "Move the selection"),
                s(&["esc", "backspace"], "Back to the projects")
                    .footer("Back")
                    .run(Dismiss),
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
            // While the modifier is held these keys come with alt.
            List::Switch if self.hold.is_some() => vec![
                s(&["alt-↓", "alt-↑"], "Move the selection"),
                s(&["alt-1…9"], "Switch to the row with that number"),
                s(
                    &["alt-→", "alt-←"],
                    "A project's windows one by one, and back",
                ),
                s(&["alt-mod-w"], "Close the window")
                    .footer("Close window")
                    .run(CloseWindow),
                s(&["type"], "Search the windows; the list stays open"),
                s(&["alt-esc"], "Cancel").footer("Cancel").run(Dismiss),
            ],
            List::Switch => {
                // Typed `@` in the project search: esc goes back to it.
                let esc = if self.expanded.is_some() {
                    "Back to every project's windows"
                } else if self.mode == Mode::Projects {
                    "Back to the projects"
                } else {
                    "Close"
                };
                let numbers = self
                    .config
                    .switch_number_modifiers()
                    .map(|mods| crate::switcher::shortcut_label(&format!("{mods}+1…9")));
                let mut keys = vec![
                    s(&["↵"], "Switch to it").footer("Switch").run(Confirm),
                    s(&["mod-w"], "Close the window")
                        .footer("Close window")
                        .run(CloseWindow),
                    s(&["→", "←"], "A project's windows one by one, and back"),
                    s(&["↑ ↓"], "Move the selection"),
                    s(&["drag"], "Move a row to another place in the list"),
                ];
                if let Some(numbers) = numbers {
                    keys.push(s(
                        &[numbers.as_str()],
                        "Switch straight to the row with that number, from anywhere",
                    ));
                }
                keys.push(s(&["esc"], esc).run(Dismiss));
                keys
            }
            List::Text => {
                let save = match self.mode {
                    Mode::Tags => "Save the tags",
                    Mode::AddCommand => "Add the command",
                    Mode::NewProject => "Make the project and open it",
                    _ => "Save the name",
                };
                let short = match self.mode {
                    Mode::NewProject => "Make",
                    Mode::AddCommand => "Add",
                    _ => "Save",
                };
                vec![
                    s(&["↵"], save).footer(short).run(Confirm),
                    s(&["esc"], "Cancel").footer("Cancel").run(Dismiss),
                ]
            }
        }
    }
}
