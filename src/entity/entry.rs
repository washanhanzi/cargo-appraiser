use serde::Serialize;
use tower_lsp::lsp_types::Range;

use super::CargoTable;

pub struct EntryDiff {
    pub created: Vec<String>,
    pub range_updated: Vec<String>,
    pub value_updated: Vec<String>,
    pub deleted: Vec<String>,
}

impl EntryDiff {
    pub fn is_empty(&self) -> bool {
        self.created.is_empty()
            && self.range_updated.is_empty()
            && self.value_updated.is_empty()
            && self.deleted.is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TomlEntry {
    pub id: String,
    pub range: Range,
    pub text: String,
    //the table the node belongs to
    pub table: CargoTable,
    //the key of the node, the type of the node
    pub kind: EntryKind,
}

impl TomlEntry {}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub enum EntryKind {
    Workspace(WorkspaceEntryKind),
    Table(CargoTable),
    Dependency(String, DependencyEntryKind),
    Value(String),
}

impl EntryKind {
    pub fn row_id(&self) -> Option<String> {
        match self {
            EntryKind::Dependency(id, _) => Some(id.to_string()),
            EntryKind::Value(id) => Some(id.to_string()),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub enum DependencyEntryKind {
    SimpleDependency,
    TableDependency,
    TableDependencyVersion,
    TableDependencyFeatures,
    TableDependencyFeature,
    TableDependencyRegistry,
    TableDependencyGit,
    TableDependencyBranch,
    TableDependencyTag,
    TableDependencyPath,
    TableDependencyRev,
    TableDependencyPackage,
    TableDependencyWorkspace,
    TableDependencyDefaultFeatures,
    TableDependencyOptional,
    TableDependencyUnknownBool,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub enum WorkspaceEntryKind {
    Members,
}

pub fn strip_quotes(s: &str) -> String {
    if s.starts_with('"') && s.ends_with('"') {
        return s[1..s.len() - 1].to_string();
    }
    s.to_string()
}
