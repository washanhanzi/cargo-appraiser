use std::collections::{HashMap, HashSet};
use std::task::Poll;

use cargo::core::Summary;
use cargo::sources::source::{QueryKind, Source};
use cargo::util::cache_lock::CacheLockMode;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity};
use tracing::error;

use crate::entity::{
    CargoError, CargoErrorKind, CargoIndex, CanonicalUri, DependencyLookupKey, TomlDependency,
    TomlNode, TomlTree, WorkspaceMember,
};

use super::appraiser::Ctx;

/// Output from cargo resolution, wrapping CargoIndex with context
#[derive(Debug)]
pub struct CargoResolveOutput {
    pub ctx: Ctx,
    pub root_manifest_uri: CanonicalUri,
    pub member_manifest_uris: Vec<CanonicalUri>,
    pub members: Vec<WorkspaceMember>,
    pub index: CargoIndex,
}

/// Resolve cargo dependencies using cargo-parser
#[tracing::instrument(name = "cargo_resolve", level = "trace")]
pub async fn cargo_resolve(ctx: &Ctx) -> Result<CargoResolveOutput, CargoError> {
    let Ok(path) = ctx.uri.to_path_buf() else {
        return Err(CargoError::resolve_error(anyhow::anyhow!(
            "Failed to convert URI to path"
        )));
    };

    // Use cargo-parser to resolve dependencies
    let index = CargoIndex::resolve(&path).map_err(|e| CargoError::resolve_error(e.into()))?;

    // Convert paths to URIs
    let root_manifest_uri = CanonicalUri::try_from_path(index.root_manifest())
        .map_err(|e| CargoError::resolve_error(e))?;

    let member_manifest_uris: Vec<CanonicalUri> = index
        .member_manifests()
        .iter()
        .filter_map(|p| CanonicalUri::try_from_path(p).ok())
        .collect();

    let members = index.members().to_vec();

    Ok(CargoResolveOutput {
        ctx: ctx.clone(),
        root_manifest_uri,
        member_manifest_uris,
        members,
        index,
    })
}

/// Resolve a package from the default source (crates.io) for completion features
pub fn resolve_package_with_default_source(
    package: &str,
    version: Option<&str>,
) -> Option<Vec<Summary>> {
    let gctx = cargo::util::context::GlobalContext::default().ok()?;
    let source_id = cargo::core::SourceId::crates_io(&gctx).unwrap();
    let dep = cargo::core::Dependency::parse(package, version, source_id).ok()?;
    let mut source = source_id.load(&gctx, &HashSet::new()).unwrap();
    let Ok(_guard) = gctx.acquire_package_cache_lock(CacheLockMode::DownloadExclusive) else {
        error!("failed to acquire package cache lock");
        return None;
    };
    let summary = source.query_vec(&dep, QueryKind::Normalized);
    source.block_until_ready().unwrap();
    match summary {
        Poll::Ready(summaries) => {
            let summaries = summaries.unwrap();
            Some(summaries.iter().map(|s| s.as_summary().clone()).collect())
        }
        Poll::Pending => None,
    }
}

impl CargoError {
    /// Generate diagnostics for cargo errors
    pub fn diagnostic(
        &self,
        keys: &[&TomlNode],
        deps: &[&TomlDependency],
        tree: &TomlTree,
    ) -> Option<Vec<(String, Diagnostic)>> {
        match &self.kind {
            CargoErrorKind::NoMatchingPackage(_) => Some(
                keys.iter()
                    .map(|key| {
                        (
                            key.id.to_string(),
                            Diagnostic {
                                range: key.range,
                                severity: Some(DiagnosticSeverity::ERROR),
                                code: None,
                                code_description: None,
                                source: Some("cargo".to_string()),
                                message: self.to_string(),
                                related_information: None,
                                tags: None,
                                data: None,
                            },
                        )
                    })
                    .collect(),
            ),
            CargoErrorKind::VersionNotFound(_, _) => Some(
                deps.iter()
                    .filter_map(|d| {
                        let version_field = d.version()?;
                        let error_msg = self.to_string();

                        // Check if the requirement in the error message matches
                        if error_msg.contains(&format!("`{} = \"", d.name)) {
                            let version_node = tree.get_node(&version_field.node_id)?;
                            Some((
                                version_field.node_id.clone(),
                                Diagnostic {
                                    range: version_node.range,
                                    severity: Some(DiagnosticSeverity::ERROR),
                                    code: None,
                                    code_description: None,
                                    source: Some("cargo".to_string()),
                                    message: error_msg,
                                    related_information: None,
                                    tags: None,
                                    data: None,
                                },
                            ))
                        } else {
                            None
                        }
                    })
                    .collect(),
            ),
            CargoErrorKind::FailedToSelectVersion(_) => {
                let mut diags = Vec::with_capacity(deps.len());
                for d in deps {
                    // Check for invalid features
                    if d.features.is_empty() {
                        continue;
                    }

                    let version_text = d.version().map(|v| v.text.as_str()).unwrap_or("*");
                    let summaries =
                        resolve_package_with_default_source(d.package_name(), Some(version_text))?;

                    let mut feature_map: HashMap<String, String> = d
                        .features
                        .iter()
                        .map(|f| (f.name.clone(), f.node_id.clone()))
                        .collect();

                    for summary in &summaries {
                        if !feature_map.is_empty() {
                            for f in summary.features().keys() {
                                feature_map.remove(f.to_string().as_str());
                            }
                        }
                    }

                    for (k, v) in feature_map {
                        let node = tree.get_node(&v)?;
                        diags.push((
                            v.clone(),
                            Diagnostic {
                                range: node.range,
                                severity: Some(DiagnosticSeverity::ERROR),
                                code: None,
                                code_description: None,
                                source: Some("cargo".to_string()),
                                message: format!("unknown feature `{}`", k),
                                related_information: None,
                                tags: None,
                                data: None,
                            },
                        ));
                    }
                }
                if !diags.is_empty() {
                    return Some(diags);
                }
                None
            }
            _ => None,
        }
    }
}

/// Create a lookup key from a TomlDependency
pub fn make_lookup_key(dep: &TomlDependency) -> DependencyLookupKey {
    DependencyLookupKey::new(
        dep.table,
        dep.platform.clone(),
        dep.package_name().to_string(),
    )
}
