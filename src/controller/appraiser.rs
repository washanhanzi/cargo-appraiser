use std::collections::HashMap;

use cargo::util::VersionExt;
use semver::Version;
use tokio::sync::{
    mpsc::{self, Sender},
    oneshot,
};
use tower_lsp::{
    lsp_types::{CodeActionResponse, CompletionResponse, Diagnostic, Hover, Position, Range, Uri},
    Client,
};
use tracing::{error, info};

use crate::{
    controller::{
        audit::into_diagnostic_text, code_action::code_action, completion::completion,
        read_file::ReadFileParam,
    },
    decoration::DecorationEvent,
    entity::{into_file_uri, CargoError, Dependency},
    usecase::Workspace,
};

use super::{
    audit::{into_diagnostic_severity, AuditController, AuditReports, AuditResult},
    capabilities::{ClientCapabilities, ClientCapability},
    cargo::{cargo_resolve, CargoResolveOutput},
    debouncer::Debouncer,
    diagnostic::DiagnosticController,
    hover::hover,
    read_file::ReadFile,
};

#[derive(Debug, Clone)]
pub struct Ctx {
    pub uri: Uri,
    pub rev: usize,
}

//CargoState will run a dedicate task which receive msg from lsp event
//the msg payload should contain the file content and lsp client
//track current opened cargo.toml file and rev
#[derive(Debug)]
pub struct Appraiser {
    client: Client,
    render_tx: Sender<DecorationEvent>,
    client_capabilities: ClientCapabilities,
}

pub enum CargoDocumentEvent {
    //cargo.toml save event
    //start to parse the document, update the state, and send event for cargo_tree task
    Opened(CargoTomlPayload),
    Saved(CargoTomlPayload),
    Changed(CargoTomlPayload),
    ReadyToResolve(Ctx),
    //reset document state
    Closed(Uri),
    //result from cargo command
    //consolidate state and send render event
    CargoResolved(CargoResolveOutput),
    //cargo.lock change
    //CargoLockCreated,
    CargoLockChanged,
    //code action, path and range
    CodeAction(Uri, Range, oneshot::Sender<CodeActionResponse>),
    //hover event, path and position
    Hovered(Uri, Position, oneshot::Sender<Hover>),
    Completion(Uri, Position, oneshot::Sender<Option<CompletionResponse>>),
    CargoDiagnostic(Uri, CargoError),
    Audited(AuditReports),
}

pub struct CargoTomlPayload {
    pub uri: Uri,
    pub text: String,
}

