//! Branch and remote information, read straight from `.git` without running git.
//! Cloning is the one place that runs it.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
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

/// Web page of the repository's `origin` remote (or its first remote), e.g.
/// `https://git.tomiworld.com/web/interactive-v2`.
pub fn git_web_url(path: &Path) -> Option<String> {
    let mut git_dir = git_dir(path)?;
    // Worktrees keep the shared config in the main repository.
    if let Ok(common) = fs::read_to_string(git_dir.join("commondir")) {
        git_dir = git_dir.join(common.trim());
    }
    let config = fs::read_to_string(git_dir.join("config")).ok()?;
    remote_web_url(&remote_url(&config)?)
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
    let mut command = Command::new("git");
    command
        .args(["clone", "--", url])
        .arg(dest)
        // Fail instead of waiting for a username on a terminal nobody sees.
        // Credential helpers (Git Credential Manager) still show their own window.
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
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
