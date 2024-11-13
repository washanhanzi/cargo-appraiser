use std::{path::Path, str::FromStr};

use tower_lsp::lsp_types::Uri;

pub fn into_file_uri(path: &Path) -> Uri {
    path.to_str().map(into_file_uri_str).unwrap()
}

pub fn into_file_uri_str(path: &str) -> Uri {
    Uri::from_str(&format!("file://{}", path)).unwrap()
}
