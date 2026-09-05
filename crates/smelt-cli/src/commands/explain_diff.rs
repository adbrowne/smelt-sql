//! `smelt explain --diff` (`docs/specs/property_diff.md`): resolve a git
//! baseline, derive every model's property profile at both versions, diff
//! them, and render the result as text (default) or JSON.
//!
//! Sequencing (`docs/outcomes/20260905-property-diff/phases/05-plan.md`
//! D2): the working tree loads and derives FIRST, so a broken working tree
//! fails before any scratch directory exists; only then is the baseline
//! resolved and materialised. The [`smelt_core::baseline::BaselineCheckout`]
//! is held until every value derived from it (the baseline's profiles and
//! the edited set) has been read, then dropped — its `Drop` deletes the
//! scratch directory (`docs/specs/property_diff.md` §Constraints item 8).
//!
//! Rendering is delegated entirely to `smelt_logical::analysis::
//! diff_render` over the `DiffReport` envelope
//! (`docs/specs/property_diff.md` §Surface "Output forms") — this module
//! assembles the report's inputs and never formats a change itself. Same
//! for the §Semantics rules `apply_failure_reasons` (C2) and
//! `DiffReport::narrow_to` (`--select`, D6): both are single-owned in
//! `smelt_logical::analysis::diff` (fix round 1, Q6) so Phase 6's Markdown
//! renderer and Phase 7's LSP reuse them rather than each reimplementing
//! the same rule.

use anyhow::{Context, Result};
use smelt_cli::argument_resolution::{compute_scope, resolve_selector_args};
use smelt_cli::{find_project_root, init_db, parse_selector, CliError, Config};
use smelt_core::baseline::{edited_set, materialize, resolve_baseline};
use smelt_core::graph::DependencyGraph;
use smelt_core::workspace::load_workspace;
use smelt_logical::analysis::diff::{
    apply_failure_reasons, diff_profiles, BaselineInfo, DiffGraph, DiffReport,
};
use smelt_logical::analysis::diff_render::text_report;
use smelt_runtime::profile::profiles_for_workspace;

use crate::ExplainArgs;

/// Run `smelt explain --diff`. `explicit_ref` is `args.diff`'s inner value
/// (`None` ⇒ default merge-base baseline).
pub async fn explain_diff(args: &ExplainArgs, explicit_ref: Option<&str>) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    // Working tree first (D2): fails before any scratch directory exists.
    let work_loaded = load_workspace(&project_dir);
    let work_profiles = profiles_for_workspace(&work_loaded)
        .with_context(|| "Failed to derive property profiles for the working tree")?;

    let resolved = resolve_baseline(&project_dir, explicit_ref)?;
    let checkout = materialize(&resolved)?;
    let base_loaded = load_workspace(checkout.project_root());
    let base_profiles = profiles_for_workspace(&base_loaded)
        .with_context(|| "Failed to derive property profiles for the baseline")?;

    let work_sources =
        smelt_core::discover_source_infos(&work_loaded.project_root, &work_loaded.config.paths);
    let base_sources =
        smelt_core::discover_source_infos(&base_loaded.project_root, &base_loaded.config.paths);
    let edited = edited_set(&base_loaded, &base_sources, &work_loaded, &work_sources);

    // The working-tree graph is the one attribution walks
    // (`docs/specs/property_diff.md` §"Attribution").
    let legacy_sources = smelt_cli::SourcesConfig::load(&project_dir).ok();
    let graph = DependencyGraph::build(work_loaded.sql_files.clone(), legacy_sources.as_ref())
        .with_context(|| "Failed to build dependency graph")?;
    let diff_graph = DiffGraph::from_dependency_graph(
        &graph,
        edited.names.clone(),
        edited.project_config_changed,
    );

    let mut diff = diff_profiles(
        &base_profiles.profiles,
        &work_profiles.profiles,
        &diff_graph,
    );
    // C2: an added/removed entry whose absence on that side was a
    // derivation FAILURE, not a genuine new/deleted model, carries that
    // failure as its reason (`docs/specs/property_diff.md` §Constraints
    // item 6, Δ1).
    apply_failure_reasons(&mut diff, &base_profiles.failures, &work_profiles.failures);

    // Everything derived from the baseline checkout has now been read;
    // drop it explicitly so the scratch directory is gone before this
    // function does any further (non-git) work.
    drop(checkout);

    let baseline_info = BaselineInfo::from(&resolved);
    let mut report = DiffReport::new(baseline_info, edited.files.clone(), diff);

    // D6: --select narrows the REPORTED set only (Δ2) — the compared set
    // (everything derived above) is untouched, so attribution stays
    // correct.
    if !args.select.is_empty() {
        let config =
            Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;
        let db = init_db(&project_dir, &work_loaded.sql_files);
        let ws = smelt_db::Workspace::try_get(&db)
            .ok_or_else(|| anyhow::anyhow!("workspace not initialized"))?;
        let project = db
            .project_input(&project_dir)
            .ok_or_else(|| anyhow::anyhow!("project not initialized"))?;
        let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
        let active_scope = compute_scope(&project_dir, &cwd, &config.paths, None);
        let resolved_select =
            resolve_selector_args(&db, ws, project, active_scope.as_ref(), &args.select)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
        let selectors: Vec<_> = resolved_select
            .iter()
            .map(|s| parse_selector(s).with_context(|| format!("Invalid selector '{}'", s)))
            .collect::<Result<_, _>>()?;
        let selected = graph
            .select_models(&selectors, &config)
            .with_context(|| "Failed to select models")?;
        report.narrow_to(&selected.into_iter().collect());
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).with_context(|| "Failed to serialize diff")?
        );
    } else {
        print!("{}", text_report(&report));
    }

    if let Some(fail_on) = &args.fail_on {
        let should_fail = match fail_on.as_str() {
            "downgrade" => report.summary.downgrades > 0,
            "any" => report.summary.shifted_models > 0,
            _ => false,
        };
        if should_fail {
            return Err(CliError::DetectedFailure(format!(
                "property diff: --fail-on {fail_on} condition held ({} downgrades, {} shifted \
                 models)",
                report.summary.downgrades, report.summary.shifted_models
            ))
            .into());
        }
    }

    Ok(())
}
