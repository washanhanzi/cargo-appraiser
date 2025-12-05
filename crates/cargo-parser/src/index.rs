//! CargoIndex: resolve cargo dependencies and provide O(1) lookups.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::task::Poll;

use cargo::core::compiler::{CompileKind, RustcTargetData};
use cargo::core::dependency::DepKind;
use cargo::core::resolver::{CliFeatures, ForceAllTargets, HasDevUnits};
use cargo::core::{Dependency, Package, Summary};
use cargo::ops::tree::{DisplayDepth, EdgeKind, Prefix, Target, TreeOptions};
use cargo::ops::Packages;
use cargo::sources::source::{QueryKind, Source};
use cargo::sources::SourceConfigMap;
use cargo::util::cache_lock::CacheLockMode;
use cargo::util::{OptVersionReq, VersionExt};
use cargo::GlobalContext;
use tracing::{debug, error, trace, warn};

use crate::error::CargoResolveError;
use crate::query::{dep_kind_to_table, DependencyLookupKey, ResolvedDependency};

/// Result of cargo resolution with indexed lookups.
#[derive(Debug)]
pub struct CargoIndex {
    /// The root manifest path
    root_manifest: std::path::PathBuf,
    /// Member manifest paths (for workspaces)
    member_manifests: Vec<std::path::PathBuf>,
    /// Primary index: lookup by (table, platform, name)
    index: HashMap<DependencyLookupKey, ResolvedDependency>,
}

impl CargoIndex {
    /// Resolve dependencies from the given manifest path.
    ///
    /// This runs cargo's resolution process and builds an index for fast lookups.
    #[tracing::instrument(name = "cargo_resolve", level = "trace")]
    pub fn resolve(manifest_path: &Path) -> Result<Self, CargoResolveError> {
        debug!("Entering cargo_resolve for manifest path: {:?}", manifest_path);

        let gctx = GlobalContext::default().map_err(CargoResolveError::global_context)?;

        // Create workspace
        let workspace = cargo::core::Workspace::new(manifest_path, &gctx)
            .map_err(CargoResolveError::workspace)?;

        let root_manifest = workspace.root().join("Cargo.toml");

        // Collect specs and dependencies from workspace members
        let mut specs = Vec::with_capacity(5);
        let mut member_manifests = Vec::with_capacity(5);
        let mut deps = HashSet::new();

        if let Ok(current) = workspace.current() {
            trace!("Processing current workspace package: {:?}", current.package_id());
            specs.push(current.package_id().to_spec());
            deps.extend(current.dependencies().to_vec());
        }

        for member in workspace.members() {
            trace!("Processing member package: {:?}", member.package_id());
            specs.push(member.package_id().to_spec());
            deps.extend(member.dependencies().to_vec());
            member_manifests.push(member.manifest_path().to_path_buf());
        }

        if deps.is_empty() {
            warn!("No dependencies collected from workspace members.");
        }

        // Group dependencies by source_id for efficient registry queries
        let mut source_deps: HashMap<cargo::core::SourceId, Vec<Dependency>> = HashMap::new();
        for dep in &deps {
            source_deps
                .entry(dep.source_id())
                .or_default()
                .push(dep.clone());
        }

        // Setup tree options for resolution
        let mut edge_kinds = HashSet::with_capacity(3);
        edge_kinds.insert(EdgeKind::Dep(DepKind::Normal));
        edge_kinds.insert(EdgeKind::Dep(DepKind::Development));
        edge_kinds.insert(EdgeKind::Dep(DepKind::Build));

        let opts = TreeOptions {
            cli_features: CliFeatures::new_all(true),
            packages: Packages::Default,
            target: Target::Host,
            edge_kinds,
            invert: vec![],
            pkgs_to_prune: vec![],
            prefix: Prefix::None,
            no_dedupe: false,
            duplicates: false,
            format: String::new(),
            graph_features: false,
            display_depth: DisplayDepth::MaxDisplayDepth(1),
            no_proc_macro: false,
            public: false,
        };

        let requested_kinds = CompileKind::from_requested_targets(workspace.gctx(), &[])
            .map_err(CargoResolveError::resolve)?;

        let mut target_data = RustcTargetData::new(&workspace, &[CompileKind::Host])
            .map_err(CargoResolveError::resolve)?;

        // Acquire package cache lock
        trace!("Attempting to acquire package cache lock...");
        let _guard = gctx
            .acquire_package_cache_lock(CacheLockMode::DownloadExclusive)
            .map_err(CargoResolveError::cache_lock)?;
        trace!("Package cache lock acquired.");

        // Resolve workspace
        trace!("Calling resolve_ws_with_opts with {} specs", specs.len());
        let ws_resolve = cargo::ops::resolve_ws_with_opts(
            &workspace,
            &mut target_data,
            &requested_kinds,
            &opts.cli_features,
            &specs,
            HasDevUnits::Yes,
            ForceAllTargets::No,
            false,
        )
        .map_err(CargoResolveError::resolve)?;

        trace!(
            "resolve_ws_with_opts successful. Package set contains {} packages.",
            ws_resolve.pkg_set.packages().count()
        );

        // Build package lookup map
        let packages: HashMap<String, Vec<Package>> = ws_resolve
            .pkg_set
            .packages()
            .fold(HashMap::new(), |mut acc, pkg| {
                acc.entry(pkg.name().to_string())
                    .or_default()
                    .push(pkg.clone());
                acc
            });

        // Query registries and build the index
        let index = Self::build_index(&gctx, deps, source_deps, &packages);

        trace!("Built index with {} entries", index.len());

        Ok(Self {
            root_manifest,
            member_manifests,
            index,
        })
    }

