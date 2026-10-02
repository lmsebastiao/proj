//! `$`: finding a file or folder in every listed project by its name, as
//! ctrl-p does in an editor, then opening it in its project's editor.
//!
//! Each project folder's files come from `git ls-files` (so what `.gitignore`
//! leaves out stays out), or else a walk that skips build output and
//! installed packages. They're read in the background the first time `$` is
//! typed and kept between openings, read again when old.

use std::{
    collections::{HashMap, HashSet},
    fs, iter,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use gpui::Context;

use crate::{fuzzy, git, store::Project, templates::SKIPPED};

use super::{
    Palette,
    items::{List, Match},
};

pub(super) const FILES_PREFIX: char = '$';
/// The most rows a search shows: the best matches.
pub(super) const MAX_RESULTS: usize = 200;
/// Where the walk of a folder that isn't a git repository stops.
const MAX_ENTRIES: usize = 100_000;
/// A folder's list is read again after this; the old one shows meanwhile.
const FRESH_FOR: Duration = Duration::from_secs(60);

/// Each folder's files, and when they were read.
type Cache = HashMap<PathBuf, (Arc<FolderFiles>, Instant)>;

/// Kept between openings of the palette.
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Mutex::default);

/// The files and folders in a project's folder.
pub(super) struct FolderFiles {
    root: PathBuf,
    /// The folder's name, in front of the paths shown.
    name: String,
    entries: Vec<FileEntry>,
}

struct FileEntry {
    /// From the folder, with `/` between the parts.
    rel: String,
    is_dir: bool,
}

/// A file or folder `$` found.
pub(super) struct FileHit {
    /// The project folder it's in.
    pub(super) root: PathBuf,
    pub(super) path: PathBuf,
    pub(super) is_dir: bool,
    /// Its name ("components/" for a folder).
    pub(super) title: String,
    /// Where it is: "proj/src/palette/files.rs".
    pub(super) subtitle: String,
}

/// What `$` has read so far, and found.
#[derive(Default)]
pub(super) struct FileSearch {
    folders: Vec<Arc<FolderFiles>>,
    started: bool,
    /// Folders still being read.
    pub(super) reading: usize,
    /// The rows `matches` points into while `$` is typed. Empty whenever
    /// `matches` is another list's.
    pub(super) hits: Vec<FileHit>,
    /// Counts searches, so one that finishes after a newer one started is dropped.
    generation: usize,
}

impl FolderFiles {
    fn read(root: PathBuf) -> Self {
        let entries = match git::listed_files(&root) {
            Some(files) => with_folders(files),
            None => walk(&root),
        };
        Self {
            name: root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.to_string_lossy().into_owned()),
            root,
            entries,
        }
    }
}

/// Git lists files only: the folders they're in go in too.
fn with_folders(files: Vec<PathBuf>) -> Vec<FileEntry> {
    let mut folders: HashSet<String> = HashSet::new();
    let mut entries = Vec::with_capacity(files.len());
    for file in files {
        let rel = file.to_string_lossy().replace('\\', "/");
        let mut end = rel.len();
        // Up from the file, until a folder that's in already: so are its parents.
        while let Some(slash) = rel[..end].rfind('/') {
            if !folders.insert(rel[..slash].to_string()) {
                break;
            }
            end = slash;
        }
        entries.push(FileEntry { rel, is_dir: false });
    }
    entries.extend(
        folders
            .into_iter()
            .map(|rel| FileEntry { rel, is_dir: true }),
    );
    entries
}

/// A folder that isn't a repository: everything but build output and
/// installed packages, up to `MAX_ENTRIES`. Links to folders aren't followed.
fn walk(root: &Path) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            if entries.len() >= MAX_ENTRIES {
                return entries;
            }
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            let name = entry.file_name();
            if is_dir
                && SKIPPED
                    .iter()
                    .any(|s| name.to_string_lossy().eq_ignore_ascii_case(s))
            {
                continue;
            }
            let path = entry.path();
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            entries.push(FileEntry {
                rel: rel.to_string_lossy().replace('\\', "/"),
                is_dir,
            });
            if is_dir {
                dirs.push(path);
            }
        }
    }
    entries
}

