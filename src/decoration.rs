use cargo::core::SourceKind;
use semver::Version;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
use tower_lsp::{
    lsp_types::{InlayHint, Range, Uri},
    Client,
};
mod vscode;

use crate::entity::{
    commit_str_short, git_ref_str, DependencyTable, ResolvedDependency, TomlDependency,
};

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
    Update(Uri, Vec<DecorationItem>),
}

#[derive(Clone)]
pub struct DecorationItem {
    pub id: String,
    pub range: Range,
    pub state: DecorationState,
}

#[derive(Clone)]
pub enum DecorationState {
    Waiting,
    Resolved {
        dep: TomlDependency,
        resolved: Option<ResolvedDependency>,
    },
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tables: Option<Vec<DependencyTable>>,
}

pub fn formatted_string(
    dep: &TomlDependency,
    resolved: Option<&ResolvedDependency>,
    formatter: &CompiledFormatter,
) -> Option<(VersionDecorationKind, String)> {
    let version = version_decoration(dep, resolved);

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

pub fn version_decoration(
    dep: &TomlDependency,
    resolved: Option<&ResolvedDependency>,
) -> DecorationPayload {
    // Check for path dependency first
    if dep.is_path() {
        return DecorationPayload {
            kind: VersionDecorationKind::Local,
            ..Default::default()
        };
    }

    // Check for git dependency
    if dep.is_git() {
        let resolved = match resolved {
            Some(r) => r,
            None => {
                return DecorationPayload {
                    kind: VersionDecorationKind::Git,
                    ..Default::default()
                }
            }
        };

        let git = resolved.package.as_ref().and_then(|pkg| {
            let source_id = pkg.package_id().source_id();
            if source_id.is_git() {
                Some((
                    git_ref_str(&source_id).unwrap_or_default(),
                    commit_str_short(&source_id)
                        .map_or(String::new(), |c| c.to_string()),
                ))
            } else {
                None
            }
        });

        return DecorationPayload {
            kind: VersionDecorationKind::Git,
            git,
            ..Default::default()
        };
    }

    let Some(resolved) = resolved else {
        return DecorationPayload {
            kind: VersionDecorationKind::NotInstalled,
            ..Default::default()
        };
    };

    let Some(pkg) = resolved.package.as_ref() else {
        return DecorationPayload {
            kind: VersionDecorationKind::NotInstalled,
            ..Default::default()
        };
    };

    // Check source kind for local/path dependencies resolved through cargo
    match pkg.package_id().source_id().kind() {
        SourceKind::Path => {
            return DecorationPayload {
                kind: VersionDecorationKind::Local,
                ..Default::default()
            };
        }
        SourceKind::Directory => {
            return DecorationPayload {
                kind: VersionDecorationKind::Local,
                ..Default::default()
            };
        }
        SourceKind::Git(_) => {
            let git = Some((
                git_ref_str(&pkg.package_id().source_id()).unwrap_or_default(),
                commit_str_short(&pkg.package_id().source_id())
                    .map_or(String::new(), |c| c.to_string()),
            ));
            return DecorationPayload {
                kind: VersionDecorationKind::Git,
                git,
                ..Default::default()
            };
        }
        _ => {}
    }

    // Registry dependency - check versions
    let installed = pkg.version().clone();
    let latest_matched = resolved.latest_matched_summary.as_ref().map(|s| s.version().clone());
    let latest = resolved.latest_summary.as_ref().map(|s| s.version().clone());

    let mut p = DecorationPayload {
        installed: Some(installed.clone()),
        latest_matched: latest_matched.clone(),
        latest: latest.clone(),
        ..Default::default()
    };

    match (latest_matched.as_ref(), latest.as_ref()) {
        (Some(latest_matched_v), Some(latest_v)) => {
            // Check if installed version is yanked (not in available_versions)
            let installed_str = installed.to_string();
            let is_yanked = !resolved.available_versions.is_empty()
                && !resolved.available_versions.contains(&installed_str);

            if is_yanked {
                p.kind = VersionDecorationKind::Yanked;
            } else if &installed == latest_matched_v && latest_matched_v == latest_v {
                p.kind = VersionDecorationKind::Latest;
            } else if &installed != latest_matched_v && latest_matched_v == latest_v {
                p.kind = VersionDecorationKind::CompatibleLatest;
            } else if &installed == latest_matched_v && latest_matched_v != latest_v {
                p.kind = VersionDecorationKind::NonCompatibleLatest;
            } else {
                p.kind = VersionDecorationKind::MixedUpgradeable;
            }
        }
        _ => {
            // Can't determine version status without summaries
            p.kind = VersionDecorationKind::NotParsed;
        }
    }

    p
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
    pub waiting: CompiledTemplate,
    pub latest: CompiledTemplate,
    pub local: CompiledTemplate,
    pub not_installed: CompiledTemplate,
    pub mixed_upgradeable: CompiledTemplate,
    pub compatible_latest: CompiledTemplate,
    pub noncompatible_latest: CompiledTemplate,
    pub yanked: CompiledTemplate,
    pub git: CompiledTemplate,
}

#[derive(Debug, Clone, Default)]
pub struct CompiledTemplate {
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

    pub fn template(&self) -> &str {
        &self.template
    }

    pub fn format(&self, version: &DecorationPayload) -> String {
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

        if let Some(tables) = version.tables.as_ref() {
            let mut table_str = String::with_capacity(15);
            table_str.push_str(" [");
            for t in tables {
                match t {
                    DependencyTable::Dependencies => {}
                    DependencyTable::DevDependencies => {
                        if table_str.len() > 2 {
                            table_str.push_str(", dev");
                        } else {
                            table_str.push_str("dev");
                        }
                    }
                    DependencyTable::BuildDependencies => {
                        if table_str.len() > 2 {
                            table_str.push_str(", build");
                        } else {
                            table_str.push_str("build");
                        }
                    }
                }
            }
            table_str.push(']');
            if !table_str.is_empty() {
                result.push_str(&table_str);
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
