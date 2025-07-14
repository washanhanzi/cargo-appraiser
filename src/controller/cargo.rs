use std::{
    collections::{HashMap, HashSet},
    task::Poll,
};

use cargo::{
    core::{
        compiler::{CompileKind, RustcTargetData},
        dependency::DepKind,
        resolver::{CliFeatures, ForceAllTargets, HasDevUnits},
        Dependency, Package, PackageIdSpec, SourceId, Summary,
    },
    util::{OptVersionReq, VersionExt},
    ops::{
        tree::{DisplayDepth, EdgeKind, Prefix, Target, TreeOptions},
        Packages,
    },
    sources::{
        source::{QueryKind, Source},
        SourceConfigMap,
    },
    util::cache_lock::CacheLockMode,
    GlobalContext,
};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity};
use tracing::{debug, error, trace, warn};
use semver;

use crate::entity::{
    from_resolve_error, CanonicalUri, CargoError, CargoErrorKind, Dependency as EntityDependency,
    SymbolTree, TomlNode,
};

use super::appraiser::Ctx;

#[derive(Debug)]
pub struct CargoResolveOutput {
    pub ctx: Ctx,
    pub root_manifest_uri: CanonicalUri,
    pub specs: Vec<PackageIdSpec>,
    pub member_manifest_uris: Vec<CanonicalUri>,
    //toml_name -> Dependency
    pub dependencies: HashMap<String, Vec<DependencyWithId>>,
    //package_name -> Vec<Package>
    pub packages: HashMap<String, Vec<Package>>,
    //DependencyId -> Vec<String> (sorted version strings for completion/hover)
    pub available_versions: HashMap<u32, Vec<String>>,
    //DependencyId -> ProcessedSummaries (pre-processed summaries)
    pub processed_summaries: HashMap<u32, ProcessedSummaries>,
}

#[derive(Debug, Clone)]
pub struct DependencyWithId(pub u32, pub Dependency);

#[derive(Debug, Clone)]
pub struct ProcessedSummaries {
    pub matched_summary: Option<Summary>,
    pub latest_summary: Option<Summary>,
    pub latest_matched_summary: Option<Summary>,
}

fn process_summaries(
    mut summaries: Vec<Summary>,
    installed_version: &semver::Version,
    version_req: &semver::VersionReq,
) -> ProcessedSummaries {
    // Sort summaries by version (descending)
    summaries.sort_by(|a, b| b.version().cmp(a.version()));
    
    let mut matched_summary = None;
    let mut latest_summary = None;
    let mut latest_matched_summary = None;
    
    for summary in summaries {
        // Early exit if all summaries are found
        if matched_summary.is_some() && latest_summary.is_some() && latest_matched_summary.is_some() {
            break;
        }
        
        // Find the exact matched summary (installed version)
        if installed_version == summary.version() {
            matched_summary = Some(summary.clone());
        }
        
        // Find the latest summary considering prerelease preference
        if latest_summary.is_none() 
            && summary.version().is_prerelease() == installed_version.is_prerelease() {
            latest_summary = Some(summary.clone());
        }
        
        // Find the latest summary that satisfies the version requirement
        if latest_matched_summary.is_none() && version_req.matches(summary.version()) {
            latest_matched_summary = Some(summary.clone());
        }
    }
    
    ProcessedSummaries {
        matched_summary,
        latest_summary,
        latest_matched_summary,
    }
}

