//! TDD coverage for the shared `smelt-runtime::diagnostics` builder
//! (`docs/plans/20260725-ui-model-diagnostics.md` Phases 2a/2b;
//! `docs/specs/ui_model_diagnostics.md` §Surface "smelt-runtime builder";
//! §Semantics "Technique preview set" / "Admissibility verdict").
//!
//! These tests exercise `build_model_diagnostics`/`build_relation_contract`/
//! `build_plan_cell_diagnostics` against the real `examples/timeseries/`
//! fixture project — no synthetic in-memory model, except where a test
//! needs a specific `RowIdentity`/`Technique` shape a constructed
//! `PlanCell` demonstrates more directly than hunting a fixture for it —
//! per the plan's "real-fixture tests" convention.

use smelt_core::config::{
    CellTechnique, Config, Grain as ConfigGrain, Granularity, MaintenanceCellConfig,
    MaintenanceConfig, Materialization, RefreshStrategy, TimeseriesConfig,
};
use smelt_core::graph::DependencyGraph;
use smelt_core::metadata::ModelMetadata;
use smelt_core::workspace::load_workspace;
use smelt_core::{ModelFile, RefInfo, SmeltRef, SourceInfo};
use smelt_logical::analysis::source_bounds::BoundContext;
use smelt_logical::analysis::walk::Comparability;
use smelt_logical::maintenance::emit::MaintenanceDialect;
use smelt_logical::maintenance::{
    ColumnGroup, Corner, PartitionLocal, PlanCell, RowIdentity, RowIdentityVerdict, Technique,
    Trigger,
};
use smelt_runtime::diagnostics::{
    build_model_diagnostics, build_plan_cell_diagnostics, build_relation_contract, Admissibility,
};
use smelt_runtime::{build_source_timeseries_map, CompilerRegistry, EphemeralResolver};
use std::path::PathBuf;

fn timeseries_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries fixture must exist")
}

/// Load the real `examples/timeseries/` project's models, declared
/// sources, and `smelt.yml` config — the same calls `smelt explain` makes
/// (`crates/smelt-cli/src/commands/explain.rs`), but without any Salsa
/// database: `load_workspace` + `discover_source_infos` are both pure,
/// disk-reading functions.
fn load_fixture() -> (Vec<ModelFile>, Vec<SourceInfo>, Config) {
    let root = timeseries_root();
    let loaded = load_workspace(&root);
    assert!(
        loaded.errors.is_empty(),
        "examples/timeseries must load without errors: {:?}",
        loaded.errors
    );
    let source_infos = smelt_core::discover_source_infos(&root, &loaded.config.paths);
    (loaded.sql_files, source_infos, loaded.config)
}

fn find_model<'a>(models: &'a [ModelFile], name: &str) -> &'a ModelFile {
    models
        .iter()
        .find(|m| m.canonical_path() == name)
        .unwrap_or_else(|| panic!("model `{name}` not found in examples/timeseries"))
}

/// Derive `model`'s real `MaintenancePlan::cells` via the same plain
/// (non-Salsa) entry point `smelt-db`'s own Salsa wrapper calls
/// (`smelt_db::lib::maintenance_plan_report`'s doc comment: "gathers
/// `file`'s referenced sources … then calls `derive_maintenance_plan`") —
/// mirroring its input assembly without a Salsa database, matching this
/// crate's existing Salsa-purity-respecting pattern of taking
/// already-resolved facts. `allow_full_scan: true` for every referenced
/// source keeps this a pure "what would the plan admit" helper: it is not
/// re-testing the K8 partition-locality guardrail, only assembling cells
/// for the technique-preview builder to render.
fn derive_plan_cells(
    model: &ModelFile,
    source_infos: &[SourceInfo],
) -> (Vec<PlanCell>, Vec<ColumnGroup>) {
    let metadata = model
        .metadata
        .as_deref()
        .expect("fixture maintained model must declare frontmatter");
    let stripped_sql = smelt_parser::strip_frontmatter(&model.content).to_string();
    let refs = smelt_logical::collect_path_refs(&stripped_sql);
    let sources: Vec<smelt_logical::maintenance::SourceFacts> = refs
        .iter()
        .filter_map(|r| {
            let stripped = r.strip_prefix("smelt.")?;
            let bare = stripped.strip_prefix("sources.").unwrap_or(stripped);
            let segs: Vec<String> = stripped.split('.').map(|s| s.to_string()).collect();
            let info = source_infos.iter().find(|s| s.address_segments == segs);
            Some(smelt_db::queries::maintenance::source_facts(
                bare, info, true,
            ))
        })
        .collect();
    let table = model
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    smelt_db::queries::maintenance::derive_model_maintenance_plan(
        &stripped_sql,
        &table,
        metadata,
        &sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &std::collections::BTreeMap::new(),
        None,
        None,
    )
    .map(|r| (r.plan.cells, r.column_groups))
    .unwrap_or_default()
}

/// The compilation machinery a technique preview's illustrative SQL is
/// built through — `smelt-cli::explain`'s own `--show-sql` setup, minus
/// anything `--period`-specific (the technique-preview builder always
/// renders symbolic placeholders, `crates/smelt-runtime/src/diagnostics.rs`
/// `build_technique_statements`'s own doc comment).
struct CompileFixture {
    registry: CompilerRegistry,
    resolver: EphemeralResolver,
}

fn compile_fixture(config: &Config) -> CompileFixture {
    let registry = CompilerRegistry::new(config, &config.targets);
    let resolver = registry
        .get("dev")
        .build_ephemeral_resolver(&[], "main")
        .expect("no ephemerals");
    CompileFixture { registry, resolver }
}

/// A `BoundContext` naming every `smelt.sources.*` ref `model` reads, keyed
/// the same way `resolve_table_ref_source_name`/`RefInfo::to_path` name a
/// `smelt.sources.raw.transactions`-style ref: the dot-joined path segments
/// after `smelt.` (`"sources.raw.transactions"`), matching
/// `build_relation_contract`'s own source-edge lookup
/// (`crates/smelt-runtime/src/diagnostics.rs`).
fn bound_ctx_for(model: &ModelFile, source_infos: &[SourceInfo]) -> BoundContext {
    let mut ctx = BoundContext::new();
    for r in &model.refs {
        let segs = r.smelt_ref.to_path();
        if segs.first().map(String::as_str) != Some("sources") {
            continue;
        }
        let Some(info) = source_infos.iter().find(|s| s.address_segments == segs) else {
            continue;
        };
        if let Some(ts) = &info.timeseries {
            ctx.add_source(&segs.join("."), &ts.partition_column);
        }
    }
    ctx
}

