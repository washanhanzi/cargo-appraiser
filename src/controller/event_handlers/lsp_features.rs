//! LSP feature handlers (hover, goto definition, completion, code action).

use tokio::sync::oneshot;
use tower_lsp::lsp_types::{
    CodeActionResponse, CompletionResponse, GotoDefinitionResponse, Hover, Position, Range, Uri,
};
use tracing::error;

use super::super::{
    code_action::code_action, completion::completion, gd::goto_definition, hover::hover,
};
use super::AppraiserContext;

/// Handle `CargoDocumentEvent::Hovered` - provide hover information.
pub async fn handle_hover(
    ctx: &mut AppraiserContext<'_>,
    uri: Uri,
    pos: Position,
    tx: oneshot::Sender<Option<Hover>>,
) {
    let Ok(canonical_uri) = uri.clone().try_into() else {
        error!("failed to canonicalize uri: {}", uri.as_str());
        return;
    };

    let Some(doc) = ctx.state.document(&canonical_uri) else {
        return;
    };

    let Some(node) = doc.precise_match(pos) else {
        return;
    };

    // Find the dependency for this node
    let dep = doc.tree().find_dependency_at_position(pos);
    let resolved = dep.and_then(|d| doc.resolved(&d.id));
    let h = hover(node, dep, resolved, doc.members.as_deref());
    let _ = tx.send(h);
}

/// Handle `CargoDocumentEvent::Gded` - provide goto definition.
pub async fn handle_gd(
    ctx: &mut AppraiserContext<'_>,
    uri: Uri,
    pos: Position,
    tx: oneshot::Sender<Option<GotoDefinitionResponse>>,
) {
    let Ok(canonical_uri) = uri.clone().try_into() else {
        error!("failed to canonicalize uri: {}", uri.as_str());
        return;
    };

    let Some(doc) = ctx.state.document(&canonical_uri) else {
        return;
    };

    let Some(node) = doc.precise_match(pos) else {
        return;
    };

    let gd = goto_definition(ctx.state, doc, node);
    let _ = tx.send(gd);
}

/// Handle `CargoDocumentEvent::Completion` - provide completion items.
pub async fn handle_completion(
    ctx: &mut AppraiserContext<'_>,
    uri: Uri,
    pos: Position,
    tx: oneshot::Sender<Option<CompletionResponse>>,
) {
    let Ok(canonical_uri) = uri.clone().try_into() else {
        error!("failed to canonicalize uri: {}", uri.as_str());
        return;
    };

    let Some(doc) = ctx.state.document(&canonical_uri) else {
        return;
    };

    let Some(node) = doc.precise_match(pos) else {
        return;
    };

    let dep = doc.tree().find_dependency_at_position(pos);
    let resolved = dep.and_then(|d| doc.resolved(&d.id));
    let comp = completion(ctx.http_client, node, dep, resolved).await;
    let _ = tx.send(comp);
}

/// Handle `CargoDocumentEvent::CodeAction` - provide code actions.
pub async fn handle_code_action(
    ctx: &mut AppraiserContext<'_>,
    uri: Uri,
    range: Range,
    tx: oneshot::Sender<CodeActionResponse>,
) {
    let Ok(canonical_uri) = uri.clone().try_into() else {
        error!("failed to canonicalize uri: {}", uri.as_str());
        return;
    };

    let Some(doc) = ctx.state.document(&canonical_uri) else {
        return;
    };

    let Some(node) = doc.precise_match(range.start) else {
        return;
    };

    let tree = doc.tree();
    let dep = tree.find_dependency_at_position(range.start);
    let resolved = dep.and_then(|d| doc.resolved(&d.id));

    let Some(action) = code_action(uri, tree, node, dep, resolved) else {
        return;
    };

    let _ = tx.send(action);
}
