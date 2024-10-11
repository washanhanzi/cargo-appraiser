use cargo::util::VersionExt;
use semver::Version;
use tokio::sync::{
    mpsc::{self, Sender},
    oneshot,
};
use tower_lsp::{
    lsp_types::{CodeActionResponse, Hover, Position, Range, Url},
    Client,
};

use crate::{
    controller::code_action::code_action, decoration::DecorationEvent, usecase::Workspace,
};

use super::{
    cargo::{parse_cargo_output, CargoResolveOutput},
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
    Changed(CargoTomlPayload),
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
            let mut state = Workspace::new();
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
                        let Some(doc) = state.state(&uri) else {
                            continue;
                        };
                        let Some(node) = doc.precise_match(pos) else {
                            continue;
                        };
                        let Some(dep) = doc.dependency(node.key.row_id()) else {
                            continue;
                        };
                        let Some(h) = hover(node, dep) else {
                            continue;
                        };
                        let _ = tx.send(h);
                    }
                    CargoDocumentEvent::CodeAction(uri, range, tx) => {
                        let Some(doc) = state.state(&uri) else {
                            continue;
                        };
                        let Some(node) = doc.precise_match(range.start) else {
                            continue;
                        };
                        let Some(dep) = doc.dependency(node.key.row_id()) else {
                            continue;
                        };
                        let Some(action) = code_action(uri, node, dep) else {
                            continue;
                        };
                        let _ = tx.send(action);
                    }
                    CargoDocumentEvent::Closed(uri) => {
                        state.del(&uri);
                    }
                    CargoDocumentEvent::CargoLockChanged => {
                        //clear state except the "current" uri
                        let Some(doc) = state.clear_except_current() else {
                            continue;
                        };
                        if let Err(e) = cargo_tx
                            .send(Ctx {
                                uri: doc.uri.clone(),
                                rev: doc.rev,
                            })
                            .await
                        {
                            eprintln!("cargo lock changed tx error: {}", e);
                        }
                    }
                    CargoDocumentEvent::Changed(msg) => {
                        let diff = state.partial_reconsile(&msg.uri, &msg.text);
                        let doc = state.state(&msg.uri).unwrap();
                        for v in &diff.range_updated {
                            if let Some(node) = doc.symbol(v) {
                                render_tx
                                    .send(DecorationEvent::DependencyRangeUpdate(
                                        msg.uri.clone(),
                                        v.to_string(),
                                        node.range,
                                    ))
                                    .await
                                    .unwrap();
                            }
                        }
                        for v in &diff.value_updated {
                            render_tx
                                .send(DecorationEvent::DependencyRemove(
                                    msg.uri.clone(),
                                    v.to_string(),
                                ))
                                .await
                                .unwrap();
                        }

                        for v in &diff.deleted {
                            render_tx
                                .send(DecorationEvent::DependencyRemove(
                                    msg.uri.clone(),
                                    v.to_string(),
                                ))
                                .await
                                .unwrap();
                        }
                    }
                    CargoDocumentEvent::Opened(msg) | CargoDocumentEvent::Saved(msg) => {
                        let diff = state.reconsile(&msg.uri, &msg.text);
                        let doc = state.state(&msg.uri).unwrap();

                        // Loop through both created and changed nodes
                        for v in &diff.created {
                            // Send to a dedicated render task
                            if let Some(n) = doc.symbol(v) {
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

                        for v in diff.range_updated.iter().chain(diff.value_updated.iter()) {
                            // Send to a dedicated render task
                            if let Some(n) = doc.symbol(v) {
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

                        for v in &diff.deleted {
                            render_tx
                                .send(DecorationEvent::DependencyRemove(
                                    msg.uri.clone(),
                                    v.to_string(),
                                ))
                                .await
                                .unwrap();
                        }

                        //no change to resolve
                        if !doc.is_dirty() {
                            continue;
                        }

                        for v in doc.dirty_nodes.keys() {
                            if let Some(n) = doc.symbol(v) {
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

                        //resolve cargo dependencies in another task
                        cargo_tx
                            .send(Ctx {
                                uri: msg.uri.clone(),
                                rev: doc.rev,
                            })
                            .await
                            .unwrap();
                    }
                    CargoDocumentEvent::CargoResolved(mut output) => {
                        let Some(doc) = state.state_mut_with_rev(&output.ctx.uri, output.ctx.rev)
                        else {
                            continue;
                        };
                        //populate deps
                        for dep in doc.dependencies.values_mut() {
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
                                        Some(cur)
                                            if summary.version() > cur
                                                && summary.version().is_prerelease()
                                                    == cur.is_prerelease() =>
                                        {
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
                            if let Some(rev) = doc.dirty_nodes.get(&dep.id) {
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
                                doc.dirty_nodes.remove(&dep.id);
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
