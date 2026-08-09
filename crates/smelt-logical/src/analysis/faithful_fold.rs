//! The faithful-fold conditions proof.
//!
//! See `docs/specs/model_properties.md` §"Faithful-fold conditions".
//! `faithful_fold` composes two independently-sourced conditions into one
//! typed verdict:
//!
//! 1. **Partitioned input** — the delta stream *partitions* the input: every
//!    row lands in exactly one delta, none are retracted-then-relanded
//!    without a matching retraction in the stream. This is a fact of the
//!    source's **declared and verified mutation posture** (`sources.md`),
//!    never derived from the model's SQL. The [`InputDeltaKind`] discovery
//!    verdict refines the *reason* only (a clocked window-forward read has a
//!    distinct blind spot: an in-place update to an already-processed
//!    partition goes entirely unseen, not merely un-undoable) — it can never
//!    widen a non-append-only posture to "retraction-free".
//! 2. **Sub-multiset fold** — the combiner's fold over any sub-multiset of a
//!    partition equals the fold over the whole partition. This is a fact of
//!    the algebraic discriminants ([`combiner_discriminants`]): a
//!    commutative monoid satisfies it; a holistic or unrecognised aggregate
//!    never does. The admitted set is `is_monoid`, widened by the
//!    order-monotone overwrite family (`MAX_BY`/`MIN_BY`, `ArgMax`/`ArgMin`
//!    — a semilattice fold, not a commutative monoid) when the plan layer's
//!    `FoldSpec` already carries the column: `smelt_db::queries::maintenance
//!    ::derive_fold_spec` only ever puts an `ArgMax`/`ArgMin` column into a
//!    `FoldSpec` after the same exact-arity check the runtime classifier
//!    (`smelt_logical::rules::cumulative::classify_order_monotone_column`)
//!    enforces — both layers decompose the column to hidden `(v, o)` state
//!    (`smelt_logical::analysis::decomposed_state::decompose_to_state`)
//!    rather than requiring a companion projection, so this stage never
//!    re-derives that proof itself. A decomposable non-monoid outside that
//!    family (`AVG`) is still not admitted here.
//!
//! Both conditions must hold for a fold technique to be admissible; either
//! failing alone refuses, and each is reported independently — the spec's
//! canonical example (a replayable, retraction-free feed folded through a
//! non-invertible combiner) passes (1) and fails (2); its dual (retractions
//! into a monoid) fails (1) and passes (2).
//!
//! Pure function over declared facts and prior verdicts — no SQL text is
//! read here, so this is neither a leaf classifier nor an advisory scan
//! under the property-composition-walk rule.

use serde::Serialize;
use smelt_types::SqlFunction;

use super::discriminants::{combiner_discriminants, Monotone};
use super::input_delta::{InputDeltaKind, MutationProfile};

/// One faithful-fold condition, independently reported: holds, or fails with
/// a rendered reason preserving the admission-refusal vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ConditionVerdict {
    /// The condition holds.
    Holds,
    /// The condition fails; `reason` is the rendered refusal prose for this
    /// condition alone (the consumer wraps it with cell context — source
    /// name, column, combiner roster — which this pure proof does not know).
    Fails { reason: String },
}

impl ConditionVerdict {
    /// Whether this condition holds.
    pub fn holds(&self) -> bool {
        matches!(self, ConditionVerdict::Holds)
    }
}

/// The faithful-fold verdict: both conditions hold, or the composition
/// fails with **each** condition independently reported — a failing verdict
/// never collapses the two into one boolean, so a consumer (or a reader of
/// `smelt explain`) can see *which* condition refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum FaithfulFold {
    /// Both conditions hold — a fold technique is admissible as far as this
    /// proof is concerned.
    Holds,
    /// At least one condition fails. Both are carried, including the one
    /// that holds, because the conditions are independent facts.
    Fails {
        /// Condition (1): the delta stream partitions the input.
        partitioned_input: ConditionVerdict,
        /// Condition (2): the fold over any sub-multiset equals the batch
        /// aggregate.
        submultiset_fold: ConditionVerdict,
    },
}

/// Prove the faithful-fold conditions for one combiner over one source.
///
/// `combiner`/`distinct` feed condition (2) via [`combiner_discriminants`]
/// (fail-closed: only a proven commutative monoid passes). `posture` is the
/// source's declared mutation posture — the sole source of the
/// retraction-freedom fact for condition (1) — and `discovery` is the
/// already-proven input-delta discovery kind for the same source, consulted
/// only to render the condition-(1) failure reason precisely (the
/// window-forward blind spot vs. the snapshot-diff/change-feed case).
pub fn faithful_fold(
    combiner: SqlFunction,
    distinct: bool,
    posture: &MutationProfile,
    discovery: InputDeltaKind,
) -> FaithfulFold {
    // Condition (1): declared posture only. Fail-closed: anything other
    // than a declared append-only posture may carry retractions (a change
    // feed reports updates/deletes — it makes them *visible*, not absent).
    let partitioned_input = if *posture == MutationProfile::AppendOnly {
        ConditionVerdict::Holds
    } else {
        let reason = if discovery == InputDeltaKind::WindowForward {
            // The SC-2 blind spot (`docs/research/property-discovery/
            // ledger.md`): a clocked source's WindowForward discovery only
            // proves how *new* rows are found — an in-place update to an
            // already-processed partition would never be re-visited at all.
            "the source is not append-only, and input-delta discovery classifies it as \
             window-forward (clocked) — a window-forward incremental read only visits new \
             partitions, so an in-place update to an already-processed partition would go \
             entirely unseen by the next run, not merely un-undoable; no un-fold mechanism \
             exists to undo an already-folded contribution either, so this refuses the fold \
             family regardless of the combiner's own algebra — the two faithful-fold \
             conditions are independent and either alone refuses"
                .to_string()
        } else {
            format!(
                "the source is not append-only and may carry retractions (input-delta \
                 discovery = {discovery:?}); no un-fold mechanism exists to undo an \
                 already-folded contribution, so this refuses the fold family regardless of \
                 the combiner's own algebra — the two faithful-fold conditions are \
                 independent and either alone refuses"
            )
        };
        ConditionVerdict::Fails { reason }
    };

    // Condition (2): combiner algebra. The admitted set is `is_monoid`,
    // widened to also admit the order-monotone overwrite family
    // (`Monotone::Order` — `MAX_BY`/`MIN_BY`, `incremental_models.md`
    // §"The column-family catalogue"): it is a semilattice fold, not a
    // commutative monoid (`is_monoid` stays `false` for it,
    // `discriminants.rs`), but its sub-multiset fold IS well-defined under
    // the same sequential-application discipline this cell's caller
    // already enforces (window-forward keyed steps apply in temporal
    // order — §"The two run shapes"; overwrite columns additionally force
    // that ordering themselves, §"Order-independence") — a decomposable
    // non-monoid (`AVG`) is still not admitted by this proof.
    let discriminants = combiner_discriminants(combiner, distinct);
    let submultiset_fold = if discriminants.is_monoid || discriminants.monotone == Monotone::Order {
        ConditionVerdict::Holds
    } else {
        ConditionVerdict::Fails {
            reason: "is holistic or unrecognised (not a monoid) — no delta+state read \
                     exists; only the recompute family (a full rebuild) can serve this cell"
                .to_string(),
        }
    };

    if partitioned_input.holds() && submultiset_fold.holds() {
        FaithfulFold::Holds
    } else {
        FaithfulFold::Fails {
            partitioned_input,
            submultiset_fold,
        }
    }
}
