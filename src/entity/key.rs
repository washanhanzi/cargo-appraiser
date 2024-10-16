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
    Dependency(DependencyKeyKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyKeyKind {
    CrateName,
    Version,
    Features,
}

impl TomlKey {
    pub fn crate_name(&self) -> Option<String> {
        match self.kind {
            KeyKind::Dependency(DependencyKeyKind::CrateName) => Some(self.text.clone()),
            _ => None,
        }
    }
}
