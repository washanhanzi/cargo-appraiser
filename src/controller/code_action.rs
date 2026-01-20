use std::collections::HashMap;

use semver::{Op, Version};
use serde_json::Value;
use tokio::sync::oneshot;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionResponse, Command, Range, TextEdit,
    Uri, WorkspaceEdit,
};
use tracing::error;

use crate::{
    decoration::{version_decoration, VersionDecorationKind},
    entity::{
        DependencyKey, DependencyStyle, DependencyValue, NodeKind, ResolvedDependency,
        TomlDependency, TomlNode, TomlTree, ValueKind, CARGO,
    },
};

use super::context::AppraiserContext;

/// Handle `CargoDocumentEvent::CodeAction` - provide code actions.
pub async fn handle_code_action(
    ctx: &mut AppraiserContext<'_>,
    uri: Uri,
    range: Range,
    tx: oneshot::Sender<CodeActionResponse>,
) {
    let Ok(canonical_uri) = uri.clone().try_into() else {
        error!("failed to canonicalize uri: {}", uri.as_str());
        return;
    };

    let Some(doc) = ctx.state.document(&canonical_uri) else {
        return;
    };

    let Some(node) = doc.precise_match(range.start) else {
        return;
    };

    let tree = doc.tree();
    let dep = tree.find_dependency_at_position(range.start);
    let resolved = dep.and_then(|d| doc.resolved(&d.id));

    let Some(action) = code_action(uri, tree, node, dep, resolved) else {
        return;
    };

    let _ = tx.send(action);
}

pub fn code_action(
    uri: Uri,
    tree: &TomlTree,
    node: &TomlNode,
    dep: Option<&TomlDependency>,
    resolved: Option<&ResolvedDependency>,
) -> Option<CodeActionResponse> {
    //only support dependency code action for now
    let dep = dep?;
    code_action_dependency(uri, tree, node, dep, resolved)
}

pub fn code_action_dependency(
    uri: Uri,
    tree: &TomlTree,
    node: &TomlNode,
    dep: &TomlDependency,
    resolved: Option<&ResolvedDependency>,
) -> Option<CodeActionResponse> {
    match &node.kind {
        NodeKind::Value(ValueKind::Dependency(DependencyValue::Simple))
        | NodeKind::Value(ValueKind::Dependency(DependencyValue::Version)) => {
            let version = version_decoration(dep, resolved);
            let mut actions = VersionCodeAction::new(uri, tree, node);
            actions.check_unresolved(resolved);

            // Add cargo update command first (at top) if applicable
            match version.kind {
                VersionDecorationKind::MixedUpgradeable
                | VersionDecorationKind::CompatibleLatest
                | VersionDecorationKind::Yanked
                | VersionDecorationKind::Git => {
                    actions.add_update_command(dep.package_name());
                }
                _ => {}
            }

            // Add simple-to-table refactor second if applicable
            if dep.style == DependencyStyle::Simple {
                actions.add_simple_table_refactor(dep, node);
            }

            match version.kind {
                VersionDecorationKind::Latest => {
                    if let Some(v) = version.latest.as_ref() {
                        actions.add_refactor(v);
                    }
                }
                VersionDecorationKind::Local => return None,
                VersionDecorationKind::NotInstalled => return None,
                VersionDecorationKind::MixedUpgradeable => {
                    if let Some(v) = version.latest_matched.as_ref() {
                        actions.add_upgrade(v);
                    }
                    if let Some(v) = version.latest.as_ref() {
                        actions.add_upgrade(v);
                    }
                }
                VersionDecorationKind::CompatibleLatest => {
                    let v = version.latest.as_ref()?;
                    actions.add_refactor(v);
                    actions.add_upgrade(v);
                }
                VersionDecorationKind::NonCompatibleLatest => {
                    let v = version.latest.as_ref()?;
                    actions.add_upgrade(v);
                }
                VersionDecorationKind::Yanked => {
                    if let Some(v) = version.latest.as_ref() {
                        actions.add_upgrade(v);
                    }
                    if let Some(v) = version.latest_matched.as_ref() {
                        actions.add_upgrade(v);
                    }
                }
                VersionDecorationKind::Git => {}
                VersionDecorationKind::NotParsed => return None,
            };
            actions.add_eq_refactor();
            if let Some(version) = resolved.and_then(|r| r.installed_version()) {
                actions.add_precise_eq_refactor(&version);
            }
            // Add table-to-simple refactor at the end if applicable
            if dep.style == DependencyStyle::Table {
                actions.add_simple_table_refactor(dep, node);
            }

            Some(actions.take())
        }
        NodeKind::Key(crate::entity::KeyKind::Dependency(DependencyKey::Workspace))
        | NodeKind::Value(ValueKind::Dependency(DependencyValue::Workspace)) => {
            let version = version_decoration(dep, resolved);
            let mut actions = VersionCodeAction::new(uri, tree, node);
            match version.kind {
                VersionDecorationKind::MixedUpgradeable => {
                    actions.add_update_command(dep.package_name());
                }
                VersionDecorationKind::CompatibleLatest => {
                    actions.add_update_command(dep.package_name());
                }
                VersionDecorationKind::Yanked => {
                    actions.add_update_command(dep.package_name());
                }
                VersionDecorationKind::Git => {
                    actions.add_update_command(dep.package_name());
                }
                _ => return None,
            };
            Some(actions.take())
        }
        _ => None,
    }
}

