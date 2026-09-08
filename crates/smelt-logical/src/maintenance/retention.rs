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
