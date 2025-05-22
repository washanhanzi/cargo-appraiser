use std::{cmp::max, env, str::FromStr};

use cargo::{core::dependency::DepKind, util::VersionExt};
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
use tracing::{debug, error, trace, warn};

use crate::{
    config::GLOBAL_CONFIG,
    controller::{code_action::code_action, completion::completion, read_file::ReadFileParam},
    decoration::DecorationEvent,
    entity::{CanonicalUri, CargoError, DependencyTable},
    usecase::{Document, Workspace},
};

use super::{
    audit::{AuditController, AuditReports},
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
    pub uri: CanonicalUri,
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
    cargo_path: String,
}

pub enum CargoDocumentEvent {
    Opened(CargoTomlPayload),
    Saved(CargoTomlPayload),
    //Parse event won't trigger Cargo.toml resolve compare to Opened and Saved
    Parse(Uri),
    Changed(CargoTomlPayload),
    ReadyToResolve(Ctx),
    //mark dependencies dirty, clear decorations
    Closed(Uri),
    //result from cargo
    CargoResolved(Box<CargoResolveOutput>),
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
    CargoDiagnostic(CanonicalUri, CargoError),
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
        cargo_path: String,
    ) -> Self {
        let client_capabilities = ClientCapabilities::new(client_capabilities);
        Self {
            client,
            render_tx,
            client_capabilities,
            cargo_path,
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
            match env::var("PATH") {
                Ok(path_var) => trace!("Current PATH: {}", path_var),
                Err(e) => warn!("Failed to read PATH environment variable: {}", e),
            }

            while let Some(event) = cargo_rx.recv().await {
                match cargo_resolve(&event).await {
                    Ok(output) => {
                        if let Err(e) = tx_for_cargo
                            .send(CargoDocumentEvent::CargoResolved(Box::new(output)))
                            .await
                        {
                            error!("error sending cargo resolved event: {}", e);
                        }
                    }
                    Err(err) => {
                        error!("error resolving: {}", err);
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
        let cargo_path = self.cargo_path.clone();
        tokio::spawn(async move {
            //workspace state
            let mut state = Workspace::new();
            //diagnostic
            let diag_client = client.clone();
            let mut diagnostic_controller = DiagnosticController::new(diag_client);

            while let Some(event) = rx.recv().await {
                match event {
                    CargoDocumentEvent::Audited(reports) => {
                        debug!("found audit reports: {:?}", reports.len());
                        let Some(root_manifest_uri) = state.root_manifest_uri.as_ref() else {
                            continue;
                        };
                        let Some(doc) = state.root_document() else {
                            continue;
                        };
                        for issues in reports.values() {
                            for issue in issues {
                                for (crate_name, paths) in &issue.dependency_paths {
                                    let depenedencies = doc.dependencies_by_crate_name(crate_name);
                                    if depenedencies.is_empty() {
                                        continue;
                                    }
                                    for dep in depenedencies {
                                        let Some(resolved) = dep.resolved.as_ref() else {
                                            continue;
                                        };
                                        let required_version = if crate_name == &issue.crate_name {
                                            issue.version.clone()
                                        } else {
                                            let mut splits = paths
                                                .last()
                                                .map(|s| s.split(" "))
                                                .unwrap_or_else(|| "".split(" "));
                                            splits.nth(1).unwrap_or_default().to_string()
                                        };
                                        if required_version.is_empty() {
                                            continue;
                                        }
                                        if required_version == resolved.version().to_string() {
                                            //send diagnostic
                                            let diag = Diagnostic {
                                                range: dep.range,
                                                severity: Some(issue.severity()),
                                                code: None,
                                                code_description: None,
                                                source: Some("cargo-appraiser".to_string()),
                                                message: issue.audit_text(Some(crate_name)),
                                                related_information: None,
                                                tags: None,
                                                data: None,
                                            };
                                            diagnostic_controller
                                                .add_audit_diagnostic(&doc.uri, &dep.id, diag)
                                                .await;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    CargoDocumentEvent::CargoDiagnostic(uri, err) => {
                        debug!(
                            "Appraiser Event: CargoDiagnostic for URI: {:?}, Error: {:?}",
                            uri, err
                        );
                        let Some(client_uri) = state.uri(&uri) else {
                            continue;
                        };
                        diagnostic_controller
                            .clear_cargo_diagnostics(&client_uri)
                            .await;
                        //we need a crate name to find something in toml
                        let Some(crate_name) = err.crate_name() else {
                            continue;
                        };

                        let Ok(canonical_uri) = uri.clone().try_into() else {
                            error!("failed to canonicalize uri: {}", uri.as_str());
                            continue;
                        };

                        let Some(doc) = state.document(&canonical_uri) else {
                            continue;
                        };
                        let keys = doc.find_keys_by_crate_name(crate_name);
                        let deps = doc.find_deps_by_crate_name(crate_name);
                        let Some(digs) = err.diagnostic(&keys, &deps, doc.tree()) else {
                            continue;
                        };
                        for (id, diag) in digs {
                            diagnostic_controller
                                .add_cargo_diagnostic(&client_uri, id.as_str(), diag)
                                .await;
                        }
                    }
                    CargoDocumentEvent::Hovered(uri, pos, tx) => {
                        let Ok(canonical_uri) = uri.clone().try_into() else {
                            error!("failed to canonicalize uri: {}", uri.as_str());
                            continue;
                        };
                        let Some(doc) = state.document(&canonical_uri) else {
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
                        let Ok(canonical_uri) = uri.clone().try_into() else {
                            error!("failed to canonicalize uri: {}", uri.as_str());
                            continue;
                        };
                        let Some(doc) = state.document(&canonical_uri) else {
                            continue;
                        };
                        let Some(node) = doc.precise_match(pos) else {
                            continue;
                        };
                        let gd = goto_definition(&state, doc, &node);
                        let _ = tx.send(gd);
                    }
                    CargoDocumentEvent::Completion(uri, pos, tx) => {
                        let Ok(canonical_uri) = uri.clone().try_into() else {
                            error!("failed to canonicalize uri: {}", uri.as_str());
                            continue;
                        };
                        let Some(doc) = state.document(&canonical_uri) else {
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
                        let Ok(canonical_uri) = uri.clone().try_into() else {
                            error!("failed to canonicalize uri: {}", uri.as_str());
                            continue;
                        };
                        let Some(doc) = state.document(&canonical_uri) else {
                            continue;
                        };
                        let Some(node) = doc.precise_match(range.start) else {
                            continue;
                        };
                        let Some(id) = node.row_id() else {
                            continue;
                        };
                        let dep = doc.dependency(&id);
                        let Some(action) = code_action(uri, node, dep) else {
                            continue;
                        };
                        let _ = tx.send(action);
                    }
                    CargoDocumentEvent::Closed(uri) => {
                        let Ok(canonical_uri) = uri.clone().try_into() else {
                            error!("failed to canonicalize uri: {}", uri.as_str());
                            continue;
                        };
                        if let Some(doc) = state.document_mut(&canonical_uri) {
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
                        debug!("Appraiser Event: Changed for URI: {:?}", msg.uri);
                        diagnostic_controller
                            .clear_parse_diagnostics(&msg.uri)
                            .await;

                        let Ok(canonical_uri) = TryInto::<CanonicalUri>::try_into(msg.uri.clone())
                        else {
                            error!("failed to canonicalize uri: {}", msg.uri.as_str());
                            continue;
                        };
                        //when Cargo.toml changed, clear audit diagnostics
                        diagnostic_controller.clear_audit_diagnostics().await;
                        let diff = match state.reconsile(
                            msg.uri.clone(),
                            canonical_uri.clone(),
                            &msg.text,
                        ) {
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
                        let doc = state.document(&canonical_uri).unwrap();
                        for v in &diff.range_updated {
                            if let Some(node) = doc.entry(v) {
                                render_tx
                                    .send(DecorationEvent::DependencyRangeUpdate(
                                        doc.uri.clone(),
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
                                    doc.uri.clone(),
                                    v.to_string(),
                                ))
                                .await
                                .unwrap();
                        }

                        for v in &diff.deleted {
                            render_tx
                                .send(DecorationEvent::DependencyRemove(
                                    doc.uri.clone(),
                                    v.to_string(),
                                ))
                                .await
                                .unwrap();
                        }
                        if let Err(e) = debouncer
                            .send_background(Ctx {
                                uri: canonical_uri,
                                rev: doc.rev,
                            })
                            .await
                        {
                            error!("debounder send interactive error: {}", e);
                        }
                    }
                    CargoDocumentEvent::Parse(uri) => {
                        debug!("Appraiser Event: Parse for URI: {:?}", uri);
                        let Ok(canonical_uri) = TryInto::<CanonicalUri>::try_into(uri.clone())
                        else {
                            error!("failed to canonicalize uri: {}", uri.as_str());
                            continue;
                        };
                        let content = if client_capabilities.can_read_file() {
                            let param = ReadFileParam { uri: uri.clone() };
                            match client.send_request::<ReadFile>(param).await {
                                Ok(content) => content.content,
                                Err(e) => {
                                    error!("read file error: {}", e);
                                    continue;
                                }
                            }
                        } else {
                            //read file with os
                            let Ok(path) = canonical_uri.to_path_buf() else {
                                error!("failed to convert canonical uri to path: {}", uri.as_str());
                                continue;
                            };
                            match std::fs::read_to_string(path) {
                                Ok(content) => content,
                                Err(e) => {
                                    error!("read file error: {}", e);
                                    continue;
                                }
                            }
                        };
                        let _ = reconsile_document(
                            &mut state,
                            &mut diagnostic_controller,
                            &CargoTomlPayload { uri, text: content },
                            &canonical_uri,
                        )
                        .await;
                    }
                    CargoDocumentEvent::Opened(msg) | CargoDocumentEvent::Saved(msg) => {
                        debug!("Appraiser Event: Opened/Saved for URI: {:?}", msg.uri);
                        let Ok(canonical_uri) = msg.uri.clone().try_into() else {
                            error!("failed to canonicalize uri: {}", msg.uri.as_str());
                            continue;
                        };
                        let Some(doc) = reconsile_document(
                            &mut state,
                            &mut diagnostic_controller,
                            &msg,
                            &canonical_uri,
                        )
                        .await
                        else {
                            continue;
                        };

                        if let Err(e) = debouncer
                            .send_interactive(Ctx {
                                uri: canonical_uri,
                                rev: doc.rev,
                            })
                            .await
                        {
                            error!("debounder send interactive error: {}", e);
                        }
                    }
                    CargoDocumentEvent::ReadyToResolve(ctx) => {
                        debug!(
                            "Appraiser Event: ReadyToResolve for URI: {:?}, rev: {}",
                            ctx.uri, ctx.rev
                        );
                        if state.check_rev(&ctx.uri, ctx.rev) {
                            let Some(doc) = state.document(&ctx.uri) else {
                                continue;
                            };
                            start_resolve(doc, &render_tx, &cargo_tx).await;
                        }
                    }
                    CargoDocumentEvent::CargoResolved(output) => {
                        debug!(
                            "Appraiser Event: CargoResolved for URI: {:?}, rev: {}. Specs: {}, Dependencies: {}, Packages: {}, Summaries: {}",
                            output.ctx.uri,
                            output.ctx.rev,
                            output.specs.len(),
                            output.dependencies.len(),
                            output.packages.len(),
                            output.summaries.len()
                        );
                        //resolve virtual manifest if we haven't
                        let root_manifest_uri = output.root_manifest_uri.clone();
                        if state.document(&root_manifest_uri).is_none() {
                            let Ok(canonical_uri) =
                                CanonicalUri::try_from(root_manifest_uri.clone())
                            else {
                                error!(
                                    "failed to canonicalize root manifest uri: {}",
                                    root_manifest_uri.as_str()
                                );
                                continue;
                            };
                            //TODO
                            let uri = Uri::from_str(root_manifest_uri.as_str()).unwrap();
                            if let Err(e) = inner_tx.send(CargoDocumentEvent::Parse(uri)).await {
                                error!("inner tx send error: {}", e);
                            }
                        }
                        state.root_manifest_uri = Some(root_manifest_uri.clone());
                        state.specs = output.specs;
                        state.member_manifest_uris = output.member_manifest_uris;

                        //send audit event
                        if !GLOBAL_CONFIG.read().unwrap().audit.disabled {
                            debug!("send audit event");
                            if let Err(e) = audit_controller
                                .send(root_manifest_uri, state.specs.clone(), &cargo_path)
                                .await
                            {
                                error!("audit controller send error: {}", e);
                            };
                        }

                        let Some(doc) =
                            state.document_mut_with_rev(&output.ctx.uri, output.ctx.rev)
                        else {
                            continue;
                        };
                        diagnostic_controller
                            .clear_cargo_diagnostics(&doc.uri)
                            .await;

                        //populate deps
                        for dep in doc.dependencies.values_mut() {
                            let Some(rev) = doc.dirty_dependencies.get(&dep.id) else {
                                continue;
                            };
                            if *rev > output.ctx.rev {
                                continue;
                            }

                            //populate requested
                            let Some(requested_result) = output.dependencies.get(&dep.name) else {
                                if let Err(e) = render_tx
                                    .send(DecorationEvent::Dependency(
                                        doc.uri.clone(),
                                        dep.id.clone(),
                                        dep.range,
                                        dep.clone(),
                                    ))
                                    .await
                                {
                                    error!("render tx send error: {}", e);
                                };
                                doc.dirty_dependencies.remove(&dep.id);
                                continue;
                            };

                            if dep.is_virtual {
                                let mut tables: Vec<DependencyTable> =
                                    Vec::with_capacity(requested_result.len());
                                let mut normal_use = false;
                                for requested_dep in requested_result {
                                    if requested_dep.1.kind() == DepKind::Normal {
                                        normal_use = true;
                                    } else {
                                        tables.push(requested_dep.1.kind().into());
                                    }
                                }
                                if !normal_use {
                                    dep.used_in_tables = tables;
                                }
                            }

                            let requested = match requested_result.len() {
                                0 => unreachable!(),
                                1 => requested_result.first().unwrap(),
                                _ => {
                                    let mut matched_requested_dep = None;
                                    for requested_dep in requested_result {
                                        // For not virtual dependency, they should in same table
                                        if !dep.is_virtual
                                            && dep.table.to_string()
                                                != requested_dep.1.kind().kind_table()
                                        {
                                            debug!(
                                                "not virtual dependency, table not match: {}",
                                                dep.id
                                            );
                                            continue;
                                        }
                                        //a dependency could specify platform in member Cargo.toml but not in workspace Cargo.toml
                                        matched_requested_dep = Some(requested_dep);
                                    }
                                    let Some(requested_dep) = matched_requested_dep else {
                                        error!("Can't find a match for dep {}", dep.id);
                                        //remove the dirty dependency else we stuck in infinite cargo resolve
                                        doc.dirty_dependencies.remove(&dep.id);
                                        continue;
                                    };
                                    requested_dep
                                }
                            };
                            dep.requested = Some(requested.1.clone());

                            let mut summaries =
                                output.summaries.get(&requested.0).cloned().unwrap();
                            summaries.sort_by(|a, b| b.version().cmp(a.version()));
                            dep.summaries = Some(summaries.clone());

                            if let Some(pkgs) =
                                output.packages.get(&requested.1.package_name().to_string())
                            {
                                for pkg in pkgs {
                                    if requested.1.matches(pkg.summary()) {
                                        dep.resolved = Some(pkg.clone());
                                        break;
                                    }
                                }
                            }

                            if let (Some(resolved), Some(requested)) =
                                (dep.resolved.as_ref(), dep.requested.as_ref())
                            {
                                let installed = resolved.version().clone();
                                let req_version = requested.version_req();

                                //order summaries by version
                                //clear matched result from previous resolve
                                dep.matched_summary = None;
                                dep.latest_matched_summary = None;
                                dep.latest_summary = None;
                                for summary in summaries {
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
                            };
                            //send to render task
                            if let Err(e) = render_tx
                                .send(DecorationEvent::Dependency(
                                    doc.uri.clone(),
                                    dep.id.clone(),
                                    dep.range,
                                    dep.clone(),
                                ))
                                .await
                            {
                                error!("render tx send error: {}", e);
                            };
                            doc.dirty_dependencies.remove(&dep.id);
                        }

                        if doc.is_dependencies_dirty() {
                            debug!("dependencies still dirty: {:?}", doc.dirty_dependencies);
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
    doc: &Document,
    render_tx: &Sender<DecorationEvent>,
    cargo_tx: &Sender<Ctx>,
) {
    debug!("start_resolve triggered for URI: {:?}", doc.uri);

    //no need to resolve
    if !doc.is_dependencies_dirty() {
        debug!(
            "Dependencies are not dirty for URI: {:?}. No resolve needed.",
            doc.uri
        );
        return;
    }

    for v in doc.dirty_dependencies.keys() {
        if let Some(n) = doc.entry(v) {
            debug!(
                "Marking dependency '{}' as waiting for URI: {:?}",
                v, doc.uri
            );
            render_tx
                .send(DecorationEvent::DependencyWaiting(
                    doc.uri.clone(),
                    v.to_string(),
                    n.range,
                ))
                .await
                .unwrap();
        }
    }

    //resolve cargo dependencies
    let resolve_ctx = Ctx {
        uri: doc.canonical_uri.clone(),
        rev: doc.rev,
    };
    debug!(
        "Sending context to cargo_tx for URI: {:?}, rev: {}",
        resolve_ctx.uri, resolve_ctx.rev
    );
    if let Err(e) = cargo_tx.send(resolve_ctx).await {
        error!("cargo resolve tx error: {}", e);
    }
}

async fn reconsile_document<'a>(
    state: &'a mut Workspace,
    diagnostic_controller: &'a mut DiagnosticController,
    msg: &CargoTomlPayload,
    canonical_uri: &CanonicalUri,
) -> Option<&'a Document> {
    match state.reconsile(msg.uri.clone(), canonical_uri.clone(), &msg.text) {
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