/// A folder's list as last read, if it was, and whether it's time to read it again.
fn cached(root: &Path) -> (Option<Arc<FolderFiles>>, bool) {
    let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    match cache.get(root) {
        Some((files, read)) => (Some(files.clone()), read.elapsed() > FRESH_FOR),
        None => (None, true),
    }
}

fn read_and_cache(root: PathBuf) -> Arc<FolderFiles> {
    let files = Arc::new(FolderFiles::read(root.clone()));
    CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(root, (files.clone(), Instant::now()));
    files
}

/// Whether the characters of `needle` (lowercase, no spaces) come in this
/// order in `text`: cheap, so most entries are passed over without scoring.
fn is_subsequence(needle: &[char], text: impl Iterator<Item = char>) -> bool {
    let mut want = needle.iter().peekable();
    for c in text {
        let Some(&&next) = want.peek() else {
            break;
        };
        if c.to_lowercase().next() == Some(next) {
            want.next();
        }
    }
    want.peek().is_none()
}

/// The best `MAX_RESULTS` matches of `query`: by name first, then by where
/// they are; between equal names, the shallower one.
fn search(folders: &[Arc<FolderFiles>], query: &str) -> (Vec<FileHit>, Vec<Match>) {
    let needle: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    let mut found: Vec<(i32, FileHit, fuzzy::ItemMatch)> = Vec::new();
    for folder in folders {
        for entry in &folder.entries {
            let place = folder.name.chars().chain(iter::once('/'));
            if !is_subsequence(&needle, place.chain(entry.rel.chars())) {
                continue;
            }
            let name = entry.rel.rsplit('/').next().unwrap_or(&entry.rel);
            let title = if entry.is_dir {
                format!("{name}/")
            } else {
                name.to_string()
            };
            let subtitle = format!("{}/{}", folder.name, entry.rel);
            let depth = entry.rel.matches('/').count() as i32;
            let Some(m) = fuzzy::score_item(query, &title, &subtitle, -depth) else {
                continue;
            };
            let path = entry
                .rel
                .split('/')
                .fold(folder.root.clone(), |path, part| path.join(part));
            let hit = FileHit {
                root: folder.root.clone(),
                path,
                is_dir: entry.is_dir,
                title,
                subtitle,
            };
            found.push((m.score, hit, m));
        }
    }
    let best = |a: &(i32, FileHit, _), b: &(i32, FileHit, _)| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.subtitle.len().cmp(&b.1.subtitle.len()))
    };
    if found.len() > MAX_RESULTS {
        found.select_nth_unstable_by(MAX_RESULTS, best);
        found.truncate(MAX_RESULTS);
    }
    found.sort_by(best);
    found
        .into_iter()
        .enumerate()
        .map(|(ix, (_, hit, m))| {
            let row = Match {
                ix,
                title_hl: m.title_hl,
                subtitle_hl: m.subtitle_hl,
            };
            (hit, row)
        })
        .unzip()
}