#[tracing::instrument(name = "cargo_resolve", level = "trace")]
pub async fn cargo_resolve(ctx: &Ctx) -> Result<CargoResolveOutput, CargoError> {
    debug!(
        "Entering cargo_resolve for manifest path: {:?}",
        ctx.uri.path()
    );
    let gctx = GlobalContext::default().unwrap();
    let Ok(path) = ctx.uri.to_path_buf() else {
        error!("Failed to convert URI to path: {:?}", ctx.uri);
        return Err(CargoError::resolve_error(anyhow::anyhow!(
            "Failed to convert URI to path"
        )));
    };

    // Create workspace and ensure it's properly configured
    let workspace =
        cargo::core::Workspace::new(&path, &gctx).map_err(CargoError::workspace_error)?;

    let path = workspace.root().join("Cargo.toml");
    let root_manifest_uri = CanonicalUri::try_from_path(&path)
        .expect("Failed to convert root manifest path to canonical URI");

    //Dependency is a what cargo.toml requested
    //workspace resolve specs
    let mut specs = Vec::with_capacity(5);
    let mut member_manifest_uris = Vec::with_capacity(5);

    let mut deps = HashSet::new();

    if let Ok(current) = workspace.current() {
        trace!(
            "Processing current workspace package: {:?}",
            current.package_id()
        );
        specs.push(current.package_id().to_spec());
        deps.extend(current.dependencies().to_vec());
        trace!(
            "Current package: specs_count={}, deps_count={}",
            specs.len(),
            deps.len()
        );
    }

    for member in workspace.members() {
        trace!("Processing member package: {:?}", member.package_id());
        specs.push(member.package_id().to_spec());
        deps.extend(member.dependencies().to_vec());
        let Ok(manifest_path) = CanonicalUri::try_from_path(member.manifest_path()) else {
            error!(
                "Failed to convert member manifest path to canonical URI, member: {:?}",
                member.manifest_path()
            );
            continue;
        };
        member_manifest_uris.push(manifest_path);
        trace!(
            "After member {:?}: specs_count={}, deps_count={}",
            member.package_id(),
            specs.len(),
            deps.len()
        );
    }

    if deps.is_empty() {
        warn!("No dependencies collected from workspace members.");
    }

    let mut deps_map = HashMap::new();
    let mut source_ids = HashMap::new();

    for (id_counter, dep) in (0_u32..).zip(deps.into_iter()) {
        let toml_name = dep.name_in_toml().to_string();
        let source_id = dep.source_id();
        let dependency_with_id = DependencyWithId(id_counter, dep);

        deps_map
            .entry(toml_name)
            .or_insert_with(Vec::new)
            .push(dependency_with_id.clone());

        source_ids
            .entry(source_id)
            .or_insert_with(Vec::new)
            .push(dependency_with_id);
    }

    trace!(
        "Initial dependencies map created with {} entries. Keys: {:?}",
        deps_map.len(),
        deps_map.keys()
    );

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
        format: "".to_string(),
        graph_features: false,
        display_depth: DisplayDepth::MaxDisplayDepth(1),
        no_proc_macro: false,
    };

    let requested_kinds = CompileKind::from_requested_targets(workspace.gctx(), &[])
        .map_err(CargoError::resolve_error)?;

    let mut target_data = RustcTargetData::new(&workspace, &[CompileKind::Host])
        .map_err(CargoError::resolve_error)?;

    // Acquire package cache lock to ensure we have access to all registry data
    trace!("Attempting to acquire package cache lock...");
    let _guard = gctx
        .acquire_package_cache_lock(CacheLockMode::DownloadExclusive)
        .map_err(|e| {
            CargoError::resolve_error(anyhow::anyhow!(
                "Failed to acquire package cache lock: {}",
                e
            ))
        })?;
    trace!("Package cache lock acquired.");

    // Convert Result to Option
    trace!(
        "Calling resolve_ws_with_opts with {} specs: {:?}",
        specs.len(),
        specs
    );

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
    .map_err(from_resolve_error)?;

    trace!(
        "resolve_ws_with_opts successful. Package set contains {} packages.",
        ws_resolve.pkg_set.packages().count()
    );

    if ws_resolve.pkg_set.packages().count() == 0 {
        warn!("resolve_ws_with_opts returned an empty package set.");
    }

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

    let (available_versions, processed_summaries) = process_summaries_map(&gctx, source_ids, &packages);

    trace!(
        "Constructed packages map with {} entries. Keys: {:?}",
        packages.len(),
        packages.keys()
    );
    trace!(
        "Constructed summaries map with {} entries. Keys: {:?}",
        available_versions.len(),
        available_versions.keys()
    );

    Ok(CargoResolveOutput {
        ctx: ctx.clone(),
        root_manifest_uri,
        member_manifest_uris,
        specs,
        dependencies: deps_map,
        packages,
        available_versions,
        processed_summaries,
    })
}

//TODO the current Vec<Summary> didn't include yanked
fn process_summaries_map(
    gctx: &GlobalContext,
    source_ids: HashMap<SourceId, Vec<DependencyWithId>>,
    packages: &HashMap<String, Vec<Package>>,
) -> (HashMap<u32, Vec<String>>, HashMap<u32, ProcessedSummaries>) {
    let mut available_versions = HashMap::new();
    let mut processed_summaries = HashMap::new();

    // For each SourceId, create and configure a source
    for (source_id, deps) in source_ids {
        let source_config_map = match SourceConfigMap::new(gctx) {
            Ok(map) => map,
            Err(e) => {
                error!("failed to create source config map: {:?}", e);
                continue;
            }
        };

        // This will respect source replacement settings from .cargo/config.toml
        let mut source = match source_config_map.load(source_id, &HashSet::new()) {
            Ok(source) => source,
            Err(e) => {
                error!("failed to load source: {:?}", e);
                continue;
            }
        };

        // Prepare the source - this may download indices, etc.
        if let Err(e) = source.block_until_ready() {
            error!("failed to prepare source: {:?}", e);
            continue;
        }

        // For each dependency, query the registry
        for dep in deps {
            // Set the version requirement to Any to get all available versions
            let mut query_dep = dep.1.clone();
            query_dep.set_version_req(OptVersionReq::Any);

            // Query for the package using the dependency itself with QueryKind::Normalized
            let dep_query = source.query_vec(&query_dep, QueryKind::Normalized);

            // Ensure the query completes
            if let Err(e) = source.block_until_ready() {
                error!(
                    "failed to complete query for {}: {:?}",
                    dep.1.package_name(),
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

                    // Sort summaries by version (descending)
                    summaries_vec.sort_by(|a, b| b.version().cmp(a.version()));

                    // Extract version strings for completion/hover
                    let versions: Vec<String> = summaries_vec
                        .iter()
                        .map(|s| s.version().to_string())
                        .collect();
                    available_versions.insert(dep.0, versions);

                    // Find the resolved package to get installed version and version requirement
                    if let Some(pkgs) = packages.get(&dep.1.package_name().to_string()) {
                        for pkg in pkgs {
                            if dep.1.matches(pkg.summary()) {
                                let installed_version = pkg.version();
                                
                                // Extract VersionReq from OptVersionReq
                                if let OptVersionReq::Req(req_version) = dep.1.version_req() {
                                    let processed = process_summaries(summaries_vec, installed_version, req_version);
                                    processed_summaries.insert(dep.0, processed);
                                }
                                break;
                            }
                        }
                    }
                }
                Poll::Ready(Err(e)) => {
                    error!(
                        "failed to query dependency {}: {:?}",
                        dep.1.package_name(),
                        e
                    );
                }
                Poll::Pending => {
                    // This shouldn't happen, but just in case
                    error!("query for {} is pending", dep.1.package_name());
                    unreachable!()
                }
            }
        }
    }

    (available_versions, processed_summaries)
}

