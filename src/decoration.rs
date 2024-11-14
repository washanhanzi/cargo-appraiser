use cargo::core::SourceKind;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
use tower_lsp::{
    lsp_types::{InlayHint, Range, Uri},
    Client,
};
use tracing::info;

use crate::entity::{commit_str_short, git_ref_str, Dependency};

pub mod inlay_hint;

#[derive(clap::ValueEnum, Debug, Clone)]
pub enum Renderer {
    #[value(name = "inlayHint")]
    InlayHint,
    #[value(name = "vscode")]
    VSCode,
}

pub trait VSCodeDecorationRenderer: Send + Sync + std::fmt::Debug {
    fn init(&self) -> Sender<DecorationEvent>;
}

pub trait InlayHintDecorationRenderer: Send + Sync + std::fmt::Debug {
    fn init(&self) -> Sender<DecorationEvent>;
    //only work for inlayHint renderer
    fn list(&self, uri: &Uri) -> Vec<InlayHint>;
}

#[derive(Debug)]
pub enum DecorationRenderer {
    VSCode(Box<dyn VSCodeDecorationRenderer>),
    InlayHint(Box<dyn InlayHintDecorationRenderer>),
}

impl DecorationRenderer {
    pub fn new(client: Client, renderer: Renderer) -> Self {
        match renderer {
            Renderer::InlayHint => DecorationRenderer::InlayHint(Box::new(
                inlay_hint::InlayHintDecoration::new(client),
            )),
            Renderer::VSCode => DecorationRenderer::InlayHint(Box::new(
                inlay_hint::InlayHintDecoration::new(client),
            )),
        }
    }
    pub fn init(&self) -> Sender<DecorationEvent> {
        match self {
            DecorationRenderer::VSCode(renderer) => renderer.init(),
            DecorationRenderer::InlayHint(renderer) => renderer.init(),
        }
    }
}

#[derive(Clone)]
pub enum DecorationEvent {
    Reset,
    DependencyRangeUpdate(Uri, String, Range),
    DependencyRemove(Uri, String),
    DependencyWaiting(Uri, String, Range),
    Dependency(Uri, String, Range, Dependency),
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum VersionDecoration {
    //installed == latest_matched == latest
    Latest,
    Local,
    NotInstalled,
    //installed != latest_matched != latest
    MixedUpgradeable,
    //installed -> latest_matched == latest
    CompatibleLatest,
    //installed !-> latest_matched == latest
    NoncompatibleLatest,
    Yanked,
    NotParsed,
}

#[derive(Debug, Default, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DecorationPayload {
    installed: String,
    latest_matched: String,
    latest: String,
    //(ref,commit)
    git: Option<(String, String)>,
}

pub fn decoration_payload(dep: &Dependency) -> DecorationPayload {
    let mut git = None;
    let installed = match dep.resolved.as_ref() {
        Some(resolved) => {
            if resolved.package_id().source_id().is_git() {
                git = Some((
                    git_ref_str(&resolved.package_id().source_id()).unwrap_or_default(),
                    commit_str_short(&resolved.package_id().source_id())
                        .map_or(String::new(), |c| c.to_string()),
                ));
                String::new()
            } else {
                resolved.version().to_string()
            }
        }
        None => "".to_string(),
    };
    let latest_matched = match dep.latest_matched_summary.as_ref() {
        Some(matched) => matched.version().to_string(),
        None => "".to_string(),
    };
    let latest = match dep.latest_summary.as_ref() {
        Some(latest) => latest.version().to_string(),
        None => "".to_string(),
    };
    DecorationPayload {
        installed,
        latest_matched,
        latest,
        git,
    }
}

pub fn formatted_string(dep: &Dependency, formatter: &DecorationFormatter) -> Option<String> {
    let version = version_decoration(dep);
    let payload = decoration_payload(dep);
    if let Some((r, commit)) = payload.git {
        return Some(
            formatter
                .git
                .replace("{{ref}}", &r)
                .replace("{{commit}}", &commit),
        );
    }
    match version {
        VersionDecoration::Latest => Some(
            formatter
                .latest
                .replace("{{installed}}", &payload.installed)
                .replace("{{latest_matched}}", &payload.latest_matched)
                .replace("{{latest}}", &payload.latest),
        ),
        VersionDecoration::Local => Some(
            formatter
                .local
                .replace("{{installed}}", &payload.installed)
                .replace("{{latest_matched}}", &payload.latest_matched)
                .replace("{{latest}}", &payload.latest),
        ),
        VersionDecoration::NotInstalled => Some(
            formatter
                .not_installed
                .replace("{{installed}}", &payload.installed)
                .replace("{{latest_matched}}", &payload.latest_matched)
                .replace("{{latest}}", &payload.latest),
        ),
        VersionDecoration::MixedUpgradeable => Some(
            formatter
                .mixed_upgradeable
                .replace("{{installed}}", &payload.installed)
                .replace("{{latest_matched}}", &payload.latest_matched)
                .replace("{{latest}}", &payload.latest),
        ),
        VersionDecoration::CompatibleLatest => Some(
            formatter
                .compatible_latest
                .replace("{{installed}}", &payload.installed)
                .replace("{{latest_matched}}", &payload.latest_matched)
                .replace("{{latest}}", &payload.latest),
        ),
        VersionDecoration::NoncompatibleLatest => Some(
            formatter
                .noncompatible_latest
                .replace("{{installed}}", &payload.installed)
                .replace("{{latest_matched}}", &payload.latest_matched)
                .replace("{{latest}}", &payload.latest),
        ),
        VersionDecoration::Yanked => Some(
            formatter
                .yanked
                .replace("{{installed}}", &payload.installed)
                .replace("{{latest_matched}}", &payload.latest_matched)
                .replace("{{latest}}", &payload.latest),
        ),
        _ => None,
    }
}

