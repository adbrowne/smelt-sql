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
