//! Branch and remote information, read straight from `.git` without running git.
//! Status (changes, ahead/behind) and cloning are the places that run it.

use std::{
    collections::{BTreeMap, HashMap},
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

/// The repository's git directory: `.git`, or where a worktree/submodule's `.git` file points.
fn git_dir(path: &Path) -> Option<PathBuf> {
    let dot_git = path.join(".git");
    if dot_git.is_file() {
        let text = fs::read_to_string(&dot_git).ok()?;
        Some(path.join(text.strip_prefix("gitdir:")?.trim()))
    } else {
        dot_git.is_dir().then_some(dot_git)
    }
}

/// Current branch (or short commit when detached), read straight from `.git/HEAD`.
pub(crate) fn git_branch(path: &Path) -> Option<String> {
    let head = fs::read_to_string(git_dir(path)?.join("HEAD")).ok()?;
    let head = head.trim();
    Some(match head.strip_prefix("ref: ") {
        Some(reference) => reference
            .strip_prefix("refs/heads/")
            .unwrap_or(reference)
            .to_string(),
        None => head.get(..7)?.to_string(),
    })
}

/// Uncommitted changes, and commits ahead of and behind the upstream branch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GitStatus {
    /// Changed, staged or untracked files.
    pub dirty: bool,
    pub ahead: u32,
    pub behind: u32,
}

impl GitStatus {
    /// What goes after the branch name: " ●" for changes, " ↑2 ↓1" for
    /// commits to push and pull. Empty when there's nothing to say.
    pub fn suffix(self) -> String {
        let mut suffix = String::new();
        if self.dirty {
            suffix.push_str(" ●");
        }
        if self.ahead > 0 {
            suffix.push_str(&format!(" ↑{}", self.ahead));
        }
        if self.behind > 0 {
            suffix.push_str(&format!(" ↓{}", self.behind));
        }
        suffix
    }
}

/// How long a status read by [`refresh_status`] counts as current.
const STATUS_MAX_AGE: Duration = Duration::from_secs(10);

static STATUS: LazyLock<Mutex<HashMap<PathBuf, (GitStatus, Instant)>>> =
    LazyLock::new(Mutex::default);

/// The status last read for `path`, and whether it's due to be read again.
pub fn cached_status(path: &Path) -> (Option<GitStatus>, bool) {
    let cache = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    match cache.get(path) {
        Some(&(status, read)) => (Some(status), read.elapsed() > STATUS_MAX_AGE),
        None => (None, true),
    }
}

/// Runs `git status` in `path` and remembers the result. Slow in big
/// repositories, so call it off the main thread.
pub fn refresh_status(path: &Path) -> Option<GitStatus> {
    git_dir(path)?;
    let mut command = Command::new("git");
    command
        // Don't take the index lock from under an editor that's using it.
        .args([
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "--branch",
        ])
        .current_dir(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    hide_window(&mut command);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let status = parse_status(&String::from_utf8_lossy(&output.stdout));
    STATUS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(path.to_path_buf(), (status, Instant::now()));
    Some(status)
}

/// `git status --porcelain=v2 --branch`: `# branch.ab +2 -1` says ahead and
/// behind; every line that isn't a `#` header is a changed file.
fn parse_status(text: &str) -> GitStatus {
    let mut status = GitStatus::default();
    for line in text.lines() {
        if let Some(counts) = line.strip_prefix("# branch.ab ") {
            let mut counts = counts.split_whitespace();
            let mut count = |sign: char| {
                counts
                    .next()
                    .and_then(|c| c.strip_prefix(sign)?.parse().ok())
                    .unwrap_or(0)
            };
            status.ahead = count('+');
            status.behind = count('-');
        } else if !line.is_empty() && !line.starts_with('#') {
            status.dirty = true;
        }
    }
    status
}

/// The repository's config file, shared by its worktrees.
fn git_config(path: &Path) -> Option<String> {
    let mut git_dir = git_dir(path)?;
    // Worktrees keep the shared config in the main repository.
    if let Ok(common) = fs::read_to_string(git_dir.join("commondir")) {
        git_dir = git_dir.join(common.trim());
    }
    fs::read_to_string(git_dir.join("config")).ok()
}

/// The URL of the repository's `origin` remote (or its first remote), as
/// `git clone` takes it.
pub fn git_remote_url(path: &Path) -> Option<String> {
    remote_url(&git_config(path)?)
}

/// Web page of the repository's `origin` remote (or its first remote), e.g.
/// `https://git.tomiworld.com/web/interactive-v2`.
pub fn git_web_url(path: &Path) -> Option<String> {
    remote_web_url(&git_remote_url(path)?)
}

/// The kind of site a repository is on, for its pull request and CI pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forge {
    GitHub,
    GitLab,
    /// Gitea and Forgejo (Codeberg).
    Gitea,
    Bitbucket,
    AzureDevOps,
}

