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
    diagnostics_for_in(&root, files, model_file)
}

/// Like [`diagnostics_for`], but ingests against a caller-supplied root —
/// lets a test stage a `.smelt/` deployed-schema snapshot at the same root
/// before ingest sees it (`diagnostics_for` creates its own private tempdir,
/// which a caller can never write into ahead of time).
fn diagnostics_for_in(
    root: &std::path::Path,
    files: &[(&str, &str)],
    model_file: &str,
) -> Vec<smelt_db::Diagnostic> {
    for (rel, content) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
    }

    let loaded = load_workspace(root);
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

const SCAN_BOUNDS_ORDERS_SOURCE: &str = r#"
description: Orders, append-only, clocked on order_date.
mutation_profile: append_only
columns:
  - { name: order_id, type: INTEGER, nullable: false }
  - { name: order_date, type: DATE, nullable: false }
  - { name: customer_id, type: INTEGER, nullable: false }
"#;

const SCAN_BOUNDS_ENRICHMENT_SOURCE: &str = r#"
description: Customer enrichment lookup, mutable snapshot, unclocked.
mutation_profile: mutable_snapshot
columns:
  - { name: customer_id, type: INTEGER, nullable: false }
  - { name: category, type: VARCHAR, nullable: true }
"#;

/// A model with exactly one payload column group (`{enrichment_category}`)
/// sensitive to the unclocked `enrichment` source — `order_date` is the
/// skeleton/clock column, excluded from `column_groups` entirely (mirrors
/// `unbounded_scan_refuses_by_default`'s own two-group fixture, minus the
/// `order_id` pass-through group, so exactly one `MaintenanceScanUnbounded`
/// is derivable per test).
fn scan_bounds_model(extra_frontmatter: &str) -> String {
    format!(
        r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
{extra_frontmatter}---
SELECT
    o.order_date,
    e.category AS enrichment_category
FROM smelt.sources.orders o
JOIN smelt.sources.enrichment e ON o.customer_id = e.customer_id
"#
    )
}

/// `scan_bounds.on_violation: warn` admits the derived plan for the
/// otherwise-unbounded `enrichment` source and reports exactly one
/// `MaintenanceScanUnbounded` diagnostic at `Warning` severity — the plan
/// still admits a `Trigger::NewData` creation cell
/// (`docs/specs/incremental_models.md` §"Partition-local maintenance (the K8
/// guardrail)").
#[test]
fn scan_bounds_on_violation_warn_admits_and_warns() {
    let model = scan_bounds_model("maintenance:\n  scan_bounds:\n    on_violation: warn\n");
    let files = [
        ("smelt.yml", SMELT_YML),
        ("models/sources/orders.yml", SCAN_BOUNDS_ORDERS_SOURCE),
        (
            "models/sources/enrichment.yml",
            SCAN_BOUNDS_ENRICHMENT_SOURCE,
        ),
        ("models/revenue.sql", model.as_str()),
    ];
    let diags = diagnostics_for(&files, "revenue");

    let scan_unbounded: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MaintenanceScanUnbounded))
        .collect();
    assert_eq!(
        scan_unbounded.len(),
        1,
        "expected exactly one MaintenanceScanUnbounded, got {diags:?}"
    );
    assert_eq!(
        scan_unbounded[0].severity,
        smelt_db::DiagnosticSeverity::Warning,
        "on_violation: warn must report a Warning, not an Error"
    );

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();
    for (rel, content) in &files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
    }
    let loaded = load_workspace(&root);
    let mut db = smelt_db::Database::default();
    let ingested = ingest_loaded_workspace(&mut db, &loaded);
    db.set_workspace(ingested.source_files.clone(), vec![ingested.project]);
    let ws = db.workspace();
    let target_path = root.join("models").join("revenue.sql");
    let file = ingested
        .source_files
        .iter()
        .zip(ingested.paths.iter())
        .find(|(_, p)| **p == target_path)
        .map(|(f, _)| *f)
        .unwrap_or_else(|| panic!("model file {target_path:?} not ingested"));
    let report = smelt_db::maintenance_plan_report(&db, ws, file)
        .expect("revenue is an incremental model with a resolved grain");
    assert!(
        report.plan.cells.iter().any(|c| matches!(
            c.trigger,
            smelt_logical::maintenance::Trigger::NewData { .. }
        )),
        "on_violation: warn must still admit a creation cell, got {:?}",
        report.plan.cells
    );
}

