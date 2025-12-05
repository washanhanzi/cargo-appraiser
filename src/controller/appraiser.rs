use std::{env, str::FromStr};

use cargo::core::PackageIdSpec;
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
    decoration::{DecorationEvent, DecorationItem, DecorationState},
    entity::{CanonicalUri, CargoError},
    usecase::{Document, Workspace},
};

use super::{
    audit::{AuditController, AuditReports},
    capabilities::{ClientCapabilities, ClientCapability},
    cargo::{cargo_resolve, make_lookup_key, CargoResolveOutput},
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
                            .send(CargoDocumentEvent::CargoResolved(output))
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
                        debug!("[AUDIT] Received {} crate reports", reports.len());
                        let Some(doc) = state.root_document() else {
                            continue;
                        };
                        for issues in reports.values() {
                            for issue in issues {
                                for (crate_name, paths) in &issue.dependency_paths {
                                    let dependencies = doc.dependencies_by_crate_name(crate_name);
                                    if dependencies.is_empty() {
                                        continue;
                                    }
                                    for dep in dependencies {
                                        let Some(resolved) = doc.resolved(&dep.id) else {
                                            continue;
                                        };
                                        let Some(pkg) = resolved.package.as_ref() else {
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
                                        if required_version == pkg.version().to_string() {
                                            let Some(entry_node) = doc.entry(&dep.id) else {
                                                continue;
                                            };
                                            debug!("[AUDIT] Adding diagnostic for {}", dep.id);
                                            let diag = Diagnostic {
                                                range: entry_node.range,
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
                            .clear_cargo_diagnostics(client_uri)
                            .await;
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
                                .add_cargo_diagnostic(client_uri, id.as_str(), diag)
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
                        // Find the dependency for this node
                        let dep = doc.tree().find_dependency_at_position(pos);
                        let resolved = dep.and_then(|d| doc.resolved(&d.id));
                        let h = hover(node, dep, resolved, doc.members.as_deref());
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
                        let gd = goto_definition(&state, doc, node);
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
                        let dep = doc.tree().find_dependency_at_position(pos);
                        let resolved = dep.and_then(|d| doc.resolved(&d.id));
                        let completion = completion(node, dep, resolved).await;
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
                        let dep = doc.tree().find_dependency_at_position(range.start);
                        let resolved = dep.and_then(|d| doc.resolved(&d.id));
                        let Some(action) = code_action(uri, node, dep, resolved) else {
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
                        let doc = match state.update(
                            msg.uri.clone(),
                            canonical_uri.clone(),
                            &msg.text,
                        ) {
                            Ok(doc) => doc,
                            Err(err) => {
                                for e in err {
                                    let diag = parse_error_to_diagnostic(&e);
                                    diagnostic_controller
                                        .add_parse_diagnostic(&msg.uri, &format!("parse_error_{}", e.message), diag)
                                        .await;
                                }
                                continue;
                            }
                        };
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
                            "Appraiser Event: CargoResolved for URI: {:?}, rev: {}. Index entries: {}",
                            output.ctx.uri,
                            output.ctx.rev,
                            output.index.len()
                        );
                        //resolve virtual manifest if we haven't
                        let root_manifest_uri = output.root_manifest_uri.clone();
                        if state.document(&root_manifest_uri).is_none() {
                            let uri = Uri::from_str(root_manifest_uri.as_str()).unwrap();
                            if let Err(e) = inner_tx.send(CargoDocumentEvent::Parse(uri)).await {
                                error!("inner tx send error: {}", e);
                            }
                        }
                        state.root_manifest_uri = Some(root_manifest_uri.clone());

                        // Build specs from index for audit
                        let specs: Vec<PackageIdSpec> = output.index.iter()
                            .filter_map(|(_, resolved)| {
                                resolved.package.as_ref().map(|p| p.package_id().to_spec())
                            })
                            .collect();
                        state.specs = specs.clone();
                        state.member_manifest_uris = output.member_manifest_uris.clone();

                        //send audit event
                        if !GLOBAL_CONFIG.read().unwrap().audit.disabled {
                            debug!("[AUDIT] Sending audit request");
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

                        // Track which deps to remove from dirty after processing
                        let mut resolved_dep_ids: Vec<String> = Vec::new();

                        // Populate resolution info for each dependency
                        let dep_ids: Vec<String> = doc.dependency_ids().cloned().collect();
                        for dep_id in dep_ids {
                            let Some(rev) = doc.dirty_dependencies.get(&dep_id) else {
                                continue;
                            };
                            if *rev > output.ctx.rev {
                                continue;
                            }

                            let Some(dep) = doc.dependency(&dep_id) else {
                                resolved_dep_ids.push(dep_id.clone());
                                continue;
                            };

                            // Create lookup key and get resolution from index
                            let lookup_key = make_lookup_key(dep);
                            if let Some(resolved) = output.index.get(&lookup_key) {
                                doc.set_resolved(&dep_id, resolved.clone());
                            }

                            resolved_dep_ids.push(dep_id);
                        }

                        // Remove resolved deps from dirty
                        for id in &resolved_dep_ids {
                            doc.mark_resolved(id);
                        }

                        // Build full update with all dependencies
                        let items: Vec<DecorationItem> = doc
                            .dependencies()
                            .filter_map(|dep| {
                                let entry = doc.entry(&dep.id)?;
                                let state = if doc.dirty_dependencies.contains_key(&dep.id) {
                                    DecorationState::Waiting
                                } else {
                                    let resolved = doc.resolved(&dep.id);
                                    DecorationState::Resolved {
                                        dep: dep.clone(),
                                        resolved: resolved.cloned(),
                                    }
                                };
                                Some(DecorationItem {
                                    id: dep.id.clone(),
                                    range: entry.range,
                                    state,
                                })
                            })
                            .collect();

                        if let Err(e) = render_tx
                            .send(DecorationEvent::Update(doc.uri.clone(), items))
                            .await
                        {
                            error!("render tx send error: {}", e);
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

    // Build a full update with waiting states for dirty deps and resolved states for clean deps
    let items: Vec<DecorationItem> = doc
        .dependencies()
        .filter_map(|dep| {
            let entry = doc.entry(&dep.id)?;
            let state = if doc.dirty_dependencies.contains_key(&dep.id) {
                debug!(
                    "Marking dependency '{}' as waiting for URI: {:?}",
                    dep.id, doc.uri
                );
                DecorationState::Waiting
            } else {
                let resolved = doc.resolved(&dep.id);
                DecorationState::Resolved {
                    dep: dep.clone(),
                    resolved: resolved.cloned(),
                }
            };
            Some(DecorationItem {
                id: dep.id.clone(),
                range: entry.range,
                state,
            })
        })
        .collect();

    if let Err(e) = render_tx
        .send(DecorationEvent::Update(doc.uri.clone(), items))
        .await
    {
        error!("render tx send error: {}", e);
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
    match state.update(msg.uri.clone(), canonical_uri.clone(), &msg.text) {
        Ok(doc) => {
            if !doc.is_dependencies_dirty() {
                None
            } else {
                Some(doc)
            }
        }
        Err(err) => {
            for e in err {
                let diag = parse_error_to_diagnostic(&e);
                diagnostic_controller
                    .add_parse_diagnostic(&msg.uri, &format!("parse_error_{}", e.message), diag)
                    .await;
            }
            None
        }
    }
}

fn parse_error_to_diagnostic(e: &toml_parser::ParseError) -> Diagnostic {
    Diagnostic {
        range: e.range,
        severity: Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR),
        code: None,
        code_description: None,
        source: Some("cargo-appraiser".to_string()),
        message: e.message.clone(),
        related_information: None,
        tags: None,
        data: None,
    }
}
