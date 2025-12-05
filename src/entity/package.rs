use cargo::core::SourceId;

pub fn git_ref_str(source_id: &SourceId) -> Option<String> {
    if source_id.is_git() {
        let r = source_id.git_reference()?;
        match r.pretty_ref(false) {
            Some(r) => return Some(r.to_string()),
            None => return None,
        };
    }
    None
}

pub fn commit_str(source_id: &SourceId) -> Option<&str> {
    if source_id.is_git() {
        source_id.precise_git_fragment()
    } else {
        None
    }
}

pub fn commit_str_short(source_id: &SourceId) -> Option<&str> {
    // Get the full commit hash
    let commit = commit_str(source_id)?;

    // Handle case where commit hash is shorter than 7 chars
    if commit.len() < 7 {
        Some(commit)
    } else {
        Some(&commit[..7])
    }
}
