use super::*;

// ============================================================================
// Deployed-schema snapshot world-fact input (phase 9,
// docs/outcomes/20260815-definition-delta-migrate)
// ============================================================================
//
// `DeployedSchemaInput` is a Salsa world-fact input registered by
// `workspace_ingest::register_deployed_schemas_from_disk` (called from
// `ingest_loaded_workspace`, itself called by both the CLI's `init_db` and
// the LSP's `initialize` — workspace-loading-parity rule). `maintenance_plan`
// resolves it by table name and threads its columns + `model_sql` into
// `derive_model_maintenance_plan`, so `MaintenanceSkeletonChanged` can now
// surface ahead of any run.

mod deployed_schema_world_fact {
    use super::*;
    use chrono::Utc;
    use smelt_state::file_store::FileStore;
    use smelt_state::schema_tracking::{DeployedColumn, DeployedSchema};

    const KEYED_SMELT_YML: &str = r#"
name: deployed_schema_fixture
version: 1

paths:
  - models

targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main

default_materialization: view
"#;

    const DEVICE_SOURCE: &str = r#"
description: Device events, append-only.
mutation_profile: append_only
columns:
  - { name: device_id, type: INTEGER, nullable: false }
  - { name: user_id, type: INTEGER, nullable: false }
"#;

    fn write_schema(
        root: &std::path::Path,
        target: &str,
        model: &str,
        columns: &[&str],
        model_sql: Option<&str>,
    ) {
        let store = FileStore::new(root, target);
        store.init().expect("init .smelt");
        let schema = DeployedSchema {
            model: model.to_string(),
            version: 1,
            deployed_at: Utc::now(),
            model_hash: "test-hash".to_string(),
            model_sql: model_sql.map(|s| s.to_string()),
            partition_column: None,
            columns: columns
                .iter()
                .map(|c| DeployedColumn {
                    name: c.to_string(),
                    data_type: "INTEGER".to_string(),
                    nullable: false,
                })
                .collect(),
        };
        store.save_schema(&schema).expect("save deployed schema");
    }

    /// Like [`write_schema`], additionally recording a declared
    /// `partition_column` address for the partition-column-rename refusal
    /// tests below.
    fn write_schema_with_partition_column(
        root: &std::path::Path,
        target: &str,
        model: &str,
        columns: &[&str],
        partition_column: &str,
    ) {
        let store = FileStore::new(root, target);
        store.init().expect("init .smelt");
        let schema = DeployedSchema {
            model: model.to_string(),
            version: 1,
            deployed_at: Utc::now(),
            model_hash: "test-hash".to_string(),
            model_sql: None,
            partition_column: Some(partition_column.to_string()),
            columns: columns
                .iter()
                .map(|c| DeployedColumn {
                    name: c.to_string(),
                    data_type: "INTEGER".to_string(),
                    nullable: false,
                })
                .collect(),
        };
        store.save_schema(&schema).expect("save deployed schema");
    }

    const PARTITION_SMELT_YML: &str = r#"
name: partition_column_rename_fixture
version: 1

paths:
  - models

targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main

default_materialization: view
"#;

    const EVENTS_SOURCE: &str = r#"
description: Events, append-only.
mutation_profile: append_only
columns:
  - { name: event_date, type: DATE, nullable: false }
  - { name: amount, type: INTEGER, nullable: false }
"#;

