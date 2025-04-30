use std::{collections::HashMap, path::Path};

use cargo::sources::source;
use tower_lsp::lsp_types::{Position, Uri};
use tracing_subscriber::field::debug;

use crate::entity::{
    cargo_dependency_to_toml_key, into_file_uri, Dependency, EntryDiff, Manifest, SymbolTree,
    TomlNode, TomlParsingError,
};

use tracing::{debug, error, info};

use super::{diff_dependency_entries, ReverseSymbolTree, Walker};

#[derive(Debug, Clone)]
pub struct Document {
    pub uri: Uri,
    pub rev: usize,
    tree: SymbolTree,
    pub reverse_tree: ReverseSymbolTree,
    //might be empty
    pub dependencies: HashMap<String, Dependency>,
    pub dirty_dependencies: HashMap<String, usize>,
    pub parsing_errors: Vec<TomlParsingError>,
    pub manifest: Manifest,
    pub members: Option<Vec<cargo::core::package::Package>>,
    pub root_manifest: Option<Uri>,
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
        Self {
            uri: uri.clone(),
            rev: 0,
            tree,
            manifest,
            reverse_tree: reverse_symbols,
            dependencies: deps,
            dirty_dependencies: HashMap::with_capacity(len),
            parsing_errors: errs,
            root_manifest: None,
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

    //maybe remove this resolve
    //move the populate to cargo resolve
    //we need hashmap<String, Vec<Dependency>>, toml_name -> Vec<Dependency>
    //cargo resolve will also get package, package_name-> Vec<Package>, dep.matches(pkg.summary())
    //id -> Vec<Summary>
    //
    //we need a temp DependencyWithId to track the Dependency -> Vec<Summary>
    pub fn populate_dependencies(&mut self) {
        let path = Path::new(self.uri.path().as_str());
        //get dependencies
        let Ok(gctx) = cargo::util::context::GlobalContext::default() else {
            return;
        };
        let Ok(workspace) = cargo::core::Workspace::new(path, &gctx) else {
            return;
        };
        self.root_manifest = Some(into_file_uri(&workspace.root().join("Cargo.toml")));

        let (sources_id_map, requested_map) = match workspace.current() {
            Ok(current) => {
                let mut deps = HashMap::with_capacity(current.dependencies().len());
                let mut sources_id_map = HashMap::with_capacity(current.dependencies().len());
                for dep in current.dependencies() {
                    sources_id_map
                        .entry(dep.name_in_toml().to_string())
                        .or_insert_with(Vec::new)
                        .push(dep.source_id());
                    deps.insert(dep.source_id(), dep);
                }
                (sources_id_map, deps)
            }
            Err(_) => {
                let members = workspace.members().cloned().collect::<Vec<_>>();
                let hint = members.len();
                self.members = Some(members);
                let mut deps = HashMap::with_capacity(hint * 10);
                let mut sources_id_map = HashMap::with_capacity(hint * 10);
                for member in workspace.members() {
                    let manifest = member.manifest();
                    for dep in manifest.dependencies() {
                        sources_id_map
                            .entry(dep.name_in_toml().to_string())
                            .or_insert_with(Vec::new)
                            .push(dep.source_id());
                        deps.insert(dep.source_id(), dep);
                    }
                }
                (sources_id_map, deps)
            }
        };

        info!(" dependencies: {:?}", sources_id_map);
        info!(" requested_map: {:?}", requested_map);

        for dep in self.dependencies.values_mut() {
            //use dep.name to find source_id in sources_id_map
            let Some(source_ids) = sources_id_map.get(&dep.name) else {
                error!("{} not found in sources_id_map", dep.name);
                unreachable!()
            };
            let requested_dep = match source_ids.len() {
                0 => {
                    // This shouldn't happen as we're adding to a Vec
                    error!("{} has empty source_ids", dep.name);
                    unreachable!()
                }
                1 => requested_map.get(&source_ids[0]).unwrap(),
                _ => {
                    // Multiple source_ids
                    let mut matched_dep = None;

                    for source_id in source_ids {
                        let Some(requested_dep) = requested_map.get(source_id) else {
                            error!("{} not found in requested_map", source_id);
                            unreachable!()
                        };

                        // For not virtual dependency, they should in same table
                        if !dep.is_virtual
                            && dep.table.to_string() != requested_dep.kind().kind_table()
                        {
                            continue;
                        }
                        // For virtual dependency, they should in same platform
                        if dep.platform().is_none() && requested_dep.platform().is_none() {
                            matched_dep = Some(requested_dep);
                            break;
                        } else if let Some(p) = requested_dep.platform() {
                            if &p.to_string() == dep.platform().unwrap_or_default() {
                                matched_dep = Some(requested_dep);
                                break;
                            }
                        }
                    }
                    let Some(requested_dep) = matched_dep else {
                        error!("{} not found in requested_map", source_ids[0]);
                        unreachable!()
                    };
                    requested_dep
                }
            };
            // Check if we need to update matched_summary
            if let Some(summary) = dep.matched_summary.as_ref() {
                if !requested_dep.matches(summary) && !requested_dep.matches_prerelease(summary) {
                    dep.matched_summary = None;
                }
            }
            dep.requested = Some((*requested_dep).clone());
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
