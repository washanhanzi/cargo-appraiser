use cargo::core::SourceId;

use super::Value;

//Package is a semantic representation of cargo.toml's package table
#[derive(Default, Debug, Clone)]
pub struct Package {
    name: Option<Value<String>>,
    version: Option<Value<String>>,
    edition: Option<Value<String>>,
    authors: Option<Value<String>>,
    description: Option<Value<String>>,
    license: Option<Value<String>>,
    repository: Option<Value<String>>,
    homepage: Option<Value<String>>,
    documentation: Option<Value<String>>,
    readme: Option<Value<String>>,
    workspace: Option<Value<String>>,
}

pub fn git_ref_str(source_id: &SourceId) -> Option<String> {
    if source_id.is_git() {
        let Some(r) = source_id.git_reference() else {
            return None;
        };
        match r.pretty_ref(false) {
            Some(r) => return Some(r.to_string()),
            None => return None,
        };
    }
    None
}

pub fn commit_str(source_id: &SourceId) -> Option<&str> {
    if source_id.is_git() {
        let Some(c) = source_id.precise_git_fragment() else {
            return None;
        };
        return Some(c);
    }
    None
}

pub fn commit_str_short(source_id: &SourceId) -> Option<&str> {
    // Get the full commit hash
    let Some(commit) = commit_str(source_id) else {
        return None;
    };

    // Handle case where commit hash is shorter than 7 chars
    if commit.len() < 7 {
        Some(commit)
    } else {
        Some(&commit[..7])
    }
}