    /// A model whose declared `timeseries.partition_column` differs from the
    /// address recorded in the deployed-schema snapshot at last deploy
    /// emits `MaintenancePartitionColumnChanged` — the address every
    /// partition-grain maintenance write targets is a world fact, not
    /// re-derivable from the compiled SQL alone.
    #[test]
    fn renamed_partition_column_emits_maintenance_partition_column_changed() {
        let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_date, SUM(amount) AS total
FROM smelt.sources.events
GROUP BY event_date
"#;
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        write_schema_with_partition_column(
            &root,
            "dev",
            "renamed_events",
            &["event_date", "total"],
            "event_day",
        );
        let diags = diagnostics_for_in(
            &root,
            &[
                ("smelt.yml", PARTITION_SMELT_YML),
                ("models/sources/events.yml", EVENTS_SOURCE),
                ("models/renamed_events.sql", model),
            ],
            "renamed_events",
        );
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(DiagnosticCode::MaintenancePartitionColumnChanged)),
            "expected MaintenancePartitionColumnChanged, got {diags:?}"
        );

        // A sibling model whose recorded and declared partition_column
        // match emits none.
        write_schema_with_partition_column(
            &root,
            "dev",
            "unchanged_events",
            &["event_date", "total"],
            "event_date",
        );
        let diags_unchanged = diagnostics_for_in(
            &root,
            &[
                ("smelt.yml", PARTITION_SMELT_YML),
                ("models/sources/events.yml", EVENTS_SOURCE),
                ("models/unchanged_events.sql", model),
            ],
            "unchanged_events",
        );
        assert!(
            diags_unchanged
                .iter()
                .all(|d| d.code != Some(DiagnosticCode::MaintenancePartitionColumnChanged)),
            "matching partition_column must emit no refusal, got {diags_unchanged:?}"
        );
    }

    /// A registered snapshot whose `model_sql` groups only by `device_id`
    /// (the current model additionally groups by `user_id`) makes
    /// `file_diagnostics` emit `MaintenanceSkeletonChanged` — the skeleton
    /// changed (a new GROUP BY key), proven by the clause-level diff rather
    /// than by a `ColumnAdded` trigger landing in a skeleton position (the
    /// current model's own SELECT list is unchanged: `device_id`, `n`).
    #[test]
    fn deployed_schema_input_surfaces_skeleton_changed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let model = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT device_id, COUNT(*) AS n FROM smelt.sources.device GROUP BY device_id
"#;
        // The deployed snapshot groups by device_id AND user_id — the
        // current model on disk dropped `user_id` from GROUP BY, a skeleton
        // (grain) change.
        let old_sql = "SELECT device_id, COUNT(*) AS n FROM smelt.sources.device \
                        GROUP BY device_id, user_id";
        write_schema(
            &root,
            "dev",
            "device_counts",
            &["device_id", "n"],
            Some(old_sql),
        );

        let diags = diagnostics_for_in(
            &root,
            &[
                ("smelt.yml", KEYED_SMELT_YML),
                ("models/sources/device.yml", DEVICE_SOURCE),
                ("models/device_counts.sql", model),
            ],
            "device_counts",
        );

        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(DiagnosticCode::MaintenanceSkeletonChanged)),
            "expected MaintenanceSkeletonChanged from the registered deployed-schema \
             snapshot's skeleton-clause diff, got {diags:?}"
        );
    }

    /// With no `.smelt/` snapshot registered at all, the diagnostic set is
    /// byte-identical to today (fail-closed regression guard) — no
    /// definition-change trigger is derivable without a world fact to
    /// compare against.
    #[test]
    fn no_deployed_schema_derives_no_definition_trigger() {
        let model = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT device_id, COUNT(*) AS n FROM smelt.sources.device GROUP BY device_id
"#;
        let diags = diagnostics_for(
            &[
                ("smelt.yml", KEYED_SMELT_YML),
                ("models/sources/device.yml", DEVICE_SOURCE),
                ("models/device_counts.sql", model),
            ],
            "device_counts",
        );
        assert!(
            diags
                .iter()
                .all(|d| d.code != Some(DiagnosticCode::MaintenanceSkeletonChanged)),
            "no registered deployed schema must derive no MaintenanceSkeletonChanged, \
             got {diags:?}"
        );
    }

    /// A registered snapshot whose columns AND `model_sql` are identical to
    /// the current model on disk is silent — no maintenance diagnostic.
    #[test]
    fn deployed_schema_matching_current_definition_is_silent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let model = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT device_id, COUNT(*) AS n FROM smelt.sources.device GROUP BY device_id
