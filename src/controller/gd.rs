use tower_lsp::lsp_types::{GotoDefinitionResponse, Location, Position};

use crate::{
    entity::{DependencyEntryKind, DependencyKeyKind, EntryKind, KeyKind, NodeKind, TomlNode},
    usecase::{Document, Workspace},
};

pub fn goto_definition(
    state: &Workspace,
    doc: &Document,
    node: &TomlNode,
) -> Option<GotoDefinitionResponse> {
    if let NodeKind::Entry(EntryKind::Dependency(
        dep_id,
        DependencyEntryKind::TableDependencyWorkspace,
    ))
    | NodeKind::Key(KeyKind::Dependency(dep_id, DependencyKeyKind::Workspace)) = &node.kind
    {
        let dep = doc.dependency(dep_id)?;
        let root_uri = doc.root_manifest.as_ref()?;
        let root_doc = state.document(root_uri)?;
        for d in root_doc.dependencies.values() {
            if d.name == dep.name && d.is_virtual {
                return Some(GotoDefinitionResponse::Scalar(Location {
                    uri: root_uri.clone(),
                    range: d.range,
                }));
            }
        }
    }
    None
}