/// The same fixture with `on_violation` absent (default `error`) still
/// refuses with an Error — guards the default.
#[test]
fn scan_bounds_on_violation_error_still_refuses() {
    let model = scan_bounds_model("maintenance:\n  scan_bounds:\n    on_violation: error\n");
    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/orders.yml", SCAN_BOUNDS_ORDERS_SOURCE),
            (
                "models/sources/enrichment.yml",
                SCAN_BOUNDS_ENRICHMENT_SOURCE,
            ),
            ("models/revenue.sql", model.as_str()),
        ],
        "revenue",
    );
    let scan_unbounded: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MaintenanceScanUnbounded))
        .collect();
    assert_eq!(
        scan_unbounded.len(),
        1,
        "expected exactly one MaintenanceScanUnbounded, got {diags:?}"
    );
    assert_eq!(
        scan_unbounded[0].severity,
        smelt_db::DiagnosticSeverity::Error,
        "explicit on_violation: error must still refuse as an Error"
    );

    let model_default = scan_bounds_model("");
    let diags_default = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/orders.yml", SCAN_BOUNDS_ORDERS_SOURCE),
            (
                "models/sources/enrichment.yml",
                SCAN_BOUNDS_ENRICHMENT_SOURCE,
            ),
            ("models/revenue.sql", model_default.as_str()),
        ],
        "revenue",
    );
    let scan_unbounded_default: Vec<_> = diags_default
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MaintenanceScanUnbounded))
        .collect();
    assert_eq!(
        scan_unbounded_default.len(),
        1,
        "absent on_violation must default to error, got {diags_default:?}"
    );
    assert_eq!(
        scan_unbounded_default[0].severity,
        smelt_db::DiagnosticSeverity::Error,
        "absent on_violation must default to Error"
    );
}

/// A project-level `on_violation: error` with a model-level `warn` resolves
/// to Warn — narrower wins, mirroring `require`'s own ladder
/// (`effective_scan_bounds_model_overrides_project`).
#[test]
fn scan_bounds_warn_is_per_model_over_project() {
    let smelt_yml_project_error: &str = r#"
name: maintenance_diagnostics_fixture
version: 1

paths:
  - models

maintenance:
  scan_bounds:
    on_violation: error

targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main

default_materialization: view
"#;
    let model = scan_bounds_model("maintenance:\n  scan_bounds:\n    on_violation: warn\n");
    let diags = diagnostics_for(
        &[
            ("smelt.yml", smelt_yml_project_error),
            ("models/sources/orders.yml", SCAN_BOUNDS_ORDERS_SOURCE),
            (
                "models/sources/enrichment.yml",
                SCAN_BOUNDS_ENRICHMENT_SOURCE,
            ),
            ("models/revenue.sql", model.as_str()),
        ],
        "revenue",
    );
    let scan_unbounded: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MaintenanceScanUnbounded))
        .collect();
    assert_eq!(
        scan_unbounded.len(),
        1,
        "expected exactly one MaintenanceScanUnbounded, got {diags:?}"
    );
    assert_eq!(
        scan_unbounded[0].severity,
        smelt_db::DiagnosticSeverity::Warning,
        "model-level warn must win over project-level error"
    );
}

/// A `grain: key` model whose body never aggregates has no fold candidate —
/// the plan-shaped read is a partition-shaped (row-per-event) body under a
/// key-addressed declaration. This particular body also declares no
/// top-level `unique_key:` and has no `GROUP BY` to derive one from, so the
/// frontmatter-time identity check (`GrainAssertionMismatch`,
/// `docs/specs/models.md` §"Constraint violations") now catches it before
/// plan-derivation's own technique-admission refusal
/// (`MaintenanceNoAdmissibleTechnique`) ever runs — never silently keeping
/// the mismatched declaration either way.
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
        .filter(|d| d.code == Some(DiagnosticCode::GrainAssertionMismatch))
        .collect();
    assert_eq!(
        refusals.len(),
        1,
        "expected exactly one GrainAssertionMismatch, got {diags:?}"
    );
    assert_eq!(
        refusals[0].severity,
        smelt_db::DiagnosticSeverity::Error,
        "grain mismatch must be an Error, never silent"
    );
}

/// A `grain: key` model with no declared top-level `unique_key:` and whose
/// own SELECT has no `GROUP BY` at all derives no identity — the plan
/// derivation refuses fail-loud (`GrainAssertionMismatch`) at frontmatter
/// time, naming the asserted grain and the empty derived key
/// (`docs/specs/models.md` §"Constraint violations").
#[test]
fn grain_key_without_unique_key_or_group_by_errors() {
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
        .filter(|d| d.code == Some(DiagnosticCode::GrainAssertionMismatch))
        .collect();
    assert_eq!(
        refusals.len(),
        1,
        "expected exactly one GrainAssertionMismatch, got {diags:?}"
    );
    assert_eq!(
        refusals[0].severity,
        smelt_db::DiagnosticSeverity::Error,
        "an underivable identity must be an Error, never silent"
    );
    assert!(
        refusals[0].message.contains("grain: key") && refusals[0].message.contains("no key"),
        "message must name the asserted grain and the empty derived key, got: {}",
        refusals[0].message
    );
}