impl Forge {
    /// The names `forges` in config.toml takes.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            "github" => Some(Self::GitHub),
            "gitlab" => Some(Self::GitLab),
            "gitea" | "forgejo" => Some(Self::Gitea),
            "bitbucket" => Some(Self::Bitbucket),
            "azure" | "azure-devops" | "azuredevops" => Some(Self::AzureDevOps),
            _ => None,
        }
    }

    /// The site a repository web page is on: as set in `forges` (host ->
    /// kind), else the well-known sites and hosts named after their software
    /// (gitlab.example.com).
    pub fn for_url(web_url: &str, forges: &BTreeMap<String, String>) -> Option<Self> {
        let host = url_host(web_url)?.to_lowercase();
        if let Some(forge) = forges
            .iter()
            .find(|(h, _)| h.eq_ignore_ascii_case(&host))
            .and_then(|(_, kind)| Self::from_name(kind))
        {
            return Some(forge);
        }
        let named = |name: &str| host.split(['.', '-']).any(|part| part == name);
        if host == "github.com" || named("github") {
            Some(Self::GitHub)
        } else if host == "dev.azure.com" || host.ends_with(".visualstudio.com") {
            Some(Self::AzureDevOps)
        } else if host == "bitbucket.org" || named("bitbucket") {
            Some(Self::Bitbucket)
        } else if named("gitlab") {
            Some(Self::GitLab)
        } else if host == "codeberg.org" || named("gitea") || named("forgejo") {
            Some(Self::Gitea)
        } else {
            None
        }
    }

    /// For display: "GitLab".
    pub fn name(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::GitLab => "GitLab",
            Self::Gitea => "Gitea",
            Self::Bitbucket => "Bitbucket",
            Self::AzureDevOps => "Azure DevOps",
        }
    }

    /// The repository's pull (merge) requests.
    pub fn pull_requests(self, web_url: &str) -> String {
        match self {
            Self::GitHub | Self::Gitea => format!("{web_url}/pulls"),
            Self::GitLab => format!("{web_url}/-/merge_requests"),
            Self::Bitbucket => format!("{web_url}/pull-requests"),
            Self::AzureDevOps => format!("{web_url}/pullrequests"),
        }
    }

    /// The repository's CI runs.
    pub fn ci(self, web_url: &str) -> String {
        match self {
            Self::GitHub | Self::Gitea => format!("{web_url}/actions"),
            Self::GitLab => format!("{web_url}/-/pipelines"),
            Self::Bitbucket => format!("{web_url}/pipelines"),
            // Pipelines belong to the project: …/org/project/_git/repo → …/org/project/_build
            Self::AzureDevOps => match web_url.split_once("/_git/") {
                Some((project, _)) => format!("{project}/_build"),
                None => format!("{web_url}/_build"),
            },
        }
    }
}

/// "git.example.com" from "https://git.example.com:3000/team/app".
pub fn url_host(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split('/').next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host.split(':').next()?;
    (!host.is_empty()).then_some(host)
}

