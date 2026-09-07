use std::path::Path;

use smelt_cli::argument_resolution::{compute_scope, resolve_argument};
use smelt_cli::{
    build_maintenance_plan_report, discover_python_models, find_project_root, init_db, Config,
    ModelDiscovery,
};
use smelt_core::graph::DependencyGraph;

pub(crate) fn synthetic_profile(
    result: &smelt_db::queries::maintenance::MaintenancePlanResult,
    model_name: &str,
) -> smelt_logical::analysis::profile::PropertyProfile {
    let properties = smelt_logical::analysis::profile::PropertySet::derive(
        model_name,
        "SELECT 1 AS c",
        &[],
        &smelt_logical::analysis::source_bounds::BoundContext::default(),
    )
    .expect("PropertySet::derive");
    let contract_points: Vec<smelt_logical::contract::ContractPointView> = result
        .plan
        .cells
        .iter()
        .map(|_| smelt_logical::contract::effective_contract(None, "", &[]).into())
        .collect();
    smelt_logical::analysis::profile::PropertyProfile::assemble(
        properties,
        &result.plan.cells,
        &contract_points,
        &result.plan.refusals,
        &[],
    )
}

/// Run the real discovery + Salsa pipeline for `project_dir` and return the
/// built maintenance-plan report for `model_name` (mirrors
/// `explain_maintenance.rs::build_report_for` /
/// `commands::explain::explain_maintenance_plan`'s resolution sequence).
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

    // Mirrors `commands::explain::explain_maintenance_plan`'s own
    // `edge_delta_types` assembly (`docs/outcomes/20260809-output-delta-
    // typing/outcome.md` phase 10).
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

    let succession_view = result.succession_recipe.as_ref().map(|recipe| {
        smelt_cli::explain::build_succession_explain_view(
            recipe,
            &source_infos,
            &model.db_name_owned(),
        )
    });

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
            None,
            &profile,
            succession_view.as_ref(),
        )
        .expect("build_maintenance_plan_report"),
    )
}
