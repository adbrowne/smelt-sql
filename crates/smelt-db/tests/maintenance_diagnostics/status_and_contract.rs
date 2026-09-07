use super::*;

/// `register_deployed_schemas_from_disk` reads one `DeployedSchemaInput` per
/// `.smelt/targets/<target>/schemas/<model>.json` file, and is a silent
/// no-op for a missing/unreadable schemas directory (the loader-file
/// precedent: a stale snapshot must never fail workspace load).
#[test]
fn register_deployed_schemas_from_disk_reads_target_schemas() {
    use chrono::Utc;
    use smelt_state::file_store::FileStore;
    use smelt_state::schema_tracking::{DeployedColumn, DeployedSchema};

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();

    // No .smelt/ at all yet — silent no-op.
    let mut db = smelt_db::Database::default();
    smelt_db::workspace_ingest::register_deployed_schemas_from_disk(&mut db, &root, "dev");
    assert!(
        db.deployed_schema(&root, "orders").is_none(),
        "missing .smelt/ must register nothing"
    );

    let store = FileStore::new(&root, "dev");
    store.init().expect("init .smelt");
    let schema = DeployedSchema {
        model: "orders".to_string(),
        version: 1,
        deployed_at: Utc::now(),
        model_hash: "h".to_string(),
        model_sql: Some("SELECT 1".to_string()),
        partition_column: None,
        columns: vec![DeployedColumn {
            name: "order_id".to_string(),
            data_type: "INTEGER".to_string(),
            nullable: false,
        }],
    };
    store.save_schema(&schema).expect("save schema");

    let mut db = smelt_db::Database::default();
    smelt_db::workspace_ingest::register_deployed_schemas_from_disk(&mut db, &root, "dev");
    let input = db
        .deployed_schema(&root, "orders")
        .expect("orders schema registered");
    assert_eq!(input.model(&db).as_ref(), "orders");
    assert_eq!(
        input
            .columns(&db)
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>(),
        vec!["order_id".to_string()]
    );
    assert_eq!(
        input.model_sql(&db).as_ref().map(|s| s.as_ref()),
        Some("SELECT 1")
    );
}

// ---------------------------------------------------------------------------
// Phase 19 (`docs/outcomes/20260815-definition-delta-migrate`): a CLOCKED
// explicitly-mutable source now derives an `UpstreamMutation` cell through
// the production wrapper (`smelt_logical::maintenance::derive::
// derive_triggers`), reachable from a real fact/dimension fixture mirroring
// `examples/timeseries/models/daily_events_status.sql` (fact `raw.events` ×
// a clocked, mutable `raw.user_status` joined on an explicit window
// predicate).

const STATUS_FIXTURE_EVENTS_SOURCE: &str = r#"
description: Raw events, append-only, clocked.
mutation_profile: append_only
columns:
  - { name: event_id, type: INTEGER, nullable: false }
  - { name: user_id, type: INTEGER, nullable: false }
  - { name: event_timestamp, type: TIMESTAMP, nullable: false }
unique_key: [event_id]
"#;

const STATUS_FIXTURE_USER_STATUS_SOURCE: &str = r#"
description: Time-varying user status, clocked, mutable.
mutation_profile:
  kind: mutable_snapshot
timeseries:
  partition_column: changed_at
  event_time_column: changed_at
  granularity: day
unique_key: [user_id]
columns:
  - { name: user_id, type: INTEGER, nullable: false }
  - { name: status, type: VARCHAR, nullable: true }
  - { name: changed_at, type: TIMESTAMP, nullable: false }
"#;

const STATUS_FIXTURE_MODEL: &str = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  partition_column: event_date
  event_time_column: event_timestamp
  granularity: day
---
SELECT
    e.event_id,
    date_trunc('day', e.event_timestamp) AS event_date,
    e.user_id,
    s.status
FROM smelt.sources.raw.events e
JOIN smelt.sources.raw.user_status s
  ON e.user_id = s.user_id
 AND s.changed_at BETWEEN e.event_timestamp - INTERVAL '1 day'
                       AND e.event_timestamp + INTERVAL '1 day'
