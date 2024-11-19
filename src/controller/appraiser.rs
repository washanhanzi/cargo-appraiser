use std::{collections::HashMap, str::FromStr};

use cargo::util::VersionExt;
use semver::Version;
use tokio::sync::{
    mpsc::{self, Sender},
    oneshot,
};
use tower_lsp::{
    lsp_types::{
        CodeActionResponse, CompletionResponse, Diagnostic, GotoDefinitionResponse, Hover,
        Position, Range, Uri,
    },
    Client,
};
use tracing::error;

use crate::{
    controller::{
        audit::into_diagnostic_text, code_action::code_action, completion::completion,
        read_file::ReadFileParam,
    },
    decoration::DecorationEvent,
    entity::{into_file_uri, CargoError, Dependency},
    usecase::{Document, Workspace},
};

use super::{
    audit::{into_diagnostic_severity, AuditController, AuditReports, AuditResult},
    capabilities::{ClientCapabilities, ClientCapability},
    cargo::{cargo_resolve, CargoResolveOutput},
    debouncer::Debouncer,
    diagnostic::DiagnosticController,
    gd::goto_definition,
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
    Opened(CargoTomlPayload),
    Saved(CargoTomlPayload),
    //Parse event won't trigger Cargo.toml resolve compare to Opened and Saved
    Parse(CargoTomlPayload),
    Changed(CargoTomlPayload),
    ReadyToResolve(Ctx),
    //mark dependencies dirty, clear decorations
    Closed(Uri),
    //result from cargo
    CargoResolved(CargoResolveOutput),
    //cargo.lock change
    //CargoLockCreated,
    CargoLockChanged,
    //code action, path and range
    CodeAction(Uri, Range, oneshot::Sender<CodeActionResponse>),
    //hover event, path and position
    Hovered(Uri, Position, oneshot::Sender<Option<Hover>>),
    Completion(Uri, Position, oneshot::Sender<Option<CompletionResponse>>),
    //goto definition
    Gded(
        Uri,
        Position,
        oneshot::Sender<Option<GotoDefinitionResponse>>,
    ),
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
        client_capabilities: Option<&[ClientCapability]>,
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
        let mut debouncer = Debouncer::new(tx.clone(), 1000, 5000);
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
                                            match dep.unresolved.as_ref() {
                                                Some(unresolved) => {
                                                    if unresolved
                                                        .version_req()
                                                        .matches(&Version::parse(v).unwrap())
                                                    {
                                                        audited.insert(
                                                            (
                                                                cargo_path_uri.clone(),
                                                                dep.id.to_string(),
                                                            ),
                                                            (dep.clone(), rr.clone()),
                                                        );
                                                    }
                                                }
                                                None => {
                                                    if let Some(v) = dep.version.as_ref() {
                                                        let Ok(req)=  cargo_util_schemas::core::PartialVersion::from_str(v.value())else{
                                                            continue;
                                                        };
                                                        if req.to_caret_req().matches(
                                                            &Version::parse(v.value()).unwrap(),
                                                        ) {
                                                            audited.insert(
                                                                (
                                                                    cargo_path_uri.clone(),
                                                                    dep.id.to_string(),
                                                                ),
                                                                (dep.clone(), rr.clone()),
                                                            );
                                                        }
                                                    }
                                                }
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
                        let h = hover(&node, dep, doc.members.as_deref());
                        let _ = tx.send(h);
                    }
                    CargoDocumentEvent::Gded(uri, pos, tx) => {
                        let Some(doc) = state.document(&uri) else {
                            continue;
                        };
                        let Some(node) = doc.precise_match(pos) else {
                            continue;
                        };
                        let gd = goto_definition(&state, doc, &node);
                        let _ = tx.send(gd);
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
                    CargoDocumentEvent::Closed(uri) => {
                        if let Some(doc) = state.document_mut(&uri) {
                            doc.mark_dirty();
                            if let Err(e) = render_tx.send(DecorationEvent::Reset(uri)).await {
                                error!("render tx send reset error: {}", e);
                            }
                        }
                    }
                    CargoDocumentEvent::CargoLockChanged => {
                        //clear state except the "current" uri
                        let uris = state.mark_all_dirty();
                        for (uri, rev) in uris {
                            if let Err(e) = debouncer.send_background(Ctx { uri, rev }).await {
                                error!("debounder send interactive error: {}", e);
                            }
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
                    CargoDocumentEvent::Parse(msg) => {
                        if let Err(e) = audit_controller.send(&msg.uri).await {
                            error!("audit controller send error: {}", e);
                        };
                        let _ =
                            reconsile_document(&mut state, &mut diagnostic_controller, &msg).await;
                    }
                    CargoDocumentEvent::Opened(msg) | CargoDocumentEvent::Saved(msg) => {
                        if let Err(e) = audit_controller.send(&msg.uri).await {
                            error!("audit controller send error: {}", e);
                        };
                        let Some(doc) =
                            reconsile_document(&mut state, &mut diagnostic_controller, &msg).await
                        else {
                            continue;
                        };

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
                        //delay audit
                        if let Err(e) = audit_controller.send(&ctx.uri).await {
                            error!("audit controller send error: {}", e);
                        };
                        if state.check_rev(&ctx.uri, ctx.rev) {
                            start_resolve(
                                &ctx.uri,
                                &mut state,
                                &render_tx,
                                &cargo_tx,
                                &inner_tx,
                                &client,
                                &client_capabilities,
                            )
                            .await;
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
                            if dep.is_virtual {
                                continue;
                            }
                            let key = dep.toml_key();

                            if let Some(rev) = doc.dirty_dependencies.get(&dep.id) {
                                if *rev > output.ctx.rev {
                                    continue;
                                }
                                // Take resolved out of the output.dependencies hashmap
                                let maybe_resolved = output.dependencies.remove(&key);
                                dep.resolved = maybe_resolved;

                                let package_name = dep.package_name();
                                let Some(mut summaries) = output.summaries.remove(package_name)
                                else {
                                    continue;
                                };
                                if let (Some(resolved), Some(unresolved)) =
                                    (dep.resolved.as_ref(), dep.unresolved.as_ref())
                                {
                                    let installed = resolved.version().clone();
                                    let req_version = unresolved.version_req();

                                    //order summaries by version
                                    summaries.sort_by(|a, b| b.version().cmp(a.version()));
                                    for summary in &summaries {
                                        if dep.matched_summary.is_some()
                                            && dep.latest_matched_summary.is_some()
                                            && dep.latest_summary.is_some()
                                        {
                                            break;
                                        }
                                        if &installed == summary.version() {
                                            dep.matched_summary = Some(summary.clone());
                                        }
                                        if dep.latest_summary.is_none()
                                            && summary.version().is_prerelease()
                                                == installed.is_prerelease()
                                        {
                                            dep.latest_summary = Some(summary.clone());
                                        }
                                        if dep.latest_matched_summary.is_none()
                                            && req_version.matches(summary.version())
                                        {
                                            dep.latest_matched_summary = Some(summary.clone());
                                        }
                                    }
                                    dep.summaries = Some(summaries.clone());
                                };
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
                                doc.dirty_dependencies.remove(&dep.id);
                            }
                        }
                        if doc.is_dependencies_dirty() {
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
    inner_tx: &Sender<CargoDocumentEvent>,
    client: &Client,
    client_capabilities: &ClientCapabilities,
) {
    let Some(doc) = state.document_mut(uri) else {
        return;
    };
    doc.populate_dependencies();

    if let Some(root_uri) = doc.root_manifest.as_ref() {
        if root_uri != uri {
            if client_capabilities.can_read_file() {
                let param = ReadFileParam {
                    uri: root_uri.clone(),
                };
                match client.send_request::<ReadFile>(param).await {
                    Ok(content) => {
                        if let Err(e) = inner_tx
                            .send(CargoDocumentEvent::Parse(CargoTomlPayload {
                                uri: root_uri.clone(),
                                text: content.content,
                            }))
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
                let content = std::fs::read_to_string(root_uri.path().as_str()).unwrap();
                if let Err(e) = inner_tx
                    .send(CargoDocumentEvent::Parse(CargoTomlPayload {
                        uri: root_uri.clone(),
                        text: content,
                    }))
                    .await
                {
                    error!("inner tx send error: {}", e);
                }
            }
        }
    }

    //virtual workspace doesn't need to resolve
    if doc.is_virtual() {
        return;
    }

    //no need to resolve
    if !doc.is_dependencies_dirty() {
        return;
    }

    for v in doc.dirty_dependencies.keys() {
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

async fn reconsile_document<'a>(
    state: &'a mut Workspace,
    diagnostic_controller: &'a mut DiagnosticController,
    msg: &CargoTomlPayload,
) -> Option<&'a Document> {
    match state.reconsile(&msg.uri, &msg.text) {
        Ok((doc, diff)) => {
            if diff.is_empty() && !doc.is_dependencies_dirty() {
                None
            } else {
                Some(doc)
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
            None
        }
    }
}
