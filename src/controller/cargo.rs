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
}

pub async fn parse_cargo_output(ctx: &Ctx) -> CargoResolveOutput {
    let path = Path::new(&ctx.path);
    let gctx = cargo::util::context::GlobalContext::default().unwrap();
    let workspace = cargo::core::Workspace::new(path, &gctx).unwrap();
    let current = workspace.current().unwrap();
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
    .unwrap();

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
                pkg.serialized(gctx.cli_unstable(), workspace.unstable_features()),
            );
        }
    }

    CargoResolveOutput {
        ctx: ctx.clone(),
        dependencies: res,
    }
}

pub fn get_latest_version(path: &str) -> HashMap<String, Vec<Summary>> {
    let path = Path::new(path);
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

mod tests {
    use std::{
        collections::{HashMap, HashSet},
        future::ready,
        mem,
        path::Path,
        task::{ready, Poll},
        vec,
    };

    use cargo::{
        core::{
            compiler::{CompileKind, RustcTargetData},
            dependency::DepKind,
            gc,
            registry::PackageRegistry,
            resolver::{CliFeatures, ForceAllTargets, HasDevUnits},
            Package, PackageId, SourceId,
        },
        ops::{
            print,
            tree::{EdgeKind, Node, Prefix, Target, TreeOptions},
            Packages,
        },
        sources::{
            registry::{MaybeIndexSummary, RegistryData, RegistryIndex},
            source::{QueryKind, Source},
            IndexSummary, RegistrySource,
        },
        util::{cache_lock::CacheLockMode, graph, interning::InternedString, OptVersionReq},
    };

    #[test]
    fn test_parse_line() {
        let path = Path::new("/Users/jingyu/Github/rust-analyzer/Cargo.toml");
        let gctx = cargo::util::context::GlobalContext::default().unwrap();
        let workspace = cargo::core::Workspace::new(path, &gctx).unwrap();
        //if it's error, it's a virtual workspace
        let current = workspace.current().unwrap();
        println!("current {:?}", current.name());
        let members = workspace.members();
        for member in members {
            println!("member {:?}", member.name());
            for dep in member.dependencies() {
                // println!("{:?}", dep.name_in_toml());
                // println!("{:?}", dep.version_req());
                // println!("{:?}", dep.source_id());
                // println!("{:?}", dep.registry_id());
            }
        }

        let mut edge_kinds = HashSet::with_capacity(3);
        edge_kinds.insert(EdgeKind::Dep(DepKind::Normal));
        edge_kinds.insert(EdgeKind::Dep(DepKind::Development));
        edge_kinds.insert(EdgeKind::Dep(DepKind::Build));

        let opts = TreeOptions {
            cli_features: CliFeatures::new_all(false),
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

        let requested_kinds =
            CompileKind::from_requested_targets(workspace.gctx(), &vec![]).unwrap();
        let mut target_data = RustcTargetData::new(&workspace, &requested_kinds).unwrap();
        let specs = opts.packages.to_package_id_specs(&workspace).unwrap();
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
        .unwrap();

        let package_map: HashMap<PackageId, &Package> = ws_resolve
            .pkg_set
            .packages()
            .map(|pkg| (pkg.package_id(), pkg))
            .collect();

        let graph = cargo::ops::tree::build(
            &workspace,
            &ws_resolve.targeted_resolve,
            &ws_resolve.resolved_features,
            &specs,
            &CliFeatures::new_all(true),
            &target_data,
            &requested_kinds,
            package_map,
            &opts,
        )
        .unwrap();

        //get root ids
        let root_ids = ws_resolve.targeted_resolve.specs_to_ids(&specs).unwrap();
        let root_indexes = graph.indexes_from_ids(&root_ids);

        if root_indexes.len() == 0 {
            println!("nothing to print");
        }
        for (i, root_index) in root_indexes.into_iter().enumerate() {
            let deps = graph.connected_nodes(root_index, &EdgeKind::Dep(DepKind::Normal));
            for dep in deps {
                let node = graph.node(dep);
                match node {
                    Node::Package {
                        package_id,
                        features,
                        kind,
                    } => {
                        let package = graph.package_for_id(*package_id);
                        // println!("{:?}", package.name());
                        // println!("{:?}", package.version());
                        // println!("{:?}", package.targets());
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn get_latest() {
        let path = Path::new("/Users/jingyu/tmp/hello-rust/Cargo.toml");
        let gctx = cargo::util::context::GlobalContext::default().unwrap();
        let workspace = cargo::core::Workspace::new(path, &gctx).unwrap();

        //if it's error, it's a virtual workspace
        let current = workspace.current().unwrap();

        let _guard = gctx
            .acquire_package_cache_lock(CacheLockMode::DownloadExclusive)
            .unwrap();

        // Step 1: Group dependencies by SourceId
        let mut source_deps: HashMap<SourceId, Vec<_>> = HashMap::new();

        for member in workspace.members() {
            for dep in member.dependencies() {
                source_deps.entry(dep.source_id()).or_default().push(dep);
            }
        }

        // Step 2: Process each source
        for (source_id, package_names) in source_deps {
            // Create the source
            let mut source = RegistrySource::remote(source_id, &HashSet::new(), &gctx).unwrap();
            source.invalidate_cache();
            source.block_until_ready().unwrap();
            let mut summaries = Vec::new();
            for dep in package_names {
                println!("package name {:?}", dep.package_name());
                let mut dep2 = dep.clone();
                dep2.set_version_req(OptVersionReq::Any);
                println!("version req {:?}", dep.version_req());

                let poll = source.query_vec(&dep2, QueryKind::Normalized);
                // let poll=index.load_summaries(dep.package_name(), &mut *source.ops);
                // let poll =
                //     index.summaries(dep.package_name(), &OptVersionReq::Any, &mut *source.ops);

                summaries.push(poll);
            }
            let mut max_version = None;
            source.block_until_ready().unwrap();
            for summary in summaries {
                match summary {
                    Poll::Ready(summaries) => {
                        let summaries = summaries.unwrap();
                        for summary in summaries {
                            if max_version.is_none()
                                || summary.as_summary().version() > max_version.as_ref().unwrap()
                            {
                                max_version = Some(summary.as_summary().version().clone());
                            }
                        }
                    }
                    Poll::Pending => unreachable!(),
                }
            }
            println!("version {:?}", max_version.unwrap());
        }
    }
}
