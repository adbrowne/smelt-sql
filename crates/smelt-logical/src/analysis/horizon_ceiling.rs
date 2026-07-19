//! Horizon-ceiling declaration — the modeller's *warning ceiling* on the
//! maintained window.
//!
//! See `docs/specs/incremental_models.md` §"Windowed maintenance and the
//! horizon": the horizon is **derived** from the model's own reach (its
//! lookback, window frames, and join contribution — `model_properties.md`),
//! never trusted from a declaration, because a declared horizon smaller than
//! the true reach would make the clamp drop rows that should have been
//! rewritten. A modeller may declare a horizon *ceiling* — smelt warns when
//! the derived horizon would exceed it — but the clamp always uses the
//! derived value. This module only decides whether to warn; it never
//! mutates the bound.

use crate::analysis::source_bounds::BoundResult;

/// Warn when the derived reach for a source exceeds a declared horizon
/// ceiling.
///
/// Returns `None` when the derived reach is within the ceiling, or when the
/// source's reach could not be derived to a concrete bound (`Unbounded` /
/// `NotDerivable` are already forbidden or handled elsewhere — there is
/// nothing to compare here). Never mutates `bound` — the clamp always uses
/// the derived value; this only decides whether to emit a warning.
pub fn check_horizon_ceiling(bound: &BoundResult, ceiling_seconds: u64) -> Option<String> {
    let BoundResult::Bounded { before, after, .. } = bound else {
        return None;
    };

    let derived_seconds = before.0.max(after.0);
    if derived_seconds <= ceiling_seconds {
        return None;
    }

    Some(format!(
        "derived horizon of {derived_secs}s ({derived_days}d) exceeds the declared \
         horizon_ceiling of {ceiling_secs}s ({ceiling_days}d) — the clamp uses the derived \
         value; the ceiling is a warning only and does not narrow it",
        derived_secs = derived_seconds,
        derived_days = derived_seconds.div_ceil(86400),
        ceiling_secs = ceiling_seconds,
        ceiling_days = ceiling_seconds.div_ceil(86400),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::source_bounds::Seconds;

    fn bounded(before_days: u64, after_days: u64) -> BoundResult {
        BoundResult::Bounded {
            source_partition_col: "event_date".to_string(),
            before: Seconds::days(before_days),
            after: Seconds::days(after_days),
        }
    }

    #[test]
    fn within_ceiling_returns_none() {
        let bound = bounded(1, 0);
        let ceiling_seconds = 30 * 86400;
        assert_eq!(check_horizon_ceiling(&bound, ceiling_seconds), None);
    }

    #[test]
    fn exceeds_ceiling_names_both_values() {
        let bound = bounded(45, 0);
        let ceiling_seconds = 30 * 86400;
        let msg = check_horizon_ceiling(&bound, ceiling_seconds)
            .expect("derived reach of 45d exceeds a 30d ceiling — must warn");
        assert!(
            msg.contains("45d") || msg.contains(&(45u64 * 86400).to_string()),
            "warning must name the derived value: {msg}"
        );
        assert!(
            msg.contains("30d") || msg.contains(&(30u64 * 86400).to_string()),
            "warning must name the declared ceiling: {msg}"
        );
    }

    #[test]
    fn exact_equality_is_within_ceiling() {
        // The ceiling is a warning boundary, not a strict-less-than assertion
        // — a derived horizon exactly at the ceiling has not exceeded it.
        let bound = bounded(30, 0);
        let ceiling_seconds = 30 * 86400;
        assert_eq!(check_horizon_ceiling(&bound, ceiling_seconds), None);
    }

    #[test]
    fn after_margin_is_also_compared() {
        let bound = bounded(0, 45);
        let ceiling_seconds = 30 * 86400;
        assert!(check_horizon_ceiling(&bound, ceiling_seconds).is_some());
    }

    #[test]
    fn unbounded_returns_none() {
        assert_eq!(
            check_horizon_ceiling(&BoundResult::Unbounded, 30 * 86400),
            None
        );
    }

    #[test]
    fn not_derivable_returns_none() {
        assert_eq!(
            check_horizon_ceiling(&BoundResult::NotDerivable, 30 * 86400),
            None
        );
    }

    /// "Cannot narrow" is true by construction: `check_horizon_ceiling` takes
    /// `&BoundResult` (shared reference) and returns only a `String` message
    /// — there is no mutating side channel through which a smaller declared
    /// ceiling could narrow the bound. This test pins that the input is
    /// unchanged after the call, guarding against a future refactor that
    /// widens the signature to `&mut BoundResult` or similar.
    #[test]
    fn ceiling_smaller_than_derived_does_not_mutate_bound() {
        let bound = bounded(45, 0);
        let before_call = bound.clone();
        let _ = check_horizon_ceiling(&bound, 30 * 86400);
        assert_eq!(
            bound, before_call,
            "check_horizon_ceiling must never mutate the bound it inspects"
        );
    }
}
