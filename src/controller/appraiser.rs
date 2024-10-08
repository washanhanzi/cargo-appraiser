use std::collections::HashMap;

use cargo::util::VersionExt;
use semver::Version;
use taplo::dom::node::DomNode;
use tokio::sync::{
    mpsc::{self, Sender},
    oneshot,
};
use tower_lsp::{
    lsp_types::{CodeActionResponse, Hover, Position, Range, Url},
    Client,
};

use crate::{
    controller::code_action::code_action,
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
    pub uri: Url,
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
    Closed(Url),
    //result from cargo command
    //consolidate state and send render event
    CargoResolved(CargoResolveOutput),
    //cargo.lock change
    //CargoLockCreated,
    CargoLockChanged,
    //code action, path and range
    CodeAction(Url, Range, oneshot::Sender<CodeActionResponse>),
    //hover event, path and position
    Hovered(Url, Position, oneshot::Sender<Hover>),
}

pub struct CargoTomlPayload {
    pub uri: Url,
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
                    CargoDocumentEvent::Hovered(uri, pos, tx) => {
                        let Some((symbol_map, reverse_map, dependencies)) = state.state(&uri)
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
                        let _ = tx.send(h);
                    }
                    CargoDocumentEvent::CodeAction(uri, range, tx) => {
                        let Some((symbol_map, reverse_map, dependencies)) = state.state(&uri)
                        else {
                            continue;
                        };
                        let Some(node) = reverse_map.precise_match(range.start, symbol_map) else {
                            continue;
                        };
                        let Some(dep) = dependencies.iter().find(|dep| dep.id == node.key.row_id())
                        else {
                            continue;
                        };
                        let Some(action) = code_action(uri, node, dep) else {
                            continue;
                        };
                        let _ = tx.send(action);
                    }
                    CargoDocumentEvent::Closed(uri) => {
                        state.close(&uri);
                    }
                    CargoDocumentEvent::CargoLockChanged => {
                        //clear state except the "current" uri
                        let Some((uri, rev)) = state.clear_except_current() else {
                            continue;
                        };
                        if let Err(e) = cargo_tx.send(Ctx { uri, rev }).await {
                            eprintln!("cargo lock changed tx error: {}", e);
                        }
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
                        let rev = state.inc_rev(&msg.uri);
                        let (symbol_map, reverse_map, dirty_nodes, dependencies) =
                            state.state_mut(&msg.uri);
                        let Ok(path) = msg.uri.to_file_path() else {
                            continue;
                        };

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
                            let gctx = cargo::util::context::GlobalContext::default().unwrap();
                            //TODO ERROR parse manifest
                            let workspace =
                                cargo::core::Workspace::new(path.as_path(), &gctx).unwrap();
                            //TODO if it's error, it's a virtual workspace
                            let current = workspace.current().unwrap();
                            let mut unresolved = HashMap::new();
                            for dep in current.dependencies() {
                                let key = cargo_dependency_to_toml_key(dep);
                                unresolved.insert(key, dep);
                            }

                            let (new_symbol_map, mut new_deps) = walker.consume();

                            //loop new_deps, get unresolved
                            for (_, dep) in &mut new_deps {
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
                        //if symbol_map is empty, then every nodes is newly created
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
                        for v in &created {
                            dependencies.push(new_deps.get(v).unwrap().clone());
                            // Send to a dedicated render task
                            if let Some(n) = symbol_map.get(v) {
                                render_tx
                                    .send(DecorationEvent::DependencyLoading(
                                        msg.uri.clone(),
                                        v.to_string(),
                                        n.range,
                                    ))
                                    .await
                                    .unwrap();
                            }
                        }

                        for v in &changed {
                            //find dep in dependencies with same id and replace it
                            for dep in dependencies.iter_mut() {
                                if dep.id == v.as_str() {
                                    if let Some(new_dep) = new_deps.get(v) {
                                        *dep = new_dep.clone();
                                    }
                                }
                            }
                            // Send to a dedicated render task
                            if let Some(n) = symbol_map.get(v) {
                                render_tx
                                    .send(DecorationEvent::DependencyLoading(
                                        msg.uri.clone(),
                                        v.to_string(),
                                        n.range,
                                    ))
                                    .await
                                    .unwrap();
                            }
                        }

                        for v in deleted {
                            //inplace mutate dependencies
                            dependencies.retain(|dep| dep.id != v);
                            //send to a dedicate render task
                            render_tx
                                .send(DecorationEvent::DependencyRemove(
                                    msg.uri.clone(),
                                    v.to_string(),
                                ))
                                .await
                                .unwrap();
                        }

                        //no change to resolve
                        if dirty_nodes.is_empty() {
                            continue;
                        }

                        //override old deps
                        //or better we only override the changed deps
                        // *dependencies = new_deps;

                        //resolve cargo dependencies in another task
                        cargo_tx
                            .send(Ctx {
                                uri: msg.uri.clone(),
                                rev,
                            })
                            .await
                            .unwrap();
                    }
                    CargoDocumentEvent::CargoResolved(mut output) => {
                        //compare path and rev
                        if !state.check(&output.ctx.uri, output.ctx.rev) {
                            continue;
                        }
                        let (_, _, dirty_nodes, dependencies) = state.state_mut(&output.ctx.uri);
                        //populate deps
                        for dep in dependencies {
                            let key = dep.toml_key();
                            if !output.dependencies.is_empty()
                                && output.dependencies.contains_key(&key)
                            {
                                // Take resolved out of the output.dependencies hashmap
                                let resolved = output.dependencies.remove(&key).unwrap();
                                dep.resolved = Some(resolved);

                                let package_name = dep.package_name();
                                if !output.summaries.contains_key(package_name) {
                                    continue;
                                }
                                let summaries = output.summaries.get(package_name).unwrap();
                                dep.summaries = Some(summaries.clone());

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
                                            if req_version
                                                .matches_prerelease(summary.version())
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
                                            if req_version
                                                .matches_prerelease(summary.version()) =>
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
                            }

                            //send to render
                            if let Some(rev) = dirty_nodes.get(&dep.id) {
                                if *rev > output.ctx.rev {
                                    continue;
                                }
                                //send to render task
                                render_tx
                                    .send(DecorationEvent::Dependency(
                                        output.ctx.uri.clone(),
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
