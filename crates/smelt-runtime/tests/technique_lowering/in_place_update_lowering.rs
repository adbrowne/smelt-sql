/// `docs/plans/20260809-sensitivity-precision.md` Phase 6: `Technique::
/// InPlaceUpdate` lowers through the same pure-emitter path every other
/// technique preview does (`smelt-runtime::diagnostics::
/// build_technique_statements`'s `Technique::InPlaceUpdate` arm) — the
/// `diagnostics.rs:574` hard refusal ("has no production consumer yet") is
/// deleted as part of this phase; this asserts the replacement actually
/// builds a statement via `smelt_logical::maintenance::emit::
/// emit_in_place_update`, never a re-implementation in `smelt-runtime`
/// itself (statement-parity's "no authoring outside the emitter" leg).
use std::collections::HashMap;

use smelt_core::config::{Grain, Materialization, RefreshStrategy, TimeseriesConfig};
use smelt_core::{Granularity, ModelFile, ModelMetadata, RefInfo, SmeltRef};
use smelt_logical::maintenance::emit::MaintenanceDialect;
use smelt_logical::maintenance::{
    Corner, PartitionLocal, PlanCell, RowIdentity, RowIdentityVerdict, Technique, Trigger,
};
use smelt_planner::SourceTimeseriesMap;
use smelt_runtime::diagnostics::{build_plan_cell_diagnostics, Admissibility};
use smelt_runtime::CompilerRegistry;

fn test_config() -> smelt_core::config::Config {
    let mut targets = HashMap::new();
    targets.insert(
        "dev".to_string(),
        smelt_core::config::Target {
            target_type: "duckdb".to_string(),
            database: Some("test.duckdb".to_string()),
            schema: "main".to_string(),
            connect_url: None,
            catalog: None,
            warehouse: None,
            format: None,
            settings: None,
            project: None,
            dataset: None,
            location: None,
        },
    );
    smelt_core::config::Config {
        name: "in_place_update_lowering_fixture".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets,
        default_materialization: smelt_core::config::Materialization::View,
        models: HashMap::new(),
        python: None,
        target: None,
        state: Default::default(),
        maintenance: None,
        probes: Default::default(),
    }
}

/// A `PureBackfill` cell: `val_doubled AS val * 2` depends only on an
/// already-stored `val` column — no upstream read, an in-place `UPDATE`
/// is admissible (`docs/specs/definition_deltas.md` §"The verdict per column group").
fn admitted_in_place_update_plan() -> PlanCell {
    PlanCell {
        group: "{val_doubled}".to_string(),
        trigger: Trigger::ColumnAdded {
            columns: vec!["val_doubled".to_string()],
        },
        corner: Corner::FoldDelta,
        technique: Technique::InPlaceUpdate,
        partition_local: PartitionLocal::Yes,
        scans: vec![],
        ledger_catch_up: true,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: Default::default(),
        key_scope: None,
        state_downgrade: None,
    }
}

#[test]
fn in_place_update_lowered_from_pure_backfill_cell() {
    let content =
        "SELECT event_date, user_id, val, val * 2 AS val_doubled FROM smelt.sources.events";
    let path: std::path::PathBuf = "events_enriched.sql".into();
    let model = ModelFile {
        name: "events_enriched".to_string(),
        model_id: smelt_core::ModelId::from_path(path.clone()),
        path,
        content: content.to_string(),
        refs: vec![RefInfo {
            has_named_params: false,
            range: Default::default(),
            smelt_ref: SmeltRef::Path(vec!["sources".to_string(), "events".to_string()]),
        }],
        parse_errors: Vec::new(),
        metadata: Some(Box::new(ModelMetadata {
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(Grain::Partition),
            materialization: Some(Materialization::Table),
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
        address_segments: vec!["events_enriched".to_string()],
    };

    let config = test_config();
    let registry = CompilerRegistry::new(&config, &config.targets);
    let resolver = registry
        .get("dev")
        .build_ephemeral_resolver(&[], "main")
        .expect("no ephemerals");
    let source_timeseries = SourceTimeseriesMap::new();
    let cell = admitted_in_place_update_plan();

    let diagnostics = build_plan_cell_diagnostics(
        &cell,
        &model,
        "main",
        "dev",
        &registry,
        &resolver,
        MaintenanceDialect::DuckDb,
        &[],
        &source_timeseries,
        &[],
    );

    let preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::InPlaceUpdate)
        .expect("InPlaceUpdate must have a preview entry");
    assert_eq!(
        preview.admissibility,
        Admissibility::Admitted,
        "the cell's own admitted technique must resolve Admitted: {preview:?}"
    );
    assert_eq!(
        preview.statements.len(),
        1,
        "expected exactly one UPDATE statement: {preview:?}"
    );
    let sql = &preview.statements[0].sql;
    assert!(
        sql.starts_with("UPDATE main.events_enriched SET val_doubled = "),
        "expected an UPDATE ... SET val_doubled = <expr> statement, got: {sql}"
    );
    assert!(
        sql.contains("val * 2") || sql.contains("val*2"),
        "the assignment must be the added column's own defining expression from the \
         model's SQL, not re-derived: {sql}"
    );
}
