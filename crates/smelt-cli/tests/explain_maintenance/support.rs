use std::path::Path;
use std::process::Command;

use smelt_cli::argument_resolution::{compute_scope, resolve_argument};
use smelt_cli::{
    build_maintenance_plan_report, discover_python_models, find_project_root, init_db, Config,
    ModelDiscovery,
};
use smelt_core::graph::DependencyGraph;

/// Run the real discovery + Salsa pipeline for `project_dir` and return the
/// built maintenance-plan report for `model_name`, mirroring
/// `commands::explain::explain_maintenance_plan`'s own resolution sequence
/// (find_project_root → Config::load → ModelDiscovery → init_db →
/// Workspace/project → compute_scope + resolve_argument → source_file →
/// smelt_db::maintenance_plan_report → build_maintenance_plan_report).
pub(crate) fn build_report_for(project_dir: &Path, model_name: &str) -> Option<String> {
    let project_dir = find_project_root(project_dir).expect("find project root");
    let config = Config::load(&project_dir).expect("load smelt.yml");
    let sources = smelt_cli::SourcesConfig::load(&project_dir).ok();

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let mut models = discovery.discover_models().expect("discover models");

    let python_files = discovery
        .discover_python_files()
        .expect("scan python files");
    if !python_files.is_empty() {
        let python_models = discover_python_models(
            &python_files,
            &models,
            &config,
            &project_dir,
            config.python.as_deref(),
        )
        .expect("discover python models");
        models.extend(python_models);
    }

    let db = init_db(&project_dir, &models);
    let ws = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");
    let project = db
        .project_input(&project_dir)
        .expect("project not initialized");

    let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
    let active_scope = compute_scope(&project_dir, &cwd, &config.paths, None);
    let canonical = resolve_argument(&db, ws, project, active_scope.as_ref(), model_name)
        .unwrap_or_else(|e| panic!("resolve_argument({model_name}): {e}"));

    let model = models
        .iter()
        .find(|m| m.canonical_path() == canonical)
        .unwrap_or_else(|| panic!("model '{canonical}' not found among discovered models"));

    let file = db
        .source_file(&model.path)
        .expect("model file not registered");

    let result = smelt_db::maintenance_plan_report(&db, ws, file)?;

    let graph = DependencyGraph::build(models.clone(), sources.as_ref()).expect("build graph");
    let upstream = graph.get_upstream(&canonical);

    let source_infos = smelt_core::discover_source_infos(&project_dir, &config.paths);
    let (own_contract, edges) =
        smelt_cli::explain::build_relation_contract(model, &models, &upstream, &source_infos);

    let maintenance_cfg = model
        .metadata
        .as_deref()
        .and_then(|m| m.maintenance.as_ref());
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] =
        maintenance_cfg.map(|m| m.cells.as_slice()).unwrap_or(&[]);
    let defaults_cfg = maintenance_cfg.and_then(|m| m.defaults.as_ref());
    let contract_cfg = model.metadata.as_deref().and_then(|m| m.contract.as_ref());

    // This model's own delta signature — the SAME single-owner derivation
    // `commands::explain::explain_maintenance_plan` calls
    // (`docs/outcomes/20260904-delta-signature-front-door/outcome.md` phase
    // 1), so this test helper exercises the real production wiring rather
    // than a stubbed `None`.
    let own_output_delta = smelt_db::model_output_delta_for(&db, ws, file);

    // Mirrors `commands::explain::explain_maintenance_plan`'s own
    // `edge_delta_types` assembly (`docs/outcomes/20260809-output-delta-
    // typing/outcome.md` phase 10) so this test helper exercises the real
    // production wiring rather than a stubbed empty slice.
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

    let bound_ctx = smelt_cli::explain::build_bound_context(&canonical, &graph, &config);
    let profile = smelt_runtime::diagnostics::build_model_profile(
        model,
        &bound_ctx,
        &result.plan.cells,
        &result.column_groups,
        &result.plan.refusals,
        &[],
        contract_cfg,
    )
    .expect("build_model_profile");

    Some(
        build_maintenance_plan_report(
            &canonical,
            &result,
            &own_contract,
            &edges,
            cells_cfg,
            defaults_cfg,
            contract_cfg,
            &source_infos,
            &[],
            smelt_core::config::ProbeCadence::PerRun,
            &edge_delta_types,
            None,
            own_output_delta.as_ref(),
            &profile,
        )
        .expect("build_maintenance_plan_report"),
    )
}

