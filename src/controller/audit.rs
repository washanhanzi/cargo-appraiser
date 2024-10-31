use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    pin::Pin,
    time::Duration,
};

use petgraph::prelude::NodeIndex;
use rustsec::Lockfile;
use tokio::{
    sync::mpsc::{self, error::SendError, Sender},
    time::Sleep,
};
use tower_lsp::lsp_types::Uri;
use tracing::error;

use super::CargoDocumentEvent;

pub type AuditReports = HashMap<PathBuf, HashMap<(String, String), Vec<AuditResult>>>;

pub struct AuditController {
    tx: Sender<CargoDocumentEvent>,
    sender: Option<Sender<Uri>>,
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
    pub fn new(tx: Sender<CargoDocumentEvent>) -> Self {
        Self { tx, sender: None }
    }

    pub async fn send(&self, uri: Uri) -> Result<(), SendError<Uri>> {
        self.sender.as_ref().unwrap().send(uri).await
    }

    pub fn spawn(&mut self) {
        //create a mpsc channel
        let (internal_tx, mut internal_rx) = mpsc::channel(32);
        let mut received_uri: Option<Uri> = None;
        self.sender = Some(internal_tx);
        let tx = self.tx.clone();
        let mut timer: Option<Pin<Box<Sleep>>> = None;
        //spawn a task to listen to the channel
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(uri) = internal_rx.recv() => {
                        received_uri = Some(uri);
                        timer = Some(Box::pin(tokio::time::sleep(Duration::from_secs(5))));
                    }
                    () = async {
                        if let Some(ref mut t) = timer {
                            t.await
                        } else {
                            futures::future::pending::<()>().await
                        }
                    }, if timer.is_some() => {
                        if let Some(uri) = &received_uri {
                            let reports = audit_workspace(uri, &vec![]).unwrap();
                            if let Err(e) = tx.send(CargoDocumentEvent::Audited(reports)).await {
                                error!("failed to send Audited event: {}", e);
                            }
                        }
                    }
                }
            }
        });
    }
}

pub fn audit_workspace(
    toml_uri: &Uri,
    members: &[&cargo::core::package::Package],
) -> Result<AuditReports, anyhow::Error> {
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

    let mut members_index_map = HashMap::new();
    let roots = tree.roots();
    for r in roots {
        let mut dfs = petgraph::visit::Dfs::new(&graph, r);
        while let Some(nx) = dfs.next(&graph) {
            // we can access `graph` mutably here still
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
    let mut reports = AuditReports::new();

    // Iterate over each warning to find paths to workspace members
    for (warning_node, warning) in warnings_map {
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

                // The last node is the warning node's package
                let dep_package = graph[path[1]].clone();

                reports
                    .entry(member.root().to_path_buf())
                    .or_default()
                    .entry((
                        dep_package.name.to_string(),
                        dep_package.version.to_string(),
                    ))
                    .or_default()
                    .push(AuditResult {
                        warning: Some(warning.clone()),
                        vuln: None,
                        workspace_member: (***member).clone(),
                        dep_package,
                        tree: tree_path,
                    });
            }
        }
    }
    Ok(reports)
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
        let mut audit = AuditController::new(mpsc::channel(32).0);
        let m: Vec<&cargo::core::package::Package> = workspace.members().collect();
        audit_workspace(
            &Uri::from_str("/Users/jingyu/Github/tauri/Cargo.toml").unwrap(),
            &m,
        )
        .unwrap();

        // for (root, results) in &audit.reports {
        //     for (dep_key, rs) in results {
        //         println!(
        //             "warning: {} -> {} -> {}: {}",
        //             root.display(),
        //             dep_key.0,
        //             dep_key.1,
        //             rs[0].tree.join(" -> ")
        //         );
        //     }
        // }
    }
}
