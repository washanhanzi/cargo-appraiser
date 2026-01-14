//! Audit event handler.

use tower_lsp::lsp_types::{Diagnostic, Uri};
use tracing::trace;

use super::super::audit::{AuditIssue, AuditReports};
use super::AppraiserContext;

/// Handle `CargoDocumentEvent::Audited` - process audit reports and add diagnostics.
pub async fn handle_audited(ctx: &mut AppraiserContext<'_>, reports: AuditReports) {
    trace!("[AUDIT] Received {} crate reports", reports.len());

    let Some(doc) = ctx.state.root_document() else {
        return;
    };

    // Collect all diagnostics to add
    let mut diagnostics_to_add: Vec<(Uri, String, Diagnostic)> = Vec::new();

    for issues in reports.values() {
        for issue in issues {
            for (crate_name, paths) in &issue.dependency_paths {
                collect_audit_diagnostics(doc, issue, crate_name, paths, &mut diagnostics_to_add);
            }
        }
    }

    // Now add all collected diagnostics
    for (uri, dep_id, diag) in diagnostics_to_add {
        ctx.diagnostic_controller
            .add_audit_diagnostic(&uri, &dep_id, diag)
            .await;
    }
}

fn collect_audit_diagnostics(
    doc: &crate::usecase::Document,
    issue: &AuditIssue,
    crate_name: &str,
    paths: &[String],
    diagnostics: &mut Vec<(Uri, String, Diagnostic)>,
) {
    let dependencies = doc.dependencies_by_crate_name(crate_name);
    if dependencies.is_empty() {
        return;
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

        if required_version == pkg.version {
            let Some(name_node) = doc.name_node(&dep.id) else {
                continue;
            };

            trace!("[AUDIT] Adding diagnostic for {}", dep.id);
            let diag = Diagnostic {
                range: name_node.range,
                severity: Some(issue.severity()),
                code: None,
                code_description: None,
                source: Some("cargo-appraiser".to_string()),
                message: issue.audit_text(Some(crate_name)),
                related_information: None,
                tags: None,
                data: None,
            };

            diagnostics.push((doc.uri.clone(), dep.id.clone(), diag));
        }
    }
}
