use tower_lsp::lsp_types::{GotoDefinitionResponse, Location, Position, Range, Uri};

use crate::{
    entity::{
        DependencyKey, DependencyValue, KeyKind, NodeKind, TomlNode, ValueKind, WorkspaceValue,
    },
    usecase::{Document, Workspace},
};

pub fn goto_definition(
    state: &Workspace,
    doc: &Document,
    node: &TomlNode,
) -> Option<GotoDefinitionResponse> {
    // Check if the node is a workspace member path
    if let NodeKind::Value(ValueKind::Workspace(WorkspaceValue::Member)) = &node.kind {
        return goto_workspace_member(doc, node);
    }

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

/// Go to definition for a workspace member path (non-glob)
fn goto_workspace_member(doc: &Document, node: &TomlNode) -> Option<GotoDefinitionResponse> {
    let member_path = &node.text;

    // Skip glob patterns
    if member_path.contains('*') || member_path.contains('?') {
        return None;
    }

    // Get the workspace root directory from the document URI
    let doc_path = doc.canonical_uri.to_path_buf().ok()?;
    let workspace_root = doc_path.parent()?;

    // Resolve the member path relative to workspace root
    let member_dir = workspace_root.join(member_path);
    let cargo_toml = member_dir.join("Cargo.toml");

    // Check if the Cargo.toml exists
    if !cargo_toml.exists() {
        return None;
    }

    let uri = Uri::try_from_path(&cargo_toml).ok()?;

    Some(GotoDefinitionResponse::Scalar(Location {
        uri,
        range: Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 0,
            },
        },
    }))
}
