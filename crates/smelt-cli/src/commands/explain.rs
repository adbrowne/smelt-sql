use anyhow::{Context, Result};
use smelt_cli::{
    argument_resolution::{compute_scope, resolve_argument, resolve_selector_args},
    build_explain_output, build_maintenance_plan_report, discover_emitted_model_files,
    discover_python_models, find_project_root, init_db, parse_selector, Config, ModelDiscovery,
    SourcesConfig,
};
use smelt_core::graph::DependencyGraph;
use smelt_logical::maintenance::Technique;
use smelt_planner::{Frontmatter, ModelGraph, ModelInfo, Planner};
use smelt_runtime::diagnostics::build_model_diagnostics;
use std::collections::HashMap;

use crate::ExplainArgs;

pub async fn explain(args: ExplainArgs, scope: Option<&str>) -> Result<()> {
    if let Some(model_name) = args.model_name.clone() {
        return explain_maintenance_plan(&args, &model_name, scope).await;
    }

    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let sources = SourcesConfig::load(&project_dir).ok();

    // Seeds are valid `smelt.ref()` targets and should appear in the graph.
    let seeds = smelt_core::discover_seed_infos(&project_dir, &config.paths);

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let sql_models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    // Build Salsa DB from all raw SQL files (including generator files) so
    // the emitted-models pipeline can run via `smelt_db::emitted_models()`,
    // and so scope resolution can call smelt_db::resolve_ref_path /
    // leaf_did_you_mean.
    let db = init_db(&project_dir, &sql_models);
    let ws = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");
    let project = db
        .project_input(&project_dir)
        .expect("project not initialized");

    // Discover generator-emitted models and their provenance.
    let (emitted_model_files, origins) =
        discover_emitted_model_files(&db, &project_dir, &config.paths);

    // Build the model list:
    //   - Exclude generator files (.gen.sql) from the hand-authored set so they
    //     don't appear as both a generator and a regular model.
    //   - Include the emitted virtual ModelFile entries produced above.
    let mut models: Vec<smelt_cli::ModelFile> = sql_models
        .into_iter()
        .filter(|m| !m.name.ends_with(".gen") && !m.path.to_string_lossy().contains(".gen."))
        .collect();
    models.extend(emitted_model_files);

    // Filter out assertion files (tests and checks) — they shouldn't appear in explain output
    models.retain(|m| !m.is_assertion());

    let python_files = discovery
        .discover_python_files()
        .with_context(|| "Failed to scan for Python models")?;

    if !python_files.is_empty() {
        let python_models = discover_python_models(
            &python_files,
            &models,
            &config,
            &project_dir,
            config.python.as_deref(),
        )
        .with_context(|| "Failed to discover Python models")?;
        models.extend(python_models);
    }

    let mut graph = DependencyGraph::build(models, sources.as_ref())
        .with_context(|| "Failed to build dependency graph")?;
    graph.add_seeds(&seeds);

    graph
        .validate()
        .with_context(|| "Dependency validation failed")?;

    // Apply --select filtering if provided.  Must happen before
    // build_explain_output so the output reflects the filtered model set.
    let execution_order: Vec<String> = if args.select.is_empty() {
        graph.execution_order()?
    } else {
        let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
        let active_scope = compute_scope(&project_dir, &cwd, &config.paths, scope);
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
        graph
            .filtered_execution_order(&selected)
            .with_context(|| "Failed to determine execution order")?
    };

    // Function bodies so batch-safety classification sees lookback declared
    // inside `smelt.define` bodies (parity with the execution path).
    let fn_bodies = smelt_runtime::build_fn_body_map(&db, ws);
    // `--period`, when supplied, resolves each incremental model's per-source
    // `scan_start`/`scan_end` in `--json` output (`docs/specs/
    // incremental_shapes.md` §"Observing the per-source clamp") — the same
    // parse `--show-sql`'s literal rendering uses.
    let period = args.period.as_deref().map(parse_period).transpose()?;
    let mut output = build_explain_output(&graph, &config, &fn_bodies, &origins, period.as_ref())?;
    // Narrow the output's execution_order to the filtered set so the
    // human-readable and JSON output reflects --select.
    output.execution_order = execution_order.clone();
    // Also filter the models map to only the selected set.
    output
        .models
        .retain(|name, _| execution_order.contains(name));

    // Build physical section via planner (no backends needed for explain).
    let mut opt_graph = ModelGraph::new();
    for model_name in &execution_order {
        let Ok(model) = graph.get_model(model_name) else {
            continue;
        };
        let metadata = model.metadata.as_deref();
        let frontmatter = Frontmatter::parse(&model.content);
        let inc_config = config
            .get_incremental_with_metadata(model_name, metadata)
            .or_else(|| frontmatter.as_ref().and_then(|f| f.batched_config()));
        let ts_config = config
            .get_timeseries_with_metadata(model_name, metadata)
            .cloned()
            .or_else(|| metadata.and_then(|m| m.timeseries.clone()));
        let plausible_columns = metadata
            .map(|m| {
                m.columns
                    .iter()
                    .filter(|(_, c)| c.contract == Some(smelt_core::metadata::Contract::Plausible))
                    .map(|(name, _)| name.clone())
                    .collect()
            })
            .unwrap_or_default();
        opt_graph.add_model(ModelInfo {
            name: model.name.clone(),
            sql: smelt_runtime::expand_function_calls(&model.content, &fn_bodies),
            refs: model
                .refs
                .iter()
                .map(|r| r.smelt_ref.to_path().join("."))
                .collect(),
            timeseries_config: ts_config,
            incremental_config: inc_config,
            plausible_columns,
        });
    }

    let planner = Planner::new();
    let (transformations, _plan_errors) = planner.plan(&opt_graph);

    // Build a PlanSummary-like physical section from planner results.
    // We don't call execute_project here since explain runs without backends.
    let physical = build_physical_section(&execution_order, &graph, &config, &transformations);
    output.physical = Some(physical);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        // Human-readable output
        println!("Project: {} (version {})", config.name, config.version);
        println!(
            "Models: {} | Execution order: {}",
            output.models.len(),
            output.execution_order.join(" → ")
        );
        println!();

        println!("Logical Graph:");
        for name in &output.execution_order {
            if let Some(model) = output.models.get(name) {
                let mat = serde_json::to_value(&model.materialization)
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| "unknown".to_string());

                print!("  {} [{}]", name, mat);

                if !model.dependencies.is_empty() {
                    print!(" ← {}", model.dependencies.join(", "));
                }

                if let Some(ref inc) = model.incremental {
                    print!(
                        " (incremental: {} by {}, {})",
                        serde_json::to_value(inc.granularity)
                            .ok()
                            .and_then(|v| v.as_str().map(|s| s.to_string()))
                            .unwrap_or_else(|| "?".to_string()),
                        inc.partition_column,
                        inc.batch_safety
                    );
                }

                if !model.tags.is_empty() {
                    print!(" [{}]", model.tags.join(", "));
                }

                if let Some(ref owner) = model.owner {
                    print!(" @{}", owner);
                }

                println!();
            }
        }

        if let Some(ref phys) = output.physical {
            println!("\nPhysical Graph:");
            if !phys.ephemerals.is_empty() {
                println!(
                    "  Ephemeral (inlined as CTEs): {}",
                    phys.ephemerals.join(", ")
                );
            }
            if !phys.transformations.is_empty() {
                println!("  Planner optimizations:");
                for t in &phys.transformations {
                    println!("    {}", t);
                }
            }
            println!("  Execution order: {}", phys.execution_order.join(" → "));
            for name in &phys.execution_order {
                if let Some(node) = phys.nodes.get(name) {
                    let mat = serde_json::to_value(&node.materialization)
                        .ok()
                        .and_then(|v| v.as_str().map(|s| s.to_string()))
                        .unwrap_or_else(|| "unknown".to_string());
                    print!("  {} [{}] {}", name, mat, node.strategy);
                    if node.logical_origins.len() > 1
                        || (node.logical_origins.len() == 1 && node.logical_origins[0] != *name)
                    {
                        print!(" (from: {})", node.logical_origins.join(", "));
                    }
                    println!();
                }
            }
        }

        println!("\nTip: Use --json for machine-readable output.");
    }

    Ok(())
}

