use tower_lsp::lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind};

use crate::entity::{
    DependencyKey, DependencyValue, KeyKind, NodeKind, ResolvedDependency, SourceKind,
    TomlDependency, TomlNode, ValueKind, WorkspaceKey, WorkspaceMember,
};

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