/// Stage a project with a clocked keyed upstream (`dag_kchain_a`, `KeyedAgg`
/// over the append-only `events` source, grouped by `id` — no clock of its
/// own) feeding a keyed-fold downstream (`dag_kchain_b`) — the generated
/// [`smelt_maintenance_testkit::dag::keyed_chain_dag`] fixture phase 6/7/8
/// already prove end-to-end for execution; here it is only staged (no run)
/// to exercise `smelt explain`'s own report.
pub(crate) fn stage_keyed_chain_project(tmp: &tempfile::TempDir) -> std::path::PathBuf {
    use smelt_maintenance_testkit::dag::{keyed_chain_dag, stage_dag};

    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    let dag = keyed_chain_dag();
    stage_dag(&dag, &project_dir, &db_path).expect("stage keyed chain dag");
    project_dir
}

/// Stage a project with: a clocked append-only `events` source; an
/// `undeclared` source with no `mutation_profile`; `windowed_upstream`
/// (refresh: incremental, a window-function output column — an operator the
/// walk cannot classify as addressable); `general_consumer` (reads
/// `windowed_upstream`); `source_consumer` (reads `sources.undeclared`);
/// `view_upstream` (plain view, no `refresh: incremental` at all); and
/// `view_consumer` (reads `view_upstream`) — one project reused across the
/// `general`/absent-verdict edge-typing tests below.
pub(crate) fn stage_delta_type_project(tmp: &tempfile::TempDir) -> std::path::PathBuf {
    let project_dir = tmp.path().join("project");
    std::fs::create_dir_all(project_dir.join("models/sources")).expect("create dirs");

    std::fs::write(
        project_dir.join("smelt.yml"),
        "name: delta_type_fixture\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    schema: main\n\
         default_materialization: view\n",
    )
    .expect("write smelt.yml");

    std::fs::write(
        project_dir.join("models/sources/events.yml"),
        "description: clocked append-only test source\n\
         mutation_profile: append_only\n\
         timeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n\
         columns:\n\
         - name: d\n  type: DATE\n\
         - name: id\n  type: INTEGER\n\
         - name: val\n  type: INTEGER\n",
    )
    .expect("write events.yml");

    std::fs::write(
        project_dir.join("models/sources/undeclared.yml"),
        "description: test source declaring no mutation_profile\n\
         columns:\n\
         - name: d\n  type: DATE\n\
         - name: id\n  type: INTEGER\n\
         - name: val\n  type: INTEGER\n",
    )
    .expect("write undeclared.yml");

    let ts_frontmatter = "---\ntimeseries:\n  event_time_column: d\n  partition_column: d\n  \
                           granularity: day\nrefresh: incremental\ngrain: partition\n---\n";

    std::fs::write(
        project_dir.join("models/windowed_upstream.sql"),
        format!(
            "{ts_frontmatter}SELECT d, id, val, ROW_NUMBER() OVER (PARTITION BY id ORDER BY d) \
             AS rn\nFROM smelt.sources.events\n"
        ),
    )
    .expect("write windowed_upstream.sql");

    std::fs::write(
        project_dir.join("models/general_consumer.sql"),
        format!("{ts_frontmatter}SELECT d, id, val, rn\nFROM smelt.windowed_upstream\n"),
    )
    .expect("write general_consumer.sql");

    std::fs::write(
        project_dir.join("models/source_consumer.sql"),
        format!("{ts_frontmatter}SELECT d, id, val\nFROM smelt.sources.undeclared\n"),
    )
    .expect("write source_consumer.sql");

    std::fs::write(
        project_dir.join("models/view_upstream.sql"),
        "SELECT d, id, val FROM smelt.sources.events\n",
    )
    .expect("write view_upstream.sql");

    std::fs::write(
        project_dir.join("models/view_consumer.sql"),
        format!("{ts_frontmatter}SELECT d, id, val\nFROM smelt.view_upstream\n"),
    )
    .expect("write view_consumer.sql");

    // A straightforward passthrough of a clocked append-only source's own
    // columns, no aggregation and no `unique_key:` — the transfer-rule
    // table's "passthrough of an append-only relation" row, own headline
    // `AppendOnlyWindow` (`docs/outcomes/20260904-delta-signature-front-
    // door/outcome.md` phase 1's `partition_grain_headline_is_window_
    // addressed` test).
    std::fs::write(
        project_dir.join("models/plain_passthrough.sql"),
        format!("{ts_frontmatter}SELECT d, id, val\nFROM smelt.sources.events\n"),
    )
    .expect("write plain_passthrough.sql");

    project_dir
}

pub(crate) const STATE_COLUMNS_SMELT_YML: &str = "name: state_columns_fixture\n\
    version: 1\n\
    paths:\n  - models\n\
    targets:\n  dev:\n    type: duckdb\n    schema: main\n\
    default_materialization: view\n";

pub(crate) const STATE_COLUMNS_EVENTS_SOURCE: &str = "description: events\n\
    mutation_profile: append_only\n\
    timeseries:\n  event_time_column: event_date\n  partition_column: event_date\n  granularity: day\n\
    columns:\n\
    - name: device_id\n  type: INTEGER\n\
    - name: event_date\n  type: DATE\n\
    - name: amount\n  type: DOUBLE\n";
