use cargo::core::SourceKind;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
use tower_lsp::{
    lsp_types::{InlayHint, Range, Url},
    Client,
};

use crate::entity::Dependency;

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
    fn list(&self, uri: &Url) -> Vec<InlayHint>;
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
    DependencyRemove(Url, String),
    DependencyLoading(Url, String, Range),
    Dependency(Url, String, Range, Dependency),
}

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
}

pub fn decoration_payload(dep: &Dependency) -> DecorationPayload {
    let installed = match dep.resolved.as_ref() {
        Some(resolved) => resolved.version.to_string(),
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
    }
}

pub fn formatted_string(dep: &Dependency, formatter: &DecorationFormatter) -> String {
    let version = version_decoration(dep);
    let payload = decoration_payload(dep);
    match version {
        VersionDecoration::Latest => formatter
            .latest
            .replace("{{installed}}", &payload.installed)
            .replace("{{latest_matched}}", &payload.latest_matched)
            .replace("{{latest}}", &payload.latest),
        VersionDecoration::Local => formatter
            .local
            .replace("{{installed}}", &payload.installed)
            .replace("{{latest_matched}}", &payload.latest_matched)
            .replace("{{latest}}", &payload.latest),
        VersionDecoration::NotInstalled => formatter
            .not_installed
            .replace("{{installed}}", &payload.installed)
            .replace("{{latest_matched}}", &payload.latest_matched)
            .replace("{{latest}}", &payload.latest),
        VersionDecoration::MixedUpgradeable => formatter
            .mixed_upgradeable
            .replace("{{installed}}", &payload.installed)
            .replace("{{latest_matched}}", &payload.latest_matched)
            .replace("{{latest}}", &payload.latest),
        VersionDecoration::CompatibleLatest => formatter
            .compatible_latest
            .replace("{{installed}}", &payload.installed)
            .replace("{{latest_matched}}", &payload.latest_matched)
            .replace("{{latest}}", &payload.latest),
        VersionDecoration::NoncompatibleLatest => formatter
            .noncompatible_latest
            .replace("{{installed}}", &payload.installed)
            .replace("{{latest_matched}}", &payload.latest_matched)
            .replace("{{latest}}", &payload.latest),
        VersionDecoration::Yanked => formatter
            .yanked
            .replace("{{installed}}", &payload.installed)
            .replace("{{latest_matched}}", &payload.latest_matched)
            .replace("{{latest}}", &payload.latest),
        _ => "".to_string(),
    }
}

pub fn version_decoration(dep: &Dependency) -> VersionDecoration {
    match dep.unresolved.as_ref().unwrap().source_id().kind() {
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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DecorationFormatter {
    #[serde(default = "default_latest")]
    pub latest: String,
    #[serde(default = "default_local")]
    pub local: String,
    #[serde(default = "default_not_installed")]
    pub not_installed: String,
    #[serde(default = "default_loading")]
    pub loading: String,
    #[serde(default = "default_mixed_upgradeable")]
    pub mixed_upgradeable: String,
    #[serde(default = "default_compatible_latest")]
    pub compatible_latest: String,
    #[serde(default = "default_noncompatible_latest")]
    pub noncompatible_latest: String,
    #[serde(default = "default_yanked")]
    pub yanked: String,
}

impl Default for DecorationFormatter {
    fn default() -> Self {
        Self {
            latest: default_latest(),
            compatible_latest: default_compatible_latest(),
            local: default_local(),
            noncompatible_latest: default_noncompatible_latest(),
            not_installed: default_not_installed(),
            loading: default_loading(),
            mixed_upgradeable: default_mixed_upgradeable(),
            yanked: default_yanked(),
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

fn default_loading() -> String {
    "Loading...".to_string()
}

fn default_local() -> String {
    "Local".to_string()
}

fn default_yanked() -> String {
    "❌ yanked {{installed}}, {{latest_matched}}".to_string()
}
