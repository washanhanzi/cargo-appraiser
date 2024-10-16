use std::{
    collections::{HashMap, HashSet},
    path::Path,
    task::Poll,
};

use cargo::{
    core::{
        compiler::{CompileKind, RustcTargetData},
        dependency::DepKind,
        package::SerializedPackage,
        resolver::{CliFeatures, ForceAllTargets, HasDevUnits},
        Package, PackageId, SourceId, Summary,
    },
    ops::{
        tree::{EdgeKind, Prefix, Target, TreeOptions},
        Packages,
    },
    sources::source::{QueryKind, Source},
    util::{cache_lock::CacheLockMode, OptVersionReq},
};

use crate::entity::cargo_dependency_to_toml_key;

use super::appraiser::Ctx;

pub struct CargoResolveOutput {
    pub ctx: Ctx,
    //the hashmap key is toml_id, which is<table>:<package name>
    pub dependencies: HashMap<String, SerializedPackage>,
    pub summaries: HashMap<String, Vec<Summary>>,
}

pub async fn parse_cargo_output(ctx: &Ctx) -> Option<CargoResolveOutput> {
    let Ok(path) = ctx.uri.to_file_path() else {
        return None;
    };
    let Ok(gctx) = cargo::util::context::GlobalContext::default() else {
        return None;
    };
    let Ok(workspace) = cargo::core::Workspace::new(path.as_path(), &gctx) else {
        return None;
    };
    let Ok(current) = workspace.current() else {
        return None;
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
    .ok()?;

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

    Some(CargoResolveOutput {
        ctx: ctx.clone(),
        dependencies: res,
        //TODO maybe reuse gctx
        summaries: summaries_map(path.as_path()),
    })
}

//TODO the current Vec<Summary> didn't include yanked
fn summaries_map(path: &Path) -> HashMap<String, Vec<Summary>> {
    let gctx = cargo::util::context::GlobalContext::default().unwrap();
    let workspace = cargo::core::Workspace::new(path, &gctx).unwrap();

    //if it's error, it's a virtual workspace
    let current = workspace.current().unwrap();

    let _guard = gctx
        .acquire_package_cache_lock(CacheLockMode::DownloadExclusive)
        .unwrap();

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
        let mut source = source_id.load(&gctx, &HashSet::new()).unwrap();
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
