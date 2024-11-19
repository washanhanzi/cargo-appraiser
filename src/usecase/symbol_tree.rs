use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

use lsp_async_stub::util::Mapper;
use taplo::{
    dom::{node::Key, Node},
    util::join_ranges,
};
use tower_lsp::lsp_types::{Position, Range};

use crate::entity::{
    validate_crate_name, validate_feature_name, validate_profile_name, CargoTable, Dependency,
    DependencyEntryKind, DependencyKeyKind, EntryDiff, EntryKind, KeyKind, Manifest, SymbolTree,
    TomlNode, TomlParsingError, Value, WorkspaceEntryKind, WorkspaceKeyKind,
};

pub struct Walker {
    keys_map: HashMap<String, TomlNode>,
    entries_map: HashMap<String, TomlNode>,
    deps: HashMap<String, Dependency>,
    mapper: Mapper,
    manifest: Manifest,
    errs: Vec<TomlParsingError>,
}

impl Walker {
    pub fn consume(
        self,
    ) -> (
        SymbolTree,
        Manifest,
        HashMap<String, Dependency>,
        Vec<TomlParsingError>,
    ) {
        (
            SymbolTree {
                keys: self.keys_map,
                entries: self.entries_map,
            },
            self.manifest,
            self.deps,
            self.errs,
        )
    }

    pub fn new(text: &str, capacity: usize) -> Self {
        let mapper = Mapper::new_utf16(text, false);
        Self {
            keys_map: HashMap::with_capacity(capacity),
            entries_map: HashMap::with_capacity(capacity),
            deps: HashMap::with_capacity(capacity),
            mapper,
            manifest: Manifest::default(),
            errs: Vec::with_capacity(capacity),
        }
    }

