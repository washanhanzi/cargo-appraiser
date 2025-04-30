use std::collections::HashMap;

use cargo::util::OptVersionReq;
use semver::{Op, Version};
use serde_json::Value;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionResponse, Command, Range, TextEdit,
    Uri, WorkspaceEdit,
};

use crate::{
    decoration::{version_decoration, VersionDecorationKind},
    entity::{
        strip_quotes, Dependency, DependencyEntryKind, DependencyKeyKind, EntryKind, KeyKind,
        NodeKind, TomlNode, CARGO,
    },
};

pub fn code_action(
    uri: Uri,
    node: TomlNode,
    dep: Option<&Dependency>,
) -> Option<CodeActionResponse> {
    //only support dependency code action fro now
    let dep = dep?;
    code_action_dependency(uri, &node, dep)
}

pub fn code_action_dependency(
    uri: Uri,
    node: &TomlNode,
    dep: &Dependency,
) -> Option<CodeActionResponse> {
    match node.kind {
        NodeKind::Entry(EntryKind::Dependency(_, DependencyEntryKind::SimpleDependency))
        | NodeKind::Entry(EntryKind::Dependency(_, DependencyEntryKind::TableDependencyVersion)) => {
            let version = version_decoration(dep);
            let mut actions = VersionCodeAction::new(uri, node);
            actions.check_unresolved(dep);
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
                        actions.add_quickfix(v);
                        // actions.add_precies_update_command(dep.package_name(), v);
                    }
                    if let Some(v) = version.latest.as_ref() {
                        actions.add_quickfix(v);
                    }
                    actions.add_update_command(dep.package_name());
                }
                VersionDecorationKind::CompatibleLatest => {
                    let v = version.latest.as_ref()?;
                    actions.add_refactor(v);
                    actions.add_quickfix(v);
                    // actions.add_precies_update_command(dep.package_name(), v);
                    actions.add_update_command(dep.package_name());
                }
                VersionDecorationKind::NonCompatibleLatest => {
                    let v = version.latest.as_ref()?;
                    actions.add_quickfix(v);
                }
                VersionDecorationKind::Yanked => {
                    if let Some(v) = version.latest.as_ref() {
                        actions.add_quickfix(v);
                    }
                    if let Some(v) = version.latest_matched.as_ref() {
                        actions.add_quickfix(v);
                    }
                    actions.add_update_command(dep.package_name());
                }
                VersionDecorationKind::Git => {
                    actions.add_update_command(dep.package_name());
                }
                VersionDecorationKind::NotParsed => return None,
            };
            actions.add_eq_refactor();
            if let Some(p) = dep.resolved.as_ref() {
                actions.add_precise_eq_refactor(p.version());
            }
            actions.add_simple_table_refactor(dep);

            Some(actions.take())
        }
        NodeKind::Key(KeyKind::Dependency(_, DependencyKeyKind::Workspace))
        | NodeKind::Entry(EntryKind::Dependency(
            _,
            DependencyEntryKind::TableDependencyWorkspace,
        )) => {
            let version = version_decoration(dep);
            let mut actions = VersionCodeAction::new(uri, node);
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
        if let Some(unresolved) = dep.requested.as_ref() {
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

    fn add_precise_eq_refactor(&mut self, v: &Version) {
        if !self.is_precise {
            self.add_code_action(
                format!("\"={}\"", v),
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
        self.actions
            .push(new_code_action(self.uri.clone(), v, kind, range, title));
    }

    fn add_precies_update_command(&mut self, package_name: &str, v: &Version) {
        self.actions
            .push(new_precise_update_command(package_name, v).into());
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

fn new_precise_update_command(package_name: &str, v: &Version) -> Command {
    Command::new(
        format!("cargo update {} --precise {}", package_name, v),
        CARGO.to_string(),
        Some(vec![
            Value::String("update".to_string()),
            Value::String(package_name.to_string()),
            Value::String("--precise".to_string()),
            Value::String(v.to_string()),
        ]),
    )
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