/// A `grain: key` model with no declared top-level `unique_key:` but whose
/// own SELECT does have a `GROUP BY` derives its identity from that GROUP
/// BY — no diagnostic.
#[test]
fn grain_key_without_unique_key_but_with_group_by_is_clean() {
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
SELECT user_id, SUM(amount) as total_amount FROM smelt.sources.payments GROUP BY user_id
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
        .filter(|d| d.code == Some(DiagnosticCode::GrainAssertionMismatch))
        .collect();
    assert!(
        refusals.is_empty(),
        "GROUP-BY-derived identity must not produce GrainAssertionMismatch, got {diags:?}"
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
        None,
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
/// `Refusal::SkeletonChanged`, which `smelt-db`'s refusal→diagnostic
/// mapping surfaces as `MaintenanceSkeletonChanged`
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
        None,
    )
    .expect("refresh: incremental model must derive a plan");

    assert!(
        result.plan.refusals.iter().any(|r| matches!(
            r,
            Refusal::SkeletonChanged { column } if column == "user_id"
        )),
        "expected a SkeletonChanged refusal naming 'user_id'; got refusals {:?}, cells {:?}",
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
        None,
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

/// Guards the pre-rename skeleton-diagnostic spelling
/// (`docs/outcomes/20260815-definition-delta-migrate/outcome.md` phase 7):
/// a half-done rename cannot pass green. The needle is built from parts so
/// this guard's own source does not itself trip the check it performs.
#[test]
fn no_stale_skeleton_column_added_spelling() {
    use std::path::Path;

    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root");

    let excluded_dirs = [
        "docs/plans",
        "docs/handoffs",
        "docs/research",
        "docs/outcomes",
        "target",
    ];
    let stale_needle = ["Skeleton", "Column", "Added"].concat();

    let mut hits = Vec::new();
    for root in ["crates", "docs/specs"] {
        for entry in walk_files(&workspace_root.join(root)) {
            let rel = entry
                .strip_prefix(&workspace_root)
                .expect("entry under workspace root");
            let rel_str = rel.to_string_lossy();
            if excluded_dirs.iter().any(|d| rel_str.starts_with(d))
                || rel.components().any(|c| c.as_os_str() == "target")
            {
                continue;
            }
            let Ok(content) = fs::read_to_string(&entry) else {
                continue;
            };
            if content.contains(&stale_needle) {
                hits.push(rel_str.into_owned());
            }
        }
    }

    assert!(
        hits.is_empty(),
        "stale skeleton-column-added spelling found in: {hits:?} — rename to the changed spelling"
    );
}

/// Guards the pre-`smelt migrate`-wiring claim
/// (`docs/outcomes/20260815-definition-delta-migrate/outcome.md` phase 8):
/// `smelt migrate` now ships, so no spec may still say it doesn't exist. The
/// needle is built from parts so this guard's own source does not itself
/// trip the check it performs.
#[test]
fn no_stale_no_migrate_command_claim() {
    use std::path::Path;

    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root");

    let excluded_dirs = [
        "docs/plans",
        "docs/handoffs",
        "docs/research",
        "docs/outcomes",
        "target",
    ];
    let stale_needle = ["No `smelt ", "migrate` ", "command exists"].concat();

    let mut hits = Vec::new();
    for entry in walk_files(&workspace_root.join("docs/specs")) {
        let rel = entry
            .strip_prefix(&workspace_root)
            .expect("entry under workspace root");
        let rel_str = rel.to_string_lossy();
        if excluded_dirs.iter().any(|d| rel_str.starts_with(d))
            || rel.components().any(|c| c.as_os_str() == "target")
        {
            continue;
        }
        let Ok(content) = fs::read_to_string(&entry) else {
            continue;
        };
        if content.contains(&stale_needle) {
            hits.push(rel_str.into_owned());
        }
    }

    assert!(
        hits.is_empty(),
        "stale 'no smelt migrate command exists' claim found in: {hits:?} — smelt migrate ships \
         now, reword to describe its actual scope"
    );
}

fn walk_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_files(&path));
        } else if path.is_file() {
            out.push(path);
        }
    }
    out
}

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
}

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