/// `docs/specs/model_properties.md` §Surface "Derived proofs": for a real
/// fixture model with a known GROUP BY grain (`daily_revenue` groups by
/// `revenue_date, user_id` and aggregates with `COUNT`/`SUM`/`AVG`/`MIN`/
/// `MAX`), assert every field `PropertySet` actually carries — columns,
/// grain, functional dependencies, determinism, comparability,
/// discriminants, row identity, and source bounds, i.e. everything
/// reachable from the three already-derived per-model calls
/// (`model_property_vector`, `row_identity`, `derive_model_bounds`) — is
/// present and non-trivial on the returned `ModelDiagnostics`.
///
/// This is **not** the full `model_properties.md` catalogue: event-time
/// monotonicity trace, partition alignment, skeleton-role, footprint
/// reflection, faithful-fold conditions, grain-alignment, standalone
/// fingerprint/cardinality derivation, and model-scoped declarations are
/// not yet covered by `PropertySet` (`docs/specs/ui_model_diagnostics.md`
/// §Known Divergences).
#[test]
fn properties_cover_derivable_catalogue_subset() {
    let (models, source_infos, config) = load_fixture();
    let model = find_model(&models, "daily_revenue");
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let upstream = graph.get_upstream(&model.canonical_path());
    let bound_ctx = bound_ctx_for(model, &source_infos);
    let cf = compile_fixture(&config);
    let source_timeseries = build_source_timeseries_map(&graph, &source_infos);

    // `daily_revenue` declares no frontmatter (no maintenance plan) — this
    // test only exercises the property-set half of the builder.
    let diagnostics = build_model_diagnostics(
        model,
        &models,
        &upstream,
        &source_infos,
        &bound_ctx,
        &[],
        "main",
        "dev",
        &cf.registry,
        &cf.resolver,
        MaintenanceDialect::DuckDb,
        &source_timeseries,
        &[],
        &[],
        &[],
        &[],
        None,
    )
    .expect("diagnostics build succeeds for a real fixture model");

    let props = &diagnostics.profile.properties;

    // Columns: every projected output column.
    assert!(
        props.columns.contains(&"revenue_date".to_string())
            && props.columns.contains(&"user_id".to_string())
            && props.columns.contains(&"total_revenue".to_string()),
        "columns should list the model's projected outputs, got {:?}",
        props.columns
    );

    // Grain: `GROUP BY 1, 2` proves a `(revenue_date, user_id)` key.
    assert!(
        !props.grain.keys.is_empty(),
        "grain should be proven from the model's GROUP BY 1, 2"
    );

    // Functional dependencies: grain implies key -> every other column.
    assert!(
        !props.functional_dependencies.is_empty(),
        "functional dependencies should be implied by the proven grain"
    );

    // Determinism: every projected column gets a verdict.
    assert_eq!(
        props.determinism.len(),
        props.columns.len(),
        "every column should carry a determinism verdict"
    );

    // Comparability: every projected column gets a verdict, and a plain
    // aggregate output (`total_revenue`) is `Comparable` (pure function of
    // processed inputs).
    assert_eq!(
        props.comparability.len(),
        props.columns.len(),
        "every column should carry a comparability verdict"
    );
    let total_revenue_comparability = props
        .comparability
        .iter()
        .find(|c| c.output == "total_revenue")
        .expect("total_revenue should have a comparability verdict");
    assert_eq!(
        total_revenue_comparability.comparability,
        Comparability::Comparable
    );

    // Discriminants: the five aggregate outputs (COUNT/SUM/AVG/MIN/MAX) each
    // carry algebraic discriminants.
    assert_eq!(
        props.discriminants.len(),
        5,
        "each of COUNT/SUM/AVG/MIN/MAX should carry discriminants, got {:?}",
        props.discriminants
    );

    // Region row identity: no declared unique_key on this model, so the
    // proven GROUP BY grain becomes the row identity.
    assert!(
        matches!(props.row_identity.identity, RowIdentity::Key(_)),
        "row identity should fall back to the proven grain key, got {:?}",
        props.row_identity
    );

    // Unified bound/reach: the model's one timeseries source
    // (`sources.raw.transactions`) should have a bound-derivation entry.
    assert!(
        props.source_bounds.contains_key("sources.raw.transactions"),
        "source_bounds should carry an entry for the model's timeseries source, got {:?}",
        props.source_bounds.keys().collect::<Vec<_>>()
    );
}

/// `docs/specs/ui_model_diagnostics.md` §Design "Why a shared smelt-runtime
/// builder": `build_relation_contract` is now derived exactly once, in this
/// crate — `smelt-cli::explain::build_relation_contract` is a `pub use`
/// re-export of the very same function, not a second implementation. This
/// test pins the exact shape produced for a real fixture model with a
/// declared source edge, guarding against a future accidental fork.
#[test]
fn relation_contract_matches_existing_explain_output() {
    let (models, source_infos, _config) = load_fixture();
    let model = find_model(&models, "daily_revenue");
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let upstream = graph.get_upstream(&model.canonical_path());

    let (own_contract, edges) = build_relation_contract(model, &models, &upstream, &source_infos);

    // `daily_revenue` declares no `timeseries:`/`unique_key` of its own.
    assert!(own_contract.clock.is_none());
    assert!(own_contract.identity.is_none());

    // Its one inbound edge is the declared `sources.raw.transactions`
    // source, which carries a `timeseries:` clock (`transaction_timestamp`,
    // day granularity).
    let edge = edges
        .iter()
        .find(|e| e.name == "sources.raw.transactions")
        .expect("daily_revenue should have a sources.raw.transactions inbound edge");
    let clock = edge
        .contract
        .clock
        .as_ref()
        .expect("the transactions source declares a timeseries clock");
    assert_eq!(clock.event_time_column, "transaction_timestamp");
    assert_eq!(clock.partition_column, "transaction_timestamp");

    // `smelt-cli::explain::build_relation_contract` is a `pub use`
    // re-export of this exact function (not a second implementation) —
    // calling it on the same inputs must produce an identical result,
    // byte-for-byte.
    let (cli_own_contract, cli_edges) =
        smelt_cli::explain::build_relation_contract(model, &models, &upstream, &source_infos);
    assert_eq!(own_contract, cli_own_contract);
    assert_eq!(edges, cli_edges);
}

