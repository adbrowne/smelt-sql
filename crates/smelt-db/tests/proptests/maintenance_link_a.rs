//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `P0-5` (`docs/research/20260705-property-discovery-loop.md` §2.1;
//! `docs/plans/20260705-property-discovery-loop.md` phase D).
//!
//! Link-A: an **abstract** contract-safety pre-filter. It models a
//! `fold-delta` maintenance technique over an abstracted combiner (additive
//! vs idempotent monoid, Link 0) and the five adversarial run-schedule kinds
//! the design mandates every Link-A/Link-C cell draw from: partition
//! (arbitrary-size deltas), reorder, re-delivery, backfill (region recompute),
//! and late arrival landing within vs beyond a derived horizon. Retraction
//! (change-feed/mutable sources) is exercised by the dedicated MIN property
//! below.
//!
//! This is **refutation-only and pre-execution**: it never touches smelt's
//! analyzer or `execute_project` (that is Link C, in `smelt-cli`'s
//! `property_discovery` test target). Its job is to predict the
//! admission-matrix boundary abstractly and hand Link C the schedule *kinds*
//! worth replaying against real smelt — not to prove anything about smelt
//! itself.

use std::collections::HashSet;

use proptest::prelude::*;

/// Link 0's two combiner families relevant to `fold-delta` (the additive
/// monoid `SUM`/`COUNT`, and the idempotent monoid `MIN`/`MAX`/`BOOL_OR`/
/// `BOOL_AND`). `AVG`/holistic/running-total combiners are out of scope for
/// this abstract scaffold — they get their own cells (`G-04`, `G-07`, `G-08`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CombinerKind {
    /// `SUM`/`COUNT` — commutative, **not** idempotent: combining a value
    /// with itself changes the result.
    Additive,
    /// `MAX` — commutative and idempotent: combining a value with itself (or
    /// re-delivering it) is a no-op on the result.
    Max,
    /// `MIN` — the idempotent-but-non-invertible case Link 0 predicts breaks
    /// under retraction.
    Min,
}

impl CombinerKind {
    fn identity(self) -> i64 {
        match self {
            CombinerKind::Additive => 0,
            CombinerKind::Max => i64::MIN,
            CombinerKind::Min => i64::MAX,
        }
    }

    fn combine(self, a: i64, b: i64) -> i64 {
        match self {
            CombinerKind::Additive => a + b,
            CombinerKind::Max => a.max(b),
            CombinerKind::Min => a.min(b),
        }
    }
}

/// One event-time-bearing delta with a stable identity (for ledger dedup) and
/// a payload value.
#[derive(Debug, Clone, Copy)]
struct Delta {
    id: u64,
    value: i64,
}

/// The five adversarial schedule-step kinds design §2.1 mandates every
/// Link-A/Link-C cell draw from. `Deliver`/`Redeliver` also carry the
/// partition/reorder axes for free — they are just whatever order and
/// grouping the generated `Vec<AbstractStep>` happens to produce.
#[derive(Debug, Clone)]
enum AbstractStep {
    /// First delivery of a delta.
    Deliver(Delta),
    /// Re-delivery of a delta already seen (kind 2: idempotency check — does
    /// re-running double-count?).
    Redeliver(Delta),
    /// A region recompute: clears the ledger for the named ids and
    /// re-delivers them fresh (kind 3: does a backfill reset the ledger?).
    Backfill(Vec<Delta>),
    /// A delta arriving "late" — after its nominal window has already run —
    /// landing either within or beyond the technique's derived horizon
    /// (kind 4). A technique with a **bounded** horizon must drop
    /// `within_horizon: false` deltas from its own ledger even though the
    /// batch/full-refresh oracle always sees them.
    LateArrival { delta: Delta, within_horizon: bool },
}

/// A `fold-delta` technique simulator: a ledger keyed by delta id (the
/// "never fold a delta twice" obligation, design §2.0) plus a folded state.
/// A `LateArrival` beyond horizon is never folded at all — the abstract
/// shape of the `SC-1` hazard (an optimistic derived bound silently drops a
/// late arrival).
struct FoldDelta {
    kind: CombinerKind,
    ledger: HashSet<u64>,
    state: i64,
}

impl FoldDelta {
    fn new(kind: CombinerKind) -> Self {
        Self {
            kind,
            ledger: HashSet::new(),
            state: kind.identity(),
        }
    }

    /// Folds a delta exactly once per id — a re-delivery of an
    /// already-ledgered id is a no-op, by construction (the ledger
    /// obligation Link 0 requires additive combiners to honor).
    fn fold_once(&mut self, delta: Delta) {
        if self.ledger.insert(delta.id) {
            self.state = self.kind.combine(self.state, delta.value);
        }
    }

