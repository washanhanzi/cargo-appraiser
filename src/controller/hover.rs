use tower_lsp::lsp_types::{Hover, HoverContents, MarkedString};

use crate::entity::{Dependency, DependencyEntryKind, EntryKind, TomlEntry};

pub fn hover(node: TomlEntry, dep: &Dependency) -> Option<Hover> {
    if let EntryKind::Dependency(id, key) = &node.kind {
        hover_dependency(id, key, &node, dep)
    } else {
        None
    }
}

fn hover_dependency(
    id: &str,
    key: &DependencyEntryKind,
    node: &TomlEntry,
    dep: &Dependency,
) -> Option<Hover> {
    match key {
        DependencyEntryKind::TableDependencyVersion | DependencyEntryKind::SimpleDependency => {
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
        DependencyEntryKind::TableDependencyFeatures | DependencyEntryKind::TableDependencyFeature => {
            let resolved = dep.resolved.as_ref()?;
            let features = resolved.features.clone();
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
        _ => None,
    }
}
