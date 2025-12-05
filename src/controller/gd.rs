use tower_lsp::lsp_types::{GotoDefinitionResponse, Location};

use crate::{
    entity::{DependencyKey, DependencyValue, KeyKind, NodeKind, TomlNode, ValueKind},
    usecase::{Document, Workspace},
};

pub fn goto_definition(
    state: &Workspace,
    doc: &Document,
    node: &TomlNode,
) -> Option<GotoDefinitionResponse> {
    // Check if the node is a workspace = true declaration
    let is_workspace_ref = matches!(
        &node.kind,
        NodeKind::Value(ValueKind::Dependency(DependencyValue::Workspace))
            | NodeKind::Key(KeyKind::Dependency(DependencyKey::Workspace))
    );

    if !is_workspace_ref {
        return None;
    }

    // Find the dependency at this position
    let dep = doc.tree().find_dependency_at_position(node.range.start)?;

    // Look for the workspace dependency definition in the root manifest
    let root_doc = state.root_document()?;

    // Find the matching workspace dependency
    for d in root_doc.dependencies() {
        // Check if it's a workspace dependency (in workspace.dependencies table)
        if doc.is_workspace_dep(d) && d.package_name() == dep.package_name() {
            // Get the entry node for the range
            let entry = root_doc.entry(&d.id)?;
            return Some(GotoDefinitionResponse::Scalar(Location {
                uri: root_doc.uri.clone(),
                range: entry.range,
            }));
        }
    }

    None
}