"#;

fn status_fixture_files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("smelt.yml", SMELT_YML),
        (
            "models/sources/raw/events.yml",
            STATUS_FIXTURE_EVENTS_SOURCE,
        ),
        (
            "models/sources/raw/user_status.yml",
            STATUS_FIXTURE_USER_STATUS_SOURCE,
        ),
        ("models/daily_events_status.sql", STATUS_FIXTURE_MODEL),
    ]
}

/// The production wrapper (`smelt_db::maintenance_plan_report`, the same
/// Salsa query `file_diagnostics` and `smelt explain` consume) derives a
/// `{status}` `UpstreamMutation{raw.user_status}` cell with no admission
/// refusals, `PartitionLocal::Yes` (a genuine scan clamp on `changed_at`,
/// per the fixture's explicit `BETWEEN` predicate), and `Technique::
/// DeleteInsert` — `raw.user_status` is read in the join's own `ON`
/// predicate, a row-admission position, so the `{status}` group is
/// membership- (not merely value-) sensitive and must admit the recompute
/// family, never a column-scoped `MERGE` (`docs/specs/incremental_models.md`
/// §"The plan matrix").
#[test]
fn daily_events_status_derives_a_status_mutation_cell_through_the_wrapper() {
    let result = plan_for(&status_fixture_files(), "daily_events_status");
    assert!(
        result.plan.refusals.is_empty(),
        "expected no admission refusals: {:?}",
        result.plan.refusals
    );

    let mutation_trigger = smelt_logical::maintenance::Trigger::UpstreamMutation {
        source: "raw.user_status".to_string(),
    };
    let cell = result
        .plan
        .cells
        .iter()
        .find(|c| c.trigger == mutation_trigger && c.group == "{status}")
        .unwrap_or_else(|| {
            panic!(
                "no {{status}} cell admitted for {mutation_trigger:?}: {:#?}",
                result.plan
            )
        });
    assert_eq!(
        cell.technique,
        smelt_logical::maintenance::Technique::DeleteInsert
    );
    assert_eq!(
        cell.corner,
        smelt_logical::maintenance::Corner::RecomputeRegion
    );
    assert_eq!(
        cell.partition_local,
        smelt_logical::maintenance::PartitionLocal::Yes
    );
    let scan = cell
        .scans
        .iter()
        .find(|s| s.source == "raw.user_status")
        .unwrap_or_else(|| panic!("no scan clamp for 'raw.user_status': {:?}", cell.scans));
    assert_eq!(scan.column, "changed_at");
}

/// The same fixture, minus the fact's own window predicate against
/// `raw.user_status` — the clocked mutable source's scan cannot be clamped
/// to the output partition axis, so it must refuse loudly
/// (`Refusal::ScanUnbounded`) rather than silently dropping the cell.
#[test]
fn clocked_mutable_source_with_no_derivable_clamp_refuses_scan_unbounded() {
    let unclamped_model = STATUS_FIXTURE_MODEL.replace(
        "JOIN smelt.sources.raw.user_status s\n  ON e.user_id = s.user_id\n AND s.changed_at BETWEEN e.event_timestamp - INTERVAL '1 day'\n                       AND e.event_timestamp + INTERVAL '1 day'\n",
        "JOIN smelt.sources.raw.user_status s\n  ON e.user_id = s.user_id\n",
    );
    assert_ne!(
        unclamped_model, STATUS_FIXTURE_MODEL,
        "the replace must actually strip the window predicate"
    );

    let mut files = status_fixture_files();
    let model_idx = files
        .iter()
        .position(|(rel, _)| *rel == "models/daily_events_status.sql")
        .unwrap();
    let leaked: &'static str = Box::leak(unclamped_model.into_boxed_str());
    files[model_idx].1 = leaked;

    let result = plan_for(&files, "daily_events_status");
    assert!(
        result
            .plan
            .refusals
            .iter()
            .any(|r| matches!(r, smelt_logical::maintenance::Refusal::ScanUnbounded { .. })),
        "expected a ScanUnbounded refusal naming raw.user_status, got {:?}",
        result.plan.refusals
    );
}