/// `url` of `[remote "origin"]`, else of the first remote, from a git config file.
fn remote_url(config: &str) -> Option<String> {
    let mut section = String::new();
    let mut first = None;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line.to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "url" || !section.starts_with("[remote ") {
            continue;
        }
        let url = value.trim().trim_matches('"').to_string();
        if section == "[remote \"origin\"]" {
            return Some(url);
        }
        first.get_or_insert(url);
    }
    first
}

/// Turns a clone URL (ssh, scp-style or http) into the repository's web page.
fn remote_web_url(url: &str) -> Option<String> {
    let (scheme, host, path) = if let Some((scheme, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        match scheme {
            "http" | "https" => (scheme, host.to_string(), path),
            // The ssh port isn't the web port.
            "ssh" | "git" | "git+ssh" => ("https", host.split(':').next()?.to_string(), path),
            _ => return None,
        }
    } else {
        // scp-like: [user@]host:group/repo.git (not a Windows drive like C:\...)
        let (authority, path) = url.split_once(':')?;
        if authority.len() < 2 || path.starts_with('\\') {
            return None;
        }
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        ("https", host.to_string(), path)
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if host.is_empty() || path.is_empty() {
        return None;
    }
    // Azure DevOps ssh remotes: ssh.dev.azure.com:v3/org/project/repo
    if host == "ssh.dev.azure.com"
        && let Some(["v3", org, project, repo]) =
            Some(path.split('/').collect::<Vec<_>>().as_slice())
    {
        return Some(format!("https://dev.azure.com/{org}/{project}/_git/{repo}"));
    }
    Some(format!("{scheme}://{host}/{path}"))
}

/// The folder `git clone` would create, if `text` is a remote URL:
/// `https://host/owner/repo`, `ssh://…`, or scp-style `git@host:owner/repo.git`.
pub fn clone_name(text: &str) -> Option<String> {
    if text.contains(char::is_whitespace) || text.contains(['?', '#']) {
        return None;
    }
    let path = if let Some((scheme, rest)) = text.split_once("://") {
        if !matches!(scheme, "http" | "https" | "ssh" | "git" | "git+ssh") {
            return None;
        }
        rest.split_once('/')?.1
    } else {
        // scp-style needs the user part, so a search like "app:v2" isn't a URL.
        let (authority, path) = text.split_once(':')?;
        if !authority.contains('@') {
            return None;
        }
        path
    };
    let name = path.trim_end_matches('/').rsplit('/').next()?;
    let name = name.strip_suffix(".git").unwrap_or(name);
    (!name.is_empty() && name != "." && name != "..").then(|| name.to_string())
}

/// Runs `git clone url dest`. The error is git's own message, e.g.
/// "repository 'https://…' not found".
pub fn clone(url: &str, dest: &Path) -> io::Result<()> {
    run_clone(url, dest, &[])
}

/// Clones just the latest commit, for a template whose history isn't wanted.
pub fn clone_latest(url: &str, dest: &Path) -> io::Result<()> {
    run_clone(url, dest, &["--depth", "1"])
}

/// Starts a new repository in `dir`.
pub fn init(dir: &Path) -> io::Result<()> {
    let mut command = Command::new("git");
    command
        .arg("init")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_window(&mut command);
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("git init failed"))
    }
}

/// The files of a repository a copy should have, relative to `dir`: tracked
/// ones and new ones, without what `.gitignore` leaves out. `None` if `dir`
/// isn't a repository (or git isn't installed).
pub fn listed_files(dir: &Path) -> Option<Vec<PathBuf>> {
    git_dir(dir)?;
    let mut command = Command::new("git");
    command
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    hide_window(&mut command);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .split('\0')
            .filter(|f| !f.is_empty())
            .map(PathBuf::from)
            .collect(),
    )
}

