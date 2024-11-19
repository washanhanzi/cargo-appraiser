use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    pin::Pin,
    str::FromStr,
    time::Duration,
};

use petgraph::prelude::NodeIndex;
use tokio::{
    sync::mpsc::{self, error::SendError, Sender},
    time::Sleep,
};
use tower_lsp::lsp_types::{DiagnosticSeverity, Uri};
use tracing::{error, info};

use crate::entity::into_file_uri_str;

use super::CargoDocumentEvent;

//pathBuf is the workspace member Cargo.toml path, the inside hashpmap has dependency Name and Version as key
#[derive(Debug, Clone)]
pub struct AuditReports {
    pub root: Uri,
    pub members: HashMap<PathBuf, HashMap<String, HashMap<String, Vec<AuditResult>>>>,
}

pub struct AuditController {
    tx: Sender<CargoDocumentEvent>,
    sender: Option<Sender<Uri>>,
}

#[derive(Debug, Clone)]
pub struct AuditResult {
    pub warning: Option<rustsec::Warning>,
    pub vuln: Option<rustsec::Vulnerability>,
    pub tree: Vec<Vec<String>>,
}

impl AuditResult {
    pub fn severity(&self) -> DiagnosticSeverity {
        if self.vuln.is_some() {
            return DiagnosticSeverity::ERROR;
        }
        if self.warning.is_some() {
            return DiagnosticSeverity::WARNING;
        }
        DiagnosticSeverity::INFORMATION
    }

    pub fn audit_text(&self) -> String {
        if let Some(vuln) = &self.vuln {
            return format!(
                "# {}\n\n\
                {}\n\n\
                * Package: {} {}\n\
                * ID: {}\n\
                {}\n\n\
                ",
                vuln.advisory.title,
                vuln.advisory.description,
                vuln.package.name,
                vuln.package.version,
                vuln.advisory.id,
                vuln.advisory
                    .url
                    .as_ref()
                    .map_or("".to_string(), |url| format!("* Url: {}", url))
            );
        }
        if let Some(warning) = &self.warning {
            return format!(
                "# Warning: {} {}\n\
                {}\n\n\
                ",
                warning.package.name, warning.package.version, warning.kind,
            );
        }
        String::new()
    }
}

pub fn into_diagnostic_text(reports: &[AuditResult]) -> String {
    let mut s = String::new();
    let mut tree = String::new();
    for r in reports {
        s.push_str(&r.audit_text());
        tree.push_str(
            r.tree
                .iter()
                .map(|path| format!("- {}\n", path.join(" -> ")))
                .collect::<Vec<_>>()
                .join("\n")
                .as_str(),
        );
    }
    s.push_str("# Dependency Paths:\n\n");
    s.push_str(&tree);
    s
}

pub fn into_diagnostic_severity(
    reports: &[AuditResult],
) -> tower_lsp::lsp_types::DiagnosticSeverity {
    reports
        .iter()
        .map(|r| r.severity())
        .min()
        .unwrap_or(DiagnosticSeverity::INFORMATION)
}

impl AuditController {
    pub fn new(tx: Sender<CargoDocumentEvent>) -> Self {
        Self { tx, sender: None }
    }

    pub async fn send(&self, uri: &Uri) -> Result<(), SendError<Uri>> {
        self.sender.as_ref().unwrap().send(uri.clone()).await
    }

    pub fn spawn(&mut self) {
        //create a mpsc channel
        let (internal_tx, mut internal_rx) = mpsc::channel(32);
        let mut received_uri = None;
        self.sender = Some(internal_tx);
        let tx = self.tx.clone();
        let mut timer: Option<Pin<Box<Sleep>>> = None;
        //spawn a task to listen to the channel
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(uri) = internal_rx.recv() => {
                        if received_uri.is_none() {
                            if uri.path().as_str().ends_with(".lock") {
                                received_uri = Some(
                                    Uri::from_str(&uri.path().as_str().replace(".lock", ".toml").to_string())
                                        .unwrap(),
                                );
                            } else {
                                received_uri = Some(uri);
                            }
                        }
                        timer = Some(Box::pin(tokio::time::sleep(Duration::from_secs(60))));
                    }
                    () = async {
                        if let Some(ref mut t) = timer {
                            t.await
                        } else {
                            futures::future::pending::<()>().await
                        }
                    }, if timer.is_some() => {
                        timer = None;
                        let mut audited_worksapce=None;
                        let uri = received_uri.take().unwrap();
                        let reports = match audit_workspace(&uri, &mut audited_worksapce) {
                            Ok(r) => r,
                            Err(e) => {
                                error!("Failed to audit workspace {}: {}", uri.path(), e);
                                continue;
                            }
                        };
                        if let Err(e) = tx.send(CargoDocumentEvent::Audited(reports)).await {
                            error!("failed to send Audited event: {}", e);
                        }
                    }
                }
            }
        });
    }
}

