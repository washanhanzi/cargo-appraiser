use std::collections::HashMap;

use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionResponse, TextEdit, Url, WorkspaceEdit,
};

use crate::entity::{CargoKey, CargoNode, Dependency, DependencyKey};

pub fn code_action(uri: Url, node: CargoNode, dep: &Dependency) -> Option<CodeActionResponse> {
    if let CargoKey::Dpendency(id, key) = &node.key {
        code_action_dependency(uri, id, key, &node, dep)
    } else {
        None
    }
}

pub fn code_action_dependency(
    uri: Url,
    id: &str,
    key: &DependencyKey,
    node: &CargoNode,
    dep: &Dependency,
) -> Option<CodeActionResponse> {
    match key {
        DependencyKey::SimpleDependency | DependencyKey::TableDependencyVersion => {
            let mut actions = vec![];
            let latest = match dep.latest_summary.as_ref() {
                Some(summary) => summary.version().to_string(),
                None => return None,
            };

            actions.push(CodeAction {
                title: "Update dependency".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: None,
                edit: Some(WorkspaceEdit {
                    changes: Some(HashMap::from([(
                        uri,
                        vec![TextEdit {
                            new_text: latest,
                            range: node.range,
                        }],
                    )])),
                    document_changes: None,
                    change_annotations: None,
                }),
                command: None,
                is_preferred: None,
                disabled: None,
                data: None,
            });
            None
        }
        _ => None,
    }
}
