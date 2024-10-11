use cargo::core::Summary;
use tower_lsp::lsp_types::Range;

use super::{DependencyTable, Value};

#[derive(Debug, Default, Clone)]
pub struct Dependency {
    pub id: String,
    pub range: Range,
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
    pub platform: Option<Value<String>>,
    pub unresolved: Option<cargo::core::Dependency>,
    pub resolved: Option<cargo::core::package::SerializedPackage>,
    pub summaries: Option<Vec<Summary>>,
    //the exact matched summary(the installed version)
    pub matched_summary: Option<Summary>,
    //the latest summary only consider pre-release
    pub latest_summary: Option<Summary>,
    //the latest summary that satisify the version requirement
    pub latest_matched_summary: Option<Summary>,
}

impl Dependency {
    pub fn package_name(&self) -> &str {
        self.package
            .as_ref()
            .map(|v| v.value.as_str())
            .unwrap_or(&self.name)
    }

    pub fn toml_key(&self) -> String {
        let platform = match &self.platform {
            Some(p) => &p.value,
            None => "",
        };
        format!("{}:{}:{}", self.table, self.name, platform)
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

pub fn cargo_dependency_to_toml_key(dep: &cargo::core::Dependency) -> String {
    let platform = match dep.platform() {
        Some(p) => p.to_string(),
        None => "".to_string(),
    };
    format!(
        "{}:{}:{}",
        dep.kind().kind_table(),
        dep.name_in_toml(),
        platform
    )
}

// pub fn cargo_package_to_dependency(pkg: &cargo::core::Package) -> String {
//     let mut dep = Dependency::default();
//     dep.id = pkg.name().to_string();
//     dep.name = pkg.name().to_string();
//     dep.version = pkg.version().to_string();
//     dep
// }