    /// Build the index by querying registries for version information.
    fn build_index(
        gctx: &GlobalContext,
        deps: HashSet<Dependency>,
        source_deps: HashMap<cargo::core::SourceId, Vec<Dependency>>,
        packages: &HashMap<String, Vec<Package>>,
    ) -> HashMap<DependencyLookupKey, ResolvedDependency> {
        let mut index = HashMap::with_capacity(deps.len());

        // For each source, query for all dependencies from that source
        for (source_id, deps_for_source) in source_deps {
            let source_config_map = match SourceConfigMap::new(gctx) {
                Ok(map) => map,
                Err(e) => {
                    error!("Failed to create source config map: {:?}", e);
                    continue;
                }
            };

            let mut source = match source_config_map.load(source_id, &HashSet::new()) {
                Ok(source) => source,
                Err(e) => {
                    error!("Failed to load source: {:?}", e);
                    continue;
                }
            };

            if let Err(e) = source.block_until_ready() {
                error!("Failed to prepare source: {:?}", e);
                continue;
            }

            // Query each dependency
            for dep in deps_for_source {
                let key = DependencyLookupKey {
                    table: dep_kind_to_table(dep.kind()),
                    platform: dep.platform().map(|p| p.to_string()),
                    name: dep.name_in_toml().to_string(),
                };

                // Find the installed package for this dependency
                let package = packages
                    .get(&dep.package_name().to_string())
                    .and_then(|pkgs| pkgs.iter().find(|p| dep.matches(p.summary())).cloned());

                // Query registry for all versions
                let mut query_dep = dep.clone();
                query_dep.set_version_req(OptVersionReq::Any);

                let dep_query = source.query_vec(&query_dep, QueryKind::Normalized);

                if let Err(e) = source.block_until_ready() {
                    error!("Failed to complete query for {}: {:?}", dep.package_name(), e);
                    continue;
                }

                match dep_query {
                    Poll::Ready(Ok(summaries)) => {
                        let mut summaries_vec: Vec<Summary> = summaries
                            .into_iter()
                            .map(|s| s.as_summary().clone())
                            .collect();

                        // Sort by version descending
                        summaries_vec.sort_by(|a, b| b.version().cmp(a.version()));

                        let available_versions: Vec<String> = summaries_vec
                            .iter()
                            .map(|s| s.version().to_string())
                            .collect();

                        // Process summaries to find latest_matched and latest
                        let (latest_matched_summary, latest_summary) = if let Some(pkg) = &package {
                            Self::find_summaries(&summaries_vec, pkg.version(), &dep)
                        } else {
                            (None, None)
                        };

                        index.insert(
                            key,
                            ResolvedDependency {
                                package,
                                available_versions,
                                latest_matched_summary,
                                latest_summary,
                            },
                        );
                    }
                    Poll::Ready(Err(e)) => {
                        error!("Failed to query dependency {}: {:?}", dep.package_name(), e);
                        // Still insert with empty data
                        index.insert(
                            key,
                            ResolvedDependency {
                                package,
                                available_versions: vec![],
                                latest_matched_summary: None,
                                latest_summary: None,
                            },
                        );
                    }
                    Poll::Pending => {
                        error!("Query for {} is pending (unexpected)", dep.package_name());
                    }
                }
            }
        }

        index
    }

    /// Find latest_matched and latest summaries from sorted summaries list.
    fn find_summaries(
        summaries: &[Summary],
        installed_version: &semver::Version,
        dep: &Dependency,
    ) -> (Option<Summary>, Option<Summary>) {
        let mut latest_summary = None;
        let mut latest_matched_summary = None;

        // Extract version requirement
        let version_req = match dep.version_req() {
            OptVersionReq::Req(req) => Some(req),
            _ => None,
        };

        for summary in summaries {
            // Find latest (considering prerelease preference)
            if latest_summary.is_none()
                && summary.version().is_prerelease() == installed_version.is_prerelease()
            {
                latest_summary = Some(summary.clone());
            }

            // Find latest that matches version requirement
            if latest_matched_summary.is_none() {
                if let Some(req) = version_req {
                    if req.matches(summary.version()) {
                        latest_matched_summary = Some(summary.clone());
                    }
                }
            }

            // Early exit if both found
            if latest_summary.is_some() && latest_matched_summary.is_some() {
                break;
            }
        }

        (latest_matched_summary, latest_summary)
    }

    /// Returns the number of dependencies in the index.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Returns true if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// O(1) lookup by composite key (table, platform, name).
    pub fn get(&self, key: &DependencyLookupKey) -> Option<&ResolvedDependency> {
        self.index.get(key)
    }

    /// Get all resolved dependencies.
    pub fn iter(&self) -> impl Iterator<Item = (&DependencyLookupKey, &ResolvedDependency)> {
        self.index.iter()
    }

    /// Get the root manifest path.
    pub fn root_manifest(&self) -> &Path {
        &self.root_manifest
    }

    /// Get member manifest paths.
    pub fn member_manifests(&self) -> &[std::path::PathBuf] {
        &self.member_manifests
    }
}