/// Build the physical explain section from planner transformations and the
/// dependency graph (for explain mode — no backends needed).
fn build_physical_section(
    execution_order: &[String],
    graph: &DependencyGraph,
    config: &Config,
    transformations: &[smelt_planner::Transformation],
) -> smelt_cli::explain::ExplainPhysical {
    use smelt_cli::explain::{ExplainPhysical, ExplainPhysicalNode};
    use smelt_core::config::Materialization;
    use smelt_planner::Transformation;
    use std::collections::BTreeMap;

    let default_target = resolve_default_target(config);

    // Parse transformations into lookup maps for physical section rendering.
    let mut incremental_overrides: HashMap<String, (String, String)> = HashMap::new();
    let mut planner_transformations: Vec<String> = Vec::new();

    for t in transformations {
        match t {
            Transformation::SetIncremental {
                model,
                partition_column,
                ..
            } => {
                incremental_overrides
                    .insert(model.clone(), (partition_column.clone(), "day".to_string()));
                planner_transformations.push(format!(
                    "{} → incremental (partition: {})",
                    model, partition_column
                ));
            }
            Transformation::ReplaceWithPlan { model, steps } => {
                planner_transformations.push(format!(
                    "{} → cube split ({} steps)",
                    model,
                    steps.len()
                ));
            }
            _ => {}
        }
    }

    let mut nodes = BTreeMap::new();
    let mut ephemerals = Vec::new();
    let mut phys_execution_order = Vec::new();

    for model_name in execution_order {
        let Ok(model_file) = graph.get_model(model_name) else {
            continue;
        };
        let metadata = model_file.metadata.as_deref();
        let materialization = config.get_materialization_with_metadata(model_name, metadata);
        let frontmatter = smelt_planner::Frontmatter::parse(&model_file.content);

        if materialization == Materialization::Ephemeral {
            ephemerals.push(model_name.clone());
            continue;
        }

        phys_execution_order.push(model_name.clone());

        let inc_config = config
            .get_incremental_with_metadata(model_name, metadata)
            .or_else(|| frontmatter.as_ref().and_then(|f| f.batched_config()));
        let ts_config = config
            .get_timeseries_with_metadata(model_name, metadata)
            .cloned()
            .or_else(|| metadata.and_then(|m| m.timeseries.clone()));

        let strategy = if let Some((part_col, gran)) = incremental_overrides.get(model_name) {
            format!(
                "incremental (partition: {}, granularity: {})",
                part_col, gran
            )
        } else if let (Some(_), Some(ts)) = (&inc_config, &ts_config) {
            let gran = serde_json::to_value(ts.granularity)
                .ok()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "?".to_string());
            format!(
                "incremental (partition: {}, granularity: {})",
                ts.partition_column, gran
            )
        } else if metadata.is_some_and(|m| m.is_keyed()) {
            "cumulative_aggregate".to_string()
        } else {
            "full_refresh".to_string()
        };

        let model_target = config.get_target(model_name, metadata, &default_target);

        nodes.insert(
            model_name.clone(),
            ExplainPhysicalNode {
                strategy,
                materialization,
                target: model_target,
                logical_origins: vec![model_name.clone()],
            },
        );
    }

    ExplainPhysical {
        execution_order: phys_execution_order,
        nodes,
        ephemerals,
        transformations: planner_transformations,
    }
}

