//! Window-independence / ordered-execution — self-edge detection over the
//! model DAG.
//!
//! See `docs/specs/model_properties.md` §"Derived proofs" →
//! "Window-independence / ordered-execution" and `docs/specs/incremental_models.md`
//! §"Window independence and self-referential models". A model that reads
//! only external sources is parallelisable across its windows/partitions
//! (`WindowIndependent`). A model with a self-edge — it reads its own prior
//! output via `smelt.<self>` — must build its windows strictly in temporal
//! order (`Ordered`), but only when the self-reference provably converges
//! partition-by-partition (a backward-bounded read of prior partitions with
//! no forward margin and a strictly positive backward reach — never a
//! forward read, an unbounded/whole-history scan, or a same-partition
//! (zero-backward, circular) read). This is a signal only: the
//! ordered-backfill chunker that consumes it is batched-local (L4).

use crate::analysis::source_bounds::{derive_model_bounds, BoundContext, BoundResult, Seconds};

/// Verdict for a model's window-independence / ordered-execution property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowIndependence {
    /// The model reads only external sources — its windows/partitions may be
    /// built in parallel, in any order.
    WindowIndependent,
    /// The model has a self-edge proven to converge partition-by-partition (a
    /// backward-bounded read of its own prior output) — its windows must be
    /// built strictly in temporal order.
    Ordered,
    /// A self-edge exists but does not provably converge
    /// partition-by-partition (a forward read, a same-partition/circular
    /// read with no backward reach, an unbounded/whole-history scan, or an
    /// underivable bound). Fail-closed: never silently treated as
    /// `Ordered`.
    Refused { reason: String },
}

/// Prove the window-independence / ordered-execution verdict for a model.
///
/// `model_name` is the model's own name, as it would appear in another
/// model's `refs` (`ModelGraph`'s `ModelInfo.refs`, e.g.
/// `"marts.running_balance"`). `refs` is this model's *own* `smelt.ref()`
/// list — a self-edge is `refs` containing `model_name`. `self_partition_col`
/// is the model's own declared `timeseries.partition_column`; `None` when the
/// model has no `timeseries:` block, which fails closed below (a clockless
/// self-reference can never be proven to converge). `sql` is the model's own
/// (expanded) SQL body, walked for the self-reference's bound the same way
/// [`derive_model_bounds`] walks any other source reference.
pub fn window_independence(
    model_name: &str,
    refs: &[String],
    self_partition_col: Option<&str>,
    sql: &str,
) -> WindowIndependence {
    match self_edge_bound_days(model_name, refs, self_partition_col, sql) {
        None => WindowIndependence::WindowIndependent,
        Some(Ok(_)) => WindowIndependence::Ordered,
        Some(Err(reason)) => WindowIndependence::Refused { reason },
    }
}

/// The day-unrolled self-edge's own backward reach — the `before_days` the
/// propagation graph (`crates/smelt-logical/src/maintenance/propagate.rs`)
/// applies once when widening a requirement up a self-edge
/// (`incremental_models.md` §"Time-unrolled self-edges"). Shares
/// [`self_edge_bound_days`] with [`window_independence`] so the two verdicts
/// — "may this model execute in ordered-backfill mode" and "may this
/// model's self-edge be admitted into the propagation graph" — can never
/// diverge: `Ordered` here always means `Ok` there, and vice versa.
///
/// `Err(model_name has no self-edge)` when `refs` does not contain
/// `model_name` — a self-edge clamp is meaningless to ask for absent one.
pub fn self_edge_clamp(
    model_name: &str,
    refs: &[String],
    self_partition_col: Option<&str>,
    sql: &str,
) -> Result<i64, String> {
    self_edge_bound_days(model_name, refs, self_partition_col, sql).unwrap_or_else(|| {
        Err(format!(
            "model '{model_name}' has no self-edge — refs does not contain its own name"
        ))
    })
}

