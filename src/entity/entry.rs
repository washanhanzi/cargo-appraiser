use serde::Serialize;
use tower_lsp::lsp_types::Range;

use super::CargoTable;

pub struct EntryDiff {
    pub created: Vec<String>,
    pub range_updated: Vec<String>,
    pub value_updated: Vec<String>,
    pub deleted: Vec<String>,
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

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub enum EntryKind {
    Table(CargoTable),
    Dependency(String, DependencyEntryKind),
    Value(String),
}

impl EntryKind {
    pub fn is_dependency(&self) -> bool {
        matches!(
            self,
            EntryKind::Dependency(_, DependencyEntryKind::SimpleDependency)
                | EntryKind::Dependency(_, DependencyEntryKind::TableDependency)
        )
    }
    pub fn row_id(&self) -> &str {
        match self {
            //TODO not ok
            EntryKind::Dependency(id, _) => id,
            EntryKind::Value(id) => id,
            _ => "",
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
