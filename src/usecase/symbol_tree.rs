use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

use lsp_async_stub::util::Mapper;
use taplo::{dom::Node, util::join_ranges};
use tower_lsp::lsp_types::{Position, Range};

use crate::entity::{CargoKey, CargoNode, CargoTable, Dependency, DependencyTable, Value};

pub struct Walker {
    symbol_map: HashMap<String, CargoNode>,
    deps: Vec<Dependency>,
    mapper: Mapper,
}

impl Walker {
    pub fn consume(self) -> (HashMap<String, CargoNode>, Vec<Dependency>) {
        (self.symbol_map, self.deps)
    }

    pub fn new(text: &str, capacity: usize) -> Self {
        let mapper = Mapper::new_utf16(text, true);
        Self {
            symbol_map: HashMap::with_capacity(capacity),
            deps: Vec::with_capacity(capacity),
            mapper,
        }
    }

    pub fn walk_root(&mut self, id: &str, name: &str, node: &Node) {
        // let range = self.mapper.range(join_ranges(node.text_ranges())).unwrap();
        // let lsp_range = into_lsp_range(range);

        // let text = serde_json::to_string(&node).unwrap();
        match node {
            Node::Table(t) => {
                //the top level table node is not write into symbol map
                let parsed_table = CargoTable::from_str(name).unwrap();
                //TODO workspace dependencies
                match parsed_table {
                    //TODO parse package
                    CargoTable::Package => {}
                    CargoTable::Dependencie(dep_table) => {
                        let entries = t.entries().read();
                        for (key, entry) in entries.iter() {
                            let new_id = id.to_string() + "." + key.value();
                            let mut dep = Dependency::default();
                            //dependency
                            dep.id = new_id.to_string();
                            dep.name = key.value().to_string();
                            let range =
                                self.mapper.range(join_ranges(entry.text_ranges())).unwrap();
                            let lsp_range = into_lsp_range(range);
                            dep.range = lsp_range;
                            dep.table = dep_table;
                            self.enter_dependency(
                                &new_id,
                                key.value(),
                                parsed_table,
                                entry,
                                None,
                                &mut dep,
                            );
                            self.deps.push(dep);
                        }
                    }
                    CargoTable::Target => {
                        let entries = t.entries().read();
                        for (key, entry) in entries.iter() {
                            let new_id = id.to_string() + "." + key.value();
                            let mut dep = Dependency::default();
                            self.enter_dependency(
                                &new_id,
                                key.value(),
                                parsed_table,
                                entry,
                                None,
                                &mut dep,
                            );
                            self.deps.push(dep);
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
        name: &str,
        table: CargoTable,
        node: &Node,
        platform: Option<&str>,
        dep: &mut Dependency,
    ) {
        //target->platform
        if table == CargoTable::Target && platform.is_none() {
            //set platform
            dep.platform = Some(Value::new(id.to_string(), name.to_string()));
            if let Node::Table(t) = node {
                let entries = t.entries().read();
                for (key, entry) in entries.iter() {
                    let new_id = id.to_string() + "." + key.value();
                    self.enter_dependency(&new_id, key.value(), table, entry, Some(name), dep);
                }
            }
            return;
        }
        //target->platform->dependency
        if table == CargoTable::Target && platform.is_some() {
            let parsed_table = CargoTable::from_str(name).unwrap();
            if let Node::Table(t) = node {
                let entries = t.entries().read();
                for (key, entry) in entries.iter() {
                    let new_id = id.to_string() + "." + key.value();
                    dep.id = new_id.to_string();
                    dep.name = key.value().to_string();
                    let range = self.mapper.range(join_ranges(node.text_ranges())).unwrap();
                    let lsp_range = into_lsp_range(range);
                    dep.range = lsp_range;
                    self.enter_dependency(&new_id, key.value(), parsed_table, entry, platform, dep);
                }
            }
            return;
        }

        let range = self.mapper.range(join_ranges(node.text_ranges())).unwrap();
        let lsp_range = into_lsp_range(range);
        let text = serde_json::to_string(&node).unwrap();

        match node {
            //inline table dependency
            Node::Table(t) => {
                self.symbol_map.insert(
                    id.to_string(),
                    CargoNode {
                        id: id.to_string(),
                        range: lsp_range,
                        text,
                        table,
                        key: CargoKey::TableDependency(name.to_string()),
                    },
                );
                let entries = t.entries().read();
                for (key, entry) in entries.iter() {
                    let new_id = id.to_string() + "." + key.value();
                    self.enter_dependency(&new_id, key.value(), table, entry, platform, dep);
                }
            }
            //feature array
            Node::Array(arr) => {
                if name == "features" {
                    //feature array
                    self.symbol_map.insert(
                        id.to_string(),
                        CargoNode {
                            id: id.to_string(),
                            range: lsp_range,
                            text,
                            table,
                            key: CargoKey::TableDependencyFeatures,
                        },
                    );
                    let items = arr.items().read();
                    let mut features = Vec::with_capacity(items.len());
                    for (i, f) in items.iter().enumerate() {
                        let new_id = id.to_string() + "." + &i.to_string();
                        let range = self.mapper.range(join_ranges(f.text_ranges())).unwrap();
                        let lsp_range = into_lsp_range(range);
                        let text = serde_json::to_string(&node).unwrap();
                        if let Node::Str(s) = f {
                            features.push(Value::new(new_id.to_string(), s.value().to_string()));
                            self.symbol_map.insert(
                                new_id.to_string(),
                                CargoNode {
                                    id: new_id,
                                    range: lsp_range,
                                    text,
                                    table,
                                    key: CargoKey::TableDependencyFeature(name.to_string()),
                                },
                            );
                        } else {
                            unreachable!()
                        }
                    }
                    dep.features = Some(features);
                } else {
                    unreachable!()
                }
            }
            //simple dependency or table dependency string key value
            Node::Str(s) => {
                let key = match name {
                    "version" => {
                        dep.version = Some(Value::new(id.to_string(), s.value().to_string()));
                        CargoKey::TableDependencyVersion
                    }
                    "branch" => {
                        dep.branch = Some(Value::new(id.to_string(), s.value().to_string()));
                        CargoKey::TableDependencyBranch
                    }
                    "tag" => {
                        dep.tag = Some(Value::new(id.to_string(), s.value().to_string()));
                        CargoKey::TableDependencyTag
                    }
                    "path" => {
                        dep.path = Some(Value::new(id.to_string(), s.value().to_string()));
                        CargoKey::TableDependencyPath
                    }
                    "rev" => {
                        dep.rev = Some(Value::new(id.to_string(), s.value().to_string()));
                        CargoKey::TableDependencyRev
                    }
                    "git" => {
                        dep.git = Some(Value::new(id.to_string(), s.value().to_string()));
                        CargoKey::TableDependencyGit
                    }
                    "registry" => {
                        dep.registry = Some(Value::new(id.to_string(), s.value().to_string()));
                        CargoKey::TableDependencyRegistry
                    }
                    "package" => {
                        dep.package = Some(Value::new(id.to_string(), s.value().to_string()));
                        CargoKey::TableDependencyPackage
                    }
                    _ => {
                        dep.version = Some(Value::new(id.to_string(), s.value().to_string()));
                        CargoKey::SimpleDependency(name.to_string())
                    }
                };
                self.symbol_map.insert(
                    id.to_string(),
                    CargoNode {
                        id: id.to_string(),
                        range: lsp_range,
                        text,
                        table,
                        key,
                    },
                );
            }
            Node::Bool(b) => {
                let key = match name {
                    "workspace" => {
                        dep.workspace = Some(Value::new(id.to_string(), b.value()));
                        CargoKey::TableDependencyWorkspace
                    }
                    "default-features" => CargoKey::TableDependencyDefaultFeatures,
                    "optional" => CargoKey::TableDependencyOptional,
                    _ => CargoKey::TableDependencyUnknownBool,
                };
                self.symbol_map.insert(
                    id.to_string(),
                    CargoNode {
                        id: id.to_string(),
                        range: lsp_range,
                        text,
                        table,
                        key,
                    },
                );
            }
            _ => unreachable!(),
        }
    }

    fn enter_generic(&mut self, id: &str, name: &str, table: CargoTable, node: &Node) {
        let range = self.mapper.range(join_ranges(node.text_ranges())).unwrap();
        let lsp_range = into_lsp_range(range);

        let text = serde_json::to_string(&node).unwrap();
        match node {
            Node::Table(t) => {
                self.symbol_map.insert(
                    id.to_string(),
                    CargoNode {
                        id: id.to_string(),
                        range: lsp_range,
                        text,
                        table,
                        key: CargoKey::Table(table),
                    },
                );
                let entries = t.entries().read();
                for (key, entry) in entries.iter() {
                    let new_id = id.to_string() + "." + key.value();
                    self.enter_generic(&new_id, key.value(), table, entry);
                }
            }
            Node::Array(arr) => {
                self.symbol_map.insert(
                    id.to_string(),
                    CargoNode {
                        id: id.to_string(),
                        range: lsp_range,
                        text,
                        table,
                        key: CargoKey::Key(name.to_string()),
                    },
                );
                let items = arr.items().read();
                for (i, c) in items.iter().enumerate() {
                    let new_id = id.to_string() + "." + &i.to_string();
                    self.enter_generic(&new_id, &i.to_string(), table, c);
                }
            }
            _ => {
                self.symbol_map.insert(
                    id.to_string(),
                    CargoNode {
                        id: id.to_string(),
                        range: lsp_range,
                        text,
                        table,
                        key: CargoKey::Key(name.to_string()),
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

//now only diff dependency nodes
pub fn diff_symbol_maps(
    old_map: &HashMap<String, CargoNode>,
    new_map: &HashMap<String, CargoNode>,
    rev: usize,
    dirty_nodes: &mut HashMap<String, usize>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let old_keys: HashSet<_> = old_map
        .iter()
        .filter(|(_, node)| {
            matches!(
                node.key,
                CargoKey::SimpleDependency(_) | CargoKey::TableDependency(_)
            )
        })
        .map(|(k, _)| k.clone())
        .collect();
    let new_keys: HashSet<_> = new_map
        .iter()
        .filter(|(_, node)| {
            matches!(
                node.key,
                CargoKey::SimpleDependency(_) | CargoKey::TableDependency(_)
            )
        })
        .map(|(k, _)| k.clone())
        .collect();

    let created: Vec<String> = new_keys.difference(&old_keys).cloned().collect();
    let deleted: Vec<String> = old_keys.difference(&new_keys).cloned().collect();
    let changed: Vec<String> = old_keys
        .intersection(&new_keys)
        .filter(|&key| {
            let old_node = &old_map[key];
            let new_node = &new_map[key];
            old_node.range != new_node.range || old_node.text != new_node.text
        })
        .cloned()
        .collect();

    // Update the dirty_nodes map
    for id in created.iter().chain(changed.iter()) {
        dirty_nodes.insert(id.clone(), rev);
    }

    (created, changed, deleted)
}
