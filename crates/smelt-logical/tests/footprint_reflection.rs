//! TDD tests for footprint reflection — the write-scope dual of the
//! read-bound derivation (`docs/specs/model_properties.md` §"Footprint
//! reflection / bounded write footprint").
//!
//! `reflect_footprint` asks: how far across the output's own partition
//! column must a write triggered by an input delta spread?  For the shapes
//! today's clamps admit (a windowed reach) the answer is the mirror of the
//! read reach; a stored trajectory column (running/cumulative fold over the
//! output axis) is the canonical `Unbounded` case the mirror cannot
//! express; a `NotDerivable`/`Unbounded` read bound reflects fail-closed to
//! `NotDerivable`/`Unbounded`, never a guessed mirror.

use smelt_logical::analysis::footprint::{reflect_footprint, FootprintResult};
use smelt_logical::analysis::source_bounds::{BoundContext, Seconds};

/// For a plain windowed aggregate — the shapes today's clamps admit — the
/// derived footprint equals the current mirror `(after, before)` of the read
/// reach: the equivalence that keeps every existing clamp consumer stable.
#[test]
fn symmetric_window_reach_reflects_to_the_mirror() {
    // Symmetric Form A frame: read reach (1d, 1d) reflects to write
    // footprint (1d, 1d).
    let sql = "SELECT event_date, AVG(x) OVER (PARTITION BY event_date ORDER BY event_ts \
               RANGE BETWEEN INTERVAL '1 day' PRECEDING AND INTERVAL '1 day' FOLLOWING) AS smoothed \
               FROM smelt.silver.events";
    let ctx = BoundContext::new().with_source("silver.events", "event_ts");
    let fps = reflect_footprint(sql, &ctx, Some("event_date"));
    assert_eq!(
        fps.get("silver.events"),
        Some(&FootprintResult::Bounded {
            output_partition_col: "event_date".to_string(),
            before: Seconds::days(1),
            after: Seconds::days(1),
        }),
        "symmetric read reach must reflect to the identical symmetric footprint"
    );

    // Asymmetric Form B band: read reach (before=0, after=14d) reflects to
    // footprint (before=14d, after=0) — a conversion at t repairs events
    // over [t − 14d, t].  This is the swap the mirror performs, so a
    // symmetric-only fixture could not catch a before/after transposition.
    let sql = "SELECT e.event_date, COUNT(c.conversion_id) AS conversions \
               FROM smelt.silver.events e \
               LEFT JOIN smelt.silver.conversions c \
                 ON c.conversion_date BETWEEN e.event_date AND e.event_date + INTERVAL '14 day' \
               GROUP BY e.event_date";
    let ctx = BoundContext::new().with_source("silver.conversions", "conversion_date");
    let fps = reflect_footprint(sql, &ctx, Some("event_date"));
    assert_eq!(
        fps.get("silver.conversions"),
        Some(&FootprintResult::Bounded {
            output_partition_col: "event_date".to_string(),
            before: Seconds::days(14),
            after: Seconds::ZERO,
        }),
        "read (0, 14d) must reflect to write (14d, 0)"
    );
}

/// A cumulative/running-total column — a running fold over the output axis —
/// is still mutable arbitrarily far downstream under late input: a late row
/// at time t changes the stored running value of every output row at or
/// after t.  The reflected footprint is `Unbounded`; the read-side mirror
/// (which would derive `(0, 0)` here — this SQL has no RANGE frame, no
/// LAG/LEAD, no `UNBOUNDED` keyword) cannot express this.
#[test]
fn trajectory_column_reflects_unbounded() {
    let sql = "SELECT event_date, user_id, \
               SUM(amount) OVER (PARTITION BY user_id ORDER BY event_date) AS running_total \
               FROM smelt.silver.events";
    let ctx = BoundContext::new().with_source("silver.events", "event_date");
    let fps = reflect_footprint(sql, &ctx, Some("event_date"));
    assert_eq!(
        fps.get("silver.events"),
        Some(&FootprintResult::Unbounded),
        "a running fold over the output axis is the canonical Unbounded footprint"
    );

    // The inverse-needing running fold (MAX) is a trajectory too: a late
    // input can raise the running maximum for every later output row.
    let sql = "SELECT event_date, \
               MAX(amount) OVER (ORDER BY event_date) AS running_max \
               FROM smelt.silver.events";
    let fps = reflect_footprint(sql, &ctx, Some("event_date"));
    assert_eq!(
        fps.get("silver.events"),
        Some(&FootprintResult::Unbounded),
        "an order-monotone running aggregate over the output axis is a trajectory column"
    );
}

