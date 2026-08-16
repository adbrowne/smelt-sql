//! `Maintenance*` diagnostics: the thin `maintenance_plan` Salsa query
//! (`crates/smelt-db/src/queries/maintenance.rs`) folds the derived plan's
//! admission refusals, plus the `maintenance.cells[]` column-group-span
//! check, into `file_diagnostics()`.
//!
//! Spec: `docs/specs/incremental_models.md` §Diagnostics, §Semantics
//! "Partition-local maintenance (the K8 guardrail)"; `docs/specs/models.md`
//! "Declared grain contradicted by the derived plan" (Constraint violations
//! table).

use std::fs;

use smelt_core::workspace::load_workspace;
use smelt_db::{workspace_ingest::ingest_loaded_workspace, DiagnosticCode};

/// Build a real on-disk workspace under a fresh tempdir, ingest it into a
/// Salsa `Database`, and return the diagnostics for `model_file` (relative
/// to `models/`, without extension).
fn diagnostics_for(files: &[(&str, &str)], model_file: &str) -> Vec<smelt_db::Diagnostic> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();
    for (rel, content) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
    }

    let loaded = load_workspace(&root);
    let mut db = smelt_db::Database::default();
    let ingested = ingest_loaded_workspace(&mut db, &loaded);
    db.set_workspace(ingested.source_files.clone(), vec![ingested.project]);
    let ws = db.workspace();

    let target_path = root.join("models").join(format!("{model_file}.sql"));
    let file = ingested
        .source_files
        .iter()
        .zip(ingested.paths.iter())
        .find(|(_, p)| **p == target_path)
        .map(|(f, _)| *f)
        .unwrap_or_else(|| panic!("model file {target_path:?} not ingested"));

    smelt_db::file_diagnostics(&db, ws, file)
}

const SMELT_YML: &str = r#"
name: maintenance_diagnostics_fixture
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

/// A cross-axis, unclocked enrichment source with no derivable predicate to
/// the output's partition axis must refuse with `MaintenanceScanUnbounded`
/// under the K8 default (`require: partition_local`, `on_violation:
/// error`); a declared `allow_full_scan: true` for that source clears it.
#[test]
fn unbounded_scan_refuses_by_default() {
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

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/orders.yml", orders_source),
            ("models/sources/enrichment.yml", enrichment_source),
            ("models/revenue.sql", model),
        ],
        "revenue",
    );

    // `enrichment` is read only in the JOIN's ON predicate — never in a
    // select item for `o.order_id` — so BOTH payload groups
    // (`{order_id}` and `{enrichment_category}`) are membership-sensitive to
    // it (`docs/specs/model_properties.md` §"Per-column mutation-sensitivity
    // / column provenance", membership paragraph) and each refuses its own
    // `MaintenanceScanUnbounded` independently.
    let scan_unbounded: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MaintenanceScanUnbounded))
        .collect();
    assert_eq!(
        scan_unbounded.len(),
        2,
        "expected one MaintenanceScanUnbounded per membership-sensitive payload \
         group, got {diags:?}"
    );

    // `allow_full_scan: true` for the enrichment source clears the refusal.
    let model_allowed = model.replacen(
        "grain: partition\n",
        "grain: partition\nmaintenance:\n  scan_bounds:\n    per_source:\n      enrichment:\n        allow_full_scan: true\n",
        1,
    );
    let diags_allowed = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/orders.yml", orders_source),
            ("models/sources/enrichment.yml", enrichment_source),
            ("models/revenue.sql", &model_allowed),
        ],
        "revenue",
    );
    assert!(
        diags_allowed
            .iter()
            .all(|d| d.code != Some(DiagnosticCode::MaintenanceScanUnbounded)),
        "allow_full_scan: true should clear MaintenanceScanUnbounded, got {diags_allowed:?}"
    );
}

