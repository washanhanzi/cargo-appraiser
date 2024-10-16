use std::collections::HashMap;

use tower_lsp::lsp_types::{Position, Url};

use crate::entity::{cargo_dependency_to_toml_key, Dependency, EntryDiff, TomlEntry, TomlKey};

use super::{diff_dependency_entries, symbol_tree::SymbolTree, ReverseSymbolTree, Walker};

#[derive(Debug, Clone)]
pub struct Document {
    pub uri: Url,
    pub rev: usize,
    pub tree: SymbolTree,
    pub reverse_tree: ReverseSymbolTree,
    //might be empty
    pub dependencies: HashMap<String, Dependency>,
    pub dirty_nodes: HashMap<String, usize>,
}

impl Document {
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

        let (tree, deps) = walker.consume();
        let len = entries.len();
        let reverse_symbols = ReverseSymbolTree::parse(&tree);
        Self {
            uri: uri.clone(),
            rev: 0,
            tree,
            reverse_tree: reverse_symbols,
            dependencies: deps,
            dirty_nodes: HashMap::with_capacity(len),
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
        for v in diff.created.iter().chain(&diff.value_updated) {
            self.dirty_nodes.insert(v.to_string(), self.rev);
            self.dependencies
                .insert(v.to_string(), new.dependencies.get(v).unwrap().clone());
        }
        for v in &diff.range_updated {
            let dep = new.dependencies.remove(v).unwrap();
            self.dependencies.get_mut(v).unwrap().merge_range(dep);
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
            let gctx = cargo::util::context::GlobalContext::default().unwrap();
            //TODO ERROR parse manifest
            let Ok(workspace) = cargo::core::Workspace::new(path.as_path(), &gctx) else {
                return;
            };
            //TODO if it's error, it's a virtual workspace
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
        self.dependencies.get(id)
    }

    pub fn entry(&self, id: &str) -> Option<&TomlEntry> {
        self.tree.entries.get(id)
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