/// Phase 28c end-to-end: `examples/source_mutation_profile_declared`'s
/// `raw_events` source already declares `mutation_profile: change_feed`
/// (that fixture is smoke-tested for diagnostics/build only). This mirrors
/// its exact source declaration and model SQL, adding the `refresh:
/// incremental` frontmatter an incremental maintenance plan needs, and
/// asserts through the same production Salsa path `file_diagnostics` uses
/// that the model now carries an `UpstreamMutation` cell for `raw_events`,
/// clamped to full-input re-derivation (`Corner::RecomputeRegion`,
/// `Technique::DeleteInsert`) — never a column-scoped merge.
#[test]
fn change_feed_declared_source_derives_upstream_mutation_cell() {
    let raw_events_source = r#"
description: Raw event feed exposing a change-data feed; smelt reads only rows changed since the last run.
mutation_profile: change_feed
source_lateness: '2 hours'
columns:
  - { name: event_id, type: INTEGER,   nullable: false }
  - { name: user_id,  type: INTEGER,   nullable: true }
  - { name: event_ts, type: TIMESTAMP, nullable: false }
  - { name: amount,   type: DOUBLE,    nullable: true }
"#;
    let events_model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  partition_column: event_ts
  event_time_column: event_ts
  granularity: day
maintenance:
  scan_bounds:
    per_source:
      raw_events:
        allow_full_scan: true
---
SELECT
    event_id,
    user_id,
    event_ts,
    amount
FROM smelt.sources.raw_events
"#;
    let files = vec![
        ("smelt.yml", SMELT_YML),
        ("models/sources/raw_events.yml", raw_events_source),
        ("models/events.sql", events_model),
    ];

    let result = plan_for(&files, "events");
    assert!(
        result.plan.refusals.is_empty(),
        "expected the change_feed cell to be admitted, got refusals {:?}",
        result.plan.refusals
    );
    let cell = result
        .plan
        .cells
        .iter()
        .find(|c| {
            matches!(&c.trigger, smelt_logical::maintenance::Trigger::UpstreamMutation { source } if source == "raw_events")
        })
        .unwrap_or_else(|| {
            panic!(
                "expected an UpstreamMutation cell for raw_events, got cells {:?}",
                result.plan.cells
            )
        });
    assert_eq!(
        cell.corner,
        smelt_logical::maintenance::Corner::RecomputeRegion
    );
    assert_eq!(
        cell.technique,
        smelt_logical::maintenance::Technique::DeleteInsert
    );
}

// ============================================================================
// State residency (phase 7, docs/outcomes/20260904-state-residency):
// `MaintenanceStateDowngraded` and `DeclaredContractRequiresState`, both
// folded from availability resolution
// (`smelt_logical::maintenance::availability::resolve_availability`) by the
// pure `maintenance_plan_diagnostics`.
// ============================================================================

const STATE_RESIDENCY_EVENTS_SOURCE: &str = r#"
description: Events, append-only, clocked on event_date.
mutation_profile: append_only
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
columns:
  - { name: device_id, type: INTEGER, nullable: false }
  - { name: event_date, type: DATE, nullable: false }
  - { name: amount, type: DOUBLE, nullable: false }
"#;

/// A `grain: key` model whose only fold column (`AVG`) admits
/// `Technique::KeyedFold` — the same fixture shape
/// `avg_model_derives_fold_spec_and_keyed_fold_cell` uses at the pure-function
/// layer, here driven through the real Salsa workspace so it also exercises
/// `project_active_backends`/`project_warehouse_tables`.
const KEYED_FOLD_MODEL: &str = r#"---
materialization: table
refresh: incremental
grain: key
maintenance:
  scan_bounds:
    per_source:
      events:
        allow_full_scan: true
