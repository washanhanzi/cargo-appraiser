use serde::Deserialize;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit, Position, Range,
    TextEdit,
};

use crate::entity::{Dependency, DependencyEntryKind, EntryKind, NodeKind, TomlNode};

pub async fn completion(node: &TomlNode, dep: Option<&Dependency>) -> Option<CompletionResponse> {
    if let Some(name) = node.crate_name() {
        //crate name completion
        return crate_name_completion(&name).await;
    }
    let dep = dep?;
    let available_versions = dep.available_versions.as_ref()?;
    //TODO dep is never resolved, manually create a dependency
    if available_versions.is_empty() {
        return None;
    }

    match &node.kind {
        NodeKind::Entry(EntryKind::Dependency(
            _,
            DependencyEntryKind::SimpleDependency | DependencyEntryKind::TableDependencyVersion,
        )) => {
            // Create a vector of CompletionItems for each version
            // available_versions is already sorted by version (descending)
            let versions: Vec<_> = available_versions
                .iter()
                .enumerate()
                .map(|(index, version)| {
                    CompletionItem {
                        label: version.clone(),
                        kind: Some(CompletionItemKind::CONSTANT),
                        detail: Some(version.clone()),
                        documentation: None,
                        sort_text: Some(format!("{:04}", index)),
                        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                            range: Range::new(
                                Position::new(
                                    node.range.start.line,
                                    node.range.start.character + 1,
                                ),
                                Position::new(node.range.end.line, node.range.end.character - 1),
                            ),
                            new_text: version.clone(),
                        })),
                        ..Default::default()
                    }
                })
                .collect();
            Some(CompletionResponse::Array(versions))
        }
        NodeKind::Entry(EntryKind::Dependency(_, DependencyEntryKind::TableDependencyFeature)) => {
            let summary = dep.matched_summary.as_ref()?;
            let versions: Vec<_> = summary
                .features()
                .keys()
                .map(|s| CompletionItem {
                    label: s.to_string(),
                    kind: Some(CompletionItemKind::CONSTANT),
                    detail: Some(s.to_string()),
                    documentation: None,
                    // sort_text: Some(format!("{:04}", index)),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range: Range::new(
                            Position::new(node.range.start.line, node.range.start.character + 1),
                            Position::new(node.range.end.line, node.range.end.character - 1),
                        ),
                        new_text: s.to_string(),
                    })),
                    ..Default::default()
                })
                .collect();
            Some(CompletionResponse::Array(versions))
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