    fn apply(&mut self, step: &AbstractStep) {
        match step {
            AbstractStep::Deliver(d) | AbstractStep::Redeliver(d) => self.fold_once(*d),
            AbstractStep::Backfill(region) => {
                // A region recompute clears the ledger for exactly the named
                // ids, then re-delivers them fresh.
                for d in region {
                    self.ledger.remove(&d.id);
                }
                for d in region {
                    self.fold_once(*d);
                }
            }
            AbstractStep::LateArrival {
                delta,
                within_horizon,
            } => {
                if *within_horizon {
                    self.fold_once(*delta);
                }
                // Beyond-horizon: the technique never sees this delta at
                // all — modelling a derived bound that silently clamps it
                // away (the SC-1 shape at the abstract level).
            }
        }
    }
}

/// The batch/full-refresh oracle: recompute directly over every delta the
/// schedule ever delivered, regardless of the fold technique's horizon or
/// ledger — this is what `execute_project`'s full-refresh baseline does at
/// step k (design N3).
fn batch_aggregate(kind: CombinerKind, deltas: &[Delta]) -> i64 {
    deltas
        .iter()
        .fold(kind.identity(), |acc, d| kind.combine(acc, d.value))
}

/// `n` deltas with fixed ids `0..n` and proptest-generated payload values.
fn arb_deltas(n: usize) -> impl Strategy<Value = Vec<Delta>> {
    proptest::collection::vec(-50_i64..50, n).prop_map(|values| {
        values
            .into_iter()
            .enumerate()
            .map(|(i, value)| Delta {
                id: i as u64,
                value,
            })
            .collect()
    })
}

/// A schedule over a fixed delta set exercising partition/reorder/
/// re-delivery/backfill (kinds 1-3): each delta is delivered once, either
/// plainly or wrapped in a single-element `Backfill` (a region recompute),
/// plus a proptest-generated number of extra re-deliveries drawn from the
/// same set. This is the corner where an idempotent combiner is predicted
/// to HOLD (Link 0: idempotent monoids fold faithfully regardless of
/// re-delivery or region-recompute).
fn arb_schedule_for(deltas: Vec<Delta>) -> impl Strategy<Value = Vec<AbstractStep>> {
    let n = deltas.len().max(1);
    (
        proptest::collection::vec(any::<bool>(), deltas.len()),
        proptest::collection::vec(0..n, 0..=(2 * n)),
    )
        .prop_map(move |(backfill_flags, extra_ids)| {
            let mut steps = Vec::new();
            for (i, d) in deltas.iter().enumerate() {
                if backfill_flags[i] {
                    steps.push(AbstractStep::Backfill(vec![*d]));
                } else {
                    steps.push(AbstractStep::Deliver(*d));
                }
            }
            for extra_id in extra_ids {
                if let Some(d) = deltas.get(extra_id) {
                    steps.push(AbstractStep::Redeliver(*d));
                }
            }
            steps
        })
}

/// Combines `arb_deltas` + `arb_schedule_for` into one joint strategy over a
/// fixed-size delta set and one of its reorder/re-deliver/backfill schedules.
fn arb_safe_schedule(n: usize) -> impl Strategy<Value = (Vec<Delta>, Vec<AbstractStep>)> {
    arb_deltas(n).prop_flat_map(|deltas| {
        let for_schedule = deltas.clone();
        arb_schedule_for(for_schedule).prop_map(move |steps| (deltas.clone(), steps))
    })
}

