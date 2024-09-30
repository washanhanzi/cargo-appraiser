use std::{collections::HashMap, path::Path};

use cargo::util::VersionExt;
use semver::Version;
use taplo::dom::{
    node::{DomNode, Invalid},
    Node,
};
use tokio::sync::{
    mpsc::{self, Sender},
    oneshot,
};
use tower_lsp::{
    lsp_types::{Hover, Position, Range},
    Client,
};

use crate::{
    decoration::DecorationEvent,
    entity::cargo_dependency_to_toml_key,
    usecase::{diff_symbol_maps, Walker},
};

use super::{
    cargo::{parse_cargo_output, CargoResolveOutput},
    document_state,
    hover::hover,
};

#[derive(Debug, Clone)]
pub struct Ctx {
    pub path: String,
    pub rev: usize,
}

//CargoState will run a dedicate task which receive msg from lsp event
//the msg payload should contain the file content and lsp client
//track current opened cargo.toml file and rev
#[derive(Debug)]
pub struct Appraiser {
    client: Client,
    render_tx: Sender<DecorationEvent>,
}

//TODO audit: cargo.toml and cargo.lock always stay together, if we get the full dep tree from cargo tree
//and then we can use cargo audit to show diagnostic for a dep
pub enum CargoDocumentEvent {
    //cargo.toml save event
    //start to parse the document, update the state, and send event for cargo_tree task
    Opened(CargoTomlPayload),
    Saved(CargoTomlPayload),
    Changed(String),
    //reset state, String is path
    Closed(String),
    //result from cargo command
    //consolidate state and send render event
    CargoResolved(CargoResolveOutput),
    //cargo.lock change
    //CargoLockCreated,
    CargoLockChanged,
    //code action, path and range
    CodeAction(String, Range),
    //hover event, path and position
    Hovered(String, Position, oneshot::Sender<Hover>),
}

pub struct CargoTomlPayload {
    pub path: String,
    pub text: String,
}

