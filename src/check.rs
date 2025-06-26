use cargo::core::compiler::unit_dependencies::{build_unit_dependencies, IsArtifact};
use cargo::core::compiler::{
    CompileKind, CompileMode, CompileTarget, RustcTargetData, Unit, UnitInterner, UserIntent
};
use cargo::core::profiles::{Profiles, UnitFor};
use cargo::core::resolver::features::{FeaturesFor, ResolvedFeatures};
use cargo::core::resolver::{CliFeatures, ForceAllTargets, HasDevUnits};
use cargo::core::{Package, Target, Workspace};
use cargo::ops::{Packages, WorkspaceResolve, resolve_ws_with_opts};
use cargo::util::command_prelude::lockfile_path;
use cargo::util::important_paths::find_root_manifest_for_wd;
use cargo::util::interning::InternedString;
use cargo::{CargoResult, GlobalContext};

fn main() -> CargoResult<()> {
    // Create the GlobalContext that is used by Cargo through out the codebase

    let gctx = GlobalContext::default()?;

    // After the GlobalContext had been made, Cargo finds the root manifest for the current
    // package.

    let manifest_root = find_root_manifest_for_wd(gctx.cwd())?;
    dbg!(&manifest_root);

    // Find the lockfile

    let lockfile_path = lockfile_path(None, &gctx)?;
    dbg!(&lockfile_path);

    // The root manifest is used to create a Workspace struct. This struct is filled with data
    // regarding Workspace, such as the packages, workspace members, etc. When the package is not a
    // workspace, the members is just the package with length 1.

    let workspace = {
        let mut workspace = Workspace::new(&manifest_root, &gctx)?;
        workspace.set_requested_lockfile_path(lockfile_path);

        // this should be one if it is a real manifest and not a virtual manifest
        // dbg!(workspace.members().collect::<Vec<_>>().len());

        workspace
    };

    // Then, we need to compile the workpace. We do this by giving a CompileKind to tell rustc how
    // to compile the crates. `RustcTargetData` contains data needed by rustc in the form of the
    // host and target.
    //
    // Here, we are compiling the binary to the host system. A good indicator if we are compiling
    // to host is the absence of a `--target` flag. Other compilation to hosts are build scripts
    // and proc macros.

    let requested_kinds = &[CompileKind::Host];
    let mut target_data = RustcTargetData::new(&workspace, requested_kinds)?;
    let explicit_host_kind = CompileKind::Target(CompileTarget::new(&target_data.rustc.host)?);

    // `specs` here refer to the crate specification that we want to compile. Here we use default
    // because we are not passing any flags, such as `--exclude` or `--workspace`.

    let specs = dbg!(Packages::Default.to_package_id_specs(&workspace)?);
    dbg!(&specs);

    // We are performing `cargo check` which doesn't use any dev dependencies. Therefore, we set
    // has_dev_units to HasDevUnits::No.

    let has_dev_units = HasDevUnits::No;

    // After all preparation, we are ready to resolve the workspace for dependencies. Here I used
    // `resolve_ws_with_opts`, but we can even break this down even further. I don't know if it is
    // needed, but its good to know that we can :)

    let WorkspaceResolve {
        pkg_set,
        workspace_resolve: _,
        targeted_resolve: resolve,
        resolved_features,
    } = resolve_ws_with_opts(
        &workspace,
        &mut target_data,
        requested_kinds,
        &CliFeatures::new_all(true),
        &specs,
        has_dev_units,
        ForceAllTargets::No,
        false,
    )?;

    dbg!(&resolve);

    // `to_build_ids` are the package ids that the user wants to build. Notice that we are passing
    // `specs` as arguments for the method.

    let to_build_ids = dbg!(resolve.specs_to_ids(&specs)?);
    dbg!(&to_build_ids);

    // `to_builds` are the package representation of the package ids of `to_build_ids`. The
    // `get_many` invocation may download crates if needed, but in our example, I don't think it
    // will, since we're only checking one crate.

    let mut to_builds = dbg!(pkg_set.get_many(to_build_ids)?);

    // We order to packages to make everything pretty :)

    to_builds.sort_by_key(|p| p.package_id());

    // Profile we want to use is dev since we are running `cargo check` in a dev build and not a
    // release build

    let profile = InternedString::new("dev");

    let mut proposals: Vec<Proposal<'_>> = Vec::new();

    dbg!(&to_builds);

    for pkg in to_builds {
        let default = filter_default_targets(dbg!(pkg.targets()));
        proposals.extend(default.into_iter().map(|target| Proposal {
            pkg,
            target,
            requires_features: false,
            mode: CompileMode::Check { test: false },
        }));
    }

    dbg!(&proposals);

    let interner = UnitInterner::new();
    let profiles = Profiles::new(&workspace, profile)?;
    let mut units = Vec::new();

    for Proposal {
        pkg, target, mode, ..
    } in proposals
    {
        // I'm still unsure when to extend the units and when not to.
        units.extend(new_units(
            &workspace,
            pkg,
            target,
            mode,
            requested_kinds,
            explicit_host_kind,
            profiles.clone(),
            &interner,
            &resolved_features,
            &target_data,
        ));
    }

    dbg!(&units);

    let unit_graph = build_unit_dependencies(
        &workspace,
        &pkg_set,
        &resolve,
        &resolved_features,
        None,
        &units,
        &Vec::new(),
        &Default::default(),
        UserIntent::Check { test: false },
        &target_data,
        &profiles,
        &interner,
    )?;

    dbg!(unit_graph);

    Ok(())
}

