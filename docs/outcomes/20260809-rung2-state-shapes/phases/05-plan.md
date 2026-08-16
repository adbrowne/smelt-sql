# Phase 5 plan — admission: `MAX_BY`/`MIN_BY` without the companion projection

## Objective

Flip the order-monotone overwrite family onto the decomposed `(v, o)` state derived in phase 2:
`classify_order_monotone_column` stops requiring a hand-written `MAX(<ordering>)`/`MIN(<ordering>)`
companion and instead classifies with `state: Some(...)`, and the plan-layer `derive_fold_spec`
drops the same requirement so the two admissions stay identical. Advances success criteria 1
(companion obligation gone from code, spec and docs-site) and 4/6 (the phase-3 storage path and
the phase-2 collision detector become reachable from real SQL).

**Design decision (single path, not a fast path).** `MAX_BY`/`MIN_BY` *always* decomposes to
hidden `(v, o)` state — the companion-projection route is deleted, not kept as a stateless
optimisation. `order_monotone_companion` and its two call sites go away entirely. Rejected:
keeping the companion as a zero-state fast path, which would leave two admission modes, two
stored-table shapes for one family, and the duplicated proof `derive_fold_spec`/`faithful_fold`
warn about. A model that already projects `MAX(ord)` keeps that column as an ordinary
extremal-fold output; it simply no longer participates in the `MAX_BY` proof.

## Spec delta (spec-first — make these edits first)

`docs/specs/incremental_models.md`:
- **§"Known Divergences"** — delete the bullet "**The order-monotone overwrite family's ordering
  value still has no decomposed-state storage wired in.**" (~line 2372). It is false once this
  phase lands; leaving it is worse than the phase-8 tidy-up it was scheduled under.
- **§"The column-family catalogue"**, the sentence after the table (~line 313) — reword the
  degenerate case: `MAX_BY(x, x)` materialises the same uniform two-column `(v, o)` state, with
  the ordering state column repeating the value expression rather than introducing a new one.
  (Today's "duplicates rather than adds a column" reads as a one-column special case; there is
  no special case.)

`docs-site/docs/reference/cumulative-aggregate.md` (~line 61) — rewrite the `MAX_BY`/`MIN_BY`
paragraph: no companion projection is required; the ordering value is kept as internal state
invisible to consumers; ties still keep the incumbent. Drop the `KeyedUnknownCombiner` sentence.

## Tests (red-green, in this order)

`crates/smelt-logical/tests/keyed_families.rs`
1. `max_by_without_companion_admits_with_hidden_state` — rewrite of the existing
   `max_by_without_companion_projection_is_refused` (~line 171): `MAX_BY(status, updated_at) AS
   status` with no `MAX(updated_at)` projection classifies; the column carries
   `state: Some` with state columns `status__v`, `status__o` and presentation `status__v`.
2. `min_by_without_companion_admits_with_hidden_state` — the `MIN_BY` mirror; `status__o` folds
   with `Min`, `status__v` with `OrderMonotone { prefer_greater: false }`.
3. `max_by_self_companion_admits_with_uniform_state` — `MAX_BY(x, x)` still admits and produces
   the same two-column state shape (the spec reword above).
4. `max_by_with_redundant_max_projection_still_admits` — adapt an existing companion-bearing
   fixture: the `MAX(updated_at)` column classifies as an ordinary extremal fold and the `MAX_BY`
   column is state-bearing regardless.
5. `max_by_wrong_arity_refuses_unknown_combiner` — a 1- or 3-argument `MAX_BY` still refuses
   `KeyedUnknownCombiner` (fail-closed survives the widen).
6. `max_by_state_column_colliding_with_user_column_refuses` — a SELECT projecting both
   `MAX_BY(status, updated_at) AS status` and something aliased `status__o` refuses
   `KeyedStateColumnCollision` (first reachable exercise of the phase-2/3 detector).

`crates/smelt-db/tests/maintenance_fold_spec_companion.rs`
7. `max_by_without_companion_is_admitted` / `min_by_without_companion_is_admitted` — flip the two
   existing `*_is_not_admitted` tests: `derive_fold_spec` now yields a `FoldSpec` carrying the
   `ArgMax`/`ArgMin` column. Keep `max_by_self_companion_is_admitted` green. Rename the file's
   module doc away from "companion" framing (leave the filename alone — historical).

`crates/smelt-logical/tests/emit_statements.rs`
8. `keyed_merge_folds_max_by_through_hidden_ordering_state` — `build_cumulative_merge_sql` over a
   state-bearing `MAX_BY` classification emits per-state-column folds (`status__o` by
   `GREATEST`, `status__v` gated on `delta.status__o > target.status__o`) plus the presented
   `status` recomputed from merged state. (Phase 3 built the mechanism against a hand-built
   classification; this is the same assertion driven from real SQL via `classify_cumulative`.)

## Tasks

1. Make the two spec edits and the docs-site edit above.
2. Write tests 1–8 red.
3. `classify_order_monotone_column`: after the arity check, call
   `decomposed_state::decompose_to_state(sql_fn, false, &[value_text, ordering_text], alias)`;
   on `Ok`, push the `AggregatorColumn` with `state: Some(state)` and
   `cross_partition_combiner: OrderMonotone { ordering_column: <the state's `__o` column>,
   prefer_greater }`; on `Err`, refuse `KeyedUnknownCombiner` with the refusal's reason.
4. Delete `order_monotone_companion` and its now-dead helper usage; update the
   `CrossPartitionCombiner::OrderMonotone` and `classify_order_monotone_column` doc comments
   (the "Storage decision" paragraph is now wrong — replace with the state route).
5. `crates/smelt-db/src/queries/maintenance.rs::derive_fold_spec`: drop the
   `order_monotone_companion(...)?` call and its import; keep the exact-2-argument requirement so
   both layers still refuse the same wrong-arity shapes. Update the doc comment's companion
   paragraph.
6. `crates/smelt-logical/src/analysis/faithful_fold.rs`: update the module doc's sub-multiset-fold
   paragraph — the order-monotone widen no longer rests on a companion proof.
7. Grep the workspace for remaining `companion` references in production code/doc comments and
   fix each (`crates/smelt-logical/src/rules/incremental.rs`, `cumulative.rs`,
   `smelt-db/src/queries/maintenance.rs`); leave `docs/plans/` untouched (historical).
8. Confirm `build_state_bearing_models` (`smelt-runtime/src/execute.rs`) now classifies real
   `MAX_BY` models as state-bearing — no code change expected, assert via the conformance run.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test keyed_families --test emit_statements --test walk_coverage`
- `cargo test -p smelt-db --test maintenance_fold_spec_companion`
- `cargo test -p smelt-runtime --test statement_parity --test technique_lowering --test execute_parity`
- `cargo test -p smelt-cli --test maintenance_conformance` — the 47 existing recipes include
  `MAX_BY` models; they now execute end-to-end with materialised `(v, o)` state against real
  DuckDB and must still match the full-refresh oracle. This is the phase's strongest gate.

## Commit message

`feat(incremental): admit MAX_BY/MIN_BY on hidden (v, o) state, no companion projection`
