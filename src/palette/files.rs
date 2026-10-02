//! `$`: finding a file in every listed project by its name and the folders
//! it's in, as ctrl-p does in an editor, then opening it in its project's
//! editor.
//!
//! Each project folder's files come from `git ls-files` (so what `.gitignore`
//! leaves out stays out), or else a walk that skips build output and
//! installed packages. They're read in the background the first time `$` is
//! typed and kept between openings, read again when old.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use gpui::Context;

use crate::{
    fuzzy::{Query, Text},
    git,
    store::Project,
    templates::SKIPPED,
};

use super::{
    Palette,
    items::{List, Match},
};

pub(super) const FILES_PREFIX: char = '$';
/// The most rows a search shows: the best matches.
pub(super) const MAX_RESULTS: usize = 200;
/// Where the walk of a folder that isn't a git repository stops.
const MAX_FILES: usize = 100_000;
/// A folder's list is read again after this; the old one shows meanwhile.
const FRESH_FOR: Duration = Duration::from_secs(60);

/// Each folder's files, and when they were read.
type Cache = HashMap<PathBuf, (Arc<FolderFiles>, Instant)>;

/// Kept between openings of the palette.
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Mutex::default);

/// The files in a project's folder.
pub(super) struct FolderFiles {
    root: PathBuf,
    /// Each file as shown and searched: the folder's name, then its path in
    /// it, with `/` between the parts ("proj/src/main.rs").
    places: Vec<String>,
    /// Where the path in the folder starts in each place, past the "/".
    rel_at: usize,
}

/// A file `$` found.
pub(super) struct FileHit {
    /// The project folder it's in.
    pub(super) root: PathBuf,
    pub(super) path: PathBuf,
    /// Its name.
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
        let files = match git::listed_files(&root) {
            Some(files) => files
                .into_iter()
                .map(|file| file.to_string_lossy().replace('\\', "/"))
                .collect(),
            None => walk(&root),
        };
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.to_string_lossy().into_owned());
        Self {
            places: files.iter().map(|rel| format!("{name}/{rel}")).collect(),
            rel_at: name.len() + 1,
            root,
        }
    }
}

