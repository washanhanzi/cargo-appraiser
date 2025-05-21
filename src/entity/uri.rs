use dunce::canonicalize;
use std::{path::Path, str::FromStr};
use tower_lsp::lsp_types::Uri;

/// Convert a filesystem path to a file:// URI, handling Windows paths correctly.
pub fn into_file_uri(path: &Path) -> Uri {
    // Use dunce to handle path canonicalization and UNC path conversion on Windows
    let path = canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    // Convert to string and handle Windows paths
    let path_str = path.to_str().unwrap_or_default();
    into_file_uri_str(path_str)
}

/// Convert a path string to a file:// URI, handling Windows paths correctly.
pub fn into_file_uri_str(path: &str) -> Uri {
    // On Windows, we need to handle drive letters and backslashes
    let path = if cfg!(windows) {
        // Convert backslashes to forward slashes
        path.replace('\\', "/")
    } else {
        path.to_string()
    };

    // Ensure the path starts with a single slash
    let path = if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    };

    // Handle Windows drive letters (e.g., C:)
    let path = if cfg!(windows) && path.len() > 1 && path.chars().nth(1) == Some(':') {
        // Convert C:/path to /c:/path
        format!("/{}{}", &path[0..1].to_lowercase(), &path[1..])
    } else {
        path
    };

    // Create the URI with proper encoding
    let uri_str = format!("file://{path}");
    Uri::from_str(&uri_str).unwrap_or_else(|_| {
        tracing::error!("Failed to parse URI: {uri_str}");
        Uri::from_str("file:///").unwrap()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_into_file_uri_unix() {
        let path = Path::new("/home/user/project/Cargo.toml");
        let uri = into_file_uri(path);
        assert_eq!(uri.to_string(), "file:///home/user/project/Cargo.toml");
    }

    #[test]
    fn test_into_file_uri_windows() {
        let path = Path::new("E:\\projects\\test\\Cargo.toml");
        let uri = into_file_uri(path);
        // The actual output will depend on the platform, but it should be a valid URI
        if cfg!(windows) {
            assert!(uri
                .to_string()
                .starts_with("file:///e:/projects/test/Cargo.toml"));
        }
    }

    #[test]
    fn test_into_file_uri_str() {
        let path = "/path/with spaces/file.txt";
        let uri = into_file_uri_str(path);
        assert_eq!(uri.to_string(), "file:///path/with%20spaces/file.txt");
    }
}
