use std::path::Path;

fn audit_lockfile(path: &str) -> Option<rustsec::Report> {
    let mut config = cargo_audit::config::AuditConfig::default();
    config.database.stale = false;
    config.output.format = cargo_audit::config::OutputFormat::Json;
    let mut app = cargo_audit::auditor::Auditor::new(&config);
    let path = Path::new(path);
    app.audit_lockfile(path).ok()
}