---
SELECT device_id, AVG(amount) AS avg_amount
FROM smelt.sources.events
GROUP BY device_id
"#;

fn smelt_yml_single_target(target_type: &str) -> String {
    format!(
        r#"
name: state_residency_fixture
version: 1

paths:
  - models

targets:
  dev:
    type: {target_type}
    database: target/dev.duckdb
    schema: main

default_materialization: view
"#
    )
}

/// A `grain: key` fold cell (`Technique::KeyedFold`, needs the
/// reconciliation ledger) on a project whose only declared target is Spark —
/// no ledger builder — gets a Warning `MaintenanceStateDowngraded` naming
/// its creation cell (`NewData`). The plan also derives a companion
/// `UpstreamMutation` cell over the same fold column, which needs the
/// (also-DuckDB-only) transactional merge ledger and downgrades
/// independently — both are expected, not just the fold cell.
#[test]
fn keyed_fold_on_a_spark_target_warns_state_downgraded() {
    let diags = diagnostics_for(
        &[
            ("smelt.yml", &smelt_yml_single_target("spark")),
            ("models/sources/events.yml", STATE_RESIDENCY_EVENTS_SOURCE),
            ("models/device_avg.sql", KEYED_FOLD_MODEL),
        ],
        "device_avg",
    );
    let downgrades: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MaintenanceStateDowngraded))
        .collect();
    assert!(
        downgrades
            .iter()
            .any(|d| d.message.contains("NewData") && d.message.contains("KeyedFold")),
        "expected a MaintenanceStateDowngraded naming the KeyedFold creation cell, got {diags:?}"
    );
    assert!(
        downgrades
            .iter()
            .all(|d| d.severity == smelt_db::DiagnosticSeverity::Warning),
        "MaintenanceStateDowngraded must be a Warning, never blocking"
    );
}

/// The same model on a DuckDB-only project — the reconciliation ledger is
/// realisable — emits no downgrade at all.
#[test]
fn keyed_fold_on_duckdb_emits_no_state_downgrade() {
    let diags = diagnostics_for(
        &[
            ("smelt.yml", &smelt_yml_single_target("duckdb")),
            ("models/sources/events.yml", STATE_RESIDENCY_EVENTS_SOURCE),
            ("models/device_avg.sql", KEYED_FOLD_MODEL),
        ],
        "device_avg",
    );
    assert!(
        diags
            .iter()
            .all(|d| d.code != Some(DiagnosticCode::MaintenanceStateDowngraded)),
        "DuckDB realises the reconciliation ledger; expected no downgrade, got {diags:?}"
    );
}

/// `state.warehouse_tables: none` forces the downgrade even on DuckDB — the
/// opt-out overrides what the backend could otherwise realise.
#[test]
fn warehouse_tables_none_warns_state_downgraded_on_duckdb() {
    let smelt_yml = format!(
        "{}\nstate:\n  warehouse_tables: none\n",
        smelt_yml_single_target("duckdb")
    );
    let diags = diagnostics_for(
        &[
            ("smelt.yml", &smelt_yml),
            ("models/sources/events.yml", STATE_RESIDENCY_EVENTS_SOURCE),
            ("models/device_avg.sql", KEYED_FOLD_MODEL),
        ],
        "device_avg",
    );
    let downgrades: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MaintenanceStateDowngraded))
        .collect();
    assert!(
        downgrades
            .iter()
            .any(|d| d.message.contains("NewData") && d.message.contains("KeyedFold")),
        "warehouse_tables: none must force the downgrade even on DuckDB, got {diags:?}"
    );
}

