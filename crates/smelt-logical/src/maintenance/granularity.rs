//! Declared-granularity **checking** (`maintenance_plan.md` §"The graph
//! layer": "the classifier only *checks* the declaration, e.g. against a
//! `date_trunc` grouping"): a leaf classifier over a model's own single
//! top-level `SELECT`, in the same family as `grouping.rs`/`skeleton.rs`.
//!
//! The graph layer's edge grain is the **declared** `timeseries.granularity`
//! — never derived from the SQL, per the ratified P3 decision
//! (`maintenance_plan.md` §Design "Grain is declared"). What P3 does license
//! is a *check*: is the declaration a **narrowing** of what the model's own
//! `partition_column` projection actually derives?
//!
//! **Directional, per widen-never-narrow** (`maintenance_plan.md` §Design
//! "Widen-never-narrow": "Widening costs compute; narrowing costs
//! correctness silently"). `Granularity`'s `Ord` is increasing coarseness
//! (`Hour` finest … `Year` coarsest); a declaration **coarser than or equal
//! to** the derived unit is a safe widen — the graph layer schedules at a
//! grid no finer than the data really supports, over-running rather than
//! silently misreading a boundary. A declaration **finer than** the
//! derived unit is the narrowing hazard this check refuses: it promises
//! the graph layer a grid resolution the model's own grouping cannot
//! actually distinguish (e.g. declaring `hour` while the SQL only
//! `date_trunc`s to `day` — every declared hour bucket within a day would
//! silently collapse onto one row).
//!
//! Reuses the same structural trace `trace_event_time` and
//! `smelt-runtime`'s run-window-vs-partition-grid check already use
//! (`analysis::monotonicity::classify_truncation_grid_unit`) — not a second
//! independent parser. Fails open (no mismatch reported) when the
//! `partition_column` projection can't be located in the SELECT list, or
//! when its shape doesn't resolve to a known grid unit — undecidable, not a
//! positive disproof, matching `classify_truncation_grid_unit`'s own
//! posture.

use smelt_core::Granularity;

use crate::analysis::monotonicity::classify_truncation_grid_unit;
use crate::analysis::{analyze_select, SelectItemKind};

/// A declared `timeseries.granularity` that **narrows** the model's own
/// derived grouping/truncation unit (`declared` is strictly finer than
/// `actual`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GranularityMismatch {
    pub declared: Granularity,
    pub actual: Granularity,
}

/// Check `declared` against the truncation/grid unit `partition_column`'s
/// SELECT-list projection actually derives to in `sql`'s single top-level
/// `SELECT`. Returns `Some(mismatch)` only when a grid unit was positively
/// derived and `declared` is strictly finer than it (a narrowing —
/// `declared < actual` under `Granularity`'s increasing-coarseness `Ord`);
/// `None` when `declared` is coarser than or equal to the derived unit (a
/// safe widen), when the projection can't be found (a
/// CTE/set-operation/derived-table shape outside this classifier's scope —
/// see module docs), or when its shape doesn't resolve to a known
/// truncation unit (undecidable).
pub fn check_declared_granularity(
    sql: &str,
    partition_column: &str,
    declared: Granularity,
) -> Option<GranularityMismatch> {
    let analysis = analyze_select(sql)?;
    let expr = analysis.items.into_iter().find_map(|item| match item {
        SelectItemKind::CountDistinct { alias, expr, .. }
        | SelectItemKind::OtherAggregate { alias, expr, .. }
        | SelectItemKind::GroupByKey { alias, expr, .. }
            if alias == partition_column =>
        {
            Some(expr)
        }
        _ => None,
    })?;
    let actual = classify_truncation_grid_unit(&expr)?;
    if declared < actual {
        Some(GranularityMismatch { declared, actual })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agrees_with_declared_day_grouping() {
        let sql = "SELECT date_trunc('day', event_time) AS order_date, SUM(amount) AS total \
                    FROM smelt.sources.orders GROUP BY 1";
        assert!(check_declared_granularity(sql, "order_date", Granularity::Day).is_none());
    }

    #[test]
    fn declaring_finer_than_actual_grouping_is_a_narrowing_error() {
        // Declared `hour` while the SQL only truncates to `day` — the
        // declaration promises a resolution the grouping cannot actually
        // distinguish.
        let sql = "SELECT date_trunc('day', event_time) AS order_date, SUM(amount) AS total \
                    FROM smelt.sources.orders GROUP BY 1";
        let mismatch = check_declared_granularity(sql, "order_date", Granularity::Hour)
            .expect("declaring hour while actual grouping is day should be a narrowing error");
        assert_eq!(mismatch.declared, Granularity::Hour);
        assert_eq!(mismatch.actual, Granularity::Day);
    }

    #[test]
    fn declaring_coarser_than_actual_grouping_is_a_safe_widen() {
        // Declared `week` while the SQL truncates to `day` — coarser than
        // actual, a safe over-running widen, never an error.
        let sql = "SELECT date_trunc('day', event_time) AS order_date, SUM(amount) AS total \
                    FROM smelt.sources.orders GROUP BY 1";
        assert!(check_declared_granularity(sql, "order_date", Granularity::Week).is_none());
    }

    #[test]
    fn undecidable_shape_fails_open() {
        // Bare column, no truncation layer at all — undecidable, not a
        // positive disproof.
        let sql = "SELECT order_date, SUM(amount) AS total FROM smelt.sources.orders \
                    GROUP BY 1";
        assert!(check_declared_granularity(sql, "order_date", Granularity::Day).is_none());
    }

    #[test]
    fn missing_projection_fails_open() {
        let sql = "SELECT customer_id, SUM(amount) AS total FROM smelt.sources.orders \
                    GROUP BY 1";
        assert!(check_declared_granularity(sql, "order_date", Granularity::Day).is_none());
    }
}
