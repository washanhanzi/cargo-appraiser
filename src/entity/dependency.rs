use cargo::core::Summary;
use tower_lsp::lsp_types::Range;

use super::{DependencyTable, Value};

#[derive(Debug, Default, Clone)]
pub struct Dependency {
    pub id: String,
    pub range: Range,
    //name in Cargo.toml
    pub name: String,
    pub table: DependencyTable,
    pub version: Option<Value<String>>,
    pub features: Option<Vec<Value<String>>>,
    pub registry: Option<Value<String>>,
    pub git: Option<Value<String>>,
    pub branch: Option<Value<String>>,
    pub tag: Option<Value<String>>,
    pub path: Option<Value<String>>,
    pub rev: Option<Value<String>>,
    pub package: Option<Value<String>>,
    pub workspace: Option<Value<bool>>,
    pub platform: Option<String>,
    pub requested: Option<cargo::core::Dependency>,
    pub resolved: Option<cargo::core::package::Package>,
    pub summaries: Option<Vec<Summary>>,
    //the exact matched summary(the installed version)
    pub matched_summary: Option<Summary>,
    //the latest summary only consider pre-release
    pub latest_summary: Option<Summary>,
    //the latest summary that satisify the version requirement
    pub latest_matched_summary: Option<Summary>,
    pub is_virtual: bool,
    pub used_in_tables: Vec<DependencyTable>,
}

impl Dependency {
    pub fn package_name(&self) -> &str {
        self.package
            .as_ref()
            .map(|v| v.value())
            .unwrap_or(&self.name)
    }

    pub fn platform(&self) -> Option<&str> {
        self.platform.as_deref()
    }

    pub fn merge_range(&mut self, dep: Dependency) {
        self.range = dep.range;
        self.version = dep.version;
        self.features = dep.features;
        self.registry = dep.registry;
        self.git = dep.git;
        self.branch = dep.branch;
        self.tag = dep.tag;
        self.path = dep.path;
        self.rev = dep.rev;
        self.package = dep.package;
        self.workspace = dep.workspace;
        self.platform = dep.platform;
    }
}
