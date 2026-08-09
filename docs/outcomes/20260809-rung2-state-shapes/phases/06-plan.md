# Phase 6 plan — once-write: admit the fallback-bearing and multi-candidate spellings

## Objective

Wire `classify_once_write` to `decomposed_state::decompose_once_write` so the two spellings that
today refuse `KeyedOnceWriteUnproven` — `COALESCE(MAX(col), <fallback>)` and
`COALESCE(MAX(a), MAX(b), …)` — admit on hidden `(value, written)` state, with the fallback and the
preference order applied fresh in `π` on every read. Advances success criterion 2, and gives
criterion 4's collision detector its second reachable family.

## Spec delta

Behaviour is already normative (`docs/specs/incremental_models.md` §"The column-family
catalogue" — "The once-write family admits four spellings" — and §"Decomposed state (rung 2) in
keyed models"); this phase only makes the implementation match, so the spec edits are
status corrections, made first:

- §Known Divergences, bullet "The once-write classifier still implements only the two narrow
  spellings" — retitle and rewrite to the *residual* gap only: no nullability route around the
  fallback case, key-derived route still requires a bare key reference (not an arbitrary
  key-derived expression), whole-scope fan-out/set-op facts rather than a per-column join trace.
  Delete the "has not yet been wired to the decomposed-state mechanism" claim.
- §Known Divergences, bullet "Ladder rung 2 is specified but only partially wired" — narrow the
  residual to `AVG`/`STDDEV_*`/`VAR_*` (row 7); once-write is no longer listed.
- `docs-site/docs/reference/cumulative-aggregate.md` — the "**No fallback after the reduction.**"
  paragraph (~line 107) is now false: rewrite it as "the fallback is kept out of stored state",
  explaining that state holds the raw reduction plus a `written` flag; add the multi-candidate
  spelling to the admitted list (~line 85); rewrite the `KeyedOnceWriteUnproven` table row
  (~line 174) to drop "or carries a fallback argument".

## Design decisions this phase implements

- **Candidate/fallback split.** Arguments are read left to right: the leading maximal run of
  single-bare-column `MAX(...)`/`MIN(...)` reductions are the *candidates*; at most one further
  trailing argument is the *fallback*. Any other shape (a candidate after the fallback, a
  non-MAX/MIN aggregate, a multi-argument reduction) refuses `Unproven` as today.
- **Every candidate needs its own proof.** Route 2's existing FD verdict (declared FD over a
  subset key, `has_fan_out_join` / `has_set_op_barrier` structural disproofs) runs per candidate
  column; the first failing candidate refuses, naming that candidate in `column`.
- **The fallback must be presentable.** `decompose_once_write` is called with
  `same_row_columns = unique_key`, so a literal or a `unique_key` column passes F7 purity and
  anything else refuses `ImpurePresentation` → `Unproven` with that reason.
- **Stateless spellings unchanged** (see outcome Decision log): key-derived and bare
  `COALESCE(MAX(col))` keep `state: None`.

## Tests

Red-green, in this order:

1. `smelt-logical/tests/keyed_families.rs::once_write_with_fallback_admits_with_decomposed_state` —
   `COALESCE(MAX(plan), 'free')` + declared FD admits; column carries two state columns
   (`plan__value` OnceWrite, `plan__written` BoolOr) and a `CASE WHEN plan__written …` π.
2. `…::once_write_multi_candidate_admits_one_state_pair_per_candidate` —
   `COALESCE(MAX(a), MAX(b))` + FDs on both: four state columns, π preserves argument order.
3. `…::once_write_multi_candidate_with_fallback_admits` — candidates then literal fallback.
4. `…::once_write_multi_candidate_unproven_second_candidate_refuses` — FD on `a` only;
   `KeyedOnceWriteUnproven` names `b`.
5. `…::once_write_fallback_referencing_a_non_key_column_refuses` — fallback `other_col` refuses
   (impure presentation), does not admit.
6. `…::once_write_bare_reduction_stays_stateless` — regression: `COALESCE(MAX(col))` and the
   key-derived spelling still admit with `state: None`.
7. `…::once_write_state_column_collides_with_user_projection` — a model also projecting
   `plan__value` raises `KeyedStateColumnCollision`.
8. `smelt-db/tests/maintenance_fold_spec_companion.rs::once_write_with_a_literal_fallback_is_admitted_into_fold_spec`
   — flip the existing `_is_not_admitted` test; plus a multi-candidate admitted case, so the plan
   layer and the runtime classifier still agree.
9. `smelt-logical/tests/emit_statements.rs::once_write_fallback_folds_state_and_recomputes_presented`
   — real-SQL-driven: the keyed merge folds `__value` with `COALESCE(target, delta)`, `__written`
   with `OR`, and recomputes the presented column from the merged state.

## Tasks

1. Make the spec + docs-site edits above.
2. `OnceWriteAdmission::Admitted` gains a payload: `Admitted { state: Option<DecomposedState> }`.
3. In `classify_once_write`, replace the single-`first`-argument route 2 with candidate-list
   parsing (split rule above) and a per-candidate FD verdict loop; keep every existing refusal
   reason text for the shapes that still refuse.
4. When a fallback is present or candidates > 1, call `decompose_once_write(candidates, fallback,
   unique_key, output_name)` and return `Admitted { state: Some(...) }`; map `DecomposeRefusal`
   onto `Unproven` with a reason naming the refusal.
5. Update the two call sites: `classify_cumulative`'s `GroupByKey` arm (thread `state` into
   `AggregatorColumn`) and `smelt-db`'s `derive_fold_spec` (pattern update only).
6. Update the `KeyedUnknownCombiner` once-write suggestion text in `classify_cumulative`
   (~line 1032) — it still advertises "no fallback argument" as a requirement.
7. Run the gates; re-verify the two phase-5 traps hold for this shape (state columns here do not
   cross-reference each other, but the state expressions do need raw source columns).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test keyed_families --test emit_statements --test walk_coverage`
- `cargo test -p smelt-db --test maintenance_fold_spec_companion`
- `cargo test -p smelt-runtime --test statement_parity --test technique_lowering`
- `cargo test -p smelt-cli --test maintenance_conformance` (47 recipes must stay green —
  no already-admitted spelling changes shape)

## Commit message

`feat(incremental): admit once-write fallback/multi-candidate spellings on (value, written) state`