/// A source whose read bound is `NotDerivable` reflects to `NotDerivable` —
/// never a guessed mirror.  Likewise an `Unbounded` read reflects to
/// `Unbounded`.  Absence of a derivable proof is a rejection
/// (`model_properties.md` §Constraints), on the write side exactly as on the
/// read side.
#[test]
fn underivable_read_bound_reflects_not_derivable() {
    // Symbolic (calendar-relative) interval: the read bound is NotDerivable.
    let sql = "SELECT event_date, COUNT(*) AS cnt FROM smelt.silver.events \
               WHERE event_ts >= event_date - INTERVAL '1 month' \
               GROUP BY event_date";
    let ctx = BoundContext::new().with_source("silver.events", "event_ts");
    let fps = reflect_footprint(sql, &ctx, Some("event_date"));
    assert_eq!(
        fps.get("silver.events"),
        Some(&FootprintResult::NotDerivable),
        "a NotDerivable read bound must reflect to NotDerivable, never a guessed mirror"
    );

    // Unbounded read (RANGE ... UNBOUNDED PRECEDING): reflects to Unbounded.
    let sql = "SELECT event_date, SUM(x) OVER (PARTITION BY event_date ORDER BY event_ts \
               RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS cume \
               FROM smelt.silver.events";
    let fps = reflect_footprint(sql, &ctx, Some("event_date"));
    assert_eq!(
        fps.get("silver.events"),
        Some(&FootprintResult::Unbounded),
        "an Unbounded read bound must reflect to Unbounded"
    );
}

/// The `Bounded` verdict carries the OUTPUT's partition column — the axis a
/// write spreads across — which is distinct from `BoundResult::Bounded`'s
/// `source_partition_col` (the axis the read is clamped on).
#[test]
fn bounded_verdict_names_the_output_partition_column() {
    let sql = "SELECT event_date, COUNT(*) AS cnt FROM smelt.silver.events \
               WHERE event_ts BETWEEN event_date - INTERVAL '2 hours' \
                                  AND event_date + INTERVAL '1 hours' \
               GROUP BY event_date";
    // The source's read axis is event_ts; the output's write axis is
    // event_date — the verdict must name the latter.
    let ctx = BoundContext::new().with_source("silver.events", "event_ts");
    let fps = reflect_footprint(sql, &ctx, Some("event_date"));
    match fps.get("silver.events") {
        Some(FootprintResult::Bounded {
            output_partition_col,
            before,
            after,
        }) => {
            assert_eq!(
                output_partition_col, "event_date",
                "the verdict carries the output's own axis"
            );
            assert_ne!(
                output_partition_col, "event_ts",
                "the output axis is distinct from the source's read column"
            );
            // Read (before=2h, after=1h) mirrors to write (before=1h, after=2h).
            assert_eq!(*before, Seconds::hours(1));
            assert_eq!(*after, Seconds::hours(2));
        }
        other => panic!("expected Bounded naming the output axis, got {other:?}"),
    }
}