/// `smelt explain <model>` — the maintenance-plan report
/// (`incremental_models.md` §Surface "CLI"). Read-only: consumes
/// `smelt_db::maintenance_plan_report` (itself a thin wrapper over the pure
/// derivation in `smelt-logical`) rather than re-deriving admission,
/// locality, or ledger logic (maintenance-plan-purity invariant,
/// `architecture.md` §"Constraints & Invariants" item 12).
async fn explain_maintenance_plan(
    args: &ExplainArgs,
    model_name: &str,
    scope: Option<&str>,
) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let sources = SourcesConfig::load(&project_dir).ok();

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let mut models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    let python_files = discovery
        .discover_python_files()
        .with_context(|| "Failed to scan for Python models")?;
    if !python_files.is_empty() {
        let python_models = discover_python_models(
            &python_files,
            &models,
            &config,
            &project_dir,
            config.python.as_deref(),
        )
        .with_context(|| "Failed to discover Python models")?;
        models.extend(python_models);
    }

    let db = init_db(&project_dir, &models);
    let ws = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");
    let project = db
        .project_input(&project_dir)
        .expect("project not initialized");

    let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
    let active_scope = compute_scope(&project_dir, &cwd, &config.paths, scope);
    let canonical = resolve_argument(&db, ws, project, active_scope.as_ref(), model_name)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let model = models
        .iter()
        .find(|m| m.canonical_path() == canonical)
        .ok_or_else(|| anyhow::anyhow!("Model '{}' not found", canonical))?;

    let file = db
        .source_file(&model.path)
        .expect("model file not registered");

    let Some(mut result) = smelt_db::maintenance_plan_report(&db, ws, file) else {
        println!(
            "no maintenance plan: `{}` is not an incremental model with a declared grain",
            canonical
        );
        return Ok(());
    };

    let graph = DependencyGraph::build(models.clone(), sources.as_ref())
        .with_context(|| "Failed to build dependency graph")?;
    let upstream = graph.get_upstream(&canonical);

    let source_infos = smelt_core::discover_source_infos(&project_dir, &config.paths);
    let (own_contract, edges) =
        smelt_cli::explain::build_relation_contract(model, &models, &upstream, &source_infos);

    // Per-inbound-edge output-delta verdict (`docs/specs/incremental_models.md`
    // §Surface "CLI"): a model edge reads the same `ModelEdge::output_shape`
    // the run loop's admission derives (`smelt_db::model_edges_for`, phase 9
    // of `docs/outcomes/20260809-output-delta-typing/outcome.md`); a source
    // edge is seeded straight from its declared mutation profile
    // (`output_delta::seed_shape_for_source`) — never re-derived, only
    // matched to the `edges` list already built above so `smelt explain`'s
    // rendering and this vector agree on which name labels which edge.
    // This model's own delta signature (`incremental_models.md` §Surface
    // "CLI", Headline bullet) — the SAME single-owner derivation
    // `ref_model_edge` applies when a downstream reports this model as an
    // upstream edge (`docs/outcomes/20260904-delta-signature-front-door/
    // outcome.md` phase 1), never a second one.
    let own_output_delta = smelt_db::model_output_delta_for(&db, ws, file);

    let edge_delta_types: Vec<(String, smelt_logical::analysis::output_delta::OutputDelta)> = {
        let model_edges = smelt_db::model_edges_for(&db, ws, file);
        edges
            .iter()
            .filter_map(|edge| match edge.provider {
                smelt_cli::explain::RelationContractProvider::Model => {
                    let bare_edge_name = edge.name.strip_prefix("models.").unwrap_or(&edge.name);
                    model_edges
                        .iter()
                        .find(|me| {
                            me.name.strip_prefix("models.").unwrap_or(&me.name) == bare_edge_name
                        })
                        .and_then(|me| me.output_shape.clone())
                        .map(|shape| (edge.name.clone(), shape))
                }
                smelt_cli::explain::RelationContractProvider::Source => {
                    let bare_name = edge.name.strip_prefix("sources.").unwrap_or(&edge.name);
                    smelt_cli::explain::find_source_info(&source_infos, bare_name).map(|info| {
                        let facts =
                            smelt_logical::analysis::output_delta::SourceFacts::from_source_info(
                                bare_name, info,
                            );
                        (
                            edge.name.clone(),
                            smelt_logical::analysis::output_delta::seed_shape_for_source(&facts),
                        )
                    })
                }
            })
            .collect()
    };

    let maintenance_cfg = model
        .metadata
        .as_deref()
        .and_then(|m| m.maintenance.as_ref());
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] =
        maintenance_cfg.map(|m| m.cells.as_slice()).unwrap_or(&[]);
    let defaults_cfg = maintenance_cfg.and_then(|m| m.defaults.as_ref());
    let contract_cfg = model.metadata.as_deref().and_then(|m| m.contract.as_ref());

    // Target/schema/dialect derivation stays offline — only `smelt.yml`
    // target metadata, never a live connection (`docs/specs/cli.md`
    // §"`smelt explain <model>` maintenance-plan report") — so it is safe
    // to compute up front and reuse for both the probe plan below and
    // `--show-sql`'s statement rendering further down.
    let default_target = resolve_default_target(&config);
    let target = config.get_target(&canonical, model.metadata.as_deref(), &default_target);
    let schema = config
        .targets
        .get(&target)
        .map(|t| t.schema.clone())
        .unwrap_or_else(|| "main".to_string());
    let sql_dialect = config
        .targets
        .get(&target)
        .and_then(|t| t.backend_type().ok())
        .map(backend_type_to_sql_dialect)
        .unwrap_or(smelt_backend::SqlDialect::DuckDB);
    let dialect = smelt_backend::maintenance_dialect(sql_dialect);

    // Availability resolution (`state.md` §"The degradation contract" step
    // 2, `docs/outcomes/20260904-state-residency/outcome.md` phase 6):
    // applied once, here, before any of the report/JSON/`--show-sql` paths
    // below consume `result.plan.cells`, so all three agree on which cells
    // carry a recorded `state_downgrade`. Resolved offline from the
    // model's declared target dialect and `state.warehouse_tables` — never
    // a live connection, matching this command's offline posture.
    let availability =
        smelt_runtime::maintenance_availability::availability_for_run(sql_dialect, &config);
    smelt_logical::maintenance::availability::resolve_availability(
        &mut result.plan.cells,
        &availability,
    );

    // Definition-delta status (`docs/specs/definition_deltas.md`
    // §"Detection"): read from the same single-owner derivation the run
    // gate and `smelt migrate` use, via `FileStore`'s local target state —
    // never a live backend connection, matching this command's offline
    // posture. Only a `Pending` verdict is reported; every other status is
    // unremarkable.
    let definition_delta_file_store =
        smelt_state::file_store::FileStore::new(&project_dir, &target);
    let pending_definition_delta = match smelt_runtime::definition_delta::detect_definition_delta(
        &definition_delta_file_store,
        model,
        &models,
        sources.as_ref(),
        &db,
    ) {
        Ok(smelt_runtime::definition_delta::DefinitionDeltaStatus::Pending {
            verdict,
            plan_hash,
            ..
        }) => Some((verdict, plan_hash)),
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("Definition-delta detection failed for '{canonical}': {e}. Omitting.");
            None
        }
    };

    // The declared-fact probe plan (`docs/specs/model_properties.md`
    // §"Probe obligation"): built pure/offline by the shared
    // `smelt_runtime::probe_plan` owner, never re-derived here
    // (`docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan
    // report").
    let probe_entries = smelt_runtime::probe_plan::probe_plan_for_model(
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

    let report = build_maintenance_plan_report(
        &canonical,
        &result,
        &own_contract,
        &edges,
        cells_cfg,
        defaults_cfg,
        contract_cfg,
        &source_infos,
        &probe_entries,
        config.probes.cadence,
        &edge_delta_types,
        pending_definition_delta.as_ref(),
        own_output_delta.as_ref(),
    )
    .with_context(|| {
        format!(
            "Failed to build maintenance plan report for `{}`",
            canonical
        )
    })?;

    // `--json` is honored with or without `--show-sql` (`docs/specs/cli.md`
    // §"`smelt explain <model>` maintenance-plan report") — the JSON schema is
    // identical either way, so only the plain-text rendering short-circuits
    // here. Falling through on `--json` alone must never print the text
    // report: a machine consumer would get prose with exit 0 (fail-loud).
    if !args.show_sql && !args.json {
        println!("{}", report);
        return Ok(());
    }

    // `--show-sql`: print, after the report, the maintenance statements
    // each cell executes — built from the same pure emitters a run
    // executes (`docs/specs/incremental_models.md` §"Statement emission
    // (single owner)"). Never connects to a backend: `CompilerRegistry`
    // only needs `smelt.yml` target metadata, not a live connection.
    let mut registry = smelt_runtime::CompilerRegistry::new(&config, &config.targets);
    let fn_bodies = smelt_runtime::build_fn_body_map(&db, ws);
    // Kept for the `--period` output-window derivation below (`expand_function_calls`
    // needs its own copy); `set_function_bodies_all` takes the registry's copy by value.
    let fn_bodies_for_windowing = fn_bodies.clone();
    registry.set_function_bodies_all(fn_bodies);

    // Wire real upstream-column typing so `apply_type_casts` sees the
    // actual `smelt.ref()` column types instead of falling through to the
    // `BigInt` default cast for fractional aggregates over upstream refs.
    // `UpstreamSchemas::from_database` is purely Salsa/type-inference
    // derived (`resolved_model_schema`) — no backend connection, so this is
    // safe inside `explain` (`docs/specs/cli.md` §"`smelt explain <model>`
    // maintenance-plan report").
    match smelt_runtime::UpstreamSchemas::from_database(&db, &project_dir, &models) {
        Ok(upstream_schemas) => {
            registry.set_upstream_schemas_all(std::sync::Arc::new(upstream_schemas));
        }
        Err(e) => {
            tracing::warn!(
                "explain --show-sql: upstream schema derivation failed ({e}); \
                 printed casts may fall back to BIGINT defaults"
            );
        }
    }

    // Wire a real ephemeral resolver (mirrors `smelt-runtime::execute.rs`'s
    // dry-run compile block) so a `smelt.<ephemeral>` ref in this model's
    // SELECT body is CTE-inlined exactly as a run would inline it, instead
    // of resolving as a physical table reference.
    // Keyed by the underscore-joined `db_name_owned()` form (matching
    // `SmeltRef::to_path().join("_")`, the key `compile_with_sql_and_ephemerals`
    // looks referenced ephemerals up by) — not the dotted `canonical_path()`,
    // which would silently miss multi-segment ephemeral refs.
    let ephemeral_models: Vec<(String, String)> = models
        .iter()
        .filter(|m| {
            config.get_materialization_with_metadata(&m.canonical_path(), m.metadata.as_deref())
                == smelt_core::config::Materialization::Ephemeral
        })
        .map(|m| (m.db_name_owned(), m.content.clone()))
        .collect();
    let resolver = registry
        .get(&target)
        .build_ephemeral_resolver(&ephemeral_models, &schema)?;

    // `Config::get_incremental_with_metadata` is the single merge point for
    // the effective `batched:` config (frontmatter wins wholesale over the
    // `smelt.yml` model override, `docs/specs/models.md` §"The Relation
    // Contract") — the same one `smelt-runtime::execute.rs`'s live
    // `decide_column_merge_dispatch` gate consults, so `--show-sql`'s
    // rendered statements stay in parity with what a real run would build
    // (`docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan
    // report"). Reading `model.metadata.batched` directly here (the prior
    // shape) missed a `unique_key` declared only via the `smelt.yml`
    // override — the only surviving frontmatter-external spelling now that
    // the `.sql` frontmatter `batched:` sub-block is retired.
    let unique_key: Vec<String> = config
        .get_incremental_with_metadata(&model.canonical_path(), model.metadata.as_deref())
        .map(|b| b.unique_key)
        .unwrap_or_default();

    let source_timeseries = smelt_runtime::build_source_timeseries_map(&graph, &source_infos);

    let region = match &args.period {
        Some(p) => {
            let (start, end) = parse_period(p)?;
            smelt_cli::explain::RegionLiterals::Period { start, end }
        }
        None => smelt_cli::explain::RegionLiterals::Placeholders,
    };

    // When a concrete `--period` was given, derive the **output window**
    // (`docs/specs/model_transforms.md` §Semantics "The output window is
    // derived, never assumed") the same way a live run's `build_model_plans`
    // does — identity for a plain `partition_column`, skew-inverted when the
    // model's own SQL declares a Form B relation between the partition
    // column and its driving date column — plus the per-source scan margin a
    // `Technique::DeleteInsert` cell's read must widen by. `--period` names
    // the *run* window a real invocation would be given; the statements
    // `--show-sql` reports must reflect what that run actually reads/writes,
    // not the literal text typed on the command line.
    let derived_window = match &args.period {
        Some(_) => build_derived_window(
            &config,
            model,
            &canonical,
            &source_timeseries,
            &fn_bodies_for_windowing,
            &region,
        )?,
        None => None,
    };

    // `--technique <name>` (`docs/specs/ui_model_diagnostics.md` §Surface
    // "CLI"): parsed once, up front, so a malformed name fails loud before
    // any of the (expensive — one emitter call per technique per cell)
    // shared-diagnostics work below runs.
    let technique_requested = args
        .technique
        .as_deref()
        .map(parse_technique_arg)
        .transpose()?;

    // The shared `smelt-runtime::diagnostics` builder is the single owner
    // of the technique-preview set and the property set
    // (`docs/specs/ui_model_diagnostics.md` §Semantics "Thin-consumer
    // boundary") — and, since this fix, the sole deriver of the *symbolic*
    // statement shape the default `--show-sql` text report renders too
    // (`build_admitted_statement_group` in `smelt_cli::explain` reads its
    // `Admitted` preview entry rather than re-deriving); `smelt-cli` no
    // longer keeps its own second copy of the no-`--period` derivation, so
    // `diagnostics` is now always built (never gated behind `--json`/
    // `--technique`).
    let diagnostics = {
        let bound_ctx = smelt_cli::explain::build_bound_context(&canonical, &graph, &config);
        build_model_diagnostics(
            model,
            &models,
            &upstream,
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
        )
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("Failed to build model diagnostics for `{}`", canonical))?
    };

    // `statements` is this cell's *admitted*-technique rendering — read
    // straight from `diagnostics.cells`' own `Admitted` preview entry for
    // every cell except a `Technique::DeleteInsert` cell under a concrete
    // `--period`, which still needs the real skew-aware/scan-widened
    // derivation `build_technique_statements`'s symbolic preview
    // deliberately does not attempt (`smelt_cli::explain::
    // build_all_cell_statements`'s own doc comment).
    let statements = smelt_cli::explain::build_all_cell_statements(
        &result.plan.cells,
        &diagnostics.cells,
        model,
        &schema,
        &target,
        &registry,
        &resolver,
        dialect,
        &region,
        derived_window.as_ref(),
    );

    if args.json {
        let json = smelt_cli::explain::build_maintenance_plan_json(
            &canonical,
            &result.plan.cells,
            &statements,
            own_contract.clone(),
            edges.clone(),
            &diagnostics.cells,
            diagnostics.properties.clone(),
            result.state_columns.clone(),
            result.execution_postures.clone(),
            result.is_snapshot_reconcile,
            probe_entries.clone(),
            config.probes.cadence,
            &result.column_groups,
            contract_cfg,
            pending_definition_delta.as_ref(),
            own_output_delta.as_ref(),
            result.plan.key_locality.as_ref(),
        );
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }

    println!("{}", report);
    match technique_requested {
        Some(technique) => {
            print!(
                "{}",
                smelt_cli::explain::render_technique_previews_text(&diagnostics.cells, technique)
            );
        }
        None => {
            print!(
                "{}",
                smelt_cli::explain::render_cell_statements_text(&statements)
            );
        }
    }

    Ok(())
}

