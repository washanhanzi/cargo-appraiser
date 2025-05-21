use dunce::canonicalize;
use std::{
    path::{Path, PathBuf},
    str::FromStr,
};
use tower_lsp::lsp_types::Uri;

/// Convert a filesystem path to a file:// URI, handling Windows paths correctly.
pub fn into_uri(path: &Path) -> Uri {
    // Use dunce to handle path canonicalization and UNC path conversion on Windows
    let path = canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    // Convert to string and handle Windows paths
    let path_str = path.to_str().unwrap_or_default();
    into_file_uri_str(path_str)
}

pub fn into_path(uri: &Uri) -> PathBuf {
    #[cfg(windows)]
    {
        into_path_win(uri)
    }
    #[cfg(not(windows))]
    {
        Path::new(uri.path().as_str()).to_path_buf()
    }
}

#[cfg(windows)]
fn into_path_win(uri: &Uri) -> PathBuf {
    use percent_encoding::percent_decode_str;
    let path_str = uri.path().as_str();
    let decoded = percent_decode_str(path_str)
        .decode_utf8()
        .unwrap()
        .to_string();
    let windows_path = decoded.trim_start_matches('/').replace('/', "\\");
    let path = Path::new(&windows_path);
    canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Convert a path string to a file:// URI, handling Windows paths correctly.
pub fn into_file_uri_str(path: &str) -> Uri {
    // Skip if path is already a URI
    if path.starts_with("file://") {
        return Uri::from_str(path).unwrap_or_else(|_| {
            tracing::error!("Failed to parse existing URI: {path}");
            Uri::from_str("file:///").unwrap()
        });
    }

    // Handle Windows paths by converting backslashes to forward slashes
    let path = path.replace('\\', "/");

    // Handle Windows drive letters (e.g., C:)
    let path = if cfg!(windows) && path.len() >= 2 && path.chars().nth(1) == Some(':') {
        // Ensure paths with drive letters are properly formatted
        if path.starts_with('/') {
            path
        } else {
            // Convert C:/path to /C:/path for Windows
            format!(
                "/{}",
                path.chars().next().unwrap().to_lowercase().to_string() + &path[1..]
            )
        }
    } else if !path.starts_with('/') {
        // Add leading slash if needed
        format!("/{path}")
    } else {
        path
    };

    // Manually encode spaces and special characters
    let encoded_path = path
        .split('/')
        .map(|part| {
            if part.is_empty() {
                String::new()
            } else if part.contains(' ') || part.contains('#') || part.contains('?') {
                // Only encode parts with spaces or special characters
                part.replace(" ", "%20")
                    .replace("#", "%23")
                    .replace("?", "%3F")
                    .replace("=", "%3D")
                    .replace("&", "%26")
                    .replace("+", "%2B")
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("/");

    // Create the URI
    let uri_str = format!("file://{encoded_path}");
    tracing::debug!("Generated URI: {uri_str}");

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
    fn test_into_path() {
        let uri = Uri::from_str("file:///home/user/project/Cargo.toml").unwrap();
        let path = into_path(&uri);
        assert_eq!(path.to_str().unwrap(), "/home/user/project/Cargo.toml");

        let uri = Uri::from_str("file:///c%3A/Users/project/Cargo.toml").unwrap();
        let path = into_path(&uri);
        assert_eq!(path.to_str().unwrap(), "c:/Users/project/Cargo.toml");
    }

    #[test]
    fn test_into_file_uri_unix() {
        let path = Path::new("/home/user/project/Cargo.toml");
        let uri = into_uri(path);
        assert_eq!(uri.to_string(), "file:///home/user/project/Cargo.toml");
    }

    #[test]
    fn test_into_file_uri_windows() {
        let path = Path::new("E:\\projects\\test\\Cargo.toml");
        let url = url::Url::from_file_path(path).unwrap();
        println!("path: {}", url);
        let uri = tower_lsp::lsp_types::Uri::from_str(path.to_str().unwrap()).unwrap();
        println!("uri: {}", uri.to_string());

        let uri = into_uri(&path);
        // The actual output will depend on the platform, but it should be a valid URI
        if cfg!(windows) {
            assert!(uri
                .to_string()
                .starts_with("file:///e:/projects/test/Cargo.toml"));
        }
    }

    #[test]
    fn test_into_file_uri_str() {
        // The tower-lsp Uri type handles URL encoding automatically
        let path = "/path/with spaces/file.txt";
        let uri = into_file_uri_str(path);
        assert_eq!(uri.to_string(), "file:///path/with%20spaces/file.txt");
    }
}
