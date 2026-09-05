//! Whole-workspace property-profile derivation
//! (`docs/specs/property_diff.md` §"The property profile") — the function
//! both sides of `smelt explain --diff` (baseline and working tree) call to
//! get one [`PropertyProfile`] per maintained model.
//!
//! Every input this builds from a [`LoadedWorkspace`] goes through the same
//! loading-parity path the CLI and LSP use
//! (`docs/specs/architecture.md` §"Workspace loading parity rule (CLI ↔
//! LSP)"): `smelt_db::workspace_ingest::ingest_loaded_workspace` +
//! `Database::set_workspace`, mirrored from `smelt_cli::init_db` and
//! `smelt-cli`'s `commands::list::run`. `smelt-runtime` already builds its
//! own `Database` this way elsewhere (`combined_loop::build_generator_db`),
//! so this is not a new pattern.
//!
//! Per model: `smelt_db::maintenance_plan_report` (the thin Salsa query over
//! pure `smelt-logical` derivation) → availability resolution → `probe_plan::
//! probe_plan_for_model` → `diagnostics::build_model_diagnostics` → its
//! `.profile` field. A model with no maintenance plan (not `refresh:
//! incremental`, or no shape-defining fact) is simply absent from the
//! returned map — it has no profile to diff, matching
//! `property_profile_parity`'s own skip
//! (`docs/specs/property_diff.md` §Interactions "Salsa purity").
//!
//! **Fail-loud, not empty** (`docs/specs/property_diff.md` §Constraints
//! item 6, "an unresolvable baseline is an error … never an empty diff"):
//! a workspace-init or dependency-graph-build failure returns
//! [`ProfileWorkspaceError`], never an empty map — an empty map here would
//! make every model in the *other* side's map look "removed", i.e. tell the
//! user they deleted their entire project. A single model's own derivation
//! failure (bad SQL mid-edit, an unresolvable ephemeral reference) is
//! recorded per-model in [`WorkspaceProfiles::failures`] rather than
//! silently skipped, so a later renderer can report that model as
//! added/removed with the failure as its reason (`docs/specs/property_diff.md`
//! §Constraints item 6) instead of a plain, unexplained absence — this phase
//! captures the reason; rendering it is a later phase's job.
//!
//! Each model's availability resolution (and the `MaintenanceDialect` its
//! probe plan uses) is keyed on that model's **own** target
//! (`smelt_runtime::execute::sql_dialect_for_target`, the same lookup
//! `smelt explain`'s single-model report uses via
//! `backend_type_to_sql_dialect`) rather than a hardcoded dialect — a
//! multi-backend project with a Spark- or BigQuery-targeted model would
//! otherwise get a diff whose techniques contradict what `smelt explain
//! <model> --json` reports for that same model
//! (`docs/specs/property_diff.md` §Constraints item 4, "Report/profile
//! parity").

use std::collections::BTreeMap;

use smelt_core::sources::{discover_source_infos, SourcesConfig};
use smelt_core::workspace::LoadedWorkspace;
use smelt_logical::analysis::profile::PropertyProfile;
use smelt_logical::analysis::source_bounds::BoundContext;
use crate::compile::CompilerRegistry;
use crate::diagnostics::build_bound_context;

/// A workspace-wide failure that means no profile can be derived for
/// *any* model — distinct from a single model's own derivation failure
/// (carried per-model in [`WorkspaceProfiles::failures`] instead).
#[derive(Debug, thiserror::Error)]
pub enum ProfileWorkspaceError {
    #[error("workspace failed to initialize: no Salsa Workspace was registered")]
    WorkspaceInitFailed,
    #[error("failed to build the dependency graph: {0}")]
    GraphBuildFailed(String),
}

/// The result of deriving profiles for every model in a workspace: the
/// profile map plus, per model that failed to derive one, the reason
/// (`docs/specs/property_diff.md` §Constraints item 6).
#[derive(Debug, Default)]
pub struct WorkspaceProfiles {
    pub profiles: BTreeMap<String, PropertyProfile>,
    /// Canonical model name -> derivation-failure reason, for a model with
    /// a maintenance plan whose profile could not be assembled.
    pub failures: BTreeMap<String, String>,
}