impl Palette {
    /// Every listed project's folders, groups' too, once each; not the
    /// ones that are gone.
    fn search_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = Vec::new();
        for project in self.projects.iter().filter(|p| !p.missing) {
            for path in project.paths() {
                if !roots.contains(&path) {
                    roots.push(path);
                }
            }
        }
        roots
    }

    /// The first time `$` is typed: the folders' lists read before show at
    /// once, and those that are old or new are read in the background,
    /// searched again as each comes in.
    fn load_files(&mut self, cx: &mut Context<Self>) {
        if self.files.started {
            return;
        }
        self.files.started = true;
        let mut due = Vec::new();
        for root in self.search_roots() {
            let (files, stale) = cached(&root);
            self.files.folders.extend(files);
            if stale {
                due.push(root);
            }
        }
        self.files.reading = due.len();
        let reads: Vec<_> = due
            .into_iter()
            .map(|root| {
                cx.background_executor()
                    .spawn(async move { read_and_cache(root) })
            })
            .collect();
        cx.spawn(async move |this, cx| {
            for read in reads {
                let files = read.await;
                let updated = this.update(cx, |this, cx| {
                    this.files.reading -= 1;
                    // In place of the list read before.
                    this.files.folders.retain(|f| f.root != files.root);
                    this.files.folders.push(files);
                    if this.list() == List::Files {
                        this.search_files(cx);
                    }
                    cx.notify();
                });
                // Closed: drop the rest.
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Searches the files for what's after `$`, in the background. The rows
    /// from before stay until the new ones are in.
    pub(super) fn search_files(&mut self, cx: &mut Context<Self>) {
        self.load_files(cx);
        if self.files.hits.is_empty() {
            // Another list's rows.
            self.matches.clear();
        }
        self.files.generation += 1;
        let generation = self.files.generation;
        let query = self.filter_query().to_string();
        if query.is_empty() {
            self.files.hits.clear();
            self.matches.clear();
            self.selected = 0;
            self.reset_list();
            cx.notify();
            return;
        }
        let folders = self.files.folders.clone();
        let search = cx
            .background_executor()
            .spawn(async move { search(&folders, &query) });
        cx.spawn(async move |this, cx| {
            let (hits, matches) = search.await;
            this.update(cx, |this, cx| {
                if this.files.generation != generation || this.list() != List::Files {
                    return;
                }
                // More folders came in: the same row stays selected, if it's still there.
                let selected = this.selected_hit().map(|hit| hit.path.clone());
                this.files.hits = hits;
                this.matches = matches;
                this.selected = 0;
                this.reset_list();
                if let Some(path) = selected {
                    this.select_where(|this, ix| this.files.hits[ix].path == path);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn selected_hit(&self) -> Option<&FileHit> {
        if self.list() != List::Files {
            return None;
        }
        let m = self.matches.get(self.selected)?;
        self.files.hits.get(m.ix)
    }

    /// The project a `$` row is in: the one with that folder, else a group with it.
    pub(super) fn project_of(&self, root: &Path) -> Option<Project> {
        let own = self
            .projects
            .iter()
            .find(|p| !p.is_workspace() && p.path == root);
        own.or_else(|| {
            self.projects
                .iter()
                .find(|p| p.paths().iter().any(|path| path == root))
        })
        .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str, entries: &[(&str, bool)]) -> Arc<FolderFiles> {
        Arc::new(FolderFiles {
            root: PathBuf::from("/repos").join(name),
            name: name.to_string(),
            entries: entries
                .iter()
                .map(|&(rel, is_dir)| FileEntry {
                    rel: rel.to_string(),
                    is_dir,
                })
                .collect(),
        })
    }

    #[test]
    fn git_files_bring_their_folders() {
        let entries = with_folders(vec!["src/palette/files.rs".into(), "src/main.rs".into()]);
        let mut folders: Vec<&str> = entries
            .iter()
            .filter(|e| e.is_dir)
            .map(|e| e.rel.as_str())
            .collect();
        folders.sort();
        assert_eq!(folders, ["src", "src/palette"]);
        assert_eq!(entries.iter().filter(|e| !e.is_dir).count(), 2);
    }

    #[test]
    fn subsequence_ignores_case() {
        let needle: Vec<char> = "mrs".chars().collect();
        assert!(is_subsequence(&needle, "src/Main.RS".chars()));
        assert!(!is_subsequence(&needle, "src/lib.rs".chars()));
    }

    #[test]
    fn names_rank_above_paths_and_shallow_above_deep() {
        let folders = [folder(
            "app",
            &[
                ("src/deep/inner/main.rs", false),
                ("src/main.rs", false),
                ("main", true),
                ("main/other.rs", false),
            ],
        )];
        let (hits, matches) = search(&folders, "main");
        let places: Vec<&str> = hits.iter().map(|h| h.subtitle.as_str()).collect();
        assert_eq!(
            places,
            [
                "app/main",
                "app/src/main.rs",
                "app/src/deep/inner/main.rs",
                "app/main/other.rs"
            ]
        );
        assert_eq!(hits[0].title, "main/");
        assert_eq!(matches[0].ix, 0);
        assert_eq!(hits[1].path, PathBuf::from("/repos/app/src/main.rs"));
    }

    #[test]
    fn searches_by_project_name_too() {
        let folders = [
            folder("web", &[("index.ts", false)]),
            folder("api", &[("index.ts", false)]),
        ];
        let (hits, _) = search(&folders, "api index");
        assert_eq!(hits[0].subtitle, "api/index.ts");
    }
}
