use tower_lsp::lsp_types::Range;

use crate::entity::DependencyEntryKind;

use super::{CargoTable, DependencyKeyKind, EntryKind, KeyKind, TomlEntry, TomlKey};

#[derive(Debug, Clone)]
pub struct TomlNode {
    pub id: String,
    pub range: Range,
    pub text: String,
    pub table: CargoTable,
    pub kind: NodeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Entry(EntryKind),
    Key(KeyKind),
}

impl TomlNode {
    pub fn is_entry(&self) -> bool {
        matches!(self.kind, NodeKind::Entry(_))
    }

    pub fn is_key(&self) -> bool {
        matches!(self.kind, NodeKind::Key(_))
    }

    pub fn is_dependency(&self) -> bool {
        match self.kind {
            NodeKind::Entry(EntryKind::Dependency(_, _)) => true,
            NodeKind::Key(KeyKind::Dependency(_, _)) => true,
            _ => false,
        }
    }

    pub fn is_top_level_dependency(&self) -> bool {
        matches!(
            self.kind,
            NodeKind::Entry(EntryKind::Dependency(
                _,
                DependencyEntryKind::SimpleDependency
            )) | NodeKind::Entry(EntryKind::Dependency(
                _,
                DependencyEntryKind::TableDependency
            ))
        )
    }

    pub fn row_id(&self) -> Option<String> {
        match &self.kind {
            NodeKind::Entry(e) => e.row_id(),
            NodeKind::Key(k) => k.row_id(),
        }
    }

    pub fn crate_name(&self) -> Option<String> {
        let NodeKind::Key(KeyKind::Dependency(_, DependencyKeyKind::CrateName)) = self.kind else {
            return None;
        };
        Some(self.text.clone())
    }

    pub fn new_entry(
        id: String,
        range: Range,
        text: String,
        table: CargoTable,
        kind: EntryKind,
    ) -> Self {
        Self {
            id,
            range,
            text,
            table,
            kind: NodeKind::Entry(kind),
        }
    }

    pub fn new_key(
        id: String,
        range: Range,
        text: String,
        table: CargoTable,
        kind: KeyKind,
    ) -> Self {
        Self {
            id,
            range,
            text,
            table,
            kind: NodeKind::Key(kind),
        }
    }
}
