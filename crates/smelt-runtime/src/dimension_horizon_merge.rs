//! Dimension-driven horizon-bounded MERGE — the mechanism behind the
//! "Dimension-driven horizon-bounded MERGE" transform.
//!
//! See `docs/specs/model_transforms.md` §Surface "Dimension-driven
//! horizon-bounded MERGE" and §Semantics "Targeted column backfill and
//! dimension-driven horizon MERGE". This is licensed by target-as-replica
//! (`merge_into`, already built) **plus** join-contribution monotonicity
//! (`smelt_logical::analysis::join_shape::join_contribution_monotone`)
//! **plus** a bounded horizon `H` — the forward `after` reach from
//! `smelt_logical::analysis::source_bounds::derive_model_bounds`. When all
//! three hold, a dimension batch merges straight into the target replica
//! slice `[conv_ts − H, conv_ts]` without re-reading the fact table at all.
//!
//! Fail-closed (`model_transforms.md` §Constraints "Equivalence or
//! refusal"): a non-monotone join contribution, or a source reach that is
//! not `Bounded` (no `H` to clamp to), refuses before emitting any SQL — the
//! caller falls back to a full rebuild.

use smelt_logical::analysis::join_shape::ContributionVerdict;
use smelt_logical::analysis::source_bounds::BoundResult;

/// Build the horizon-clamped `SELECT` that becomes `merge_into`'s
/// `source_sql` argument, or refuse with a reason when the horizon MERGE is
/// not licensed.
///
/// `dimension_batch_sql` is the dimension batch's recompute `SELECT`
/// — never a re-read of the fact table. `conv_ts_column` is the column on
/// that batch tracking when each row last changed; `conv_ts` is the run's
/// current point-in-time (a literal timestamp string). The returned SQL
/// clamps the batch to `[conv_ts − H, conv_ts]`, where `H` is `bound`'s
/// forward `after` reach.
///
/// Refuses (returns `Err`) when:
/// - `contribution` is [`ContributionVerdict::Refused`] — its reason is
///   propagated.
/// - `bound` is not [`BoundResult::Bounded`] — there is no horizon `H` to
///   clamp the merge to.
pub fn dimension_horizon_merge(
    contribution: &ContributionVerdict,
    bound: &BoundResult,
    table: &str,
    conv_ts_column: &str,
    conv_ts: &str,
    dimension_batch_sql: &str,
) -> Result<String, String> {
    match contribution {
        ContributionVerdict::Refused(reason) => {
            return Err(format!(
                "dimension-driven horizon MERGE refused for table '{table}': {reason}"
            ));
        }
        ContributionVerdict::Monotone => {}
    }

    let after = match bound {
        BoundResult::Bounded { after, .. } => *after,
        BoundResult::Unbounded => {
            return Err(format!(
                "dimension-driven horizon MERGE refused for table '{table}': source reach is \
                 unbounded — no horizon H to clamp the merge to, would have to re-read the full \
                 fact history"
            ));
        }
        BoundResult::NotDerivable => {
            return Err(format!(
                "dimension-driven horizon MERGE refused for table '{table}': source reach could \
                 not be derived — no horizon H to clamp the merge to"
            ));
        }
    };

    let safe_col = conv_ts_column.replace('\'', "''");
    let safe_ts = conv_ts.replace('\'', "''");

    Ok(format!(
        "SELECT * FROM ({dimension_batch_sql}) AS _horizon_merge_source WHERE {safe_col} >= \
         (TIMESTAMP '{safe_ts}' - INTERVAL '{} seconds') AND {safe_col} <= TIMESTAMP '{safe_ts}'",
        after.0
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_logical::analysis::source_bounds::Seconds;

    #[test]
    fn monotone_contribution_with_bounded_horizon_emits_clamped_select() {
        let contribution = ContributionVerdict::Monotone;
        let bound = BoundResult::Bounded {
            source_partition_col: "conv_ts".to_string(),
            before: Seconds::ZERO,
            after: Seconds::hours(6),
        };

        let sql = dimension_horizon_merge(
            &contribution,
            &bound,
            "main.dim_users",
            "conv_ts",
            "2026-07-05 00:00:00",
            "SELECT user_id, conv_ts, plan_tier FROM main.dim_users_batch",
        )
        .expect("licensed horizon MERGE should succeed");

        assert!(sql.contains("FROM (SELECT user_id, conv_ts, plan_tier FROM main.dim_users_batch)"));
        assert!(
            sql.contains("conv_ts >= (TIMESTAMP '2026-07-05 00:00:00' - INTERVAL '21600 seconds')")
        );
        assert!(sql.contains("conv_ts <= TIMESTAMP '2026-07-05 00:00:00'"));
        // Never a full rebuild, never a bare unclamped read of the batch.
        assert!(!sql.to_uppercase().contains("CREATE"));
    }

    #[test]
    fn non_monotone_contribution_is_refused() {
        let contribution = ContributionVerdict::Refused(
            "join fans out (row-multiplying) into a decrementing aggregate".to_string(),
        );
        let bound = BoundResult::Bounded {
            source_partition_col: "conv_ts".to_string(),
            before: Seconds::ZERO,
            after: Seconds::hours(6),
        };

        let err = dimension_horizon_merge(
            &contribution,
            &bound,
            "main.dim_users",
            "conv_ts",
            "2026-07-05 00:00:00",
            "SELECT user_id, conv_ts, plan_tier FROM main.dim_users_batch",
        )
        .expect_err("non-monotone contribution must be refused");

        assert!(err.contains("row-multiplying"));
        assert!(!err.to_uppercase().contains("SELECT * FROM"));
    }

    #[test]
    fn unbounded_horizon_is_refused() {
        let contribution = ContributionVerdict::Monotone;

        let err = dimension_horizon_merge(
            &contribution,
            &BoundResult::Unbounded,
            "main.dim_users",
            "conv_ts",
            "2026-07-05 00:00:00",
            "SELECT user_id, conv_ts, plan_tier FROM main.dim_users_batch",
        )
        .expect_err("unbounded horizon must be refused, never merge approximately");

        assert!(err.contains("unbounded"));
        assert!(!err.to_uppercase().contains("SELECT * FROM"));
    }

    #[test]
    fn not_derivable_horizon_is_refused() {
        let contribution = ContributionVerdict::Monotone;

        let err = dimension_horizon_merge(
            &contribution,
            &BoundResult::NotDerivable,
            "main.dim_users",
            "conv_ts",
            "2026-07-05 00:00:00",
            "SELECT user_id, conv_ts, plan_tier FROM main.dim_users_batch",
        )
        .expect_err("not-derivable horizon must be refused, never merge approximately");

        assert!(err.contains("could not be derived"));
        assert!(!err.to_uppercase().contains("SELECT * FROM"));
    }
}
