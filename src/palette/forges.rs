//! What a self-hosted git site runs (GitLab, Gitea…): asked the first time its
//! pull requests or CI runs are opened, then kept under `forges` in config.toml.

use std::collections::BTreeMap;

use gpui::{Context, Window};

use crate::{config, git::Forge, open};

use super::{
    Palette,
    items::{Mode, ProjectAction},
};

/// What a site can run, as the list shows it and as `forges` names it.
/// The ones usually self-hosted first.
pub(super) const CHOICES: [(&str, &str, &str); 6] = [
    ("GitLab", "gitlab", "Merge requests and pipelines"),
    ("Gitea", "gitea", "Pull requests and Actions"),
    (
        "Forgejo",
        "forgejo",
        "Pull requests and Actions, as on Codeberg",
    ),
    (
        "GitHub",
        "github",
        "GitHub Enterprise Server: pull requests and Actions",
    ),
    // proj's Bitbucket links are bitbucket.org's, not Data Center's.
    (
        "Bitbucket",
        "bitbucket",
        "Pull requests and Pipelines, as on bitbucket.org",
    ),
    (
        "Azure DevOps",
        "azure",
        "Azure DevOps Server: pull requests and pipelines",
    ),
];

/// The site being asked about, and what to open once it's known.
pub(super) struct ForgePick {
    /// The project's repository page, e.g. https://git.example.com/me/app.
    pub(super) web: String,
    pub(super) host: String,
    /// The project's key, to highlight it again when going back.
    pub(super) key: String,
    /// `PullRequests` or `Ci`, to open once it's known; `None` when changing
    /// it from the actions menu.
    pub(super) then: Option<ProjectAction>,
}

impl ForgePick {
    /// What it opens, for the page's explanation.
    pub(super) fn opens(&self) -> Option<&'static str> {
        match self.then? {
            ProjectAction::Ci => Some("CI runs"),
            _ => Some("pull requests"),
        }
    }
}

/// What `host` is known to run, for the actions menu: the choice's label,
/// and whether that's from the config file (else it's from the site's name).
pub(super) fn known(
    host: &str,
    web: &str,
    forges: &BTreeMap<String, String>,
) -> Option<(String, bool)> {
    let set = forges
        .iter()
        .find(|(h, _)| h.eq_ignore_ascii_case(host))
        .map(|(_, kind)| kind.as_str());
    if let Some(&(label, _, _)) = set.and_then(|kind| {
        CHOICES
            .iter()
            .find(|(_, name, _)| name.eq_ignore_ascii_case(kind.trim()))
    }) {
        return Some((label.to_string(), true));
    }
    let forge = Forge::for_url(web, forges)?;
    Some((forge.name().to_string(), false))
}

impl Palette {
    /// Asks what `pick.host` runs, instead of saying to edit the config.
    pub(super) fn ask_forge(&mut self, pick: ForgePick, cx: &mut Context<Self>) {
        // Start on what it's known to run, if anything, so a stray enter
        // changes nothing.
        let set = self
            .config
            .forges
            .iter()
            .find(|(h, _)| h.eq_ignore_ascii_case(&pick.host))
            .map(|(_, kind)| kind.trim().to_lowercase());
        let detected = Forge::for_url(&pick.web, &self.config.forges);
        self.forge_pick = Some(pick);
        self.set_mode(Mode::Forges, cx);
        self.select_where(|_, ix| {
            let (_, name, _) = CHOICES[ix];
            match &set {
                Some(kind) => name == kind,
                None => detected.is_some() && Forge::from_name(name) == detected,
            }
        });
    }

    /// Enter on a choice: saves it for the site and opens the page asked for.
    pub(super) fn choose_forge(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(pick), Some(&(label, name, _))) = (self.forge_pick.as_ref(), CHOICES.get(ix))
        else {
            return;
        };
        let Some(forge) = Forge::from_name(name) else {
            return;
        };
        let host = pick.host.clone();
        let url = match pick.then {
            Some(ProjectAction::Ci) => Some(forge.ci(&pick.web)),
            Some(_) => Some(forge.pull_requests(&pick.web)),
            None => None,
        };
        if let Err(err) = config::set_forge(&host, name) {
            return self.problem(format!("Could not save config: {err}"), cx);
        }
        self.config.forges.insert(host.clone(), name.to_string());
        // Changed from the actions menu: nothing to open.
        let Some(url) = url else {
            self.back_to_projects(cx);
            return self.notice(format!("{host} runs {label}"), cx);
        };
        match open::open_url(&url) {
            Ok(()) => window.remove_window(),
            Err(err) => {
                self.back_to_projects(cx);
                self.problem(
                    format!("{host} is now known as {label}, but {url} didn't open: {err}"),
                    cx,
                );
            }
        }
    }
}
