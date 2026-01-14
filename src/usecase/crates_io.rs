use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;
use std::time::Duration;

use moka::future::Cache;
use serde::Deserialize;
use tracing::{debug, error};

/// Cache TTL: 3 minutes
const CACHE_TTL: Duration = Duration::from_secs(3 * 60);

/// Global cache for crate index data with 5-minute TTL
static CRATE_INDEX_CACHE: LazyLock<Cache<String, CrateIndexData>> = LazyLock::new(|| {
    Cache::builder()
        .time_to_live(CACHE_TTL)
        .max_capacity(1000)
        .build()
});

/// Global cache for crate search results with 5-minute TTL
static CRATE_SEARCH_CACHE: LazyLock<Cache<String, Vec<CrateInfo>>> = LazyLock::new(|| {
    Cache::builder()
        .time_to_live(CACHE_TTL)
        .max_capacity(500)
        .build()
});

/// Debounce state for crate search - simple atomic counter
static SEARCH_QUERY_ID: AtomicU64 = AtomicU64::new(0);

/// Debounce wait time (ms) - wait for user to pause typing
/// 150ms balances responsiveness with avoiding excessive requests
const DEBOUNCE_MS: u64 = 150;

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

/// Get the sparse index URL path for a crate name
fn index_path(crate_name: &str) -> String {
    let name = crate_name.to_lowercase();
    match name.len() {
        1 => format!("1/{}", name),
        2 => format!("2/{}", name),
        3 => format!("3/{}/{}", &name[..1], name),
        _ => format!("{}/{}/{}", &name[..2], &name[2..4], name),
    }
}

/// Fetch and parse the crate index, using cache if available
async fn get_crate_index(
    http_client: &reqwest::Client,
    crate_name: &str,
) -> Option<CrateIndexData> {
    let cache_key = crate_name.to_lowercase();

    // Check cache first
    if let Some(cached) = CRATE_INDEX_CACHE.get(&cache_key).await {
        debug!("cache hit for '{}'", crate_name);
        return Some(cached);
    }

    debug!("cache miss for '{}', fetching from index", crate_name);

    // Fetch from sparse index
    let url = format!("https://index.crates.io/{}", index_path(crate_name));

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

    // Reverse to get newest first (index is oldest first)
    versions.reverse();

    debug!("parsed {} versions for '{}'", versions.len(), crate_name);

    let data = CrateIndexData {
        versions,
        features: features_map,
    };

    // Cache the result
    CRATE_INDEX_CACHE.insert(cache_key, data.clone()).await;

    Some(data)
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

/// Search crates.io for crates matching the given search string.
/// Results are cached by exact query and debounced to avoid excessive HTTP requests.
pub async fn search_crates(
    http_client: &reqwest::Client,
    search_str: &str,
) -> Option<Vec<CrateInfo>> {
    let query = search_str.to_lowercase();
    debug!("search_crates() for: {}", query);

    // Check cache first (instant return, no debounce needed)
    if let Some(cached) = CRATE_SEARCH_CACHE.get(&query).await {
        debug!(
            "search_crates: cache hit for '{}', returning {} results",
            query,
            cached.len()
        );
        return Some(cached);
    }

    // Register this query and get its ID (trailing debounce)
    let query_id = SEARCH_QUERY_ID.fetch_add(1, Ordering::SeqCst) + 1;
    debug!(
        "search_crates: query='{}', query_id={}, waiting {}ms",
        query, query_id, DEBOUNCE_MS
    );

    // Wait for user to pause typing
    tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS)).await;

    // Check if this query is still the latest (user might have typed more)
    let current_id = SEARCH_QUERY_ID.load(Ordering::SeqCst);
    debug!(
        "search_crates: query='{}' (id={}) after wait, current_id={}",
        query, query_id, current_id
    );
    if query_id != current_id {
        debug!(
            "search_crates: query '{}' (id={}) superseded by id={}, SKIPPING",
            query, query_id, current_id
        );
        return None;
    }
    debug!(
        "search_crates: query='{}' (id={}) is latest, PROCEEDING",
        query, query_id
    );

    // Double-check cache (might have been populated during debounce wait)
    if let Some(cached) = CRATE_SEARCH_CACHE.get(&query).await {
        debug!("search_crates: cache hit for '{}' after debounce", query);
        return Some(cached);
    }

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

    let url = format!(
        "https://crates.io/api/v1/crates?page=1&per_page=10&q={}",
        search_str
    );

    let resp = match http_client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to search crates.io for '{}': {}", search_str, e);
            return None;
        }
    };

    let search_response: SearchCrateResponse = match resp.json().await {
        Ok(r) => r,
        Err(e) => {
            error!(
                "Failed to parse search response for '{}': {}",
                search_str, e
            );
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

    // Cache results
    if !crates.is_empty() {
        debug!(
            "search_crates: caching {} results for '{}'",
            crates.len(),
            query
        );
        CRATE_SEARCH_CACHE.insert(query, crates.clone()).await;
    }

    debug!(
        "search_crates: returning {} results for original query",
        crates.len()
    );
    Some(crates)
}
