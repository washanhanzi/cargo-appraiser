use cargo::core::SourceKind;
use semver::Version;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
use tower_lsp::{
    lsp_types::{InlayHint, Range, Uri},
    Client,
};
mod vscode;

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
            Renderer::VSCode => {
                DecorationRenderer::VSCode(Box::new(vscode::VSCodeDecoration::new(client)))
            }
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
    Reset(Uri),
    DependencyRangeUpdate(Uri, String, Range),
    DependencyRemove(Uri, String),
    DependencyWaiting(Uri, String, Range),
    Dependency(Uri, String, Range, Dependency),
}

#[derive(Debug, Default, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VersionDecorationKind {
    #[default]
    NotParsed,
    //installed == latest_matched == latest
    Latest,
    Local,
    NotInstalled,
    //installed != latest_matched != latest
    MixedUpgradeable,
    //installed -> latest_matched == latest
    CompatibleLatest,
    //installed !-> latest_matched == latest
    NonCompatibleLatest,
    Yanked,
    Git,
}

#[derive(Debug, Default, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DecorationPayload {
    pub kind: VersionDecorationKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed: Option<Version>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_matched: Option<Version>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<Version>,
    //(ref,commit)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<(String, String)>,
}

pub fn formatted_string(
    dep: &Dependency,
    formatter: &CompiledFormatter,
) -> Option<(VersionDecorationKind, String)> {
    let version = version_decoration(dep);

    let template = match &version.kind {
        VersionDecorationKind::Git => &formatter.git,
        VersionDecorationKind::Latest => &formatter.latest,
        VersionDecorationKind::Local => &formatter.local,
        VersionDecorationKind::NotInstalled => &formatter.not_installed,
        VersionDecorationKind::MixedUpgradeable => &formatter.mixed_upgradeable,
        VersionDecorationKind::CompatibleLatest => &formatter.compatible_latest,
        VersionDecorationKind::NonCompatibleLatest => &formatter.noncompatible_latest,
        VersionDecorationKind::Yanked => &formatter.yanked,
        VersionDecorationKind::NotParsed => return None,
    };

    Some((version.kind.clone(), template.format(&version)))
}

pub fn version_decoration(dep: &Dependency) -> DecorationPayload {
    let Some(unresolved) = dep.unresolved.as_ref() else {
        return DecorationPayload {
            kind: VersionDecorationKind::NotParsed,
            ..Default::default()
        };
    };
    let Some(resolved) = dep.resolved.as_ref() else {
        return DecorationPayload {
            kind: VersionDecorationKind::NotInstalled,
            ..Default::default()
        };
    };
    match unresolved.source_id().kind() {
        SourceKind::Path => DecorationPayload {
            kind: VersionDecorationKind::Local,
            ..Default::default()
        },
        //TODO idk what's this
        SourceKind::Directory => DecorationPayload {
            kind: VersionDecorationKind::Local,
            ..Default::default()
        },
        SourceKind::Git(_) => {
            let mut git = None;
            if resolved.package_id().source_id().is_git() {
                git = Some((
                    git_ref_str(&resolved.package_id().source_id()).unwrap_or_default(),
                    commit_str_short(&resolved.package_id().source_id())
                        .map_or(String::new(), |c| c.to_string()),
                ));
            };
            DecorationPayload {
                kind: VersionDecorationKind::Git,
                git,
                ..Default::default()
            }
        }
        _ => {
            match (
                dep.matched_summary.as_ref(),
                dep.latest_matched_summary.as_ref(),
                dep.latest_summary.as_ref(),
            ) {
                (Some(matched), Some(latest_matched), Some(latest)) => {
                    //latest
                    let mut p = DecorationPayload::default();
                    if matched.version() == latest_matched.version()
                        && latest_matched.version() == latest.version()
                    {
                        p.kind = VersionDecorationKind::Latest;
                    } else if matched.version() != latest_matched.version()
                        && latest_matched.version() == latest.version()
                    {
                        p.kind = VersionDecorationKind::CompatibleLatest;
                    } else if matched.version() == latest_matched.version()
                        && latest_matched.version() != latest.version()
                    {
                        p.kind = VersionDecorationKind::NonCompatibleLatest;
                    } else {
                        p.kind = VersionDecorationKind::MixedUpgradeable;
                    }
                    p.installed = Some(matched.version().clone());
                    p.latest = Some(latest.version().clone());
                    p.latest_matched = Some(latest_matched.version().clone());
                    p
                }
                (None, Some(latest_matched), Some(latest)) => DecorationPayload {
                    kind: VersionDecorationKind::Yanked,
                    installed: Some(resolved.version().clone()),
                    latest_matched: Some(latest_matched.version().clone()),
                    latest: Some(latest.version().clone()),
                    ..Default::default()
                },
                //TODO any other match arm?
                _ => unreachable!(),
            }
        }
    }
}

