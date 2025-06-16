use std::collections::HashMap;

use tower_lsp::lsp_types::{Position, Uri};

use crate::entity::{
    CanonicalUri, Dependency, EntryDiff, Manifest, SymbolTree, TomlNode, TomlParsingError,
};

use super::{diff_dependency_entries, ReverseSymbolTree, Walker};

#[derive(Debug, Clone)]
pub struct Document {
    pub uri: Uri,
    pub canonical_uri: CanonicalUri,
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

    pub fn parse(uri: Uri, canonical_uri: CanonicalUri, text: &str) -> Self {
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
            uri,
            canonical_uri,
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
                self.crate_name_map
                    .entry(dep.package_name().to_string())
                    .or_default()
                    .push(v.to_string());
                self.dependencies.insert(v.to_string(), dep.clone());
            }
        }
        for v in &diff.value_updated {
            self.dirty_dependencies.insert(v.to_string(), self.rev);
            if let Some(new_dep) = new.dependencies.remove(v) {
                if let Some(old_dep) = self.dependencies.get_mut(v) {
                    let old_name = old_dep.package_name().to_string();
                    if let Some(ids) = self.crate_name_map.get_mut(&old_name) {
                        ids.retain(|id| id != v);
                        if ids.is_empty() {
                            self.crate_name_map.remove(&old_name);
                        }
                    }
                    let new_name = new_dep.package_name().to_string();
                    self.crate_name_map
                        .entry(new_name)
                        .or_default()
                        .push(v.to_string());
                    old_dep.version = new_dep.version.clone();
                    old_dep.features = new_dep.features.clone();
                    old_dep.registry = new_dep.registry.clone();
                    old_dep.git = new_dep.git.clone();
                    old_dep.branch = new_dep.branch.clone();
                    old_dep.tag = new_dep.tag.clone();
                    old_dep.path = new_dep.path.clone();
                    old_dep.rev = new_dep.rev.clone();
                    old_dep.package = new_dep.package.clone();
                    old_dep.workspace = new_dep.workspace.clone();
                    old_dep.platform = new_dep.platform.clone();
                    old_dep.requested = None;
                    old_dep.resolved = None;
                    old_dep.latest_summary = None;
                    old_dep.latest_matched_summary = None;
                    old_dep.range = new_dep.range;
                } else {
                    self.crate_name_map
                        .entry(new_dep.package_name().to_string())
                        .or_default()
                        .push(v.to_string());
                    self.dependencies.insert(v.to_string(), new_dep);
                }
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
            if let Some(dep) = self.dependencies.remove(v) {
                if let Some(ids) = self.crate_name_map.get_mut(dep.package_name()) {
                    ids.retain(|id| id != v);
                    if ids.is_empty() {
                        self.crate_name_map.remove(dep.package_name());
                    }
                }
            }
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

#[cfg(test)]
mod tests {
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
        // Use a platform-agnostic approach with a temp file
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_cargo_appraiser.toml");

        // Create the temp file so canonicalization works
        std::fs::write(&temp_file, "").unwrap();

        let uri = Uri::try_from_path(&temp_file).unwrap();
        let canonical_uri = uri.clone().try_into().unwrap();

        // Clean up the temp file
        std::fs::remove_file(&temp_file).unwrap();

        let doc = Document::parse(
            uri,
            canonical_uri,
            r#"
            [dependencies]
            a = "0.1.0"
            b
            "#,
        );
        assert_eq!(doc.tree.keys.len(), 2);
        assert_eq!(doc.tree.entries.len(), 1);

        // Check that we have keys for both dependencies "a" and "b"
        let mut found_keys = vec![];
        for (_, v) in doc.tree.keys.iter() {
            assert_eq!(
                v.table,
                CargoTable::Dependencies(DependencyTable::Dependencies)
            );
            match &v.kind {
                NodeKind::Key(KeyKind::Dependency(id, DependencyKeyKind::CrateName)) => {
                    found_keys.push(id.clone());
                }
                _ => panic!("Unexpected key kind: {:?}", v.kind),
            }
        }
        found_keys.sort();
        assert_eq!(found_keys, vec!["dependencies.a", "dependencies.b"]);

        // Check that we have an entry only for dependency "a" (which has a value)
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
