use serde::Deserialize;
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, CompletionResponse};
use tracing::info;

use crate::entity::{
    DependencyEntryKind, DependencyKeyKind, EntryKind, KeyKind, TomlEntry, TomlKey,
};

pub async fn completion(
    key: Option<TomlKey>,
    node: Option<TomlEntry>,
) -> Option<CompletionResponse> {
    if let Some(key) = key {
        //crate name completion
        if let Some(crate_name) = key.crate_name() {
            return crate_name_completion(&crate_name).await;
        }
    }
    if let Some(node) = node {
        match &node.kind {
            // EntryKind::Invalid(InvalideKey::Dependency(InvalidDependencyKey::CrateName)) => {
            //     crate_name_completion(&node)
            // }
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
