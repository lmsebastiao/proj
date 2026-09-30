//! Every shortcut of the current list. The footer shows the common ones; F1 (or
//! clicking "all keys") lists them all in a dropdown, where clicking one runs it.

use gpui::Action;

use super::{Palette, items::List, keymap::*, secondary};

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
fn file_manager() -> (&'static str, &'static str) {
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
                    s(&["mod-w"], "Open with another editor, once or always…")
                        .footer("open with…")
                        .run(OpenWithMenu),
                    s(&["tab", "shift-tab"], "Mark to open several in one window")
                        .footer(if marking { "mark" } else { "combine" })
                        .run(ToggleMark),
                    s(&["mod-e"], reveal)
                        .footer_if(!marking, reveal_short)
                        .run(ShowInFileManager),
                    s(&["mod-t"], "Open a terminal there").run(OpenTerminal),
                    s(&["mod-p"], "Pin or unpin (pinned stay on top)").run(TogglePin),
                    s(&["f2"], "Rename").run(Rename),
                    s(&["mod-g"], "Open the repository web page").run(OpenRemote),
                    s(&["mod-c"], "Copy the path").run(CopyPath),
                    s(&["mod-d", "shift-del"], "Remove from the list").run(Remove),
                    s(&["mod-o"], "Add projects…").run(AddProjects),
                    s(&["alt-↵"], "Change the default editor for all projects").run(ChooseEditor),
                    s(&[">"], "Commands: start on login, config file…"),
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
                let own = self
                    .open_with_project()
                    .map(|p| p.editors.as_slice())
                    .unwrap_or_default();
                let selected = self.selected_editor().map(|e| &e.command);
                let (always, always_short) = if selected.is_some() && own.first() == selected {
                    ("Stop always using it for this project", "undo always")
                } else {
                    ("Always open this project with it", "always")
                };
                let (list, list_short) = if selected.is_some_and(|c| own.contains(c)) {
                    ("Remove it from the editors to pick between", "remove")
                } else {
                    (
                        "Add it to the editors to pick between each time",
                        "pick list",
                    )
                };
                vec![
                    s(&["↵"], "Open with it just this once")
                        .footer("open once")
                        .run(Confirm),
                    s(&["mod-↵"], always)
                        .footer(always_short)
                        .run(AlwaysOpenWith),
                    s(&["mod-p"], list).footer(list_short).run(TogglePin),
                    s(
                        &["alt-↵"],
                        "Change the default editor for all projects instead",
                    )
                    .run(ChooseEditor),
                    s(&["esc"], "Back").footer("back").run(Dismiss),
                ]
            }
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
                let mut keys = vec![
                    s(&["↵"], "Use it for all projects")
                        .footer(if self.config.editor.is_some() {
                            "set default"
                        } else {
                            "select"
                        })
                        .run(Confirm),
                ];
                if self.open_with.is_some() {
                    keys.push(
                        s(&["mod-w"], "Only for the selected project…")
                            .footer("this project only")
                            .run(OpenWithMenu),
                    );
                }
                keys.push(s(&["esc"], "Back").footer(esc).run(Dismiss));
                keys
            }
            // Shown while the modifier is held, so these keys come with alt.
            List::Switch => vec![
                s(&["alt-↓", "alt-↑"], "Move the selection"),
                s(&["alt-↵"], "Switch now").footer("switch").run(Confirm),
                s(&["alt-esc"], "Cancel").footer("cancel").run(Dismiss),
            ],
            List::Rename => vec![
                s(&["↵"], "Save the name").footer("save").run(Confirm),
                s(&["esc"], "Cancel").footer("cancel").run(Dismiss),
            ],
        }
    }
}
