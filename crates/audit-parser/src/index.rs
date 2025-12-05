//! AuditIndex: run cargo audit and provide O(1) lookups by (crate_name, version).

use std::collections::HashMap;
use std::path::Path;

use crate::error::AuditError;
use crate::issue::AuditIssue;
use crate::parser::parse_audit_output;

/// Composite key for O(1) audit lookup by (crate_name, version).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AuditLookupKey {
    /// The crate name
    pub name: String,
    /// The crate version
    pub version: String,
}

impl AuditLookupKey {
    /// Create a new audit lookup key.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

/// Result of cargo audit with indexed lookups.
///
/// Provides O(1) lookups by crate name or by (crate_name, version).
#[derive(Debug, Default)]
pub struct AuditIndex {
    /// Primary index: lookup by (name, version) for exact matches
    by_name_version: HashMap<AuditLookupKey, Vec<AuditIssue>>,
    /// Secondary index: lookup by name only (for all versions)
    by_name: HashMap<String, Vec<AuditIssue>>,
}

impl AuditIndex {
    /// Run `cargo audit` on the given Cargo.lock file and parse the results.
    ///
    /// # Arguments
    ///
    /// * `cargo_lock_path` - Path to the Cargo.lock file
    /// * `workspace_members` - List of workspace member package names
    /// * `cargo_path` - Path to the cargo executable (defaults to "cargo")
    ///
    /// # Example
    ///
    /// ```ignore
    /// use audit_parser::AuditIndex;
    /// use std::path::Path;
    ///
    /// let index = AuditIndex::audit(
    ///     Path::new("/path/to/Cargo.lock"),
    ///     &["my-app"],
    ///     "cargo",
    /// ).await?;
    ///
    /// // O(1) lookup by name and version
    /// if let Some(issues) = index.get("serde", "1.0.0") {
    ///     for issue in issues {
    ///         println!("{}: {}", issue.id, issue.title);
    ///     }
    /// }
    /// ```
    pub async fn audit(
        cargo_lock_path: &Path,
        workspace_members: &[&str],
        cargo_path: &str,
    ) -> Result<Self, AuditError> {
        let output = tokio::process::Command::new(cargo_path)
            .arg("audit")
            .arg("-f")
            .arg(cargo_lock_path)
            .arg("-c")
            .arg("never")
            .output()
            .await
            .map_err(|e| AuditError::SpawnFailed(e.to_string()))?;

        if output.stdout.is_empty() {
            return Err(AuditError::EmptyOutput);
        }

        let stdout_str = String::from_utf8_lossy(&output.stdout);
        let issues_by_name = parse_audit_output(&stdout_str, workspace_members);

        Ok(Self::from_issues(issues_by_name))
    }

    /// Create an AuditIndex from pre-parsed issues.
    ///
    /// This is useful for testing or when you've already parsed the output.
    pub fn from_issues(issues_by_name: HashMap<String, Vec<AuditIssue>>) -> Self {
        let mut by_name_version: HashMap<AuditLookupKey, Vec<AuditIssue>> = HashMap::new();
        let mut by_name: HashMap<String, Vec<AuditIssue>> = HashMap::new();

        for (name, issues) in issues_by_name {
            by_name.insert(name.clone(), issues.clone());

            for issue in issues {
                let key = AuditLookupKey::new(&issue.crate_name, &issue.version);
                by_name_version.entry(key).or_default().push(issue);
            }
        }

        Self {
            by_name_version,
            by_name,
        }
    }

    /// O(1) lookup by crate name and version.
    ///
    /// Returns all issues for the exact (name, version) combination.
    pub fn get(&self, name: &str, version: &str) -> Option<&Vec<AuditIssue>> {
        let key = AuditLookupKey::new(name, version);
        self.by_name_version.get(&key)
    }