/// `docs/specs/ui_model_diagnostics.md` §Constraints: "must not require a
/// live backend connection or ledger state". `build_model_diagnostics`'s
/// signature takes no target/backend/connection argument at all — this test
/// documents and locks that shape by calling it with only disk-derived
/// facts and asserting success.
#[test]
fn no_live_backend_required() {
    let (models, source_infos, config) = load_fixture();
    let model = find_model(&models, "daily_revenue");
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let upstream = graph.get_upstream(&model.canonical_path());
    let bound_ctx = bound_ctx_for(model, &source_infos);
    let cf = compile_fixture(&config);
    let source_timeseries = build_source_timeseries_map(&graph, &source_infos);

    let result = build_model_diagnostics(
        model,
        &models,
        &upstream,
        &source_infos,
        &bound_ctx,
        &[],
        "main",
        "dev",
        &cf.registry,
        &cf.resolver,
        MaintenanceDialect::DuckDb,
        &source_timeseries,
        &[],
        &[],
        &[],
        &[],
        None,
    );
    assert!(
        result.is_ok(),
        "diagnostics build must succeed without any live backend/target: {:?}",
        result.err()
    );
}

// ---------------------------------------------------------------------
// Phase 2b: technique previews + admissibility
// (`docs/plans/20260725-ui-model-diagnostics.md` §"Phase 2b";
// `docs/specs/ui_model_diagnostics.md` §Semantics "Technique preview set" /
// "Admissibility verdict")
// ---------------------------------------------------------------------

/// A synthetic `PlanCell` for tests that need a specific `RowIdentity`/
/// `Technique` shape more directly than hunting a fixture for it — mirrors
/// `smelt_logical::maintenance::choice`'s own test helpers
/// (`admitted_plan` in `choice.rs`'s `#[cfg(test)] mod tests`).
fn synthetic_cell(technique: Technique, row_identity: RowIdentity) -> PlanCell {
    PlanCell {
        group: "{status}".to_string(),
        trigger: smelt_logical::maintenance::Trigger::UpstreamMutation {
            source: "raw.user_status".to_string(),
        },
        corner: smelt_logical::maintenance::Corner::ColumnMerge,
        technique,
        partition_local: smelt_logical::maintenance::PartitionLocal::Yes,
        scans: vec![],
        ledger_catch_up: false,
        row_identity: smelt_logical::maintenance::RowIdentityVerdict {
            identity: row_identity,
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: Default::default(),
        key_scope: None,
        state_downgrade: None,
    }
}

/// `docs/specs/ui_model_diagnostics.md` §Constraints: "the technique-preview
/// builder must call the same pure emitters … that a live run uses for the
/// admitted technique — a preview's statements for the `Admitted` entry
/// must be identical to what `incremental_models.md`'s 'Statement emission
/// (single owner)' rule already guarantees a run executes." `daily_cube_
/// metrics`'s single creation cell (`Trigger::NewData`, `Technique::
/// DeleteInsert`) always builds cleanly (a declared `timeseries.
/// partition_column`, no `unique_key`/keyed-fold shape needed) — its
/// `Admitted` preview must render real, non-empty statements, and
/// `smelt-cli::explain::build_admitted_statement_group` — the CLI's own
/// thin reader over this same entry — must reproduce it byte-identically,
/// substituting real `--period` literals for the builder's symbolic
/// `{{window_start}}`/`{{window_end}}` placeholders when a concrete period
/// is given (`smelt-cli` no longer keeps a second, independent derivation
/// for this cell — `docs/plans/20260725-ui-model-diagnostics.md`).
#[test]
fn admitted_preview_matches_live_run_statements() {
    let (models, source_infos, config) = load_fixture();
    let model = find_model(&models, "daily_cube_metrics");
    let cf = compile_fixture(&config);
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let source_timeseries = build_source_timeseries_map(&graph, &source_infos);
    let (cells, column_groups) = derive_plan_cells(model, &source_infos);
    let cell = cells
        .iter()
        .find(|c| c.technique == Technique::DeleteInsert)
        .expect("daily_cube_metrics must admit a DeleteInsert creation cell");

    let diagnostics = build_plan_cell_diagnostics(
        cell,
        model,
        "main",
        "dev",
        &cf.registry,
        &cf.resolver,
        MaintenanceDialect::DuckDb,
        &[],
        &source_timeseries,
        &column_groups,
    );

    let admitted = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.admissibility == Admissibility::Admitted)
        .expect("exactly one preview must be Admitted");
    assert_eq!(admitted.technique, Technique::DeleteInsert);
    assert!(
        !admitted.statements.is_empty(),
        "the Admitted DeleteInsert preview must render real statements"
    );

    // No `--period`: the CLI's reader must reproduce the preview's own
    // symbolic-placeholder statements verbatim, byte-identical.
    let placeholders = smelt_cli::explain::build_admitted_statement_group(
        &diagnostics,
        &smelt_cli::explain::RegionLiterals::Placeholders,
    )
    .expect("smelt-cli's reader must succeed for an Admitted preview with real statements");
    let admitted_sql: Vec<&str> = admitted.statements.iter().map(|s| s.sql.as_str()).collect();
    let placeholder_sql: Vec<&str> = placeholders
        .statements
        .iter()
        .map(|s| s.sql.as_str())
        .collect();
    assert_eq!(
        admitted_sql, placeholder_sql,
        "smelt-cli::explain::build_admitted_statement_group must reproduce the shared \
         builder's own Admitted preview statements byte-identically when no --period is given"
    );
    assert_eq!(admitted.transactional, placeholders.transactional);

    // A concrete `--period`: every `{{window_start}}`/`{{window_end}}`
    // token in the preview's own statements must be replaced by the real
    // literal, nothing else touched.
    let with_period = smelt_cli::explain::build_admitted_statement_group(
        &diagnostics,
        &smelt_cli::explain::RegionLiterals::Period {
            start: "2024-01-01".to_string(),
            end: "2024-01-03".to_string(),
        },
    )
    .expect("smelt-cli's reader must succeed under a concrete --period");
    for stmt in &with_period.statements {
        assert!(
            !stmt.sql.contains("{{window_start}}") && !stmt.sql.contains("{{window_end}}"),
            "expected the real --period literals substituted in, no placeholders left: {}",
            stmt.sql
        );
    }
    assert!(
        with_period
            .statements
            .iter()
            .any(|s| s.sql.contains("2024-01-01") && s.sql.contains("2024-01-03")),
        "expected the real --period literals present in the substituted statements: {:?}",
        with_period.statements
    );
}

