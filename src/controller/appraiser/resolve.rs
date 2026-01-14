//! Cargo resolve handlers (ReadyToResolve, CargoResolved, CargoLockChanged).

use std::str::FromStr;

use tokio::sync::mpsc::Sender;
use tower_lsp::lsp_types::Uri;
use tracing::{debug, error, trace};

use crate::{
    config::GLOBAL_CONFIG,
    decoration::{DecorationEvent, DecorationItem, DecorationState},
    usecase::Document,
};

use super::super::{
    cargo::{make_lookup_key, CargoResolveOutput},
    context::{AppraiserContext, CargoDocumentEvent, Ctx},
};

/// Handle `CargoDocumentEvent::ReadyToResolve`.
pub async fn handle_ready_to_resolve(ctx: &mut AppraiserContext<'_>, event_ctx: Ctx) {
    debug!(
        "Appraiser Event: ReadyToResolve for URI: {:?}, rev: {}",
        event_ctx.uri, event_ctx.rev
    );

    if ctx.state.check_rev(&event_ctx.uri, event_ctx.rev) {
        let Some(doc) = ctx.state.document(&event_ctx.uri) else {
            return;
        };
        start_resolve(doc, ctx.render_tx, ctx.cargo_tx).await;
    }
}

/// Handle `CargoDocumentEvent::CargoLockChanged`.
pub async fn handle_cargo_lock_changed(ctx: &mut AppraiserContext<'_>) {
    debug!("Appraiser Event: CargoLockChanged");

    // Clear audit diagnostics and reset audit timer since lock file changed
    ctx.diagnostic_controller.clear_audit_diagnostics().await;

    if let Err(e) = ctx.audit_controller.reset().await {
        error!("audit controller reset error: {}", e);
    }

    // Clear state except the "current" uri
    let uris = ctx.state.mark_all_dirty();
    for (uri, rev) in uris {
        if let Err(e) = ctx.debouncer.send_background(Ctx { uri, rev }).await {
            error!("debouncer send interactive error: {}", e);
        }
    }
}

/// Handle `CargoDocumentEvent::CargoResolved`.
pub async fn handle_cargo_resolved(ctx: &mut AppraiserContext<'_>, output: CargoResolveOutput) {
    debug!(
        "Appraiser Event: CargoResolved for URI: {:?}, rev: {}. Index entries: {}",
        output.ctx.uri,
        output.ctx.rev,
        output.index.len()
    );

    // Check if originating document still exists and has matching rev
    // Skip processing if document was closed
    if ctx.state.document(&output.ctx.uri).is_none() {
        debug!(
            "Skipping CargoResolved - document was closed: {:?}",
            output.ctx.uri
        );
        return;
    }

    // Resolve virtual manifest if we haven't
    let root_manifest_uri = output.root_manifest_uri.clone();
    if ctx.state.document(&root_manifest_uri).is_none() {
        let uri = Uri::from_str(root_manifest_uri.as_str()).unwrap();
        if let Err(e) = ctx.inner_tx.send(CargoDocumentEvent::Parse(uri)).await {
            error!("inner tx send error: {}", e);
        }
    }
    ctx.state.root_manifest_uri = Some(root_manifest_uri.clone());

    // Build member names for audit
    let member_names: Vec<String> = output.members.iter().map(|m| m.name.clone()).collect();
    ctx.state.member_names = member_names.clone();
    ctx.state.member_manifest_uris = output.member_manifest_uris.clone();

    // Send audit event
    if !GLOBAL_CONFIG.read().unwrap().audit.disabled {
        trace!("[AUDIT] Sending audit request");
        if let Err(e) = ctx
            .audit_controller
            .send(
                root_manifest_uri,
                ctx.state.member_names.clone(),
                ctx.cargo_path,
            )
            .await
        {
            error!("audit controller send error: {}", e);
        };
    }

    let Some(doc) = ctx
        .state
        .document_mut_with_rev(&output.ctx.uri, output.ctx.rev)
    else {
        return;
    };

    // Set workspace members for hover support
    doc.members = Some(output.members.clone());

    ctx.diagnostic_controller
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
        // For workspace dependencies, use name-only lookup since the table
        // in toml-parser (always Dependencies) may not match how member
        // packages actually use the dependency
        let resolved = if doc.is_workspace_dep(dep) {
            output
                .index
                .find_by_name(dep.package_name(), dep.platform.as_deref())
        } else {
            let lookup_key = make_lookup_key(dep);
            output.index.get(&lookup_key)
        };

        if let Some(resolved) = resolved {
            debug!("Setting resolved for dep_id={}", dep_id);
            doc.set_resolved(&dep_id, resolved.clone());
        } else {
            debug!(
                "No resolution found for dep_id={}, package={}",
                dep_id,
                dep.package_name()
            );
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

    if let Err(e) = ctx
        .render_tx
        .send(DecorationEvent::Update(doc.uri.clone(), items))
        .await
    {
        error!("render tx send error: {}", e);
    }

    if doc.is_dependencies_dirty() {
        debug!("dependencies still dirty: {:?}", doc.dirty_dependencies);
        if let Err(e) = ctx
            .debouncer
            .send_background(Ctx {
                uri: output.ctx.uri,
                rev: doc.rev,
            })
            .await
        {
            error!("debouncer send background error: {}", e);
        }
    }
}

/// Start the cargo resolve process for a document.
async fn start_resolve(
    doc: &Document,
    render_tx: &Sender<DecorationEvent>,
    cargo_tx: &Sender<Ctx>,
) {
    debug!("start_resolve triggered for URI: {:?}", doc.uri);

    // No need to resolve
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

    // Resolve cargo dependencies
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
