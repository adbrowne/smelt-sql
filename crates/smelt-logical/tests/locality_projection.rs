//! Phase 3 TDD: the partition-locality projection proof
//! (`docs/specs/model_properties.md` §"Partition-locality projection") —
//! `LocalityVerdict = Local | NotLocal{reason}` per `(cell × source)`,
//! composing the read-side bound (`derive_model_bounds`) and the reflected
//! write footprint (`reflect_footprint`), with cross-axis linkage admitted
//! only through an explicit, derivable predicate relating the source's
//! partition column to the output's — never a margin-inferred guess.

use std::collections::BTreeSet;

use smelt_logical::analysis::footprint::{reflect_footprint, FootprintResult};
use smelt_logical::analysis::locality_projection::{locality_verdict, LocalityVerdict};
use smelt_logical::analysis::source_bounds::{
    defining_expr_siblings, derive_cross_axis_links, derive_model_bounds, BoundContext,
    BoundResult, CrossAxisLink, Seconds,
};
use smelt_logical::maintenance::derive::{derive_maintenance_plan, FoldSpec, ModelInputs};
use smelt_logical::maintenance::{
    ColumnGroup, Grain, MutationProfile, OutputSpec, PartitionLocal, SourceFacts, Trigger,
};
use smelt_types::SqlFunction;

fn bounded(col: &str, before: Seconds, after: Seconds) -> BoundResult {
    BoundResult::Bounded {
        source_partition_col: col.to_string(),
        before,
        after,
    }
}

fn bounded_footprint(axis: &str, before: Seconds, after: Seconds) -> FootprintResult {
    FootprintResult::Bounded {
        output_partition_col: axis.to_string(),
        before,
        after,
    }
}

// ---------------------------------------------------------------------------
// 1. Same-axis source: bounded read + bounded reflected footprint → Local.
// ---------------------------------------------------------------------------

#[test]
fn same_axis_bounded_read_and_footprint_is_local() {
    let verdict = locality_verdict(
        &bounded("event_date", Seconds::ZERO, Seconds::ZERO),
        &bounded_footprint("event_date", Seconds::ZERO, Seconds::ZERO),
        Some("event_date"),
        Some("event_date"),
        CrossAxisLink::SameAxis,
    );
    assert_eq!(verdict, LocalityVerdict::Local);

    // The evidence derivation itself classifies a shared axis as SameAxis.
    let sql = "SELECT event_date, COUNT(*) AS n FROM smelt.sources.events GROUP BY event_date";
    let ctx = BoundContext::new().with_source("events", "event_date");
    let links = derive_cross_axis_links(sql, &ctx, "event_date");
    assert_eq!(links.get("events"), Some(&CrossAxisLink::SameAxis));
}

// ---------------------------------------------------------------------------
// 2. Cross-axis source linked by an explicit, derivable interval predicate
//    relating its partition column to the output axis → Local. The predicate
//    — not the margin — is the evidence.
// ---------------------------------------------------------------------------

#[test]
fn cross_axis_source_with_explicit_interval_predicate_is_local() {
    // Output partitioned on event_date (defined as CAST(event_ts AS DATE));
    // the source is arrival-partitioned. The WHERE band explicitly relates
    // arrival_date to the output axis's own defining expression.
    let sql = "SELECT event_id, CAST(event_ts AS DATE) AS event_date \
               FROM smelt.sources.bronze_events \
               WHERE arrival_date BETWEEN CAST(event_ts AS DATE) \
                                      AND CAST(event_ts AS DATE) + INTERVAL '2 days'";
    let ctx = BoundContext::new().with_source("bronze_events", "arrival_date");

    let links = derive_cross_axis_links(sql, &ctx, "event_date");
    assert_eq!(
        links.get("bronze_events"),
        Some(&CrossAxisLink::ExplicitPredicate),
        "the anchored interval band is explicit predicate evidence"
    );

    let bounds = derive_model_bounds(sql, &ctx);
    let footprints = reflect_footprint(sql, &ctx, Some("event_date"));
    let verdict = locality_verdict(
        &bounds["bronze_events"],
        &footprints["bronze_events"],
        Some("arrival_date"),
        Some("event_date"),
        links["bronze_events"],
    );
    assert_eq!(verdict, LocalityVerdict::Local);
}