/// A `grain: key` model whose body never aggregates has no fold candidate —
/// the plan-shaped read is a partition-shaped (row-per-event) body under a
/// key-addressed declaration, and the derivation refuses honestly
/// (`MaintenanceNoAdmissibleTechnique`) rather than silently keeping the
/// mismatched declaration (`docs/specs/models.md`: "Declared grain
/// contradicted by the derived plan ... Hard error").
#[test]
fn grain_mismatch_is_error_never_silent() {
    let payments_source = r#"
description: Payments, append-only.
mutation_profile: append_only
columns:
  - { name: user_id, type: INTEGER, nullable: false }
  - { name: amount, type: DOUBLE, nullable: false }
"#;
    let model = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT user_id, amount FROM smelt.sources.payments
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/payments.yml", payments_source),
            ("models/lifetime_value.sql", model),
        ],
        "lifetime_value",
    );

    let refusals: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MaintenanceNoAdmissibleTechnique))
        .collect();
    assert_eq!(
        refusals.len(),
        1,
        "expected exactly one MaintenanceNoAdmissibleTechnique, got {diags:?}"
    );
    assert_eq!(
        refusals[0].severity,
        smelt_db::DiagnosticSeverity::Error,
        "grain mismatch must be an Error, never silent"
    );
}

/// `maintenance.cells[].columns` naming members of two different derived
/// column groups is an error — it would silently re-partition the plan.
#[test]
fn cells_columns_spanning_groups_error() {
    let payments_source = r#"
description: Payments, append-only.
mutation_profile: append_only
columns:
  - { name: order_id, type: INTEGER, nullable: false }
  - { name: order_date, type: DATE, nullable: false }
  - { name: amount, type: DOUBLE, nullable: false }
"#;
    let shipments_source = r#"
description: Shipments, mutable snapshot.
mutation_profile: mutable_snapshot
columns:
  - { name: order_id, type: INTEGER, nullable: false }
  - { name: shipped_flag, type: BOOLEAN, nullable: true }
"#;
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
maintenance:
  cells:
    - columns: [amount, shipped_flag]
      on: payments
---
SELECT
    p.order_id,
    p.order_date,
    p.amount,
    s.shipped_flag
FROM smelt.sources.payments p
JOIN smelt.sources.shipments s ON p.order_id = s.order_id
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/payments.yml", payments_source),
            ("models/sources/shipments.yml", shipments_source),
            ("models/orders_enriched.sql", model),
        ],
        "orders_enriched",
    );

    let violations: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::MaintenanceNoAdmissibleTechnique)
                && d.message.contains("spans")
        })
        .collect();
    assert_eq!(
        violations.len(),
        1,
        "expected exactly one cells[].columns-spans-groups violation, got {diags:?}"
    );
}

/// The web-analytics tracer's flagship shape (Blocked-phases entry, W1,
/// 2026-07-18): a `grain: key` + `timeseries:` event-grain dedupe whose
/// `partition_column` is populated by an extremal fold (`MIN(...)`) over a
/// `GROUP BY event_id`. Before the grouped-extremal nullability rule, `MIN`
/// inferred nullable unconditionally, so the derived `partition_column`
/// could never satisfy `timeseries.md`'s NOT-NULL precondition —
/// `MalformedTimeseries` fired unconditionally, and the plan derivation
/// refused with `MaintenanceNoAdmissibleTechnique` as a downstream
/// consequence of the same failed fold classification. Both must be gone
/// now that `MIN(event_date)` under `GROUP BY event_id` infers NOT NULL.
#[test]
fn grouped_extremal_fold_partition_column_satisfies_timeseries_not_null() {
    let events_source = r#"
description: Raw events, append-only, redelivery-prone.
mutation_profile:
  kind: append_only
  key_recurrence:
    key: [event_id]
    window: '1 day'
columns:
  - { name: event_id, type: INTEGER, nullable: false }
  - { name: device_id, type: VARCHAR, nullable: true }
  - { name: user_id, type: INTEGER, nullable: true }
  - { name: event_time, type: TIMESTAMP, nullable: false }
  - { name: event_date, type: DATE, nullable: false }
  - { name: utm_campaign, type: VARCHAR, nullable: true }
  - { name: payload, type: VARCHAR, nullable: true }
"#;
    let model = r#"---
materialization: table
refresh: incremental
grain: key
timeseries:
  event_time_column: first_seen_date
  partition_column: first_seen_date
  granularity: day
---
SELECT
    event_id,
    MIN(device_id) AS device_id,
    MIN(user_id) AS user_id,
    MIN(CAST(event_time AS TIMESTAMP)) AS event_ts,
    MIN(CAST(event_date AS DATE)) AS first_seen_date,
    MIN(utm_campaign) AS utm_campaign,
    MIN(payload) AS payload
FROM smelt.sources.raw.events
GROUP BY event_id
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/raw/events.yml", events_source),
            ("models/events_deduped.sql", model),
        ],
        "events_deduped",
    );

    assert!(
        diags
            .iter()
            .all(|d| d.code != Some(DiagnosticCode::MalformedTimeseries)),
        "grouped MIN(event_date) partition_column must satisfy the timeseries \
         NOT-NULL precondition; got {diags:?}"
    );
    assert!(
        diags
            .iter()
            .all(|d| d.code != Some(DiagnosticCode::MaintenanceNoAdmissibleTechnique)),
        "the fold classification must succeed now that the extremal-fold \
         partition_column is NOT NULL; got {diags:?}"
    );
}

