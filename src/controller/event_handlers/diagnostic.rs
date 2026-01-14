//! Diagnostic event handler.

use tracing::debug;

use crate::entity::{CanonicalUri, CargoError};

use super::AppraiserContext;

/// Handle `CargoDocumentEvent::CargoDiagnostic` - process cargo errors as diagnostics.
pub async fn handle_cargo_diagnostic(
    ctx: &mut AppraiserContext<'_>,
    uri: CanonicalUri,
    err: CargoError,
) {
    debug!(
        "Appraiser Event: CargoDiagnostic for URI: {:?}, Error: {:?}",
        uri, err
    );

    let Some(client_uri) = ctx.state.uri(&uri) else {
        return;
    };

    ctx.diagnostic_controller
        .clear_cargo_diagnostics(client_uri)
        .await;

    // We need a crate name to find something in toml
    let Some(crate_name) = err.crate_name() else {
        return;
    };

    let Some(doc) = ctx.state.document(&uri) else {
        return;
    };

    let keys = doc.find_keys_by_crate_name(crate_name);
    let deps = doc.find_deps_by_crate_name(crate_name);

    let Some(digs) = err.diagnostic(&keys, &deps, doc.tree()) else {
        return;
    };

    for (id, diag) in digs {
        ctx.diagnostic_controller
            .add_cargo_diagnostic(client_uri, id.as_str(), diag)
            .await;
    }
}
