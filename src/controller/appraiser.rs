use cargo::util::VersionExt;
use semver::Version;
use tokio::sync::{
    mpsc::{self, Sender},
    oneshot,
};
use tower_lsp::{
    lsp_types::{CodeActionResponse, CompletionResponse, Hover, Position, Range, Url},
    Client,
};
use tracing::error;

use crate::{
    controller::{code_action::code_action, completion::completion},
    decoration::DecorationEvent,
    entity::CargoError,
    usecase::Workspace,
};

use super::{
    cargo::{cargo_resolve, CargoResolveOutput},
    debouncer::Debouncer,
    diagnostic::DiagnosticController,
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
    ReadyToResolve(Ctx),
    //reset document state
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
    Completion(Url, Position, oneshot::Sender<Option<CompletionResponse>>),
    CargoDiagnostic(Url, CargoError),
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
                match cargo_resolve(&event).await {
                    Ok(output) => {
                        if let Err(e) = tx_for_cargo
                            .send(CargoDocumentEvent::CargoResolved(output))
                            .await
                        {
                            error!("error sending cargo resolved event: {}", e);
                        }
                    }
                    Err(err) => {
                        if let Err(e) = tx_for_cargo
                            .send(CargoDocumentEvent::CargoDiagnostic(event.uri.clone(), err))
                            .await
                        {
                            error!("error sending diagnostic event: {}", e);
                        }
                    }
                }
            }
        });

        //timer task
        let mut debouncer = Debouncer::new(tx.clone(), 300, 3000);
        debouncer.spawn();

        //main loop
        //render task sender
        let render_tx = self.render_tx.clone();
        let client = self.client.clone();
        tokio::spawn(async move {
            //workspace state
            let mut state = Workspace::new();
            //diagnostic
            let mut diagnostic_controller = DiagnosticController::new(client);

            while let Some(event) = rx.recv().await {
                match event {
                    CargoDocumentEvent::CargoDiagnostic(uri, err) => {
                        diagnostic_controller.clear_cargo_diagnostics(&uri).await;
                        //we need a crate name to find something in toml
                        let Some(crate_name) = err.crate_name() else {
                            continue;
                        };

                        let Some(doc) = state.document(&uri) else {
                            continue;
                        };
                        let keys = doc.find_keys_by_crate_name(crate_name);
                        let deps = doc.find_deps_by_crate_name(crate_name);
                        let Some(digs) = err.diagnostic(&keys, &deps, doc.tree()) else {
                            continue;
                        };
                        for (id, diag) in digs {
                            diagnostic_controller
                                .add_cargo_diagnostic(&uri, id.as_str(), diag)
                                .await;
                        }
                    }
                    CargoDocumentEvent::Hovered(uri, pos, tx) => {
                        let Some(doc) = state.document(&uri) else {
                            continue;
                        };
                        let Some(node) = doc.precise_match_entry(pos) else {
                            continue;
                        };
                        let Some(dep) = doc.dependency(node.kind.entry_id()) else {
                            continue;
                        };
                        let Some(h) = hover(node, dep) else {
                            continue;
                        };
                        let _ = tx.send(h);
                    }
                    CargoDocumentEvent::Completion(uri, pos, tx) => {
                        let Some(doc) = state.document(&uri) else {
                            continue;
                        };
                        let key = doc.precise_match_key(pos);
                        let entry = doc.precise_match_entry(pos);
                        let dep = match &entry {
                            Some(entry) => doc.dependency(entry.kind.entry_id()),
                            None => None,
                        };
                        let completion = completion(key.as_ref(), entry.as_ref(), dep).await;
                        let _ = tx.send(completion);
                    }
                    CargoDocumentEvent::CodeAction(uri, range, tx) => {
                        let Some(doc) = state.document(&uri) else {
                            continue;
                        };
                        let Some(node) = doc.precise_match_entry(range.start) else {
                            continue;
                        };
                        let Some(dep) = doc.dependency(node.kind.entry_id()) else {
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
                        if let Err(e) = debouncer
                            .send_interactive(Ctx {
                                uri: doc.uri.clone(),
                                rev: doc.rev,
                            })
                            .await
                        {
                            eprintln!("debounder send interactive error: {}", e);
                        }
                    }
                    CargoDocumentEvent::Changed(msg) => {
                        diagnostic_controller
                            .clear_parse_diagnostics(&msg.uri)
                            .await;
                        let diff = match state.reconsile(&msg.uri, &msg.text) {
                            Ok((diff, _)) => diff,
                            Err(err) => {
                                for e in err {
                                    let Some((id, diag)) = e.diagnostic() else {
                                        continue;
                                    };
                                    diagnostic_controller
                                        .add_parse_diagnostic(&msg.uri, &id, diag)
                                        .await;
                                }
                                continue;
                            }
                        };
                        let doc = state.document(&msg.uri).unwrap();
                        for v in &diff.range_updated {
                            if let Some(node) = doc.entry(v) {
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
                        if let Err(e) = debouncer
                            .send_background(Ctx {
                                uri: msg.uri,
                                rev: doc.rev,
                            })
                            .await
                        {
                            eprintln!("debounder send interactive error: {}", e);
                        }
                    }
                    CargoDocumentEvent::Opened(msg) | CargoDocumentEvent::Saved(msg) => {
                        let rev = match state.reconsile(&msg.uri, &msg.text) {
                            Ok((_, rev)) => rev,
                            Err(err) => {
                                for e in err {
                                    let Some((id, diag)) = e.diagnostic() else {
                                        continue;
                                    };
                                    diagnostic_controller
                                        .add_parse_diagnostic(&msg.uri, &id, diag)
                                        .await;
                                }
                                continue;
                            }
                        };

                        if let Err(e) = debouncer.send_interactive(Ctx { uri: msg.uri, rev }).await
                        {
                            eprintln!("debounder send interactive error: {}", e);
                        }
                    }
                    CargoDocumentEvent::ReadyToResolve(ctx) => {
                        if state.check_rev(&ctx.uri, ctx.rev) {
                            start_resolve(&ctx.uri, &mut state, &render_tx, &cargo_tx).await;
                        }
                    }
                    CargoDocumentEvent::CargoResolved(mut output) => {
                        let Some(doc) = state.state_mut_with_rev(&output.ctx.uri, output.ctx.rev)
                        else {
                            continue;
                        };
                        diagnostic_controller
                            .clear_cargo_diagnostics(&output.ctx.uri)
                            .await;
                        //populate deps
                        for dep in doc.dependencies.values_mut() {
                            let key = dep.toml_key();
                            if !output.dependencies.is_empty()
                                && output.dependencies.contains_key(&key)
                                && doc.dirty_nodes.contains_key(&dep.id)
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
                                                    == installed.is_prerelease() =>
                                        {
                                            latest = Some(summary.version());
                                            dep.latest_summary = Some(summary.clone());
                                        }
                                        None if summary.version().is_prerelease()
                                            == installed.is_prerelease() =>
                                        {
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
                        if doc.is_dirty() {
                            if let Err(e) = debouncer
                                .send_background(Ctx {
                                    uri: output.ctx.uri,
                                    rev: doc.rev,
                                })
                                .await
                            {
                                eprintln!("debounder send background error: {}", e);
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

async fn start_resolve(
    uri: &Url,
    state: &mut Workspace,
    render_tx: &Sender<DecorationEvent>,
    cargo_tx: &Sender<Ctx>,
) {
    //start from here the resolve process
    state.populate_dependencies(uri);
    let doc = state.document(uri).unwrap();

    //no change to resolve
    if !doc.is_dirty() {
        return;
    }

    for v in doc.dirty_nodes.keys() {
        if let Some(n) = doc.entry(v) {
            render_tx
                .send(DecorationEvent::DependencyWaiting(
                    uri.clone(),
                    v.to_string(),
                    n.range,
                ))
                .await
                .unwrap();
        }
    }

    //resolve cargo dependencies in another task
    if let Err(e) = cargo_tx
        .send(Ctx {
            uri: uri.clone(),
            rev: doc.rev,
        })
        .await
    {
        eprintln!("cargo resolve tx error: {}", e);
    }
}
