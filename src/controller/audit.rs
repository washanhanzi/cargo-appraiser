use std::{collections::HashMap, path::Path};

use tower_lsp::lsp_types::Uri;

pub struct AuditController {
    pub reports: HashMap<Uri, rustsec::Report>,
}

fn audit_lockfile(toml_uri: &Uri) -> Option<rustsec::Report> {
    let mut config = cargo_audit::config::AuditConfig::default();
    config.database.stale = false;
    config.output.format = cargo_audit::config::OutputFormat::Json;
    let mut app = cargo_audit::auditor::Auditor::new(&config);
    let lock_file_path = toml_uri.path().as_str().replace(".toml", ".lock");
    app.audit_lockfile(Path::new(&lock_file_path)).ok()
}
