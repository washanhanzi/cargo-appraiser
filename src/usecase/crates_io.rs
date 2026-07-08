use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use moka::future::Cache;
use serde::Deserialize;
use tracing::{debug, error};

use crate::config::GLOBAL_CONFIG;

/// Cache TTL: 3 minutes
const CACHE_TTL: Duration = Duration::from_secs(3 * 60);

/// Global cache for crate index data (expires after CACHE_TTL)
static CRATE_INDEX_CACHE: LazyLock<Cache<String, CrateIndexData>> = LazyLock::new(|| {
    Cache::builder()
        .time_to_live(CACHE_TTL)
        .max_capacity(1000)
        .build()
});

/// Global cache for crate search results (expires after CACHE_TTL)
static CRATE_SEARCH_CACHE: LazyLock<Cache<String, Vec<CrateInfo>>> = LazyLock::new(|| {
    Cache::builder()
        .time_to_live(CACHE_TTL)
        .max_capacity(500)
        .build()
});

/// Parsed crate index data containing all versions and their features
#[derive(Debug, Clone)]
pub struct CrateIndexData {
    /// All versions, sorted newest first (non-yanked only)
    pub versions: Vec<String>,
    /// Features for each version
    pub features: HashMap<String, HashMap<String, Vec<String>>>,
}

/// A single version entry from the index
#[derive(Deserialize, Debug)]
struct IndexEntry {
    vers: String,
    features: HashMap<String, Vec<String>>,
    #[serde(default)]
    yanked: bool,
}

/// Information about a crate from crates.io search
#[derive(Debug, Clone)]
pub struct CrateInfo {
    pub name: String,
    pub max_version: String,
    pub description: Option<String>,
}

/// Get the sparse index URL path for a crate name.
/// Returns None for non-ASCII names (crates.io names are always ASCII);
/// byte-slicing a multi-byte name would panic.
fn index_path(crate_name: &str) -> Option<String> {
    let name = crate_name.to_lowercase();
    if !name.is_ascii() {
        return None;
    }
    let path = match name.len() {
        0 => return None,
        1 => format!("1/{}", name),
        2 => format!("2/{}", name),
        3 => format!("3/{}/{}", &name[..1], name),
        _ => format!("{}/{}/{}", &name[..2], &name[2..4], name),
    };
    Some(path)
}

/// Fetch and parse the crate index, using cache if available.
/// Concurrent calls for the same crate are coalesced: only one HTTP request
/// runs, the rest await its result. Failed fetches are not cached.
async fn get_crate_index(
    http_client: &reqwest::Client,
    crate_name: &str,
) -> Option<CrateIndexData> {
    let cache_key = crate_name.to_lowercase();
    CRATE_INDEX_CACHE
        .optionally_get_with(cache_key, fetch_crate_index(http_client, crate_name))
        .await
}

/// Fetch and parse the crate index from the sparse index (no caching)
async fn fetch_crate_index(
    http_client: &reqwest::Client,
    crate_name: &str,
) -> Option<CrateIndexData> {
    debug!("cache miss for '{}', fetching from index", crate_name);

    // Get base URL from config (None = feature disabled)
    let base_url = match &GLOBAL_CONFIG.read().crates_io.sparse_index_url {
        Some(url) => url.clone(),
        None => {
            debug!("sparse index lookup disabled by config");
            return None;
        }
    };

    // Fetch from sparse index
    let rel_path = index_path(crate_name)?;
    let url = format!("{}/{}", base_url, rel_path);

    let resp = match http_client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to fetch index for '{}': {}", crate_name, e);
            return None;
        }
    };

    if !resp.status().is_success() {
        error!(
            "index.crates.io returned {} for '{}'",
            resp.status(),
            crate_name
        );
        return None;
    }

    let text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            error!("Failed to read index response for '{}': {}", crate_name, e);
            return None;
        }
    };

    // Parse each line as a JSON entry
    let mut versions = Vec::new();
    let mut features_map = HashMap::new();

    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<IndexEntry>(line) {
            Ok(entry) => {
                if !entry.yanked {
                    versions.push(entry.vers.clone());
                }
                features_map.insert(entry.vers, entry.features);
            }
            Err(e) => {
                debug!("Failed to parse index line: {}", e);
            }
        }
    }

    // Sort newest first. The sparse index is publish-ordered, not
    // semver-ordered (backported patches are appended last), so a plain
    // reverse() would rank e.g. 0.2.26 above 1.0.0.
    versions.sort_by_cached_key(|v| std::cmp::Reverse(semver::Version::parse(v).ok()));

    debug!("parsed {} versions for '{}'", versions.len(), crate_name);

    Some(CrateIndexData {
        versions,
        features: features_map,
    })
}