/// The production `Trigger::ColumnAdded` derivation
/// (`docs/plans/20260809-sensitivity-precision.md` Phase 6,
/// `docs/specs/definition_deltas.md` §"The verdict per column group"):
/// `derive_model_maintenance_plan`'s new `deployed_column_names` parameter
/// diffs the model's currently-projected columns against a supplied
/// deployed-schema snapshot and, when the diff finds a genuinely new
/// column, pushes a `Trigger::ColumnAdded` — `smelt-db`'s own callers (this
/// module, `smelt explain`) have no I/O access to a real snapshot and
/// always pass `&[]` (asserted by the earlier tests in this file never
/// seeing the trigger fire); this test exercises the pure function
/// directly with a caller-supplied snapshot, exactly as `smelt-runtime`'s
/// maintenance driver does in production.
///
/// A column whose expression reads only already-stored columns (`val`, via
/// `val * 2`) over an append-only source with no aggregation classifies
/// `PureBackfill` — an admissible `Technique::InPlaceUpdate` cell.
#[test]
fn column_added_trigger_derived_from_deployed_schema() {
    use smelt_core::config::{Grain, Granularity, RefreshStrategy, TimeseriesConfig};
    use smelt_core::ModelMetadata;
    use smelt_logical::maintenance::{MutationProfile, SourceFacts, Technique, Trigger};

    // The model as it exists AFTER the edit — `val_doubled` is a new
    // output column, added to the deployed schema's prior shape below.
    let sql = "SELECT event_date, user_id, val, val * 2 AS val_doubled \
               FROM smelt.sources.events";
    let metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::Partition),
        timeseries: Some(TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        ..Default::default()
    };
    let sources = vec![SourceFacts {
        name: "events".to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: Some("event_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }];
    // The deployed schema's column names BEFORE the edit — `val_doubled`
    // is absent.
    let deployed_column_names = vec![
        "event_date".to_string(),
        "user_id".to_string(),
        "val".to_string(),
    ];

    let result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql,
        "main.events_enriched",
        &metadata,
        &sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &deployed_column_names,
        &std::collections::BTreeMap::new(),
    )
    .expect("refresh: incremental model must derive a plan");

    let column_added_cell = result
        .plan
        .cells
        .iter()
        .find(|c| matches!(&c.trigger, Trigger::ColumnAdded { columns } if columns == &vec!["val_doubled".to_string()]))
        .unwrap_or_else(|| {
            panic!(
                "expected a ColumnAdded cell for [\"val_doubled\"]; got cells {:?}, \
                 refusals {:?}",
                result.plan.cells, result.plan.refusals
            )
        });
    assert_eq!(
        column_added_cell.technique,
        Technique::InPlaceUpdate,
        "a pure function of already-stored columns must admit InPlaceUpdate: {column_added_cell:?}"
    );
}

/// The skeleton-add direction of the same production derivation: an added
/// column that occupies a `GROUP BY` key position is a grain change, never
/// a column backfill (EX-39) — the plan refuses with
/// `Refusal::SkeletonColumnAdded`, which `smelt-db`'s refusal→diagnostic
/// mapping surfaces as `MaintenanceSkeletonColumnAdded`
/// (`crates/smelt-db/src/lib.rs`'s `file_diagnostics` match arm).
#[test]
fn column_added_trigger_skeleton_position_refuses() {
    use smelt_core::config::{Grain, RefreshStrategy};
    use smelt_core::ModelMetadata;
    use smelt_logical::maintenance::{MutationProfile, Refusal, SourceFacts, Trigger};

    // AFTER the edit: `user_id` was added to the GROUP BY key set — a grain
    // change, not a payload backfill.
    let sql = "SELECT device_id, user_id, COUNT(*) AS n \
               FROM smelt.sources.events GROUP BY device_id, user_id";
    let metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::Key),
        ..Default::default()
    };
    let sources = vec![SourceFacts {
        name: "events".to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: None,
        unique_key: vec![],
        allow_full_scan: false,
    }];
    // BEFORE the edit: only `device_id` was grouped by.
    let deployed_column_names = vec!["device_id".to_string(), "n".to_string()];

    let result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql,
        "main.device_user_counts",
        &metadata,
        &sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &deployed_column_names,
        &std::collections::BTreeMap::new(),
    )
    .expect("refresh: incremental model must derive a plan");

    assert!(
        result.plan.refusals.iter().any(|r| matches!(
            r,
            Refusal::SkeletonColumnAdded { column } if column == "user_id"
        )),
        "expected a SkeletonColumnAdded refusal naming 'user_id'; got refusals {:?}, cells {:?}",
        result.plan.refusals,
        result.plan.cells
    );
    assert!(
        !result
            .plan
            .cells
            .iter()
            .any(|c| matches!(&c.trigger, Trigger::ColumnAdded { .. })),
        "a skeleton-position add must admit no cell at all: {:?}",
        result.plan.cells
    );
}

