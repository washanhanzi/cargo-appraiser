use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
    str::FromStr,
    sync::OnceLock,
    time::Duration,
};

use cargo::core::PackageIdSpec;
use regex::Regex;
use tokio::{
    sync::mpsc::{self, error::SendError, Sender},
    time::Sleep,
};
use tower_lsp::lsp_types::{DiagnosticSeverity, Uri};
use tracing::{debug, error, info};

use crate::{config::GLOBAL_CONFIG, entity::CanonicalUri};

use super::CargoDocumentEvent;

// Static regex patterns using OnceLock
static TREE_LINE_RE: OnceLock<Regex> = OnceLock::new();
static ROOT_LINE_RE: OnceLock<Regex> = OnceLock::new();

pub type AuditReports = HashMap<String, Vec<AuditIssue>>;

// Initialize regex patterns
fn tree_line_re() -> &'static Regex {
    TREE_LINE_RE.get_or_init(|| Regex::new(r"^([│\s]*)(?:├──|└──)\s*(\S+)\s+(\S+)").unwrap())
}

fn root_line_re() -> &'static Regex {
    ROOT_LINE_RE.get_or_init(|| Regex::new(r"^([a-zA-Z0-9_-]+)\s+(\S+)$").unwrap())
}

pub struct AuditPayload {
    pub root_manifest_uri: CanonicalUri,
    pub specs: Vec<PackageIdSpec>,
    pub cargo_path: String,
}

pub struct AuditController {
    tx: Sender<CargoDocumentEvent>,
    sender: Option<Sender<AuditPayload>>,
}

impl AuditController {
    pub fn new(tx: Sender<CargoDocumentEvent>) -> Self {
        Self { tx, sender: None }
    }

    pub async fn send(
        &self,
        uri: CanonicalUri,
        specs: Vec<PackageIdSpec>,
        cargo_path: &str,
    ) -> Result<(), SendError<AuditPayload>> {
        self.sender
            .as_ref()
            .unwrap()
            .send(AuditPayload {
                root_manifest_uri: uri,
                specs,
                cargo_path: cargo_path.to_string(),
            })
            .await
    }