struct VersionCodeAction<'a> {
    uri: Uri,
    tree: &'a TomlTree,
    major_code_action: bool,
    minor_code_action: bool,
    actions: CodeActionResponse,
    node: &'a TomlNode,
    is_precise: bool,
}

impl<'a> VersionCodeAction<'a> {
    fn new(uri: Uri, tree: &'a TomlTree, node: &'a TomlNode) -> Self {
        Self {
            uri,
            tree,
            major_code_action: false,
            minor_code_action: false,
            actions: Vec::with_capacity(6),
            node,
            is_precise: false,
        }
    }

    fn take(self) -> CodeActionResponse {
        self.actions
    }

    // check version req contain minor or patch
    // if the version req contains minor, provide a code action to refactor version to <major>
    // if the version req contains no minor, provide a code action to refactor version to <major.minor>
    // if the version req contains patch, provide the above two code actions
    fn check_unresolved(&mut self, _resolved: Option<&ResolvedDependency>) {
        // Parse version from node text to determine precision
        let version_text = strip_quotes(&self.node.text);
        if let Ok(version) = semver::VersionReq::parse(&version_text) {
            for r in &version.comparators {
                if r.minor.is_some() {
                    self.major_code_action = true;
                }
                if r.minor.is_none() {
                    self.minor_code_action = true;
                }
                if r.patch.is_some() {
                    self.major_code_action = true;
                    self.minor_code_action = true;
                }
                if r.op == Op::Exact {
                    self.is_precise = true;
                }
            }
        }
    }

    fn add_simple_table_refactor(&mut self, dep: &TomlDependency, node: &TomlNode) {
        if dep.style == DependencyStyle::Simple {
            self.add_code_action(
                format!("{{ version = \"{}\" }}", strip_quotes(&node.text)),
                CodeActionKind::REFACTOR,
                node.range,
                None,
            );
        }
        if dep.style == DependencyStyle::Table
            && matches!(
                node.kind,
                NodeKind::Value(ValueKind::Dependency(DependencyValue::Version))
            )
        {
            // Only offer if the table has just version key
            if dep.fields.len() == 1 && dep.version().is_some() {
                // Get the table node's range to replace the entire { version = "x.y.z" }
                if let Some(table_node) = self.tree.get_dependency_entry_node(dep) {
                    self.add_code_action(
                        format!("\"{}\"", strip_quotes(&node.text)),
                        CodeActionKind::REFACTOR,
                        table_node.range,
                        Some("Refactor to simple version".to_string()),
                    );
                }
            }
        }
    }

    fn add_eq_refactor(&mut self) {
        if !self.is_precise {
            self.add_code_action(
                format!("\"={}\"", strip_quotes(&self.node.text)),
                CodeActionKind::REFACTOR,
                self.node.range,
                None,
            );
        }
    }

    fn add_precise_eq_refactor(&mut self, v: &Version) {
        if !self.is_precise {
            // Avoid duplicate if the precise version matches the current version text
            let precise = format!("\"={}\"", v);
            let current_eq = format!("\"={}\"", strip_quotes(&self.node.text));
            if precise != current_eq {
                self.add_code_action(precise, CodeActionKind::REFACTOR, self.node.range, None);
            }
        }
    }

    fn add_refactor(&mut self, v: &Version) {
        // Add alternative version formats as refactor options
        // Skip the format that's already offered as quickfix
        if v.major >= 1 {
            // Quickfix is major, so offer minor and patch as refactors
            if self.minor_code_action {
                self.add_code_action(
                    format!("\"{}.{}\"", v.major, v.minor),
                    CodeActionKind::REFACTOR,
                    self.node.range,
                    None,
                );
            }
            self.add_code_action(
                format!("\"{}\"", v),
                CodeActionKind::REFACTOR,
                self.node.range,
                None,
            );
        } else if v.minor != 0 {
            // Quickfix is major.minor, so offer major, major.minor and patch as refactors
            if self.major_code_action {
                self.add_code_action(
                    format!("\"{}\"", v.major),
                    CodeActionKind::REFACTOR,
                    self.node.range,
                    None,
                );
            }
            if self.minor_code_action {
                self.add_code_action(
                    format!("\"{}.{}\"", v.major, v.minor),
                    CodeActionKind::REFACTOR,
                    self.node.range,
                    None,
                );
            }
            self.add_code_action(
                format!("\"{}\"", v),
                CodeActionKind::REFACTOR,
                self.node.range,
                None,
            );
        } else {
            // v.major == 0 && v.minor == 0: quickfix is full version
            // Offer major and major.minor as refactors
            if self.major_code_action {
                self.add_code_action(
                    format!("\"{}\"", v.major),
                    CodeActionKind::REFACTOR,
                    self.node.range,
                    None,
                );
            }
            if self.minor_code_action {
                self.add_code_action(
                    format!("\"{}.{}\"", v.major, v.minor),
                    CodeActionKind::REFACTOR,
                    self.node.range,
                    None,
                );
            }
        }
    }

