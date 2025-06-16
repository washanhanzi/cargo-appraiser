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

    pub fn ensure_lock(&self) -> Self {
        if self.path().as_str().ends_with(".toml") {
            let path = self.as_str().replace(".toml", ".lock");
            return CanonicalUri(Uri::from_str(&path).unwrap());
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
