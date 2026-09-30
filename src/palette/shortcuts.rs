//! Every shortcut of the current list. The footer shows the common ones; F1 (or
//! clicking "all keys") lists them all in a dropdown, where clicking one runs it.

use gpui::Action;

use super::{
    Palette,
    items::{List, Mode},
    keymap::*,
    secondary,
};

pub(super) struct Shortcut {
    /// All keys that do it; the footer shows the first.
    pub(super) keys: Vec<String>,
    /// What it does, in the dropdown.
    pub(super) action: &'static str,
    /// Its short label when the footer shows it too.
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

    fn footer_if(self, show: bool, label: &'static str) -> Self {
        if show { self.footer(label) } else { self }
    }

    fn run(mut self, action: impl Action) -> Self {
        self.run = Some(Box::new(action));
        self
    }
}

/// "Show in Explorer" and the footer's short name for it.
pub(super) fn file_manager() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("Show in Explorer", "explorer")
    } else if cfg!(target_os = "macos") {
        ("Show in Finder", "finder")
    } else {
        ("Show in the file manager", "folder")
    }
}

impl Palette {
    pub(super) fn shortcuts(&self) -> Vec<Shortcut> {
        let s = Shortcut::new;
        let (reveal, reveal_short) = file_manager();
        match self.list() {
            List::Projects => {
                let marking = !self.marked.is_empty();
                let (open, open_short) = if self.marked.len() > 1 {
                    ("Open the marked projects in one window", "open together")
                } else {
                    ("Open in the editor", "open")
                };
                let (esc, esc_short) = if marking {
                    ("Clear the marks", "clear")
                } else {
                    ("Close", "close")
                };
                vec![
                    s(&["↵"], open).footer(open_short).run(Confirm),
                    s(&["→"], "Browse its files and folders").footer_if(!marking, "files"),
                    s(&["alt-↵"], "Open with another editor, or set its default…")
                        .footer("open with…")
                        .run(OpenWithMenu),
                    s(&["tab", "shift-tab"], "Mark to open several in one window")
                        .footer(if marking { "mark" } else { "combine" })
                        .run(ToggleMark),
                    s(
                        &["mod-k", "shift-f10"],
                        "Actions: pin, rename, remove, copy path…",
                    )
                    .footer_if(!marking, "actions")
                    .run(ShowActions),
                    s(&["mod-e"], reveal).run(ShowInFileManager),
                    s(&["mod-t"], "Open a terminal there").run(OpenTerminal),
                    s(&["mod-g"], "Open the repository web page").run(OpenRemote),
                    s(&["mod-c"], "Copy the path").run(CopyPath),
                    s(&["mod-o"], "Add projects…").run(AddProjects),
                    s(&[">"], "Commands: default editor, updates, start on login…"),
                    s(&["@"], "Open editor windows, to search and switch to"),
                    s(&["↑ ↓"], "Move the selection"),
                    s(&["esc"], esc).footer_if(marking, esc_short).run(Dismiss),
                    s(&["mod-q"], "Quit proj").run(QuitApp),
                ]
            }
            List::Browse => vec![
                s(&["↵"], "Open (files open in the project's window)")
                    .footer("open")
                    .run(Confirm),
                s(&["→"], "Go into the folder").footer("enter"),
                s(&["←"], "Back up a folder").footer("back"),
                s(&["mod-e"], reveal)
                    .footer(reveal_short)
                    .run(ShowInFileManager),
                s(&["mod-t"], "Open a terminal there")
                    .footer("terminal")
                    .run(OpenTerminal),
                s(&["mod-c"], "Copy the path").run(CopyPath),
                s(&["↑ ↓"], "Move the selection"),
                s(&["esc"], "Back to the projects").run(Dismiss),
            ],
            List::OpenWith => {
                let own = self.open_with_project().and_then(|p| p.editor.as_ref());
                let selected = self.selected_editor().map(|e| &e.command);
                let (default, default_short) = if selected.is_some() && own == selected {
                    ("Go back to the default for all projects", "undo default")
                } else {
                    ("Make it this project's default", "make default")
                };
                vec![
                    s(&["↵"], "Open with it just this once")
                        .footer("open once")
                        .run(Confirm),
                    s(&["mod-↵"], default)
                        .footer(default_short)
                        .run(ToggleProjectDefault),
                    s(&["esc"], "Back").footer("back").run(Dismiss),
                ]
            }
            List::Actions => vec![
                s(&["↵"], "Run it").footer("run").run(Confirm),
                s(&["↑ ↓"], "Move the selection"),
                s(&["esc"], "Back to the projects")
                    .footer("back")
                    .run(Dismiss),
            ],
            List::Commands => vec![
                s(&["↵"], "Run").footer("run").run(Confirm),
                s(&["esc"], "Back").footer("back").run(Dismiss),
            ],
            List::Editors => {
                let esc = if self.config.editor.is_some() {
                    "back"
                } else {
                    "close"
                };
                vec![
                    s(&["↵"], "Use it for all projects")
                        .footer(if self.config.editor.is_some() {
                            "set default"
                        } else {
                            "select"
                        })
                        .run(Confirm),
                    s(&["esc"], "Back").footer(esc).run(Dismiss),
                ]
            }
            // While the modifier is held these keys come with alt.
            List::Switch if self.hold.is_some() => vec![
                s(&["alt-↓", "alt-↑"], "Move the selection"),
                s(&["alt-1…9"], "Switch to the window with that number"),
                s(&["alt-esc"], "Cancel").footer("cancel").run(Dismiss),
            ],
            List::Switch => {
                // Typed `@` in the project search: esc goes back to it.
                let (esc, esc_short) = if self.mode == Mode::Projects {
                    ("Back to the projects", "back")
                } else {
                    ("Close", "close")
                };
                let numbers = self
                    .config
                    .switch_number_modifiers()
                    .map(|mods| crate::switcher::shortcut_label(&format!("{mods}+1…9")));
                let mut keys = vec![
                    s(&["↵"], "Switch to it").footer("switch").run(Confirm),
                    s(&["↑ ↓"], "Move the selection"),
                    s(&["drag"], "Move a window to another place in the list"),
                ];
                if let Some(numbers) = numbers {
                    keys.push(s(
                        &[numbers.as_str()],
                        "Switch straight to the window with that number, from anywhere",
                    ));
                }
                keys.push(s(&["esc"], esc).footer(esc_short).run(Dismiss));
                keys
            }
            List::Rename => vec![
                s(&["↵"], "Save the name").footer("save").run(Confirm),
                s(&["esc"], "Cancel").footer("cancel").run(Dismiss),
            ],
        }
    }
}