//uri should be a Cargo.toml file
pub fn audit_workspace(
    uri: &Uri,
    audited: &mut Option<String>,
) -> Result<AuditReports, anyhow::Error> {
    let gctx = cargo::util::context::GlobalContext::default()?;
    let path = Path::new(uri.path().as_str());
    let workspace = cargo::core::Workspace::new(path, &gctx)?;

    let root = workspace.lock_root().display().to_string();
    let root_uri = into_file_uri_str(&(root.to_string() + "/Cargo.toml"));
    let lock = root + "/Cargo.lock";
    //if audited is some and eq to lock, return
    if let Some(audited_lock) = audited {
        if *audited_lock == lock {
            return Ok(AuditReports {
                root: root_uri,
                members: HashMap::new(),
            });
        }
    }
    *audited = Some(lock.to_string());

    let mut config = cargo_audit::config::AuditConfig::default();
    config.database.stale = false;
    config.output.format = cargo_audit::config::OutputFormat::Json;
    config.output.quiet = true;
    config.output.disable_print_report = true;
    let mut app = cargo_audit::auditor::Auditor::new(&config);
    let lock_file_path = Path::new(&lock);
    let report = app.audit_lockfile(lock_file_path)?;

    let lockfile = cargo_lock::Lockfile::load(lock_file_path)?;
    let tree = lockfile.dependency_tree()?;
    let graph = tree.graph();

    let mut members_map = HashMap::new();
    for m in workspace.members() {
        members_map.insert((m.name().to_string(), m.version().to_string()), m);
    }

    let mut members_index_map = HashMap::new();
    let roots = tree.roots();
    for r in roots {
        let mut dfs = petgraph::visit::Dfs::new(&graph, r);
        while let Some(nx) = dfs.next(&graph) {
            let node = graph.node_weight(nx).unwrap();
            if members_map.contains_key(&(node.name.to_string(), node.version.to_string())) {
                members_index_map.insert(
                    nx,
                    members_map
                        .get(&(node.name.to_string(), node.version.to_string()))
                        .unwrap(),
                );
            }
        }
    }

    let mut warnings_map: HashMap<NodeIndex, rustsec::Warning> = HashMap::new();
    let mut vulns_map: HashMap<NodeIndex, rustsec::Vulnerability> = HashMap::new();

    for warnings in report.warnings.values() {
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
    let mut reports = AuditReports {
        root: root_uri,
        members: HashMap::new(),
    };

    // Iterate over each warning to find paths to workspace members
    for (warning_node, warning) in warnings_map {
        let mut tree_map = HashMap::new();
        // For each workspace member, find all paths from the member to the warning node
        for (dep_key, member) in &members_index_map {
            // Find the node index of the workspace member
            //use dfs to fin the member node

            // Use petgraph's all_simple_paths to find all paths from dep_node to warning_node
            // Note: Adjust the direction based on the actual edge direction in your graph
            // Assuming edges point from dependent to dependency
            let paths = petgraph::algo::all_simple_paths::<Vec<_>, _>(
                &graph,
                *dep_key,
                warning_node,
                0,        // Start depth
                Some(10), // Maximum depth to prevent excessive paths
            );

            for path in paths {
                // Convert NodeIndex path to package names
                // remove the first element
                let mut tree_path: Vec<String> =
                    path.iter().map(|n| graph[*n].name.to_string()).collect();
                tree_path.remove(0);

                // The last node is the warning node's direct dependency package
                // we need to group the same dep_package and make tree a Vec<Vec<String>>
                let dep_package = graph[path[1]].clone();
                tree_map
                    .entry((
                        member.root().to_path_buf(),
                        dep_package.name.to_string(),
                        dep_package.version.to_string(),
                    ))
                    .or_insert(vec![])
                    .push(tree_path);
            }
        }
        for ((path, name, version), tree) in tree_map {
            reports
                .members
                .entry(path)
                .or_default()
                .entry(name)
                .or_default()
                .entry(version)
                .or_default()
                .push(AuditResult {
                    warning: Some(warning.clone()),
                    vuln: None,
                    tree,
                });
        }
    }
    for (warning_node, vuln) in vulns_map {
        let mut tree_map = HashMap::new();
        // For each workspace member, find all paths from the member to the warning node
        for (dep_key, member) in &members_index_map {
            // Find the node index of the workspace member
            //use dfs to fin the member node

            // Use petgraph's all_simple_paths to find all paths from dep_node to warning_node
            // Note: Adjust the direction based on the actual edge direction in your graph
            // Assuming edges point from dependent to dependency
            let paths = petgraph::algo::all_simple_paths::<Vec<_>, _>(
                &graph,
                *dep_key,
                warning_node,
                0,        // Start depth
                Some(10), // Maximum depth to prevent excessive paths
            );

            for path in paths {
                // Convert NodeIndex path to package names
                // remove the first element
                let mut tree_path: Vec<String> =
                    path.iter().map(|n| graph[*n].name.to_string()).collect();
                tree_path.remove(0);

                // The last node is the warning node's direct dependency package
                // we need to group the same dep_package and make tree a Vec<Vec<String>>
                let dep_package = graph[path[1]].clone();
                tree_map
                    .entry((
                        member.root().to_path_buf(),
                        dep_package.name.to_string(),
                        dep_package.version.to_string(),
                    ))
                    .or_insert(vec![])
                    .push(tree_path);
            }
        }
        for ((path, name, version), tree) in tree_map {
            reports
                .members
                .entry(path)
                .or_default()
                .entry(name)
                .or_default()
                .entry(version)
                .or_default()
                .push(AuditResult {
                    warning: None,
                    vuln: Some(vuln.clone()),
                    tree,
                });
        }
    }
    Ok(reports)
}

mod tests {
    use crate::entity::into_file_uri;

    use super::*;
    #[test]
    fn test_audit_lockfile() {
        let path = Path::new("/Users/jingyu/Github/tauri/Cargo.toml");
        let uri = into_file_uri(path);
        let mut audited = None;
        let audit = audit_workspace(&uri, &mut audited).unwrap();

        println!("audit root: {:?}", audit.root);

        for (root, results) in &audit.members {
            for (name, rs) in results {
                for (version, rs) in rs {
                    for r in rs {
                        println!(
                            "warning: {} -> {} -> {}: target: {}, path: {}",
                            root.display(),
                            name,
                            version,
                            r.warning.as_ref().unwrap().package.name,
                            r.tree.concat().join(" -> ")
                        );
                    }
                }
            }
        }
    }
}