/// `docs/specs/ui_model_diagnostics.md` §Semantics "Admissibility verdict":
/// "Region recompute is always `InterchangeableAlternative` when not itself
/// the admitted technique". A cell whose admitted technique is
/// `ColumnScopedMerge` still has a declared timeseries partition axis, so
/// its `DeleteInsert` preview always builds — and must never be `Admitted`
/// or `NotApplicable` here.
#[test]
fn recompute_is_always_interchangeable_when_not_admitted() {
    let (models, source_infos, config) = load_fixture();
    let model = find_model(&models, "daily_cube_metrics");
    let cf = compile_fixture(&config);
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let source_timeseries = build_source_timeseries_map(&graph, &source_infos);

    // A synthetic cell borrowing `daily_cube_metrics`'s own model (which
    // declares a `timeseries.partition_column`, so `DeleteInsert` always
    // builds) but a different admitted technique, so the `DeleteInsert`
    // preview is never the `Admitted` entry.
    let cell = synthetic_cell(
        Technique::ColumnScopedMerge,
        RowIdentity::Key(vec!["k".into()]),
    );

    let diagnostics = build_plan_cell_diagnostics(
        &cell,
        model,
        "main",
        "dev",
        &cf.registry,
        &cf.resolver,
        MaintenanceDialect::DuckDb,
        &["k".to_string()],
        &source_timeseries,
        &[],
    );

    let delete_insert_preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::DeleteInsert)
        .expect("every_known_technique_has_an_entry: DeleteInsert must have an entry");
    assert_eq!(
        delete_insert_preview.admissibility,
        Admissibility::InterchangeableAlternative,
        "region recompute must be InterchangeableAlternative when not the admitted technique, \
         got {:?}",
        delete_insert_preview.admissibility
    );
    assert!(
        !delete_insert_preview.statements.is_empty(),
        "an InterchangeableAlternative preview must still carry real illustrative SQL"
    );
}

/// `docs/specs/ui_model_diagnostics.md` §Semantics "Admissibility verdict":
/// a `NotApplicable` preview must carry a non-empty `reason` and must still
/// show illustrative SQL where the emitter can render it. A cell with
/// `RowIdentity::WholeRow` cannot structurally support a keyed-fold's
/// per-row addressing — `daily_events_status`'s real `{status}` cell is
/// exactly this shape (its own doc comment: "this cell's own row identity
/// resolves `WholeRow`").
#[test]
fn not_applicable_carries_reason() {
    let (models, source_infos, config) = load_fixture();
    let model = find_model(&models, "daily_events_status");
    let cf = compile_fixture(&config);
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let source_timeseries = build_source_timeseries_map(&graph, &source_infos);
    let (cells, column_groups) = derive_plan_cells(model, &source_infos);
    let cell = cells
        .iter()
        .find(|c| matches!(c.row_identity.identity, RowIdentity::WholeRow))
        .expect("daily_events_status must have a WholeRow-identity cell");

    let diagnostics = build_plan_cell_diagnostics(
        cell,
        model,
        "main",
        "dev",
        &cf.registry,
        &cf.resolver,
        MaintenanceDialect::DuckDb,
        &[],
        &source_timeseries,
        &column_groups,
    );

    let keyed_fold_preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::KeyedFold)
        .expect("KeyedFold must have an entry");
    match &keyed_fold_preview.admissibility {
        Admissibility::NotApplicable { reason } => {
            assert!(
                !reason.is_empty(),
                "a NotApplicable preview must always carry a non-empty reason"
            );
        }
        other => panic!("expected NotApplicable for a WholeRow-identity cell, got {other:?}"),
    }
}

/// `docs/specs/ui_model_diagnostics.md` §Semantics "Admissibility verdict":
/// "Exactly one preview entry per cell is `Admitted`." Checked across every
/// cell of several real fixture models with distinct admitted-technique
/// shapes.
#[test]
fn exactly_one_admitted_per_cell() {
    let (models, source_infos, config) = load_fixture();
    let cf = compile_fixture(&config);
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let source_timeseries = build_source_timeseries_map(&graph, &source_infos);

    for model_name in [
        "daily_cube_metrics",
        "daily_events_enriched",
        "daily_events_status",
    ] {
        let model = find_model(&models, model_name);
        let (cells, column_groups) = derive_plan_cells(model, &source_infos);
        assert!(
            !cells.is_empty(),
            "{model_name} must derive at least one maintenance-plan cell"
        );
        for cell in &cells {
            let diagnostics = build_plan_cell_diagnostics(
                cell,
                model,
                "main",
                "dev",
                &cf.registry,
                &cf.resolver,
                MaintenanceDialect::DuckDb,
                &[],
                &source_timeseries,
                &column_groups,
            );
            let admitted_count = diagnostics
                .technique_previews
                .iter()
                .filter(|p| p.admissibility == Admissibility::Admitted)
                .count();
            assert_eq!(
                admitted_count, 1,
                "{model_name} cell {:?} must have exactly one Admitted preview, got {}: {:?}",
                diagnostics.group, admitted_count, diagnostics.technique_previews
            );
        }
    }
}