/// A scalar subquery embedded in `WHERE` (not a FROM item, so the
/// walk-normalized tree stays "supported") may itself contain a windowed
/// aggregate ordered by the output axis — but it is not one of the model's
/// own output columns, so it must never be treated as a trajectory column.
/// The walk-backed derivation composes only the outer SELECT's own
/// select-list items; a whole-syntax-tree scan (the `has_unsupported`
/// fallback, wrongly taken for a supported tree) would wrongly pick up the
/// subquery's window and misclassify this as `Unbounded`. This pins the
/// walk/fallback dispatch — not just the trajectory leaf classifier.
#[test]
fn window_inside_a_where_subquery_is_not_a_trajectory_of_the_outer_select() {
    let sql = "SELECT event_date, COUNT(*) AS cnt FROM smelt.silver.events \
               WHERE event_ts BETWEEN event_date - INTERVAL '2 hours' \
                                  AND event_date + INTERVAL '1 hours' \
                 AND 1 > (SELECT SUM(x) OVER (ORDER BY event_date) FROM smelt.silver.other) \
               GROUP BY event_date";
    let ctx = BoundContext::new().with_source("silver.events", "event_ts");
    let fps = reflect_footprint(sql, &ctx, Some("event_date"));
    assert_eq!(
        fps.get("silver.events"),
        Some(&FootprintResult::Bounded {
            output_partition_col: "event_date".to_string(),
            before: Seconds::hours(1),
            after: Seconds::hours(2),
        }),
        "a window inside an unrelated WHERE subquery must not be mistaken for the outer \
         select's own trajectory column"
    );
}

/// A `RECURSIVE` CTE's body is opaque to the walk by construction
/// (`analysis::walk::normalize_ctes`: a recursive CTE's self-reference would
/// be misread as a base table, so its body normalizes to a bare
/// `QueryNode::Unsupported` carrying no captured substructure at all — even
/// though the CTE's body verdict is unconditionally folded into every
/// consuming scope's children, `walk_node` on an `Unsupported` node never
/// recurses into it, so a trajectory column inside that body is invisible to
/// the walk regardless of whether the CTE is referenced). This makes the
/// whole tree `has_unsupported() == true`, so real (un-mutated) code takes
/// the coverage-preserving CST-fallback path — a flat syntactic scan that,
/// unaware of `normalize`'s judgment, still finds the recursive CTE's own
/// raw `SELECT_STMT` text and its trajectory column, correctly reporting
/// `Unbounded`. This pins the walk/fallback dispatch itself
/// (`footprint.rs:116`'s guard): forcing the walk path unconditionally
/// would instead see only the opaque `Unsupported` placeholder — never the
/// hidden trajectory — and wrongly report `Bounded`.
#[test]
fn trajectory_hidden_inside_a_recursive_cte_body_forces_the_fallback() {
    let sql = "WITH RECURSIVE rec AS ( \
                 SELECT event_date, SUM(x) OVER (ORDER BY event_date) AS running_total \
                 FROM smelt.silver.other \
               ) \
               SELECT event_date, COUNT(*) AS cnt FROM smelt.silver.events \
               WHERE event_ts BETWEEN event_date - INTERVAL '1 day' AND event_date + INTERVAL '1 day' \
               GROUP BY event_date";
    let ctx = BoundContext::new().with_source("silver.events", "event_ts");
    let fps = reflect_footprint(sql, &ctx, Some("event_date"));
    assert_eq!(
        fps.get("silver.events"),
        Some(&FootprintResult::Unbounded),
        "a tree the walk cannot normalize must take the CST-fallback trajectory scan, not \
         the walk composition — the two disagree exactly when a trajectory column is hidden \
         inside a construct (here a RECURSIVE CTE body) the walk treats as fully opaque"
    );
}

/// A running fold whose `ORDER BY` leads with a column other than the
/// output's own partition axis is not a trajectory of that axis — only a
/// fold ordered along the axis itself has the "late input at t rewrites
/// every later output row" shape. This pins `expr_is_column`'s role in
/// distinguishing the axis column from any other `ORDER BY` column.
#[test]
fn window_ordered_by_non_axis_column_is_not_a_trajectory() {
    let sql = "SELECT event_date, event_ts, \
               SUM(amount) OVER (PARTITION BY event_date ORDER BY event_ts) AS running \
               FROM smelt.silver.events \
               WHERE event_ts BETWEEN event_date - INTERVAL '1 day' AND event_date + INTERVAL '1 day'";
    let ctx = BoundContext::new().with_source("silver.events", "event_ts");
    let fps = reflect_footprint(sql, &ctx, Some("event_date"));
    assert_eq!(
        fps.get("silver.events"),
        Some(&FootprintResult::Bounded {
            output_partition_col: "event_date".to_string(),
            before: Seconds::days(1),
            after: Seconds::days(1),
        }),
        "a running fold ordered by a non-axis column must not be classified as a trajectory \
         column of the output axis"
    );
}
