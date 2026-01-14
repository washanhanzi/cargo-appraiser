use tokio::sync::oneshot;
use tower_lsp::lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position, Uri};
use tracing::error;

use crate::entity::{
    DependencyKey, DependencyValue, KeyKind, NodeKind, ResolvedDependency, SourceKind,
    TomlDependency, TomlNode, ValueKind, WorkspaceKey, WorkspaceMember,
};

use super::context::AppraiserContext;

/// Handle `CargoDocumentEvent::Hovered` - provide hover information.
pub async fn handle_hover(
    ctx: &mut AppraiserContext<'_>,
    uri: Uri,
    pos: Position,
    tx: oneshot::Sender<Option<Hover>>,
) {
    let Ok(canonical_uri) = uri.clone().try_into() else {
        error!("failed to canonicalize uri: {}", uri.as_str());
        return;
    };

    let Some(doc) = ctx.state.document(&canonical_uri) else {
        return;
    };

    let Some(node) = doc.precise_match(pos) else {
        return;
    };

    // Find the dependency for this node
    let dep = doc.tree().find_dependency_at_position(pos);
    let resolved = dep.and_then(|d| doc.resolved(&d.id));
    let h = hover(node, dep, resolved, doc.members.as_deref());
    let _ = tx.send(h);
}

