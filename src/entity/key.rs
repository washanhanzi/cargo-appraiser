use tower_lsp::lsp_types::Range;

use super::CargoTable;

#[derive(Debug, Clone)]
pub struct TomlKey {
    pub id: String,
    pub range: Range,
    pub text: String,
    pub table: CargoTable,
    pub kind: KeyKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyKind {
    Workspace(WorkspaceKeyKind),
    Dependency(String, DependencyKeyKind),
}

impl KeyKind {
    pub fn row_id(&self) -> Option<String> {
        match self {
            KeyKind::Dependency(id, _) => Some(id.to_string()),
            KeyKind::Workspace(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyKeyKind {
    CrateName,
    Version,
    Features,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceKeyKind {
    Members,
}
