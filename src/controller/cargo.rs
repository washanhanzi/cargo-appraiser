use std::collections::{HashMap, HashSet};
use std::task::Poll;

use cargo::core::Summary;
use cargo::sources::source::{QueryKind, Source};
use cargo::util::cache_lock::CacheLockMode;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity};
use tracing::error;

use crate::entity::{
    CanonicalUri, CargoError, CargoErrorKind, CargoIndex, DependencyLookupKey, TomlDependency,
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
    let index =
        CargoIndex::resolve(&path).map_err(|e| crate::entity::from_resolve_error(e.into()))?;

    // Convert paths to URIs
    let root_manifest_uri =
        CanonicalUri::try_from_path(index.root_manifest()).map_err(CargoError::resolve_error)?;

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
        self.diagnostic_with_suggestion(keys, deps, tree, None)
    }

    /// Generate diagnostics for cargo errors with an optional crate name suggestion
    pub fn diagnostic_with_suggestion(
        &self,
        keys: &[&TomlNode],
        deps: &[&TomlDependency],
        tree: &TomlTree,
        crate_suggestion: Option<String>,
    ) -> Option<Vec<(String, Diagnostic)>> {
        match &self.kind {
            CargoErrorKind::NoMatchingPackage(_) => {
                let base_message = self.to_string();

                Some(
                    keys.iter()
                        .map(|key| {
                            let message = if let Some(ref suggestion) = crate_suggestion {
                                format!("{}, did you mean `{}`?", base_message, suggestion)
                            } else {
                                base_message.clone()
                            };
                            (
                                key.id.to_string(),
                                Diagnostic {
                                    range: key.range,
                                    severity: Some(DiagnosticSeverity::ERROR),
                                    code: None,
                                    code_description: None,
                                    source: Some("cargo".to_string()),
                                    message,
                                    related_information: None,
                                    tags: None,
                                    data: None,
                                },
                            )
                        })
                        .collect(),
                )
            }
            CargoErrorKind::VersionNotFound(crate_name, _) => Some(
                deps.iter()
                    .filter_map(|d| {
                        let version_field = d.version()?;
                        let base_message = self.to_string();

                        // Check if the requirement in the error message matches
                        if base_message.contains(&format!("`{} = \"", d.name)) {
                            let version_node = tree.get_node(&version_field.node_id)?;

                            // Try to get the latest available version
                            let message = if let Some(latest) = get_latest_version(crate_name) {
                                format!("{}, latest is `{}`", base_message, latest)
                            } else {
                                base_message
                            };

                            Some((
                                version_field.node_id.clone(),
                                Diagnostic {
                                    range: version_node.range,
                                    severity: Some(DiagnosticSeverity::ERROR),
                                    code: None,
                                    code_description: None,
                                    source: Some("cargo".to_string()),
                                    message,
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

                    // Collect all valid features from summaries
                    let mut all_valid_features: Vec<String> = Vec::new();
                    for summary in &summaries {
                        if !feature_map.is_empty() {
                            for f in summary.features().keys() {
                                let feature_name = f.to_string();
                                feature_map.remove(&feature_name);
                                all_valid_features.push(feature_name);
                            }
                        }
                    }

                    for (unknown_feature, node_id) in feature_map {
                        let node = tree.get_node(&node_id)?;

                        // Try to find a close match for the typo
                        let message = if let Some(suggestion) =
                            find_closest_feature(&unknown_feature, &all_valid_features)
                        {
                            format!(
                                "unknown feature `{}`, did you mean `{}`?",
                                unknown_feature, suggestion
                            )
                        } else {
                            format!("unknown feature `{}`", unknown_feature)
                        };

                        diags.push((
                            node_id.clone(),
                            Diagnostic {
                                range: node.range,
                                severity: Some(DiagnosticSeverity::ERROR),
                                code: None,
                                code_description: None,
                                source: Some("cargo".to_string()),
                                message,
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

/// Get the latest stable version of a crate from crates.io.
fn get_latest_version(crate_name: &str) -> Option<String> {
    // Query without version constraint to get all versions
    let summaries = resolve_package_with_default_source(crate_name, None)?;

    // Find the latest stable version (non-prerelease)
    summaries
        .iter()
        .filter(|s| s.version().pre.is_empty())
        .max_by(|a, b| a.version().cmp(b.version()))
        .map(|s| s.version().to_string())
}

/// Search for crates with similar names using the crates.io API.
/// Returns the closest matching crate name using Levenshtein distance.
pub async fn search_similar_crates(
    http_client: &reqwest::Client,
    invalid_name: &str,
) -> Option<String> {
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct SearchCrateOutput {
        name: String,
    }

    #[derive(Deserialize)]
    struct SearchCrateResponse {
        crates: Vec<SearchCrateOutput>,
    }

    // Search crates.io for similar names
    let url = format!(
        "https://crates.io/api/v1/crates?page=1&per_page=20&q={}",
        invalid_name
    );

    let resp = http_client.get(&url).send().await.ok()?;
    let search_response: SearchCrateResponse = resp.json().await.ok()?;

    if search_response.crates.is_empty() {
        return None;
    }

    // Find the closest match using Levenshtein distance
    let crate_names: Vec<String> = search_response.crates.into_iter().map(|c| c.name).collect();

    // Return the best match if it's close enough (distance <= 3)
    crate_names
        .iter()
        .map(|name| (name, levenshtein_distance(invalid_name, name)))
        .filter(|(_, dist)| *dist <= 3 && *dist > 0) // Must be similar but not exact
        .min_by_key(|(_, dist)| *dist)
        .map(|(name, _)| name.clone())
}

/// Find the closest matching feature using Levenshtein distance.
/// Returns None if no feature is within the threshold distance (3).
fn find_closest_feature(unknown: &str, valid_features: &[String]) -> Option<String> {
    if valid_features.is_empty() {
        return None;
    }

    valid_features
        .iter()
        .map(|f| (f, levenshtein_distance(unknown, f)))
        .filter(|(_, dist)| *dist <= 3) // Only suggest if edit distance <= 3
        .min_by_key(|(_, dist)| *dist)
        .map(|(f, _)| f.clone())
}

/// Calculate the Levenshtein (edit) distance between two strings.
/// This measures the minimum number of single-character edits (insertions,
/// deletions, substitutions) required to change one string into the other.
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    // Early exit for empty strings
    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    // Use two rows for space optimization
    let mut prev_row: Vec<usize> = (0..=b_len).collect();
    let mut curr_row: Vec<usize> = vec![0; b_len + 1];

    for (i, a_char) in a_chars.iter().enumerate() {
        curr_row[0] = i + 1;

        for (j, b_char) in b_chars.iter().enumerate() {
            let cost = if a_char == b_char { 0 } else { 1 };
            curr_row[j + 1] = std::cmp::min(
                std::cmp::min(
                    prev_row[j + 1] + 1, // deletion
                    curr_row[j] + 1,     // insertion
                ),
                prev_row[j] + cost, // substitution
            );
        }

        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[b_len]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(levenshtein_distance("", ""), 0);
        assert_eq!(levenshtein_distance("abc", ""), 3);
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", "abc"), 0);
        assert_eq!(levenshtein_distance("derive", "de1rive"), 1);
        assert_eq!(levenshtein_distance("derive", "deriv"), 1);
        assert_eq!(levenshtein_distance("derive", "deriver"), 1);
        assert_eq!(levenshtein_distance("serde", "serd"), 1);
        assert_eq!(levenshtein_distance("abc", "xyz"), 3);
    }

    #[test]
    fn test_find_closest_feature() {
        let features = vec![
            "derive".to_string(),
            "serde".to_string(),
            "default".to_string(),
        ];

        // Close typos should return suggestions
        assert_eq!(
            find_closest_feature("de1rive", &features),
            Some("derive".to_string())
        );
        assert_eq!(
            find_closest_feature("deriv", &features),
            Some("derive".to_string())
        );
        assert_eq!(
            find_closest_feature("serd", &features),
            Some("serde".to_string())
        );

        // Completely different string should return None
        assert_eq!(find_closest_feature("xyz123", &features), None);

        // Empty features list should return None
        assert_eq!(find_closest_feature("derive", &[]), None);
    }
}
