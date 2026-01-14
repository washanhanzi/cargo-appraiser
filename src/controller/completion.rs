use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionResponse, CompletionTextEdit,
    Position, Range, TextEdit,
};
use tracing::debug;

use crate::entity::{
    DependencyValue, NodeKind, ResolvedDependency, TomlDependency, TomlNode, ValueKind,
};
use crate::usecase::{fetch_features, fetch_versions, search_crates};

pub async fn completion(
    http_client: &reqwest::Client,
    node: &TomlNode,
    dep: Option<&TomlDependency>,
    resolved: Option<&ResolvedDependency>,
) -> Option<CompletionResponse> {
    debug!(
        "completion: node.kind={:?}, node.text='{}', crate_name={:?}",
        node.kind,
        node.text,
        node.crate_name()
    );

    if let Some(name) = node.crate_name() {
        debug!("completion: calling crate_name_completion for '{}'", name);
        let result = crate_name_completion(http_client, name, node.range).await;
        debug!(
            "completion: crate_name_completion returned {:?} items",
            result.as_ref().map(|r| match r {
                CompletionResponse::Array(items) => items.len(),
                CompletionResponse::List(list) => list.items.len(),
            })
        );
        return result;
    }

    match &node.kind {
        NodeKind::Value(ValueKind::Dependency(DependencyValue::Simple))
        | NodeKind::Value(ValueKind::Dependency(DependencyValue::Version)) => {
            // Try resolved versions first (fast path)
            if let Some(resolved) = resolved {
                let available_versions = &resolved.available_versions;
                if !available_versions.is_empty() {
                    return Some(version_completion_from_list(available_versions, node));
                }
            }

            // Fallback: fetch versions from crates.io for unresolved dependencies
            if let Some(dep) = dep {
                return fetch_versions_for_crate(http_client, dep.package_name(), node).await;
            }

            None
        }
        NodeKind::Value(ValueKind::Dependency(DependencyValue::Feature)) => {
            // Try resolved features first (fast path)
            if let Some(resolved) = resolved {
                if let Some(features) = resolved.features() {
                    return Some(feature_completion_from_keys(features.keys(), node));
                }
            }

            // Fallback: fetch features from crates.io for unresolved dependencies
            if let Some(dep) = dep {
                if let Some(version) = dep.version() {
                    return fetch_features_for_crate(
                        http_client,
                        dep.package_name(),
                        &version.text,
                        node,
                    )
                    .await;
                }
            }

            None
        }
        _ => None,
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
    replace_range: Range,
) -> Option<CompletionResponse> {
    let crates = search_crates(http_client, crate_name).await?;

    let completion_items: Vec<CompletionItem> = crates
        .into_iter()
        .map(|crate_info| CompletionItem {
            label: crate_info.name.clone(),
            kind: Some(CompletionItemKind::MODULE),
            detail: Some(format!("v{}", crate_info.max_version)),
            documentation: crate_info
                .description
                .map(tower_lsp::lsp_types::Documentation::String),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: replace_range,
                new_text: crate_info.name,
            })),
            ..Default::default()
        })
        .collect();

    // Return CompletionList with is_incomplete=true to tell VS Code to re-request
    // completions as user types more. This helps work around VS Code's client-side
    // throttling of completion requests during rapid typing.
    Some(CompletionResponse::List(CompletionList {
        is_incomplete: true,
        items: completion_items,
    }))
}

/// Fetch available versions for a crate from crates.io (fallback for unresolved deps)
async fn fetch_versions_for_crate(
    http_client: &reqwest::Client,
    crate_name: &str,
    node: &TomlNode,
) -> Option<CompletionResponse> {
    let versions = fetch_versions(http_client, crate_name).await?;
    Some(version_completion_from_list(&versions, node))
}

/// Build feature completion from feature keys
fn feature_completion_from_keys<'a, I>(keys: I, node: &TomlNode) -> CompletionResponse
where
    I: Iterator<Item = &'a String>,
{
    let feature_items: Vec<_> = keys
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
    CompletionResponse::Array(feature_items)
}

/// Fetch features for a crate version from crates.io (fallback for unresolved deps)
async fn fetch_features_for_crate(
    http_client: &reqwest::Client,
    crate_name: &str,
    version: &str,
    node: &TomlNode,
) -> Option<CompletionResponse> {
    let features = fetch_features(http_client, crate_name, version).await?;
    Some(feature_completion_from_keys(features.keys(), node))
}