/// `MaintenanceStateDowngraded`'s Warning severity never blocks the plan —
/// the downgraded cell still resolves (no accompanying
/// `MaintenanceNoAdmissibleTechnique`/`GrainAssertionMismatch` refusal).
#[test]
fn state_downgrade_warning_never_blocks() {
    let diags = diagnostics_for(
        &[
            ("smelt.yml", &smelt_yml_single_target("spark")),
            ("models/sources/events.yml", STATE_RESIDENCY_EVENTS_SOURCE),
            ("models/device_avg.sql", KEYED_FOLD_MODEL),
        ],
        "device_avg",
    );
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(DiagnosticCode::MaintenanceStateDowngraded)),
        "expected the downgrade to still fire, got {diags:?}"
    );
    assert!(
        diags
            .iter()
            .all(|d| d.severity != smelt_db::DiagnosticSeverity::Error),
        "a state downgrade must never accompany an Error-severity refusal, got {diags:?}"
    );
}

const CONTRACT_ORDERS_SOURCE: &str = r#"
description: Orders, append-only, clocked on order_date.
mutation_profile: append_only
columns:
  - { name: order_date, type: DATE, nullable: false }
  - { name: amount, type: DOUBLE, nullable: false }
"#;

/// A model-level `contract.deferral` declaration on a Spark-only target (no
/// reconciliation ledger) gets one Error `DeclaredContractRequiresState`
/// naming the declaration and the missing structure.
#[test]
fn deferral_without_a_ledger_refuses_declared_contract_requires_state() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
contract:
  deferral: 1 day
---
SELECT order_date, SUM(amount) AS total
FROM smelt.sources.orders
GROUP BY order_date
"#;
    let diags = diagnostics_for(
        &[
            ("smelt.yml", &smelt_yml_single_target("spark")),
            ("models/sources/orders.yml", CONTRACT_ORDERS_SOURCE),
            ("models/order_totals.sql", model),
        ],
        "order_totals",
    );
    let refusals: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::DeclaredContractRequiresState))
        .collect();
    assert_eq!(
        refusals.len(),
        1,
        "expected exactly one DeclaredContractRequiresState, got {diags:?}"
    );
    assert_eq!(
        refusals[0].severity,
        smelt_db::DiagnosticSeverity::Error,
        "DeclaredContractRequiresState must be an Error"
    );
    assert!(
        refusals[0].message.contains("contract.deferral"),
        "message must name the declaration, got: {}",
        refusals[0].message
    );
}

/// Same refusal, declared via `contract.cells[].deferral` instead of the
/// model-level default.
#[test]
fn cell_level_deferral_without_a_ledger_refuses() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
contract:
  cells:
    - columns: [total]
      on: orders
      deferral: 1 day
---
SELECT order_date, SUM(amount) AS total
FROM smelt.sources.orders
GROUP BY order_date
"#;
    let diags = diagnostics_for(
        &[
            ("smelt.yml", &smelt_yml_single_target("spark")),
            ("models/sources/orders.yml", CONTRACT_ORDERS_SOURCE),
            ("models/order_totals.sql", model),
        ],
        "order_totals",
    );
    let refusals: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::DeclaredContractRequiresState))
        .collect();
    assert_eq!(
        refusals.len(),
        1,
        "expected exactly one DeclaredContractRequiresState, got {diags:?}"
    );
    assert!(
        refusals[0].message.contains("contract.cells"),
        "message must name the cell-level declaration, got: {}",
        refusals[0].message
    );
}

/// The same declaration on a DuckDB-only project (default
/// `warehouse_tables: allowed`) is admitted — no refusal.
#[test]
fn deferral_on_duckdb_is_admitted() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
contract:
  deferral: 1 day
---
SELECT order_date, SUM(amount) AS total
FROM smelt.sources.orders
GROUP BY order_date
"#;
    let diags = diagnostics_for(
        &[
            ("smelt.yml", &smelt_yml_single_target("duckdb")),
            ("models/sources/orders.yml", CONTRACT_ORDERS_SOURCE),
            ("models/order_totals.sql", model),
        ],
        "order_totals",
    );
    assert!(
        diags
            .iter()
            .all(|d| d.code != Some(DiagnosticCode::DeclaredContractRequiresState)),
        "DuckDB realises the reconciliation ledger; expected no refusal, got {diags:?}"
    );
}
