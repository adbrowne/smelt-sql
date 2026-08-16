# Phase 2 summary — derive concrete state shapes in `smelt-logical`

## Shipped

- `Discriminants::is_holistic_or_unknown()` (`discriminants.rs`) — the entry-gate predicate
  `decompose_to_state` now checks instead of `!decomposable`, so an order-monotone combiner
  reaches the state-shape match without changing any raw F4 fact.
- `decompose_to_state` widened from `arg_expr: &str` to `arg_exprs: &[&str]`, matched on
  `(function, arg_exprs)` — a wrong-arity call falls through to `UnknownStateShape`, never panics.
- Variance/stddev family (`VAR_POP`/`VAR_SAMP`/`VARIANCE`/`STDDEV`/`STDDEV_POP`/`STDDEV_SAMP`) ->
  `(n, sx, sxx)` state with the population/sample divisor and minimum-`n` NULL guard, `STDDEV_*`
  wrapped in `SQRT` (`decompose_variance_family`, `variance_family_shape`).
- `ARG_MAX`/`ARG_MIN` (`MAX_BY`/`MIN_BY`) -> `(v, o)` state via `decompose_arg_by`; `π` is the bare
  `v` column.
- `decompose_once_write(candidates, fallback, same_row_columns, output_name)` +
  `OnceWriteCandidate` — single, fallback-bearing, and multi-candidate once-write spellings all
  decompose to per-candidate `(value, written)` state; `π` applies the fallback/preference order
  fresh on every read, never merged into the stored value.
- `state_column_collisions(state_columns, user_columns)` — pure detector returning colliding
  `(state, user)` name pairs, ready for phase 3's `KeyedStateColumnCollision` diagnostic.
- `build_decomposed_state` gained an `extra_pure_columns` parameter threading once-write's
  caller-vouched same-row columns into the F7 purity proof.
- `presentation.rs`: three new tests (`variance_closed_form_is_pure`, `bare_state_column_is_pure`,
  `coalesce_over_state_columns_is_pure`) — all pass unchanged against the existing walk; no new
  arms were needed (CASE/binary-op/scalar-function coverage already handled these shapes).

## Decisions

- Entry gate keyed on `is_holistic_or_unknown()`, not `!decomposable` — see outcome.md decision
  log (plan 2). Confirmed by `order_monotone_reaches_state_shape_despite_non_decomposable_discriminant`.
- Wrong-arity calls reuse `DecomposeRefusal::UnknownStateShape` rather than a new variant — no
  test or consumer needs to distinguish "wrong arity" from "no encoded shape"; both are "this
  mechanism doesn't have a shape for this call."
- Once-write naming: single candidate -> `__value`/`__written`; multiple -> `__value_<i>`/
  `__written_<i>` (1-based), matching the spec catalogue's suffix convention.
- Multi-candidate `π` supports an optional trailing fallback (`ELSE <fallback>` after the
  preference-order `WHEN`s) even though the two catalogue rows tested don't combine multi-candidate
  with a fallback — the shape falls out of the same CASE construction as the single-candidate case,
  so no separate code path was needed.

## For the next planner

- Phase 3 (storage + emitters) can consume `StateColumn`/`DecomposedState`/`state_column_collisions`
  directly; no rework needed here.
- Phase 3's `KeyedStateColumnCollision` diagnostic wiring: `state_column_collisions` returns raw
  name pairs only — the diagnostic's message text (naming the reserved suffix) is phase 3's to
  compose, not derived here (an earlier draft added a `STATE_COLUMN_RESERVED_SUFFIX` constant but
  it didn't correspond to anything the detector actually needed, so it was dropped).
- `decompose_once_write`'s SQL classification of which real spelling maps to which
  `OnceWriteCandidate`/fallback is explicitly phase 5's job (this phase only decomposes an
  already-classified candidate list).
- No follow-up work was found that serves this outcome's success criteria and was skipped —
  everything in the phase-2 plan's task list landed.

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy zero-warnings, full workspace
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test walk_coverage` — 4/4 pass (no new whole-text scan).
- `cargo test -p smelt-logical --quiet` — all green.
- `rg -n "decomposable" crates/smelt-logical/src/analysis/discriminants.rs` — confirmed the raw
  F4 facts (`decomposable: bool` field and its per-function values) are untouched; only the new
  `is_holistic_or_unknown` doc comment and test names reference the word.
