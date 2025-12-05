//! Parser for cargo audit text output.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use regex::Regex;

use crate::issue::{AuditIssue, AuditKind};

// Static regex patterns using OnceLock
static TREE_LINE_RE: OnceLock<Regex> = OnceLock::new();
static ROOT_LINE_RE: OnceLock<Regex> = OnceLock::new();

fn tree_line_re() -> &'static Regex {
    TREE_LINE_RE.get_or_init(|| Regex::new(r"^([│\s]*)(?:├──|└──)\s*(\S+)\s+(\S+)").unwrap())
}

fn root_line_re() -> &'static Regex {
    ROOT_LINE_RE.get_or_init(|| Regex::new(r"^([a-zA-Z0-9_-]+)\s+(\S+)$").unwrap())
}

/// Parse cargo audit text output into a map of crate name -> issues.
///
/// # Arguments
///
/// * `stdout` - The text output from `cargo audit`
/// * `workspace_members` - List of workspace member package names (used to identify dependency paths)
///
/// # Returns
///
/// A HashMap where keys are crate names and values are vectors of `AuditIssue`s.
pub fn parse_audit_output(
    stdout: &str,
    workspace_members: &[&str],
) -> HashMap<String, Vec<AuditIssue>> {
    let mut issues: HashMap<String, Vec<AuditIssue>> = HashMap::new();
    let mut current_issue: Option<AuditIssue> = None;
    let mut parsing_tree = false;
    let mut current_path: Vec<String> = Vec::new();
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
                    issue.crate_name = value.trim().to_string();
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
                        _ => {}
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

                // Workspace member found
                if pkg_match_set.contains(pkg_name) {
                    // Get the last package from path
                    if let Some(Some(parent_name_from_path)) =
                        current_path.last().map(|s| s.split_whitespace().next())
                    {
                        if !parent_name_from_path.is_empty() {
                            if let Some(issue) = current_issue.as_mut() {
                                issue.dependency_paths.insert(
                                    parent_name_from_path.to_string(),
                                    current_path.clone(),
                                );
                            }
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

    issues
}

fn save_current_issue(
    issues: &mut HashMap<String, Vec<AuditIssue>>,
    issue: &mut Option<AuditIssue>,
) {
    let Some(issue) = issue.take() else { return };
    if !issue.crate_name.is_empty() {
        issues
            .entry(issue.crate_name.clone())
            .or_default()
            .push(issue);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_vulnerability() {
        let output = r#"    Fetching advisory database from `https://github.com/RustSec/advisory-db.git`
      Loaded 776 security advisories (from /Users/user/.cargo/advisory-db)
    Scanning Cargo.lock for vulnerabilities (100 crate dependencies)
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
│       └── my-app 0.1.0
└── gix 0.70.0
    └── cargo 0.88.0
        └── my-app 0.1.0

error: 1 vulnerability found!"#;

        let result = parse_audit_output(output, &["my-app"]);
        assert_eq!(result.len(), 1);

        let issues = result.get("crossbeam-channel").unwrap();
        assert_eq!(issues.len(), 1);

        let issue = &issues[0];
        assert_eq!(issue.crate_name, "crossbeam-channel");
        assert_eq!(issue.version, "0.5.13");
        assert_eq!(issue.title, "crossbeam-channel: double free on Drop");
        assert_eq!(issue.id, "RUSTSEC-2025-0024");
        assert_eq!(
            issue.url,
            Some("https://rustsec.org/advisories/RUSTSEC-2025-0024".to_string())
        );
        assert_eq!(issue.solution, Some("Upgrade to >=0.5.15".to_string()));
        assert!(issue.kind.is_vulnerability());
    }

    #[test]
    fn test_parse_warning() {
        let output = r#"Crate:     tokio
Version:   1.44.1
Warning:   unsound
Title:     Broadcast channel calls clone in parallel, but does not require `Sync`
Date:      2025-04-07
ID:        RUSTSEC-2025-0023
URL:       https://rustsec.org/advisories/RUSTSEC-2025-0023
Dependency tree:
tokio 1.44.1
└── my-app 0.1.0

warning: 1 warning found"#;

        let result = parse_audit_output(output, &["my-app"]);
        assert_eq!(result.len(), 1);

        let issues = result.get("tokio").unwrap();
        assert_eq!(issues.len(), 1);

        let issue = &issues[0];
        assert_eq!(issue.crate_name, "tokio");
        assert!(issue.kind.is_warning());
        assert_eq!(issue.kind.warning_type(), Some("unsound"));
    }

    #[test]
    fn test_parse_with_severity() {
        let output = r#"Crate:     gix-features
Version:   0.38.2
Title:     SHA-1 collision attacks are not detected
ID:        RUSTSEC-2025-0021
URL:       https://rustsec.org/advisories/RUSTSEC-2025-0021
Severity:  6.8 (medium)
Solution:  Upgrade to >=0.41.0"#;

        let result = parse_audit_output(output, &[]);
        let issues = result.get("gix-features").unwrap();
        assert_eq!(issues[0].severity, Some("6.8 (medium)".to_string()));
    }
}