pub fn hover(
    node: &TomlNode,
    _dep: Option<&TomlDependency>,
    resolved: Option<&ResolvedDependency>,
    members: Option<&[WorkspaceMember]>,
) -> Option<Hover> {
    match &node.kind {
        // Version hover - show available versions
        NodeKind::Value(ValueKind::Dependency(DependencyValue::Version))
        | NodeKind::Value(ValueKind::Dependency(DependencyValue::Simple)) => {
            let resolved = resolved?;
            let available_versions = &resolved.available_versions;
            if available_versions.is_empty() {
                return None;
            }

            // available_versions is already sorted by version (descending)
            let formatted_versions = available_versions
                .iter()
                .map(|v| format!("- {}", v))
                .collect::<Vec<_>>()
                .join("\n");

            Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: formatted_versions,
                }),
                range: Some(node.range),
            })
        }
        // Features hover - show all features
        NodeKind::Key(KeyKind::Dependency(DependencyKey::Features)) => {
            let resolved = resolved?;
            let features = resolved.features()?;

            let mut feature_list: Vec<_> = features.keys().collect();
            feature_list.sort();
            let mut s = String::new();
            for key in feature_list {
                s.push_str(&format!("- {}", key));
                if let Some(deps) = features.get(key) {
                    if !deps.is_empty() {
                        s.push_str(": [");
                        s.push_str(&deps.join(", "));
                        s.push(']');
                    }
                }
                s.push('\n');
            }

            Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: s,
                }),
                range: Some(node.range),
            })
        }
        // Single feature hover - show what it enables
        NodeKind::Value(ValueKind::Dependency(DependencyValue::Feature)) => {
            let resolved = resolved?;
            let features = resolved.features()?;

            let mut s = String::new();
            if let Some(deps) = features.get(&node.text) {
                for dep in deps {
                    s.push_str(&format!("- {}\n", dep));
                }
            }
            if s.is_empty() {
                return None;
            }
            Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: s,
                }),
                range: Some(node.range),
            })
        }
        // Workspace members hover
        NodeKind::Key(KeyKind::Workspace(WorkspaceKey::Members)) => {
            let members = members?;
            let member_list = members
                .iter()
                .map(|m| format!("- [{}]({})", m.name, m.manifest_path.display()))
                .collect::<Vec<_>>()
                .join("\n");
            Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: member_list,
                }),
                range: Some(node.range),
            })
        }
        // Git dependency hover - show ref and commit
        NodeKind::Value(ValueKind::Dependency(DependencyValue::Git)) => {
            let resolved = resolved?;
            let source = resolved.source_kind()?;

            let mut s = String::new();
            if let SourceKind::Git {
                reference,
                full_commit,
                ..
            } = source
            {
                if let Some(git_ref) = reference {
                    s.push_str(&format!("- {}\n", git_ref));
                }
                if let Some(commit) = full_commit {
                    s.push_str(&format!("- {}\n", commit));
                }
            }
            if s.is_empty() {
                return None;
            }
            Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: s,
                }),
                range: Some(node.range),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tower_lsp::lsp_types::Range;

    fn make_range() -> Range {
        Range::default()
    }

    #[test]
    fn test_hover_version_shows_available_versions() {
        let node = TomlNode::new(
            "test".to_string(),
            make_range(),
            "1.0".to_string(),
            NodeKind::Value(ValueKind::Dependency(DependencyValue::Version)),
        );

        let resolved = ResolvedDependency {
            package: None,
            available_versions: vec![
                "2.0.0".to_string(),
                "1.5.0".to_string(),
                "1.0.0".to_string(),
            ],
            latest_matched_version: None,
            latest_version: None,
        };

        let result = hover(&node, None, Some(&resolved), None);
        assert!(result.is_some());

        let hover = result.unwrap();
        if let HoverContents::Markup(content) = hover.contents {
            assert!(content.value.contains("2.0.0"));
            assert!(content.value.contains("1.5.0"));
            assert!(content.value.contains("1.0.0"));
        } else {
            panic!("Expected Markup content");
        }
    }

    #[test]
    fn test_hover_version_returns_none_when_no_versions() {
        let node = TomlNode::new(
            "test".to_string(),
            make_range(),
            "1.0".to_string(),
            NodeKind::Value(ValueKind::Dependency(DependencyValue::Version)),
        );

        let resolved = ResolvedDependency {
            package: None,
            available_versions: vec![],
            latest_matched_version: None,
            latest_version: None,
        };

        let result = hover(&node, None, Some(&resolved), None);
        assert!(result.is_none());
    }

    #[test]
    fn test_hover_returns_none_for_unknown_node() {
        let node = TomlNode::new(
            "test".to_string(),
            make_range(),
            "test".to_string(),
            NodeKind::Value(ValueKind::Other),
        );

        let result = hover(&node, None, None, None);
        assert!(result.is_none());
    }

    #[test]
    fn test_hover_returns_none_when_resolved_is_none() {
        let node = TomlNode::new(
            "test".to_string(),
            make_range(),
            "1.0".to_string(),
            NodeKind::Value(ValueKind::Dependency(DependencyValue::Version)),
        );

        let result = hover(&node, None, None, None);
        assert!(result.is_none());
    }

    #[test]
    fn test_hover_workspace_members() {
        let node = TomlNode::new(
            "test".to_string(),
            make_range(),
            "members".to_string(),
            NodeKind::Key(KeyKind::Workspace(WorkspaceKey::Members)),
        );

        let members = vec![
            WorkspaceMember {
                name: "crate-a".to_string(),
                manifest_path: PathBuf::from("/workspace/crate-a/Cargo.toml"),
            },
            WorkspaceMember {
                name: "crate-b".to_string(),
                manifest_path: PathBuf::from("/workspace/crate-b/Cargo.toml"),
            },
        ];

        let result = hover(&node, None, None, Some(&members));
        assert!(result.is_some());

        let hover = result.unwrap();
        if let HoverContents::Markup(content) = hover.contents {
            assert!(content.value.contains("crate-a"));
            assert!(content.value.contains("crate-b"));
        } else {
            panic!("Expected Markup content");
        }
    }

    #[test]
    fn test_hover_simple_dependency() {
        // Simple dependency should behave same as Version
        let node = TomlNode::new(
            "test".to_string(),
            make_range(),
            "1.0".to_string(),
            NodeKind::Value(ValueKind::Dependency(DependencyValue::Simple)),
        );

        let resolved = ResolvedDependency {
            package: None,
            available_versions: vec!["1.0.0".to_string()],
            latest_matched_version: None,
            latest_version: None,
        };

        let result = hover(&node, None, Some(&resolved), None);
        assert!(result.is_some());
    }
}
