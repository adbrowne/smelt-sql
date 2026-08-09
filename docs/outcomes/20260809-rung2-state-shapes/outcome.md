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
| 1 | Spec: decomposed-state semantics — state shapes, presentation projection, widened admissions, obligations to delete | done |
| 2 | Derive concrete state shapes in `smelt-logical` for the decomposable catalogue (`decomposed_state.rs` stops refusing); widen `π` purity to the new shapes; pure state/user column collision detector | done |
| 3 | Storage + emitters: state columns materialised in the stored table, keyed fold over state, `KeyedStateColumnCollision` diagnostic wiring | planned |
| 4 | Presentation projection: state columns invisible to `ref()` expansion, `SELECT *`, declared-schema checks, downstream type inference | pending |
| 5 | Admission: `MAX_BY`/`MIN_BY` without the companion projection | pending |
| 6 | Admission: classify the once-write fallback/multi-candidate spellings onto the derived `(value, written)` state; admit `AVG`/`stddev`-class folds | pending |
| 7 | Conformance-gate recipes for decomposed-state families; ledger grading audit | pending |
| 8 | Surface cleanup: delete superseded obligations from spec + docs-site; `smelt explain` state rendering | pending |

## Decision log

<!-- Dated one-liners appended by plan/implement steps. -->

- 2026-08-09 (plan 1): no reshape — phase list stands as scaffolded. Split of spec work
  fixed: phase 1 writes the normative rung-2 semantics and *rewrites* the three rung-2
  Known Divergences entries to the residual gap; phase 7 deletes them plus the docs-site
  obligations once the code actually behaves that way (success criteria 1–2 need both).
- 2026-08-09 (plan 1): physical-layout decision handed to the phase-1 spec pass — state
  columns live in the same stored table as the presented columns (suffix `__part`),
  hidden from the public schema, rather than a separate state table + presentation view;
  keeps `ref()` a table and leaves backend DDL/atomic-swap paths untouched.
- 2026-08-09 (implement 1): phase 1 landed the spec text (`incremental_models.md`
  §"Decomposed state (rung 2) in keyed models", catalogue/diagnostics/Known-Divergences
  edits, plus `model_properties.md`/`model_transforms.md` cross-references). Self-review
  caught and fixed a stale Design-section claim ("keyed families sit on the direct-monoid
  rung") and two missing table rows (admission matrix, derived execution postures) for
  the new decomposed-fold family. Only `AVG` is encoded in `decomposed_state.rs` today;
  phase 2 needs to decide whether `MAX_BY`/once-write widen through the same
  `decomposable`-discriminant entry point or need their own.
- 2026-08-09 (plan 2): entry-point decision for the summary's open question — `decompose_to_state`
  stops gating on `combiner_discriminants(...).decomposable` and instead refuses only the
  *holistic-or-unknown* verdict (no monoid fact, no decomposability, no monotonicity). `MAX_BY`/
  `MIN_BY` (`Monotone::Order`) then reach the state-shape match without restating F4's raw facts:
  the discriminants stay exactly as `model_properties.md` defines them, and "has an encoded state
  shape" stays a property of this mechanism, not of the algebra. Rejected: flipping `ArgMax`/
  `ArgMin` to `decomposable: true`, which would corrupt a raw discriminant to serve one consumer.
- 2026-08-09 (plan 2, reshape): once-write is not a `SqlFunction`, so its `(value, written)` state
  needs its own entry point. Phase 2 derives it from an already-classified spelling; the SQL-level
  classification of the fallback/multi-candidate spellings stays phase 5 (row text sharpened).
  `KeyedStateColumnCollision` splits: the pure detector lands in phase 2 alongside the shapes it
  checks, the diagnostic wiring in phase 3 where the plan first carries state columns (row text
  sharpened). No work left the outcome.
- 2026-08-09 (implement 2): `decompose_to_state` now derives variance/stddev `(n, sx, sxx)`,
  `ARG_MAX`/`ARG_MIN` `(v, o)`, and once-write `(value, written)` (via a new
  `decompose_once_write` entry point) state shapes, gated on `Discriminants::is_holistic_or_unknown()`
  per the plan-2 decision. `presentation.rs`'s existing walk needed no new arms — CASE/binary-op/
  scalar-function coverage already accepted the new `π` shapes. All 14 new/updated
  `decomposed_state` tests and 3 new `presentation` tests pass; full `verify-phase.sh` green.

- 2026-08-09 (plan 3, reshape): old row 3 split into two — row 3 (state columns materialised +
  keyed fold over state + collision diagnostic) and a new row 4 (presentation projection: hiding
  state columns from `ref()` expansion, `SELECT *`, declared-schema checks, downstream type
  inference). The hiding half lives in a different layer (`smelt-db`/`smelt-runtime` schema
  resolution) from the storage/emitter half and is what success criterion 4 checks; bundling both
  in one phase made a row that could not be red-green'd coherently. Admission/conformance/surface
  rows shift to 5–8. Nothing left the outcome.
- 2026-08-09 (plan 3): phase 3 carries state through the classification without widening
  admission — the state-bearing shapes stay unreachable from real SQL until rows 5–6 flip
  admission, so phase 3's tests construct the state-bearing classification directly. Rejected:
  folding the `MAX_BY` admission flip into phase 3 to get an end-to-end fixture, which would make
  one phase both mechanism and admission and leave no clean red test for either.
- 2026-08-09 (plan 3): small spec correction queued into phase 3 (spec-first) — the state-shape
  catalogue's once-write "combiner over state" cell currently reads last-write-wins ("`value` is
  the incumbent's unless the delta's `written` is true, in which case the delta's"), which
  contradicts the family's first-write-wins semantics and its own rung-1 `COALESCE(target, delta)`
  form. The fold is `COALESCE(target.value, delta.value)`.

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
