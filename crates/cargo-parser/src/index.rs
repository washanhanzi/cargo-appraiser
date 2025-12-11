//! CargoIndex: resolve cargo dependencies and provide O(1) lookups.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::task::Poll;

use cargo::core::compiler::{CompileKind, RustcTargetData};
use cargo::core::dependency::DepKind;
use cargo::core::resolver::{CliFeatures, ForceAllTargets, HasDevUnits};
use cargo::core::{Dependency, Package, SourceId, Summary};
use cargo::ops::tree::{DisplayDepth, EdgeKind, Prefix, Target, TreeOptions};
use cargo::ops::Packages;
use cargo::sources::source::{QueryKind, Source};
use cargo::sources::SourceConfigMap;
use cargo::util::cache_lock::CacheLockMode;
use cargo::util::{OptVersionReq, VersionExt};
use cargo::GlobalContext;
use tracing::{error, trace, warn};

use crate::entity::{ResolvedPackage, SourceKind, VersionSummary};
use crate::error::CargoResolveError;
use crate::query::{dep_kind_to_table, DependencyLookupKey, ResolvedDependency};

/// Result of cargo resolution with indexed lookups.
#[derive(Debug)]
pub struct CargoIndex {
    /// The root manifest path
    root_manifest: std::path::PathBuf,
    /// Member manifest paths (for workspaces)
    member_manifests: Vec<std::path::PathBuf>,
    /// Member packages (for workspaces) - only populated when using resolve_direct
    member_packages: Vec<Package>,
    /// Workspace members (name and manifest path) - populated from either resolve or resolve_direct
    members: Vec<crate::entity::WorkspaceMember>,
    /// Primary index: lookup by (table, platform, name)
    index: HashMap<DependencyLookupKey, ResolvedDependency>,
}

impl CargoIndex {
    /// Resolve dependencies from the given manifest path using a subprocess.
    ///
    /// This spawns the current executable with the "resolve" subcommand to run
    /// cargo resolution in an isolated process. This prevents memory leaks from
    /// cargo's InternedString cache accumulating in the long-lived LSP process.
    #[tracing::instrument(name = "cargo_resolve", level = "trace")]
    pub fn resolve(manifest_path: &Path) -> Result<Self, CargoResolveError> {
        use std::process::Command;

        let current_exe = std::env::current_exe().map_err(|e| {
            CargoResolveError::resolve(anyhow::anyhow!("Failed to get current exe: {}", e))
        })?;

        let output = Command::new(&current_exe)
            .arg("resolve")
            .arg(manifest_path)
            .output()
            .map_err(|e| {
                CargoResolveError::resolve(anyhow::anyhow!("Failed to spawn worker: {}", e))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CargoResolveError::resolve(anyhow::anyhow!("{}", stderr)));
        }

        let serializable: crate::entity::SerializableCargoIndex =
            serde_json::from_slice(&output.stdout).map_err(|e| {
                CargoResolveError::resolve(anyhow::anyhow!("Failed to parse worker output: {}", e))
            })?;

        Ok(Self::from_serializable(serializable))
    }

