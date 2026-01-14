//! Document lifecycle handlers (opened, saved, changed, closed, parse).

use tower_lsp::lsp_types::{Diagnostic, Uri};
use tracing::{debug, error};

use crate::{decoration::DecorationEvent, entity::CanonicalUri};

use super::super::{
    context::{AppraiserContext, CargoTomlPayload, Ctx},
    read_file::{ReadFile, ReadFileParam},
};

/// Handle `CargoDocumentEvent::Opened` or `CargoDocumentEvent::Saved`.
pub async fn handle_opened_saved(ctx: &mut AppraiserContext<'_>, msg: CargoTomlPayload) {
    debug!("Appraiser Event: Opened/Saved for URI: {:?}", msg.uri);

    let Ok(canonical_uri): Result<CanonicalUri, _> = msg.uri.clone().try_into() else {
        error!("failed to canonicalize uri: {}", msg.uri.as_str());
        return;
    };

    let (doc, errors) = ctx
        .state
        .update(msg.uri.clone(), canonical_uri.clone(), &msg.text);

    for e in &errors {
        let diag = parse_error_to_diagnostic(e);
        ctx.diagnostic_controller
            .add_parse_diagnostic(&msg.uri, &format!("parse_error_{}", e.message), diag)
            .await;
    }

    if !errors.is_empty() {
        return;
    }

    if !doc.is_dependencies_dirty() {
        return;
    }

    let doc_rev = doc.rev;

    if let Err(e) = ctx
        .debouncer
        .send_interactive(Ctx {
            uri: canonical_uri,
            rev: doc_rev,
        })
        .await
    {
        error!("debouncer send interactive error: {}", e);
    }
}

/// Handle `CargoDocumentEvent::Changed`.
pub async fn handle_changed(ctx: &mut AppraiserContext<'_>, msg: CargoTomlPayload) {
    debug!("Appraiser Event: Changed for URI: {:?}", msg.uri);

    ctx.diagnostic_controller
        .clear_parse_diagnostics(&msg.uri)
        .await;

    let Ok(canonical_uri) = TryInto::<CanonicalUri>::try_into(msg.uri.clone()) else {
        error!("failed to canonicalize uri: {}", msg.uri.as_str());
        return;
    };

    // When Cargo.toml changed, clear audit diagnostics
    ctx.diagnostic_controller.clear_audit_diagnostics().await;

    let (doc, errors) = ctx
        .state
        .update(msg.uri.clone(), canonical_uri.clone(), &msg.text);

    for e in &errors {
        let diag = parse_error_to_diagnostic(e);
        ctx.diagnostic_controller
            .add_parse_diagnostic(&msg.uri, &format!("parse_error_{}", e.message), diag)
            .await;
    }

    if !errors.is_empty() {
        return;
    }

    let doc_rev = doc.rev;

    if let Err(e) = ctx
        .debouncer
        .send_background(Ctx {
            uri: canonical_uri,
            rev: doc_rev,
        })
        .await
    {
        error!("debouncer send interactive error: {}", e);
    }
}

/// Handle `CargoDocumentEvent::Closed`.
pub async fn handle_closed(ctx: &mut AppraiserContext<'_>, uri: Uri) {
    debug!("Appraiser Event: Closed for URI: {:?}", uri);

    let Ok(canonical_uri) = uri.clone().try_into() else {
        error!("failed to canonicalize uri: {}", uri.as_str());
        return;
    };

    ctx.state.remove(&canonical_uri);
    debug!(
        "Document removed. Workspace now has {} documents",
        ctx.state.documents.len()
    );

    // Keep diagnostics - user may still view them in Problems panel
    if let Err(e) = ctx.render_tx.send(DecorationEvent::Reset(uri)).await {
        error!("render tx send reset error: {}", e);
    }
}

/// Handle `CargoDocumentEvent::Parse`.
pub async fn handle_parse(ctx: &mut AppraiserContext<'_>, uri: Uri) {
    debug!("Appraiser Event: Parse for URI: {:?}", uri);

    let Ok(canonical_uri) = TryInto::<CanonicalUri>::try_into(uri.clone()) else {
        error!("failed to canonicalize uri: {}", uri.as_str());
        return;
    };

    let content = if ctx.client_capabilities.can_read_file() {
        let param = ReadFileParam { uri: uri.clone() };
        match ctx.client.send_request::<ReadFile>(param).await {
            Ok(content) => content.content,
            Err(e) => {
                error!("read file error: {}", e);
                return;
            }
        }
    } else {
        // Read file with OS
        let Ok(path) = canonical_uri.to_path_buf() else {
            error!("failed to convert canonical uri to path: {}", uri.as_str());
            return;
        };
        match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(e) => {
                error!("read file error: {}", e);
                return;
            }
        }
    };

    // Parse and update state
    let (_doc, errors) = ctx
        .state
        .update(uri.clone(), canonical_uri.clone(), &content);

    for e in &errors {
        let diag = parse_error_to_diagnostic(e);
        ctx.diagnostic_controller
            .add_parse_diagnostic(&uri, &format!("parse_error_{}", e.message), diag)
            .await;
    }
}

/// Convert a parse error to an LSP Diagnostic.
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
