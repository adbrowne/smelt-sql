# Phase 2 plan — derive concrete state shapes in `smelt-logical`

## Objective

Make `crates/smelt-logical/src/analysis/decomposed_state.rs` derive the concrete state shape and
presentation map `π` for every family in the spec's state-shape catalogue — `AVG` (already
encoded), `STDDEV_*`/`VAR_*`, `MAX_BY`/`MIN_BY`, and once-write — instead of refusing with
`UnknownStateShape`/`Holistic`. Pure derivation only: no storage, no emitters, no admission
wiring (phases 3–5 consume this). Advances success criteria 1, 2 and 3 by supplying the state
shapes their admissions need, and criterion 4 by deriving the presented-vs-state column split.

## Spec delta

None. `docs/specs/incremental_models.md` §"Decomposed state (rung 2) in keyed models" already
fixes every shape and `π` this phase encodes (phase 1). No user-visible behaviour changes here —
the derivation has no consumer until phase 3.

## Design decisions this phase implements

- **Entry gate.** `decompose_to_state` refuses `Holistic` only for the fail-closed
  *holistic-or-unknown* discriminant verdict (`!is_monoid && !decomposable && monotone == None`),
  not for `!decomposable`. This lets `ArgMax`/`ArgMin` (`Monotone::Order`) reach the shape match
  without changing any raw F4 fact in `discriminants.rs`. Everything reaching the match with no
  encoded shape still fails closed as `UnknownStateShape`.
- **Once-write entry point.** Once-write is a spelling, not a `SqlFunction`, so it gets
  `decompose_once_write(candidates, fallback, output_name)` taking an *already-classified*
  spelling (phase 5 does the SQL classification). `π` is purity-proven over the state columns plus
  the caller-supplied same-row column names it vouches for; a fallback reaching anywhere else is
  `ImpurePresentation`, never assumed pure.
- **Naming.** Suffixes follow the spec catalogue exactly: `__sum`/`__count`, `__n`/`__sx`/`__sxx`,
  `__v`/`__o`, `__value`/`__written` (per candidate: `__value_<i>`/`__written_<i>`).

## Tests (red-green, in `decomposed_state.rs` unit tests unless noted)

- `variance_family_decomposes_to_n_sx_sxx` — each of `VAR_POP`/`VAR_SAMP`/`VARIANCE`/`STDDEV`/
  `STDDEV_POP`/`STDDEV_SAMP` yields `(n, sx, sxx)` state with the additive per-partition exprs.
- `variance_presentation_uses_family_divisor_and_minimum_n` — population vs sample divisor
  differ; each `π` is `NULL` below the family's minimum `n` (`0` pop, `1` samp); `STDDEV_*` wraps
  its variance form in `SQRT`.
- `max_by_decomposes_to_value_and_ordering_state` — `MAX_BY(v, o)` yields `__v`/`__o`, `π` is
  `__v` alone (the ordering value is never presented); `MIN_BY` mirrors it.
- `order_monotone_reaches_state_shape_despite_non_decomposable_discriminant` — pins the entry-gate
  decision: `combiner_discriminants(ArgMax, false).decomposable` is still `false` and the
  decomposition still succeeds.
- `holistic_combiner_is_refused` / `exact_distinct_is_refused_as_holistic` — existing tests must
  still pass unchanged under the new gate (regression guard on fail-closedness).
- `approx_count_distinct_still_refuses_unknown_state_shape` — sketch state is explicitly out of
  scope for this outcome; must fail closed, not guess.
- `once_write_single_reduction_decomposes_to_value_and_written` — no-fallback spelling:
  `(value, written)`, `written = (value IS NOT NULL)`, `π = value`.
- `once_write_fallback_is_applied_in_presentation_not_merged` — `π` is
  `CASE WHEN written THEN value ELSE <fallback> END`; the state expr is the *bare* reduction.
- `once_write_multi_candidate_keeps_one_pair_per_candidate` — two candidates → four state columns;
  `π` applies the declared preference order over the written candidates.
- `once_write_fallback_reaching_outside_the_row_is_refused` — a fallback referencing a column the
  caller did not vouch for yields `ImpurePresentation`.
- `state_column_collision_is_detected` — pure detector returns the colliding
  `(state column, user column)` pairs; a non-colliding set returns empty.
- In `presentation.rs`: `variance_closed_form_is_pure`, `bare_state_column_is_pure`,
  `coalesce_over_state_columns_is_pure` — the new `π` shapes pass the F7 proof (add only what the
  walk genuinely does not yet accept; do **not** loosen a fail-closed arm to make one pass).

## Tasks

1. Add `Discriminants::is_holistic_or_unknown()` (or an equivalent predicate) and switch
   `decompose_to_state`'s gate to it; keep the `UnknownStateShape` fallthrough.
2. Encode the variance/stddev family: `(n, sx, sxx)` state columns with `COUNT(x)`, `SUM(x)`,
   `SUM(x * x)` per-partition exprs; per-family closed-form `π` with the minimum-`n` guard.
3. Encode `ArgMax`/`ArgMin`: `(v, o)` state from the two argument expressions, `π = <v state col>`.
   `decompose_to_state`'s single `arg_expr` parameter does not fit a two-argument combiner — widen
   it to a slice of argument expressions (or add the ordering argument), updating the `AVG` call
   sites; a wrong-arity call is a refusal, not a panic.
4. Add the once-write entry point + its `OnceWriteCandidate`/fallback input types and `π` builders.
5. Thread the caller-vouched same-row column names into `build_decomposed_state`'s purity call.
6. Add the pure state/user column-collision detector (names + reserved suffix in its output, so
   phase 3's diagnostic can render the spec's message without re-deriving).
7. Extend `presentation.rs`'s walk only as far as the new `π` shapes require; every added arm keeps
   the fail-closed default.
8. Doc-comment each state shape with the spec section that fixes it (§"Decomposed state (rung 2) in
   keyed models") — no re-specification of the rule in code comments.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test walk_coverage` (walk rule: this phase adds no whole-text scan)
- `cargo test -p smelt-logical --quiet 2>&1 | tail -40`
- `rg -n "decomposable" crates/smelt-logical/src/analysis/discriminants.rs` — confirm the raw F4
  facts are untouched by this phase.

## Commit message

`feat(incremental): derive decomposed state shapes for the rung-2 keyed catalogue`
