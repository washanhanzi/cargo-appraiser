use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use petgraph::{
    prelude::NodeIndex,
    visit::{Dfs, EdgeRef, IntoEdgesDirected},
    Direction,
};
use rustsec::Lockfile;
use tower_lsp::lsp_types::Uri;

#[derive(Default)]
pub struct AuditController {
    pub reports: HashMap<PathBuf, Vec<AuditResult>>,
}

#[derive(Debug, Clone)]
pub struct AuditResult {
    pub warning: Option<rustsec::Warning>,
    pub vuln: Option<rustsec::Vulnerability>,
    pub workspace_member: cargo::core::package::Package,
    pub dep_package: cargo_lock::Package,
    pub tree: Vec<String>,
}

impl AuditController {
    pub fn new() -> Self {
        Self {
            reports: HashMap::new(),
        }
    }

    pub fn audit_lockfile(
        &mut self,
        toml_uri: &Uri,
        members: &[&cargo::core::package::Package],
    ) -> Result<(), anyhow::Error> {
        let mut config = cargo_audit::config::AuditConfig::default();
        config.database.stale = false;
        config.output.format = cargo_audit::config::OutputFormat::Json;
        let mut app = cargo_audit::auditor::Auditor::new(&config);
        let lock_file_path_str = toml_uri.path().as_str().replace(".toml", ".lock");
        let lock_file_path = Path::new(&lock_file_path_str);
        let report = app.audit_lockfile(lock_file_path)?;

        let lockfile = Lockfile::load(lock_file_path)?;
        let tree = lockfile.dependency_tree()?;
        let graph = tree.graph();

        let mut members_map = HashMap::new();
        for m in members {
            members_map.insert((m.name().to_string(), m.version().to_string()), m);
        }

        let mut warnings_map: HashMap<NodeIndex, rustsec::Warning> = HashMap::new();
        let mut vulns_map: HashMap<NodeIndex, rustsec::Vulnerability> = HashMap::new();

        for (_, warnings) in &report.warnings {
            for w in warnings {
                let p = w.package.clone();

                //this is the warning's package node index
                let package_node_indx = tree.nodes()[&cargo_lock::Dependency::from(&p)];
                warnings_map.insert(package_node_indx, w.clone());
            }
        }

        for vul in &report.vulnerabilities.list {
            let p = vul.package.clone();
            let package_node_indx = tree.nodes()[&cargo_lock::Dependency::from(&p)];
            vulns_map.insert(package_node_indx, vul.clone());
        }

        //try walk from the warning's package node index to the root node
        //record the root and the direct dep to the root

        for (i, w) in warnings_map {
            let mut stack = vec![i];
            let mut visited = HashSet::new();
            let mut path_map: HashMap<NodeIndex, Vec<String>> = HashMap::new();

            // Initialize path for starting node
            path_map.insert(i, vec![graph.node_weight(i).unwrap().name.to_string()]);

            while let Some(current) = stack.pop() {
                if !visited.insert(current) {
                    continue;
                }

                let current_path = path_map.get(&current).unwrap().clone();

                // For each incoming edge, extend the path
                for edge in graph.edges_directed(current, Direction::Incoming) {
                    let source = edge.source();
                    let source_pkg = graph.node_weight(source).unwrap();

                    // Create new path by adding the source package
                    let mut new_path = current_path.clone();
                    new_path.insert(0, source_pkg.name.to_string());
                    path_map.insert(source, new_path);

                    if let Some(m) = members_map
                        .get(&(source_pkg.name.to_string(), source_pkg.version.to_string()))
                    {
                        let current_pkg = graph.node_weight(current).unwrap();
                        self.reports
                            .entry(m.root().to_path_buf())
                            .or_default()
                            .push(AuditResult {
                                warning: Some(w.clone()),
                                vuln: None,
                                workspace_member: (**m).clone(),
                                dep_package: current_pkg.clone(),
                                tree: path_map.get(&source).unwrap().clone(),
                            });
                    }
                    stack.push(source);
                }
            }
        }

        for (root, results) in &self.reports {
            for r in results {
                println!(
                    "warning: {} -> {} -> {}: {}",
                    root.display(),
                    r.dep_package.name,
                    r.warning.as_ref().unwrap().package.name,
                    r.tree.join(" -> ")
                );
            }
        }

        Ok(())
    }
}

mod tests {
    use std::str::FromStr;

    use super::*;
    #[test]
    fn test_audit_lockfile() {
        let path = Path::new("/Users/jingyu/Github/tauri/Cargo.toml");
        //get dependencies
        let gctx = cargo::util::context::GlobalContext::default().unwrap();
        let workspace = cargo::core::Workspace::new(path, &gctx).unwrap();
        let mut audit = AuditController::default();
        let m: Vec<&cargo::core::package::Package> = workspace.members().collect();
        audit
            .audit_lockfile(
                &Uri::from_str("/Users/jingyu/Github/tauri/Cargo.toml").unwrap(),
                &m,
            )
            .unwrap();
    }
}