"#;
        fs::write(root.join("smelt.yml"), KEYED_SMELT_YML).unwrap();
        fs::create_dir_all(root.join("models/sources")).unwrap();
        fs::write(root.join("models/sources/device.yml"), DEVICE_SOURCE).unwrap();
        fs::write(root.join("models/device_counts.sql"), model).unwrap();

        write_schema(
            &root,
            "dev",
            "device_counts",
            &["device_id", "n"],
            Some(model),
        );

        let diags = diagnostics_for_in(
            &root,
            &[
                ("smelt.yml", KEYED_SMELT_YML),
                ("models/sources/device.yml", DEVICE_SOURCE),
                ("models/device_counts.sql", model),
            ],
            "device_counts",
        );
        assert!(
            diags
                .iter()
                .all(|d| d.code != Some(DiagnosticCode::MaintenanceSkeletonChanged)),
            "a snapshot matching the current definition must be silent, got {diags:?}"
        );
    }

    /// The same column set (`category`, `total`) on both sides, but the
    /// GROUP BY changed (an extra grouping key not itself projected) — the
    /// refusal fires from the clause diff, not from a `ColumnAdded` trigger
    /// (there is no added column here at all).
    #[test]
    fn skeleton_clause_change_surfaces_without_a_column_add() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let sales_source = r#"
description: Sales, append-only.
mutation_profile: append_only
columns:
  - { name: category, type: VARCHAR, nullable: false }
  - { name: region, type: VARCHAR, nullable: false }
  - { name: amount, type: DOUBLE, nullable: false }
"#;
        let model = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT category, SUM(amount) AS total FROM smelt.sources.sales GROUP BY category, region
"#;
        fs::write(root.join("smelt.yml"), KEYED_SMELT_YML).unwrap();
        fs::create_dir_all(root.join("models/sources")).unwrap();
        fs::write(root.join("models/sources/sales.yml"), sales_source).unwrap();
        fs::write(root.join("models/category_totals.sql"), model).unwrap();

        let old_sql = "SELECT category, SUM(amount) AS total FROM smelt.sources.sales \
                        GROUP BY category";
        write_schema(
            &root,
            "dev",
            "category_totals",
            &["category", "total"],
            Some(old_sql),
        );

        let diags = diagnostics_for_in(
            &root,
            &[
                ("smelt.yml", KEYED_SMELT_YML),
                ("models/sources/sales.yml", sales_source),
                ("models/category_totals.sql", model),
            ],
            "category_totals",
        );
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(DiagnosticCode::MaintenanceSkeletonChanged)),
            "a changed GROUP BY with an unchanged column set must still refuse via the \
             clause diff, got {diags:?}"
        );
    }

    /// Re-setting an already-registered `DeployedSchemaInput`'s fields
    /// within the SAME `Database` re-invalidates `maintenance_plan` — Salsa
    /// invalidation is real here, not just a load-time snapshot.
    #[test]
    fn updating_the_deployed_schema_input_reinvalidates() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let model = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT device_id, COUNT(*) AS n FROM smelt.sources.device GROUP BY device_id
"#;

        fs::write(root.join("smelt.yml"), KEYED_SMELT_YML).unwrap();
        fs::create_dir_all(root.join("models/sources")).unwrap();
        fs::write(root.join("models/sources/device.yml"), DEVICE_SOURCE).unwrap();
        fs::write(root.join("models/device_counts.sql"), model).unwrap();
        let loaded = load_workspace(&root);

        let mut db = smelt_db::Database::default();
        let ingested = smelt_db::workspace_ingest::ingest_loaded_workspace(&mut db, &loaded);
        db.set_workspace(ingested.source_files.clone(), vec![ingested.project]);
        let ws = db.workspace();
        let target_path = root.join("models/device_counts.sql");
        let file = ingested
            .source_files
            .iter()
            .zip(ingested.paths.iter())
            .find(|(_, p)| **p == target_path)
            .map(|(f, _)| *f)
            .unwrap_or_else(|| panic!("model file {target_path:?} not ingested"));

        // No snapshot registered yet — silent.
        let diags_before = smelt_db::file_diagnostics(&db, ws, file);
        assert!(
            diags_before
                .iter()
                .all(|d| d.code != Some(DiagnosticCode::MaintenanceSkeletonChanged)),
            "no snapshot registered yet must be silent, got {diags_before:?}"
        );

        // Register a snapshot whose skeleton clause differs — the same
        // Database instance must now re-derive the refusal.
        let old_sql = "SELECT device_id, COUNT(*) AS n FROM smelt.sources.device \
                        GROUP BY device_id, user_id";
        db.set_deployed_schema(
            std::sync::Arc::from("device_counts"),
            root.clone(),
            vec![std::sync::Arc::from("device_id"), std::sync::Arc::from("n")],
            Some(std::sync::Arc::from(old_sql)),
            None,
        );
        let diags_after = smelt_db::file_diagnostics(&db, ws, file);
        assert!(
            diags_after
                .iter()
                .any(|d| d.code == Some(DiagnosticCode::MaintenanceSkeletonChanged)),
            "setting the deployed-schema input must re-invalidate maintenance_plan \
             within the same Database, got {diags_after:?}"
        );
    }

    const CLOCKED_BASE_SOURCE: &str = r#"
