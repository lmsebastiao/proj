//! Projects that aren't plain local folders: in WSL (`\\wsl.localhost\Ubuntu\…`),
//! on another machine over SSH (`ssh://me@box/home/me/app`), and local ones
//! with a dev container. Editors that know about them open them their own
//! way: VS Code and its forks through their remote extensions, Zed over SSH;
//! terminals open inside WSL or over SSH.

use std::path::{Path, PathBuf};

use crate::jsonc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Remote {
    /// A folder in a WSL distribution, by its Linux path.
    Wsl { distro: String, path: String },
    /// A folder on another machine: `host` may have a user (`me@box`).
    Ssh {
        host: String,
        port: Option<u16>,
        path: String,
    },
}

impl Remote {
    /// What a project's path says: `\\wsl$\Ubuntu\home\me\app` or
    /// `\\wsl.localhost\…`, or `ssh://[user@]host[:port]/path`.
    pub fn of(path: &Path) -> Option<Self> {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix("ssh://") {
            let (authority, path) = rest.split_once('/')?;
            let (host, port) = match authority.rsplit_once(':') {
                Some((host, port)) => (host, Some(port.parse().ok()?)),
                None => (authority, None),
            };
            let host_name = host.rsplit_once('@').map_or(host, |(_, h)| h);
            if host_name.is_empty() {
                return None;
            }
            return Some(Self::Ssh {
                host: host.to_string(),
                port,
                path: format!("/{}", path.trim_end_matches('/')),
            });
        }
        let lower = text.to_lowercase();
        let prefix = [
            r"\\wsl$\",
            r"\\wsl.localhost\",
            "//wsl$/",
            "//wsl.localhost/",
        ]
        .into_iter()
        .find(|p| lower.starts_with(p))?;
        let rest = &text[prefix.len()..];
        let (distro, path) = rest.split_once(['\\', '/']).unwrap_or((rest, ""));
        if distro.is_empty() {
            return None;
        }
        Some(Self::Wsl {
            distro: distro.to_string(),
            path: format!("/{}", path.replace('\\', "/").trim_end_matches('/')),
        })
    }

    /// For the right of its row: "SSH · box", "WSL · Ubuntu".
    pub fn label(&self) -> String {
        match self {
            Self::Wsl { distro, .. } => format!("WSL · {distro}"),
            Self::Ssh { host, .. } => {
                let name = host.rsplit_once('@').map_or(host.as_str(), |(_, h)| h);
                format!("SSH · {name}")
            }
        }
    }

    pub fn is_ssh(&self) -> bool {
        matches!(self, Self::Ssh { .. })
    }

    /// The URI VS Code (and its forks) open it with: through the WSL or
    /// Remote - SSH extension.
    pub(crate) fn vs_code_uri(&self) -> String {
        match self {
            Self::Wsl { distro, path } => {
                format!(
                    "vscode-remote://wsl+{}{}",
                    encode(distro),
                    encode_path(path)
                )
            }
            Self::Ssh { host, port, path } => {
                // A port can't go in the plain form: it takes the host as
                // hex-encoded JSON then, as the extension's own links do.
                let authority = match port {
                    None => encode(host),
                    Some(port) => {
                        let (user, name) = match host.rsplit_once('@') {
                            Some((user, name)) => (Some(user), name),
                            None => (None, host.as_str()),
                        };
                        let mut json = serde_json::json!({ "hostName": name, "port": port });
                        if let Some(user) = user {
                            json["user"] = user.into();
                        }
                        hex(json.to_string().as_bytes())
                    }
                };
                format!(
                    "vscode-remote://ssh-remote+{authority}{}",
                    encode_path(path)
                )
            }
        }
    }

    /// `ssh://me@box:2222/home/me/app`, as Zed takes it.
    fn ssh_url(&self) -> Option<String> {
        let Self::Ssh { host, port, path } = self else {
            return None;
        };
        let port = port.map(|p| format!(":{p}")).unwrap_or_default();
        Some(format!("ssh://{host}{port}{path}"))
    }

    /// The program and arguments that start a shell there, running `command`
    /// first if there is one; the shell stays open after it.
    pub fn shell(&self, command: Option<&str>) -> (String, Vec<String>) {
        match self {
            Self::Wsl { distro, path } => {
                let mut args = vec!["-d".into(), distro.clone(), "--cd".into(), path.clone()];
                if let Some(command) = command {
                    args.extend([
                        "--".into(),
                        "sh".into(),
                        "-lc".into(),
                        format!("{command}; exec \"${{SHELL:-sh}}\" -l"),
                    ]);
                }
                ("wsl.exe".into(), args)
            }
            Self::Ssh { host, port, path } => {
                let mut args = vec!["-t".to_string()];
                if let Some(port) = port {
                    args.extend(["-p".into(), port.to_string()]);
                }
                let run = command.map(|c| format!("{c}; ")).unwrap_or_default();
                args.extend([
                    host.clone(),
                    format!("cd {} && {run}exec \"$SHELL\" -l", shell_quote(path)),
                ]);
                ("ssh".into(), args)
            }
        }
    }
}