    pub fn spawn(&mut self) {
        //create a mpsc channel
        let (internal_tx, mut internal_rx) = mpsc::channel(32);
        let mut received_uri = None;
        let mut specs = None;
        self.sender = Some(internal_tx);
        let tx = self.tx.clone();
        let mut cargo_path: Option<String> = None;
        let mut timer: Option<Pin<Box<Sleep>>> = None;
        //spawn a task to listen to the channel
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(payload) = internal_rx.recv() => {
                        if received_uri.is_none() {
                            received_uri = Some(payload.root_manifest_uri.ensure_lock());
                            specs=Some(payload.specs);
                        }
                        if cargo_path.is_none() {
                            cargo_path = Some(payload.cargo_path);
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
                        let uri = received_uri.take().unwrap();
                        let reports = match audit_workspace(&uri, &specs.take().unwrap(),cargo_path.as_deref().unwrap_or("cargo")).await {
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

#[derive(Debug, Clone, Default)]
pub struct AuditIssue {
    pub crate_name: String,
    pub version: String,
    pub title: String,
    pub id: String,
    pub url: Option<String>,
    pub solution: Option<String>,
    pub kind: AuditKind,
    // Map of direct dependencies to their full dependency paths
    // Key: direct dependency name (what your workspace directly depends on)
    // Value: full path from workspace member through direct dependency to the vulnerable crate
    pub dependency_paths: HashMap<String, Vec<String>>,
    pub severity: Option<String>,
}

impl AuditIssue {
    pub fn severity(&self) -> DiagnosticSeverity {
        match self.kind {
            AuditKind::Vulnerability => DiagnosticSeverity::ERROR,
            AuditKind::Warning(_) => DiagnosticSeverity::WARNING,
        }
    }

    pub fn audit_text(&self, hint_crate_name: Option<&str>) -> String {
        match &self.kind {
            AuditKind::Vulnerability => {
                let mut text = format!(
                    "# Crate: {}\n* Version: {}\n* Title: {}\n* ID: {}\n",
                    self.crate_name, self.version, self.title, self.id
                );
                if let Some(url) = &self.url {
                    text.push_str(&format!("* Url: {url}\n"));
                }
                if let Some(solution) = &self.solution {
                    text.push_str(&format!("* Solution: {solution}\n"));
                }
                if !self.dependency_paths.is_empty() {
                    if let Some(hint_crate_name) = hint_crate_name {
                        if let Some(dependency_paths) = self.dependency_paths.get(hint_crate_name) {
                            text.push_str("* Dependency paths:\n");
                            let reversed = dependency_paths
                                .clone()
                                .into_iter()
                                .rev()
                                .collect::<Vec<_>>();
                            text.push_str(reversed.join(" -> ").as_str());
                        }
                    }
                }
                text
            }
            AuditKind::Warning(warning) => {
                let mut text = format!(
                    "# Crate: {}\n* Version: {}\n* Warning: {}\n",
                    self.crate_name, self.version, warning
                );
                if !self.title.is_empty() {
                    text.push_str(&format!("* Title: {}\n", self.title));
                }
                if !self.id.is_empty() {
                    text.push_str(&format!("* ID: {}\n", self.id));
                }
                if let Some(url) = self.url.as_ref() {
                    text.push_str(&format!("* URL: {url}\n"));
                }
                if !self.dependency_paths.is_empty() {
                    if let Some(hint_crate_name) = hint_crate_name {
                        if let Some(dependency_paths) = self.dependency_paths.get(hint_crate_name) {
                            text.push_str("* Dependency paths:\n");
                            let reversed = dependency_paths
                                .clone()
                                .into_iter()
                                .rev()
                                .collect::<Vec<_>>();
                            text.push_str(reversed.join(" -> ").as_str());
                        }
                    }
                }
                text
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AuditKind {
    #[default]
    Vulnerability,
    Warning(String),
}

#[tracing::instrument(name = "audit_workspace", level = "trace")]
pub async fn audit_workspace(
    //the root Cargo.toml path
    root_manifest_uri: &CanonicalUri,
    specs: &[PackageIdSpec],
    cargo_path: &str,
) -> Result<AuditReports, anyhow::Error> {
    debug!(
        "Auditing workspace for root manifest: {}",
        root_manifest_uri.as_str()
    );

    let Ok(manifest_path) = root_manifest_uri.to_path_buf() else {
        error!("Failed to convert URI to path: {:?}", root_manifest_uri);
        return Err(anyhow::anyhow!("Failed to convert URI to path"));
    };
    info!("manifest path {}", manifest_path.to_str().unwrap());

    let output = match tokio::process::Command::new(cargo_path)
        .arg("audit")
        .arg("-f")
        .arg(&manifest_path)
        .arg("-c")
        .arg("never")
        .output()
        .await
    {
        Ok(output) => output,
        Err(e) => {
            error!("Failed to spawn cargo audit: {}", e);
            return Err(anyhow::anyhow!("Failed to spawn cargo audit"));
        }
    };

    if output.stdout.is_empty() {
        error!("cargo output stdout empty");
        return Err(anyhow::anyhow!("cargo output stdout empty"));
    }

    let stdout_str = String::from_utf8_lossy(&output.stdout);

    // Parse the text output
    let workspace_members_refs: Vec<&str> = specs.iter().map(|s| s.name()).collect();
    let parsed_issues = parse_audit_text_output(&stdout_str, &workspace_members_refs)?;

    Ok(parsed_issues)
}

// Helper function to parse cargo audit text output
fn parse_audit_text_output(
    stdout: &str,
    workspace_members: &[&str],
) -> Result<AuditReports, anyhow::Error> {
    let mut issues = HashMap::new();
    let mut current_issue: Option<AuditIssue> = None;
    let mut parsing_tree = false;
    let mut current_path: Vec<String> = Vec::new(); // (indent, package_str)
    let pkg_match_set: HashSet<String> =
        HashSet::from_iter(workspace_members.iter().map(|s| s.to_string()));

    for line in stdout.lines() {
        // Skip lines that start with whitespace (likely continuation lines)
        if line.starts_with(" ") && !parsing_tree {
            continue;
        }

        // Check if this is a line starting with "Crate:" which indicates a new issue
        if line.starts_with("Crate:") {
            parsing_tree = false;
            save_current_issue(&mut issues, &mut current_issue);
            current_issue = Some(AuditIssue::default());
            if let Some((_, value)) = line.split_once(':') {
                if let Some(issue) = current_issue.as_mut() {
                    let crate_name = value.trim().to_string();
                    issue.crate_name = crate_name.clone();
                }
            }
        } else if line.starts_with("Version:")
            || line.starts_with("Title:")
            || line.starts_with("ID:")
            || line.starts_with("URL:")
            || line.starts_with("Solution:")
            || line.starts_with("Warning:")
            || line.starts_with("Severity:")
        {
            if let Some(issue) = current_issue.as_mut() {
                if let Some((key_str, value_str)) = line.split_once(':') {
                    let key = key_str.trim();
                    let value_trimmed = value_str.trim();
                    match key {
                        "Version" => issue.version = value_trimmed.to_string(),
                        "Title" => issue.title = value_trimmed.to_string(),
                        "ID" => issue.id = value_trimmed.to_string(),
                        "URL" => issue.url = Some(value_trimmed.to_string()),
                        "Solution" => issue.solution = Some(value_trimmed.to_string()),
                        "Warning" => issue.kind = AuditKind::Warning(value_trimmed.to_string()),
                        "Severity" => issue.severity = Some(value_trimmed.to_string()),
                        _ => { /* This case should ideally not be reached */ }
                    }
                }
            }
            continue;
        } else if line.starts_with("Dependency tree:") {
            parsing_tree = true;
            current_path.clear();
            continue;
        }

        // If we're parsing a dependency tree
        if parsing_tree {
            if let Some(caps) = root_line_re().captures(line.trim()) {
                // Handle the root line of the tree (no indent)
                let pkg_name = caps.get(1).map_or("", |m| m.as_str());
                let pkg_version = caps.get(2).map_or("", |m| m.as_str());
                if !pkg_name.is_empty() {
                    current_path.push(format!("{} {}", pkg_name, pkg_version));
                }
            } else if let Some(caps) = tree_line_re().captures(line) {
                let indent = caps.get(1).unwrap().as_str().chars().count();
                let pkg_name = caps.get(2).unwrap().as_str();
                let pkg_version = caps.get(3).unwrap().as_str();

                current_path.truncate((indent / 4) + 1);

                //workspace member found
                if pkg_match_set.contains(pkg_name) {
                    //get the last package from path
                    let Some(Some(parent_name_from_path)) =
                        current_path.last().map(|s| s.split_whitespace().next())
                    else {
                        error!("Failed to get parent name from path");
                        continue;
                    };

                    if !parent_name_from_path.is_empty() {
                        if let Some(issue) = current_issue.as_mut() {
                            issue.dependency_paths.insert(
                                parent_name_from_path.to_string(), // This is the key
                                current_path.clone(),              // This is the path to the parent
                            );
                        }
                    }
                }
                current_path.push(format!("{} {}", pkg_name, pkg_version));
            }
            continue;
        }
    }

    // Handle the last issue
    save_current_issue(&mut issues, &mut current_issue);

    Ok(issues)
}

// Helper function to finalize and save the current issue
fn save_current_issue(
    issues: &mut HashMap<String, Vec<AuditIssue>>,
    issue: &mut Option<AuditIssue>,
) {
    let Some(issue) = issue.take() else { return };
    match (
        &issue.kind,
        GLOBAL_CONFIG.read().unwrap().audit.level.as_str(),
    ) {
        (_, "warning") => {}
        (AuditKind::Vulnerability, "vulnerability") => {}
        _ => return,
    }
    // Only add if it's not empty
    if !issue.crate_name.is_empty() {
        issues
            .entry(issue.crate_name.clone())
            .or_default()
            .push(issue);
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, str::FromStr};

    use cargo::core::PackageIdSpec;
    use tower_lsp::lsp_types::Uri;

    use crate::entity::CanonicalUri;

    use super::audit_workspace;

    #[tokio::test]
    async fn test_audit_workspace() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_test_writer()
            .try_init();

        let uri = Uri::from_str("file:///Users/jingyu/Github/cargo-appraiser/Cargo.toml").unwrap();
        let res = audit_workspace(
            &CanonicalUri::try_from(uri).unwrap(),
            &[PackageIdSpec::new("cargo-appraiser".to_string())],
            "cargo",
        )
        .await;
        assert!(res.is_ok(), "Failed to audit workspace: {:?}", res);
        let reports = res.unwrap();
        assert!(!reports.is_empty());
        for (k, v) in reports {
            println!("{}", k);
            for issue in v {
                println!("{}", issue.id);
            }
        }
    }

    #[test]
    fn test_parse_audit_text_output() {
        let test_cases = vec![
            (
                "case 1",
                r#"    Fetching advisory database from `https://github.com/RustSec/advisory-db.git`
      Loaded 776 security advisories (from /Users/jingyu/.cargo/advisory-db)
    Updating crates.io index
    Scanning Cargo.lock for vulnerabilities (566 crate dependencies)
Crate:     crossbeam-channel
Version:   0.5.13
Title:     crossbeam-channel: double free on Drop
Date:      2025-04-08
ID:        RUSTSEC-2025-0024
URL:       https://rustsec.org/advisories/RUSTSEC-2025-0024
Solution:  Upgrade to >=0.5.15
Dependency tree:
crossbeam-channel 0.5.13
├── tame-index 0.14.0
│   └── rustsec 0.30.0
│       ├── cargo-audit 0.21.0
│       │   └── cargo-appraiser 0.0.1
│       └── cargo-appraiser 0.0.1
├── gix-features 0.40.0
│   ├── gix-worktree 0.39.0
│   │   ├── gix-dir 0.12.0
│   │   │   └── gix 0.70.0
│   │   │       └── cargo 0.88.0
│   │   │           └── cargo-appraiser 0.0.1
│   │   └── gix 0.70.0
│   └── gix 0.70.0
└── gix-features 0.38.2
    ├── gix-worktree-state 0.13.0
    │   └── gix 0.66.0
    │       ├── tame-index 0.14.0
    │       └── rustsec 0.30.0
    └── gix 0.66.0

Crate:     gix-features
Version:   0.38.2
Title:     SHA-1 collision attacks are not detected
Date:      2025-04-03
ID:        RUSTSEC-2025-0021
URL:       https://rustsec.org/advisories/RUSTSEC-2025-0021
Severity:  6.8 (medium)
Solution:  Upgrade to >=0.41.0
Dependency tree:
gix-features 0.38.2
├── gix-worktree-state 0.13.0
│   └── gix 0.66.0
│       ├── tame-index 0.14.0
│       │   └── rustsec 0.30.0
│       │       ├── cargo-audit 0.21.0
│       │       │   └── cargo-appraiser 0.0.1
│       │       └── cargo-appraiser 0.0.1
│       └── rustsec 0.30.0
└── gix 0.66.0

Crate:     tokio
Version:   1.44.1
Warning:   unsound
Title:     Broadcast channel calls clone in parallel, but does not require `Sync`
Date:      2025-04-07
ID:        RUSTSEC-2025-0023
URL:       https://rustsec.org/advisories/RUSTSEC-2025-0023
Dependency tree:
tokio 1.44.1
├── tower-lsp 0.20.0
│   └── cargo-appraiser 0.0.1
└── cargo-appraiser 0.0.1

Crate:     crossbeam-channel
Version:   0.5.13
Warning:   yanked

error: 5 vulnerabilities found!
warning: 2 allowed warnings found"#,
                vec!["cargo-appraiser"],
                HashMap::from([
                    (
                        "crossbeam-channel".to_string(),
                        vec![
                            super::AuditIssue {
                            crate_name: "crossbeam-channel".to_string(),
                            version: "0.5.13".to_string(),
                            title: "crossbeam-channel: double free on Drop".to_string(),
                            id: "RUSTSEC-2025-0024".to_string(),
                            url: Some(
                                "https://rustsec.org/advisories/RUSTSEC-2025-0024".to_string(),
                            ),
                            solution: Some("Upgrade to >=0.5.15".to_string()),
                            kind: super::AuditKind::Vulnerability,
                            dependency_paths: HashMap::from([(
                                "cargo-audit".to_string(),
                                vec![
                                "crossbeam-channel 0.5.13".to_string(),
                                    "tame-index 0.14.0".to_string(),
                                    "rustsec 0.30.0".to_string(),
                                    "cargo-audit 0.21.0".to_string(),
                                ],
                            ),
                            (
                                "cargo".to_string(),
                                vec![
                                "crossbeam-channel 0.5.13".to_string(),
                                    "gix-features 0.40.0".to_string(),
                                    "gix-worktree 0.39.0".to_string(),
                                    "gix-dir 0.12.0".to_string(),
                                    "gix 0.70.0".to_string(),
                                    "cargo 0.88.0".to_string(),
                                ],
                            ),
                            (
                                "rustsec".to_string(),
                                vec![
                                "crossbeam-channel 0.5.13".to_string(),
                                    "tame-index 0.14.0".to_string(),
                                    "rustsec 0.30.0".to_string(),
                                ],
                            ),

                            ]),
                            severity: None, // No severity for this issue
                        },
                            super::AuditIssue {
                            crate_name: "crossbeam-channel".to_string(),
                            version: "0.5.13".to_string(),
                            title: "".to_string(),
                            id: "".to_string(),
                            url: None,
                            solution: None,
                            kind: super::AuditKind::Warning("yanked".to_string()),
                            dependency_paths: HashMap::new(),
                            severity: None, // No severity for this issue
                        },
                        ],
                    ),
                    (
                        "gix-features".to_string(),
                        vec![super::AuditIssue {
                            crate_name: "gix-features".to_string(),
                            version: "0.38.2".to_string(),
                            title: "SHA-1 collision attacks are not detected".to_string(),
                            id: "RUSTSEC-2025-0021".to_string(),
                            url: Some(
                                "https://rustsec.org/advisories/RUSTSEC-2025-0021".to_string(),
                            ),
                            solution: Some("Upgrade to >=0.41.0".to_string()),
                            kind: super::AuditKind::Vulnerability,
                            dependency_paths: HashMap::from([
                                (
                                    "rustsec".to_string(),
                                    vec![
                                    "gix-features 0.38.2".to_string(),
                                        "gix-worktree-state 0.13.0".to_string(),
                                        "gix 0.66.0".to_string(),
                                        "tame-index 0.14.0".to_string(),
                                        "rustsec 0.30.0".to_string()
                                        ],
                                ),
                                (
                                    "cargo-audit".to_string(),
                                    vec![
                                    "gix-features 0.38.2".to_string(),
                                        "gix-worktree-state 0.13.0".to_string(),
                                        "gix 0.66.0".to_string(),
                                        "tame-index 0.14.0".to_string(),
                                        "rustsec 0.30.0".to_string(),
                                        "cargo-audit 0.21.0".to_string(),
                                        ],
                                ),
                            ]), // Initialize empty for cases we don't test
                            severity: Some("6.8 (medium)".to_string()),
                        }],
                    ),
                    (
                    "tokio".to_string(),
                    vec![super::AuditIssue {
                        crate_name: "tokio".to_string(),
                        version: "1.44.1".to_string(),
                        title: "Broadcast channel calls clone in parallel, but does not require `Sync`"
                            .to_string(),
                        id: "RUSTSEC-2025-0023".to_string(),
                        url: Some("https://rustsec.org/advisories/RUSTSEC-2025-0023".to_string()),
                        solution: None,
                        kind: super::AuditKind::Warning("unsound".to_string()),
                        dependency_paths: HashMap::from([
                            ("tower-lsp".to_string(),
                            vec![
                            "tokio 1.44.1".to_string(),
                            "tower-lsp 0.20.0".to_string(),
                        ]),
                        (
                            "tokio".to_string(),
                            vec!["tokio 1.44.1".to_string()],
                        )]),
                        severity: None,
                    }],
                )
                ]),
            ),
            (
                "case 2",
                r#"    Fetching advisory database from `https://github.com/RustSec/advisory-db.git`
      Loaded 776 security advisories (from /Users/jingyu/.cargo/advisory-db)
    Updating crates.io index
    Scanning Cargo.lock for vulnerabilities (266 crate dependencies)
Crate:     dotenv
Version:   0.15.0
Warning:   unmaintained
Title:     dotenv is Unmaintained
Date:      2021-12-24
ID:        RUSTSEC-2021-0141
URL:       https://rustsec.org/advisories/RUSTSEC-2021-0141
Dependency tree:
dotenv 0.15.0
└── firecrawl-mcp 0.3.0

Crate:     paste
Version:   1.0.15
Warning:   unmaintained
Title:     paste - no longer maintained
Date:      2024-10-07
ID:        RUSTSEC-2024-0436
URL:       https://rustsec.org/advisories/RUSTSEC-2024-0436
Dependency tree:
paste 1.0.15
├── rmcp 0.1.5
│   └── firecrawl-mcp 0.3.0
└── async-claude 0.15.0
    ├── firecrawl-sdk 0.3.0
    │   └── firecrawl-mcp 0.3.0
    └── firecrawl-mcp 0.3.0

warning: 2 allowed warnings found"#,
                vec!["firecrawl-mcp"],
                HashMap::from([
                    (
                    "dotenv".to_string(),
                vec![    super::AuditIssue {
                        crate_name: "dotenv".to_string(),
                        version: "0.15.0".to_string(),
                        title: "dotenv is Unmaintained".to_string(),
                        id: "RUSTSEC-2021-0141".to_string(),
                        url: Some("https://rustsec.org/advisories/RUSTSEC-2021-0141".to_string()),
                        solution: None,
                        kind: super::AuditKind::Warning("unmaintained".to_string()),
                        dependency_paths: HashMap::from([
                            (
                                "dotenv".to_string(),
                                vec!["dotenv 0.15.0".to_string()]
                            )
                        ]), // Matches the empty case
                        severity:None,
                    }],
                    ),
                    (
                "paste".to_string(),
                vec![super::AuditIssue {
                    crate_name: "paste".to_string(),
                    version: "1.0.15".to_string(),
                    title: "paste - no longer maintained".to_string(),
                    id: "RUSTSEC-2024-0436".to_string(),
                    url: Some("https://rustsec.org/advisories/RUSTSEC-2024-0436".to_string()),
                    solution: None,
                    kind:super::AuditKind::Warning("unmaintained".to_string()),
                    dependency_paths: HashMap::from([
                        (
                        "async-claude".to_string(),
                         vec![
                            "paste 1.0.15".to_string(),
                            "async-claude 0.15.0".to_string(),
                        ]),
                        (
                        "rmcp".to_string(),
                         vec![
                            "paste 1.0.15".to_string(),
                            "rmcp 0.1.5".to_string(),
                        ]),
                        (
                        "firecrawl-sdk".to_string(),
                         vec![
                            "paste 1.0.15".to_string(),
                            "async-claude 0.15.0".to_string(),
                            "firecrawl-sdk 0.3.0".to_string(),
                        ]),
                    ]),
                    severity: None,
                }],
                    )
                ])
            ),
        ];

        for (case_name, sample_output, workspace_members, wanted) in test_cases {
            let result = super::parse_audit_text_output(sample_output, &workspace_members).unwrap();
            // Check the total count of crates with issues
            assert_eq!(
                result.len(),
                wanted.len(),
                "Should parse issues for {} crates from the sample output in {}",
                wanted.len(),
                case_name
            );
            // Check each expected issue against the actual result
            for (crate_name, expected) in wanted {
                let found_issues = result.get(&crate_name).unwrap_or_else(|| {
                    panic!("Missing issues for crate: {} in {}", crate_name, case_name)
                });

                // Since we've changed to Vec<AuditIssue>, we expect one issue in the vector for these tests
                assert_eq!(
                    found_issues.len(),
                    expected.len(),
                    "Expected exactly {} issues for {} in {}",
                    expected.len(),
                    crate_name,
                    case_name
                );

                for want_issue in expected {
                    for got_issue in found_issues {
                        //TODO warning don't have ID
                        if want_issue.id == got_issue.id {
                            assert_eq!(
                                got_issue.crate_name, want_issue.crate_name,
                                "Crate name mismatch for {} in {}",
                                crate_name, case_name
                            );
                            assert_eq!(
                                got_issue.version, want_issue.version,
                                "Version mismatch for {} in {}",
                                crate_name, case_name
                            );
                            assert_eq!(
                                got_issue.title, want_issue.title,
                                "Title mismatch for {} in {}",
                                crate_name, case_name
                            );
                            assert_eq!(
                                got_issue.id, want_issue.id,
                                "ID mismatch for {} in {}",
                                crate_name, case_name
                            );
                            assert_eq!(
                                got_issue.url, want_issue.url,
                                "URL mismatch for {} in {}",
                                crate_name, case_name
                            );
                            assert_eq!(
                                got_issue.solution, want_issue.solution,
                                "Solution mismatch for {} in {}",
                                crate_name, case_name
                            );
                            assert_eq!(
                                got_issue.kind, want_issue.kind,
                                "Kind mismatch for {} in {}",
                                crate_name, case_name
                            );
                            assert_eq!(
                                got_issue.severity, want_issue.severity,
                                "Severity mismatch for {} in {}",
                                crate_name, case_name
                            );

                            for (key, value) in got_issue.dependency_paths.clone() {
                                let want_value =
                                    want_issue.dependency_paths.get(&key).unwrap_or_else(|| {
                                        panic!(
                                            "Missing dependency path for {} in {}",
                                            crate_name, case_name
                                        )
                                    });
                                // First, check that both vectors have the same length
                                assert_eq!(
    value.len(), want_value.len(),
    "Dependency path length mismatch for key {} in {}: got {:?}, expected {:?}",
    key, crate_name, value, want_value
);

                                // Then check each element in order
                                for (i, (actual, expected)) in
                                    value.iter().zip(want_value.iter()).enumerate()
                                {
                                    assert_eq!(
        actual, expected,
        "Dependency path element mismatch at position {} for key {} in {}: got {}, expected {}",
        i, key, crate_name, actual, expected
    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