/// `docs/specs/ui_model_diagnostics.md` §Semantics "Technique preview set":
/// "never partial by omission" — every cell's preview set carries one entry
/// per technique the emitters implement, regardless of the cell's own
/// admitted technique.
#[test]
fn every_known_technique_has_an_entry() {
    let (models, source_infos, config) = load_fixture();
    let model = find_model(&models, "daily_events_enriched");
    let cf = compile_fixture(&config);
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let source_timeseries = build_source_timeseries_map(&graph, &source_infos);
    let (cells, column_groups) = derive_plan_cells(model, &source_infos);
    let cell = cells.first().expect("must have at least one cell");

    let diagnostics = build_plan_cell_diagnostics(
        cell,
        model,
        "main",
        "dev",
        &cf.registry,
        &cf.resolver,
        MaintenanceDialect::DuckDb,
        &[],
        &source_timeseries,
        &column_groups,
    );

    let techniques: std::collections::BTreeSet<String> = diagnostics
        .technique_previews
        .iter()
        .map(|p| format!("{:?}", p.technique))
        .collect();
    assert_eq!(
        techniques,
        std::collections::BTreeSet::from([
            "DeleteInsert".to_string(),
            "KeyedFold".to_string(),
            "ColumnScopedMerge".to_string(),
            "InPlaceUpdate".to_string(),
            "PerGroupRecompute".to_string(),
        ]),
        "every known technique must have exactly one preview entry, got {:?}",
        techniques
    );
}

/// `docs/plans/20260725-ui-model-diagnostics.md` §"Phase 2b" required
/// regression guard: `smelt_logical::maintenance::choice::resolve_cell_choice`
/// is unaffected by this phase's additions (the new `technique_requires_
/// row_identity` classifier is read-only and additive, consulted only by
/// the `smelt-runtime` technique-preview builder, never by `resolve_cell_
/// choice` itself). Mirrors `choice.rs`'s own `pin_bypasses_cost_model_but_
/// not_admission` test scenario as a pinned-output guard.
#[test]
fn choice_rs_execution_semantics_unchanged() {
    use smelt_core::config::CellTechnique;
    use smelt_logical::maintenance::choice::{
        resolve_cell_choice, ChosenTechnique, EffectiveOverride,
    };
    use smelt_logical::maintenance::{Corner, MaintenancePlan, PartitionLocal, Trigger};

    let plan = MaintenancePlan {
        cells: vec![PlanCell {
            group: "{tier}".to_string(),
            trigger: Trigger::UpstreamMutation {
                source: "users".to_string(),
            },
            corner: Corner::ColumnMerge,
            technique: Technique::ColumnScopedMerge,
            partition_local: PartitionLocal::Yes,
            scans: vec![],
            ledger_catch_up: false,
            row_identity: smelt_logical::maintenance::RowIdentityVerdict {
                identity: RowIdentity::WholeRow,
                proven_mismatch: None,
            },
            skeleton_source_closure: None,
            fingerprint_projections: Default::default(),
            key_scope: None,
            state_downgrade: None,
        }],
        refusals: vec![],
        key_locality: None,
    };
    let trigger = Trigger::UpstreamMutation {
        source: "users".to_string(),
    };

    // No override: an admitted+live technique is preferred over recompute.
    let resolved = resolve_cell_choice(
        plan.cell_for(&trigger),
        &trigger,
        &EffectiveOverride::default(),
        None,
        true,
    )
    .expect("no pin — never refuses");
    assert_eq!(
        resolved,
        ChosenTechnique::Admitted(Technique::ColumnScopedMerge)
    );

    // A pin naming a technique this cell did not admit still refuses —
    // unaffected by the technique-preview builder's wider display-only set.
    let bad_overrides = EffectiveOverride {
        prefer: None,
        technique: Some(CellTechnique::Fold),
    };
    let err = resolve_cell_choice(
        plan.cell_for(&trigger),
        &trigger,
        &bad_overrides,
        None,
        true,
    )
    .expect_err("pinning an unadmitted technique must still refuse");
    assert!(err.to_string().contains("MaintenanceUnboundedFootprint"));

    // `recompute` remains always resolvable.
    let recompute_overrides = EffectiveOverride {
        prefer: None,
        technique: Some(CellTechnique::Recompute),
    };
    let resolved = resolve_cell_choice(
        plan.cell_for(&trigger),
        &trigger,
        &recompute_overrides,
        None,
        true,
    )
    .expect("recompute is always resolvable");
    assert_eq!(resolved, ChosenTechnique::RegionRecompute);
}

/// The repair family's technique preview (`docs/specs/incremental_models.md`
/// §"The repair family"): an admitted `Technique::PerGroupRecompute` cell
/// renders real, illustrative statements built by the SAME emitter +
/// affected-key/candidate builders a live run uses
/// (`smelt_runtime::maintenance_driver::repair_affected_keys_select`/
/// `repair_candidate_select` → `emit_per_group_recompute`) — not the
/// "no live statement builder yet" refusal that stood in for it while the
/// family had no runtime lowering.
#[test]
fn per_group_recompute_preview_renders_statements_for_an_admitted_repair_cell() {
    let (models, source_infos, config) = load_fixture();
    let model = find_model(&models, "daily_events_status");
    let cf = compile_fixture(&config);
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let source_timeseries = build_source_timeseries_map(&graph, &source_infos);

    // A repair cell's shape, verbatim from
    // `smelt_logical::maintenance::repair::derive_repair_cell`: a proven
    // group key plus the bounded per-group read slice.
    let mut cell = synthetic_cell(
        Technique::PerGroupRecompute,
        RowIdentity::Key(vec!["user_id".to_string()]),
    );
    cell.scans = vec![smelt_logical::maintenance::ScanClamp {
        source: "raw.user_status".to_string(),
        column: "changed_at".to_string(),
        before: smelt_logical::analysis::source_bounds::Seconds::days(1),
        after: smelt_logical::analysis::source_bounds::Seconds::ZERO,
        write_footprint: None,
    }];

    let diagnostics = build_plan_cell_diagnostics(
        &cell,
        model,
        "main",
        "dev",
        &cf.registry,
        &cf.resolver,
        MaintenanceDialect::DuckDb,
        &["user_id".to_string()],
        &source_timeseries,
        &[],
    );

    let preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::PerGroupRecompute)
        .expect("every technique has a preview entry");
    assert_eq!(
        preview.admissibility,
        Admissibility::Admitted,
        "the cell's own admitted technique must render as Admitted, got {:?}",
        preview.admissibility
    );
    assert!(
        !preview.statements.is_empty(),
        "the repair preview must render the emitter's statement group, not an empty refusal"
    );
    assert!(
        preview.transactional,
        "the repair group's DELETE+INSERT pair is transactional"
    );
    let joined = preview
        .statements
        .iter()
        .map(|s| s.sql.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("DELETE FROM main.daily_events_status USING")
            && joined.contains("__smelt_affected"),
        "the repair's DELETE must be restricted to the affected-key relation: {joined}"
    );
    assert!(
        joined.contains("main.sources_raw_user_status") && joined.contains("changed_at >= "),
        "the affected-key read must name the mutated source and carry the cell's clamp: {joined}"
    );
}