impl Appraiser {
    pub fn new(
        client: Client,
        render_tx: Sender<DecorationEvent>,
        client_capabilities: &[ClientCapability],
    ) -> Self {
        let client_capabilities = ClientCapabilities::new(client_capabilities);
        Self {
            client,
            render_tx,
            client_capabilities,
        }
    }
    pub fn initialize(&self) -> Sender<CargoDocumentEvent> {
        //create mpsc channel
        let (tx, mut rx) = mpsc::channel::<CargoDocumentEvent>(64);
        let inner_tx = tx.clone();

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

        //audit task
        let mut audit_controller = AuditController::new(tx.clone());
        audit_controller.spawn();

        //main loop
        //render task sender
        let render_tx = self.render_tx.clone();
        let client = self.client.clone();
        let client_capabilities = self.client_capabilities.clone();
        tokio::spawn(async move {
            //workspace state
            let mut state = Workspace::new();
            //diagnostic
            let diag_client = client.clone();
            let mut diagnostic_controller = DiagnosticController::new(diag_client);

            while let Some(event) = rx.recv().await {
                match event {
                    CargoDocumentEvent::Audited(reports) => {
                        //a hashset to record which is already audited
                        let mut audited: HashMap<(Uri, String), (Dependency, Vec<AuditResult>)> =
                            HashMap::new();
                        for (path, report) in &reports.members {
                            let cargo_path_uri = into_file_uri(path.join("Cargo.toml").as_path());
                            //go to state.document(uri) and then state.document(root_manifest)
                            let doc = match state.document(&cargo_path_uri) {
                                Some(doc) => doc,
                                None => match state.document(&reports.root) {
                                    Some(doc) => doc,
                                    None => continue,
                                },
                            };
                            //loop dependencies and write the audited with root_manifest
                            for dep in doc.dependencies.values() {
                                //if it has resolved dependency, we can compare the version
                                //if it doesn't(for virtual workspace), we can just compare the version compatibility
                                //first find matching dependency name in resports
                                let Some(reports_map) = report.get(dep.package_name()) else {
                                    continue;
                                };
                                //then find matching dependency version in reports_map
                                match dep.resolved.as_ref() {
                                    Some(resolved) => {
                                        //then find matching dependency version in rs
                                        let Some(rr) = reports_map
                                            .get(resolved.version().to_string().as_str())
                                        else {
                                            continue;
                                        };
                                        audited.insert(
                                            (cargo_path_uri.clone(), dep.id.to_string()),
                                            (dep.clone(), rr.clone()),
                                        );
                                    }
                                    None => {
                                        for (v, rr) in reports_map {
                                            if dep
                                                .unresolved
                                                .as_ref()
                                                .unwrap()
                                                .version_req()
                                                .matches(&Version::parse(v).unwrap())
                                            {
                                                audited.insert(
                                                    (cargo_path_uri.clone(), dep.id.to_string()),
                                                    (dep.clone(), rr.clone()),
                                                );
                                            }
                                        }
                                    }
                                };
                            }
                            //send to diagnostic
                            for ((uri, _), (dep, rr)) in &audited {
                                let diag = Diagnostic {
                                    range: dep.range,
                                    severity: Some(into_diagnostic_severity(rr)),
                                    code: None,
                                    code_description: None,
                                    source: Some("cargo-appraiser".to_string()),
                                    message: into_diagnostic_text(rr),
                                    related_information: None,
                                    tags: None,
                                    data: None,
                                };
                                diagnostic_controller
                                    .add_audit_diagnostic(uri, &dep.id, diag)
                                    .await;
                            }
                        }
                    }
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
                        let Some(node) = doc.precise_match(pos) else {
                            continue;
                        };
                        let dep = match node.row_id() {
                            Some(id) => doc.dependency(&id),
                            None => None,
                        };
                        let Some(h) = hover(&node, dep, doc.members.as_deref()) else {
                            continue;
                        };
                        let _ = tx.send(h);
                    }
                    CargoDocumentEvent::Completion(uri, pos, tx) => {
                        let Some(doc) = state.document(&uri) else {
                            continue;
                        };
                        let Some(node) = doc.precise_match(pos) else {
                            continue;
                        };
                        let Some(id) = node.row_id() else {
                            continue;
                        };
                        let dep = doc.dependency(&id);
                        let completion = completion(&node, dep).await;
                        let _ = tx.send(completion);
                    }
                    CargoDocumentEvent::CodeAction(uri, range, tx) => {
                        let Some(doc) = state.document(&uri) else {
                            continue;
                        };
                        let Some(node) = doc.precise_match_entry(range.start) else {
                            continue;
                        };
                        let Some(id) = node.row_id() else {
                            continue;
                        };
                        let Some(dep) = doc.dependency(&id) else {
                            continue;
                        };
                        let Some(action) = code_action(uri, node, dep) else {
                            continue;
                        };
                        let _ = tx.send(action);
                    }
                    CargoDocumentEvent::Closed(uri) => {}
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
                            error!("debounder send interactive error: {}", e);
                        }
                    }
                    CargoDocumentEvent::Changed(msg) => {
                        diagnostic_controller
                            .clear_parse_diagnostics(&msg.uri)
                            .await;
                        //when Cargo.toml changed, clear audit diagnostics
                        diagnostic_controller.clear_audit_diagnostics().await;
                        let diff = match state.reconsile(&msg.uri, &msg.text) {
                            Ok((_, diff)) => diff,
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
                            error!("debounder send interactive error: {}", e);
                        }
                    }
                    CargoDocumentEvent::Opened(msg) | CargoDocumentEvent::Saved(msg) => {
                        if let Err(e) = audit_controller.send(&msg.uri).await {
                            error!("audit controller send error: {}", e);
                        };
                        let doc = match state.reconsile(&msg.uri, &msg.text) {
                            Ok((doc, diff)) => {
                                if diff.is_empty() {
                                    continue;
                                } else {
                                    doc
                                }
                            }
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

                        if let Some(uri) = doc.root_manifest.as_ref() {
                            if uri != &msg.uri {
                                if client_capabilities.can_read_file() {
                                    let param = ReadFileParam { uri: uri.clone() };
                                    match client.send_request::<ReadFile>(param).await {
                                        Ok(content) => {
                                            if let Err(e) = inner_tx
                                                .send(CargoDocumentEvent::Opened(
                                                    CargoTomlPayload {
                                                        uri: uri.clone(),
                                                        text: content.content,
                                                    },
                                                ))
                                                .await
                                            {
                                                error!("inner tx send error: {}", e);
                                            }
                                        }
                                        Err(e) => {
                                            error!("read file error: {}", e);
                                        }
                                    }
                                } else {
                                    //read file with os
                                    let content =
                                        std::fs::read_to_string(uri.path().as_str()).unwrap();
                                    if let Err(e) = inner_tx
                                        .send(CargoDocumentEvent::Opened(CargoTomlPayload {
                                            uri: uri.clone(),
                                            text: content,
                                        }))
                                        .await
                                    {
                                        error!("inner tx send error: {}", e);
                                    }
                                }
                            }
                        }

                        if let Err(e) = debouncer
                            .send_interactive(Ctx {
                                uri: msg.uri,
                                rev: doc.rev,
                            })
                            .await
                        {
                            error!("debounder send interactive error: {}", e);
                        }
                    }
                    CargoDocumentEvent::ReadyToResolve(ctx) => {
                        if state.check_rev(&ctx.uri, ctx.rev) {
                            start_resolve(&ctx.uri, &mut state, &render_tx, &cargo_tx).await;
                        }
                    }
                    CargoDocumentEvent::CargoResolved(mut output) => {
                        let Some(doc) =
                            state.document_mut_with_rev(&output.ctx.uri, output.ctx.rev)
                        else {
                            continue;
                        };
                        diagnostic_controller
                            .clear_cargo_diagnostics(&output.ctx.uri)
                            .await;
                        //populate deps
                        for dep in doc.dependencies.values_mut() {
                            let key = dep.toml_key();

                            if doc.dirty_nodes.contains_key(&dep.id) {
                                // Take resolved out of the output.dependencies hashmap
                                let maybe_resolved = output.dependencies.remove(&key);
                                dep.resolved = maybe_resolved;

                                let package_name = dep.package_name();
                                let Some(summaries) = output.summaries.remove(package_name) else {
                                    continue;
                                };
                                dep.summaries = Some(summaries.clone());

                                if let Some(resolved) = dep.resolved.as_ref() {
                                    let installed = resolved.version().clone();
                                    let req_version =
                                        dep.unresolved.as_ref().unwrap().version_req();

                                    let mut latest: Option<&Version> = None;
                                    let mut latest_matched: Option<&Version> = None;
                                    for summary in &summaries {
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
                                        match (latest_matched.as_ref(), installed.is_prerelease()) {
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
                                            (None, false)
                                                if req_version.matches(summary.version()) =>
                                            {
                                                latest_matched = Some(summary.version());
                                                dep.latest_matched_summary = Some(summary.clone());
                                            }
                                            _ => {}
                                        }
                                    }
                                };

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
                                error!("debounder send background error: {}", e);
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
    uri: &Uri,
    state: &mut Workspace,
    render_tx: &Sender<DecorationEvent>,
    cargo_tx: &Sender<Ctx>,
) {
    let Some(doc) = state.document_mut(uri) else {
        return;
    };
    doc.populate_dependencies();

    //virtual workspace doesn't need to resolve
    if doc.is_virtual() {
        return;
    }

    //no change to document
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
        error!("cargo resolve tx error: {}", e);
    }
}
