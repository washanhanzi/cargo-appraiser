use std::{collections::HashMap, fmt::Display};

use cargo::util::OptVersionReq;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionResponse, Range, TextEdit, Uri, WorkspaceEdit,
};

use crate::{
    decoration::{version_decoration, VersionDecoration},
    entity::{Dependency, DependencyEntryKind, EntryKind, TomlEntry},
};

pub fn code_action(uri: Uri, node: TomlEntry, dep: &Dependency) -> Option<CodeActionResponse> {
    if let EntryKind::Dependency(id, key) = &node.kind {
        code_action_dependency(uri, id, key, &node, dep)
    } else {
        None
    }
}

pub fn code_action_dependency(
    uri: Uri,
    id: &str,
    key: &DependencyEntryKind,
    node: &TomlEntry,
    dep: &Dependency,
) -> Option<CodeActionResponse> {
    match key {
        DependencyEntryKind::SimpleDependency | DependencyEntryKind::TableDependencyVersion => {
            let mut actions: CodeActionResponse = vec![];
            let version_deco = version_decoration(dep);
            let latest = dep.latest_summary.as_ref().map(|s| s.version());
            let latest_matched = dep.latest_matched_summary.as_ref().map(|s| s.version());
            //check version req contain minor or patch
            // if the version req contain minor, provide a code action with version only contain major version
            // if the version req contain patch, provide a code action with version only contain major and minor version
            let mut major_code_action = false;
            let mut minor_code_action = false;
            match dep.unresolved.as_ref().unwrap().version_req() {
                OptVersionReq::Req(req) => {
                    for r in &req.comparators {
                        if r.minor.is_some() {
                            major_code_action = true;
                        }
                        if r.minor.is_none() {
                            minor_code_action = true;
                        }
                        if r.patch.is_some() {
                            major_code_action = true;
                            minor_code_action = true;
                        }
                    }
                }
                _ => return None,
            };
            //TODO refactor
            match version_deco {
                VersionDecoration::Latest => {
                    let v = latest?;
                    if major_code_action {
                        actions.push(
                            make_code_action(
                                uri.clone(),
                                v.major,
                                CodeActionKind::REFACTOR,
                                node.range,
                            )
                            .into(),
                        );
                    }
                    if minor_code_action {
                        actions.push(
                            make_code_action(
                                uri.clone(),
                                format!("{}.{}", v.major, v.minor),
                                CodeActionKind::REFACTOR,
                                node.range,
                            )
                            .into(),
                        );
                    }
                }
                VersionDecoration::Local => return None,
                VersionDecoration::NotInstalled => return None,
                VersionDecoration::MixedUpgradeable => {
                    if let Some(v) = latest_matched {
                        actions.push(
                            make_code_action(uri.clone(), v, CodeActionKind::QUICKFIX, node.range)
                                .into(),
                        );
                    }
                    if let Some(v) = latest {
                        actions.push(
                            make_code_action(uri.clone(), v, CodeActionKind::QUICKFIX, node.range)
                                .into(),
                        );
                    }
                }
                VersionDecoration::CompatibleLatest => {
                    let v = latest?;
                    if major_code_action {
                        actions.push(
                            make_code_action(
                                uri.clone(),
                                v.major,
                                CodeActionKind::REFACTOR,
                                node.range,
                            )
                            .into(),
                        );
                    }
                    if minor_code_action {
                        actions.push(
                            make_code_action(
                                uri.clone(),
                                format!("{}.{}", v.major, v.minor),
                                CodeActionKind::REFACTOR,
                                node.range,
                            )
                            .into(),
                        );
                    }
                    actions.push(
                        make_code_action(uri.clone(), v, CodeActionKind::QUICKFIX, node.range)
                            .into(),
                    );
                }
                VersionDecoration::NoncompatibleLatest => {
                    if let Some(v) = latest {
                        actions.push(
                            make_code_action(uri.clone(), v, CodeActionKind::QUICKFIX, node.range)
                                .into(),
                        );
                    }
                }
                VersionDecoration::Yanked => {
                    if let Some(v) = latest {
                        actions.push(
                            make_code_action(uri.clone(), v, CodeActionKind::QUICKFIX, node.range)
                                .into(),
                        );
                    }
                    if let Some(v) = latest_matched {
                        actions.push(
                            make_code_action(uri, v, CodeActionKind::QUICKFIX, node.range).into(),
                        );
                    }
                }
                VersionDecoration::NotParsed => return None,
            };

            Some(actions)
        }
        _ => None,
    }
}

fn make_code_action(uri: Uri, v: impl Display, kind: CodeActionKind, range: Range) -> CodeAction {
    CodeAction {
        title: v.to_string(),
        kind: Some(kind),
        diagnostics: None,
        edit: Some(WorkspaceEdit {
            changes: Some(HashMap::from([(
                uri,
                vec![TextEdit {
                    new_text: format!("\"{}\"", v),
                    range,
                }],
            )])),
            document_changes: None,
            change_annotations: None,
        }),
        ..Default::default()
    }
}