/// Editors that open remote folders their own way.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    VsCode,
    Zed,
    Other,
}

fn family(editor: &str) -> Family {
    let path = Path::new(editor.trim());
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match stem.as_str() {
        "code" | "code-insiders" | "cursor" | "windsurf" | "codium" => Family::VsCode,
        "zed" | "zed-preview" => Family::Zed,
        // macOS: the command inside the app.
        "cli" if editor.contains("/Zed") => Family::Zed,
        _ => Family::Other,
    }
}

/// Whether `editor` is VS Code or one of its forks, which open dev containers.
pub fn is_vs_code(editor: &str) -> bool {
    family(editor) == Family::VsCode
}

/// The arguments that open `paths` in `editor` when some are remote:
/// `Ok(None)` when they open as plain folders (none is remote, or the editor
/// reads WSL's folders through their network path), or why it can't.
pub fn editor_args(editor: &str, paths: &[PathBuf]) -> Result<Option<Vec<String>>, String> {
    let remotes: Vec<Option<Remote>> = paths.iter().map(|p| Remote::of(p)).collect();
    if remotes.iter().all(Option::is_none) {
        return Ok(None);
    }
    match family(editor) {
        Family::VsCode => {
            let mut args = Vec::new();
            for (path, remote) in paths.iter().zip(&remotes) {
                let uri = match remote {
                    Some(remote) => remote.vs_code_uri(),
                    None => file_uri(path),
                };
                args.extend(["--folder-uri".to_string(), uri]);
            }
            Ok(Some(args))
        }
        Family::Zed if remotes.iter().flatten().any(Remote::is_ssh) => {
            let mut args = Vec::new();
            for (path, remote) in paths.iter().zip(&remotes) {
                match remote.as_ref().and_then(Remote::ssh_url) {
                    Some(url) => args.push(url),
                    None => args.push(path.to_string_lossy().into_owned()),
                }
            }
            Ok(Some(args))
        }
        Family::Zed | Family::Other if remotes.iter().flatten().any(Remote::is_ssh) => {
            Err("it can't open projects over SSH (VS Code, its forks and Zed can)".into())
        }
        // WSL: through its network path.
        Family::Zed | Family::Other => Ok(None),
    }
}

/// A `ssh://` URL typed or pasted to add as a project, in the form it's kept:
/// `ssh://[user@]host[:port]/path`. Not one for cloning: those end in `.git`
/// or log in as `git`.
pub fn parse_ssh(text: &str) -> Option<PathBuf> {
    let text = text.trim();
    let rest = text.strip_prefix("ssh://")?;
    let (authority, path) = rest.split_once('/')?;
    if text.contains(char::is_whitespace)
        || path.trim_end_matches('/').ends_with(".git")
        || authority.starts_with("git@")
        || path.is_empty()
    {
        return None;
    }
    let path = PathBuf::from(format!("ssh://{authority}/{}", path.trim_end_matches('/')));
    Remote::of(&path).map(|_| path)
}