// ---------------------------------------------------------------------
// Phase 27a: `--show-sql` previews the change-suppressed matched arm
// (`docs/outcomes/20260815-definition-delta-migrate/phases/27a-plan.md`;
// `docs/specs/incremental_models.md` §"Statement emission (single owner)")
// ---------------------------------------------------------------------

/// A minimal synthetic `ColumnScopedMerge` model: a fact joined to a
/// mutable dimension, projecting `expr_sql AS user_name`. Controlling the
/// projected expression lets each test choose whether `user_name` is P3
/// change-comparable (a bare column reference) or not (`RANDOM()`, a
/// row-nondeterministic function — `walk.rs::expr_comparability`'s own
/// fail-closed case).
fn merge_probe_model(expr_sql: &str) -> ModelFile {
    let content = format!(
        "SELECT e.event_id, e.user_id, {expr_sql} AS user_name FROM smelt.sources.raw.events e \
         JOIN smelt.sources.raw.users u ON e.user_id = u.user_id"
    );
    let path: PathBuf = "merge_probe.sql".into();
    ModelFile {
        name: "merge_probe".to_string(),
        model_id: smelt_core::ModelId::from_path(path.clone()),
        path,
        content,
        refs: vec![
            RefInfo {
                has_named_params: false,
                range: Default::default(),
                smelt_ref: SmeltRef::Path(vec![
                    "sources".to_string(),
                    "raw".to_string(),
                    "events".to_string(),
                ]),
            },
            RefInfo {
                has_named_params: false,
                range: Default::default(),
                smelt_ref: SmeltRef::Path(vec![
                    "sources".to_string(),
                    "raw".to_string(),
                    "users".to_string(),
                ]),
            },
        ],
        parse_errors: Vec::new(),
        metadata: Some(Box::new(ModelMetadata {
            materialization: Some(Materialization::Table),
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(ConfigGrain::Partition),
            timeseries: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            ..Default::default()
        })),
        kind: smelt_core::ModelKind::Sql,
        address_segments: vec!["merge_probe".to_string()],
    }
}

/// A synthetic `Technique::ColumnScopedMerge` cell over `merge_probe_model`'s
/// `{user_name}` column group, keyed on `event_id`.
fn merge_probe_cell(trigger: Trigger, ledger_catch_up: bool) -> PlanCell {
    PlanCell {
        group: "{user_name}".to_string(),
        trigger,
        corner: Corner::ColumnMerge,
        technique: Technique::ColumnScopedMerge,
        partition_local: PartitionLocal::Yes,
        scans: vec![],
        ledger_catch_up,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::Key(vec!["event_id".to_string()]),
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: Default::default(),
        key_scope: None,
        state_downgrade: None,
    }
}

fn merge_probe_column_groups() -> Vec<ColumnGroup> {
    vec![ColumnGroup {
        columns: vec!["user_name".to_string()],
        mutation_sensitivity: Default::default(),
        membership_sensitivity: Default::default(),
    }]
}

/// `27a-plan.md`'s `column_scoped_merge_preview_renders_the_suppressed_matched_arm`:
/// a suppressible `ColumnScopedMerge` cell's preview statements carry the
/// `IS DISTINCT FROM` matched-arm guard — `user_name` is a bare pass-through
/// column (P3 `Comparable`), the row identity is a proven key (P2), and the
/// steady-state `UpstreamMutation` trigger over prior state (`ledger_catch_up:
/// false`) is the structural default `resolve_write_variant` prefers.
#[test]
fn column_scoped_merge_preview_renders_the_suppressed_matched_arm() {
    let model = merge_probe_model("u.user_name");
    let cell = merge_probe_cell(
        Trigger::UpstreamMutation {
            source: "raw.users".to_string(),
        },
        false,
    );
    let column_groups = merge_probe_column_groups();
    let config = load_fixture().2;
    let registry = CompilerRegistry::new(&config, &config.targets);
    let resolver = registry
        .get("dev")
        .build_ephemeral_resolver(&[], "main")
        .expect("no ephemerals");
    let source_timeseries = smelt_planner::SourceTimeseriesMap::new();

    let diagnostics = build_plan_cell_diagnostics(
        &cell,
        &model,
        "main",
        "dev",
        &registry,
        &resolver,
        MaintenanceDialect::DuckDb,
        &["event_id".to_string()],
        &source_timeseries,
        &column_groups,
    );

    let preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::ColumnScopedMerge)
        .expect("ColumnScopedMerge must have a preview entry");
    assert_eq!(preview.admissibility, Admissibility::Admitted);
    let sql = preview
        .statements
        .first()
        .expect("the Admitted ColumnScopedMerge preview must render a statement")
        .sql
        .clone();
    assert!(
        sql.contains("IS DISTINCT FROM") && sql.contains("user_name"),
        "expected the change-suppressed matched-arm guard over `user_name`: {sql}"
    );
}