    /// Resolve dependencies directly (without subprocess).
    ///
    /// This is used by the worker subprocess. Do not call this from the LSP server
    /// as it will cause memory leaks from cargo's InternedString cache.
    #[tracing::instrument(name = "cargo_resolve_direct", level = "trace")]
    pub fn resolve_direct(manifest_path: &Path) -> Result<Self, CargoResolveError> {
        trace!(
            "Entering cargo_resolve for manifest path: {:?}",
            manifest_path
        );

        let gctx = GlobalContext::default().map_err(CargoResolveError::global_context)?;

        // Create workspace
        let workspace = cargo::core::Workspace::new(manifest_path, &gctx)
            .map_err(CargoResolveError::workspace)?;

        let root_manifest = workspace.root().join("Cargo.toml");

        // Collect specs and dependencies from workspace members
        let mut specs = Vec::with_capacity(5);
        let mut member_manifests = Vec::with_capacity(5);
        let mut member_packages = Vec::with_capacity(5);
        let mut deps = HashSet::new();

        if let Ok(current) = workspace.current() {
            trace!(
                "Processing current workspace package: {:?}",
                current.package_id()
            );
            specs.push(current.package_id().to_spec());
            deps.extend(current.dependencies().to_vec());
        }

        for member in workspace.members() {
            trace!("Processing member package: {:?}", member.package_id());
            specs.push(member.package_id().to_spec());
            deps.extend(member.dependencies().to_vec());
            member_manifests.push(member.manifest_path().to_path_buf());
            member_packages.push(member.clone());
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

        // Acquire package cache lock with retry to avoid deadlock with rust-analyzer
        const MAX_RETRIES: u32 = 6;
        const RETRY_INTERVAL_SECS: u64 = 5;

        let mut attempts = 0;
        let _guard = loop {
            attempts += 1;
            trace!(
                "Attempting to acquire package cache lock (attempt {}/{})",
                attempts,
                MAX_RETRIES
            );

            match gctx.acquire_package_cache_lock(CacheLockMode::DownloadExclusive) {
                Ok(guard) => {
                    trace!("Package cache lock acquired on attempt {}", attempts);
                    break guard;
                }
                Err(e) => {
                    if attempts >= MAX_RETRIES {
                        return Err(CargoResolveError::cache_lock(e));
                    }
                    warn!(
                        "Failed to acquire package cache lock (attempt {}): {}, retrying in {}s",
                        attempts, e, RETRY_INTERVAL_SECS
                    );
                    std::thread::sleep(std::time::Duration::from_secs(RETRY_INTERVAL_SECS));
                }
            }
        };

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
        let packages: HashMap<String, Vec<Package>> =
            ws_resolve
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

        // Build members list from member_packages
        let members: Vec<crate::entity::WorkspaceMember> = member_packages
            .iter()
            .map(|p| crate::entity::WorkspaceMember {
                name: p.name().to_string(),
                manifest_path: p.manifest_path().to_path_buf(),
            })
            .collect();

        Ok(Self {
            root_manifest,
            member_manifests,
            member_packages,
            members,
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
                    name: dep.package_name().to_string(),
                };

                // Find the installed package for this dependency
                let cargo_package = packages
                    .get(&dep.package_name().to_string())
                    .and_then(|pkgs| pkgs.iter().find(|p| dep.matches(p.summary())).cloned());

                // Convert cargo Package to our ResolvedPackage
                let package = cargo_package
                    .as_ref()
                    .map(|pkg| Self::package_to_resolved(pkg));

                // Query registry for all versions
                let mut query_dep = dep.clone();
                query_dep.set_version_req(OptVersionReq::Any);

                let dep_query = source.query_vec(&query_dep, QueryKind::Normalized);

                if let Err(e) = source.block_until_ready() {
                    error!(
                        "Failed to complete query for {}: {:?}",
                        dep.package_name(),
                        e
                    );
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
                        let (latest_matched_version, latest_version) =
                            if let Some(pkg) = &cargo_package {
                                Self::find_summaries(&summaries_vec, pkg.version(), &dep)
                            } else {
                                (None, None)
                            };

                        index.insert(
                            key,
                            ResolvedDependency {
                                package,
                                available_versions,
                                latest_matched_version,
                                latest_version,
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
                                latest_matched_version: None,
                                latest_version: None,
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

    /// Convert a cargo Package to our serializable ResolvedPackage.
    fn package_to_resolved(pkg: &Package) -> ResolvedPackage {
        let source_id = pkg.package_id().source_id();
        let source = Self::source_id_to_kind(&source_id);

        let features = pkg
            .manifest()
            .summary()
            .features()
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().map(|fv| fv.to_string()).collect()))
            .collect();

        ResolvedPackage {
            version: pkg.version().to_string(),
            source,
            features,
        }
    }

    /// Convert a cargo SourceId to our serializable SourceKind.
    fn source_id_to_kind(source_id: &SourceId) -> SourceKind {
        use cargo::core::SourceKind as CargoSourceKind;

        match source_id.kind() {
            CargoSourceKind::Path => SourceKind::Path,
            CargoSourceKind::Directory => SourceKind::Directory,
            CargoSourceKind::Git(_) => {
                let reference = source_id
                    .git_reference()
                    .and_then(|r| r.pretty_ref(false).map(|s| s.to_string()));
                let full_commit = source_id.precise_git_fragment().map(|s| s.to_string());
                let short_commit = full_commit.as_ref().map(|c| {
                    if c.len() >= 7 {
                        c[..7].to_string()
                    } else {
                        c.clone()
                    }
                });

                SourceKind::Git {
                    reference,
                    short_commit,
                    full_commit,
                }
            }
            CargoSourceKind::Registry | CargoSourceKind::SparseRegistry => {
                if source_id.is_crates_io() {
                    SourceKind::CratesIo
                } else {
                    SourceKind::Registry {
                        name: source_id.display_registry_name(),
                    }
                }
            }
            CargoSourceKind::LocalRegistry => SourceKind::Registry {
                name: "local".to_string(),
            },
        }
    }

    /// Find latest_matched and latest version summaries from sorted summaries list.
    fn find_summaries(
        summaries: &[Summary],
        installed_version: &semver::Version,
        dep: &Dependency,
    ) -> (Option<VersionSummary>, Option<VersionSummary>) {
        let mut latest_version = None;
        let mut latest_matched_version = None;

        // Extract version requirement
        let version_req = match dep.version_req() {
            OptVersionReq::Req(req) => Some(req),
            _ => None,
        };

        for summary in summaries {
            // Find latest (considering prerelease preference)
            if latest_version.is_none()
                && summary.version().is_prerelease() == installed_version.is_prerelease()
            {
                latest_version = Some(VersionSummary::new(summary.version().to_string()));
            }

            // Find latest that matches version requirement
            if latest_matched_version.is_none() {
                if let Some(req) = version_req {
                    if req.matches(summary.version()) {
                        latest_matched_version =
                            Some(VersionSummary::new(summary.version().to_string()));
                    }
                }
            }

            // Early exit if both found
            if latest_version.is_some() && latest_matched_version.is_some() {
                break;
            }
        }

        (latest_matched_version, latest_version)
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

    /// Get member packages.
    pub fn member_packages(&self) -> &[Package] {
        &self.member_packages
    }

    /// Convert to serializable format for IPC.
    pub fn to_serializable(&self) -> crate::entity::SerializableCargoIndex {
        crate::entity::SerializableCargoIndex {
            root_manifest: self.root_manifest.clone(),
            member_manifests: self.member_manifests.clone(),
            members: self
                .member_packages
                .iter()
                .map(|p| crate::entity::WorkspaceMember {
                    name: p.name().to_string(),
                    manifest_path: p.manifest_path().to_path_buf(),
                })
                .collect(),
            dependencies: self.index.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        }
    }

    /// Construct from serializable format (received from worker subprocess).
    ///
    /// Note: This does not restore member_packages as cargo Package types.
    /// The members are preserved for hover functionality.
    fn from_serializable(s: crate::entity::SerializableCargoIndex) -> Self {
        Self {
            root_manifest: s.root_manifest,
            member_manifests: s.member_manifests,
            member_packages: Vec::new(), // Cannot reconstruct cargo Package from serialized data
            members: s.members,
            index: s.dependencies.into_iter().collect(),
        }
    }

    /// Get workspace members (name and manifest path).
    pub fn members(&self) -> &[crate::entity::WorkspaceMember] {
        &self.members
    }

    /// Find a resolved dependency by package name, ignoring table type.
    ///
    /// This is useful for workspace dependencies where the table in toml-parser
    /// (always Dependencies) may not match the table in cargo resolution
    /// (depends on how member packages use the dependency).
    ///
    /// Returns the first match found (prefers Dependencies > DevDependencies > BuildDependencies).
    pub fn find_by_name(&self, name: &str, platform: Option<&str>) -> Option<&ResolvedDependency> {
        use crate::query::DependencyTable;

        // Try each table type in order of preference
        let tables = [
            DependencyTable::Dependencies,
            DependencyTable::DevDependencies,
            DependencyTable::BuildDependencies,
        ];

        for table in tables {
            let key = DependencyLookupKey::new(table, platform.map(String::from), name.to_string());
            if let Some(resolved) = self.index.get(&key) {
                return Some(resolved);
            }
        }

        None
    }
}