    /// O(1) lookup by crate name only.
    ///
    /// Returns all issues for all versions of the crate.
    pub fn get_by_name(&self, name: &str) -> Option<&Vec<AuditIssue>> {
        self.by_name.get(name)
    }

    /// Check if a specific (name, version) has any issues.
    pub fn has_issues(&self, name: &str, version: &str) -> bool {
        self.get(name, version).map_or(false, |v| !v.is_empty())
    }

    /// Check if a crate (any version) has any issues.
    pub fn has_issues_by_name(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Returns the total number of unique (name, version) combinations with issues.
    pub fn len(&self) -> usize {
        self.by_name_version.len()
    }

    /// Returns true if there are no issues.
    pub fn is_empty(&self) -> bool {
        self.by_name_version.is_empty()
    }

    /// Returns the number of unique crate names with issues.
    pub fn crate_count(&self) -> usize {
        self.by_name.len()
    }

    /// Iterate over all issues grouped by (name, version).
    pub fn iter(&self) -> impl Iterator<Item = (&AuditLookupKey, &Vec<AuditIssue>)> {
        self.by_name_version.iter()
    }

    /// Iterate over all issues grouped by crate name.
    pub fn iter_by_name(&self) -> impl Iterator<Item = (&String, &Vec<AuditIssue>)> {
        self.by_name.iter()
    }

    /// Get all crate names that have issues.
    pub fn affected_crates(&self) -> impl Iterator<Item = &String> {
        self.by_name.keys()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::AuditKind;

    fn create_test_issue(name: &str, version: &str, id: &str) -> AuditIssue {
        AuditIssue {
            crate_name: name.to_string(),
            version: version.to_string(),
            title: format!("Test issue for {} {}", name, version),
            id: id.to_string(),
            kind: AuditKind::Vulnerability,
            ..Default::default()
        }
    }

    #[test]
    fn test_lookup_by_name_version() {
        let mut issues = HashMap::new();
        issues.insert(
            "serde".to_string(),
            vec![
                create_test_issue("serde", "1.0.0", "RUSTSEC-0001"),
                create_test_issue("serde", "1.0.1", "RUSTSEC-0002"),
            ],
        );

        let index = AuditIndex::from_issues(issues);

        // Exact lookup
        let found = index.get("serde", "1.0.0").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "RUSTSEC-0001");

        let found = index.get("serde", "1.0.1").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "RUSTSEC-0002");

        // Non-existent version
        assert!(index.get("serde", "2.0.0").is_none());
    }

    #[test]
    fn test_lookup_by_name() {
        let mut issues = HashMap::new();
        issues.insert(
            "tokio".to_string(),
            vec![
                create_test_issue("tokio", "1.0.0", "RUSTSEC-0001"),
                create_test_issue("tokio", "1.1.0", "RUSTSEC-0002"),
            ],
        );

        let index = AuditIndex::from_issues(issues);

        let found = index.get_by_name("tokio").unwrap();
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn test_has_issues() {
        let mut issues = HashMap::new();
        issues.insert(
            "regex".to_string(),
            vec![create_test_issue("regex", "1.0.0", "RUSTSEC-0001")],
        );

        let index = AuditIndex::from_issues(issues);

        assert!(index.has_issues("regex", "1.0.0"));
        assert!(!index.has_issues("regex", "2.0.0"));
        assert!(index.has_issues_by_name("regex"));
        assert!(!index.has_issues_by_name("serde"));
    }

    #[test]
    fn test_counts() {
        let mut issues = HashMap::new();
        issues.insert(
            "a".to_string(),
            vec![
                create_test_issue("a", "1.0", "ID1"),
                create_test_issue("a", "2.0", "ID2"),
            ],
        );
        issues.insert("b".to_string(), vec![create_test_issue("b", "1.0", "ID3")]);

        let index = AuditIndex::from_issues(issues);

        assert_eq!(index.len(), 3); // 3 unique (name, version) pairs
        assert_eq!(index.crate_count(), 2); // 2 unique crate names
    }
}