// ---------------------------------------------------------------------------
// 3. Cross-axis source with NO derivable predicate relating the two columns
//    → NotLocal naming the missing link, even though a nonzero margin exists
//    elsewhere in the query (the false positive the old margin heuristic
//    admitted: `before > 0 || after > 0` as cross-axis evidence).
// ---------------------------------------------------------------------------

#[test]
fn cross_axis_source_without_predicate_is_not_local() {
    // A rolling window frame gives the snapshots source a genuine 1-day read
    // margin — but nothing relates snapshot_date to event_date.
    let sql = "SELECT user_id, \
               SUM(amount) OVER (ORDER BY snapshot_ts \
                 RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW) AS rolling, \
               CAST(snapshot_ts AS DATE) AS event_date \
               FROM smelt.sources.snapshots";
    let ctx = BoundContext::new().with_source("snapshots", "snapshot_date");

    let bounds = derive_model_bounds(sql, &ctx);
    assert!(
        matches!(&bounds["snapshots"], BoundResult::Bounded { before, .. } if *before > Seconds::ZERO),
        "precondition: the frame really does produce a nonzero margin, got {:?}",
        bounds["snapshots"]
    );

    let links = derive_cross_axis_links(sql, &ctx, "event_date");
    assert_eq!(links.get("snapshots"), Some(&CrossAxisLink::Absent));

    let footprints = reflect_footprint(sql, &ctx, Some("event_date"));
    let verdict = locality_verdict(
        &bounds["snapshots"],
        &footprints["snapshots"],
        Some("snapshot_date"),
        Some("event_date"),
        links["snapshots"],
    );
    match verdict {
        LocalityVerdict::NotLocal { reason } => assert!(
            reason.contains("no predicate links"),
            "reason must name the missing link, got: {reason}"
        ),
        LocalityVerdict::Local => panic!("margin alone must never link a cross-axis source"),
    }

    // And through the plan derivation: the cell's verdict is non-local.
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "t".to_string(),
            grain: Grain::Partition {
                partition_col: "event_date".to_string(),
            },
            skeleton_columns: BTreeSet::new(),
        },
        sources: vec![SourceFacts {
            name: "snapshots".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some("snapshot_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        }],
        column_groups: vec![],
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
    };
    let plan = derive_maintenance_plan(&inputs, &[Trigger::Backfill]);
    let cell = &plan.cells[0];
    assert!(
        matches!(&cell.partition_local, PartitionLocal::No { source, why }
            if source == "snapshots" && why.contains("no predicate links")),
        "expected the missing-link non-local verdict, got {:?}",
        cell.partition_local
    );
    assert!(cell.scans.is_empty(), "no clamp without a derived link");
}

// ---------------------------------------------------------------------------
// 2b. A downstream predicate anchored on a SIBLING column of the source's
//     partition column — a different alias in the SOURCE's own SELECT whose
//     defining expression is textually identical to the partition column's
//     own (e.g. a historical column name kept for a pre-existing consumer,
//     `MIN(x) AS legacy_name` alongside `MIN(x) AS declared_partition_col`)
//     — links exactly as an anchor on the partition column itself would.
//     `derive_cross_axis_links` never sees this without the sibling
//     registered on `BoundContext` (that's the previously-missing plumbing:
//     `defining_expr_siblings` over the UPSTREAM's own SQL, threaded through
//     `ModelEdge::clock_col_aliases`).
// ---------------------------------------------------------------------------

#[test]
fn defining_expr_siblings_finds_duplicate_aliased_expression() {
    // Mirrors `examples/web_analytics/models/silver/events_deduped.sql`'s
    // shape: two output aliases, `event_date` (kept for a pre-existing
    // consumer) and `first_seen_date` (the declared partition column),
    // assigned the exact same aggregate expression.
    let upstream_sql = "SELECT event_id, \
                         MIN(CAST(event_date AS DATE)) AS event_date, \
                         MIN(CAST(event_date AS DATE)) AS first_seen_date \
                         FROM smelt.sources.raw.events GROUP BY event_id";
    let siblings = defining_expr_siblings(upstream_sql, "first_seen_date");
    assert_eq!(
        siblings,
        vec!["event_date".to_string()],
        "the identically-defined sibling alias must be found"
    );

    // Symmetric: querying from the other alias finds the reverse sibling.
    let reverse = defining_expr_siblings(upstream_sql, "event_date");
    assert_eq!(reverse, vec!["first_seen_date".to_string()]);
}

#[test]
fn defining_expr_siblings_is_empty_for_a_column_with_no_duplicate() {
    let sql = "SELECT event_id, MIN(event_date) AS event_date, MIN(user_id) AS user_id \
               FROM smelt.sources.raw.events GROUP BY event_id";
    assert!(defining_expr_siblings(sql, "event_date").is_empty());
    // Unknown alias: no target expression to compare against, so no siblings.
    assert!(defining_expr_siblings(sql, "no_such_column").is_empty());
}

#[test]
fn cross_axis_source_links_via_registered_partition_col_alias() {
    // The downstream WHERE band anchors on `event_date` (the source's
    // sibling column) and `session_start_date` (the output's own axis) —
    // never mentioning the source's declared partition column
    // `first_seen_date` at all. Without the alias registered, the LHS
    // column-name match fails and the link is `Absent` (the regression this
    // pins: `docs/plans/20260715-composed-axes-conditional-maintenance.md`'s
    // `silver.sessions` ← `silver.events_deduped` composed-model edge).
    let downstream_sql = "SELECT device_id, \
                           CAST(event_ts AS DATE) AS session_start_date \
                           FROM smelt.silver.events_deduped \
                           WHERE event_date BETWEEN session_start_date \
                                              AND session_start_date + INTERVAL '1 day'";

    let mut ctx_without_alias = BoundContext::new();
    ctx_without_alias.add_source("silver.events_deduped", "first_seen_date");
    let links_without_alias =
        derive_cross_axis_links(downstream_sql, &ctx_without_alias, "session_start_date");
    assert_eq!(
        links_without_alias.get("silver.events_deduped"),
        Some(&CrossAxisLink::Absent),
        "no alias registered: the sibling-anchored band must not be seen"
    );

    let mut ctx_with_alias = BoundContext::new();
    ctx_with_alias.add_source("silver.events_deduped", "first_seen_date");
    ctx_with_alias
        .add_source_partition_col_aliases("silver.events_deduped", vec!["event_date".to_string()]);
    let links_with_alias =
        derive_cross_axis_links(downstream_sql, &ctx_with_alias, "session_start_date");
    assert_eq!(
        links_with_alias.get("silver.events_deduped"),
        Some(&CrossAxisLink::ExplicitPredicate),
        "the sibling-column alias must license the same explicit-predicate link \
         as the partition column's own name would"
    );
}

// ---------------------------------------------------------------------------
// 3b. A source whose partition column is literally the same name as the
//     output axis is linked by that identity alone — the SameAxis link
//     evidence is a corroborating derivation, not the sole admissible proof
//     of a shared axis.
// ---------------------------------------------------------------------------

#[test]
fn same_named_axis_is_local_even_without_same_axis_link_evidence() {
    let verdict = locality_verdict(
        &bounded("event_date", Seconds::ZERO, Seconds::ZERO),
        &bounded_footprint("event_date", Seconds::ZERO, Seconds::ZERO),
        Some("event_date"),
        Some("event_date"),
        CrossAxisLink::Absent,
    );
    assert_eq!(
        verdict,
        LocalityVerdict::Local,
        "a source and output partition column sharing a name are linked by identity alone, \
         independent of the separately-derived CrossAxisLink evidence"
    );
}

// ---------------------------------------------------------------------------
// 4. An Unbounded reflected footprint defeats locality regardless of the
//    read bound — the composition the spec requires.
// ---------------------------------------------------------------------------

#[test]
fn unbounded_footprint_is_not_local_even_with_bounded_read() {
    let verdict = locality_verdict(
        &bounded("event_date", Seconds::ZERO, Seconds::ZERO),
        &FootprintResult::Unbounded,
        Some("event_date"),
        Some("event_date"),
        CrossAxisLink::SameAxis,
    );
    match verdict {
        LocalityVerdict::NotLocal { reason } => assert!(
            reason.contains("write footprint"),
            "reason must name the footprint side, got: {reason}"
        ),
        LocalityVerdict::Local => {
            panic!("a bounded read must not outvote an unbounded write footprint")
        }
    }
}

// ---------------------------------------------------------------------------
// 5. Verdicts are per (cell × source): a cell with one local and one
//    non-local source carries both — the local source's derived clamp AND
//    the first-failure-wins non-local fold.
// ---------------------------------------------------------------------------

#[test]
fn verdicts_are_per_cell_per_source() {
    let sql = "SELECT e.event_date, e.amount, s.label \
               FROM smelt.sources.events e \
               JOIN smelt.sources.snapshots s ON s.user_id = e.user_id";
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "t".to_string(),
            grain: Grain::Partition {
                partition_col: "event_date".to_string(),
            },
            skeleton_columns: BTreeSet::new(),
        },
        sources: vec![
            SourceFacts {
                name: "events".to_string(),
                mutation: MutationProfile::AppendOnly,
                partition_col: Some("event_date".to_string()),
                unique_key: vec![],
                allow_full_scan: false,
            },
            SourceFacts {
                name: "snapshots".to_string(),
                mutation: MutationProfile::AppendOnly,
                partition_col: Some("snapshot_date".to_string()),
                unique_key: vec![],
                allow_full_scan: false,
            },
        ],
        column_groups: vec![ColumnGroup {
            columns: vec!["amount".to_string(), "label".to_string()],
            mutation_sensitivity: BTreeSet::new(),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
    };
    let plan = derive_maintenance_plan(&inputs, &[Trigger::Backfill]);
    let cell = &plan.cells[0];

    // The local source's verdict survives as its derived clamp…
    assert!(
        cell.scans.iter().any(|c| c.source == "events"),
        "the same-axis source keeps its derived clamp, got {:?}",
        cell.scans
    );
    // …while the non-local source decides the folded cell verdict
    // (first failure wins).
    assert!(
        matches!(&cell.partition_local, PartitionLocal::No { source, .. }
            if source == "snapshots"),
        "expected snapshots to decide the folded verdict, got {:?}",
        cell.partition_local
    );
}

// ---------------------------------------------------------------------------
// 6. The keyed-grain vacuous-locality residue (`project_source_link`'s
//    documented pre-proof policy for `output_partition_col: None`, kept
//    verbatim rather than routed through the partition-locality proof): a
//    clocked source's nonzero read margin alone links it — no cross-axis
//    predicate is consulted because there is no output partition axis to
//    pose the question against.
// ---------------------------------------------------------------------------

fn keyed_fold_plan(sql: &str) -> smelt_logical::maintenance::MaintenancePlan {
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "t".to_string(),
            grain: Grain::Key {
                unique_key: vec!["user_id".to_string()],
            },
            skeleton_columns: BTreeSet::new(),
        },
        sources: vec![SourceFacts {
            name: "payments".to_string(),
            mutation: MutationProfile::MutableSnapshot,
            partition_col: Some("pay_date".to_string()),
            unique_key: vec![],
            allow_full_scan: true,
        }],
        column_groups: vec![ColumnGroup {
            columns: vec!["total".to_string()],
            mutation_sensitivity: BTreeSet::from(["payments".to_string()]),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: Some(FoldSpec {
            add_columns: vec![("total".to_string(), SqlFunction::Sum)],
        }),
        old_columns: Vec::new(),
        old_sql: None,
    };
    derive_maintenance_plan(
        &inputs,
        &[Trigger::UpstreamMutation {
            source: "payments".to_string(),
        }],
    )
}

/// A keyed-grain source with a genuine nonzero read margin (a rolling window
/// over its own clock) links: the cell carries the derived clamp, mirroring
/// the read reach exactly (`ScanClamp::before` = the derived margin,
/// `after: ZERO` since the frame is one-sided).
#[test]
fn keyed_grain_source_with_nonzero_margin_links() {
    let sql = "SELECT user_id, SUM(amount) OVER (PARTITION BY user_id ORDER BY pay_date \
               RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW) AS total \
               FROM smelt.sources.payments";
    let plan = keyed_fold_plan(sql);
    let cell = &plan.cells[0];
    assert_eq!(
        cell.scans,
        vec![smelt_logical::maintenance::ScanClamp {
            source: "payments".to_string(),
            column: "pay_date".to_string(),
            before: Seconds::days(1),
            after: Seconds::ZERO,
        }],
        "a nonzero read margin on a keyed-grain source must produce the mirrored clamp, \
         got {:?}",
        cell.scans
    );
    assert_eq!(cell.partition_local, PartitionLocal::Yes);
}

/// A keyed-grain source whose margin is entirely on the `after` side
/// (`before: ZERO`) still links — the `before > ZERO || after > ZERO`
/// disjunction must check EACH side independently; a fixture with a nonzero
/// `before` alone (the previous test) short-circuits past the `after` half
/// of that check, which is what this pins.
#[test]
fn keyed_grain_source_with_after_only_margin_links() {
    let sql = "SELECT user_id, SUM(amount) OVER (PARTITION BY user_id ORDER BY pay_date \
               RANGE BETWEEN CURRENT ROW AND INTERVAL '1 day' FOLLOWING) AS total \
               FROM smelt.sources.payments";
    let plan = keyed_fold_plan(sql);
    let cell = &plan.cells[0];
    assert_eq!(
        cell.scans,
        vec![smelt_logical::maintenance::ScanClamp {
            source: "payments".to_string(),
            column: "pay_date".to_string(),
            before: Seconds::ZERO,
            after: Seconds::days(1),
        }],
        "an after-only margin must still produce the mirrored clamp, got {:?}",
        cell.scans
    );
    assert_eq!(cell.partition_local, PartitionLocal::Yes);
}

/// A keyed-grain source with NO read margin at all (a plain grouped
/// aggregate, no window, no WHERE band) does not link — the vacuous-locality
/// residue policy requires a genuine nonzero margin, not merely a `Bounded`
/// verdict at zero.
#[test]
fn keyed_grain_source_with_zero_margin_does_not_link() {
    let sql = "SELECT user_id, SUM(amount) AS total FROM smelt.sources.payments GROUP BY user_id";
    let plan = keyed_fold_plan(sql);
    let cell = &plan.cells[0];
    assert!(
        cell.scans.is_empty(),
        "a zero read margin must not produce a clamp, got {:?}",
        cell.scans
    );
    match &cell.partition_local {
        PartitionLocal::No { source, why } => {
            assert_eq!(source, "payments");
            assert!(
                why.contains("no predicate links"),
                "expected the missing-link reason, got: {why}"
            );
        }
        other => panic!("expected a zero-margin source to fail to link, got {other:?}"),
    }
}
