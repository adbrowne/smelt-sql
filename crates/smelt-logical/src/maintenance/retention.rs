//! The verdict → outcome mapping for a source's declared `retention:` bound
//! (`docs/specs/model_properties.md` §"Reach versus retained history").
//!
//! [`crate::analysis::retention_reach::derive_retention_verdicts`] is a proof
//! only — it names no diagnostic and takes no plan-time action. This module
//! is the single, total consumer of that proof: [`retention_outcomes`] maps
//! every [`crate::analysis::retention_reach::RetentionVerdict`] onto exactly
//! one of a refusal, a recorded downgrade, or silence, with no other outcome
//! admissible (the no-silent-under-read property).

use std::collections::HashMap;

use chrono::NaiveDate;

use crate::analysis::retention_reach::{RetentionVerdict, UnprovableReason};
use crate::analysis::source_bounds::Seconds;
use crate::maintenance::Refusal;

/// Bare source name → declared `retention:` world-fact, threaded into
/// [`super::derive::ModelInputs`]'s `BoundContext` the same way
/// [`super::derive::SourceReferentialIntegrity`] threads referential
/// integrity — a side channel rather than a `SourceFacts` field, so the many
/// existing `SourceFacts`/`ModelInputs` literal-construction call sites
/// across the workspace stay unaffected by a channel this phase alone
/// introduces.
pub type SourceRetentions = std::collections::BTreeMap<String, smelt_core::config::DataLatency>;

/// A source's reach could not be *proven* to fit inside its declared
/// retention bound — recorded rather than silently admitted
/// (`docs/specs/sources.md` §Semantics 5 "Retention refusal"). Unlike
/// [`Refusal::SourceRetentionExceeded`], this does not block the plan: the
/// model still derives cells, but its pre-bound region stops being claimed
/// replayable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionDowngrade {
    pub source: String,
    pub retained: Seconds,
    pub reason: UnprovableReason,
}

/// Fold every [`RetentionVerdict`] `derive_retention_verdicts` produced onto
/// its outcome: `Exceeds` refuses, `UnprovableWithin` records a downgrade,
/// `Within`/`NoDeclaredBound` record nothing (`model_properties.md`
/// §"Reach versus retained history"). Total over every verdict shape — this
/// totality is the no-silent-under-read property: no verdict can land
/// anywhere but exactly one of the two returned lists, or silence.
///
/// Iterates sources in sorted order so the returned lists are deterministic
/// regardless of the input `HashMap`'s own iteration order.
pub fn retention_outcomes(
    verdicts: &HashMap<String, RetentionVerdict>,
) -> (Vec<Refusal>, Vec<RetentionDowngrade>) {
    let mut sources: Vec<&String> = verdicts.keys().collect();
    sources.sort();
    let mut refusals = Vec::new();
    let mut downgrades = Vec::new();
    for source in sources {
        match &verdicts[source] {
            RetentionVerdict::NoDeclaredBound | RetentionVerdict::Within { .. } => {}
            RetentionVerdict::Exceeds {
                required_lookback,
                retained,
            } => {
                refusals.push(Refusal::SourceRetentionExceeded {
                    source: source.clone(),
                    required_lookback_secs: required_lookback.0,
                    retained_secs: retained.0,
                });
            }
            RetentionVerdict::UnprovableWithin { retained, reason } => {
                downgrades.push(RetentionDowngrade {
                    source: source.clone(),
                    retained: *retained,
                    reason: *reason,
                });
            }
        }
    }
    (refusals, downgrades)
}

/// One source's **bounded** reach-versus-retention proof, carried on
/// [`super::MaintenancePlan::retention_reaches`] so a run can re-evaluate it
/// against its own window age without re-walking the model's SQL
/// (`docs/outcomes/20260906-trimmed-history-sources/outcome.md` criterion 5,
/// maintenance-plan purity). `required_lookback` here is the model's own
/// derived reach at plan-derivation time (`window_age: Seconds::ZERO`) —
/// never the run-window-aged quantity, which [`retention_refusals_at_age`]
/// folds in later, once, at the run's own clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionReach {
    pub source: String,
    pub required_lookback: Seconds,
    pub retained: Seconds,
}

