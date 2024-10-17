use serde::Deserialize;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionResponse, InsertReplaceEdit, TextEdit,
};

use crate::entity::{Dependency, DependencyEntryKind, EntryKind, TomlEntry, TomlKey};

pub async fn completion(
    key: Option<&TomlKey>,
    node: Option<&TomlEntry>,
    dep: Option<&Dependency>,
) -> Option<CompletionResponse> {
    if let Some(key) = key {
        //crate name completion
        if let Some(crate_name) = key.crate_name() {
            return crate_name_completion(&crate_name).await;
        }
    }
    let dep = dep?;
    let summaries = dep.summaries.as_ref()?;
    //TODO dep is never resolved, manually create a dependency
    if summaries.is_empty() {
        return None;
    }

    if let Some(node) = node {
        match &node.kind {
            EntryKind::Dependency(
                _,
                DependencyEntryKind::SimpleDependency | DependencyEntryKind::TableDependencyVersion,
            ) => {
                // Order summaries by version
                let mut summaries = summaries.clone();
                // Sort summaries in descending order by version
                summaries.sort_by(|a, b| b.version().cmp(a.version()));

                // Create a vector of CompletionItems for each version
                let versions: Vec<_> = summaries
                    .iter()
                    .enumerate()
                    .map(|(index, s)| {
                        let version = s.version().to_string();
                        CompletionItem {
                            label: version.to_string(),
                            kind: Some(CompletionItemKind::CONSTANT),
                            detail: Some(version.to_string()),
                            documentation: None,
                            sort_text: Some(format!("{:04}", index)),
                            insert_text: Some(version.to_string()),
                            filter_text: Some(node.text.to_string()),
                            ..Default::default()
                        }
                    })
                    .collect();
                return Some(CompletionResponse::Array(versions));
            }
            EntryKind::Dependency(_, DependencyEntryKind::TableDependencyFeature) => {
                let summary = dep.matched_summary.as_ref()?;
                let versions: Vec<_> = summary
                    .features()
                    .keys()
                    .enumerate()
                    .map(|(index, s)| CompletionItem {
                        label: s.to_string(),
                        kind: Some(CompletionItemKind::CONSTANT),
                        detail: Some(s.to_string()),
                        documentation: None,
                        sort_text: Some(format!("{:04}", index)),
                        insert_text: Some(s.to_string()),
                        filter_text: Some(node.text.to_string()),
                        ..Default::default()
                    })
                    .collect();
                return Some(CompletionResponse::Array(versions));
            }
            _ => return None,
        }
    }
    None
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