pub fn resolve_package_with_default_source(
    package: &str,
    version: Option<&str>,
) -> Option<Vec<Summary>> {
    let gctx = cargo::util::context::GlobalContext::default().ok()?;
    let source_id = cargo::core::SourceId::crates_io(&gctx).unwrap();
    let dep = cargo::core::Dependency::parse(package, version, source_id).ok()?;
    let mut source = source_id.load(&gctx, &HashSet::new()).unwrap();
    let Ok(_guard) = gctx.acquire_package_cache_lock(CacheLockMode::DownloadExclusive) else {
        error!("failed to acquire package cache lock");
        return None;
    };
    let summary = source.query_vec(&dep, QueryKind::Normalized);
    source.block_until_ready().unwrap();
    match summary {
        Poll::Ready(summaries) => {
            let summaries = summaries.unwrap();
            Some(summaries.iter().map(|s| s.as_summary().clone()).collect())
        }
        Poll::Pending => None,
    }
}

impl CargoError {
    pub fn diagnostic(
        self,
        keys: &[&TomlNode],
        deps: &[&EntityDependency],
        tree: &SymbolTree,
    ) -> Option<Vec<(String, Diagnostic)>> {
        match &self.kind {
            CargoErrorKind::NoMatchingPackage(_) => Some(
                keys.iter()
                    .map(|key| {
                        (
                            key.id.to_string(),
                            Diagnostic {
                                range: key.range,
                                severity: Some(DiagnosticSeverity::ERROR),
                                code: None,
                                code_description: None,
                                source: Some("cargo".to_string()),
                                message: self.to_string(),
                                related_information: None,
                                tags: None,
                                data: None,
                            },
                        )
                    })
                    .collect(),
            ),
            CargoErrorKind::VersionNotFound(_, _) => {
                Some(
                    deps.iter()
                        .filter_map(|d| {
                            let req = d.requested.as_ref()?.version_req().to_string();
                            let error_msg = self.to_string();

                            // Check if the requirement in the error message matches the dependency's requirement
                            if error_msg.contains(&format!("`{} = \"{}\"", d.name, req)) {
                                let version = d.version.as_ref()?.id();
                                let range = tree.entries.get(version)?.range;
                                Some((
                                    version.to_string(),
                                    Diagnostic {
                                        range,
                                        severity: Some(DiagnosticSeverity::ERROR),
                                        code: None,
                                        code_description: None,
                                        source: Some("cargo".to_string()),
                                        message: error_msg,
                                        related_information: None,
                                        tags: None,
                                        data: None,
                                    },
                                ))
                            } else {
                                None
                            }
                        })
                        .collect(),
                )
            }
            CargoErrorKind::FailedToSelectVersion(_) => {
                //TODO multiple deps
                //check features
                let mut diags = Vec::with_capacity(deps.len());
                for d in deps {
                    let Some(unresolved) = d.requested.as_ref() else {
                        continue;
                    };
                    let Some(features) = &d.features else {
                        continue;
                    };
                    let mut feature_map = HashMap::with_capacity(features.len());
                    for f in features {
                        feature_map.insert(f.value().to_string(), f.id().to_string());
                    }
                    let version = unresolved.version_req().to_string();
                    let summaries =
                        resolve_package_with_default_source(d.package_name(), Some(&version))
                            .unwrap();
                    for summary in &summaries {
                        if !feature_map.is_empty() {
                            for f in summary.features().keys() {
                                feature_map.remove(f.to_string().as_str());
                            }
                        }
                    }
                    for (k, v) in feature_map {
                        diags.push((
                            v.to_string(),
                            Diagnostic {
                                range: tree.entries.get(v.as_str())?.range,
                                severity: Some(DiagnosticSeverity::ERROR),
                                code: None,
                                code_description: None,
                                source: Some("cargo".to_string()),
                                message: format!("unknown feature `{}`", k),
                                related_information: None,
                                tags: None,
                                data: None,
                            },
                        ));
                    }
                }
                if !diags.is_empty() {
                    return Some(diags);
                }
                None
            }
            _ => None,
        }
    }
}