fn run_clone(url: &str, dest: &Path, options: &[&str]) -> io::Result<()> {
    let mut command = Command::new("git");
    command
        .arg("clone")
        .args(options)
        .args(["--", url])
        .arg(dest)
        // Fail instead of waiting for a username on a terminal nobody sees.
        // Credential helpers (Git Credential Manager) still show their own window.
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    hide_window(&mut command);
    let output = command.output().map_err(|err| match err.kind() {
        io::ErrorKind::NotFound => io::Error::new(err.kind(), "git is not installed"),
        _ => err,
    })?;
    if output.status.success() {
        return Ok(());
    }
    Err(io::Error::other(clone_error(&String::from_utf8_lossy(
        &output.stderr,
    ))))
}

/// No console window flashing up for git on Windows.
fn hide_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// The line that says what went wrong: git follows it with hints like
/// "Please make sure you have the correct access rights".
fn clone_error(stderr: &str) -> String {
    let mut lines = stderr.lines().map(str::trim).filter(|l| !l.is_empty());
    match lines.clone().find_map(|l| l.strip_prefix("fatal: ")) {
        Some(fatal) => fatal.to_string(),
        None => lines.next_back().unwrap_or("git clone failed").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_urls_and_folder_names() {
        let cases = [
            ("https://github.com/lmsebastiao/proj", "proj"),
            ("https://github.com/lmsebastiao/proj.git", "proj"),
            (
                "https://git.tomiworld.com/web/interactive-v2/",
                "interactive-v2",
            ),
            (
                "ssh://git@git.tomiworld.com:222/web/interactive-v2.git",
                "interactive-v2",
            ),
            ("git@github.com:owner/repo.git", "repo"),
            ("git@ssh.dev.azure.com:v3/org/project/repo", "repo"),
        ];
        for (url, name) in cases {
            assert_eq!(clone_name(url).as_deref(), Some(name), "{url}");
        }
        for text in [
            "interactive",
            "app:v2",
            r"C:\repos\app",
            "https://github.com",
            "https://github.com/",
            "https://github.com/o/r?tab=readme",
            "ftp://host/repo",
            "git@github.com:owner/repo extra",
        ] {
            assert_eq!(clone_name(text), None, "{text}");
        }
    }

    #[test]
    fn clone_errors_are_git_first_complaint() {
        let stderr = "Cloning into 'x'...\n\
            fatal: 'C:/repos/missing' does not appear to be a git repository\n\
            fatal: Could not read from remote repository.\n\n\
            Please make sure you have the correct access rights\n\
            and the repository exists.\n";
        assert_eq!(
            clone_error(stderr),
            "'C:/repos/missing' does not appear to be a git repository"
        );
        assert_eq!(
            clone_error("error: something odd\n"),
            "error: something odd"
        );
        assert_eq!(clone_error(""), "git clone failed");
    }

    #[test]
    fn reads_git_branch() {
        let dir = std::env::temp_dir().join(format!("proj-test-{}", std::process::id()));
        let repo = dir.join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join(".git/HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        assert_eq!(git_branch(&repo).as_deref(), Some("feature/x"));

        fs::write(repo.join(".git/HEAD"), "0123456789abcdef\n").unwrap();
        assert_eq!(git_branch(&repo).as_deref(), Some("0123456"));

        // Worktree: .git is a file pointing elsewhere.
        let worktree = dir.join("wt");
        fs::create_dir_all(dir.join("gitdir")).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(dir.join("gitdir/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", dir.join("gitdir").display()),
        )
        .unwrap();
        assert_eq!(git_branch(&worktree).as_deref(), Some("main"));

        assert_eq!(git_branch(&dir), None);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn git_remotes_become_web_urls() {
        let cases = [
            (
                "ssh://git@git.tomiworld.com:222/web/interactive-v2.git",
                "https://git.tomiworld.com/web/interactive-v2",
            ),
            (
                "https://github.com/lmsebastiao/proj.git",
                "https://github.com/lmsebastiao/proj",
            ),
            (
                "https://user:token@git.tomiworld.com/tomi/shared-sdk.git",
                "https://git.tomiworld.com/tomi/shared-sdk",
            ),
            (
                "git@github.com:owner/repo.git",
                "https://github.com/owner/repo",
            ),
            (
                "http://gitea.local:3000/team/app/",
                "http://gitea.local:3000/team/app",
            ),
            (
                "git@ssh.dev.azure.com:v3/org/project/repo",
                "https://dev.azure.com/org/project/_git/repo",
            ),
        ];
        for (remote, web) in cases {
            assert_eq!(remote_web_url(remote).as_deref(), Some(web), "{remote}");
        }
        assert_eq!(remote_web_url(r"C:\repos\local-mirror"), None);
        assert_eq!(remote_web_url("/srv/git/repo.git"), None);
    }

    #[test]
    fn reads_status_output() {
        let clean = "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n\
                     # branch.ab +0 -0\n";
        assert_eq!(parse_status(clean), GitStatus::default());
        assert_eq!(parse_status(clean).suffix(), "");
        let busy = "# branch.head main\n# branch.ab +2 -11\n\
                    1 .M N... 100644 100644 100644 a b src/main.rs\n? notes.txt\n";
        let status = parse_status(busy);
        assert_eq!(
            status,
            GitStatus {
                dirty: true,
                ahead: 2,
                behind: 11
            }
        );
        assert_eq!(status.suffix(), " ● ↑2 ↓11");
        // No upstream: no branch.ab line.
        assert_eq!(parse_status("# branch.head wip\n? new.rs\n").suffix(), " ●");
    }

    #[test]
    fn links_to_pull_requests_and_ci() {
        let none = BTreeMap::new();
        let forge = |url: &str| Forge::for_url(url, &none);
        let gh = "https://github.com/lmsebastiao/proj";
        assert_eq!(forge(gh), Some(Forge::GitHub));
        assert_eq!(Forge::GitHub.pull_requests(gh), format!("{gh}/pulls"));
        assert_eq!(Forge::GitHub.ci(gh), format!("{gh}/actions"));
        let gl = "https://gitlab.example.com/team/app";
        assert_eq!(forge(gl), Some(Forge::GitLab));
        assert_eq!(Forge::GitLab.ci(gl), format!("{gl}/-/pipelines"));
        assert_eq!(
            forge("https://codeberg.org/me/app"),
            Some(Forge::Gitea),
            "Forgejo"
        );
        let azure = "https://dev.azure.com/org/project/_git/repo";
        assert_eq!(forge(azure), Some(Forge::AzureDevOps));
        assert_eq!(
            Forge::AzureDevOps.ci(azure),
            "https://dev.azure.com/org/project/_build"
        );
        // Self-hosted sites with no telling name: from `forges` in config.toml.
        let own = "https://git.tomiworld.com/web/interactive-v2";
        assert_eq!(forge(own), None);
        let forges = BTreeMap::from([("git.tomiworld.com".to_string(), "Forgejo".to_string())]);
        assert_eq!(Forge::for_url(own, &forges), Some(Forge::Gitea));
        assert_eq!(
            url_host("http://gitea.local:3000/team/app"),
            Some("gitea.local")
        );
    }

    #[test]
    fn prefers_origin_remote() {
        let config = r#"
[core]
    bare = false
[remote "upstream"]
    url = https://github.com/upstream/repo.git
    fetch = +refs/heads/*:refs/remotes/upstream/*
[remote "origin"]
    url = git@github.com:me/repo.git
"#;
        assert_eq!(
            remote_url(config).as_deref(),
            Some("git@github.com:me/repo.git")
        );
        assert_eq!(
            remote_url("[remote \"fork\"]\n\turl = https://x.com/a/b\n").as_deref(),
            Some("https://x.com/a/b")
        );
        assert_eq!(remote_url("[core]\n\tbare = false\n"), None);
    }
}
