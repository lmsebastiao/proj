# proj

A project launcher that runs in the background: press a global shortcut, fuzzy-search your
projects, hit enter to open one in your editor. Built with [gpui](https://crates.io/crates/gpui).

## Usage

```sh
cargo build --release
proj            # start the launcher in the background
```

Press **alt+space** to toggle the dialog, or click the tray icon.
Right-click the tray icon for: open, start on login, open config file, check for updates, quit
(Windows and macOS; Linux has no tray icon). `hotkey` can also be a list:
`["alt+space", "f8"]`. On Windows, alt+space normally opens a window's system menu, and
PowerToys Run uses it too; if another app has it, the dialog says so and you can set another.

On first launch the dialog opens by itself. It asks which editor to use (it lists the ones it
finds installed, plus "Other…" to pick any program, or "No editor" to use the file manager),
then lists the folders you opened lately in other editors (see Importing), all ticked, to add
at once. If it finds none, it opens a folder picker where you can select one or more projects.

### Importing other editors' recent projects

Type `>` and pick **Import recent projects** (or `proj recent add` in a terminal) to list
the folders you opened lately in Zed, VS Code, VS Code Insiders, Cursor, Windsurf, VSCodium,
JetBrains IDEs and Android Studio that aren't in your list yet, the latest first, with the
editors they were found in. **tab** or **space** unticks one, **ctrl-enter** ticks all or none,
**enter** adds the ticked ones. A Zed workspace of several folders adds each folder and a
group of them (see Groups). Home, app data and temp folders are left out.

proj reads the editors' own files: Zed's database (`workspaces`, read without locking it), VS
Code's `storage.json`, JetBrains' `recentProjects.xml`.

| Key                     | Action                                     |
| ----------------------- | ------------------------------------------ |
| type                    | fuzzy filter by name, then by path         |
| ↑ / ↓                   | move selection                             |
| pgup / pgdn, home / end | a page at a time; the first / last (with nothing typed) |
| ctrl-1 … 9, alt-1 … 9   | open the project in that place (hold ctrl or alt a moment to see the numbers) |
| enter                   | open in editor                             |
| ctrl-enter, alt-enter   | open with… (another editor once, or set the project's default) |
| ctrl-k / alt-k, shift-f10, menu key, right-click | actions for the project: rename, tags, remove, and the ones below |
| ctrl-r / alt-r          | its commands to run: package.json scripts and your own (see Commands) |
| f2                      | rename                                     |
| shift-delete            | remove from the list (the folder stays)    |
| ctrl-z                  | put back the project just removed          |
| ctrl-t / alt-t          | open a terminal there (Windows Terminal if installed) |
| → / ←                   | browse into a project / back up (see below) |
| backspace               | with nothing typed: back a page (out of a folder, the Open-with list…) |
| tab / shift-tab         | mark projects to open together once, moving down / up (see Groups) |
| `#tag`                  | only the projects with that tag (see Tags) |
| ctrl-e / alt-e          | show in Explorer (Finder on macOS, the file manager on Linux) |
| ctrl-g / alt-g          | open the repository page (from the git `origin` remote) |
| ctrl-c / alt-c          | copy the project path (ctrl-c: the text, if some is selected) |
| ctrl-o                  | add projects (multi-select folder picker)  |
| ctrl-,                  | open the config file                       |
| f1                      | list every shortcut (↑/↓ and enter, or click one, to run it) |
| esc, clicking elsewhere | close                                      |
| ctrl-q                  | quit the launcher                          |

The keys for the highlighted project's actions work with **alt** as well as ctrl (not on
macOS, where option types characters). Alt is still down right after alt+space, so
alt+space then alt-k, without letting go, opens the actions menu. Ctrl-z, ctrl-o, ctrl-q and
ctrl-, stay on ctrl only: they mean the same everywhere.

The footer has buttons for the current list's main two keys, like PowerToys' Command
Palette: **Open ↵** and **Actions ctrl k** on the projects. Its left side says how many
projects there are, or what just happened. Problems show above it in full, in red, until you
type. **f1**, or the **?** button, opens a dropdown with every key. While it's open, ↑/↓ move
through it instead of the list and enter runs the highlighted key; clicking one runs it too.
Esc or typing closes it.

**ctrl-k** (or alt-k, handy right after alt+space; shift-f10, the menu key, or right-clicking a row) opens a menu over the list,
at the bottom right, with everything you can do with the highlighted project: open with,
show in Explorer, terminal, run a command (the ctrl-r menu, see Commands), rename,
tags, open the repository page, its pull requests or its CI runs, copy the clone URL, change
what its git site runs, copy the path, start a new project from it, remove. Each shows its own key, if it has one. Type to filter it and press
enter; esc, ctrl-k (or alt-k) again, backspace with nothing typed or clicking outside closes it. The
highlighted row, and any row under the mouse, also has a **⋯** icon that opens the same menu.

With nothing typed, the projects with an editor window open come first: the one you were
in when you pressed the shortcut, then the others, the one used last first. Then the rest,
the one opened last first, under a line.

Searching uses fzf's way of matching (through [nucleo](https://github.com/helix-editor/nucleo),
as Helix and Zed do): letters at the start of words and next to each other count most, and
each word you type matches on its own, in any order (`sdk shared` finds `shared-sdk`).
The best matches come first, and among matches about as good, the projects
you open more, and more lately: each of the last ten opens counts, less the older it is. So
`examp` puts the `example-v2` you opened minutes ago above the `example` from an hour
ago, though its name is longer. A group is an entry of its own, with its own opens.

