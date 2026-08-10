//! `frozen_horizon` — the contract-lattice point declaring that partitions
//! older than `H` are never revisited by maintenance
//! (`docs/specs/incremental_models.md` §"Contract relaxations
//! (`contract:`)"). This module owns the two legs that land in phase 2: the
//! grain-admissibility validator and the pure write-range clamp transform.
//! The late-arrival probe emitter lands in phase 3.

use smelt_core::config::Grain;

/// Validates that `frozen_horizon:` is declared only on a partition-grain
/// model. Format validity (a parseable interval) is checked earlier, at
/// frontmatter-parse time, by `smelt_core::metadata`'s strict `contract:`
/// pre-validation (`MetadataError::ContractFrozenHorizonInvalid`) — this
/// function only makes the grain-admissibility check, which needs the
/// model's derived `grain`, unavailable to that pure-parse validator.
///
/// Returns `Err` naming the offending grain when `grain` is not
/// [`Grain::Partition`].
pub fn validate_frozen_horizon(grain: Grain) -> Result<(), String> {
    if grain != Grain::Partition {
        return Err(format!(
            "contract.frozen_horizon is admitted only on a partition-grain model; found grain {grain:?}"
        ));
    }
    Ok(())
}

/// Narrows the write-eligible start of a run range to the frozen-horizon
/// floor `end - h`, never widening: `start' = max(start, end - h)`
/// (`docs/outcomes/20260809-contract-lattice-v1/phases/02-plan.md` — "the
/// clamp only ever narrows"). Unit-agnostic: the caller supplies
/// `start`/`end`/`h` in the same unit (days, in the current
/// `smelt-runtime` call site).
pub fn clamp_write_range(start: i64, end: i64, h: i64) -> i64 {
    start.max(end - h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_grain_declaration_is_refused() {
        let err = validate_frozen_horizon(Grain::Key).unwrap_err();
        assert!(
            err.contains("Key"),
            "error must name the offending grain, got: {err}"
        );
    }

    #[test]
    fn partition_grain_declaration_is_admitted() {
        assert!(validate_frozen_horizon(Grain::Partition).is_ok());
    }

    #[test]
    fn clamp_narrows_start_to_end_minus_h() {
        // A 400-day run range with H = 90 days floors at end - 90d.
        let start = 0;
        let end = 400;
        let h = 90;
        assert_eq!(clamp_write_range(start, end, h), 310);
    }

    #[test]
    fn clamp_never_widens() {
        // A run range shorter than H is returned unchanged.
        let start = 350;
        let end = 400;
        let h = 90;
        assert_eq!(clamp_write_range(start, end, h), start);
    }
}
