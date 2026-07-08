use std::{
    ops::Deref,
    path::{Path, PathBuf},
    str::FromStr,
};

use tower_lsp_server::ls_types::Uri;

/// A file URI normalized through the filesystem (symlinks resolved, `..`
/// segments removed, consistent casing on Windows via dunce), so that two
/// URIs referring to the same file compare equal. Used as the key type for
/// the workspace document map.
///
/// ls-types' `Uri` provides `from_file_path`/`to_file_path`; the
/// canonicalization step lives here (it used to be a patch in the forked
/// lsp-types crate).
#[derive(Debug, Clone, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub struct CanonicalUri(Uri);

impl TryFrom<Uri> for CanonicalUri {
    type Error = anyhow::Error;

    fn try_from(uri: Uri) -> Result<Self, Self::Error> {
        if uri.scheme().as_str() != "file" {
            // Non-file URIs have no filesystem identity to normalize.
            return Ok(CanonicalUri(uri));
        }
        let path = uri
            .to_file_path()
            .ok_or_else(|| anyhow::anyhow!("URI has no file path: {}", uri.as_str()))?;
        Self::try_from_path(path)
    }
}

impl CanonicalUri {
    pub fn try_from_path<T: AsRef<Path>>(path: T) -> Result<Self, anyhow::Error> {
        let canonical = dunce::canonicalize(&path).map_err(|e| {
            anyhow::anyhow!("failed to canonicalize path {:?}: {}", path.as_ref(), e)
        })?;
        let uri = Uri::from_file_path(&canonical)
            .ok_or_else(|| anyhow::anyhow!("failed to convert path to URI: {:?}", canonical))?;
        Ok(CanonicalUri(uri))
    }

    pub fn to_path_buf(&self) -> Result<PathBuf, anyhow::Error> {
        self.0
            .to_file_path()
            .map(|p| p.into_owned())
            .ok_or_else(|| anyhow::anyhow!("URI has no file path: {}", self.0.as_str()))
    }

    /// Map a `.toml` manifest URI to its sibling `.lock` file.
    /// Only the file extension is rewritten; other occurrences of ".toml"
    /// in the path must not be touched.
    pub fn ensure_lock(&self) -> Self {
        if let Some(stripped) = self.as_str().strip_suffix(".toml") {
            let lock = format!("{}.lock", stripped);
            if let Ok(uri) = Uri::from_str(&lock) {
                return CanonicalUri(uri);
            }
        }
        self.clone()
    }
}

impl Deref for CanonicalUri {
    type Target = Uri;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
