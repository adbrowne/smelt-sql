//! Oracle mode selection for mixed models
//! (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 4;
//! design doc `docs/research/20260711-generative-maintenance-conformance.md`
//! §6 "The equivalence oracle, generalized" — "Mixed models"): the pure
//! `OracleMode` enum a mixed (append-only fact + mutable dimension) case
//! selects between. Selection itself is one named type so the choice isn't
//! duplicated ad hoc at each call site; the bookkeeping of *when* a mutation
//! becomes outstanding and *when* a catch-up run clears it lives on
//! [`crate::s_tracker::STracker`] (it owns per-run/per-window state already,
//! design §6: "The S-tracker owns this bookkeeping").

#![allow(dead_code)]

/// Which equivalence discipline currently applies to a tracked model
/// (design §6 "Mixed models").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleMode {
    /// No mutation is outstanding against any region the driving source has
    /// processed: full S-restricted equivalence is assertable right now
    /// (Phase 3's append-only discipline; for a mixed model the dimension
    /// additionally contributes its CURRENT physical state — design §6).
    SRestricted,
    /// A mutable source's mutation is outstanding against a region no
    /// catch-up run has yet re-covered: full equality only holds at the next
    /// **settled point** (the catch-up run covering that region). Between
    /// now and then, only the weaker expected-staleness contract holds, and
    /// it is asserted non-fatally, never as a hard failure (design §6).
    SettledPoint,
}

/// One row of a keyed model's end state: `(key, value)` — generic enough for
/// both [`crate::recipe::KeyedCombiner`] families' single-aggregate-column
/// shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct KeyedOracleRow {
    pub key: i64,
    pub value: i64,
}

/// The snapshot-reconcile carve-out
/// (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 5;
/// `incremental_models.md` §"Two named carve-outs"; `incremental_models.md`
/// §"End-state equivalence": "Keys absent from the snapshot are retained (a
/// named divergence from the oracle relation — the stored table is the
/// oracle's rows plus retained departed keys)"). Pure data adjustment, no
/// I/O, so it is directly unit-testable independent of whether the
/// snapshot-reconcile executor exists (`incremental_models.md` §Known Divergences "The key grain":
/// "The snapshot-reconcile executor is unbuilt").
///
/// `retained_departed_keys` are keys present in stored state but absent from
/// the current snapshot; a key present in BOTH `oracle_rows` and
/// `retained_departed_keys` keeps its `oracle_rows` value (the fresher,
/// still-present one) — this is the documented adjustment itself, not a
/// blanket tolerance: every key not covered by the current oracle evaluation
/// is retained exactly once, nothing else is forgiven.
///
/// **No longer the default-point comparator.** Posture-derived deletion
/// (`docs/outcomes/20260816-keyed-grain-residue-v2` phase 1;
/// `incremental_shapes.md` §"Departed keys and deletion") means the
/// *default* contract point deletes a departed key rather than retaining
/// it — the unadjusted full-scan oracle is the right comparator there
/// (`crates/smelt-cli/tests/maintenance_conformance/gate.rs::
/// snapshot_reconcile_plain_overwrite_settles_after_key_departure`). This
/// function is retained as pure data because it is exactly the quotient
/// transform the `retain_departed` contract-lattice point (a DECLARED
/// relaxation, phase 2 of the same outcome) needs as its oracle — but no
/// default-point comparison in this crate calls it any more.
pub fn keyed_end_state_with_retained_departed_keys(
    oracle_rows: &[KeyedOracleRow],
    retained_departed_keys: &[KeyedOracleRow],
) -> Vec<KeyedOracleRow> {
    let oracle_keys: std::collections::HashSet<i64> = oracle_rows.iter().map(|r| r.key).collect();
    let mut merged: Vec<KeyedOracleRow> = oracle_rows.to_vec();
    for row in retained_departed_keys {
        if !oracle_keys.contains(&row.key) {
            merged.push(*row);
        }
    }
    merged.sort();
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two variants compare and print distinctly — a smoke test that the
    /// enum itself is usable as a `match` target the way `STracker::oracle_mode`
    /// and its callers rely on.
    #[test]
    fn oracle_mode_variants_are_distinct() {
        assert_ne!(OracleMode::SRestricted, OracleMode::SettledPoint);
    }

    /// `retained_departed_keys_adjusts_the_oracle`'s pure-function half (plan
    /// Phase 5 TDD list): a key departed from the current snapshot is
    /// retained exactly once; a key present in both sides keeps the oracle's
    /// (fresher) value, never duplicated.
    #[test]
    fn keyed_end_state_retains_departed_keys_exactly_once() {
        let oracle_rows = vec![
            KeyedOracleRow { key: 1, value: 10 },
            KeyedOracleRow { key: 2, value: 20 },
        ];
        // key 1 collides with the oracle (must NOT duplicate, oracle wins);
        // key 3 is a genuine departed key (retained).
        let stored_before = [
            KeyedOracleRow { key: 1, value: 999 },
            KeyedOracleRow { key: 3, value: 30 },
        ];
        let retained_departed: Vec<KeyedOracleRow> = stored_before
            .iter()
            .filter(|r| !oracle_rows.iter().any(|o| o.key == r.key))
            .copied()
            .collect();

        let adjusted =
            keyed_end_state_with_retained_departed_keys(&oracle_rows, &retained_departed);
        assert_eq!(
            adjusted,
            vec![
                KeyedOracleRow { key: 1, value: 10 },
                KeyedOracleRow { key: 2, value: 20 },
                KeyedOracleRow { key: 3, value: 30 },
            ]
        );
    }
}
