//! Window-independence / ordered-execution — self-edge detection over the
//! model DAG.
//!
//! See `docs/specs/model_properties.md` §"Derived proofs" →
//! "Window-independence / ordered-execution" and `docs/specs/batched_models.md`
//! §"Window independence and self-referential models". A model that reads
//! only external sources is parallelisable across its windows/partitions
//! (`WindowIndependent`). A model with a self-edge — it reads its own prior
//! output via `smelt.<self>` — must build its windows strictly in temporal
//! order (`Ordered`), but only when the self-reference provably converges
//! partition-by-partition (a backward-bounded read of prior partitions,
//! never a forward read or an unbounded/whole-history scan). This is a
//! signal only: the ordered-backfill chunker that consumes it is
//! batched-local (L4).

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
    /// partition-by-partition (a forward read, an unbounded/whole-history
    /// scan, or an underivable bound). Fail-closed: never silently treated
    /// as `Ordered`.
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
    if !refs.iter().any(|r| r == model_name) {
        return WindowIndependence::WindowIndependent;
    }

    let Some(partition_col) = self_partition_col else {
        return WindowIndependence::Refused {
            reason: format!(
                "model '{model_name}' has a self-edge but declares no timeseries \
                 partition column to prove convergence"
            ),
        };
    };

    let ctx = BoundContext::new().with_source(model_name, partition_col);
    let bound = derive_model_bounds(sql, &ctx)
        .remove(model_name)
        .unwrap_or(BoundResult::NotDerivable);

    match bound {
        BoundResult::Bounded { after, .. } if after == Seconds::ZERO => {
            WindowIndependence::Ordered
        }
        BoundResult::Bounded { after, .. } => WindowIndependence::Refused {
            reason: format!(
                "model '{model_name}' self-edge reads {} forward of the current partition \
                 — not provably convergent",
                after.to_iso8601()
            ),
        },
        BoundResult::Unbounded => WindowIndependence::Refused {
            reason: format!(
                "model '{model_name}' self-edge reads unbounded/whole history \
                 — not provably convergent partition-by-partition"
            ),
        },
        BoundResult::NotDerivable => WindowIndependence::Refused {
            reason: format!(
                "model '{model_name}' self-edge's bound could not be derived from its SQL"
            ),
        },
    }
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
            window_independence(
                "marts.running_balance",
                &refs,
                Some("partition_date"),
                sql
            ),
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
}
