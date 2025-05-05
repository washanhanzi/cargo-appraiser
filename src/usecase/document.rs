use std::{collections::HashMap, path::Path};

use cargo::sources::source;
use tower_lsp::lsp_types::{Position, Uri};
use tracing_subscriber::field::debug;

use crate::entity::{
    into_file_uri, Dependency, EntryDiff, Manifest, SymbolTree, TomlNode, TomlParsingError,
};

use tracing::{debug, error, info};

use super::{diff_dependency_entries, ReverseSymbolTree, Walker};

#[derive(Debug, Clone)]
pub struct Document {
    pub uri: Uri,
    pub rev: usize,
    tree: SymbolTree,
    pub reverse_tree: ReverseSymbolTree,
    //hashmap key is id
    pub dependencies: HashMap<String, Dependency>,
    //crate name to Vec<dependency id>
    pub crate_name_map: HashMap<String, Vec<String>>,
    pub dirty_dependencies: HashMap<String, usize>,
    pub parsing_errors: Vec<TomlParsingError>,
    pub manifest: Manifest,
    pub members: Option<Vec<cargo::core::package::Package>>,
}

impl Document {
    pub fn tree(&self) -> &SymbolTree {
        &self.tree
    }

    pub fn parse(uri: &Uri, text: &str) -> Self {
        //TODO I'm too stupid to apprehend the rowan tree else I would use incremental patching
        let p = taplo::parser::parse(text);
        let dom = p.into_dom();
        let table = dom.as_table().unwrap();
        let entries = table.entries().read();

        let mut walker = Walker::new(text, entries.len());

        for (key, entry) in entries.iter() {
            if key.value().is_empty() {
                continue;
            }
            walker.walk_root(key.value(), key.value(), entry)
        }

        let (tree, manifest, deps, errs) = walker.consume();
        let len = entries.len();
        let reverse_symbols = ReverseSymbolTree::parse(&tree);

        //construct crate_name_map
        let mut crate_name_map = HashMap::with_capacity(deps.len());
        for (id, dep) in deps.iter() {
            crate_name_map
                .entry(dep.package_name().to_string())
                .or_insert_with(Vec::new)
                .push(id.to_string());
        }
        Self {
            uri: uri.clone(),
            rev: 0,
            tree,
            manifest,
            reverse_tree: reverse_symbols,
            dependencies: deps,
            crate_name_map,
            dirty_dependencies: HashMap::with_capacity(len),
            parsing_errors: errs,
            members: None,
        }
    }

    //currently only diff dependency entries
    pub fn diff(old: Option<&Document>, new: &Document) -> EntryDiff {
        let old = old.map(|d| &d.tree.entries);
        diff_dependency_entries(old, &new.tree.entries)
    }

    pub fn reconsile(&mut self, mut new: Document, diff: &EntryDiff) {
        self.tree.entries = new.tree.entries;
        self.tree.keys = new.tree.keys;
        self.reverse_tree = new.reverse_tree;
        self.rev += 1;
        //merge dependencies
        for v in &diff.created {
            self.dirty_dependencies.insert(v.to_string(), self.rev);
            if let Some(dep) = new.dependencies.get(v) {
                self.dependencies.insert(v.to_string(), dep.clone());
            }
        }
        for v in &diff.value_updated {
            self.dirty_dependencies.insert(v.to_string(), self.rev);
            if let Some(new_dep) = new.dependencies.remove(v) {
                self.dependencies
                    .entry(v.to_string())
                    .and_modify(|dep| {
                        dep.version = new_dep.version.clone();
                        dep.features = new_dep.features.clone();
                        dep.registry = new_dep.registry.clone();
                        dep.git = new_dep.git.clone();
                        dep.branch = new_dep.branch.clone();
                        dep.tag = new_dep.tag.clone();
                        dep.path = new_dep.path.clone();
                        dep.rev = new_dep.rev.clone();
                        dep.package = new_dep.package.clone();
                        dep.workspace = new_dep.workspace.clone();
                        dep.platform = new_dep.platform.clone();
                        dep.requested = None;
                        dep.resolved = None;
                        dep.latest_summary = None;
                        dep.latest_matched_summary = None;
                        dep.range = new_dep.range;
                        //dep.matched_summary not reset
                        //dep.summaries not reset
                    })
                    .or_insert(new_dep);
            }
        }
        for v in &diff.range_updated {
            if let Some(dep) = new.dependencies.remove(v) {
                if let Some(old_dep) = self.dependencies.get_mut(v) {
                    old_dep.merge_range(dep);
                }
            }
        }
        for v in &diff.deleted {
            self.dirty_dependencies.remove(v);
        }
    }