impl Appraiser {
    pub fn new(client: Client, render_tx: Sender<DecorationEvent>) -> Self {
        Self { client, render_tx }
    }
    pub fn initialize(&self) -> Sender<CargoDocumentEvent> {
        //create mpsc channel
        let (tx, mut rx) = mpsc::channel::<CargoDocumentEvent>(64);

        //cargo tree task
        //cargo tree channel
        let (cargo_tx, mut cargo_rx) = mpsc::channel::<Ctx>(32);
        let tx_for_cargo = tx.clone();
        tokio::spawn(async move {
            while let Some(event) = cargo_rx.recv().await {
                let output = parse_cargo_output(&event).await;
                if let Err(e) = tx_for_cargo
                    .send(CargoDocumentEvent::CargoResolved(output))
                    .await
                {
                    eprintln!("cargo resolved tx error: {}", e);
                }
            }
        });

        //main loop
        //render task sender
        let render_tx = self.render_tx.clone();
        tokio::spawn(async move {
            //state
            let mut state = document_state::DocumentState::new();
            //symbol_map store the ui representation of the Cargo.toml file, this is a snapshot of latest save
            // let mut symbol_map: HashMap<String, CargoNode> = HashMap::new();
            // let mut reverse_map: ReverseSymbolTree = ReverseSymbolTree::new(&symbol_map);
            // //dependencies only store the dependencies in Cargo.toml, this is a snapshot of latest save
            // let mut dependencies: Vec<Dependency> = Vec::new();
            // //the dirty nodes of latest save
            // let mut dirty_nodes: HashMap<String, usize> = HashMap::new();

            while let Some(event) = rx.recv().await {
                match event {
                    CargoDocumentEvent::Hovered(req_path, pos, tx) => {
                        let Some((symbol_map, reverse_map, dependencies)) = state.state(&req_path)
                        else {
                            continue;
                        };
                        let Some(node) = reverse_map.precise_match(pos, symbol_map) else {
                            continue;
                        };
                        let Some(dep) = dependencies.iter().find(|dep| dep.id == node.key.row_id())
                        else {
                            continue;
                        };
                        let Some(h) = hover(node, dep) else {
                            continue;
                        };
                        tx.send(h).unwrap()
                    }
                    CargoDocumentEvent::Closed(req_path) => {
                        state.close(&req_path);
                    }
                    CargoDocumentEvent::CargoLockChanged => {
                        state.clear();
                        render_tx.send(DecorationEvent::Reset).await.unwrap();
                    }
                    CargoDocumentEvent::Changed(text) => {
                        let p = taplo::parser::parse(&text);
                        let dom = p.into_dom();
                        if dom.validate().is_err() {
                            eprintln!("changed semantic Error: {:?}", dom.errors());
                        }
                        let table = dom.as_table().unwrap();
                        let entries = table.entries().read();
                        let mut walker = Walker::new(&text, entries.len());

                        for (key, entry) in entries.iter() {
                            if key.value().is_empty() {
                                continue;
                            }
                            walker.walk_root(key.value(), key.value(), entry)
                        }
                    }
                    CargoDocumentEvent::Opened(msg) | CargoDocumentEvent::Saved(msg) => {
                        let rev = state.inc_rev(&msg.path);
                        let (symbol_map, reverse_map, dirty_nodes, dependencies) =
                            state.state_mut(&msg.path);

                        let (new_symbol_map, new_deps) = {
                            //parse cargo.toml text
                            //I'm too stupid to apprehend the rowan tree
                            //else I would use incremental patching
                            //This's a dumb full parsing
                            let p = taplo::parser::parse(&msg.text);
                            if !p.errors.is_empty() {
                                continue;
                            }
                            let dom = p.into_dom();
                            if dom.validate().is_err() {
                                continue;
                            }
                            let table = dom.as_table().unwrap();
                            let entries = table.entries().read();

                            let mut walker = Walker::new(&msg.text, entries.len());

                            for (key, entry) in entries.iter() {
                                if key.value().is_empty() {
                                    continue;
                                }
                                walker.walk_root(key.value(), key.value(), entry)
                            }

                            //get dependencies
                            let path = Path::new(&msg.path);
                            let gctx = cargo::util::context::GlobalContext::default().unwrap();
                            //TODO ERROR parse manifest
                            let workspace = cargo::core::Workspace::new(path, &gctx).unwrap();
                            //TODO if it's error, it's a virtual workspace
                            let current = workspace.current().unwrap();
                            let mut unresolved = HashMap::new();
                            for dep in current.dependencies() {
                                let key = cargo_dependency_to_toml_key(dep);
                                unresolved.insert(key, dep);
                            }

                            let (new_symbol_map, mut new_deps) = walker.consume();

                            //loop new_deps, get unresolved
                            for dep in &mut new_deps {
                                let key = dep.toml_key();
                                if unresolved.contains_key(&key) {
                                    //take out value from unresolved
                                    let u = unresolved.remove(&key).unwrap();
                                    //update value to dep.unresolved
                                    dep.unresolved = Some(u.clone());
                                }
                            }

                            (new_symbol_map, new_deps)
                            //reconsile dependencies
                        }; // This block ensures taplo::dom objects are dropped before the await point

                        //diff
                        //diff walker.symbol_map with latest saved symbol_map
                        //if symbol_map is empty, then every nodes is created
                        //diff compare range and text equablity
                        //diff result contains:
                        //created, changed, deleted nodes
                        //dirty nodes includes created, changed nodes
                        let (created, changed, deleted) =
                            diff_symbol_maps(symbol_map, &new_symbol_map, rev, dirty_nodes);

                        //override old symbol map
                        *symbol_map = new_symbol_map;
                        //generate reverse symbol tree
                        reverse_map.init(symbol_map);

                        // Loop through both created and changed nodes
                        for v in created.iter().chain(changed.iter()) {
                            // Send to a dedicated render task
                            if let Some(n) = symbol_map.get(v) {
                                render_tx
                                    .send(DecorationEvent::DependencyLoading(
                                        msg.path.to_string(),
                                        v.to_string(),
                                        n.range,
                                    ))
                                    .await
                                    .unwrap();
                            }
                        }

                        for v in deleted {
                            //send to a dedicate render task
                            render_tx
                                .send(DecorationEvent::DependencyRemove(
                                    msg.path.to_string(),
                                    v.to_string(),
                                ))
                                .await
                                .unwrap();
                        }

                        //override old deps
                        *dependencies = new_deps;

                        //no change to resolve
                        if dirty_nodes.is_empty() {
                            continue;
                        }

                        //resolve cargo dependencies in another task
                        cargo_tx
                            .send(Ctx {
                                path: msg.path.to_string(),
                                rev,
                            })
                            .await
                            .unwrap();
                    }
                    CargoDocumentEvent::CargoResolved(mut output) => {
                        //compare path and rev
                        if !state.check(&output.ctx.path, output.ctx.rev) {
                            continue;
                        }
                        let (_, _, dirty_nodes, dependencies) = state.state_mut(&output.ctx.path);
                        //populate deps
                        for dep in dependencies {
                            let key = dep.toml_key();
                            if output.dependencies.is_empty()
                                || !output.dependencies.contains_key(&key)
                            {
                                continue;
                            }
                            // Take resolved out of the output.dependencies hashmap
                            let resolved = output.dependencies.remove(&key).unwrap();
                            dep.resolved = Some(resolved);

                            let package_name = dep.package_name();
                            if !output.summaries.contains_key(package_name) {
                                continue;
                            }
                            let summaries = output.summaries.get(package_name).unwrap();
                            dep.summaries = Some(summaries.clone());

                            //if not installed, we don't need to know the latest version
                            if dep.resolved.is_none() {
                                continue;
                            }

                            let installed = dep.resolved.as_ref().unwrap().version.clone();
                            let req_version = dep.unresolved.as_ref().unwrap().version_req();

                            let mut latest: Option<&Version> = None;
                            let mut latest_matched: Option<&Version> = None;
                            for summary in summaries {
                                if &installed == summary.version() {
                                    dep.matched_summary = Some(summary.clone());
                                }
                                match latest {
                                    Some(cur) if summary.version() > cur => {
                                        latest = Some(summary.version());
                                        dep.latest_summary = Some(summary.clone());
                                    }
                                    None => {
                                        latest = Some(summary.version());
                                        dep.latest_summary = Some(summary.clone());
                                    }
                                    _ => {}
                                }
                                match (latest_matched, installed.is_prerelease()) {
                                    (Some(cur), true)
                                        if req_version.matches_prerelease(summary.version())
                                            && summary.version() > cur =>
                                    {
                                        latest_matched = Some(summary.version());
                                        dep.latest_matched_summary = Some(summary.clone());
                                    }
                                    (Some(cur), false)
                                        if req_version.matches(summary.version())
                                            && summary.version() > cur =>
                                    {
                                        latest_matched = Some(summary.version());
                                        dep.latest_matched_summary = Some(summary.clone());
                                    }
                                    (None, true)
                                        if req_version.matches_prerelease(summary.version()) =>
                                    {
                                        latest_matched = Some(summary.version());
                                        dep.latest_matched_summary = Some(summary.clone());
                                    }
                                    (None, false) if req_version.matches(summary.version()) => {
                                        latest_matched = Some(summary.version());
                                        dep.latest_matched_summary = Some(summary.clone());
                                    }
                                    _ => {}
                                }
                            }

                            //send to render
                            if let Some(rev) = dirty_nodes.get(&dep.id) {
                                if *rev > output.ctx.rev {
                                    continue;
                                }
                                //send to render task
                                render_tx
                                    .send(DecorationEvent::Dependency(
                                        output.ctx.path.to_string(),
                                        dep.id.clone(),
                                        dep.range,
                                        dep.clone(),
                                    ))
                                    .await
                                    .unwrap();
                                dirty_nodes.remove(&dep.id);
                            }
                        }
                    }
                    _ => {}
                }
            }
        });
        tx
    }
}
