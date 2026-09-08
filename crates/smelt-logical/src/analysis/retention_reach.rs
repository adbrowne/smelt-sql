//! Reach versus retained history (`docs/specs/model_properties.md` §"Reach
//! versus retained history").
//!
//! This module holds **no text scan** of its own: its only input is the
//! unified bound/reach derivation's own output (`derive_model_bounds`'s
//! `BoundResult`, the composition walk's product) plus the source's declared
//! `retention:` bound. The comparison is a pure fold over that verdict — it
//! inherits the walk's series composition (stacked reaches add) and its
//! fail-closed behaviour (`Unbounded`/`NotDerivable` never pass) for free,
//! rather than re-deriving either.
//!
//! This is a **proof only**. It names no diagnostic and takes no plan-time
//! action — the refusal or degradation that consumes it belongs to a later
//! layer (`sources.md`'s `SourceRetentionExceeded`, the degradation
//! contract).

use std::collections::HashMap;

use crate::analysis::source_bounds::{derive_model_bounds, BoundContext, BoundResult, Seconds};

impl BoundContext {
    /// Register `source`'s declared `retention:` bound, converting the
    /// declaration's [`smelt_core::config::DataLatency`] to [`Seconds`] here
    /// — the one place a `retention:` declaration is converted for the
    /// retention-reach comparison, so it and the comparison can never drift
    /// in units.
    pub fn with_source_retention(
        mut self,
        source: &str,
        retention: &smelt_core::config::DataLatency,
    ) -> Self {
        self.add_source_retention(source, retention);
        self
    }

    /// See [`Self::with_source_retention`].
    pub fn add_source_retention(
        &mut self,
        source: &str,
        retention: &smelt_core::config::DataLatency,
    ) {
        self.retentions
            .insert(source.to_string(), Seconds(retention.seconds));
    }
}

/// Why a source's reach could not be proven to fit inside its retained
/// bound. Distinguishes an unbounded/whole-history reach from one the
/// unified derivation simply could not classify (`ROWS` frames, symbolic
/// `INTERVAL` literals) — both fail closed to [`RetentionVerdict::UnprovableWithin`],
/// but for different reasons a caller may want to report differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnprovableReason {
    /// The unified derivation returned `BoundResult::Unbounded` — the source
    /// requires reading unbounded history (e.g. a cumulative aggregation).
    UnboundedReach,
    /// The unified derivation returned `BoundResult::NotDerivable` — the
    /// analyzer could not derive a bound from the SQL patterns present.
    ReachNotDerivable,
}

/// The retention-admissibility verdict for one source (`docs/specs/model_properties.md`
/// §"Reach versus retained history").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetentionVerdict {
    /// The source declares no `retention:` — trusted replayable
    /// (`sources.md` §Semantics 5). No comparison to make.
    NoDeclaredBound,
    /// The model's required reach (`before` + `window_age`) fits inside the
    /// source's declared retained bound.
    Within {
        required_lookback: Seconds,
        retained: Seconds,
    },
    /// The model's required reach exceeds the source's declared retained
    /// bound.
    Exceeds {
        required_lookback: Seconds,
        retained: Seconds,
    },
    /// The unified derivation could not bound the source's reach at all —
    /// fails closed rather than optimistically passing. Absence of a proof
    /// is a rejection, never a `Within`.
    UnprovableWithin {
        retained: Seconds,
        reason: UnprovableReason,
    },
}