/// `None` when `refs` carries no self-edge at all; `Some(Ok(before_days))`
/// for a proven backward-bounded self-edge (whole days, ceiled outward, with
/// a strictly positive backward reach); `Some(Err(reason))` for every other
/// case — no declared partition column, a forward read, a same-partition
/// (zero-backward, circular) read, an unbounded/whole-history scan, or an
/// underivable bound.
fn self_edge_bound_days(
    model_name: &str,
    refs: &[String],
    self_partition_col: Option<&str>,
    sql: &str,
) -> Option<Result<i64, String>> {
    if !refs.iter().any(|r| r == model_name) {
        return None;
    }

    let Some(partition_col) = self_partition_col else {
        return Some(Err(format!(
            "model '{model_name}' has a self-edge but declares no timeseries \
             partition column to prove convergence"
        )));
    };

    let ctx = BoundContext::new().with_source(model_name, partition_col);
    let bound = derive_model_bounds(sql, &ctx)
        .remove(model_name)
        .unwrap_or(BoundResult::NotDerivable);

    Some(match bound {
        BoundResult::Bounded { after, before, .. } if after == Seconds::ZERO && before.0 > 0 => {
            Ok(before.0.div_ceil(86_400) as i64)
        }
        BoundResult::Bounded { after, before, .. } if after == Seconds::ZERO && before.0 == 0 => {
            Err(format!(
                "model '{model_name}' self-edge reads only its own current partition \
                 — circular, not convergent partition-by-partition"
            ))
        }
        BoundResult::Bounded { after, .. } => Err(format!(
            "model '{model_name}' self-edge reads {} forward of the current partition \
             — not provably convergent",
            after.to_iso8601()
        )),
        BoundResult::Unbounded => Err(format!(
            "model '{model_name}' self-edge reads unbounded/whole history \
             — not provably convergent partition-by-partition"
        )),
        BoundResult::NotDerivable => Err(format!(
            "model '{model_name}' self-edge's bound could not be derived from its SQL"
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_self_edge_is_window_independent() {
        let refs = vec!["silver.transactions".to_string()];
        let sql = "SELECT * FROM smelt.silver.transactions";
        assert_eq!(
            window_independence("marts.running_balance", &refs, Some("partition_date"), sql),
            WindowIndependence::WindowIndependent
        );
    }

    #[test]
    fn backward_bounded_self_edge_is_ordered() {
        let refs = vec![
            "marts.running_balance".to_string(),
            "silver.transactions".to_string(),
        ];
        let sql = "SELECT bal.balance + t.amount AS balance \
                   FROM smelt.marts.running_balance bal \
                   JOIN smelt.silver.transactions t ON bal.acct_id = t.acct_id \
                   WHERE bal.partition_date >= t.partition_date - INTERVAL '1 day' \
                     AND bal.partition_date < t.partition_date";
        assert_eq!(
            window_independence("marts.running_balance", &refs, Some("partition_date"), sql),
            WindowIndependence::Ordered
        );
    }

    #[test]
    fn forward_reading_self_edge_is_refused_fail_closed() {
        let refs = vec!["marts.running_balance".to_string()];
        let sql = "SELECT bal.balance AS balance \
                   FROM smelt.marts.running_balance bal \
                   WHERE bal.partition_date <= m.partition_date + INTERVAL '1 day'";
        let verdict =
            window_independence("marts.running_balance", &refs, Some("partition_date"), sql);
        match verdict {
            WindowIndependence::Refused { reason } => {
                assert!(
                    reason.contains("marts.running_balance"),
                    "reason must name the self-edge: {reason}"
                );
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn whole_history_self_edge_is_refused_fail_closed() {
        let refs = vec!["marts.running_balance".to_string()];
        let sql = "SELECT SUM(bal.balance) OVER (\
                       ORDER BY bal.partition_date \
                       RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW\
                   ) AS balance \
                   FROM smelt.marts.running_balance bal";
        let verdict =
            window_independence("marts.running_balance", &refs, Some("partition_date"), sql);
        assert!(matches!(verdict, WindowIndependence::Refused { .. }));
    }

    #[test]
    fn self_edge_without_declared_partition_column_is_refused_fail_closed() {
        let refs = vec!["marts.running_balance".to_string()];
        let sql = "SELECT bal.balance AS balance FROM smelt.marts.running_balance bal";
        let verdict = window_independence("marts.running_balance", &refs, None, sql);
        assert!(matches!(verdict, WindowIndependence::Refused { .. }));
    }

    #[test]
    fn self_edge_clamp_returns_backward_reach_for_an_ordered_self_edge() {
        let refs = vec![
            "marts.running_balance".to_string(),
            "silver.transactions".to_string(),
        ];
        let sql = "SELECT bal.balance + t.amount AS balance \
                   FROM smelt.marts.running_balance bal \
                   JOIN smelt.silver.transactions t ON bal.acct_id = t.acct_id \
                   WHERE bal.partition_date >= t.partition_date - INTERVAL '1 day' \
                     AND bal.partition_date < t.partition_date";
        assert_eq!(
            window_independence("marts.running_balance", &refs, Some("partition_date"), sql),
            WindowIndependence::Ordered
        );
        assert_eq!(
            self_edge_clamp("marts.running_balance", &refs, Some("partition_date"), sql),
            Ok(1)
        );
    }

    #[test]
    fn self_edge_clamp_returns_the_same_refusal_reason_as_window_independence() {
        let refs = vec!["marts.running_balance".to_string()];
        let sql = "SELECT bal.balance AS balance \
                   FROM smelt.marts.running_balance bal \
                   WHERE bal.partition_date <= m.partition_date + INTERVAL '1 day'";
        let verdict =
            window_independence("marts.running_balance", &refs, Some("partition_date"), sql);
        let WindowIndependence::Refused {
            reason: verdict_reason,
        } = verdict
        else {
            panic!("expected Refused");
        };
        let clamp_err =
            self_edge_clamp("marts.running_balance", &refs, Some("partition_date"), sql)
                .expect_err("forward-reading self-edge must not yield a clamp");
        assert_eq!(clamp_err, verdict_reason);
    }

    #[test]
    fn same_partition_self_read_is_refused_fail_closed() {
        let refs = vec!["marts.running_balance".to_string()];
        let sql = "SELECT bal.balance AS balance \
                   FROM smelt.marts.running_balance bal \
                   JOIN smelt.marts.running_balance cur \
                     ON cur.partition_date = bal.partition_date";
        let verdict =
            window_independence("marts.running_balance", &refs, Some("partition_date"), sql);
        match verdict {
            WindowIndependence::Refused { reason } => {
                assert!(
                    reason.contains("marts.running_balance"),
                    "reason must name the self-edge: {reason}"
                );
                assert!(
                    reason.contains("current partition") || reason.contains("circular"),
                    "reason must describe the same-partition (non-convergent) shape: {reason}"
                );
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn self_edge_clamp_refuses_a_same_partition_self_read_with_the_same_reason() {
        let refs = vec!["marts.running_balance".to_string()];
        let sql = "SELECT bal.balance AS balance \
                   FROM smelt.marts.running_balance bal \
                   JOIN smelt.marts.running_balance cur \
                     ON cur.partition_date = bal.partition_date";
        let verdict =
            window_independence("marts.running_balance", &refs, Some("partition_date"), sql);
        let WindowIndependence::Refused {
            reason: verdict_reason,
        } = verdict
        else {
            panic!("expected Refused");
        };
        let clamp_err =
            self_edge_clamp("marts.running_balance", &refs, Some("partition_date"), sql)
                .expect_err("same-partition self-read must not yield a clamp");
        assert_eq!(clamp_err, verdict_reason);
    }

    #[test]
    fn sub_day_backward_self_edge_stays_ordered() {
        let refs = vec![
            "marts.running_balance".to_string(),
            "silver.transactions".to_string(),
        ];
        let sql = "SELECT bal.balance + t.amount AS balance \
                   FROM smelt.marts.running_balance bal \
                   JOIN smelt.silver.transactions t ON bal.acct_id = t.acct_id \
                   WHERE bal.partition_date >= t.partition_date - INTERVAL '1 hour' \
                     AND bal.partition_date < t.partition_date";
        assert_eq!(
            window_independence("marts.running_balance", &refs, Some("partition_date"), sql),
            WindowIndependence::Ordered
        );
        assert_eq!(
            self_edge_clamp("marts.running_balance", &refs, Some("partition_date"), sql),
            Ok(1)
        );
    }

    #[test]
    fn self_edge_clamp_none_for_no_self_edge() {
        let refs = vec!["silver.transactions".to_string()];
        let sql = "SELECT * FROM smelt.silver.transactions";
        let err = self_edge_clamp("marts.running_balance", &refs, Some("partition_date"), sql)
            .expect_err("no self-edge must not yield a clamp");
        assert!(err.contains("no self-edge"));
    }
}