/// Collect the **bounded** verdicts (`Within` and `Exceeds` alike) as
/// [`RetentionReach`] proofs — the totality of `derive_retention_verdicts`'s
/// output this phase's rolling re-evaluation needs. `NoDeclaredBound` (no
/// comparison exists) and `UnprovableWithin` (no bound to age at all — the
/// reach itself was never proven finite) carry no reach and are omitted.
/// Iterates sources in sorted order, matching [`retention_outcomes`].
pub fn retention_reaches(verdicts: &HashMap<String, RetentionVerdict>) -> Vec<RetentionReach> {
    let mut sources: Vec<&String> = verdicts.keys().collect();
    sources.sort();
    sources
        .into_iter()
        .filter_map(|source| match &verdicts[source] {
            RetentionVerdict::Within {
                required_lookback,
                retained,
            }
            | RetentionVerdict::Exceeds {
                required_lookback,
                retained,
            } => Some(RetentionReach {
                source: source.clone(),
                required_lookback: *required_lookback,
                retained: *retained,
            }),
            RetentionVerdict::NoDeclaredBound | RetentionVerdict::UnprovableWithin { .. } => None,
        })
        .collect()
}

/// Age every [`RetentionReach`] by `window_age` (the run's own backfill age,
/// [`run_window_age`]) and refuse the ones that now exceed their retained
/// bound — the rolling re-evaluation
/// (`docs/outcomes/20260906-trimmed-history-sources/outcome.md` criterion 5):
/// a reach that fit last month can stop fitting today with no change to the
/// model's SQL. Pure fold over the plan's own already-derived proof; never
/// re-walks anything. Monotone in `window_age` — a reach already exceeding
/// at age zero stays refused at every larger age (adding a non-negative
/// quantity to an already-too-large `required_lookback` cannot bring it back
/// under `retained`).
pub fn retention_refusals_at_age(reaches: &[RetentionReach], window_age: Seconds) -> Vec<Refusal> {
    reaches
        .iter()
        .filter_map(|reach| {
            let aged_lookback = Seconds(reach.required_lookback.0.saturating_add(window_age.0));
            if aged_lookback > reach.retained {
                Some(Refusal::SourceRetentionExceeded {
                    source: reach.source.clone(),
                    required_lookback_secs: aged_lookback.0,
                    retained_secs: reach.retained.0,
                })
            } else {
                None
            }
        })
        .collect()
}

/// The age of a run's window, measured from `window_start` to the run's own
/// clock (`now`) — the quantity [`retention_refusals_at_age`] folds onto
/// each reach's plan-time `required_lookback`
/// (`docs/specs/sources.md` §Semantics 5 "Retention refusal"). Saturates at
/// zero for a forward-dated or same-day window rather than going negative —
/// a forward-only run (no explicit `--start`) has age zero by construction,
/// which is what keeps steady-state maintenance unaffected by this check.
pub fn run_window_age(window_start: NaiveDate, now: NaiveDate) -> Seconds {
    let days = now.signed_duration_since(window_start).num_days().max(0);
    Seconds(days as u64 * 86400)
}

#[cfg(test)]
mod rolling_tests {
    use super::*;

    fn verdicts_with(source: &str, verdict: RetentionVerdict) -> HashMap<String, RetentionVerdict> {
        let mut m = HashMap::new();
        m.insert(source.to_string(), verdict);
        m
    }