/// Derive the retention-admissibility verdict for every source `ctx`
/// declares a retention bound for, folding `derive_model_bounds(sql, ctx)`'s
/// own output against `ctx.retentions` and `window_age` — the run window's
/// own age, added to the walk's derived backward reach so the verdict is a
/// **rolling** re-evaluation against the bound in effect at plan time on
/// every run (`sources.md` §Semantics 5), rather than one fixed at the point
/// the model was authored.
///
/// A source declared in `ctx.retentions` but absent from `derive_model_bounds`'s
/// output (the model never actually reads it) is absent from the returned
/// map too — never a phantom `NoDeclaredBound`.
pub fn derive_retention_verdicts(
    sql: &str,
    ctx: &BoundContext,
    window_age: Seconds,
) -> HashMap<String, RetentionVerdict> {
    let bounds = derive_model_bounds(sql, ctx);
    let mut verdicts = HashMap::new();
    for (source, retained) in &ctx.retentions {
        let Some(bound) = bounds.get(source) else {
            continue;
        };
        let verdict = match bound {
            BoundResult::Bounded { before, .. } => {
                let required_lookback = Seconds(before.0.saturating_add(window_age.0));
                if required_lookback <= *retained {
                    RetentionVerdict::Within {
                        required_lookback,
                        retained: *retained,
                    }
                } else {
                    RetentionVerdict::Exceeds {
                        required_lookback,
                        retained: *retained,
                    }
                }
            }
            BoundResult::Unbounded => RetentionVerdict::UnprovableWithin {
                retained: *retained,
                reason: UnprovableReason::UnboundedReach,
            },
            BoundResult::NotDerivable => RetentionVerdict::UnprovableWithin {
                retained: *retained,
                reason: UnprovableReason::ReachNotDerivable,
            },
        };
        verdicts.insert(source.clone(), verdict);
    }
    verdicts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `add_source_retention` converts the declaration's
    /// [`smelt_core::config::DataLatency`] to [`Seconds`] once, at
    /// population time — the comparison and the declaration can never drift
    /// in units.
    #[test]
    fn retention_interval_converts_to_seconds_for_the_comparison() {
        let mut ctx = BoundContext::new();
        ctx.add_source_retention(
            "silver.events",
            &smelt_core::config::DataLatency::parse("45 days").unwrap(),
        );
        assert_eq!(
            ctx.retentions.get("silver.events"),
            Some(&Seconds::days(45))
        );
    }

    #[test]
    fn no_declared_retention_is_no_declared_bound() {
        // ctx.retentions is empty for this source, so no verdict is produced
        // at all — the "NoDeclaredBound" case is expressed by the caller
        // treating an absent map entry as no declared bound, not by this
        // function returning a variant for it. This test pins that absence.
        let sql = "SELECT event_date, COUNT(*) AS cnt FROM smelt.silver.events \
                   WHERE event_date >= CURRENT_DATE - INTERVAL '7 days' \
                   GROUP BY event_date";
        let ctx = BoundContext::new().with_source("silver.events", "event_date");
        let verdicts = derive_retention_verdicts(sql, &ctx, Seconds::ZERO);
        assert!(
            !verdicts.contains_key("silver.events"),
            "no declared retention must produce no verdict entry: {verdicts:?}"
        );
    }

    #[test]
    fn bounded_reach_inside_the_retained_bound_is_within() {
        let sql = "SELECT event_date, COUNT(*) AS cnt FROM smelt.silver.events \
                   WHERE event_date >= CURRENT_DATE - INTERVAL '7 days' \
                   GROUP BY event_date";
        let mut ctx = BoundContext::new().with_source("silver.events", "event_date");
        ctx.add_source_retention(
            "silver.events",
            &smelt_core::config::DataLatency::parse("45 days").unwrap(),
        );
        let verdicts = derive_retention_verdicts(sql, &ctx, Seconds::ZERO);
        assert_eq!(
            verdicts.get("silver.events"),
            Some(&RetentionVerdict::Within {
                required_lookback: Seconds::days(7),
                retained: Seconds::days(45),
            })
        );
    }

    #[test]
    fn bounded_reach_past_the_retained_bound_exceeds() {
        let sql = "SELECT event_date, COUNT(*) AS cnt FROM smelt.silver.events \
                   WHERE event_date >= CURRENT_DATE - INTERVAL '90 days' \
                   GROUP BY event_date";
        let mut ctx = BoundContext::new().with_source("silver.events", "event_date");
        ctx.add_source_retention(
            "silver.events",
            &smelt_core::config::DataLatency::parse("45 days").unwrap(),
        );
        let verdicts = derive_retention_verdicts(sql, &ctx, Seconds::ZERO);
        assert_eq!(
            verdicts.get("silver.events"),
            Some(&RetentionVerdict::Exceeds {
                required_lookback: Seconds::days(90),
                retained: Seconds::days(45),
            })
        );
    }

    #[test]
    fn window_age_pushes_an_otherwise_within_reach_past_the_bound() {
        let sql = "SELECT event_date, COUNT(*) AS cnt FROM smelt.silver.events \
                   WHERE event_date >= CURRENT_DATE - INTERVAL '7 days' \
                   GROUP BY event_date";
        let mut ctx = BoundContext::new().with_source("silver.events", "event_date");
        ctx.add_source_retention(
            "silver.events",
            &smelt_core::config::DataLatency::parse("45 days").unwrap(),
        );
        let verdicts = derive_retention_verdicts(sql, &ctx, Seconds::days(60));
        assert_eq!(
            verdicts.get("silver.events"),
            Some(&RetentionVerdict::Exceeds {
                required_lookback: Seconds::days(67),
                retained: Seconds::days(45),
            })
        );
    }

    #[test]
    fn unbounded_reach_against_a_finite_bound_is_unprovable() {
        let sql = "SELECT event_date, SUM(amount) OVER (PARTITION BY event_date \
                   ORDER BY event_date RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running \
                   FROM smelt.silver.events";
        let mut ctx = BoundContext::new().with_source("silver.events", "event_date");
        ctx.add_source_retention(
            "silver.events",
            &smelt_core::config::DataLatency::parse("45 days").unwrap(),
        );
        let verdicts = derive_retention_verdicts(sql, &ctx, Seconds::ZERO);
        assert_eq!(
            verdicts.get("silver.events"),
            Some(&RetentionVerdict::UnprovableWithin {
                retained: Seconds::days(45),
                reason: UnprovableReason::UnboundedReach,
            })
        );
    }

    #[test]
    fn not_derivable_reach_fails_closed_to_unprovable() {
        let sql = "SELECT id, ts, LAG(x) OVER (PARTITION BY id ORDER BY ts) AS prev_x \
                   FROM smelt.silver.events";
        let mut ctx = BoundContext::new().with_source("silver.events", "event_date");
        ctx.add_source_retention(
            "silver.events",
            &smelt_core::config::DataLatency::parse("45 days").unwrap(),
        );
        let verdicts = derive_retention_verdicts(sql, &ctx, Seconds::ZERO);
        assert_eq!(
            verdicts.get("silver.events"),
            Some(&RetentionVerdict::UnprovableWithin {
                retained: Seconds::days(45),
                reason: UnprovableReason::ReachNotDerivable,
            })
        );
    }

    #[test]
    fn series_composition_through_a_cte_is_visible_to_the_verdict() {
        // Two stacked 4-day RANGE frames across a CTE boundary must ADD to
        // an 8-day reach (the walk's series composition) — a whole-text
        // max-merge would under-derive this to 4 days and wrongly admit it
        // under the 5-day bound.
        let sql = "WITH stage1 AS ( \
                       SELECT event_date, \
                              SUM(amount) OVER (PARTITION BY acct_id ORDER BY event_date \
                                  RANGE BETWEEN INTERVAL '4 days' PRECEDING AND CURRENT ROW) AS running1 \
                       FROM smelt.silver.events \
                   ) \
                   SELECT event_date, \
                          SUM(running1) OVER (PARTITION BY event_date ORDER BY event_date \
                              RANGE BETWEEN INTERVAL '4 days' PRECEDING AND CURRENT ROW) AS running2 \
                   FROM stage1";
        let mut ctx = BoundContext::new().with_source("silver.events", "event_date");
        ctx.add_source_retention(
            "silver.events",
            &smelt_core::config::DataLatency::parse("5 days").unwrap(),
        );
        let verdicts = derive_retention_verdicts(sql, &ctx, Seconds::ZERO);
        assert_eq!(
            verdicts.get("silver.events"),
            Some(&RetentionVerdict::Exceeds {
                required_lookback: Seconds::days(8),
                retained: Seconds::days(5),
            }),
            "stacked frames must compose in series (add), not max-merge: {verdicts:?}"
        );
    }

    #[test]
    fn a_source_absent_from_the_model_gets_no_verdict() {
        // "silver.orphan" has a declared retention bound but is not
        // registered in `ctx.source_partition_cols` — this model does not
        // read it as a timeseries source at all — so `derive_model_bounds`
        // produces no entry for it (the whole-text top-up only backfills
        // sources actually present in `ctx.source_partition_cols`), and the
        // retention verdict map must not invent one either.
        let sql = "SELECT event_date, COUNT(*) AS cnt FROM smelt.silver.events \
                   GROUP BY event_date";
        let mut ctx = BoundContext::new().with_source("silver.events", "event_date");
        ctx.add_source_retention(
            "silver.orphan",
            &smelt_core::config::DataLatency::parse("45 days").unwrap(),
        );
        let verdicts = derive_retention_verdicts(sql, &ctx, Seconds::ZERO);
        assert!(
            !verdicts.contains_key("silver.orphan"),
            "a declared retention with no corresponding source in ctx must get no verdict: {verdicts:?}"
        );
    }
}
