use std::path::Path;

fn test() {
    let mut config = cargo_audit::config::AuditConfig::default();
    config.database.stale = false;
    config.output.format = cargo_audit::config::OutputFormat::Json;
    let mut app = cargo_audit::auditor::Auditor::new(&config);
    let path = Path::new("/Users/jingyu/tmp/hello-rust/Cargo.lock");
    let result = app.audit_lockfile(path).unwrap();
    let s = serde_json::to_string_pretty(&result).unwrap();
    println!("{}", s);
}

mod tests {
    #[test]
    fn test() {
        super::test();
    }
}
