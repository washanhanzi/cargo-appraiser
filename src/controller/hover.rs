use std::collections::HashMap;

use tower_lsp::lsp_types::{Hover, HoverContents, MarkedString};
use tracing::info;

use crate::entity::{
    commit_str, git_ref_str, Dependency, DependencyEntryKind, DependencyKeyKind, EntryKind,
    KeyKind, NodeKind, TomlNode, WorkspaceKeyKind,
};

pub fn hover(
    node: &TomlNode,
    dep: Option<&Dependency>,
    members: Option<&[cargo::core::package::Package]>,
) -> Option<Hover> {
    match node.kind {
        NodeKind::Key(KeyKind::Dependency(_, DependencyKeyKind::Version))
        | NodeKind::Entry(EntryKind::Dependency(_, DependencyEntryKind::TableDependencyVersion))
        | NodeKind::Entry(EntryKind::Dependency(_, DependencyEntryKind::SimpleDependency)) => {
            let dep = dep?;
            let summaries = dep.summaries.as_ref()?;
            let mut versions = summaries
                .iter()
                .map(|s| s.version().clone())
                .collect::<Vec<_>>();

            versions.sort_by(|a, b| b.cmp(a));

            let formatted_versions = versions
                .iter()
                .map(|v| format!("- {}", v))
                .collect::<Vec<_>>()
                .join("\n");

            Some(Hover {
                contents: HoverContents::Scalar(MarkedString::String(formatted_versions)),
                range: Some(node.range),
            })
        }
        NodeKind::Key(KeyKind::Dependency(_, DependencyKeyKind::Features)) => {
            let dep = dep?;
            let resolved = dep.resolved.as_ref()?;

            let features: HashMap<_, Vec<_>> = resolved
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
                contents: HoverContents::Scalar(MarkedString::String(s)),
                range: Some(node.range),
            })
        }
        NodeKind::Entry(EntryKind::Dependency(_, DependencyEntryKind::TableDependencyFeature)) => {
            let dep = dep?;
            let resolved = dep.resolved.as_ref()?;
            let feature = resolved
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
                contents: HoverContents::Scalar(MarkedString::String(s)),
                range: Some(node.range),
            })
        }
        NodeKind::Key(KeyKind::Workspace(WorkspaceKeyKind::Members)) => {
            let members = members?;
            let member_list = members
                .iter()
                .map(|m| format!("- [{}]({})", m.name(), m.manifest_path().display()))
                .collect::<Vec<_>>()
                .join("\n");
            Some(Hover {
                contents: HoverContents::Scalar(MarkedString::String(member_list)),
                range: Some(node.range),
            })
        }
        NodeKind::Entry(EntryKind::Dependency(_, DependencyEntryKind::TableDependencyGit)) => {
            let source_id = dep?.resolved.as_ref()?.package_id().source_id();
            let git_ref = git_ref_str(&source_id);
            let commit = commit_str(&source_id);
            //make a new string of markdown list "- <git_ref>\n - <commit>\n"
            //if git_ref is some and commit is some
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
                contents: HoverContents::Scalar(MarkedString::String(s)),
                range: Some(node.range),
            })
        }
        _ => None,
    }
}