    pub fn walk_root(&mut self, id: &str, name: &str, node: &Node) {
        match node {
            Node::Table(t) => {
                let parsed_table = CargoTable::from_str(name).unwrap();
                match parsed_table {
                    CargoTable::Package => {}
                    CargoTable::Workspace => {
                        let entries = t.entries().read();
                        for (key, entry) in entries.iter() {
                            let id = id.to_string() + "." + key.value();
                            match key.value() {
                                "members" => {
                                    let (_, _) = self.insert_key(
                                        &id,
                                        parsed_table,
                                        key,
                                        KeyKind::Workspace(WorkspaceKeyKind::Members),
                                    );
                                    self.insert_entry(
                                        &id,
                                        entry,
                                        parsed_table,
                                        EntryKind::Workspace(WorkspaceEntryKind::Members),
                                    );
                                }
                                "dependencies" => {
                                    let Node::Table(table) = entry else {
                                        continue;
                                    };
                                    let entries = table.entries().read();
                                    for (key, entry) in entries.iter() {
                                        let new_id = id.to_string() + "." + key.value();

                                        //insert dep
                                        let mut dep = Dependency {
                                            id: new_id.clone(),
                                            name: key.value().to_string(),
                                            table: crate::entity::DependencyTable::WorkspaceDependencies,
                                            range: into_lsp_range(
                                                self.mapper
                                                    .range(join_ranges(entry.text_ranges()))
                                                    .unwrap(),
                                            ),
                                            is_virtual:true,
                                            ..Default::default()
                                        };
                                        self.enter_dependency(
                                            &new_id,
                                            key,
                                            parsed_table,
                                            entry,
                                            &mut dep,
                                        );
                                        self.deps.insert(new_id, dep);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    CargoTable::Profile => {
                        let entries = t.entries().read();
                        //profile table should contain only 1 child node
                        if entries.len() != 1 {
                            return;
                        }
                        for (key, _) in entries.iter() {
                            if let Err(e) = validate_profile_name(key.value()) {
                                self.errs.push(TomlParsingError::new(
                                    key.value().to_string(),
                                    e,
                                    into_lsp_range(
                                        self.mapper.range(join_ranges(key.text_ranges())).unwrap(),
                                    ),
                                ));
                            }
                        }
                    }
                    CargoTable::Dependencies(dep_table) => {
                        let entries = t.entries().read();
                        for (key, entry) in entries.iter() {
                            let new_id = id.to_string() + "." + key.value();

                            //insert dep
                            let mut dep = Dependency {
                                id: new_id.clone(),
                                name: key.value().to_string(),
                                table: dep_table,
                                range: into_lsp_range(
                                    self.mapper.range(join_ranges(entry.text_ranges())).unwrap(),
                                ),
                                ..Default::default()
                            };
                            self.enter_dependency(&new_id, key, parsed_table, entry, &mut dep);
                            self.deps.insert(new_id, dep);
                        }
                    }
                    CargoTable::Target => {
                        let entries = t.entries().read();
                        for (key, entry) in entries.iter() {
                            let platform = key.value();
                            if let Node::Table(platform_table) = entry {
                                let entries = platform_table.entries().read();
                                for (key, entry) in entries.iter() {
                                    let parsed_table = CargoTable::from_str(key.value()).unwrap();
                                    let CargoTable::Dependencies(dep_table) = parsed_table else {
                                        continue;
                                    };
                                    if let Node::Table(table) = entry {
                                        let entries = table.entries().read();
                                        for (dep_name, entry) in entries.iter() {
                                            let new_id = id.to_string()
                                                + "."
                                                + platform
                                                + "."
                                                + key.value()
                                                + "."
                                                + dep_name.value();

                                            //insert dep
                                            let mut dep = Dependency {
                                                id: new_id.to_string(),
                                                name: dep_name.to_string(),
                                                table: dep_table,
                                                range: into_lsp_range(
                                                    self.mapper
                                                        .range(join_ranges(entry.text_ranges()))
                                                        .unwrap(),
                                                ),
                                                platform: Some(platform.to_string()),
                                                ..Default::default()
                                            };
                                            self.enter_dependency(
                                                &new_id,
                                                dep_name,
                                                parsed_table,
                                                entry,
                                                &mut dep,
                                            );
                                            self.deps.insert(new_id, dep);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => self.enter_generic(id, name, parsed_table, node),
                }
            }
            _ => self.enter_generic(id, name, CargoTable::from_str(name).unwrap(), node),
        }
    }

    fn insert_key(
        &mut self,
        id: &str,
        table: CargoTable,
        key: &Key,
        kind: KeyKind,
    ) -> (String, Range) {
        let key_id = id.to_string() + ".key";
        let key_range = into_lsp_range(self.mapper.range(join_ranges(key.text_ranges())).unwrap());

        if let Err(e) = validate_crate_name(key.value()) {
            self.errs
                .push(TomlParsingError::new(id.to_string(), e, key_range));
        }

        self.keys_map.insert(
            id.to_string(),
            TomlNode::new_key(
                id.to_string(),
                key_range,
                key.value().to_string(),
                table,
                kind,
            ),
        );
        (key_id.to_string(), key_range)
    }

    fn insert_entry(&mut self, id: &str, node: &Node, table: CargoTable, kind: EntryKind) {
        let range = self.mapper.range(join_ranges(node.text_ranges())).unwrap();
        let lsp_range = into_lsp_range(range);
        let text = serde_json::to_string(&node).unwrap_or_default();
        self.entries_map.insert(
            id.to_string(),
            TomlNode::new_entry(id.to_string(), lsp_range, text, table, kind),
        );
    }

    fn enter_dependency(
        &mut self,
        id: &str,
        key: &Key,
        table: CargoTable,
        node: &Node,
        dep: &mut Dependency,
    ) {
        match node {
            //invalid node
            Node::Invalid(_) => {
                let (key_id, key_range) = self.insert_key(
                    id,
                    table,
                    key,
                    KeyKind::Dependency(dep.id.to_string(), DependencyKeyKind::CrateName),
                );

                if let Err(e) = validate_crate_name(key.value()) {
                    self.errs.push(TomlParsingError::new(key_id, e, key_range));
                }
            }
            //inline table dependency
            Node::Table(t) => {
                //insert key
                let (key_id, key_range) = self.insert_key(
                    id,
                    table,
                    key,
                    KeyKind::Dependency(dep.id.to_string(), DependencyKeyKind::CrateName),
                );

                if let Err(e) = validate_crate_name(key.value()) {
                    self.errs
                        .push(TomlParsingError::new(key_id.to_string(), e, key_range));
                }
                self.insert_entry(
                    id,
                    node,
                    table,
                    dep.is_virtual
                        .then(|| {
                            EntryKind::Dependency(
                                dep.id.to_string(),
                                DependencyEntryKind::VirtualTableDependency,
                            )
                        })
                        .unwrap_or(EntryKind::Dependency(
                            dep.id.to_string(),
                            DependencyEntryKind::TableDependency,
                        )),
                );
                let entries = t.entries().read();
                for (key, entry) in entries.iter() {
                    let new_id = id.to_string() + "." + key.value();
                    self.enter_dependency(&new_id, key, table, entry, dep);
                }
            }
            //feature array
            Node::Array(arr) => {
                if key.value() == "features" {
                    let _ = self.insert_key(
                        id,
                        table,
                        key,
                        KeyKind::Dependency(dep.id.to_string(), DependencyKeyKind::Features),
                    );
                    //feature array
                    self.insert_entry(
                        id,
                        node,
                        table,
                        EntryKind::Dependency(
                            dep.id.to_string(),
                            DependencyEntryKind::TableDependencyFeatures,
                        ),
                    );
                    let items = arr.items().read();
                    let mut features = Vec::with_capacity(items.len());
                    for (i, f) in items.iter().enumerate() {
                        let new_id = id.to_string() + "." + &i.to_string();
                        let range = self.mapper.range(join_ranges(f.text_ranges())).unwrap();
                        let lsp_range = into_lsp_range(range);
                        if let Node::Str(s) = f {
                            if let Err(e) = validate_feature_name(s.value()) {
                                self.errs.push(TomlParsingError::new(
                                    new_id.to_string(),
                                    e,
                                    lsp_range,
                                ));
                            }

                            features.push(Value::new(new_id.to_string(), s.value().to_string()));
                            self.entries_map.insert(
                                new_id.to_string(),
                                TomlNode::new_entry(
                                    new_id,
                                    lsp_range,
                                    s.value().to_string(),
                                    table,
                                    EntryKind::Dependency(
                                        dep.id.to_string(),
                                        DependencyEntryKind::TableDependencyFeature,
                                    ),
                                ),
                            );
                        }
                    }
                    dep.features = Some(features);
                }
            }
            //simple dependency or table dependency string key value
            Node::Str(s) => {
                let entry_kind = match key.value() {
                    "version" => {
                        let (key_id, _) = self.insert_key(
                            id,
                            table,
                            key,
                            KeyKind::Dependency(dep.id.to_string(), DependencyKeyKind::Version),
                        );
                        dep.version = Some(Value::new(key_id, s.value().to_string()));
                        EntryKind::Dependency(
                            dep.id.to_string(),
                            DependencyEntryKind::TableDependencyVersion,
                        )
                    }
                    "branch" => {
                        dep.branch = Some(Value::new(id.to_string(), s.value().to_string()));
                        EntryKind::Dependency(
                            dep.id.to_string(),
                            DependencyEntryKind::TableDependencyBranch,
                        )
                    }
                    "tag" => {
                        dep.tag = Some(Value::new(id.to_string(), s.value().to_string()));
                        EntryKind::Dependency(
                            dep.id.to_string(),
                            DependencyEntryKind::TableDependencyTag,
                        )
                    }
                    "path" => {
                        dep.path = Some(Value::new(id.to_string(), s.value().to_string()));
                        EntryKind::Dependency(
                            dep.id.to_string(),
                            DependencyEntryKind::TableDependencyPath,
                        )
                    }
                    "rev" => {
                        dep.rev = Some(Value::new(id.to_string(), s.value().to_string()));
                        EntryKind::Dependency(
                            dep.id.to_string(),
                            DependencyEntryKind::TableDependencyRev,
                        )
                    }
                    "git" => {
                        dep.git = Some(Value::new(id.to_string(), s.value().to_string()));
                        EntryKind::Dependency(
                            dep.id.to_string(),
                            DependencyEntryKind::TableDependencyGit,
                        )
                    }
                    "registry" => {
                        dep.registry = Some(Value::new(id.to_string(), s.value().to_string()));
                        EntryKind::Dependency(
                            dep.id.to_string(),
                            DependencyEntryKind::TableDependencyRegistry,
                        )
                    }
                    "package" => {
                        dep.package = Some(Value::new(id.to_string(), s.value().to_string()));
                        EntryKind::Dependency(
                            dep.id.to_string(),
                            DependencyEntryKind::TableDependencyPackage,
                        )
                    }
                    _ => {
                        //insert key
                        self.insert_key(
                            id,
                            table,
                            key,
                            KeyKind::Dependency(dep.id.to_string(), DependencyKeyKind::CrateName),
                        );
                        dep.version = Some(Value::new(id.to_string(), s.value().to_string()));
                        match dep.is_virtual {
                            true => EntryKind::Dependency(
                                dep.id.to_string(),
                                DependencyEntryKind::VirtualSimpleDependency,
                            ),
                            false => EntryKind::Dependency(
                                dep.id.to_string(),
                                DependencyEntryKind::SimpleDependency,
                            ),
                        }
                    }
                };
                self.insert_entry(id, node, table, entry_kind);
            }
            Node::Bool(b) => {
                let entry_kind = match key.value() {
                    "workspace" => {
                        let _ = self.insert_key(
                            id,
                            table,
                            key,
                            KeyKind::Dependency(dep.id.to_string(), DependencyKeyKind::Workspace),
                        );
                        dep.workspace = Some(Value::new(id.to_string(), b.value()));
                        EntryKind::Dependency(
                            dep.id.to_string(),
                            DependencyEntryKind::TableDependencyWorkspace,
                        )
                    }
                    "default-features" => EntryKind::Dependency(
                        dep.id.to_string(),
                        DependencyEntryKind::TableDependencyDefaultFeatures,
                    ),
                    "optional" => EntryKind::Dependency(
                        dep.id.to_string(),
                        DependencyEntryKind::TableDependencyOptional,
                    ),
                    _ => EntryKind::Dependency(
                        dep.id.to_string(),
                        DependencyEntryKind::TableDependencyUnknownBool,
                    ),
                };
                self.insert_entry(id, node, table, entry_kind);
            }
            _ => {}
        }
    }

    fn enter_generic(&mut self, id: &str, name: &str, table: CargoTable, node: &Node) {
        let range = self.mapper.range(join_ranges(node.text_ranges())).unwrap();
        let lsp_range = into_lsp_range(range);

        let text = serde_json::to_string(&node).unwrap_or_default();
        match node {
            Node::Table(t) => {
                self.entries_map.insert(
                    id.to_string(),
                    TomlNode::new_entry(
                        id.to_string(),
                        lsp_range,
                        text,
                        table,
                        EntryKind::Table(table),
                    ),
                );
                let entries = t.entries().read();
                for (key, entry) in entries.iter() {
                    let new_id = id.to_string() + "." + key.value();
                    self.enter_generic(&new_id, key.value(), table, entry);
                }
            }
            Node::Array(arr) => {
                self.entries_map.insert(
                    id.to_string(),
                    TomlNode::new_entry(
                        id.to_string(),
                        lsp_range,
                        text,
                        table,
                        EntryKind::Value(name.to_string()),
                    ),
                );
                let items = arr.items().read();
                for (i, c) in items.iter().enumerate() {
                    let new_id = id.to_string() + "." + &i.to_string();
                    self.enter_generic(&new_id, &i.to_string(), table, c);
                }
            }
            _ => {
                self.entries_map.insert(
                    id.to_string(),
                    TomlNode::new_entry(
                        id.to_string(),
                        lsp_range,
                        text,
                        table,
                        EntryKind::Value(name.to_string()),
                    ),
                );
            }
        }
    }
}

fn into_lsp_range(range: lsp_async_stub::util::Range) -> Range {
    Range {
        start: Position {
            line: range.start.line as u32,
            character: range.start.character as u32,
        },
        end: Position {
            line: range.end.line as u32,
            character: range.end.character as u32,
        },
    }
}

pub fn diff_dependency_entries(
    old_map: Option<&HashMap<String, TomlNode>>,
    new_map: &HashMap<String, TomlNode>,
) -> EntryDiff {
    let Some(old_map) = old_map else {
        return EntryDiff {
            created: new_map
                .iter()
                .filter(|(_, node)| node.is_top_level_dependency())
                .map(|(k, _)| k.to_string())
                .collect(),
            range_updated: vec![],
            value_updated: vec![],
            deleted: vec![],
        };
    };
    let old_keys: HashSet<_> = old_map
        .iter()
        .filter(|(_, node)| node.is_top_level_dependency())
        .map(|(k, _)| k.as_str())
        .collect();
    let new_keys: HashSet<_> = new_map
        .iter()
        .filter(|(_, node)| node.is_top_level_dependency())
        .map(|(k, _)| k.as_str())
        .collect();

    let created: Vec<String> = new_keys
        .difference(&old_keys)
        .map(|&s| s.to_string())
        .collect();
    let deleted: Vec<String> = old_keys
        .difference(&new_keys)
        .map(|&s| s.to_string())
        .collect();
    let field_updated: Vec<String> = old_keys
        .intersection(&new_keys)
        .filter(|&&key| {
            let old_node = &old_map[key];
            let new_node = &new_map[key];
            old_node.text != new_node.text
        })
        .map(|&s| s.to_string())
        .collect();
    let range_updated: Vec<String> = old_keys
        .intersection(&new_keys)
        .filter(|&&key| {
            let old_node = &old_map[key];
            let new_node = &new_map[key];
            old_node.range != new_node.range
        })
        .map(|&s| s.to_string())
        .collect();
    EntryDiff {
        created,
        range_updated,
        value_updated: field_updated,
        deleted,
    }
}