proptest! {
    /// Acceptance (design plan, phase D): an idempotent monoid (`MAX`/`MIN`)
    /// folded via `fold-delta` over partitioned, reordered, re-delivered, and
    /// backfilled deltas matches the batch aggregate over the same delivered
    /// set — HOLDS over N. Idempotency means a duplicate delivery or a
    /// region-recompute backfill can never move the folded state away from
    /// the true aggregate.
    #[test]
    fn idempotent_monoid_fold_delta_matches_batch_over_reorder_redeliver_backfill(
        (kind, (deltas, steps)) in (
            prop_oneof![Just(CombinerKind::Max), Just(CombinerKind::Min)],
            1_usize..6,
        ).prop_flat_map(|(kind, n)| {
            arb_safe_schedule(n).prop_map(move |sched| (kind, sched))
        })
    ) {
        let mut technique = FoldDelta::new(kind);
        for step in &steps {
            technique.apply(step);
        }
        let batch = batch_aggregate(kind, &deltas);

        prop_assert_eq!(
            technique.state,
            batch,
            "idempotent combiner {:?} fold-delta diverged from batch over schedule {:?} \
             (deltas {:?}) — HOLDS predicted, found a witness",
            kind,
            steps,
            deltas
        );
    }

    /// The additive combiner (`SUM`) under the SAME re-delivery/backfill
    /// schedule kind stays correct too — but only because `FoldDelta`
    /// enforces the ledger ("never fold a delta twice", Link 0). This is the
    /// control: it proves the abstract model's ledger obligation is doing
    /// real work, not that `SUM` is idempotent (it isn't — `G-02` covers
    /// what happens when a real re-delivery has NO ledger).
    #[test]
    fn additive_monoid_fold_delta_with_ledger_matches_batch_over_reorder_redeliver_backfill(
        (deltas, steps) in (1_usize..6).prop_flat_map(arb_safe_schedule)
    ) {
        let mut technique = FoldDelta::new(CombinerKind::Additive);
        for step in &steps {
            technique.apply(step);
        }
        let batch = batch_aggregate(CombinerKind::Additive, &deltas);

        prop_assert_eq!(
            technique.state,
            batch,
            "additive combiner fold-delta (WITH ledger) diverged from batch over schedule {:?} \
             (deltas {:?})",
            steps,
            deltas
        );
    }

    /// Kind 4 (late arrival beyond horizon) — the abstract shape of `SC-1`.
    /// A `LateArrival { within_horizon: false }` step is, by `FoldDelta`'s
    /// construction, never folded — modelling a technique whose derived
    /// bound silently clamps the delta away. The batch oracle always sees
    /// it. This is a REFUTED witness for "fold-delta with a horizon that
    /// excludes a delta the batch oracle includes" — Link A predicting
    /// exactly the boundary `SC-1` will replay against real smelt in Link C.
    #[test]
    fn late_arrival_beyond_horizon_makes_fold_delta_diverge_from_batch(
        kind in prop_oneof![Just(CombinerKind::Max), Just(CombinerKind::Min), Just(CombinerKind::Additive)],
        seen_value in -50_i64..50,
        late_value in -50_i64..50,
    ) {
        let seen = Delta { id: 0, value: seen_value };
        let late = Delta { id: 1, value: late_value };

        let mut technique = FoldDelta::new(kind);
        technique.apply(&AbstractStep::Deliver(seen));
        technique.apply(&AbstractStep::LateArrival { delta: late, within_horizon: false });

        let batch = batch_aggregate(kind, &[seen, late]);

        // The witness only exists when the late value actually moves the
        // aggregate (e.g. MAX/MIN with a late value beyond the current
        // extreme, or any nonzero SUM contribution) — skip the degenerate
        // cases where dropping it happens not to matter.
        prop_assume!(technique.state != batch);

        prop_assert_ne!(
            technique.state,
            batch,
            "expected a beyond-horizon late arrival to diverge from batch (unreachable after \
             prop_assume)"
        );
    }
}

#[test]
fn min_fold_delta_diverges_when_the_unique_minimum_is_retracted() {
    // Kind 5 (retraction) — deterministic witness, not a proptest search:
    // MIN is idempotent but NON-INVERTIBLE (Link 0). `FoldDelta` has no
    // "un-fold" operation — a retraction of the delta currently holding the
    // minimum cannot lower the fold's memory of what the minimum was, while
    // the batch oracle, recomputed over the post-retraction active set,
    // necessarily reports a new (higher-or-equal) minimum. This is the
    // REFUTED witness for "fold-delta over MIN under retraction", matching
    // Link 0's prediction ("idempotent … over retractable/mutable? no
    // (non-invertible)") and the shape `G-04` will replay against real smelt.
    let deltas = vec![
        Delta { id: 0, value: 10 },
        Delta { id: 1, value: -5 }, // the unique minimum
        Delta { id: 2, value: 3 },
    ];

    let mut technique = FoldDelta::new(CombinerKind::Min);
    for d in &deltas {
        technique.apply(&AbstractStep::Deliver(*d));
    }
    assert_eq!(
        technique.state, -5,
        "fold-delta must fold to the true minimum before retraction"
    );

    // Retract id 1 (the unique minimum). Fold-delta has no inverse for MIN,
    // so its ledger simply never revisits id 1 — the state is left
    // unchanged (still -5), even though id 1 is no longer in the active set.
    let active: Vec<Delta> = deltas.into_iter().filter(|d| d.id != 1).collect();
    let batch_after_retract = batch_aggregate(CombinerKind::Min, &active);

    assert_ne!(
        technique.state, batch_after_retract,
        "fold-delta over MIN is expected to diverge from batch after retracting the unique \
         minimum — MIN is non-invertible (Link 0); this divergence IS the REFUTED witness, not \
         a test bug. technique retained {} but the active set's true minimum is now {}",
        technique.state, batch_after_retract
    );
}