/// Which targets are automatically added to the package list? By default, `cargo check` includes
/// all binaries and libraries to the target.
fn filter_default_targets(targets: &[Target]) -> Vec<&Target> {
    targets
        .iter()
        .filter(|t| t.is_bin() || t.is_lib())
        .collect()
}

#[derive(Debug)]
struct Proposal<'a> {
    pkg: &'a Package,
    target: &'a Target,
    requires_features: bool,
    mode: CompileMode,
}

fn new_units(
    ws: &Workspace,
    pkg: &Package,
    target: &Target,
    initial_target_mode: CompileMode,
    requested_kinds: &[CompileKind],
    explicit_host_kind: CompileKind,
    profiles: Profiles,
    interner: &UnitInterner,
    resolved_features: &ResolvedFeatures,
    target_data: &RustcTargetData,
) -> Vec<Unit> {
    let target_mode = initial_target_mode;
    let is_local = pkg.package_id().source_id().is_path();

    let features_for = FeaturesFor::from_for_host(target.proc_macro());
    let features = resolved_features.activated_features(pkg.package_id(), features_for);

    let explicit_kinds = if let Some(k) = pkg.manifest().forced_kind() {
        vec![k]
    } else {
        requested_kinds
            .iter()
            .map(|kind| match kind {
                CompileKind::Host => pkg.manifest().default_kind().unwrap_or(explicit_host_kind),
                CompileKind::Target(t) => CompileKind::Target(*t),
            })
            .collect()
    };

    dbg!(&explicit_kinds);

    explicit_kinds
        .into_iter()
        .map(move |kind| {
            let unit_for = if initial_target_mode.is_any_test() {
                UnitFor::new_test(ws.gctx(), kind)
            } else if target.for_host() {
                UnitFor::new_compiler(kind)
            } else {
                UnitFor::new_normal(kind)
            };

            let profile = profiles.get_profile(
                pkg.package_id(),
                ws.is_member(pkg),
                is_local,
                unit_for,
                kind,
            );

            let kind = kind.for_target(target);

            interner.intern(
                pkg,
                target,
                profile,
                kind,
                target_mode,
                features.clone(),
                target_data.info(kind).rustflags.clone(),
                target_data.info(kind).rustdocflags.clone(),
                target_data.target_config(kind).links_overrides.clone(),
                /*is_std*/ false,
                /*dep_hash*/ 0,
                IsArtifact::No,
                None,
                false,
            )
        })
        .collect()
}
