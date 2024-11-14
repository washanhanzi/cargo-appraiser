use std::collections::HashMap;

use cargo::util::OptVersionReq;
use semver::{Op, Version};
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionResponse, Range, TextEdit, Uri, WorkspaceEdit,
};

use crate::{
    decoration::{version_decoration, VersionDecoration},
    entity::{strip_quotes, Dependency, DependencyEntryKind, EntryKind, NodeKind, TomlNode},
};

pub fn code_action(uri: Uri, node: TomlNode, dep: &Dependency) -> Option<CodeActionResponse> {
    if let NodeKind::Entry(EntryKind::Dependency(id, key)) = &node.kind {
        code_action_dependency(uri, id, key, &node, dep)
    } else {
        None
    }
}

pub fn code_action_dependency(
    uri: Uri,
    id: &str,
    key: &DependencyEntryKind,
    node: &TomlNode,
    dep: &Dependency,
) -> Option<CodeActionResponse> {
    match key {
        DependencyEntryKind::SimpleDependency | DependencyEntryKind::TableDependencyVersion => {
            let version_deco = version_decoration(dep);
            let latest = dep.latest_summary.as_ref().map(|s| s.version());
            let latest_matched = dep.latest_matched_summary.as_ref().map(|s| s.version());
            let mut actions = VersionCodeAction::new(uri, node);
            actions.check_unresolved(dep);
            match version_deco {
                VersionDecoration::Latest => {
                    if let Some(v) = latest {
                        actions.add_refactor(v);
                    }
                }
                VersionDecoration::Local => return None,
                VersionDecoration::NotInstalled => return None,
                VersionDecoration::MixedUpgradeable => {
                    if let Some(v) = latest_matched {
                        actions.add_quickfix(v);
                    }
                    if let Some(v) = latest {
                        actions.add_quickfix(v);
                    }
                }
                VersionDecoration::CompatibleLatest => {
                    let v = latest?;
                    actions.add_refactor(v);
                    actions.add_quickfix(v);
                }
                VersionDecoration::NoncompatibleLatest => {
                    let v = latest?;
                    actions.add_quickfix(v);
                }
                VersionDecoration::Yanked => {
                    if let Some(v) = latest {
                        actions.add_quickfix(v);
                    }
                    if let Some(v) = latest_matched {
                        actions.add_quickfix(v);
                    }
                }
                VersionDecoration::NotParsed => return None,
            };
            actions.add_eq_refactor();
            actions.add_simple_table_refactor(dep);

            Some(actions.take())
        }
        _ => None,
    }
}

struct VersionCodeAction<'a> {
    uri: Uri,
    major_code_action: bool,
    minor_code_action: bool,
    actions: CodeActionResponse,
    node: &'a TomlNode,
    is_precise: bool,
}

impl<'a> VersionCodeAction<'a> {
    fn new(uri: Uri, node: &'a TomlNode) -> Self {
        Self {
            uri,
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
    fn check_unresolved(&mut self, dep: &Dependency) {
        if let Some(unresolved) = dep.unresolved.as_ref() {
            if let OptVersionReq::Req(req) = unresolved.version_req() {
                for r in &req.comparators {
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
    }

    fn add_simple_table_refactor(&mut self, dep: &Dependency) {
        if matches!(
            self.node.kind,
            NodeKind::Entry(EntryKind::Dependency(
                _,
                DependencyEntryKind::SimpleDependency,
            ))
        ) {
            self.add_code_action(
                format!("{{ version = {} }}", self.node.text),
                CodeActionKind::REFACTOR,
                dep.range,
                None,
            );
        }
        if matches!(
            self.node.kind,
            NodeKind::Entry(EntryKind::Dependency(
                _,
                DependencyEntryKind::TableDependencyVersion
            ))
        ) {
            self.add_code_action(
                self.node.text.to_string(),
                CodeActionKind::REFACTOR,
                dep.range,
                Some("Refactor to simple version".to_string()),
            );
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

    fn add_refactor(&mut self, v: &Version) {
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

    fn add_quickfix(&mut self, v: &Version) {
        self.add_code_action(
            format!("\"{}.{}\"", v.major, v.minor),
            CodeActionKind::QUICKFIX,
            self.node.range,
            None,
        );
        self.add_code_action(
            format!("\"{}\"", v),
            CodeActionKind::QUICKFIX,
            self.node.range,
            None,
        );
    }

    fn add_code_action(
        &mut self,
        v: String,
        kind: CodeActionKind,
        range: Range,
        title: Option<String>,
    ) {
        self.actions.push(
            CodeAction {
                title: title.unwrap_or(v.to_string()),
                kind: Some(kind),
                diagnostics: None,
                edit: Some(WorkspaceEdit {
                    changes: Some(HashMap::from([(
                        self.uri.clone(),
                        vec![TextEdit { new_text: v, range }],
                    )])),
                    document_changes: None,
                    change_annotations: None,
                }),
                ..Default::default()
            }
            .into(),
        );
    }
}