/// The project's default active target (`config.target`) when declared;
/// otherwise the lexicographically-first target name. A project declaring
/// 2+ targets and no `target:` default has no canonical "the" target, so
/// falling back to `config.targets.keys().next()` (`HashMap` iteration,
/// randomized per process) previously made `smelt explain`'s per-model
/// dialect derivation — and, since availability resolution was wired in
/// (`docs/outcomes/20260904-state-residency/outcome.md` phase 6), a
/// ledger-requiring cell's admitted-vs-downgraded verdict — nondeterministic
/// across runs of the same command against the same project. Sorting is a
/// stopgap for a real "no target declared" diagnostic; it only needs to be
/// deterministic, not any particular target.
fn resolve_default_target(config: &Config) -> String {
    if let Some(target) = &config.target {
        return target.clone();
    }
    let mut names: Vec<&String> = config.targets.keys().collect();
    names.sort();
    names.first().map(|s| s.to_string()).unwrap_or_default()
}

/// `BackendType` has only two variants today (`DuckDB`, `Spark`) —
/// `smelt_backend::maintenance_dialect` and availability resolution both
/// take the richer `SqlDialect` (which also has `PostgreSQL`); this is the
/// narrow bridge from a target's declared backend to it.
fn backend_type_to_sql_dialect(
    backend_type: smelt_core::config::BackendType,
) -> smelt_backend::SqlDialect {
    match backend_type {
        smelt_core::config::BackendType::DuckDB => smelt_backend::SqlDialect::DuckDB,
        smelt_core::config::BackendType::Spark => smelt_backend::SqlDialect::SparkSQL,
        smelt_core::config::BackendType::BigQuery => smelt_backend::SqlDialect::BigQuery,
    }
}