**f2** renames a project; renaming to an empty name goes back to the folder name.
**shift-delete** removes it, only from the list, as the folder isn't touched. The footer
then offers **Undo** (or ctrl-z) for a few seconds.

Every row starts with an icon: the editor the project opens in, a folder or file while
browsing, the program in the editor and window lists. On the pages you reach from the project
list (Open with, browsing, rename, tags…), the search bar starts with a back button and the
page's name; backspace with nothing typed goes back too. Copying a path says **Copied** for
a moment before the dialog closes.

If the dialog closes without opening anything (esc, the shortcut again, or a click
elsewhere), opening it again within 30 seconds brings back what you'd typed, selected so that
typing replaces it.

Type `>` to list commands: start on login, add projects, import recent projects, new project from a template, change
the default editor, theme, open the config file, remove missing projects (when some are),
check for updates / install update (installed copies), quit.
Type `@` to list your open editor windows instead, the same list as the switcher (below):
keep typing to filter it, enter switches, esc goes back to the projects.
Type `$` and part of a file's name to find it in any of your projects, as ctrl-p does in an
editor (see Finding files).

The dialog follows Windows' light/dark app setting, switching live when Windows does. `>`
**Theme** goes through system → light → dark and saves the choice as `theme` in config.toml.

### Browsing inside a project

Press **→** on a project to list its files and folders; **→** on a folder goes deeper,
**←** (or backspace, with nothing typed) goes back up (and back to the project list from the
top), **esc** returns to the list.
Typing filters the current folder. **Enter** opens a file in the project's editor, inside the
project's window (`zed <project> <file>`), or opens a subfolder as its own workspace.
**ctrl-e** shows it in the file manager, **ctrl-t** opens a terminal there, and **ctrl-c**
copies its path. Nothing you browse is added to the project list.

### Finding files

Type **`$`** and then part of a file's name (`$button`) to search every listed project's files
at once. Words before the name narrow it by the folders it's in, the project's included, in
any order, split by spaces or either slash: `$palette files`, `$src/main`, `$web\src\index`. A folder's
name alone (`$components`) lists what's in it, after any files named like it. Files whose
name matches come first, the ones nearer the top of their project before deeper ones. Enter
opens the file in its project's editor and window, as in browsing; **ctrl-e**, **ctrl-t** and
**ctrl-c** work there too.

Repositories list what git does, so what `.gitignore` leaves out (build output, packages)
stays out. Other folders leave out `node_modules`, `target` and the like. Submodules, and
repositories inside a project that were never added to it, are searched as part of it. A
project inside another one that's in your list too (a submodule, or just a folder) keeps its
files to itself, so each file shows once, under the innermost project. The lists are read
in the background the first time you type `$`, and kept, then read again when a minute old.
The best 200 matches show.

→/← only browse when the text cursor is at the end/start of the search box, so they still
move the cursor while you edit a search.

### Switching between open projects

Hold **alt** and tap **\\** to switch between your open editor windows, like Alt+Tab but only
for editors. It's the \\ key wherever your layout has it: left of 1 on Portuguese keyboards,
above enter on US ones. Keep tapping to move down the list; **alt+shift+\\** or **alt+↑** goes
back up, and a number (**alt+1** to **9**) switches straight to that row. Let go of alt and
the highlighted window comes to the front. The list shows as soon as you press the key, like
Alt+Tab; a quick tap switches to the editor window you used before. **alt-esc** cancels.

