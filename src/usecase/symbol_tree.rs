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
    strip_quote, validate_crate_name, validate_feature_name, validate_profile_name, CargoTable,
    Dependency, DependencyEntryKind, DependencyKeyKind, DependencyTable, EntryDiff, EntryKind,
    KeyKind, Manifest, SymbolTree, TomlEntry, TomlKey, TomlParsingError, Value,
};

pub struct Walker {
    keys_map: HashMap<String, TomlKey>,
    entries_map: HashMap<String, TomlEntry>,
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
        HashMap<String, Dependency>,
        Vec<TomlParsingError>,
    ) {
        (
            SymbolTree {
                keys: self.keys_map,
                entries: self.entries_map,
            },
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
                            self.enter_dependency(
                                &new_id,
                                key,
                                key.value(),
                                parsed_table,
                                entry,
                                &mut dep,
                            );
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
                                            let name = dep_name.value();
                                            let new_id = id.to_string()
                                                + "."
                                                + platform
                                                + "."
                                                + key.value()
                                                + "."
                                                + name;

                                            //insert dep
                                            let mut dep = Dependency {
                                                id: new_id.to_string(),
                                                name: name.to_string(),
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
                                                name,
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

    fn enter_dependency(
        &mut self,
        id: &str,
        key: &Key,
        name: &str,
        table: CargoTable,
        node: &Node,
        dep: &mut Dependency,
    ) {
        let range = self.mapper.range(join_ranges(node.text_ranges())).unwrap();
        let lsp_range = into_lsp_range(range);
        let text = serde_json::to_string(&node).unwrap_or_default();

        match node {
            //invalid node
            Node::Invalid(_) => {
                //insert key
                let key_id = id.to_string() + ".key";

                let v = key.value();
                let key_range =
                    into_lsp_range(self.mapper.range(join_ranges(key.text_ranges())).unwrap());

                if let Err(e) = validate_crate_name(v) {
                    self.errs
                        .push(TomlParsingError::new(key_id.to_string(), e, key_range));
                }

                self.keys_map.insert(
                    key_id.to_string(),
                    TomlKey {
                        id: key_id,
                        range: key_range,
                        text: v.to_string(),
                        table,
                        kind: KeyKind::Dependency(DependencyKeyKind::CrateName),
                    },
                );
            }
            //inline table dependency
            Node::Table(t) => {
                //insert key
                let key_id = id.to_string() + ".key";

                let v = key.value();
                let key_range =
                    into_lsp_range(self.mapper.range(join_ranges(key.text_ranges())).unwrap());
                if let Err(e) = validate_crate_name(v) {
                    self.errs
                        .push(TomlParsingError::new(key_id.to_string(), e, key_range));
                }
                self.keys_map.insert(
                    key_id.to_string(),
                    TomlKey {
                        id: key_id,
                        range: key_range,
                        text: v.to_string(),
                        table,
                        kind: KeyKind::Dependency(DependencyKeyKind::CrateName),
                    },
                );
                self.entries_map.insert(
                    id.to_string(),
                    TomlEntry {
                        id: id.to_string(),
                        range: lsp_range,
                        text,
                        table,
                        kind: EntryKind::Dependency(
                            dep.id.to_string(),
                            DependencyEntryKind::TableDependency,
                        ),
                    },
                );
                let entries = t.entries().read();
                for (key, entry) in entries.iter() {
                    let new_id = id.to_string() + "." + key.value();
                    self.enter_dependency(&new_id, key, key.value(), table, entry, dep);
                }
            }
            //feature array
            Node::Array(arr) => {
                if name == "features" {
                    //feature array
                    self.entries_map.insert(
                        id.to_string(),
                        TomlEntry {
                            id: id.to_string(),
                            range: lsp_range,
                            text,
                            table,
                            kind: EntryKind::Dependency(
                                dep.id.to_string(),
                                DependencyEntryKind::TableDependencyFeatures,
                            ),
                        },
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
                                TomlEntry {
                                    id: new_id,
                                    range: lsp_range,
                                    text: s.value().to_string(),
                                    table,
                                    kind: EntryKind::Dependency(
                                        dep.id.to_string(),
                                        DependencyEntryKind::TableDependencyFeature,
                                    ),
                                },
                            );
                        }
                    }
                    dep.features = Some(features);
                }
            }
            //simple dependency or table dependency string key value
            Node::Str(s) => {
                let key = match name {
                    "version" => {
                        dep.version = Some(Value::new(id.to_string(), s.value().to_string()));
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
                        let key_id = id.to_string() + ".key";
                        self.keys_map.insert(
                            key_id.to_string(),
                            TomlKey {
                                id: key_id,
                                range: into_lsp_range(
                                    self.mapper.range(join_ranges(key.text_ranges())).unwrap(),
                                ),
                                text: key.value().to_string(),
                                table,
                                kind: KeyKind::Dependency(DependencyKeyKind::CrateName),
                            },
                        );
                        dep.version = Some(Value::new(id.to_string(), s.value().to_string()));
                        EntryKind::Dependency(
                            dep.id.to_string(),
                            DependencyEntryKind::SimpleDependency,
                        )
                    }
                };
                self.entries_map.insert(
                    id.to_string(),
                    TomlEntry {
                        id: id.to_string(),
                        range: lsp_range,
                        text: strip_quote(text),
                        table,
                        kind: key,
                    },
                );
            }
            Node::Bool(b) => {
                let key = match name {
                    "workspace" => {
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
                self.entries_map.insert(
                    id.to_string(),
                    TomlEntry {
                        id: id.to_string(),
                        range: lsp_range,
                        text: strip_quote(text),
                        table,
                        kind: key,
                    },
                );
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
                    TomlEntry {
                        id: id.to_string(),
                        range: lsp_range,
                        text: strip_quote(text),
                        table,
                        kind: EntryKind::Table(table),
                    },
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
                    TomlEntry {
                        id: id.to_string(),
                        range: lsp_range,
                        text: strip_quote(text),
                        table,
                        kind: EntryKind::Value(name.to_string()),
                    },
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
                    TomlEntry {
                        id: id.to_string(),
                        range: lsp_range,
                        text: strip_quote(text),
                        table,
                        kind: EntryKind::Value(name.to_string()),
                    },
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
    old_map: Option<&HashMap<String, TomlEntry>>,
    new_map: &HashMap<String, TomlEntry>,
) -> EntryDiff {
    let Some(old_map) = old_map else {
        return EntryDiff {
            created: new_map
                .iter()
                .filter(|(_, node)| node.kind.is_dependency())
                .map(|(k, _)| k.to_string())
                .collect(),
            range_updated: vec![],
            value_updated: vec![],
            deleted: vec![],
        };
    };
    let old_keys: HashSet<_> = old_map
        .iter()
        .filter(|(_, node)| node.kind.is_dependency())
        .map(|(k, _)| k.as_str())
        .collect();
    let new_keys: HashSet<_> = new_map
        .iter()
        .filter(|(_, node)| node.kind.is_dependency())
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
