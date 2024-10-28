use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use cargo_lock::Checksum;
use petgraph::{
    visit::{Dfs, EdgeRef},
    Direction,
};
use rustsec::Lockfile;
use tower_lsp::lsp_types::Uri;

#[derive(Default)]
pub struct AuditController {
    pub reports: HashMap<Uri, rustsec::Report>,
}
impl AuditController {
    pub fn audit_lockfile(&mut self, toml_uri: &Uri) -> Result<(), anyhow::Error> {
        let mut config = cargo_audit::config::AuditConfig::default();
        config.database.stale = false;
        config.output.format = cargo_audit::config::OutputFormat::Json;
        let mut app = cargo_audit::auditor::Auditor::new(&config);
        let lock_file_path_str = toml_uri.path().as_str().replace(".toml", ".lock");
        let lock_file_path = Path::new(&lock_file_path_str);
        let report = app.audit_lockfile(lock_file_path)?;

        // Record warnings and vulnerabilities with details
        let mut warn_checksums: HashMap<Checksum, &rustsec::Warning> = HashMap::new();
        let mut warn_name_vers: HashMap<(String, String), &rustsec::Warning> = HashMap::new();
        let mut vuln_checksums: HashMap<Checksum, &rustsec::Vulnerability> = HashMap::new();
        let mut vuln_name_vers: HashMap<(String, String), &rustsec::Vulnerability> = HashMap::new();

        // Record warnings
        println!("warnings: {:?}", report.warnings.len());
        for (_, warnings) in &report.warnings {
            for w in warnings {
                if let Some(checksum) = &w.package.checksum {
                    warn_checksums.insert(checksum.clone(), w);
                } else {
                    warn_name_vers.insert(
                        (w.package.name.to_string(), w.package.version.to_string()),
                        w,
                    );
                }
            }
        }

        // Record vulnerabilities
        println!("vulnerabilities: {:?}", report.vulnerabilities.list.len());
        for v in &report.vulnerabilities.list {
            if let Some(checksum) = &v.package.checksum {
                vuln_checksums.insert(checksum.clone(), v);
            } else {
                vuln_name_vers.insert(
                    (v.package.name.to_string(), v.package.version.to_string()),
                    v,
                );
            }
        }

        let lockfile = Lockfile::load(lock_file_path)?;
        let tree = lockfile.dependency_tree()?;
        let roots = tree.roots();
        let graph = tree.graph();

        // Maps for storing results
        let mut warning_deps_map: HashMap<String, Vec<rustsec::Warning>> = HashMap::new();
        let mut vuln_deps_map: HashMap<String, Vec<rustsec::Vulnerability>> = HashMap::new();
        let mut current_direct_dep: Option<String> = None;

        for root in &roots {
            let mut dfs = Dfs::new(&graph, *root);

            while let Some(node_idx) = dfs.next(&graph) {
                //if it's root, continue
                if node_idx == *root {
                    continue;
                }

                let node = graph.node_weight(node_idx).unwrap();

                // Check if this is a direct dependency of root
                for edge in graph.edges_directed(node_idx, Direction::Incoming) {
                    if edge.source() == *root {
                        current_direct_dep = Some(node.name.to_string());
                    }
                }

                // Check if node is vulnerable or has warnings
                let warning_info = if let Some(check) = &node.checksum {
                    warn_checksums.get(check)
                } else {
                    warn_name_vers.get(&(node.name.to_string(), node.version.to_string()))
                };

                let vuln_info = if let Some(check) = &node.checksum {
                    vuln_checksums.get(check)
                } else {
                    vuln_name_vers.get(&(node.name.to_string(), node.version.to_string()))
                };

                if let Some(direct_dep) = &current_direct_dep {
                    if let Some(warning_info) = warning_info {
                        warning_deps_map
                            .entry(direct_dep.clone())
                            .or_insert_with(Vec::new)
                            .push((*warning_info).clone());
                    }
                    if let Some(vuln_info) = vuln_info {
                        vuln_deps_map
                            .entry(direct_dep.clone())
                            .or_insert_with(Vec::new)
                            .push((*vuln_info).clone());
                    }
                }
            }
        }

        for (k, v) in warning_deps_map {
            println!("warning: {} -> {:?}", k, v);
        }
        for (k, v) in vuln_deps_map {
            println!("vuln: {} -> {:?}", k, v);
        }

        Ok(())
    }
}

mod tests {
    use std::str::FromStr;

    use super::*;
    #[test]
    fn test_audit_lockfile() {
        let mut audit = AuditController::default();
        audit
            .audit_lockfile(&Uri::from_str("/Users/jingyu/Github/tauri/Cargo.toml").unwrap())
            .unwrap();
    }
}
