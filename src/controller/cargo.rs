use std::{
    collections::{HashMap, HashSet},
    task::Poll,
};

use cargo::{
    core::{
        compiler::{CompileKind, RustcTargetData},
        dependency::DepKind,
        package::SerializedPackage,
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
use tracing::{error, info};

use crate::entity::{cargo_dependency_to_toml_key, CargoError};

use super::appraiser::Ctx;

pub struct CargoResolveOutput {
    pub ctx: Ctx,
    //the hashmap key is toml_id, which is<table>:<package name>
    pub dependencies: HashMap<String, SerializedPackage>,
    pub summaries: HashMap<String, Vec<Summary>>,
}

#[tracing::instrument(name = "cargo_resolve")]
pub async fn cargo_resolve(ctx: &Ctx) -> Result<CargoResolveOutput, CargoError> {
    let Ok(path) = ctx.uri.to_file_path() else {
        return Err(CargoError::other(anyhow::anyhow!("uri is not a file")));
    };
    let Ok(gctx) = cargo::util::context::GlobalContext::default() else {
        return Err(CargoError::other(anyhow::anyhow!("failed to create gctx")));
    };
    let workspace = match cargo::core::Workspace::new(path.as_path(), &gctx) {
        Ok(workspace) => workspace,
        Err(e) => {
            //TOML parse error at line 14, column 1
            //    |
            // 14 | 1serde = { version = "1", features = ["derive"] }
            //    | ^^^^^^
            // invalid character `1` in package name: `1serde`, the name cannot start with a digit
            error!("failed to create workspace: {}", e);
            return Err(CargoError::other(anyhow::anyhow!(
                "failed to create workspace: {}",
                e
            )));
        }
    };
    //TODO virtual workspace
    let Ok(current) = workspace.current() else {
        return Err(CargoError::other(anyhow::anyhow!("virtual workspace")));
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

    let requested_kinds = CompileKind::from_requested_targets(workspace.gctx(), &[]).unwrap();
    let mut target_data = RustcTargetData::new(&workspace, &[CompileKind::Host]).unwrap();
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
            // 1. no matching package named `aaxum-extra` found
            //
            // no matching package named `aserde` found
            // location searched: registry `crates-io`
            // required by package `hello-rust v0.1.0 (/Users/jingyu/tmp/hello-rust)`
            //
            // search keys for matching package name
            //
            // 2. version not found
            //
            // failed to select a version for the requirement `serde = "^2"`
            // candidate versions found which didn't match: 1.0.210, 1.0.209, 1.0.208, ...
            // location searched: crates.io index
            // required by package `hello-rust v0.1.0 (/Users/jingyu/tmp/hello-rust)`
            // if you are looking for the prerelease package it needs to be specified explicitly
            // serde = { version = "1.0.172-alpha.0" }
            //
            // 3. feature not found
            //
            // failed to select a version for `serde`.
            // ... required by package `hello-rust v0.1.0 (/Users/jingyu/tmp/hello-rust)`
            // versions that meet the requirements `^1` (locked to 1.0.210) are: 1.0.210
            //
            // the package `hello-rust` depends on `serde`, with features: `de1rive` but `serde` does not have these features.
            //
            //
            // failed to select a version for `serde` which could resolve this conflict
            //
            // 4. cyclic
            //
            // cyclic package dependency: package `A v0.0.0 (registry `https://example.com/`)` depends on itself. Cycle:
            // package `A v0.0.0 (registry `https://example.com/`)`
            //     ... which satisfies dependency `A = \"*\"` of package `C v0.0.0 (registry `https://example.com/`)`
            //     ... which satisfies dependency `C = \"*\"` of package `A v0.0.0 (registry `https://example.com/`)`\
            //
            // send err to diagnostic task
            let err: CargoError = e.into();
            return Err(err);
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
                pkg.serialized(
                    workspace.gctx().cli_unstable(),
                    workspace.unstable_features(),
                ),
            );
        }
    }

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
