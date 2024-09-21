use serde::Serialize;
use tower_lsp::lsp_types::Range;

use super::{package::Package, CargoTable};

//requirement:
//1. quick find range of a node, like quick find the range of a package name
//2. quick find the node from a range, like quick find what the given range point to

//HashMap<id,TomlNode> is a raw representation of cargo.toml
#[derive(Debug, Serialize)]
pub struct CargoNode {
    pub id: String,
    pub range: Range,
    pub text: String,
    //the table the node belongs to
    pub table: CargoTable,
    //the key of the node, the type of the node
    pub key: CargoKey,
}

//packageNode is of node id and value
#[derive(Debug, Clone, Default)]
pub struct Value<T> {
    pub id: String,
    pub value: T,
}

impl<T> Value<T> {
    pub fn new(id: String, value: T) -> Self {
        Self { id, value }
    }
}

#[derive(Default)]
pub struct Manifest {
    package: Package,
}

#[derive(Debug, Serialize)]
pub enum CargoKey {
    Table(CargoTable),
    SimpleDependency(String),
    TableDependency(String),
    TableDependencyVersion,
    TableDependencyFeatures,
    TableDependencyFeature(String),
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
    Key(String),
}
