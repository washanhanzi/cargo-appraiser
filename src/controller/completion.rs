use serde::Deserialize;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit, Position, Range,
    TextEdit,
};
use tracing::{debug, error};

use crate::entity::{
    DependencyValue, NodeKind, ResolvedDependency, TomlDependency, TomlNode, ValueKind,
};

pub async fn completion(
    http_client: &reqwest::Client,
    node: &TomlNode,
    dep: Option<&TomlDependency>,
    resolved: Option<&ResolvedDependency>,
) -> Option<CompletionResponse> {
    debug!(
        "completion() called - node.kind: {:?}, node.range: {:?}, dep: {:?}, resolved: {:?}",
        node.kind,
        node.range,
        dep.map(|d| d.package_name()),
        resolved.is_some()
    );

    if let Some(name) = node.crate_name() {
        debug!("crate_name detected: {}", name);
        return crate_name_completion(http_client, name).await;
    }

    debug!("no crate_name, matching on node.kind: {:?}", node.kind);
    match &node.kind {
        NodeKind::Value(ValueKind::Dependency(DependencyValue::Simple))
        | NodeKind::Value(ValueKind::Dependency(DependencyValue::Version)) => {
            debug!("matched Simple or Version dependency value");
            // Try resolved versions first (fast path)
            if let Some(resolved) = resolved {
                let available_versions = &resolved.available_versions;
                debug!("resolved available_versions count: {}", available_versions.len());
                if !available_versions.is_empty() {
                    debug!("returning version completion from resolved list");
                    return Some(version_completion_from_list(available_versions, node));
                }
            }

            // Fallback: fetch versions from crates.io for unresolved dependencies
            if let Some(dep) = dep {
                debug!("falling back to fetch_versions_for_crate for: {}", dep.package_name());
                return fetch_versions_for_crate(http_client, dep.package_name(), node).await;
            }

            debug!("no dep available, returning None");
            None
        }
        NodeKind::Value(ValueKind::Dependency(DependencyValue::Feature)) => {
            debug!("matched Feature dependency value");
            let resolved = resolved?;
            let features = resolved.features()?;
            debug!("features count: {}", features.len());
            let feature_items: Vec<_> = features
                .keys()
                .map(|s| CompletionItem {
                    label: s.clone(),
                    kind: Some(CompletionItemKind::CONSTANT),
                    detail: Some(s.clone()),
                    documentation: None,
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range: Range::new(
                            Position::new(node.range.start.line, node.range.start.character + 1),
                            Position::new(node.range.end.line, node.range.end.character - 1),
                        ),
                        new_text: s.clone(),
                    })),
                    ..Default::default()
                })
                .collect();
            Some(CompletionResponse::Array(feature_items))
        }
        _ => {
            debug!("node.kind did not match any completion handler");
            None
        }
    }
}

/// Build version completion from a pre-loaded list
fn version_completion_from_list(
    available_versions: &[String],
    node: &TomlNode,
) -> CompletionResponse {
    let versions: Vec<_> = available_versions
        .iter()
        .enumerate()
        .map(|(index, version)| CompletionItem {
            label: version.clone(),
            kind: Some(CompletionItemKind::CONSTANT),
            detail: Some(version.clone()),
            documentation: None,
            sort_text: Some(format!("{:04}", index)),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: Range::new(
                    Position::new(node.range.start.line, node.range.start.character + 1),
                    Position::new(node.range.end.line, node.range.end.character - 1),
                ),
                new_text: version.clone(),
            })),
            ..Default::default()
        })
        .collect();
    CompletionResponse::Array(versions)
}

async fn crate_name_completion(
    http_client: &reqwest::Client,
    crate_name: &str,
) -> Option<CompletionResponse> {
    debug!("crate_name_completion() for: {}", crate_name);
    #[derive(Deserialize, Debug)]
    struct SearchCrateOutput {
        name: String,
        max_version: String,
        description: Option<String>,
    }

    #[derive(Deserialize, Debug)]
    struct SearchCrateResponse {
        crates: Vec<SearchCrateOutput>,
    }

    let url = format!(
        "https://crates.io/api/v1/crates?page=1&per_page=30&q={}",
        crate_name
    );

    let resp = match http_client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to search crates.io for '{}': {}", crate_name, e);
            return None;
        }
    };

    let search_response: SearchCrateResponse = match resp.json().await {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to parse search response for '{}': {}", crate_name, e);
            return None;
        }
    };

    debug!("search returned {} crates", search_response.crates.len());
    let completion_items: Vec<CompletionItem> = search_response
        .crates
        .into_iter()
        .map(|crate_info| CompletionItem {
            label: crate_info.name,
            kind: Some(CompletionItemKind::MODULE),
            detail: Some(format!("v{}", crate_info.max_version)),
            documentation: crate_info
                .description
                .map(tower_lsp::lsp_types::Documentation::String),
            ..Default::default()
        })
        .collect();

    debug!("returning {} completion items", completion_items.len());
    Some(CompletionResponse::Array(completion_items))
}

/// Fetch available versions for a crate from crates.io (fallback for unresolved deps)
async fn fetch_versions_for_crate(
    http_client: &reqwest::Client,
    crate_name: &str,
    node: &TomlNode,
) -> Option<CompletionResponse> {
    debug!("fetch_versions_for_crate() for: {}", crate_name);
    #[derive(Deserialize, Debug)]
    struct Version {
        num: String,
        yanked: bool,
    }

    #[derive(Deserialize, Debug)]
    struct CrateResponse {
        versions: Vec<Version>,
    }

    let url = format!("https://crates.io/api/v1/crates/{}/versions", crate_name);

    let resp = match http_client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to fetch versions for crate '{}': {}", crate_name, e);
            return None;
        }
    };

    if !resp.status().is_success() {
        error!(
            "crates.io returned {} for crate '{}'",
            resp.status(),
            crate_name
        );
        return None;
    }

    let crate_response: CrateResponse = match resp.json().await {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to parse versions response for '{}': {}", crate_name, e);
            return None;
        }
    };

    // Filter out yanked versions and collect
    let versions: Vec<String> = crate_response
        .versions
        .into_iter()
        .filter(|v| !v.yanked)
        .map(|v| v.num)
        .collect();

    debug!("fetched {} non-yanked versions", versions.len());

    if versions.is_empty() {
        debug!("no versions available, returning None");
        return None;
    }

    debug!("returning version completion from fetched list");
    Some(version_completion_from_list(&versions, node))
}
