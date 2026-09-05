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
//! pure `smelt-logical` derivation) → `probe_plan::probe_plan_for_model` →
//! `diagnostics::build_model_diagnostics` → its `.profile` field. A model
//! with no maintenance plan (not `refresh: incremental`, or no shape-defining
//! fact) is simply absent from the returned map — it has no profile to diff,
//! matching `property_profile_parity`'s own skip
//! (`docs/specs/property_diff.md` §Interactions "Salsa purity").
//!
//! **Scope note.** The maintenance dialect used here is fixed to
//! [`MaintenanceDialect::DuckDb`], the same simplification
//! `property_profile_parity`'s harness already made (its own doc comment:
//! "every fixture workspace this gate runs over targets DuckDB").
//! Per-target dialect resolution and `state.md`'s live availability
//! resolution (`maintenance_availability::availability_for_run` +
//! `smelt_logical::maintenance::availability::resolve_availability`, which
//! populates a cell's `state_downgrade`) are not wired in here — both need
//! a target's declared backend, which is a CLI-command-level concern
//! (`smelt_cli::commands::explain`'s `backend_type_to_sql_dialect`) not
//! reproduced in this phase. `CellVerdict::state_downgrade` is carried and
//! diffed as its own dimension regardless (`docs/specs/property_diff.md`
//! §"Direction" "state_downgrade" row); it will simply be `None` for every
//! cell derived through this function until a later phase wires live
//! availability resolution into the diff's derivation, same as the parity
//! gate today.

use std::collections::BTreeMap;

use smelt_core::sources::{discover_source_infos, SourcesConfig};
use smelt_core::workspace::LoadedWorkspace;
use smelt_logical::analysis::profile::PropertyProfile;
use smelt_logical::analysis::source_bounds::BoundContext;
use smelt_logical::maintenance::emit::MaintenanceDialect;

use crate::compile::CompilerRegistry;
use crate::diagnostics::build_bound_context;

/// Derive one [`PropertyProfile`] per maintained model in `loaded`
/// (`docs/specs/property_diff.md` §"The property profile"), keyed by the
/// model's canonical dot-path — the same key `DiffGraph`'s `upstream`/
/// `edited` and `diff_profiles`'s `P_old`/`P_new` maps use.
pub fn profiles_for_workspace(loaded: &LoadedWorkspace) -> BTreeMap<String, PropertyProfile> {
    let mut profiles = BTreeMap::new();

    let mut db = smelt_db::Database::default();
    let ingested = smelt_db::workspace_ingest::ingest_loaded_workspace(&mut db, loaded);
    db.set_workspace(ingested.source_files.clone(), vec![ingested.project]);
    let Some(ws) = smelt_db::Workspace::try_get(&db) else {
        return profiles;
    };

    let legacy_sources = SourcesConfig::load(&loaded.project_root).ok();
    let graph = match smelt_core::graph::DependencyGraph::build(
        loaded.sql_files.clone(),
        legacy_sources.as_ref(),
    ) {
        Ok(g) => g,
        Err(_) => return profiles,
    };
    let source_infos = discover_source_infos(&loaded.project_root, &loaded.config.paths);
    let dialect = MaintenanceDialect::DuckDb;

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
        let Some(result) = smelt_db::maintenance_plan_report(&db, ws, *source_file) else {
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

        let resolver = match registry
            .get(&target)
            .build_ephemeral_resolver(&ephemeral_models, &schema)
        {
            Ok(r) => r,
            Err(_) => continue,
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
            Err(_) => continue,
        };

        profiles.insert(canonical, diagnostics.profile);
    }

    profiles
}