/// decoration formatter
/// the formatter has 7 fields:
/// latest: the dependency has the latest version installed
/// local: the dependency is a local path dependency
/// not_installed: the dependency is not installed maybe because of platform mismatch
/// loading: the dependency is loading
/// mixed_upgradeable: the installed version has an compatible upgrade, but the latest version is not compatible with the current version requirement
/// compatible_latest: the installed version can update to latest version
/// noncompatible_latest: the installed version can't upate to latest version and there is no compatible upgrade
/// yanked: the installed version is yanked
/// git: the dependency is a git dependency, support {{ref}}, {{commit}} template strings
///
/// each field's value may has 3 template strings:
/// - installed: the installed version
/// - latest_matched: the latest compatible version
/// - latest: the latest version, the latest version may or may not be compatilbe with the version requirement
/// - git: if the dependency source is git
#[derive(Debug, Serialize, Deserialize, Clone)]
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

impl DecorationFormatter {
    pub fn compile(&self) -> CompiledFormatter {
        CompiledFormatter {
            waiting: CompiledTemplate::new(self.waiting.clone()),
            latest: CompiledTemplate::new(self.latest.clone()),
            local: CompiledTemplate::new(self.local.clone()),
            not_installed: CompiledTemplate::new(self.not_installed.clone()),
            mixed_upgradeable: CompiledTemplate::new(self.mixed_upgradeable.clone()),
            compatible_latest: CompiledTemplate::new(self.compatible_latest.clone()),
            noncompatible_latest: CompiledTemplate::new(self.noncompatible_latest.clone()),
            yanked: CompiledTemplate::new(self.yanked.clone()),
            git: CompiledTemplate::new(self.git.clone()),
        }
    }
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

#[derive(Debug, Clone, Default)]
pub struct CompiledFormatter {
    waiting: CompiledTemplate,
    latest: CompiledTemplate,
    local: CompiledTemplate,
    not_installed: CompiledTemplate,
    mixed_upgradeable: CompiledTemplate,
    compatible_latest: CompiledTemplate,
    noncompatible_latest: CompiledTemplate,
    yanked: CompiledTemplate,
    git: CompiledTemplate,
}

#[derive(Debug, Clone, Default)]
struct CompiledTemplate {
    template: String,
    needs_installed: bool,
    needs_latest_matched: bool,
    needs_latest: bool,
    needs_git_ref: bool,
    needs_git_commit: bool,
}

impl CompiledTemplate {
    fn new(template: String) -> Self {
        Self {
            needs_installed: template.contains("{{installed}}"),
            needs_latest_matched: template.contains("{{latest_matched}}"),
            needs_latest: template.contains("{{latest}}"),
            needs_git_ref: template.contains("{{ref}}"),
            needs_git_commit: template.contains("{{commit}}"),
            template,
        }
    }

    fn template(&self) -> &str {
        &self.template
    }

    fn format(&self, version: &DecorationPayload) -> String {
        let mut result = self.template.clone();

        if self.needs_installed && version.installed.is_some() {
            result = result.replace(
                "{{installed}}",
                &version.installed.as_ref().unwrap().to_string(),
            );
        }
        if self.needs_latest_matched && version.latest_matched.is_some() {
            result = result.replace(
                "{{latest_matched}}",
                &version.latest_matched.as_ref().unwrap().to_string(),
            );
        }
        if self.needs_latest && version.latest.is_some() {
            result = result.replace("{{latest}}", &version.latest.as_ref().unwrap().to_string());
        }
        if let Some((ref_str, commit)) = version.git.as_ref() {
            if self.needs_git_ref {
                result = result.replace("{{ref}}", ref_str);
            }
            if self.needs_git_commit {
                result = result.replace("{{commit}}", commit);
            }
        }

        result
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