/// Fetch available versions for a crate (non-yanked, newest first)
pub async fn fetch_versions(
    http_client: &reqwest::Client,
    crate_name: &str,
) -> Option<Vec<String>> {
    let index = get_crate_index(http_client, crate_name).await?;

    if index.versions.is_empty() {
        return None;
    }

    Some(index.versions)
}

/// Fetch features for a specific crate version.
/// The version can be a version requirement (e.g., "0.12") which will be resolved
/// to the latest matching exact version.
pub async fn fetch_features(
    http_client: &reqwest::Client,
    crate_name: &str,
    version_req_str: &str,
) -> Option<HashMap<String, Vec<String>>> {
    let index = get_crate_index(http_client, crate_name).await?;

    // Resolve version requirement to exact version
    let exact_version = resolve_version(&index, version_req_str)?;
    debug!(
        "resolved version '{}' to '{}'",
        version_req_str, exact_version
    );

    index.features.get(&exact_version).cloned()
}

/// Resolve a version requirement string to an exact version.
fn resolve_version(index: &CrateIndexData, version_req_str: &str) -> Option<String> {
    // First, check if it's already an exact version
    if index.features.contains_key(version_req_str) {
        return Some(version_req_str.to_string());
    }

    // Parse as version requirement (Cargo uses ^ by default)
    let version_req = semver::VersionReq::parse(version_req_str)
        .or_else(|_| semver::VersionReq::parse(&format!("^{}", version_req_str)))
        .ok()?;

    // Find the latest version that matches (versions are newest first)
    for v in &index.versions {
        if let Ok(ver) = semver::Version::parse(v) {
            if version_req.matches(&ver) {
                return Some(v.clone());
            }
        }
    }

    debug!("no version matching '{}' found", version_req_str);
    None
}

/// Get cached search results for a query, if present.
/// Lets callers skip their debounce delay on a cache hit.
pub async fn get_cached_search(search_str: &str) -> Option<Vec<CrateInfo>> {
    CRATE_SEARCH_CACHE.get(&search_str.to_lowercase()).await
}

/// Search crates.io for crates matching the given search string.
/// Results (including empty ones) are cached by lowercased query, and
/// concurrent calls for the same query are coalesced into one HTTP request.
/// Failed requests are not cached.
///
/// Debouncing is the caller's concern (see the controller layer).
pub async fn search_crates(
    http_client: &reqwest::Client,
    search_str: &str,
) -> Option<Vec<CrateInfo>> {
    let query = search_str.to_lowercase();
    debug!("search_crates() for: {}", query);

    CRATE_SEARCH_CACHE
        .optionally_get_with(query.clone(), fetch_search(http_client, &query))
        .await
}

/// Fetch search results from the crates.io API (no caching).
/// Returns Some (possibly empty) on success, None on error.
async fn fetch_search(http_client: &reqwest::Client, query: &str) -> Option<Vec<CrateInfo>> {
    // Get base URL from config (None = feature disabled)
    let base_url = match &GLOBAL_CONFIG.read().crates_io.api_url {
        Some(url) => url.clone(),
        None => {
            debug!("crates.io search disabled by config");
            return None;
        }
    };

    debug!("fetching from crates.io for '{}'", query);

    #[derive(Deserialize, Debug)]
    struct SearchCrateOutput {
        name: String,
        max_version: String,
        description: Option<String>,
    }

    #[derive(Deserialize, Debug)]
    struct SearchCrateResponse {
        crates: Vec<SearchCrateOutput>,
    }

    let resp = match http_client
        .get(&base_url)
        .query(&[("page", "1"), ("per_page", "10"), ("q", query)])
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to search crates.io for '{}': {}", query, e);
            return None;
        }
    };

    if !resp.status().is_success() {
        error!(
            "crates.io search returned {} for '{}'",
            resp.status(),
            query
        );
        return None;
    }

    let search_response: SearchCrateResponse = match resp.json().await {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to parse search response for '{}': {}", query, e);
            return None;
        }
    };

    debug!(
        "search_crates: HTTP response for '{}' returned {} crates",
        query,
        search_response.crates.len()
    );

    let crates: Vec<CrateInfo> = search_response
        .crates
        .into_iter()
        .map(|c| CrateInfo {
            name: c.name,
            max_version: c.max_version,
            description: c.description,
        })
        .collect();

    Some(crates)
}
