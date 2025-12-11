use serde::Deserialize;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit, Position, Range,
    TextEdit,
};

use crate::entity::{
    DependencyValue, NodeKind, ResolvedDependency, TomlDependency, TomlNode, ValueKind,
};

pub async fn completion(
    node: &TomlNode,
    _dep: Option<&TomlDependency>,
    resolved: Option<&ResolvedDependency>,
) -> Option<CompletionResponse> {
    if let Some(name) = node.crate_name() {
        //crate name completion
        return crate_name_completion(name).await;
    }

    let resolved = resolved?;
    let available_versions = &resolved.available_versions;
    if available_versions.is_empty() {
        return None;
    }

    match &node.kind {
        NodeKind::Value(ValueKind::Dependency(DependencyValue::Simple))
        | NodeKind::Value(ValueKind::Dependency(DependencyValue::Version)) => {
            // Create a vector of CompletionItems for each version
            // available_versions is already sorted by version (descending)
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
            Some(CompletionResponse::Array(versions))
        }
        NodeKind::Value(ValueKind::Dependency(DependencyValue::Feature)) => {
            let features = resolved.features()?;
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
        _ => None,
    }
}

async fn crate_name_completion(crate_name: &str) -> Option<CompletionResponse> {
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

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("User-Agent", "lsp-cargo-appraiser")
        .send()
        .await
        .ok()?;

    let search_response: SearchCrateResponse = resp.json().await.ok()?;

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

    Some(CompletionResponse::Array(completion_items))
}
