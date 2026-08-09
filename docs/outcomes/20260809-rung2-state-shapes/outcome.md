# Outcome: Rung 2 — decomposed combiner state

**Created:** 2026-08-09
**Status:** active
**Source:** `docs/research/20260809-incremental-rethink.md` §2 P-A, §6 step 1
**Spec anchors:** `docs/specs/incremental_models.md` (algebraic ladder, column-family catalogue), `docs/specs/model_properties.md` (algebraic discriminants)

## The outcome

Decomposable combiners get concrete auxiliary state: the stored table carries
the state columns a combiner needs to fold correctly (`AVG → (sum, count)`,
`MAX_BY(v, o) → (v, o)`, once-write → written-flag, `stddev`-family →
`(n, Σx, Σx²)`), and a presentation projection hides them from consumers.
Admission then widens to everything rung 2 licenses, and the user-visible
obligations that existed only because rung 2 was unbuilt are deleted.

## Success criteria (checkable)

1. `MAX_BY`/`MIN_BY` admitted **without** the hand-written companion
   `MAX(<ordering>)` projection; the companion-projection obligation is gone
   from spec and docs-site.
2. The once-write family admits the fallback-bearing and multi-candidate
   `COALESCE` spellings that today refuse with "waiting on machinery".
3. `AVG` (and at least one `stddev`-class aggregate) folds incrementally at
   keyed grain instead of refusing.
4. State columns are invisible to downstream consumers (presentation map
   projects them away; `smelt explain` shows them as internal state).
5. `cargo test -p smelt-cli --test maintenance_conformance` generates and
   passes decomposed-state recipes for every newly admitted family.
6. All standing gates green (`verify-phase.sh`, walk_coverage,
   statement_parity); no new whole-text scans (walk rule holds).

## Out of scope

- Ladder rungs 3–4 (change-feed consumption, bounded-domain multiset).
- Approximate-sketch state (HLL) — a later contract-lattice item.
- The `smelt.latest`/`smelt.once`/`smelt.current` pattern functions.

## Phases

One line each — intent only. The planner step details (and may reshape) this
list at the start of every phase; it must not defer work that serves the
success criteria above.

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec: decomposed-state semantics — state shapes, presentation projection, widened admissions, obligations to delete | pending |
| 2 | Derive concrete state shapes in `smelt-logical` for the decomposable catalogue (`decomposed_state.rs` stops refusing) | pending |
| 3 | Storage + emitters: state columns in the stored table, keyed fold over state, presentation projection | pending |
| 4 | Admission: `MAX_BY`/`MIN_BY` without the companion projection | pending |
| 5 | Admission: once-write fallback spellings and `AVG`/`stddev`-class folds | pending |
| 6 | Conformance-gate recipes for decomposed-state families; ledger grading audit | pending |
| 7 | Surface cleanup: delete superseded obligations from spec + docs-site; `smelt explain` state rendering | pending |

## Decision log

<!-- Dated one-liners appended by plan/implement steps. -->

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