A project's windows share one row ("3 windows · …"), which switches to the one you used last
(or, for the project you're in, to its other window). **→** on that row (**alt+→** while
holding) lists its windows one by one; **←** goes back. **ctrl-w** (**ctrl+alt+w** while
holding) closes the highlighted window, as its close button would, so the editor can still
ask about unsaved changes. So does the **✕** at the end of the highlighted row, or of any row
under the mouse.

The list keeps its order: rows stay where they first appeared, new ones are added at the
end, and switching doesn't move anything. The highlight starts on the window you used before
the current one. To put a row somewhere else, drag it with the mouse and drop it on
another; a line shows where it will go. It keeps that place (and number) until it closes.
Dragging works while the list shows every row, not while you're searching it. If you let
go of alt mid-drag, the list stays open so you can drop the row.

To search instead, start typing while you still hold alt: the list stays open when you let
go, typing filters it, **enter** switches and **esc** closes. From the project search, typing
`@` gets you the same list.

**alt+shift+1** to **9** switch straight to that row of the list, without showing it,
like Win+1 on the taskbar. The list numbers its first nine rows, and since it keeps its
order, each row keeps its number until it closes. Set other modifiers with
`switch_number_modifiers` in config.toml, but not ctrl+alt on Windows: Windows reads AltGr as
ctrl+alt, so AltGr+2 (@), AltGr+7 ({) and so on would stop typing in every app.

Each window is listed under its project, with the branch and the editor. proj works out the
project from the window title: the folder name (Zed, VS Code, Cursor…), a Zed workspace's
folder list, or the solution name (Visual Studio). Windows it can't match are listed by their
title. Projects with a window open get a green dot after their name in the normal list, and **enter** on
one switches to its window instead of opening it again (**alt-↵** still opens it anew, in any
editor).

`switch_hotkey` in config.toml sets another shortcut, e.g. `"alt+q"`, or `""` turns the
switcher off. Windows and macOS; on macOS proj needs to be allowed under System Settings ›
Privacy & Security › Accessibility (it asks the first time), and lists the apps with windows
on screen, so not ones with only minimized windows or windows on other Spaces.

### Groups: opening projects together

Several projects can open in one editor window: `zed example-v2 sample-sdk`, and the
same for VS Code. Folders are passed in the order they were ticked.

**Once:** press **tab** on a project to mark it (marks stay while you change the search), and
**enter** opens all the marked ones together. For example: type `exam`, tab, type `sample`,
tab, enter. Nothing is saved. Esc clears the marks.

**To keep:** ctrl-k on a project → **Open together with…** opens a page listing your projects
with that one ticked. **tab** or **space** ticks or unticks the highlighted project, and
**alt-↑/↓** moves it earlier or later in the order (its number shows it). **enter** opens the
ticked ones together once; **ctrl-enter** saves them as a group, a row of its own in the list,
with its own history, editor, tags and commands, so next time you just search for it.
Ticking the same projects in another order finds the same group.

A group is named after its first folder, `example-v2 Workspace`, as everything it does goes
by that folder (its repository pages, terminal, file manager). Groups that start with the same
folder add their others: `example-v2 Workspace (sample-sdk)`. So searching `sample` finds the
`sample-sdk` project first, above the groups it's in, which follow since their folders still
match. A group's row has a folder icon with how many folders it has, and its folders' names under
its name, each with a green dot when that project has a window of its own open. Its branches
show on the right; hovering them lists each folder's branch and changes. ctrl-k on a group →
**Folders…** opens the same page with its folders ticked, to add, take out or reorder them;
enter saves, keeping its name, tags and history. Rename it (f2) to give it a shorter name;
search still finds it by its folders' names. Removing it (shift-delete) forgets the group, not
the projects.

### Per-project editors

Enter opens a project in its own default editor if it has one, and otherwise in the default
editor for all projects. Rows of projects with their own default show its name.

- **Just this once:** press **ctrl-enter** (or alt-enter) on a project, pick an editor, press **enter**.
  Nothing is saved, so next time enter uses the usual editor again.
- **This project's default:** in the same list, press **ctrl-enter** on an editor
  (on **Other…**, it asks for a program first). Press ctrl-enter on it again to go back to
  the default for all projects.
- **All projects:** type `>` and pick **Change the default editor**. Projects with their own
  default keep it.

For example, a WPF app can default to Visual Studio while everything else opens in Zed.
Visual Studio is found automatically and gets the project's `.sln`/`.slnx` instead of the
folder. Project defaults live in `projects.toml` under `[editors]`.

Each row shows the current git branch (read from `.git/HEAD`) and when you last opened it.
After the branch, **●** means uncommitted changes and **↑2 ↓1** commits to push and pull:
`git status` runs in the background each time the dialog opens (at most every 10 seconds per
repository), so the counts show up a moment later, and right away the next time.
Projects that share a folder name get their parent folder added, e.g. `app (client)`.

A project you added whose folder is gone (moved, deleted, or on a drive that isn't
connected) stays listed at the end, dimmed and marked **missing**, instead of vanishing. Type
`>` and pick **Remove missing projects** to forget them all, or remove one with shift-delete.

### Tags

ctrl-k → **Tags…** on a project, type words like `work oss` and press enter. Tags show after
the project's name. Search for `#work` to list only the projects tagged with it; the start of a
tag is enough (`#wo`), several tags must all match, and the rest of the search filters as
usual: `#work api`.

### Commands

**ctrl-r** (or alt-r, or ctrl-k → **Run a command…**) on a project opens its commands menu,
over the list like the ctrl-k one: the ones you added, then its `package.json` scripts, run
with npm, pnpm, yarn or bun according to its lock file, then **Add a command…** (e.g.
`npm run dev`). Type to filter them. Enter opens a terminal in the project's folder running
it, which stays open when it ends. **shift-delete** on an added command takes it out again.
ctrl-r again, or esc, closes the menu; ctrl-k switches to the actions.

### Pull requests and CI

ctrl-k → **Open the pull requests** and **Open the CI runs** go to those pages of the
repository's site, from its `origin` remote: GitHub, GitLab, Gitea, Forgejo (Codeberg),
Bitbucket and Azure DevOps, by their site names (`gitlab.example.com` counts too). For a
self-hosted site with another name, proj asks what it runs the first time (GitLab, Gitea,
Forgejo…), opens the page, and saves the answer in config.toml as
`forges = { "git.example.com" = "gitlab" }`. To change it later, ctrl-k →
**Change what git.example.com runs…**, or edit that line. **Copy the clone URL** copies the remote's URL.

### New projects from templates

Type `>` and pick **New project from a template**, or ctrl-k → **New project from this one…**
on any project. Templates are the folders and git URLs listed under `templates` in
config.toml. After picking one, type the new project's folder name and press enter: it's made
in the first `scan_dirs` folder (or one you pick), with a git history of its own, and opens.
A folder that is a git repository is copied as git would commit it, so without what
`.gitignore` leaves out; other folders are copied whole, but for `node_modules`, `target` and
the like. A git URL is cloned without its history.

Typing or pasting a folder path (`C:\…`, `~/…`, `/…`) shows an **Add and open** row: enter adds
it to the list and opens it in the default editor; ctrl-enter (cmd-enter on macOS) just opens it.
A file path shows an **Open** row: the file opens in the editor of the listed project it's in (so
it lands in that project's window), or in the default editor.

Pasting a git URL (`https://…`, `ssh://…`, `git@host:owner/repo.git`) shows a **Clone** row.
Enter runs `git clone` into the first `scan_dirs` folder (or asks for a folder if there are none),
then opens the new project. The dialog stays open while it clones; if you close it, the clone
still finishes and is listed next time. Private repositories work the way they do for `git` on
your machine (SSH keys, Git Credential Manager).

CLI:

```sh
proj open QUERY     # open the best match, as if you'd typed QUERY and pressed enter
proj add [PATH]     # add a project (default: current directory)
proj remove PATH    # remove / hide a project
proj list           # list projects
proj recent [add]   # list the projects other editors opened lately; add: add them
proj paths          # where the config and database live
proj autostart [on|off]  # start proj when you log in
proj path [add|remove]   # put proj's folder on your user PATH (Windows)
proj update         # install the latest release, if it is newer (see Updates)
proj version        # show the installed version
```

## Configuration

`config.toml` (see `proj paths`) is created on first run with comments explaining each option:

```toml
hotkey = "alt+space"
# switch_hotkey = "alt+q"  # window switcher; default alt+\, "" = off
# switch_number_modifiers = "ctrl+shift"  # + 1…9: straight to that row; default alt+shift
scan_dirs = []          # optional: list every sub-folder of these folders as projects
# templates = ['C:\templates\rust-cli', "https://github.com/me/web-starter"]
# forges = { "git.example.com" = "gitlab" }  # for pull request and CI links
scan_depth = 1          # >1 descends into non-git folders
check_for_updates = true  # look for a new release about once a day
theme = "system"        # or "light" / "dark"; also set with > Theme
monitor = "cursor"      # the screen it opens on: "cursor", "focused" (the window in front) or "primary" (Windows only)
editor = "zed"          # set from the launcher; "" = file manager
editor_args = []        # e.g. ["--new-window"]
```

Picking an editor in the launcher updates only the `editor` line; the rest of the file,
comments included, is left alone.

Config and folder scans are re-read every time the dialog opens, so changes apply immediately.
That includes `hotkey`: a new shortcut starts working once the dialog has opened, by the old
shortcut or the tray icon. If none of the new shortcuts can be registered, the old ones keep
working and the dialog says why.
Manually added projects, hidden projects, names, tags, added commands and open history live
in `projects.toml`.

## Windows installer

```powershell
.\scripts\build-installer.ps1              # cargo build --release + package
.\scripts\build-installer.ps1 -SkipBuild   # package the last release build
```

This produces `dist\proj-setup-<version>.exe` (version from `Cargo.toml`). The script uses
`makensis` if NSIS is installed; otherwise it downloads the pinned portable NSIS release once into
`%LOCALAPPDATA%\proj-build`, checking its SHA-256. Nothing is installed system-wide.

The installer is per user, so it needs no admin rights:

- installs to `%LOCALAPPDATA%\Programs\proj` and adds that folder to the user PATH, so `proj add .`
  works in new terminals
- adds a Start menu shortcut and an "Apps & features" entry
- offers "Start proj when I log in" and "Start proj now" on the finish page
- stops a running proj before upgrading
- the uninstaller removes the PATH entry and start-on-login, but keeps your settings in
  `%APPDATA%\proj`
- supports silent install and uninstall: `proj-setup-x.y.z.exe /S`, `uninstall.exe /S`

The PATH is edited by `proj path add/remove`, not by NSIS. NSIS strings are limited to 1024
characters, which would truncate a long PATH.

## Updates

A copy installed with the installer looks for a newer
[GitHub release](https://github.com/lmsebastiao/proj/releases) a minute after it starts and
then about once a day. Set `check_for_updates = false` to turn that off. When a new version
is out, the tray menu's **Check for updates** item becomes **Install update x.y.z**, and so does
the same command in the `>` list. Either one checks now when clicked, or once an update is
found, downloads the installer and runs it silently
(`proj-setup-x.y.z.exe /S /RELAUNCH /D=<install folder>`). The installer replaces proj.exe
and starts the new version. Nothing is installed without that click, or without running
`proj update`.

Copies built with `cargo build` don't update themselves: they have no `uninstall.exe` next to
them, so neither the tray nor the `>` list has the update command, and `proj update` only says
whether a newer version exists.

## Releasing

GitHub Actions runs `cargo fmt --check`, clippy and the tests on every push to `main` and every
pull request (`.github/workflows/ci.yml`). To publish a version:

1. Bump `version` in `Cargo.toml` (e.g. `0.3.0`), run `cargo check` so `Cargo.lock` follows,
   commit and push to `main`.
2. Run the **Release** workflow: on GitHub, Actions → Release → Run workflow (branch `main`),
   or `gh workflow run release.yml`.

`.github/workflows/release.yml` refuses to run from any branch other than `main`, or when the
version in `Cargo.toml` is already tagged. It then runs `scripts/build-installer.ps1` and
creates the `v0.3.0` tag and a release with `proj-setup-0.3.0.exe` and `SHA256SUMS.txt`, with
notes generated from the commits. You don't create the tag yourself. Installed copies pick the
release up on their next check.

## Memory

The dialog window is destroyed when it closes. On Windows the process then trims its working
set, so it sits at about 1–3 MB of working set while idle. Committed memory stays higher,
around 35 MB before the first open and about 70 MB after it, mostly gpui's GPU and font state.

## Start on login

Type `>` and pick **Start on login**, or run `proj autostart on`. This uses the registry Run
key on Windows, a LaunchAgent on macOS, and `~/.config/autostart` on Linux.

## Platform notes

Only tested on Windows so far. Global hotkeys don't work on Linux Wayland (a limitation of
`global-hotkey`); X11 is fine. On macOS the app shows a Dock icon, because gpui 0.2 doesn't
expose the accessory activation policy. The macOS editor and window icons and the window
switcher (through the Accessibility API) are written but haven't been run on a Mac yet. On
Linux the switcher lists no windows and rows have no icons.
