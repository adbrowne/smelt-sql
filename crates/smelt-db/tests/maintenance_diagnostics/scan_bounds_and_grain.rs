use super::*;

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

/// A key-addressed model (`grain: key`) declaring top-level
/// `safety_overrides:` gets the dedicated `KeyedForbidsSafetyOverrides`
/// diagnostic (CLI + LSP parity via `file_diagnostics()`), not the
/// misdirecting `PartitionGrainRequiresRefreshIncremental`.
#[test]
fn keyed_safety_overrides_is_dedicated_error() {
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
unique_key: [user_id]
safety_overrides:
  allow_having: true
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
        .filter(|d| d.code == Some(DiagnosticCode::KeyedForbidsSafetyOverrides))
        .collect();
    assert_eq!(
        refusals.len(),
        1,
        "expected exactly one KeyedForbidsSafetyOverrides, got {diags:?}"
    );
    assert_eq!(
        refusals[0].severity,
        smelt_db::DiagnosticSeverity::Error,
        "keyed safety_overrides must be an Error"
    );
    assert!(
        diags
            .iter()
            .all(|d| !d.message.contains("PartitionGrainRequiresRefreshIncremental")),
        "must not also raise the misdirecting PartitionGrainRequiresRefreshIncremental, got {diags:?}"
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