description: Base rows, append-only, clocked on event_date.
mutation_profile: append_only
columns:
  - { name: id, type: INTEGER, nullable: false }
  - { name: event_date, type: DATE, nullable: false }
  - { name: a, type: INTEGER, nullable: false }
"#;

    /// Phase 25 (`docs/outcomes/20260815-definition-delta-migrate`,
    /// `docs/specs/definition_deltas.md` §"Detection" posture rule 1): two
    /// added, non-skeleton columns whose classifications disagree (`b` is a
    /// pure function of an already-stored column, `c` depends on `b` — a
    /// column that did not exist before this edit, so it re-derives) cannot
    /// share one technique. Reported as `MaintenanceColumnAddNotBackfillable`
    /// — a Warning, never an Error — and the message names `smelt migrate`.
    #[test]
    fn not_backfillable_column_add_is_a_warning_naming_smelt_migrate() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT id, event_date, a, a + 1 AS b, b + 1 AS c FROM smelt.sources.base
"#;
        write_schema(
            &root,
            "dev",
            "derived_totals",
            &["id", "event_date", "a"],
            Some("SELECT id, event_date, a FROM smelt.sources.base"),
        );

        let diags = diagnostics_for_in(
            &root,
            &[
                ("smelt.yml", SMELT_YML),
                ("models/sources/base.yml", CLOCKED_BASE_SOURCE),
                ("models/derived_totals.sql", model),
            ],
            "derived_totals",
        );

        let warning = diags
            .iter()
            .find(|d| d.code == Some(DiagnosticCode::MaintenanceColumnAddNotBackfillable))
            .unwrap_or_else(|| {
                panic!("expected MaintenanceColumnAddNotBackfillable, got {diags:?}")
            });
        assert_eq!(
            warning.severity,
            smelt_db::DiagnosticSeverity::Warning,
            "a non-backfillable column add must never be an Error: {warning:?}"
        );
        assert!(
            warning.message.contains("smelt migrate"),
            "message must point at smelt migrate: {}",
            warning.message
        );
        assert!(
            diags.iter().all(
                |d| d.code != Some(DiagnosticCode::MaintenanceSkeletonChanged)
                    && d.code != Some(DiagnosticCode::MaintenanceNoAdmissibleTechnique)
            ),
            "a non-backfillable column add must never ALSO surface as an Error code, \
             got {diags:?}"
        );
    }

    /// Posture rule 1 does not widen to an ordinary ongoing-fold refusal: a
    /// ScanUnbounded refusal from a plain `Trigger::NewData` fold (no
    /// definition delta involved — the deployed snapshot's columns match the
    /// current model exactly) stays `MaintenanceScanUnbounded` at Error, even
    /// now that real deployed column names are threaded through.
    #[test]
    fn ongoing_fold_refusal_is_still_an_error_with_a_deployed_snapshot() {
        let orders_source = r#"
description: Orders, append-only, clocked on order_date.
mutation_profile: append_only
columns:
  - { name: order_id, type: INTEGER, nullable: false }
  - { name: order_date, type: DATE, nullable: false }
  - { name: customer_id, type: INTEGER, nullable: false }
"#;
        let enrichment_source = r#"
description: Customer enrichment lookup, mutable snapshot, unclocked.
mutation_profile: mutable_snapshot
columns:
  - { name: customer_id, type: INTEGER, nullable: false }
  - { name: category, type: VARCHAR, nullable: true }
"#;
        let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
---
SELECT
    o.order_id,
    o.order_date,
    e.category AS enrichment_category
FROM smelt.sources.orders o
JOIN smelt.sources.enrichment e ON o.customer_id = e.customer_id
"#;
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        // The deployed snapshot's columns match the current model's output
        // exactly — no `Trigger::ColumnAdded` derives at all.
        write_schema(
            &root,
            "dev",
            "revenue",
            &["order_id", "order_date", "enrichment_category"],
            Some(model),
        );

        let diags = diagnostics_for_in(
            &root,
            &[
                ("smelt.yml", SMELT_YML),
                ("models/sources/orders.yml", orders_source),
                ("models/sources/enrichment.yml", enrichment_source),
                ("models/revenue.sql", model),
            ],
            "revenue",
        );

        let scan_unbounded: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::MaintenanceScanUnbounded))
            .collect();
        assert!(
            !scan_unbounded.is_empty(),
            "expected the ordinary fold's MaintenanceScanUnbounded to survive threading a \
             real deployed snapshot, got {diags:?}"
        );
        assert!(
            scan_unbounded
                .iter()
                .all(|d| d.severity == smelt_db::DiagnosticSeverity::Error),
            "an ordinary fold's ScanUnbounded refusal must stay Error: {scan_unbounded:?}"
        );
        assert!(
            diags
                .iter()
                .all(|d| d.code != Some(DiagnosticCode::MaintenanceColumnAddNotBackfillable)),
            "no definition delta exists here (deployed columns match current output); no \
             MaintenanceColumnAddNotBackfillable should fire, got {diags:?}"
        );
    }

    /// Posture rule 3: a model declaring `schema_evolution: strategy:
    /// full_refresh` derives no definition-change trigger in the gate at
    /// all, even though the registered snapshot is missing an additive
    /// column — the runtime rebuilds the whole table on its next run, so
    /// there is no in-place backfill obligation to warn about ahead of time.
    #[test]
    fn full_refresh_schema_evolution_model_derives_no_definition_change_refusal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