    fn add_upgrade(&mut self, v: &Version) {
        // Offer the recommended compatible version format based on semver conventions
        if v.major >= 1 {
            // For major >= 1, major version is compatible
            self.add_code_action(
                format!("\"{}\"", v.major),
                CodeActionKind::REFACTOR,
                self.node.range,
                None,
            );
        } else if v.minor != 0 {
            // For 0.x.y where x != 0, major.minor is compatible
            self.add_code_action(
                format!("\"{}.{}\"", v.major, v.minor),
                CodeActionKind::REFACTOR,
                self.node.range,
                None,
            );
        } else {
            // For 0.0.x, full version is needed
            self.add_code_action(
                format!("\"{}\"", v),
                CodeActionKind::REFACTOR,
                self.node.range,
                None,
            );
        }
    }

    fn add_code_action(
        &mut self,
        v: String,
        kind: CodeActionKind,
        range: Range,
        title: Option<String>,
    ) {
        self.actions
            .push(new_code_action(self.uri.clone(), v, kind, range, title));
    }

    fn add_update_command(&mut self, package_name: &str) {
        self.actions.push(new_update_command(package_name).into());
    }
}

fn new_code_action(
    uri: Uri,
    v: String,
    kind: CodeActionKind,
    range: Range,
    title: Option<String>,
) -> CodeActionOrCommand {
    CodeAction {
        title: title.unwrap_or(v.to_string()),
        kind: Some(kind),
        diagnostics: None,
        edit: Some(WorkspaceEdit {
            changes: Some(HashMap::from([(
                uri,
                vec![TextEdit { new_text: v, range }],
            )])),
            document_changes: None,
            change_annotations: None,
        }),
        ..Default::default()
    }
    .into()
}

fn new_update_command(package_name: &str) -> Command {
    Command::new(
        format!("cargo update {}", package_name),
        CARGO.to_string(),
        Some(vec![
            Value::String("update".to_string()),
            Value::String(package_name.to_string()),
        ]),
    )
}

fn strip_quotes(s: &str) -> String {
    if s.starts_with('"') && s.ends_with('"') {
        return s[1..s.len() - 1].to_string();
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_quotes_removes_double_quotes() {
        assert_eq!(strip_quotes("\"1.0.0\""), "1.0.0");
        assert_eq!(strip_quotes("\"hello world\""), "hello world");
    }

    #[test]
    fn test_strip_quotes_handles_unquoted() {
        assert_eq!(strip_quotes("1.0.0"), "1.0.0");
        assert_eq!(strip_quotes("hello"), "hello");
    }

    #[test]
    fn test_strip_quotes_handles_empty() {
        assert_eq!(strip_quotes("\"\""), "");
        assert_eq!(strip_quotes(""), "");
    }

    #[test]
    fn test_strip_quotes_partial_quotes() {
        // Only strips if both start AND end with quotes
        assert_eq!(strip_quotes("\"hello"), "\"hello");
        assert_eq!(strip_quotes("hello\""), "hello\"");
    }

    #[test]
    fn test_new_update_command() {
        let cmd = new_update_command("serde");
        assert_eq!(cmd.title, "cargo update serde");
        assert_eq!(cmd.command, CARGO);
        assert!(cmd.arguments.is_some());
        let args = cmd.arguments.unwrap();
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn test_new_code_action() {
        let uri: Uri = "file:///test.toml".parse().unwrap();
        let range = Range::default();
        let action = new_code_action(
            uri,
            "\"1.0.0\"".to_string(),
            CodeActionKind::REFACTOR,
            range,
            Some("Update to 1.0.0".to_string()),
        );

        match action {
            CodeActionOrCommand::CodeAction(ca) => {
                assert_eq!(ca.title, "Update to 1.0.0");
                assert_eq!(ca.kind, Some(CodeActionKind::REFACTOR));
                assert!(ca.edit.is_some());
            }
            _ => panic!("Expected CodeAction"),
        }
    }

    #[test]
    fn test_new_code_action_default_title() {
        let uri: Uri = "file:///test.toml".parse().unwrap();
        let range = Range::default();
        let action = new_code_action(
            uri,
            "\"2.0.0\"".to_string(),
            CodeActionKind::QUICKFIX,
            range,
            None, // No title provided, should use value
        );

        match action {
            CodeActionOrCommand::CodeAction(ca) => {
                assert_eq!(ca.title, "\"2.0.0\"");
            }
            _ => panic!("Expected CodeAction"),
        }
    }
}
