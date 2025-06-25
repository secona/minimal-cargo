use cargo::core::compiler::{CompileKind, RustcTargetData, Unit};
use cargo::core::resolver::{CliFeatures, ForceAllTargets, HasDevUnits};
use cargo::core::{Target, Workspace};
use cargo::ops::{resolve_ws_with_opts, Packages, WorkspaceResolve};
use cargo::util::interning::InternedString;
use cargo::{CargoResult, GlobalContext};
use cargo::util::important_paths::find_root_manifest_for_wd;

fn main() -> CargoResult<()> {
    // Create the GlobalContext that is used by Cargo through out the codebase

    let mut gctx = GlobalContext::default()?;

    // After the GlobalContext had been made, Cargo finds the root manifest for the current
    // package.

    let manifest_root = dbg!(find_root_manifest_for_wd(gctx.cwd())?);

    // The root manifest is used to create a Workspace struct. This struct is filled with data
    // regarding Workspace, such as the packages, workspace members, etc. When the package is not a
    // workspace, the members is just the package with length 1.

    let workspace = Workspace::new(&manifest_root, &gctx)?;
    // this should be one if it is a real manifest and not a virtual manifest
    // dbg!(workspace.members().collect::<Vec<_>>().len());

    // Then, we need to compile the workpace. We do this by giving a CompileKind to tell rustc how
    // to compile the crates. `RustcTargetData` contains data needed by rustc in the form of the
    // host and target.

    let requested_kinds = &[CompileKind::Host];
    let mut target_data = RustcTargetData::new(&workspace, requested_kinds)?;

    // `specs` here refer to the crate specification that we want to compile. Here we use default
    // because we are not passing any flags, such as `--exclude` or `--workspace`.

    let specs = dbg!(Packages::Default.to_package_id_specs(&workspace)?);

    // We are performing `cargo check` which doesn't use any dev dependencies. Therefore, we set
    // has_dev_units to HasDevUnits::No.

    let has_dev_units = HasDevUnits::No;

    // After all preparation, we are ready to resolve the workspace for dependencies. Here I used
    // `resolve_ws_with_opts`, but we can even break this down even further. I don't know if it is
    // needed, but its good to know that we can :)

    let WorkspaceResolve {
        mut pkg_set,
        workspace_resolve,
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

    // `to_build_ids` are the package ids that the user wants to build.

    let to_build_ids = dbg!(resolve.specs_to_ids(&specs)?);

    // `to_builds` are the package representation of the package ids of `to_build_ids`. The
    // `get_many` invocation may download crates if needed, but in our example, I don't think it
    // will, since we're only checking one crate.

    let mut to_builds = dbg!(pkg_set.get_many(to_build_ids)?);

    // We order to packages to make everything pretty :)

    to_builds.sort_by_key(|p| p.package_id());

    // Profile we want to use is dev since we are running `cargo check` in a dev build and not a
    // release build

    let profile = InternedString::new("dev");

    // TODO: continue with unit generate root units and building the unit dependencies.

    let units: Vec<Unit> = Vec::new();

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