    #[test]
    fn retention_reaches_carry_the_bounded_proof() {
        let mut verdicts = HashMap::new();
        verdicts.insert(
            "silver.within".to_string(),
            RetentionVerdict::Within {
                required_lookback: Seconds::days(7),
                retained: Seconds::days(45),
            },
        );
        verdicts.insert(
            "silver.exceeds".to_string(),
            RetentionVerdict::Exceeds {
                required_lookback: Seconds::days(90),
                retained: Seconds::days(45),
            },
        );
        verdicts.insert(
            "silver.unprovable".to_string(),
            RetentionVerdict::UnprovableWithin {
                retained: Seconds::days(45),
                reason: UnprovableReason::UnboundedReach,
            },
        );
        verdicts.insert(
            "silver.undeclared".to_string(),
            RetentionVerdict::NoDeclaredBound,
        );

        let reaches = retention_reaches(&verdicts);
        assert_eq!(
            reaches,
            vec![
                RetentionReach {
                    source: "silver.exceeds".to_string(),
                    required_lookback: Seconds::days(90),
                    retained: Seconds::days(45),
                },
                RetentionReach {
                    source: "silver.within".to_string(),
                    required_lookback: Seconds::days(7),
                    retained: Seconds::days(45),
                },
            ],
            "exactly one entry per bounded verdict (Within and Exceeds), sorted by source, \
             none for UnprovableWithin or NoDeclaredBound: {reaches:?}"
        );
    }

    #[test]
    fn an_admissible_reach_refuses_once_the_window_ages_past_the_bound() {
        let reaches = vec![RetentionReach {
            source: "silver.events".to_string(),
            required_lookback: Seconds::days(7),
            retained: Seconds::days(45),
        }];

        assert!(
            retention_refusals_at_age(&reaches, Seconds::ZERO).is_empty(),
            "age zero must not refuse a reach that fits at authoring time"
        );

        let refusals = retention_refusals_at_age(&reaches, Seconds::days(60));
        assert_eq!(refusals.len(), 1);
        match &refusals[0] {
            Refusal::SourceRetentionExceeded {
                source,
                required_lookback_secs,
                retained_secs,
            } => {
                assert_eq!(source, "silver.events");
                assert_eq!(*required_lookback_secs, Seconds::days(67).0);
                assert_eq!(*retained_secs, Seconds::days(45).0);
            }
            other => panic!("expected SourceRetentionExceeded, got {other:?}"),
        }
    }

    #[test]
    fn age_never_rescues_an_already_exceeding_reach() {
        let reaches = vec![RetentionReach {
            source: "silver.events".to_string(),
            required_lookback: Seconds::days(90),
            retained: Seconds::days(45),
        }];
        for age in [Seconds::ZERO, Seconds::days(1), Seconds::days(365)] {
            let refusals = retention_refusals_at_age(&reaches, age);
            assert_eq!(
                refusals.len(),
                1,
                "a reach already exceeding at age zero must stay refused at age {age:?}: {refusals:?}"
            );
        }
    }

    #[test]
    fn run_window_age_is_zero_for_a_window_at_or_after_now() {
        let now = NaiveDate::from_ymd_opt(2026, 9, 9).unwrap();
        assert_eq!(run_window_age(now, now), Seconds::ZERO);
        let forward_dated = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        assert_eq!(
            run_window_age(forward_dated, now),
            Seconds::ZERO,
            "a window starting after the run clock must saturate at zero age, not go negative"
        );
    }

    #[test]
    fn retention_reaches_survive_a_plan_round_trip_unchanged() {
        let verdicts = verdicts_with(
            "silver.events",
            RetentionVerdict::Within {
                required_lookback: Seconds::days(7),
                retained: Seconds::days(45),
            },
        );
        let reaches = retention_reaches(&verdicts);
        // The rolling fold reads only the already-derived `RetentionReach`
        // list — no SQL, no `BoundContext`, nothing re-walked — so calling
        // it twice on the exact same slice is byte-identical, which is the
        // only property maintenance-plan purity can be checked for at this
        // layer (the walk itself is `walk_coverage`'s job).
        let first = retention_refusals_at_age(&reaches, Seconds::days(60));
        let second = retention_refusals_at_age(&reaches, Seconds::days(60));
        assert_eq!(first, second);
    }
}
