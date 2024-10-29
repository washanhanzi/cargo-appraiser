use std::collections::HashMap;

use tower_lsp::lsp_types::{Hover, HoverContents, MarkedString};

use crate::entity::{
    Dependency, DependencyEntryKind, DependencyKeyKind, EntryKind, KeyKind, NodeKind, TomlNode,
    WorkspaceKeyKind,
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
        NodeKind::Key(KeyKind::Dependency(_, DependencyKeyKind::Features))
        | NodeKind::Entry(EntryKind::Dependency(_, DependencyEntryKind::TableDependencyFeature)) => {
            let dep = dep?;
            let resolved = dep.resolved.as_ref()?;

            let features: HashMap<_, Vec<_>> = resolved
                .manifest()
                .summary()
                .features()
                .iter()
                .map(|(k, v)| (*k, v.iter().map(|fv| fv.to_string()).collect()))
                .collect();
            let feature_list = features
                .keys()
                .map(|key| format!("- {}", key))
                .collect::<Vec<_>>()
                .join("\n");

            Some(Hover {
                contents: HoverContents::Scalar(MarkedString::String(feature_list)),
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
        _ => None,
    }
}