    pub fn self_reconsile(&mut self, diff: &EntryDiff) {
        self.rev += 1;
        //merge dependencies
        for v in diff
            .created
            .iter()
            .chain(&diff.range_updated)
            .chain(&diff.value_updated)
        {
            self.dirty_dependencies.insert(v.to_string(), self.rev);
        }
    }

    pub fn is_dependencies_dirty(&self) -> bool {
        !self.dirty_dependencies.is_empty()
    }

    pub fn precise_match(&self, pos: Position) -> Option<TomlNode> {
        match self.precise_match_key(pos) {
            Some(node) => Some(node),
            None => self.precise_match_entry(pos),
        }
    }

    pub fn precise_match_entry(&self, pos: Position) -> Option<TomlNode> {
        self.reverse_tree
            .precise_match_entry(pos, &self.tree.entries)
    }

    pub fn precise_match_key(&self, pos: Position) -> Option<TomlNode> {
        self.reverse_tree.precise_match_key(pos, &self.tree.keys)
    }

    pub fn dependency(&self, id: &str) -> Option<&Dependency> {
        if id.is_empty() {
            return None;
        }
        self.dependencies.get(id)
    }

    pub fn dependencies_by_crate_name(&self, crate_name: &str) -> Vec<&Dependency> {
        self.crate_name_map
            .get(crate_name)
            .map(|ids| {
                ids.iter()
                    .map(|id| self.dependencies.get(id).unwrap())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn entry(&self, id: &str) -> Option<&TomlNode> {
        self.tree.entries.get(id)
    }

    pub fn find_keys_by_crate_name(&self, crate_name: &str) -> Vec<&TomlNode> {
        self.tree
            .keys
            .values()
            .filter(|v| v.text == crate_name)
            .collect()
    }

    pub fn find_deps_by_crate_name(&self, crate_name: &str) -> Vec<&Dependency> {
        self.dependencies
            .values()
            .filter(|v| v.package_name() == crate_name)
            .collect()
    }

    pub fn mark_dirty(&mut self) {
        self.rev += 1;
        for k in self.dependencies.keys() {
            self.dirty_dependencies.insert(k.to_string(), self.rev);
        }
    }
}

mod tests {
    use std::str::FromStr;

    use tower_lsp::lsp_types::Uri;

    use crate::{
        entity::{
            CargoTable, DependencyEntryKind, DependencyKeyKind, DependencyTable, EntryKind,
            KeyKind, NodeKind,
        },
        usecase::document::Document,
    };

    #[test]
    fn test_parse() {
        let doc = Document::parse(
            &Uri::from_str("file:///C:/Users/test.toml").unwrap(),
            r#"
            [dependencies]
            a = "0.1.0"
            b
            "#,
        );
        assert_eq!(doc.tree.keys.len(), 2);
        assert_eq!(doc.tree.entries.len(), 1);
        for (_, v) in doc.tree.keys.iter() {
            assert_eq!(
                v.table,
                CargoTable::Dependencies(DependencyTable::Dependencies)
            );
            assert_eq!(
                v.kind,
                NodeKind::Key(KeyKind::Dependency(
                    "dependencies.a".to_string(),
                    DependencyKeyKind::CrateName
                ))
            );
        }
        for (_, v) in doc.tree.entries.iter() {
            assert_eq!(
                v.table,
                CargoTable::Dependencies(DependencyTable::Dependencies)
            );
            assert_eq!(
                v.kind,
                NodeKind::Entry(EntryKind::Dependency(
                    "dependencies.a".to_string(),
                    DependencyEntryKind::SimpleDependency
                ))
            );
        }
    }
}
