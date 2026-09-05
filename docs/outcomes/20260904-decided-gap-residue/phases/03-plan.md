# Phase 3 plan — once-write fallback-case nullability route

**Objective.** Give the once-write classifier the nullability route the fallback-bearing
spelling is missing: when a candidate's payload is provably non-null within its group, the
fallback can never stand in for a value a later window would supply, so the spelling stays the
plain `COALESCE(target, delta)` fold and needs no decomposed `(value, written)` state. Advances
success criterion 3 (classifier route + `incremental_shapes.md` bullet + generative pool
coverage).

**Scoping decision (record in the outcome's Decision log).** The route proves non-nullness from
the model's own `unique_key` only — the fact `classify_once_write` already holds, and exactly
the argument spec route 1 makes for the bare key-derived spelling. The driving-clock-derived
case stays out: `derive_fold_spec` (`smelt-db/src/queries/maintenance.rs`) resolves no driving
source, so admitting it would need a new plan-layer input and would risk CLI↔runtime admission
divergence. The Known-Divergences bullet is therefore *narrowed*, not deleted — its other two
clauses (key-derived expressions; whole-scope fan-out/set-op facts) belong to other tracks.

## Spec delta (spec-first — make these edits before the code)

`docs/specs/incremental_shapes.md`:

1. §"The column-family catalogue", the `COALESCE(MAX(<col>), <fallback>)` bullet — add: when
   `<col>` is provably non-null within its group (a `unique_key` column of the model), the
   fallback is dead and the spelling keeps the bare `COALESCE(target, delta)` fold with no
   decomposed state; the functional dependency is still required.
2. §"Decomposed state (rung 2) in keyed models", the sentence "The bare key-derived spelling
   needs no decomposed state…" — widen to "a spelling whose every candidate payload is provably
   non-null needs no decomposed state", giving the same by-construction reason.
3. §Known Divergences "The key grain", the bullet "The once-write classifier has no nullability
   route around the fallback case" — rewrite to the residual gap: the nullability route proves
   non-nullness only from the model's own `unique_key`; a driving-clock-derived payload still
   takes the decomposed-state route because the plan-layer derivation resolves no driving
   source. Keep the bullet's remaining two clauses verbatim.

## Tests (red-green, in this order)

1. `analysis::not_null` unit — `key_column_is_provably_not_null` / `non_key_column_is_not_proven`:
   the new pure `column_provably_not_null(unique_key, column)` returns `true` for a `unique_key`
   member (case-insensitively) and `false` otherwise.
2. `rules::cumulative` unit — `once_write_fallback_over_a_not_null_key_payload_admits_without_state`:
   `COALESCE(MAX(id), 0) AS first_id` with `unique_key = [id]` and a declared `id → id` FD
   yields `OnceWriteAdmission::Admitted { state: None }`.
3. `rules::cumulative` unit — `once_write_fallback_over_a_nullable_payload_still_decomposes`:
   `COALESCE(MAX(val), 0)` under an `id → val` FD still yields `Admitted { state: Some(_) }`
   (regression guard — the route must not swallow the ordinary fallback case).
4. `rules::cumulative` unit — `once_write_not_null_route_still_requires_the_functional_dependency`:
   the same not-null payload with NO declared FD stays `Unproven` (nullability never substitutes
   for per-key constancy).
5. `smelt-db` `maintenance_fold_spec_companion.rs` — `fold_spec_admits_the_not_null_fallback_spelling`:
   `derive_fold_spec` admits the phase's spelling (plan layer ↔ runtime admission parity).
6. `smelt-cli` `maintenance_conformance` — extend
   `once_write_null_pool_upholds_end_state_equivalence`'s combiner array with the new
   `KeyedCombiner::OnceWriteFallbackNotNull`, and assert its admitted cell carries no
   decomposed state columns (the route's observable consequence) while end-state equivalence
   still holds after every window.

## Tasks

1. Make the three spec edits above.
2. Add `pub fn column_provably_not_null(unique_key: &[String], column: &str) -> bool` to
   `crates/smelt-logical/src/analysis/not_null.rs`, documented as a leaf classifier; have
   `partition_column_provably_not_null`'s existing `unique_key` case call it rather than
   restating the check.
3. In `classify_once_write` (`crates/smelt-logical/src/rules/cumulative.rs`), after every
   candidate is FD-proven: if there is exactly one candidate, a fallback is present, and that
   candidate's payload column is `column_provably_not_null`, return
   `Admitted { state: None }` with a comment citing the spec sentence. Multi-candidate spellings
   keep the decomposed route.
4. Update `classify_once_write`'s doc comment to name the route.
5. Add `KeyedCombiner::OnceWriteFallbackNotNull` to `crates/smelt-maintenance-testkit/src/recipe.rs`:
   `kind_name`, `agg_and_alias`/`ordering_alias` arms, and `projection_sql` →
   `COALESCE(MAX(<key>), -1) AS <alias>`; keep it out of `arb_keyed_combiner` for the same
   world-fact reason as the other once-write variants.
6. In `crates/smelt-maintenance-testkit/src/render.rs`, emit the new variant's
   `functional_dependencies:` block with `determines: <key_column>` (self-determining, trivially
   true) instead of the payload column.
7. Extend the conformance test per test 6; add the no-decomposed-state assertion.
8. Append the phase's dated decision-log line to the outcome and write `phases/03-summary.md`.

## Verification

- `cargo test -p smelt-logical --lib` (units 1–4)
- `cargo test -p smelt-db --test maintenance_fold_spec_companion`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-runtime --test statement_parity`
- `bash .claude/scripts/verify-phase.sh`

## Commit message

`feat(once-write): admit a fallback over a provably non-null key payload without decomposed state`
