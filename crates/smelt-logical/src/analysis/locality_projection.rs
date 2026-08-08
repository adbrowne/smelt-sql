//! Partition-locality projection (`docs/specs/model_properties.md`
//! §"Partition-locality projection") — the pure proof consumed by
//! `incremental_models.md` §"Partition-local maintenance" (the K8 guardrail).
//!
//! `locality_verdict` composes the read-side bound
//! ([`super::source_bounds::derive_model_bounds`]) and the write-side
//! reflected footprint ([`super::footprint::reflect_footprint`]): a cell is
//! local in a source when **both** project onto a bounded interval of,
//! respectively, the source's and the output's partition column. A
//! cross-axis source (partition column not the output's own axis) is local
//! only through an explicit, derivable predicate connecting the two columns
//! ([`CrossAxisLink::ExplicitPredicate`], derived by
//! [`super::source_bounds::derive_cross_axis_links`] from the same Form B
//! matchers the bound derivation runs) — smelt does not infer a relationship
//! between two differently-named partition columns, so the absence of such a
//! predicate is the absence of a link, never a zero-cost default and never a
//! margin-inferred guess.
//!
//! This module is the *proof* only. The policy layer — defaults,
//! `allow_full_scan`, `max_lookback`, the `MaintenanceScanUnbounded`
//! diagnostic, creation-cell no-refuse, the keyed-grain vacuous-locality
//! sentinel — stays with the consumers (`crate::maintenance::derive`,
//! `smelt-db` diagnostics), never here.

use crate::analysis::footprint::FootprintResult;
use crate::analysis::source_bounds::{BoundResult, CrossAxisLink};

/// The two-armed per-`(cell × source)` verdict of the partition-locality
/// projection. Fail-closed: every underivable input is a `NotLocal` with the
/// failing side named — never a third "unknown" arm and never an optimistic
/// default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalityVerdict {
    /// The read bound and the reflected write footprint both project onto
    /// bounded intervals of their respective partition columns, and the
    /// source's axis is linked to the output's (identity or explicit
    /// predicate).
    Local,
    /// Not local; `reason` names which composition leg failed.
    NotLocal { reason: String },
}

/// Compose the partition-locality verdict for one `(cell × source)` pair.
///
/// - `read` — the source's derived scan bound (`derive_model_bounds`).
/// - `footprint` — the source's reflected write footprint on the output axis
///   (`reflect_footprint`).
/// - `source_axis` — the source's declared partition column (`None` for an
///   unclocked lookup source).
/// - `output_axis` — the output's own partition column (`None` for a
///   keyed-grain output, where the question cannot be posed).
/// - `cross_axis_link` — the derived predicate evidence relating the two
///   axes (`derive_cross_axis_links`).
///
/// Check order mirrors the composition: read bound first, then the axis
/// link, then the reflected footprint — the first failing leg names the
/// reason.
pub fn locality_verdict(
    read: &BoundResult,
    footprint: &FootprintResult,
    source_axis: Option<&str>,
    output_axis: Option<&str>,
    cross_axis_link: CrossAxisLink,
) -> LocalityVerdict {
    let Some(source_axis) = source_axis else {
        return LocalityVerdict::NotLocal {
            reason: "source declares no partition clock — a change's footprint projects onto \
                     no bounded partition interval of the output"
                .to_string(),
        };
    };
    let Some(output_axis) = output_axis else {
        return LocalityVerdict::NotLocal {
            reason: "output declares no partition axis — the locality question cannot be \
                     posed for a keyed output"
                .to_string(),
        };
    };

    // Leg 1: the read-side bound must project onto a bounded interval of the
    // source's partition column.
    match read {
        BoundResult::Unbounded => {
            return LocalityVerdict::NotLocal {
                reason: "derived scan is unbounded".to_string(),
            };
        }
        BoundResult::NotDerivable => {
            return LocalityVerdict::NotLocal {
                reason: "scan bound not derivable".to_string(),
            };
        }
        BoundResult::Bounded { .. } => {}
    }

    // Leg 2: the source's axis must be linked to the output's — by identity,
    // or by an explicit, derivable predicate. A nonzero margin is never
    // evidence ("never an inferred guess").
    let same_axis =
        source_axis.eq_ignore_ascii_case(output_axis) || cross_axis_link == CrossAxisLink::SameAxis;
    if !same_axis && cross_axis_link != CrossAxisLink::ExplicitPredicate {
        return LocalityVerdict::NotLocal {
            reason: format!(
                "no predicate links '{source_axis}' to the output partition axis \
                 '{output_axis}' — no explicit, derivable predicate relates the two \
                 columns; the scan cannot be partition-pruned"
            ),
        };
    }

    // Leg 3: the reflected write footprint must project onto a bounded
    // interval of the output's partition column.
    match footprint {
        FootprintResult::Unbounded => LocalityVerdict::NotLocal {
            reason: "derived write footprint on the output partition axis is unbounded — a \
                     delta can rewrite arbitrarily distant output partitions"
                .to_string(),
        },
        FootprintResult::NotDerivable => LocalityVerdict::NotLocal {
            reason: "write footprint on the output partition axis is not derivable".to_string(),
        },
        FootprintResult::Bounded { .. } => LocalityVerdict::Local,
    }
}