/// Derive one [`PropertyProfile`] per maintained model in `loaded`
/// (`docs/specs/property_diff.md` §"The property profile"), keyed by the
/// model's canonical dot-path — the same key `DiffGraph`'s `upstream`/
/// `edited` and `diff_profiles`'s `P_old`/`P_new` maps use.
pub fn profiles_for_workspace(
    loaded: &LoadedWorkspace,
) -> Result<WorkspaceProfiles, ProfileWorkspaceError> {
    let mut out = WorkspaceProfiles::default();

    let mut db = smelt_db::Database::default();
    let ingested = smelt_db::workspace_ingest::ingest_loaded_workspace(&mut db, loaded);
    db.set_workspace(ingested.source_files.clone(), vec![ingested.project]);
    let Some(ws) = smelt_db::Workspace::try_get(&db) else {
        return Err(ProfileWorkspaceError::WorkspaceInitFailed);
    };

    let legacy_sources = SourcesConfig::load(&loaded.project_root).ok();
    let graph = smelt_core::graph::DependencyGraph::build(
        loaded.sql_files.clone(),
        legacy_sources.as_ref(),
    )
    .map_err(|e| ProfileWorkspaceError::GraphBuildFailed(e.to_string()))?;

    let source_infos = discover_source_infos(&loaded.project_root, &loaded.config.paths);

    let mut registry = CompilerRegistry::new(&loaded.config, &loaded.config.targets);
    let fn_bodies = crate::fn_bodies::build_fn_body_map(&db, ws);
    registry.set_function_bodies_all(fn_bodies);

    let ephemeral_models: Vec<(String, String)> = loaded
        .sql_files
        .iter()
        .filter(|m| {
            loaded
                .config
                .get_materialization_with_metadata(&m.canonical_path(), m.metadata.as_deref())
                == smelt_core::config::Materialization::Ephemeral
        })
        .map(|m| (m.db_name_owned(), m.content.clone()))
        .collect();

    let default_target = loaded
        .config
        .target
        .clone()
        .or_else(|| {
            let mut names: Vec<&String> = loaded.config.targets.keys().collect();
            names.sort();
            names.first().map(|s| s.to_string())
        })
        .unwrap_or_default();

    let source_timeseries = crate::execute::build_source_timeseries_map(&graph, &source_infos);

    for (model, source_file) in loaded.sql_files.iter().zip(ingested.source_files.iter()) {
        let canonical = model.canonical_path();
        let Some(mut result) = smelt_db::maintenance_plan_report(&db, ws, *source_file) else {
            continue;
        };

        let bound_ctx: BoundContext = build_bound_context(&canonical, &graph, &loaded.config);
        let target =
            loaded
                .config
                .get_target(&canonical, model.metadata.as_deref(), &default_target);
        let schema = loaded
            .config
            .targets
            .get(&target)
            .map(|t| t.schema.clone())
            .unwrap_or_else(|| "main".to_string());
        // Per-model dialect/availability, keyed on this model's own target
        // (`docs/specs/property_diff.md` §Constraints item 4) — the same
        // resolution `smelt explain <model>`'s report makes, never a
        // workspace-wide default.
        let sql_dialect = crate::execute::sql_dialect_for_target(&loaded.config, &target);
        let dialect = smelt_backend::maintenance_dialect(sql_dialect);
        let availability = crate::maintenance_availability::availability_for_run(
            sql_dialect,
            &loaded.config,
        );
        smelt_logical::maintenance::availability::resolve_availability(
            &mut result.plan.cells,
            &availability,
        );

        let resolver = match registry
            .get(&target)
            .build_ephemeral_resolver(&ephemeral_models, &schema)
        {
            Ok(r) => r,
            Err(e) => {
                out.failures.insert(canonical, e.to_string());
                continue;
            }
        };

        let unique_key: Vec<String> = loaded
            .config
            .get_incremental_with_metadata(&canonical, model.metadata.as_deref())
            .map(|b| b.unique_key)
            .unwrap_or_default();
        let contract_cfg = model.metadata.as_deref().and_then(|m| m.contract.as_ref());
        let model_upstream = graph.get_upstream(&canonical);

        let probe_entries = crate::probe_plan::probe_plan_for_model(
            &canonical,
            &schema,
            &model.db_name_owned(),
            model.metadata.as_deref(),
            model.metadata.as_ref().and_then(|m| m.timeseries.as_ref()),
            model,
            &source_infos,
            &target,
            &result.plan.cells,
            result.plan.key_locality.as_ref(),
            dialect,
        );

        let diagnostics = match crate::diagnostics::build_model_diagnostics(
            model,
            &loaded.sql_files,
            &model_upstream,
            &source_infos,
            &bound_ctx,
            &result.plan.cells,
            &schema,
            &target,
            &registry,
            &resolver,
            dialect,
            &source_timeseries,
            &unique_key,
            &result.column_groups,
            &result.plan.refusals,
            &probe_entries,
            contract_cfg,
        ) {
            Ok(d) => d,
            Err(e) => {
                out.failures.insert(canonical, e.to_string());
                continue;
            }
        };

        out.profiles.insert(canonical, diagnostics.profile);
    }

    Ok(out)
}
