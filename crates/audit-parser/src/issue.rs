//! Audit issue types.

use std::collections::HashMap;

/// A security or warning issue from cargo audit.
#[derive(Debug, Clone, Default)]
pub struct AuditIssue {
    /// Name of the affected crate
    pub crate_name: String,
    /// Version of the affected crate
    pub version: String,
    /// Title/description of the issue
    pub title: String,
    /// RUSTSEC ID (e.g., "RUSTSEC-2025-0024")
    pub id: String,
    /// URL for more information
    pub url: Option<String>,
    /// Suggested solution (e.g., "Upgrade to >=0.5.15")
    pub solution: Option<String>,
    /// Type of issue (vulnerability or warning)
    pub kind: AuditKind,
    /// Dependency paths from workspace members to the vulnerable crate.
    /// Key: direct dependency name (what your workspace directly depends on)
    /// Value: full path from the vulnerable crate to the workspace member
    pub dependency_paths: HashMap<String, Vec<String>>,
    /// CVSS severity score (e.g., "6.8 (medium)")
    pub severity: Option<String>,
}

impl AuditIssue {
    /// Format the issue as human-readable text.
    ///
    /// If `hint_crate_name` is provided, only show the dependency path for that crate.
    pub fn format_text(&self, hint_crate_name: Option<&str>) -> String {
        match &self.kind {
            AuditKind::Vulnerability => {
                let mut text = format!(
                    "# Crate: {}\n* Version: {}\n* Title: {}\n* ID: {}\n",
                    self.crate_name, self.version, self.title, self.id
                );
                if let Some(severity) = &self.severity {
                    text.push_str(&format!("* Severity: {severity}\n"));
                }
                if let Some(url) = &self.url {
                    text.push_str(&format!("* URL: {url}\n"));
                }
                if let Some(solution) = &self.solution {
                    text.push_str(&format!("* Solution: {solution}\n"));
                }
                self.append_dependency_paths(&mut text, hint_crate_name);
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
                self.append_dependency_paths(&mut text, hint_crate_name);
                text
            }
        }
    }

    fn append_dependency_paths(&self, text: &mut String, hint_crate_name: Option<&str>) {
        if self.dependency_paths.is_empty() {
            return;
        }

        if let Some(hint_crate_name) = hint_crate_name {
            if let Some(dependency_paths) = self.dependency_paths.get(hint_crate_name) {
                text.push_str("* Dependency paths:\n");
                let reversed: Vec<_> = dependency_paths.iter().rev().cloned().collect();
                text.push_str(&reversed.join(" -> "));
            }
        }
    }
}

/// The type of audit issue.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AuditKind {
    /// A security vulnerability (RUSTSEC advisory)
    #[default]
    Vulnerability,
    /// A warning (e.g., "unmaintained", "unsound", "yanked")
    Warning(String),
}

impl AuditKind {
    /// Returns true if this is a vulnerability.
    pub fn is_vulnerability(&self) -> bool {
        matches!(self, AuditKind::Vulnerability)
    }

    /// Returns true if this is a warning.
    pub fn is_warning(&self) -> bool {
        matches!(self, AuditKind::Warning(_))
    }

    /// Returns the warning type if this is a warning.
    pub fn warning_type(&self) -> Option<&str> {
        match self {
            AuditKind::Warning(s) => Some(s),
            _ => None,
        }
    }
}
