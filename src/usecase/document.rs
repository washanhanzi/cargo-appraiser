use std::collections::HashMap;

use tower_lsp::lsp_types::{Position, Url};

use crate::entity::{
    cargo_dependency_to_toml_key, Dependency, EntryDiff, SymbolTree, TomlEntry, TomlKey,
    TomlParsingError,
};

use super::{diff_dependency_entries, ReverseSymbolTree, Walker};

#[derive(Debug, Clone)]
pub struct Document {
    pub uri: Url,
    pub rev: usize,
    tree: SymbolTree,
    pub reverse_tree: ReverseSymbolTree,
    //might be empty
    pub dependencies: HashMap<String, Dependency>,
    pub dirty_nodes: HashMap<String, usize>,
    pub parsing_errors: Vec<TomlParsingError>,
}

impl Document {
    pub fn tree(&self) -> &SymbolTree {
        &self.tree
    }

    pub fn parse(uri: &Url, text: &str) -> Self {
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

        let (tree, deps, errs) = walker.consume();
        let len = entries.len();
        let reverse_symbols = ReverseSymbolTree::parse(&tree);
        Self {
            uri: uri.clone(),
            rev: 0,
            tree,
            reverse_tree: reverse_symbols,
            dependencies: deps,
            dirty_nodes: HashMap::with_capacity(len),
            parsing_errors: errs,
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
            self.dirty_nodes.insert(v.to_string(), self.rev);
            if let Some(dep) = new.dependencies.get(v) {
                self.dependencies.insert(v.to_string(), dep.clone());
            }
        }
        for v in &diff.value_updated {
            self.dirty_nodes.insert(v.to_string(), self.rev);
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
                        dep.unresolved = None;
                        dep.resolved = None;
                        dep.latest_summary = None;
                        dep.latest_matched_summary = None;
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
            self.dirty_nodes.remove(v);
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
            self.dirty_nodes.insert(v.to_string(), self.rev);
        }
    }

    //TODO return error
    pub fn populate_dependencies(&mut self) {
        if let Ok(path) = self.uri.to_file_path() {
            //get dependencies
            let Ok(gctx) = cargo::util::context::GlobalContext::default() else {
                return;
            };
            let Ok(workspace) = cargo::core::Workspace::new(path.as_path(), &gctx) else {
                return;
            };
            let Ok(current) = workspace.current() else {
                return;
            };
            let mut unresolved = HashMap::with_capacity(current.dependencies().len());
            for dep in current.dependencies() {
                let key = cargo_dependency_to_toml_key(dep);
                unresolved.insert(key, dep);
            }

            for dep in self.dependencies.values_mut() {
                let key = dep.toml_key();
                if let Some(u) = unresolved.remove(&key) {
                    //if dep has matched_summary and the new unresolved doesn't match it
                    if let Some(summary) = dep.matched_summary.as_ref() {
                        if !u.matches(summary) && !u.matches_prerelease(summary) {
                            dep.matched_summary = None;
                        }
                    }
                    //update value to dep.unresolved
                    dep.unresolved = Some(u.clone());
                }
            }
        }
    }

    pub fn is_dirty(&self) -> bool {
        !self.dirty_nodes.is_empty()
    }

    pub fn precise_match_entry(&self, pos: Position) -> Option<TomlEntry> {
        self.reverse_tree
            .precise_match_entry(pos, &self.tree.entries)
    }

    pub fn precise_match_key(&self, pos: Position) -> Option<TomlKey> {
        self.reverse_tree.precise_match_key(pos, &self.tree.keys)
    }

    pub fn dependency(&self, id: &str) -> Option<&Dependency> {
        if id.is_empty() {
            return None;
        }
        self.dependencies.get(id)
    }

    pub fn entry(&self, id: &str) -> Option<&TomlEntry> {
        self.tree.entries.get(id)
    }

    pub fn find_keys_by_crate_name(&self, crate_name: &str) -> Vec<&TomlKey> {
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
}

mod tests {
    use tower_lsp::lsp_types::Url;

    use crate::{
        entity::{
            CargoTable, DependencyEntryKind, DependencyKeyKind, DependencyTable, EntryKind, KeyKind,
        },
        usecase::document::Document,
    };

    #[test]
    fn test_parse() {
        let doc = Document::parse(
            &Url::parse("file:///C:/Users/test.toml").unwrap(),
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
            assert_eq!(v.kind, KeyKind::Dependency(DependencyKeyKind::CrateName));
        }
        for (_, v) in doc.tree.entries.iter() {
            assert_eq!(
                v.table,
                CargoTable::Dependencies(DependencyTable::Dependencies)
            );
            assert_eq!(
                v.kind,
                EntryKind::Dependency(
                    "dependencies.a".to_string(),
                    DependencyEntryKind::SimpleDependency
                )
            );
        }
    }
}
