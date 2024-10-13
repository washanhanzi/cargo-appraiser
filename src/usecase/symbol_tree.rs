use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

use lsp_async_stub::util::Mapper;
use taplo::{dom::Node, util::join_ranges};
use tower_lsp::lsp_types::{Position, Range};

use crate::entity::{
    CargoKey, CargoNode, CargoTable, Dependency, DependencyKey, InvalidDependencyKey, InvalideKey,
    SymbolDiff, Value,
};

pub struct Walker {
    symbol_map: HashMap<String, CargoNode>,
    deps: HashMap<String, Dependency>,
    mapper: Mapper,
}

impl Walker {
    pub fn consume(self) -> (HashMap<String, CargoNode>, HashMap<String, Dependency>) {
        (self.symbol_map, self.deps)
    }

    pub fn new(text: &str, capacity: usize) -> Self {
        let mapper = Mapper::new_utf16(text, false);
        Self {
            symbol_map: HashMap::with_capacity(capacity),
            deps: HashMap::with_capacity(capacity),
            mapper,
        }
    }

    pub fn walk_root(&mut self, id: &str, name: &str, node: &Node) {
        match node {
            Node::Table(t) => {
                let parsed_table = CargoTable::from_str(name).unwrap();
                match parsed_table {
                    CargoTable::Package => {}
                    CargoTable::Dependencies(dep_table) => {
                        let entries = t.entries().read();
                        for (key, entry) in entries.iter() {
                            let new_id = id.to_string() + "." + key.value();
                            let mut dep = Dependency {
                                id: new_id.clone(),
                                ..Default::default()
                            };
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
                            self.deps.insert(new_id, dep);
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
                            self.deps.insert(new_id, dep);
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
        let text = serde_json::to_string(&node).unwrap_or_default();

        match node {
            //invalid node
            Node::Invalid(_) => {
                self.symbol_map.insert(
                    id.to_string(),
                    CargoNode {
                        id: id.to_string(),
                        range: lsp_range,
                        text: name.to_string(),
                        table,
                        key: CargoKey::Invalid(InvalideKey::Dependency(
                            InvalidDependencyKey::CrateName,
                        )),
                    },
                );
            }
            //inline table dependency
            Node::Table(t) => {
                self.symbol_map.insert(
                    id.to_string(),
                    CargoNode {
                        id: id.to_string(),
                        range: lsp_range,
                        text,
                        table,
                        key: CargoKey::Dpendency(
                            dep.id.to_string(),
                            DependencyKey::TableDependency,
                        ),
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
                            key: CargoKey::Dpendency(
                                dep.id.to_string(),
                                DependencyKey::TableDependencyFeatures,
                            ),
                        },
                    );
                    let items = arr.items().read();
                    let mut features = Vec::with_capacity(items.len());
                    for (i, f) in items.iter().enumerate() {
                        let new_id = id.to_string() + "." + &i.to_string();
                        let range = self.mapper.range(join_ranges(f.text_ranges())).unwrap();
                        let lsp_range = into_lsp_range(range);
                        let text = serde_json::to_string(f).unwrap();
                        if let Node::Str(s) = f {
                            features.push(Value::new(new_id.to_string(), s.value().to_string()));
                            self.symbol_map.insert(
                                new_id.to_string(),
                                CargoNode {
                                    id: new_id,
                                    range: lsp_range,
                                    text: strip_quote(text),
                                    table,
                                    key: CargoKey::Dpendency(
                                        dep.id.to_string(),
                                        DependencyKey::TableDependencyFeature,
                                    ),
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
                        CargoKey::Dpendency(
                            dep.id.to_string(),
                            DependencyKey::TableDependencyVersion,
                        )
                    }
                    "branch" => {
                        dep.branch = Some(Value::new(id.to_string(), s.value().to_string()));
                        CargoKey::Dpendency(
                            dep.id.to_string(),
                            DependencyKey::TableDependencyBranch,
                        )
                    }
                    "tag" => {
                        dep.tag = Some(Value::new(id.to_string(), s.value().to_string()));
                        CargoKey::Dpendency(dep.id.to_string(), DependencyKey::TableDependencyTag)
                    }
                    "path" => {
                        dep.path = Some(Value::new(id.to_string(), s.value().to_string()));
                        CargoKey::Dpendency(dep.id.to_string(), DependencyKey::TableDependencyPath)
                    }
                    "rev" => {
                        dep.rev = Some(Value::new(id.to_string(), s.value().to_string()));
                        CargoKey::Dpendency(dep.id.to_string(), DependencyKey::TableDependencyRev)
                    }
                    "git" => {
                        dep.git = Some(Value::new(id.to_string(), s.value().to_string()));
                        CargoKey::Dpendency(dep.id.to_string(), DependencyKey::TableDependencyGit)
                    }
                    "registry" => {
                        dep.registry = Some(Value::new(id.to_string(), s.value().to_string()));
                        CargoKey::Dpendency(
                            dep.id.to_string(),
                            DependencyKey::TableDependencyRegistry,
                        )
                    }
                    "package" => {
                        dep.package = Some(Value::new(id.to_string(), s.value().to_string()));
                        CargoKey::Dpendency(
                            dep.id.to_string(),
                            DependencyKey::TableDependencyPackage,
                        )
                    }
                    _ => {
                        dep.version = Some(Value::new(id.to_string(), s.value().to_string()));
                        CargoKey::Dpendency(dep.id.to_string(), DependencyKey::SimpleDependency)
                    }
                };
                self.symbol_map.insert(
                    id.to_string(),
                    CargoNode {
                        id: id.to_string(),
                        range: lsp_range,
                        text: strip_quote(text),
                        table,
                        key,
                    },
                );
            }
            Node::Bool(b) => {
                let key = match name {
                    "workspace" => {
                        dep.workspace = Some(Value::new(id.to_string(), b.value()));
                        CargoKey::Dpendency(
                            dep.id.to_string(),
                            DependencyKey::TableDependencyWorkspace,
                        )
                    }
                    "default-features" => CargoKey::Dpendency(
                        dep.id.to_string(),
                        DependencyKey::TableDependencyDefaultFeatures,
                    ),
                    "optional" => CargoKey::Dpendency(
                        dep.id.to_string(),
                        DependencyKey::TableDependencyOptional,
                    ),
                    _ => CargoKey::Dpendency(
                        dep.id.to_string(),
                        DependencyKey::TableDependencyUnknownBool,
                    ),
                };
                self.symbol_map.insert(
                    id.to_string(),
                    CargoNode {
                        id: id.to_string(),
                        range: lsp_range,
                        text: strip_quote(text),
                        table,
                        key,
                    },
                );
            }
            _ => {}
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
                        text: strip_quote(text),
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
                        text: strip_quote(text),
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
                        text: strip_quote(text),
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
pub fn diff_symbols(
    old_map: Option<&HashMap<String, CargoNode>>,
    new_map: &HashMap<String, CargoNode>,
) -> SymbolDiff {
    let Some(old_map) = old_map else {
        return SymbolDiff {
            created: new_map
                .iter()
                .filter(|(_, node)| node.key.is_dependency())
                .map(|(k, _)| k.to_string())
                .collect(),
            range_updated: vec![],
            value_updated: vec![],
            deleted: vec![],
        };
    };
    let old_keys: HashSet<_> = old_map
        .iter()
        .filter(|(_, node)| node.key.is_dependency())
        .map(|(k, _)| k.as_str())
        .collect();
    let new_keys: HashSet<_> = new_map
        .iter()
        .filter(|(_, node)| node.key.is_dependency())
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
    let range_updated: Vec<String> = old_keys
        .intersection(&new_keys)
        .filter(|&&key| {
            let old_node = &old_map[key];
            let new_node = &new_map[key];
            old_node.range != new_node.range
        })
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

    SymbolDiff {
        created,
        range_updated,
        value_updated: field_updated,
        deleted,
    }
}

fn strip_quote(s: String) -> String {
    if s.starts_with('"') && s.ends_with('"') {
        return s[1..s.len() - 1].to_string();
    }
    s
}
