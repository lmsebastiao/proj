//! Updates from GitHub Releases: finds a newer release and runs its installer.
//!
//! Only copies installed by the Windows installer update themselves; the
//! installer replaces proj.exe in place and starts the new version.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use serde::Deserialize;

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// A release newer than this build, with an installer to download.
#[derive(Clone, Debug)]
pub struct Update {
    pub version: String,
    installer_url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// The releases page, for copies that can't update themselves.
pub fn releases_url() -> String {
    format!("{}/releases", env!("CARGO_PKG_REPOSITORY"))
}

/// Whether this copy came from the installer (and so can be replaced by the next
/// one). A `cargo build` doesn't update itself.
pub fn is_installed() -> bool {
    cfg!(windows) && install_dir().is_some_and(|dir| dir.join("uninstall.exe").is_file())
}

fn install_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

/// Asks GitHub for the latest release. `Ok(None)` means this is the newest.
pub fn check() -> Result<Option<Update>, String> {
    // The installer that updated us to this version is no longer needed.
    fs::remove_file(installer_path(CURRENT)).ok();

    let repo = env!("CARGO_PKG_REPOSITORY").trim_start_matches("https://github.com/");
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let release: Release = match agent(Duration::from_secs(30))
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .and_then(|mut response| response.body_mut().read_json())
    {
        Ok(release) => release,
        // Nothing released yet.
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(err) => return Err(format!("could not check for updates: {err}")),
    };
    Ok(newer(&release, CURRENT))
}

/// `release` if it is newer than `current` and its installer has been uploaded.
fn newer(release: &Release, current: &str) -> Option<Update> {
    let version = release.tag_name.trim_start_matches('v');
    if parse_version(version)? <= parse_version(current)? {
        return None;
    }
    let name = format!("proj-setup-{version}.exe");
    let asset = release.assets.iter().find(|asset| asset.name == name)?;
    Some(Update {
        version: version.into(),
        installer_url: asset.browser_download_url.clone(),
    })
}

/// "1.2.3" → `[1, 2, 3]`. Pre-releases ("1.2.3-beta") aren't offered.
fn parse_version(text: &str) -> Option<[u64; 3]> {
    let mut parts = text.split('.').map(|part| part.parse().ok());
    let version = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some(version)
}

/// Downloads the installer and starts it silently. It stops every running proj,
/// this one included, replaces proj.exe and starts the new version, so the caller
/// should exit right away.
pub fn install(update: &Update) -> Result<(), String> {
    let dir = install_dir().filter(|_| is_installed()).ok_or_else(|| {
        format!(
            "this copy wasn't installed by the installer; get {}",
            releases_url()
        )
    })?;
    let installer = installer_path(&update.version);
    download(&update.installer_url, &installer)
        .map_err(|err| format!("could not download the update: {err}"))?;
    run_installer(&installer, &dir).map_err(|err| format!("could not start the installer: {err}"))
}

fn installer_path(version: &str) -> PathBuf {
    std::env::temp_dir().join(format!("proj-setup-{version}.exe"))
}

/// Saves `url` to `path`, via a temporary file so a partial download never runs.
fn download(url: &str, path: &Path) -> Result<(), ureq::Error> {
    let response = agent(Duration::from_secs(600)).get(url).call()?;
    let partial = path.with_extension("part");
    let mut file = fs::File::create(&partial)?;
    io::copy(&mut response.into_body().into_reader(), &mut file)?;
    drop(file);
    fs::rename(&partial, path)?;
    Ok(())
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(format!("proj/{CURRENT}"))
        .timeout_global(Some(timeout))
        .build()
        .into()
}

#[cfg(windows)]
fn run_installer(installer: &Path, dir: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    // NSIS wants /D last and unquoted, even with spaces, so it can't go through `arg`.
    // /RELAUNCH makes the installer start the new version when it's done.
    Command::new(installer)
        .raw_arg(format!("/S /RELAUNCH /D={}", dir.display()))
        .spawn()
        .map(drop)
}

#[cfg(not(windows))]
fn run_installer(_installer: &Path, _dir: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, assets: &[&str]) -> Release {
        Release {
            tag_name: tag.into(),
            assets: assets
                .iter()
                .map(|name| Asset {
                    name: (*name).into(),
                    browser_download_url: format!("https://example.com/{name}"),
                })
                .collect(),
        }
    }

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("0.12.3"), Some([0, 12, 3]));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("1.2.3-beta"), None);
    }

    #[test]
    fn offers_only_newer_releases_with_an_installer() {
        let update = newer(&release("v0.2.0", &["proj-setup-0.2.0.exe"]), "0.1.9").unwrap();
        assert_eq!(update.version, "0.2.0");
        assert_eq!(
            update.installer_url,
            "https://example.com/proj-setup-0.2.0.exe"
        );

        // Compared as numbers, not text.
        assert!(newer(&release("v0.10.0", &["proj-setup-0.10.0.exe"]), "0.9.0").is_some());
        assert!(newer(&release("v0.2.0", &["proj-setup-0.2.0.exe"]), "0.2.0").is_none());
        assert!(newer(&release("v0.1.0", &["proj-setup-0.1.0.exe"]), "0.2.0").is_none());
        // Published, but the installer isn't uploaded yet.
        assert!(newer(&release("v0.3.0", &[]), "0.2.0").is_none());
    }
}
