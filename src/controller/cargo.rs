use std::{
    collections::{HashMap, HashSet},
    path::Path,
    task::Poll,
};

use cargo::{
    core::{
        compiler::{CompileKind, RustcTargetData},
        dependency::DepKind,
        resolver::{CliFeatures, ForceAllTargets, HasDevUnits},
        Package, PackageId, SourceId, Summary, Workspace,
    },
    ops::{
        tree::{EdgeKind, Prefix, Target, TreeOptions},
        Packages,
    },
    sources::source::{QueryKind, Source},
    util::{cache_lock::CacheLockMode, OptVersionReq},
    GlobalContext,
};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity};
use tracing::{error, info};

use crate::entity::{
    cargo_dependency_to_toml_key, from_resolve_error, CargoError, CargoErrorKind, Dependency,
    SymbolTree, TomlNode,
};

use super::appraiser::Ctx;

pub struct CargoResolveOutput {
    pub ctx: Ctx,
    //the hashmap key is toml_id, which is<table>:<package name>
    pub dependencies: HashMap<String, Package>,
    pub summaries: HashMap<String, Vec<Summary>>,
}

#[tracing::instrument(name = "cargo_resolve", level = "trace")]
pub async fn cargo_resolve(ctx: &Ctx) -> Result<CargoResolveOutput, CargoError> {
    info!("start resolve {}", ctx.rev);
    let path = Path::new(ctx.uri.path().as_str());
    let gctx = cargo::util::context::GlobalContext::default().map_err(CargoError::resolve_error)?;
    let workspace =
        cargo::core::Workspace::new(path, &gctx).map_err(CargoError::workspace_error)?;
    let Ok(current) = workspace.current() else {
        return Err(CargoError::workspace_error(anyhow::anyhow!(
            "virtual workspace"
        )));
    };
    let deps = current.dependencies();

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
        max_display_depth: 1,
        no_proc_macro: false,
    };

    let requested_kinds = CompileKind::from_requested_targets(workspace.gctx(), &[])
        .map_err(CargoError::resolve_error)?;
    let mut target_data = RustcTargetData::new(&workspace, &[CompileKind::Host])
        .map_err(CargoError::resolve_error)?;
    let specs = opts.packages.to_package_id_specs(&workspace).unwrap();
    // Convert Result to Option
    let ws_resolve = match cargo::ops::resolve_ws_with_opts(
        &workspace,
        &mut target_data,
        &requested_kinds,
        &opts.cli_features,
        &specs,
        HasDevUnits::Yes,
        ForceAllTargets::No,
        false,
    ) {
        Ok(ws_resolve) => ws_resolve,
        Err(e) => {
            return Err(from_resolve_error(e));
        }
    };

    let package_map: HashMap<PackageId, &Package> = ws_resolve
        .pkg_set
        .packages()
        .map(|pkg| (pkg.package_id(), pkg))
        .collect();

    let mut res = HashMap::with_capacity(deps.len());
    for dep in deps {
        //does the values().find introduce a loop inside loop?
        if let Some(pkg) = package_map.values().find(|&pkg| dep.matches(pkg.summary())) {
            let toml_key = cargo_dependency_to_toml_key(dep);
            res.insert(
                toml_key,
                (*pkg).clone(),
                // pkg.serialized(
                //     workspace.gctx().cli_unstable(),
                //     workspace.unstable_features(),
                // ),
            );
        }
    }
    info!("finished resolve inside {}", ctx.rev);
    Ok(CargoResolveOutput {
        ctx: ctx.clone(),
        dependencies: res,
        summaries: summaries_map(&gctx, &workspace),
    })
}

//TODO the current Vec<Summary> didn't include yanked
fn summaries_map(gctx: &GlobalContext, workspace: &Workspace) -> HashMap<String, Vec<Summary>> {
    let Ok(_guard) = gctx.acquire_package_cache_lock(CacheLockMode::DownloadExclusive) else {
        error!("failed to acquire package cache lock");
        return HashMap::new();
    };

    let mut res = HashMap::new();

    // Step 1: Group dependencies by SourceId
    let mut source_deps: HashMap<SourceId, Vec<_>> = HashMap::new();

    for member in workspace.members() {
        for dep in member.dependencies() {
            source_deps.entry(dep.source_id()).or_default().push(dep);
        }
    }

    // Step 2: Process each source
    for (source_id, package_names) in source_deps {
        let mut source = source_id.load(gctx, &HashSet::new()).unwrap();
        source.invalidate_cache();
        source.block_until_ready().unwrap();
        let mut summaries = Vec::new();
        for dep in &package_names {
            let mut any_dep = (*dep).clone();
            any_dep.set_version_req(OptVersionReq::Any);
            let poll = source.query_vec(&any_dep, QueryKind::Normalized);
            summaries.push(poll);
        }
        source.block_until_ready().unwrap();
        for summary in summaries {
            match summary {
                Poll::Ready(summaries) => {
                    let summaries = summaries.unwrap();
                    let mut sums = Vec::new();
                    let mut package_name = "".to_string();
                    //map summaries to d.as_summary()
                    for summary in &summaries {
                        let name = summary.as_summary().name().to_string();
                        package_name = name;
                        sums.push(summary.as_summary().clone());
                    }
                    res.insert(package_name, sums);
                }
                Poll::Pending => unreachable!(),
            }
        }
    }
    res
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
        deps: &[&Dependency],
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
                            let req = d.unresolved.as_ref()?.version_req().to_string();
                            let error_msg = self.to_string();

                            // Check if the requirement in the error message matches the dependency's requirement
                            if error_msg.contains(&format!("`{} = \"{}\"", d.name, req)) {
                                let version = d.version.as_ref()?.id.as_str();
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
                    let Some(unresolved) = d.unresolved.as_ref() else {
                        continue;
                    };
                    let Some(features) = &d.features else {
                        continue;
                    };
                    let mut feature_map = HashMap::with_capacity(features.len());
                    for f in features {
                        feature_map.insert(f.value.to_string(), f.id.to_string());
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