/// A folder that isn't a repository: its files but for build output and
/// installed packages, up to `MAX_FILES`. Links to folders aren't followed.
fn walk(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            if files.len() >= MAX_FILES {
                return files;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                let name = entry.file_name();
                if !SKIPPED
                    .iter()
                    .any(|s| name.to_string_lossy().eq_ignore_ascii_case(s))
                {
                    dirs.push(path);
                }
            } else if let Ok(rel) = path.strip_prefix(root) {
                files.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    files
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

/// The query's words: split at spaces and at `/` or `\`, so a path typed
/// either way works.
fn words(query: &str) -> Vec<&str> {
    query
        .split(|c: char| c == '/' || c == '\\' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .collect()
}

/// A `$` query, ready to match every file with.
struct FileQuery {
    /// The last word: for the file's name.
    name: Query,
    /// The words before it: for the folders it's in. `None` if there are none.
    folders: Option<Query>,
    /// All of them: for anywhere along its path.
    anywhere: Query,
}

impl FileQuery {
    fn new(query: &str) -> Option<Self> {
        let words = words(query);
        let (last, before) = words.split_last()?;
        Some(Self {
            name: Query::new(last),
            folders: (!before.is_empty()).then(|| Query::new(&before.join(" "))),
            anywhere: Query::new(&words.join(" ")),
        })
    }

    /// A file's score, and with `positions`, the matched byte offsets in its
    /// name and in its place. The last word goes against its name and the
    /// ones before it against the folders it's in: "palette files",
    /// `src\main`. Failing that, all of them anywhere along its path (just a
    /// folder's name, say), which ranks lower. Between equals, the shallower
    /// file.
    fn score(&self, place: &str, positions: bool) -> Option<(i32, Vec<usize>, Vec<usize>)> {
        let (folders, name) = place.split_at(place.rfind('/').map_or(0, |slash| slash + 1));
        let depth = place.matches('/').count() as i32;
        if let Some((name_score, name_hl)) = self.name.score(name, Text::Name, positions) {
            let in_folders = match &self.folders {
                Some(query) => query.score(folders, Text::Path, positions),
                None => Some((0, Vec::new())),
            };
            if let Some((folders_score, folders_hl)) = in_folders {
                let score = 1000 + name_score + folders_score - depth;
                return Some((score, name_hl, folders_hl));
            }
        }
        let (score, hl) = self.anywhere.score(place, Text::Path, positions)?;
        Some((score - depth, Vec::new(), hl))
    }
}

/// The best `MAX_RESULTS` files for `query`, best first. Every file is
/// scored, and only those that show get their highlights worked out.
fn search(folders: &[Arc<FolderFiles>], query: &str) -> (Vec<FileHit>, Vec<Match>) {
    let Some(query) = FileQuery::new(query) else {
        return Default::default();
    };
    let mut found: Vec<(i32, &FolderFiles, &str)> = folders
        .iter()
        .flat_map(|folder| {
            let query = &query;
            folder.places.iter().filter_map(move |place| {
                let (score, ..) = query.score(place, false)?;
                Some((score, folder.as_ref(), place.as_str()))
            })
        })
        .collect();
    let best = |a: &(i32, &FolderFiles, &str), b: &(i32, &FolderFiles, &str)| {
        b.0.cmp(&a.0).then_with(|| a.2.len().cmp(&b.2.len()))
    };
    if found.len() > MAX_RESULTS {
        found.select_nth_unstable_by(MAX_RESULTS, best);
        found.truncate(MAX_RESULTS);
    }
    found.sort_by(best);
    found
        .into_iter()
        .enumerate()
        .map(|(ix, (_, folder, place))| {
            let (_, title_hl, subtitle_hl) = query.score(place, true).unwrap_or_default();
            let path = place[folder.rel_at..]
                .split('/')
                .fold(folder.root.clone(), |path, part| path.join(part));
            let hit = FileHit {
                root: folder.root.clone(),
                path,
                title: place.rsplit('/').next().unwrap_or(place).to_string(),
                subtitle: place.to_string(),
            };
            let row = Match {
                ix,
                title_hl,
                subtitle_hl,
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

    fn folder(name: &str, files: &[&str]) -> Arc<FolderFiles> {
        Arc::new(FolderFiles {
            root: PathBuf::from("/repos").join(name),
            places: files.iter().map(|f| format!("{name}/{f}")).collect(),
            rel_at: name.len() + 1,
        })
    }

    fn places(folders: &[Arc<FolderFiles>], query: &str) -> Vec<String> {
        search(folders, query)
            .0
            .into_iter()
            .map(|hit| hit.subtitle)
            .collect()
    }

    #[test]
    fn words_split_at_either_slash() {
        assert_eq!(words(r"src\palette  files"), ["src", "palette", "files"]);
        assert_eq!(words("src/main.rs"), ["src", "main.rs"]);
        assert!(words(r" / \ ").is_empty());
    }

    #[test]
    fn case_doesnt_matter() {
        let folders = [folder("app", &["src/Main.RS", "src/lib.rs"])];
        assert_eq!(places(&folders, "mrs"), ["app/src/Main.RS"]);
    }

    #[test]
    fn names_rank_above_paths_and_shallow_above_deep() {
        let folders = [folder(
            "app",
            &["src/deep/inner/main.rs", "src/main.rs", "main/other.rs"],
        )];
        assert_eq!(
            places(&folders, "main"),
            [
                "app/src/main.rs",
                "app/src/deep/inner/main.rs",
                "app/main/other.rs"
            ]
        );
        let (hits, matches) = search(&folders, "main");
        assert_eq!(hits[0].title, "main.rs");
        assert_eq!(matches[0].ix, 0);
        assert_eq!(hits[0].path, PathBuf::from("/repos/app/src/main.rs"));
    }

    #[test]
    fn words_before_the_name_match_its_folders() {
        let folders = [folder(
            "proj",
            &["src/palette/files.rs", "src/files.rs", "docs/palette.md"],
        )];
        for query in [
            "palette files",
            "palette/files",
            r"palette\files",
            r"src\pal\fil",
        ] {
            assert_eq!(
                places(&folders, query)[0],
                "proj/src/palette/files.rs",
                "{query}"
            );
        }
        // A folder's name alone finds what's in it, after files named so.
        assert_eq!(
            places(&folders, "palette"),
            ["proj/docs/palette.md", "proj/src/palette/files.rs"]
        );
        // The highlights: the name in the title, the folders in the path.
        let (_, matches) = search(&folders, "palette files");
        assert_eq!(matches[0].title_hl, [0, 1, 2, 3, 4]);
        // The word together, not the "p" of "proj" and the rest later.
        assert_eq!(matches[0].subtitle_hl, (9..16).collect::<Vec<_>>());
    }

    #[test]
    fn words_before_the_name_go_in_any_order() {
        let folders = [folder("proj", &["src/palette/files.rs", "src/files.rs"])];
        assert_eq!(
            places(&folders, "palette src files")[0],
            "proj/src/palette/files.rs"
        );
    }

    #[test]
    fn searches_by_project_name_too() {
        let folders = [folder("web", &["index.ts"]), folder("api", &["index.ts"])];
        assert_eq!(places(&folders, "api index"), ["api/index.ts"]);
    }
}
