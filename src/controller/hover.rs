use std::collections::HashMap;

use tower_lsp::lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind};

use crate::entity::{
    commit_str, git_ref_str, DependencyKey, DependencyValue, NodeKind, ResolvedDependency,
    TomlDependency, TomlNode, ValueKind,
};

pub fn hover(
    node: &TomlNode,
    _dep: Option<&TomlDependency>,
    resolved: Option<&ResolvedDependency>,
    members: Option<&[cargo::core::package::Package]>,
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
        NodeKind::Key(key_kind)
            if matches!(
                key_kind,
                crate::entity::KeyKind::Dependency(DependencyKey::Features)
            ) =>
        {
            let resolved = resolved?;
            let pkg = resolved.package.as_ref()?;

            let features: HashMap<_, Vec<_>> = pkg
                .manifest()
                .summary()
                .features()
                .iter()
                .map(|(k, v)| (*k, v.iter().map(|fv| fv.to_string()).collect()))
                .collect();
            let mut feature_list = features.keys().collect::<Vec<_>>();
            feature_list.sort();
            let mut s = String::new();
            for key in feature_list {
                s.push_str(&format!("- {}", key));
                if !features[key].is_empty() {
                    s.push_str(": [");
                    s.push_str(&features[key].join(", "));
                    s.push(']');
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
            let pkg = resolved.package.as_ref()?;
            let feature = pkg
                .manifest()
                .summary()
                .features()
                .iter()
                .filter(|(f, _)| f.to_string() == node.text)
                .collect::<Vec<_>>();
            let mut s = String::new();
            for (_, v) in feature {
                for fv in v {
                    s.push_str(&format!("- {}\n", fv));
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
        NodeKind::Key(key_kind)
            if matches!(
                key_kind,
                crate::entity::KeyKind::Workspace(crate::entity::WorkspaceKey::Members)
            ) =>
        {
            let members = members?;
            let member_list = members
                .iter()
                .map(|m| format!("- [{}]({})", m.name(), m.manifest_path().display()))
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
            let pkg = resolved.package.as_ref()?;
            let source_id = pkg.package_id().source_id();
            let git_ref = git_ref_str(&source_id);
            let commit = commit_str(&source_id);
            let mut s = String::new();
            if let Some(git_ref) = git_ref {
                s.push_str(&format!("- {}\n", git_ref));
            }
            if let Some(commit) = commit {
                s.push_str(&format!("- {}\n", commit));
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