/// Derive the `--period`-relative output window + per-source scan margin a
/// `Technique::DeleteInsert` cell's statements are built from
/// (`smelt_cli::explain::DerivedWindow`), by calling the exact same
/// single-owner windowing derivation a live run's `build_model_plans` uses
/// (`smelt_runtime::windowing::compute_incremental_windows`,
/// `docs/specs/model_transforms.md` §Semantics "The output window is
/// derived, never assumed"). Returns `Ok(None)` for a model with no
/// `timeseries:`/`incremental:` declared (nothing to derive; the caller's
/// `Technique::DeleteInsert` branch itself already refuses those with a
/// clear reason) or an empty `--period` range.
fn build_derived_window(
    config: &Config,
    model: &smelt_core::ModelFile,
    canonical: &str,
    source_timeseries: &smelt_planner::SourceTimeseriesMap,
    fn_bodies: &smelt_runtime::FnBodyMap,
    region: &smelt_cli::explain::RegionLiterals,
) -> Result<Option<smelt_cli::explain::DerivedWindow>> {
    let smelt_cli::explain::RegionLiterals::Period { start, end } = region else {
        return Ok(None);
    };

    let metadata = model.metadata.as_deref();
    let frontmatter = Frontmatter::parse(&model.content);
    let Some(ts) = config
        .get_timeseries_with_metadata(canonical, metadata)
        .cloned()
        .or_else(|| metadata.and_then(|m| m.timeseries.clone()))
    else {
        return Ok(None);
    };
    let Some(inc) = config
        .get_incremental_with_metadata(canonical, metadata)
        .or_else(|| frontmatter.as_ref().and_then(|f| f.batched_config()))
    else {
        return Ok(None);
    };

    // Own `smelt.ref()` list, restricted to sources this model actually
    // depends on — mirrors `execute.rs::build_model_plans`'s `dep_ts`
    // construction exactly (`source_timeseries` also carries this model's
    // own frontmatter `timeseries:` entry, which must be excluded or it
    // inflates the map with a spurious self-entry).
    let model_ref_paths: std::collections::HashSet<String> = model
        .refs
        .iter()
        .map(|r| format!("smelt.{}", r.smelt_ref.to_path().join(".")))
        .collect();
    let dep_ts: HashMap<String, (Vec<String>, String)> = source_timeseries
        .iter()
        .filter(|(smelt_ref, _)| model_ref_paths.contains(*smelt_ref))
        .filter_map(|(smelt_ref, ts_cfg)| {
            let path = smelt_ref.strip_prefix("smelt.")?;
            let segs: Vec<String> = path.split('.').map(String::from).collect();
            Some((smelt_ref.clone(), (segs, ts_cfg.partition_column.clone())))
        })
        .collect();

    // Bound derivation (both the skew classifier and the temporal-dependency
    // scan-widening classifier) must see a `smelt.define` function body's own
    // `RANGE BETWEEN INTERVAL` frames — expand exactly as
    // `build_explain_output`/`build_model_plans` already do.
    let expanded_sql = smelt_runtime::expand_function_calls(&model.content, fn_bodies);

    // `smelt explain` has no Salsa handle to resolve the model's schema, so
    // it can't read `partition_column`'s type the way a live run does
    // (`execute.rs::resolve_partition_axes`) — it infers the axis from the
    // `--period` literal's own form instead, the same fail-open posture
    // `build_model_plans` already uses when a schema read is unavailable
    // (`smelt_runtime::windowing::axis_implied_by_literal_form`).
    let axis = smelt_runtime::windowing::axis_implied_by_literal_form(Some(start));
    let full_range = smelt_runtime::TimeRange {
        start: start.clone(),
        end: end.clone(),
        axis,
    };

    let windows = smelt_runtime::windowing::compute_incremental_windows(
        &ts,
        &inc,
        &expanded_sql,
        &dep_ts,
        &full_range,
        axis,
        None,
        false,
    )
    .map_err(|e| anyhow::anyhow!("Failed to derive the output window for '{canonical}': {e}"))?;

    let Some((output_start, output_end)) = windows
        .batches
        .iter()
        .map(|b| (b.partition_start, b.partition_end))
        .reduce(|(acc_start, acc_end), (s, e)| (acc_start.min(s), acc_end.max(e)))
    else {
        return Ok(None);
    };

    let scan_bounds = smelt_runtime::build_model_source_bounds(model, source_timeseries, canonical);

    Ok(Some(smelt_cli::explain::DerivedWindow {
        output_start: output_start.to_string(),
        output_end: output_end.to_string(),
        scan_bounds,
        skew: windows.skew,
        run_start: chrono::Utc::now(),
        axis,
    }))
}