/// `27a-plan.md`'s `first_build_cell_preview_keeps_the_unconditional_matched_arm`:
/// a cell with no prior stored state to diff against (`Trigger::Backfill`)
/// resolves the unconditional matched arm by default, matching
/// `resolve_write_variant`'s `FirstBuildPosture` branch — even though the
/// P2/P3 proof itself is admitted.
#[test]
fn first_build_cell_preview_keeps_the_unconditional_matched_arm() {
    let model = merge_probe_model("u.user_name");
    let cell = merge_probe_cell(Trigger::Backfill, false);
    let column_groups = merge_probe_column_groups();
    let config = load_fixture().2;
    let registry = CompilerRegistry::new(&config, &config.targets);
    let resolver = registry
        .get("dev")
        .build_ephemeral_resolver(&[], "main")
        .expect("no ephemerals");
    let source_timeseries = smelt_planner::SourceTimeseriesMap::new();

    let diagnostics = build_plan_cell_diagnostics(
        &cell,
        &model,
        "main",
        "dev",
        &registry,
        &resolver,
        MaintenanceDialect::DuckDb,
        &["event_id".to_string()],
        &source_timeseries,
        &column_groups,
    );

    let preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::ColumnScopedMerge)
        .expect("ColumnScopedMerge must have a preview entry");
    let sql = preview
        .statements
        .first()
        .expect("the Admitted ColumnScopedMerge preview must render a statement")
        .sql
        .clone();
    assert!(
        !sql.contains("IS DISTINCT FROM"),
        "a first-build cell (no prior stored state) must keep the unconditional matched arm: {sql}"
    );
}

/// `27a-plan.md`'s `incomparable_group_preview_keeps_the_unconditional_matched_arm`:
/// a P3 refusal (a row-nondeterministic `RANDOM()` projection) renders the
/// plain unconditional arm, even over a steady-state trigger with prior
/// state.
#[test]
fn incomparable_group_preview_keeps_the_unconditional_matched_arm() {
    let model = merge_probe_model("RANDOM()");
    let cell = merge_probe_cell(
        Trigger::UpstreamMutation {
            source: "raw.users".to_string(),
        },
        false,
    );
    let column_groups = merge_probe_column_groups();
    let config = load_fixture().2;
    let registry = CompilerRegistry::new(&config, &config.targets);
    let resolver = registry
        .get("dev")
        .build_ephemeral_resolver(&[], "main")
        .expect("no ephemerals");
    let source_timeseries = smelt_planner::SourceTimeseriesMap::new();

    let diagnostics = build_plan_cell_diagnostics(
        &cell,
        &model,
        "main",
        "dev",
        &registry,
        &resolver,
        MaintenanceDialect::DuckDb,
        &["event_id".to_string()],
        &source_timeseries,
        &column_groups,
    );

    let preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::ColumnScopedMerge)
        .expect("ColumnScopedMerge must have a preview entry");
    let sql = preview
        .statements
        .first()
        .expect("the Admitted ColumnScopedMerge preview must render a statement")
        .sql
        .clone();
    assert!(
        !sql.contains("IS DISTINCT FROM"),
        "an incomparable compared column must fail closed to the unconditional matched arm: {sql}"
    );
}

/// `27a-plan.md`'s `suppress_pin_over_a_refused_proof_yields_no_preview_statements`:
/// a `technique: suppress` pin whose write-suppression proof refused (P3
/// incomparable) surfaces as a build error — empty statements, non-`Admitted`
/// admissibility — never a silent unconditional fallback (a live run over
/// this same cell would hard-fail the whole run, `maintenance_driver.rs`'s
/// own `ChoiceRefusal` propagation).
#[test]
fn suppress_pin_over_a_refused_proof_yields_no_preview_statements() {
    let mut model = merge_probe_model("RANDOM()");
    model.metadata.as_mut().unwrap().maintenance = Some(MaintenanceConfig {
        defaults: None,
        cells: vec![MaintenanceCellConfig {
            columns: vec!["user_name".to_string()],
            on: "raw.users".to_string(),
            prefer: None,
            technique: Some(CellTechnique::Suppress),
            write: None,
        }],
        scan_bounds: None,
    });
    let cell = merge_probe_cell(
        Trigger::UpstreamMutation {
            source: "raw.users".to_string(),
        },
        false,
    );
    let column_groups = merge_probe_column_groups();
    let config = load_fixture().2;
    let registry = CompilerRegistry::new(&config, &config.targets);
    let resolver = registry
        .get("dev")
        .build_ephemeral_resolver(&[], "main")
        .expect("no ephemerals");
    let source_timeseries = smelt_planner::SourceTimeseriesMap::new();

    let diagnostics = build_plan_cell_diagnostics(
        &cell,
        &model,
        "main",
        "dev",
        &registry,
        &resolver,
        MaintenanceDialect::DuckDb,
        &["event_id".to_string()],
        &source_timeseries,
        &column_groups,
    );

    let preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::ColumnScopedMerge)
        .expect("ColumnScopedMerge must have a preview entry");
    assert!(
        preview.statements.is_empty(),
        "a refused suppress pin must yield no preview statements: {:?}",
        preview.statements
    );
    match &preview.admissibility {
        Admissibility::NotApplicable { reason } => {
            assert!(
                !reason.is_empty(),
                "the refusal reason must be surfaced, not empty"
            );
        }
        other => panic!(
            "expected NotApplicable for a technique:suppress pin over a refused proof, got {other:?}"
        ),
    }
}