/// Reviewer coverage gap (`docs/plans/20260809-sensitivity-precision.md`
/// Phase 6): the deployed-schema diff has no rename detection — a column
/// rename (deployed `[event_date, user_id, foo]` → current `[event_date,
/// user_id, bar]`, `foo` dropped, `bar` new) must be classified as a plain
/// column add of `bar`, fresh-derived from its own expression, never
/// mistaken for an in-place rename of `foo`. Since `bar`'s expression reads
/// an upstream source column (`raw_bar`, not one of the deployed model's
/// OWN already-stored columns), `classify_definition_change`'s leg 3 must
/// fall through to `UpstreamRederive` — `bar` is never eligible for
/// `Technique::InPlaceUpdate` on a fabricated "this is just `foo`
/// renamed" assumption. The load-bearing assertion is simply that
/// derivation completes sanely (no panic, no `InPlaceUpdate` cell for
/// `bar`) — `derive_model_maintenance_plan` is never told anything about
/// `foo`'s disappearance; that is `schema_evolution`'s own independent
/// `RemoveColumn` concern (physical `ALTER TABLE ... DROP COLUMN`), out of
/// scope for this trigger.
#[test]
fn column_added_trigger_rename_case_never_treated_as_in_place_update() {
    use smelt_core::config::{Grain, Granularity, RefreshStrategy, TimeseriesConfig};
    use smelt_core::ModelMetadata;
    use smelt_logical::maintenance::{MutationProfile, SourceFacts, Technique, Trigger};

    // AFTER the edit: `foo` was renamed away and replaced by `bar`, whose
    // expression reads an upstream source column — not derivable purely
    // from the model's own previously-stored columns.
    let sql = "SELECT event_date, user_id, raw_bar AS bar \
               FROM smelt.sources.events";
    let metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::Partition),
        timeseries: Some(TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        ..Default::default()
    };
    let sources = vec![SourceFacts {
        name: "events".to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: Some("event_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }];
    // BEFORE the edit: `foo`, not `bar` — the deployed snapshot has no
    // knowledge that `bar` is "really" a rename of `foo`.
    let deployed_column_names = vec![
        "event_date".to_string(),
        "user_id".to_string(),
        "foo".to_string(),
    ];

    // Must never panic.
    let result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql,
        "main.events_enriched",
        &metadata,
        &sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &deployed_column_names,
        &std::collections::BTreeMap::new(),
    )
    .expect("refresh: incremental model must derive a plan");

    // `bar` must never resolve to `Technique::InPlaceUpdate` — it is not
    // `PureBackfill`-derivable from the deployed columns (`foo` is not
    // `bar`'s dependency; `bar` reads an upstream source column).
    let bar_in_place_update_cell = result.plan.cells.iter().find(|c| {
        matches!(&c.trigger, Trigger::ColumnAdded { columns } if columns == &vec!["bar".to_string()])
            && c.technique == Technique::InPlaceUpdate
    });
    assert!(
        bar_in_place_update_cell.is_none(),
        "a renamed-in column sourced from upstream data must never admit InPlaceUpdate as if it \
         were a pure backfill of the old (now-absent) column: cells {:?}, refusals {:?}",
        result.plan.cells,
        result.plan.refusals
    );
}
