use super::*;

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
        None,
        &[],
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
        None,
        &[],
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
        None,
        &[],
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

/// Phase 26a: a `grain: key` model's declared `timeseries.partition_column`
/// must arrive at the real derivation as `ModelInputs::keyed_time_axis` —
/// not just in a unit test's hand-built `ModelInputs` literal, but through
/// the production Salsa wrapper (`crates/smelt-db/src/queries/
/// maintenance.rs`). Evidenced by a scan clamp that carries a derived write
/// footprint (`docs/specs/model_properties.md` §"Footprint reflection /
/// bounded write footprint") — a bare keyed model with no declared axis
/// never gets one (`bare_keyed_output_clamp_carries_no_footprint_claim`,
/// `crates/smelt-logical/tests/keyed_footprint.rs`).
#[test]
fn keyed_model_time_axis_reaches_plan_derivation() {
    let payments_source = r#"
description: Payments, append-only, clocked on pay_date.
mutation_profile: append_only
timeseries:
  event_time_column: pay_date
  partition_column: pay_date
  granularity: day
columns:
  - { name: user_id, type: INTEGER, nullable: false }
  - { name: amount, type: DOUBLE, nullable: false }
  - { name: pay_date, type: DATE, nullable: false }
"#;
    let model = r#"---
materialization: table
refresh: incremental
grain: key
timeseries:
  event_time_column: first_pay_date
  partition_column: first_pay_date
  granularity: day
unique_key: [user_id, first_pay_date]
---
SELECT
    user_id,
    pay_date AS first_pay_date,
    MIN(amount) AS amount
FROM smelt.sources.payments
GROUP BY user_id, pay_date
"#;

    let result = plan_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/payments.yml", payments_source),
            ("models/lifetime_spend.sql", model),
        ],
        "lifetime_spend",
    );

    assert!(
        result.plan.refusals.is_empty(),
        "expected no refusals, got {:?}",
        result.plan.refusals
    );
    let clamp = result
        .plan
        .cells
        .iter()
        .flat_map(|c| &c.scans)
        .find(|c| c.source == "payments")
        .unwrap_or_else(|| {
            panic!(
                "expected a scan clamp on 'payments', got {:?}",
                result.plan.cells
            )
        });
    assert!(
        clamp.footprint().is_some(),
        "the declared timeseries.partition_column must reach ModelInputs::keyed_time_axis and \
         derive a write footprint on the clamp, got {:?}",
        clamp.footprint()
    );
}

/// `KeyedRetractableContribution` (`docs/specs/incremental_shapes.md`
/// §"Enrichment joins", §Diagnostics): a `grain: key` model folds `SUM` over
/// a value read off a JOINed `mutable_snapshot` dimension that declares no
/// `unique_key` — the join's fan-out cannot be proven one-to-one, and `SUM`
/// is a decrementing aggregate, so the enrichment contribution is
/// retractable. The same missing `unique_key` also makes repair's
/// affected-key discovery fail, so the technique-admission refusal fires
/// too — this diagnostic is additive alongside it, never a replacement.
#[test]
fn keyed_retractable_contribution_is_an_error_diagnostic() {
    let orders_source = r#"
description: Orders, append-only, unclocked.
mutation_profile: append_only
columns:
  - { name: order_id, type: INTEGER, nullable: false }
  - { name: customer_id, type: INTEGER, nullable: false }
"#;
    let customers_source = r#"
description: Customer dimension, mutable snapshot, no declared unique_key.
mutation_profile: mutable_snapshot
columns:
  - { name: customer_id, type: INTEGER, nullable: false }
  - { name: discount, type: DOUBLE, nullable: false }
"#;
    let model = r#"---
materialization: table
refresh: incremental
grain: key
unique_key: [customer_id]
---
SELECT
    o.customer_id,
    SUM(c.discount) AS total_discount
FROM smelt.sources.orders o
JOIN smelt.sources.customers c ON o.customer_id = c.customer_id
GROUP BY o.customer_id
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/orders.yml", orders_source),
            ("models/sources/customers.yml", customers_source),
            ("models/customer_totals.sql", model),
        ],
        "customer_totals",
    );

    let retractable: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::KeyedRetractableContribution))
        .collect();
    assert_eq!(
        retractable.len(),
        1,
        "expected exactly one KeyedRetractableContribution, got {diags:?}"
    );
    assert_eq!(
        retractable[0].severity,
        smelt_db::DiagnosticSeverity::Error,
        "KeyedRetractableContribution must be an Error, never silent"
    );
    assert!(
        retractable[0].message.contains("customers"),
        "message must name the failing source, got: {}",
        retractable[0].message
    );
    assert!(
        retractable[0].message.contains("materialized_view"),
        "message must steer toward refresh: materialized_view or DAG composition, got: {}",
        retractable[0].message
    );
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(DiagnosticCode::MaintenanceNoAdmissibleTechnique)),
        "the pre-existing MaintenanceNoAdmissibleTechnique refusal must still fire, got {diags:?}"
    );
}
