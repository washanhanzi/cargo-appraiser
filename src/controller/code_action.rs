use std::collections::HashMap;

use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionResponse, TextEdit, Url, WorkspaceEdit,
};

use crate::{
    decoration::{version_decoration, VersionDecoration},
    entity::{EntryKind, TomlEntry, Dependency, DependencyEntryKind},
};

pub fn code_action(uri: Url, node: TomlEntry, dep: &Dependency) -> Option<CodeActionResponse> {
    if let EntryKind::Dependency(id, key) = &node.kind {
        code_action_dependency(uri, id, key, &node, dep)
    } else {
        None
    }
}

pub fn code_action_dependency(
    uri: Url,
    id: &str,
    key: &DependencyEntryKind,
    node: &TomlEntry,
    dep: &Dependency,
) -> Option<CodeActionResponse> {
    match key {
        DependencyEntryKind::SimpleDependency | DependencyEntryKind::TableDependencyVersion => {
            let mut actions: CodeActionResponse = vec![];
            let version_deco = version_decoration(dep);
            if version_deco == VersionDecoration::Latest {
                return None;
            }
            let latest = dep.latest_summary.as_ref().map(|s| s.version());
            let latest_matched = dep.latest_matched_summary.as_ref().map(|s| s.version());
            //TODO refactor
            match version_deco {
                VersionDecoration::Latest => return None,
                VersionDecoration::Local => return None,
                VersionDecoration::NotInstalled => return None,
                VersionDecoration::MixedUpgradeable => {
                    if let Some(v) = latest_matched {
                        let latest_matched = format!("\"{}\"", v);
                        actions.push(
                            CodeAction {
                                title: v.to_string(),
                                kind: Some(CodeActionKind::QUICKFIX),
                                diagnostics: None,
                                edit: Some(WorkspaceEdit {
                                    changes: Some(HashMap::from([(
                                        uri.clone(),
                                        vec![TextEdit {
                                            new_text: latest_matched,
                                            range: node.range,
                                        }],
                                    )])),
                                    document_changes: None,
                                    change_annotations: None,
                                }),
                                ..Default::default()
                            }
                            .into(),
                        );
                    }
                    if let Some(v) = latest {
                        let latest = format!("\"{}\"", v);
                        actions.push(
                            CodeAction {
                                title: v.to_string(),
                                kind: Some(CodeActionKind::QUICKFIX),
                                diagnostics: None,
                                edit: Some(WorkspaceEdit {
                                    changes: Some(HashMap::from([(
                                        uri,
                                        vec![TextEdit {
                                            new_text: latest,
                                            range: node.range,
                                        }],
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
                VersionDecoration::CompatibleLatest => {
                    if let Some(v) = latest_matched {
                        let latest_matched = format!("\"{}\"", v);
                        actions.push(
                            CodeAction {
                                title: v.to_string(),
                                kind: Some(CodeActionKind::QUICKFIX),
                                diagnostics: None,
                                edit: Some(WorkspaceEdit {
                                    changes: Some(HashMap::from([(
                                        uri,
                                        vec![TextEdit {
                                            new_text: latest_matched,
                                            range: node.range,
                                        }],
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
                VersionDecoration::NoncompatibleLatest => {
                    if let Some(v) = latest {
                        let latest = format!("\"{}\"", v);
                        actions.push(
                            CodeAction {
                                title: v.to_string(),
                                kind: Some(CodeActionKind::QUICKFIX),
                                diagnostics: None,
                                edit: Some(WorkspaceEdit {
                                    changes: Some(HashMap::from([(
                                        uri,
                                        vec![TextEdit {
                                            new_text: latest,
                                            range: node.range,
                                        }],
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
                VersionDecoration::Yanked => {
                    if let Some(v) = latest {
                        let latest = format!("\"{}\"", v);
                        actions.push(
                            CodeAction {
                                title: v.to_string(),
                                kind: Some(CodeActionKind::QUICKFIX),
                                diagnostics: None,
                                edit: Some(WorkspaceEdit {
                                    changes: Some(HashMap::from([(
                                        uri.clone(),
                                        vec![TextEdit {
                                            new_text: latest,
                                            range: node.range,
                                        }],
                                    )])),
                                    document_changes: None,
                                    change_annotations: None,
                                }),
                                ..Default::default()
                            }
                            .into(),
                        );
                    }
                    if let Some(v) = latest_matched {
                        let latest_matched = format!("\"{}\"", v);
                        actions.push(
                            CodeAction {
                                title: v.to_string(),
                                kind: Some(CodeActionKind::QUICKFIX),
                                diagnostics: None,
                                edit: Some(WorkspaceEdit {
                                    changes: Some(HashMap::from([(
                                        uri,
                                        vec![TextEdit {
                                            new_text: latest_matched,
                                            range: node.range,
                                        }],
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
                VersionDecoration::NotParsed => return None,
            };

            Some(actions)
        }
        _ => None,
    }
}