/// Parse `--period <start>..<end>` — either `YYYY-MM-DD..YYYY-MM-DD` (a
/// calendar-axis model) or `<int>..<int>` (an integer-axis model,
/// `docs/specs/incremental_shapes.md` §"The partition grain" rule 8a —
/// "`--period` bounds are read in the same domain"), end exclusive. A named
/// CLI error (never a panic) on a malformed range — mirrors
/// `smelt_runtime::propagation::parse_landed_range`'s grammar and error
/// shape for the sibling `--landed` flag.
fn parse_period(value: &str) -> Result<(String, String)> {
    let (start, end) = value
        .split_once("..")
        .with_context(|| format!("malformed --period range '{value}': expected <start>..<end>"))?;

    if let (Ok(start_i), Ok(end_i)) = (start.parse::<i64>(), end.parse::<i64>()) {
        if end_i < start_i {
            anyhow::bail!("malformed --period range '{value}': end is before start");
        }
        return Ok((start_i.to_string(), end_i.to_string()));
    }

    let start_date = chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d")
        .with_context(|| format!("malformed --period range '{value}': invalid start date"))?;
    let end_date = chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d")
        .with_context(|| format!("malformed --period range '{value}': invalid end date"))?;
    if end_date < start_date {
        anyhow::bail!("malformed --period range '{value}': end is before start");
    }
    Ok((start_date.to_string(), end_date.to_string()))
}

