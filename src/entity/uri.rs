use std::{ops::Deref, path::Path, str::FromStr};

use tower_lsp::lsp_types::Uri;

#[derive(Debug, Clone, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub struct CanonicalUri(Uri);

impl TryFrom<Uri> for CanonicalUri {
    type Error = anyhow::Error;

    fn try_from(uri: Uri) -> Result<Self, Self::Error> {
        let canonical_uri = uri.canonical().map_err(anyhow::Error::from)?;
        Ok(CanonicalUri(canonical_uri))
    }
}

impl CanonicalUri {
    pub fn try_from_path<T: AsRef<Path>>(path: T) -> Result<Self, anyhow::Error> {
        let uri = Uri::try_from_path(path).map_err(anyhow::Error::from)?;
        Ok(CanonicalUri(uri))
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