pub fn version_decoration(dep: &Dependency) -> VersionDecoration {
    let Some(unresolved) = dep.unresolved.as_ref() else {
        return VersionDecoration::NotParsed;
    };
    match unresolved.source_id().kind() {
        SourceKind::Path => VersionDecoration::Local,
        //TODO idk what's this
        SourceKind::Directory => VersionDecoration::Local,
        _ => {
            match (
                dep.resolved.as_ref(),
                dep.matched_summary.as_ref(),
                dep.latest_matched_summary.as_ref(),
                dep.latest_summary.as_ref(),
            ) {
                (Some(_), Some(matched), Some(latest_matched), Some(latest)) => {
                    //latest
                    if matched.version() == latest_matched.version()
                        && latest_matched.version() == latest.version()
                    {
                        VersionDecoration::Latest
                    } else if matched.version() != latest_matched.version()
                        && latest_matched.version() == latest.version()
                    {
                        VersionDecoration::CompatibleLatest
                    } else if matched.version() == latest_matched.version()
                        && latest_matched.version() != latest.version()
                    {
                        VersionDecoration::NoncompatibleLatest
                    } else {
                        VersionDecoration::MixedUpgradeable
                    }
                }
                (Some(_), None, Some(_), Some(_)) => VersionDecoration::Yanked,
                (None, _, _, _) => VersionDecoration::NotInstalled,
                //TODO get latest version for not installed
                //TODO any other match arm?
                _ => unreachable!(),
            }
        }
    }
}

/// decoration formatter
/// the formatter may has 3 template strings:
/// - installed: the installed version
/// - latest_matched: the latest compatible version
/// - latest: the latest version, the latest version may or may not be compatilbe with the version requirement
/// - git: if the dependency source is git
///
/// the formatter has 7 fields:
/// latest: the dependency has the latest version installed
/// local: the dependency is a local path dependency
/// not_installed: the dependency is not installed
/// loading: the dependency is loading
/// mixed_upgradeable: the installed version has an compatible upgrade, and the latest version is not compatible with the current version requirement
/// compatible_latest: the installed version can update to latest version
/// noncompatible_latest: the installed version can't upate to latest version
/// yanked: the installed version is yanked
/// git: support {{ref}}, {{commit}}
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DecorationFormatter {
    #[serde(default = "default_latest")]
    pub latest: String,
    #[serde(default = "default_local")]
    pub local: String,
    #[serde(default = "default_not_installed")]
    pub not_installed: String,
    #[serde(default = "default_waiting")]
    pub waiting: String,
    #[serde(default = "default_mixed_upgradeable")]
    pub mixed_upgradeable: String,
    #[serde(default = "default_compatible_latest")]
    pub compatible_latest: String,
    #[serde(default = "default_noncompatible_latest")]
    pub noncompatible_latest: String,
    #[serde(default = "default_yanked")]
    pub yanked: String,
    #[serde(default = "default_git")]
    pub git: String,
}

impl Default for DecorationFormatter {
    fn default() -> Self {
        Self {
            latest: default_latest(),
            compatible_latest: default_compatible_latest(),
            local: default_local(),
            noncompatible_latest: default_noncompatible_latest(),
            not_installed: default_not_installed(),
            waiting: default_waiting(),
            mixed_upgradeable: default_mixed_upgradeable(),
            yanked: default_yanked(),
            git: default_git(),
        }
    }
}

fn default_latest() -> String {
    "✅ {{installed}}".to_string()
}

fn default_mixed_upgradeable() -> String {
    "🚀🔒 {{installed}} -> {{latest_matched}},  {{latest}}".to_string()
}

fn default_compatible_latest() -> String {
    "🚀 {{installed}} -> {{latest}}".to_string()
}

fn default_noncompatible_latest() -> String {
    "🔒 {{installed}}, {{latest}}".to_string()
}

fn default_not_installed() -> String {
    "Not installed".to_string()
}

fn default_waiting() -> String {
    "Waiting...".to_string()
}

fn default_local() -> String {
    "Local".to_string()
}

fn default_yanked() -> String {
    "❌ yanked {{installed}}, {{latest_matched}}".to_string()
}

fn default_git() -> String {
    "🐙 {{commit}}".to_string()
}