/// Parse `--technique <name>` into a [`Technique`]
/// (`docs/specs/ui_model_diagnostics.md` §Surface "CLI"): a named CLI error
/// (never a panic) on an unrecognized name, listing the accepted set
/// (fail-loud discipline, `CLAUDE.md` §"Fail-loud discipline"). `recompute`
/// and `delete_insert` both resolve to `Technique::DeleteInsert` — `--show-
/// sql`'s own maintenance-plan report already documents `DeleteInsert` as
/// doubling for region recompute (`docs/specs/incremental_models.md`), and
/// `smelt_runtime::diagnostics`'s technique registry has no separate
/// recompute emitter or `Technique` variant.
fn parse_technique_arg(value: &str) -> Result<Technique> {
    match value {
        "delete_insert" | "recompute" => Ok(Technique::DeleteInsert),
        "keyed_fold" => Ok(Technique::KeyedFold),
        "column_scoped_merge" => Ok(Technique::ColumnScopedMerge),
        "in_place_update" => Ok(Technique::InPlaceUpdate),
        "per_group_recompute" => Ok(Technique::PerGroupRecompute),
        other => anyhow::bail!(
            "unknown --technique '{other}': expected one of delete_insert, keyed_fold, \
             column_scoped_merge, in_place_update, per_group_recompute, recompute"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_core::{
        config::{Materialization, RefreshStrategy},
        discovery::ModelKind,
        graph::DependencyGraph,
        metadata::ModelMetadata,
        model_id::ModelId,
        ModelFile,
    };

    /// A model declared with `materialization: table` + `refresh: incremental`
    /// and `grain: key` (the former `refresh: cumulative`/`keyed`) must
    /// report `"cumulative_aggregate"` as its physical strategy in `smelt
    /// explain` output (the human-readable strategy label), NOT
    /// `"full_refresh"`.
    #[test]
    fn refresh_cumulative_table_strategy_is_cumulative_aggregate() {
        // Minimal smelt.yml in a temp dir.
        let tmp = tempfile::TempDir::new().expect("create tempdir");
        let yml = "name: test_proj\n\
                   version: 1\n\
                   paths:\n  - models\n\
                   targets:\n  dev:\n    type: duckdb\n    schema: main\n\
                   default_materialization: view\n";
        std::fs::write(tmp.path().join("smelt.yml"), yml).unwrap();
        let config = smelt_cli::Config::load(tmp.path()).expect("Config::load from temp smelt.yml");

        // Build a ModelFile with `materialization: table` + `refresh: incremental` + `grain: key`.
        let model_name = "my_cumulative_model";
        let path: std::path::PathBuf = format!("models/{}.sql", model_name).into();
        let metadata = ModelMetadata {
            materialization: Some(Materialization::Table),
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(smelt_core::config::Grain::Key),
            ..ModelMetadata::default()
        };
        let model_file = ModelFile {
            name: model_name.to_string(),
            model_id: ModelId::from_path(path.clone()),
            path,
            content: String::new(),
            refs: vec![],
            parse_errors: vec![],
            metadata: Some(Box::new(metadata)),
            kind: ModelKind::Sql,
            address_segments: vec![model_name.to_string()],
        };

        let graph = DependencyGraph::build(vec![model_file], None).expect("DependencyGraph::build");

        let execution_order = vec![model_name.to_string()];
        let physical = build_physical_section(&execution_order, &graph, &config, &[]);

        let node = physical.nodes.get(model_name).unwrap_or_else(|| {
            panic!(
                "expected node '{}' in physical section; got: {:?}",
                model_name,
                physical.nodes.keys().collect::<Vec<_>>()
            )
        });

        assert_eq!(
            node.strategy, "cumulative_aggregate",
            "model with materialization: table + refresh: cumulative must report \
             strategy 'cumulative_aggregate', not '{}'",
            node.strategy
        );
    }

    /// Sanity check: a plain `materialization: table` model (no refresh: cumulative)
    /// still reports `"full_refresh"`.
    #[test]
    fn plain_table_strategy_is_full_refresh() {
        let tmp = tempfile::TempDir::new().expect("create tempdir");
        let yml = "name: test_proj\n\
                   version: 1\n\
                   paths:\n  - models\n\
                   targets:\n  dev:\n    type: duckdb\n    schema: main\n\
                   default_materialization: view\n";
        std::fs::write(tmp.path().join("smelt.yml"), yml).unwrap();
        let config = smelt_cli::Config::load(tmp.path()).expect("Config::load from temp smelt.yml");

        let model_name = "plain_table";
        let path: std::path::PathBuf = format!("models/{}.sql", model_name).into();
        let metadata = ModelMetadata {
            materialization: Some(Materialization::Table),
            refresh: None,
            ..ModelMetadata::default()
        };
        let model_file = ModelFile {
            name: model_name.to_string(),
            model_id: ModelId::from_path(path.clone()),
            path,
            content: String::new(),
            refs: vec![],
            parse_errors: vec![],
            metadata: Some(Box::new(metadata)),
            kind: ModelKind::Sql,
            address_segments: vec![model_name.to_string()],
        };

        let graph = DependencyGraph::build(vec![model_file], None).expect("DependencyGraph::build");
        let execution_order = vec![model_name.to_string()];
        let physical = build_physical_section(&execution_order, &graph, &config, &[]);

        let node = physical.nodes.get(model_name).expect("node exists");
        assert_eq!(
            node.strategy, "full_refresh",
            "plain table model must report strategy 'full_refresh'"
        );
    }
}