schema_evolution:
  strategy: full_refresh
---
SELECT id, event_date, a, a + 1 AS b FROM smelt.sources.base
"#;
        write_schema(
            &root,
            "dev",
            "full_refresh_totals",
            &["id", "event_date", "a"],
            Some("SELECT id, event_date, a FROM smelt.sources.base"),
        );

        let diags = diagnostics_for_in(
            &root,
            &[
                ("smelt.yml", SMELT_YML),
                ("models/sources/base.yml", CLOCKED_BASE_SOURCE),
                ("models/full_refresh_totals.sql", model),
            ],
            "full_refresh_totals",
        );

        assert!(
            diags.iter().all(|d| {
                d.code != Some(DiagnosticCode::MaintenanceColumnAddNotBackfillable)
                    && d.code != Some(DiagnosticCode::MaintenanceSkeletonChanged)
            }),
            "schema_evolution: strategy: full_refresh must derive no definition-change \
             trigger at all, got {diags:?}"
        );
    }

    /// The Salsa path (`maintenance_plan_report`, via `plan_for`) now sees a
    /// real `Trigger::ColumnAdded` cell — proof the threading actually
    /// happened, not just that the diagnostic mapping is wired.
    #[test]
    fn maintenance_plan_derives_the_column_added_cell_from_the_registered_snapshot() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT id, event_date, a, a + 1 AS b FROM smelt.sources.base
"#;
        write_schema(
            &root,
            "dev",
            "pure_backfill_totals",
            &["id", "event_date", "a"],
            Some("SELECT id, event_date, a FROM smelt.sources.base"),
        );
        let result = plan_for_in(
            &root,
            &[
                ("smelt.yml", SMELT_YML),
                ("models/sources/base.yml", CLOCKED_BASE_SOURCE),
                ("models/pure_backfill_totals.sql", model),
            ],
            "pure_backfill_totals",
        );

        let column_added_cell = result.plan.cells.iter().find(|c| {
            matches!(&c.trigger, smelt_logical::maintenance::Trigger::ColumnAdded { columns }
                if columns == &vec!["b".to_string()])
        });
        assert!(
            column_added_cell.is_some(),
            "expected a real Trigger::ColumnAdded cell for [\"b\"] once the registered \
             snapshot's column names are threaded; got cells {:?}, refusals {:?}",
            result.plan.cells,
            result.plan.refusals
        );
        assert_eq!(
            column_added_cell.unwrap().technique,
            smelt_logical::maintenance::Technique::InPlaceUpdate,
        );
    }
}