/// The dev container configuration of a local project folder:
/// `.devcontainer/devcontainer.json`, `.devcontainer.json`, or the first of
/// `.devcontainer/<name>/devcontainer.json`.
pub fn devcontainer(dir: &Path) -> Option<PathBuf> {
    if Remote::of(dir).is_some() {
        return None;
    }
    let candidates = [
        dir.join(".devcontainer").join("devcontainer.json"),
        dir.join(".devcontainer.json"),
    ];
    if let Some(found) = candidates.into_iter().find(|p| p.is_file()) {
        return Some(found);
    }
    let mut named: Vec<PathBuf> = std::fs::read_dir(dir.join(".devcontainer"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("devcontainer.json"))
        .filter(|p| p.is_file())
        .collect();
    named.sort();
    named.into_iter().next()
}

/// The URI VS Code reopens `dir` in its dev container with (the Dev
/// Containers extension): the folder, hex-encoded, and where it is in the
/// container: `workspaceFolder` from `config`, else `/workspaces/<name>`.
pub fn devcontainer_uri(dir: &Path, config: &Path) -> String {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let inside = std::fs::read_to_string(config)
        .ok()
        .and_then(|text| jsonc::parse(&text))
        .and_then(|json| json["workspaceFolder"].as_str().map(str::to_string))
        .map(|folder| folder.replace("${localWorkspaceFolderBasename}", &name))
        .unwrap_or_else(|| format!("/workspaces/{name}"));
    let inside = format!("/{}", inside.trim_start_matches('/'));
    format!(
        "vscode-remote://dev-container+{}{}",
        hex(dir.to_string_lossy().as_bytes()),
        encode_path(&inside)
    )
}

/// A Zed remote connection (from its `remote_connections` table) and a
/// folder on it, as a project path: a WSL distribution's network path, or an
/// `ssh://` URL.
pub fn from_zed(
    kind: &str,
    host: Option<&str>,
    user: Option<&str>,
    port: Option<i64>,
    distro: Option<&str>,
    path: &str,
) -> Option<PathBuf> {
    let path = format!("/{}", path.trim_start_matches('/').trim_end_matches('/'));
    match kind {
        "wsl" => Some(wsl_path(distro?, &path)),
        "ssh" => {
            let host = host.filter(|h| !h.is_empty())?;
            let user = user.map(|u| format!("{u}@")).unwrap_or_default();
            let port = port.map(|p| format!(":{p}")).unwrap_or_default();
            Some(PathBuf::from(format!("ssh://{user}{host}{port}{path}")))
        }
        _ => None,
    }
}

/// A folder VS Code opened remotely (`vscode-remote://wsl+Ubuntu/home/me/app`,
/// `vscode-remote://ssh-remote+box/…`), as a project path.
pub fn from_vs_code(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("vscode-remote://")?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let authority = decode(authority)?;
    let path = format!("/{}", decode(path)?.trim_end_matches('/'));
    if let Some(distro) = authority.strip_prefix("wsl+") {
        return Some(wsl_path(distro, &path));
    }
    let host = authority.strip_prefix("ssh-remote+")?;
    // The hex-encoded JSON form, for hosts with a port.
    let decoded =
        unhex(host).and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    let host = match decoded {
        Some(json) => {
            let name = json["hostName"].as_str()?;
            let user = json["user"]
                .as_str()
                .map(|u| format!("{u}@"))
                .unwrap_or_default();
            let port = json["port"]
                .as_u64()
                .map(|p| format!(":{p}"))
                .unwrap_or_default();
            format!("{user}{name}{port}")
        }
        None => host.to_string(),
    };
    Some(PathBuf::from(format!("ssh://{host}{path}")))
}

/// The network path Windows reaches a WSL distribution's folder by.
fn wsl_path(distro: &str, path: &str) -> PathBuf {
    PathBuf::from(format!(
        r"\\wsl.localhost\{distro}{}",
        path.replace('/', "\\").trim_end_matches('\\')
    ))
}

fn file_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = if text.starts_with('/') {
        text
    } else {
        format!("/{text}")
    };
    format!("file://{}", encode_path(&text))
}

/// `text` with everything but unreserved characters percent-encoded.
fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~@".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A path for a URI: each part encoded, the slashes kept.
fn encode_path(path: &str) -> String {
    path.split('/').map(encode).collect::<Vec<_>>().join("/")
}

fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(text.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || text.is_empty() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// `text` in single quotes for a POSIX shell.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_paths() {
        assert_eq!(
            Remote::of(Path::new(r"\\wsl.localhost\Ubuntu-24.04\home\me\app")),
            Some(Remote::Wsl {
                distro: "Ubuntu-24.04".into(),
                path: "/home/me/app".into()
            })
        );
        assert_eq!(
            Remote::of(Path::new(r"\\WSL$\Debian")),
            Some(Remote::Wsl {
                distro: "Debian".into(),
                path: "/".into()
            })
        );
        assert_eq!(
            Remote::of(Path::new("ssh://me@box:2222/home/me/app/")),
            Some(Remote::Ssh {
                host: "me@box".into(),
                port: Some(2222),
                path: "/home/me/app".into()
            })
        );
        assert_eq!(Remote::of(Path::new(r"C:\repos\app")), None);
        assert_eq!(Remote::of(Path::new(r"\\server\share\app")), None);
        assert_eq!(Remote::of(Path::new("ssh://box:port/x")), None);
    }

    #[test]
    fn pasted_ssh_projects_but_not_git_remotes() {
        assert_eq!(
            parse_ssh("ssh://me@box/home/me/app/"),
            Some(PathBuf::from("ssh://me@box/home/me/app"))
        );
        assert!(parse_ssh("ssh://box:22/srv/app").is_some());
        assert_eq!(parse_ssh("ssh://git@github.com/owner/repo"), None);
        assert_eq!(parse_ssh("ssh://me@git.example.com/team/repo.git"), None);
        assert_eq!(parse_ssh("ssh://box/"), None);
        assert_eq!(parse_ssh("https://box/home"), None);
    }

    #[test]
    fn editors_open_them_their_own_way() {
        let wsl = PathBuf::from(r"\\wsl.localhost\Ubuntu\home\me\my app");
        let ssh = PathBuf::from("ssh://me@box/home/me/api");
        let ported = PathBuf::from("ssh://box:2222/srv/x");
        assert_eq!(
            editor_args("code", std::slice::from_ref(&wsl)).unwrap(),
            Some(vec![
                "--folder-uri".into(),
                "vscode-remote://wsl+Ubuntu/home/me/my%20app".into()
            ])
        );
        assert_eq!(
            editor_args(
                r"C:\Programs\cursor\resources\app\bin\cursor.cmd",
                std::slice::from_ref(&ssh)
            )
            .unwrap(),
            Some(vec![
                "--folder-uri".into(),
                "vscode-remote://ssh-remote+me@box/home/me/api".into()
            ])
        );
        let uri = &editor_args("code", std::slice::from_ref(&ported))
            .unwrap()
            .unwrap()[1];
        let host = uri
            .strip_prefix("vscode-remote://ssh-remote+")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&unhex(host).unwrap()).unwrap();
        assert_eq!(json, serde_json::json!({ "hostName": "box", "port": 2222 }));
        assert_eq!(
            editor_args("zed", &[ported]).unwrap(),
            Some(vec!["ssh://box:2222/srv/x".into()])
        );
        // Zed reads WSL's folders through their network path.
        assert_eq!(
            editor_args("zed", std::slice::from_ref(&wsl)).unwrap(),
            None
        );
        assert!(editor_args("subl", &[ssh]).is_err());
        assert_eq!(editor_args("subl", &[wsl]).unwrap(), None);
        assert_eq!(editor_args("code", &[PathBuf::from("/a")]).unwrap(), None);
    }

    #[test]
    fn shells_there() {
        let ssh = Remote::of(Path::new("ssh://me@box:2222/srv/it's")).unwrap();
        assert_eq!(
            ssh.shell(Some("npm run dev")),
            (
                "ssh".to_string(),
                vec![
                    "-t".to_string(),
                    "-p".into(),
                    "2222".into(),
                    "me@box".into(),
                    r#"cd '/srv/it'\''s' && npm run dev; exec "$SHELL" -l"#.into()
                ]
            )
        );
        let wsl = Remote::of(Path::new(r"\\wsl$\Ubuntu\home\me")).unwrap();
        assert_eq!(
            wsl.shell(None).1,
            ["-d", "Ubuntu", "--cd", "/home/me"].map(String::from)
        );
        assert_eq!(wsl.label(), "WSL · Ubuntu");
        assert_eq!(ssh.label(), "SSH · box");
    }

    #[test]
    fn imported_remote_folders() {
        assert_eq!(
            from_zed(
                "ssh",
                Some("box"),
                Some("me"),
                Some(2222),
                None,
                "/home/me/app"
            ),
            Some(PathBuf::from("ssh://me@box:2222/home/me/app"))
        );
        assert_eq!(
            from_zed("wsl", None, None, None, Some("Ubuntu"), "/home/me/app"),
            Some(PathBuf::from(r"\\wsl.localhost\Ubuntu\home\me\app"))
        );
        assert_eq!(
            from_vs_code("vscode-remote://ssh-remote%2Bme%40box/home/me/app"),
            Some(PathBuf::from("ssh://me@box/home/me/app"))
        );
        assert_eq!(
            from_vs_code("vscode-remote://wsl%2BUbuntu/home/me/app"),
            Some(PathBuf::from(r"\\wsl.localhost\Ubuntu\home\me\app"))
        );
        let ported = format!(
            "vscode-remote://ssh-remote%2B{}/srv/x",
            hex(br#"{"hostName":"box","port":2222,"user":"me"}"#)
        );
        assert_eq!(
            from_vs_code(&ported),
            Some(PathBuf::from("ssh://me@box:2222/srv/x"))
        );
        assert_eq!(from_vs_code("vscode-remote://dev-container%2B61/x"), None);
    }

    #[test]
    fn dev_containers() {
        let dir = std::env::temp_dir()
            .join(format!("proj-devc-{}", std::process::id()))
            .join("app");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".devcontainer")).unwrap();
        assert_eq!(devcontainer(&dir), None);
        let config = dir.join(".devcontainer").join("devcontainer.json");
        std::fs::write(&config, "{ // comment\n \"image\": \"x\", }").unwrap();
        assert_eq!(devcontainer(&dir), Some(config.clone()));
        let uri = devcontainer_uri(&dir, &config);
        assert!(uri.starts_with("vscode-remote://dev-container+"));
        assert!(uri.ends_with("/workspaces/app"), "{uri}");
        let host = uri
            .strip_prefix("vscode-remote://dev-container+")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        assert_eq!(unhex(host).unwrap(), dir.to_string_lossy().as_bytes());
        std::fs::write(
            &config,
            r#"{ "workspaceFolder": "/src/${localWorkspaceFolderBasename}" }"#,
        )
        .unwrap();
        assert!(devcontainer_uri(&dir, &config).ends_with("/src/app"));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