/// `27a-plan.md`'s `keyed_fold_preview_renders_the_suppressed_matched_arm`:
/// a suppressible `KeyedFold` cell's preview statements carry the guard
/// comparing the stored value against the fold's own combine expression —
/// `total_amount` (a plain `SUM`) is P3 `Comparable`
/// (`walk.rs::expr_comparability`: a registry-backed, non-nondeterministic
/// aggregate taints nothing) and the derived key is a proven `RowIdentity::Key`.
#[test]
fn keyed_fold_preview_renders_the_suppressed_matched_arm() {
    let content =
        "SELECT device_id, SUM(amount) AS total_amount FROM smelt.events GROUP BY device_id";
    let path: PathBuf = "device_total.sql".into();
    let model = ModelFile {
        name: "device_total".to_string(),
        model_id: smelt_core::ModelId::from_path(path.clone()),
        path,
        content: content.to_string(),
        refs: vec![RefInfo {
            has_named_params: false,
            range: Default::default(),
            smelt_ref: SmeltRef::Path(vec!["events".to_string()]),
        }],
        parse_errors: Vec::new(),
        metadata: Some(Box::new(ModelMetadata {
            materialization: Some(Materialization::Table),
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(ConfigGrain::Key),
            ..Default::default()
        })),
        kind: smelt_core::ModelKind::Sql,
        address_segments: vec!["device_total".to_string()],
    };
    let metadata = model.metadata.as_deref().unwrap();
    let stripped_sql = smelt_parser::strip_frontmatter(&model.content).to_string();

    let sources = vec![smelt_logical::maintenance::SourceFacts {
        name: "events".to_string(),
        mutation: smelt_logical::maintenance::MutationProfile::AppendOnly,
        partition_col: Some("event_date".to_string()),
        unique_key: vec![],
        allow_full_scan: true,
    }];
    let plan_result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        &stripped_sql,
        "device_total",
        metadata,
        &sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &std::collections::BTreeMap::new(),
        None,
        None,
    )
    .expect("device_total must derive a maintenance plan");
    let cell = plan_result
        .plan
        .cells
        .iter()
        .find(|c| c.technique == Technique::KeyedFold)
        .expect("device_total must admit a KeyedFold cell");

    let mut source_timeseries = smelt_planner::SourceTimeseriesMap::new();
    source_timeseries.insert(
        "smelt.events".to_string(),
        TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        },
    );

    let config = load_fixture().2;
    let registry = CompilerRegistry::new(&config, &config.targets);
    let resolver = registry
        .get("dev")
        .build_ephemeral_resolver(&[], "main")
        .expect("no ephemerals");

    let diagnostics = build_plan_cell_diagnostics(
        cell,
        &model,
        "main",
        "dev",
        &registry,
        &resolver,
        MaintenanceDialect::DuckDb,
        &[],
        &source_timeseries,
        &plan_result.column_groups,
    );

    let preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::KeyedFold)
        .expect("KeyedFold must have a preview entry");
    assert_eq!(preview.admissibility, Admissibility::Admitted);
    let sql = preview
        .statements
        .first()
        .expect("the Admitted KeyedFold preview must render a statement")
        .sql
        .clone();
    assert!(
        sql.contains("IS DISTINCT FROM"),
        "expected the change-suppressed matched-arm guard over `total_amount`: {sql}"
    );
}

/// `docs/outcomes/20260815-definition-delta-migrate/phases/33-plan.md`:
/// `smelt explain --show-sql`'s keyed-fold preview must honour a
/// `maintenance.cells[].technique: unconditional` pin addressing the
/// keyed fold's driving source — preview/live parity, since the live
/// windowed-keyed-maintenance path now folds the same override ladder in
/// (`crate::cumulative::resolve_cumulative_write_suppression`). Otherwise
/// identical to `keyed_fold_preview_renders_the_suppressed_matched_arm`
/// above, whose model/plan this pin is layered onto — without the pin the
/// preview would render the change-suppressed guard.
#[test]
fn explain_show_sql_keyed_fold_honours_the_pin() {
    let content =
        "SELECT device_id, SUM(amount) AS total_amount FROM smelt.events GROUP BY device_id";
    let path: PathBuf = "device_total.sql".into();
    let model = ModelFile {
        name: "device_total".to_string(),
        model_id: smelt_core::ModelId::from_path(path.clone()),
        path,
        content: content.to_string(),
        refs: vec![RefInfo {
            has_named_params: false,
            range: Default::default(),
            smelt_ref: SmeltRef::Path(vec!["events".to_string()]),
        }],
        parse_errors: Vec::new(),
        metadata: Some(Box::new(ModelMetadata {
            materialization: Some(Materialization::Table),
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(ConfigGrain::Key),
            maintenance: Some(smelt_core::config::MaintenanceConfig {
                defaults: None,
                cells: vec![smelt_core::config::MaintenanceCellConfig {
                    columns: vec![],
                    on: "smelt.events".to_string(),
                    prefer: None,
                    technique: Some(smelt_core::config::CellTechnique::Unconditional),
                    write: None,
                }],
                scan_bounds: None,
            }),
            ..Default::default()
        })),
        kind: smelt_core::ModelKind::Sql,
        address_segments: vec!["device_total".to_string()],
    };
    let metadata = model.metadata.as_deref().unwrap();
    let stripped_sql = smelt_parser::strip_frontmatter(&model.content).to_string();

    let sources = vec![smelt_logical::maintenance::SourceFacts {
        name: "events".to_string(),
        mutation: smelt_logical::maintenance::MutationProfile::AppendOnly,
        partition_col: Some("event_date".to_string()),
        unique_key: vec![],
        allow_full_scan: true,
    }];
    let plan_result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        &stripped_sql,
        "device_total",
        metadata,
        &sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &std::collections::BTreeMap::new(),
        None,
        None,
    )
    .expect("device_total must derive a maintenance plan");
    let cell = plan_result
        .plan
        .cells
        .iter()
        .find(|c| c.technique == Technique::KeyedFold)
        .expect("device_total must admit a KeyedFold cell");

    let mut source_timeseries = smelt_planner::SourceTimeseriesMap::new();
    source_timeseries.insert(
        "smelt.events".to_string(),
        TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        },
    );

    let config = load_fixture().2;
    let registry = CompilerRegistry::new(&config, &config.targets);
    let resolver = registry
        .get("dev")
        .build_ephemeral_resolver(&[], "main")
        .expect("no ephemerals");

    let diagnostics = build_plan_cell_diagnostics(
        cell,
        &model,
        "main",
        "dev",
        &registry,
        &resolver,
        MaintenanceDialect::DuckDb,
        &[],
        &source_timeseries,
        &plan_result.column_groups,
    );

    let preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::KeyedFold)
        .expect("KeyedFold must have a preview entry");
    assert_eq!(preview.admissibility, Admissibility::Admitted);
    let sql = preview
        .statements
        .first()
        .expect("the Admitted KeyedFold preview must render a statement")
        .sql
        .clone();
    assert!(
        !sql.contains("IS DISTINCT FROM"),
        "the pinned technique: unconditional must suppress the change-suppressed guard: {sql}"
    );
}
