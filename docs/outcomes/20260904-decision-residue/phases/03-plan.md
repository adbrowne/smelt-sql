# Phase 3 plan — route 2's derived key-derived-expression sub-route

## Objective

Make key temporal locality route 2 derive the key→partition dependency from the model's own SQL
where decidable, consulting the declared `functional_dependencies:` entry only where it is not
(decision 3, derive-over-declare). Advances success criterion 3: a model whose partition
projection is provably a per-key constant admits route 2 with no declaration, the
maintenance-conformance pool carries a recipe for that shape, and the declared-FD sub-route
still admits where derivation is undecidable.

## Spec delta (first)

`docs/specs/incremental_shapes.md`:

- §"Key temporal locality (the time-partitioned output)", route 2 sentence — state the two
  sub-routes in order: **derived** (the partition projection is a deterministic expression over
  `unique_key` columns only — key membership establishes per-key constancy, the same argument
  §"The column-family catalogue"'s once-write key-derived spelling makes, extended to a
  `MIN`/`MAX` wrapper), then **declared** (a `functional_dependencies:` entry, for an origin
  derivation cannot decide). Say explicitly that the derived sub-route is consulted first and
  outranks the extremal-fold refusal *only* when every column reference in the projection is a
  key column (a `MAX` over the key is the key); an extremal fold over a non-key column stays
  refused for route 2 and remains route 3's shape.
- §Known Divergences — delete the "**Key temporal locality route 2 admits only a declared
  functional dependency**" bullet, and narrow the "no runnable end-to-end route-2 fixture exists
  yet" clause of the "Locality machinery gaps" bullet to whatever is still true after the
  fixture below lands (delete the clause if the fixture runs end-to-end).

## Tests

Unit — `crates/smelt-logical/src/analysis/key_derived.rs` (new, CST-based over
`analysis::analyze_select`; no raw-text scan, so `walk_coverage` stays green):

- `cast_of_a_key_column_is_key_derived` — `CAST(d AS DATE) AS pdate` with `unique_key: [id, d]`.
- `min_max_wrapper_over_a_key_column_is_key_derived` — `MAX(d) AS pdate`, same key.
- `reference_outside_the_key_is_not_derived` — `CAST(other AS DATE)` names the offending column.
- `mixed_key_and_non_key_refs_are_not_derived` — one non-key ref disqualifies the expression.
- `nondeterministic_function_is_not_derived` — `CAST(NOW() AS DATE)` refused even with no
  non-key column reference.
- `absent_projection_is_not_derived` — `partition_column` not in the select list.

`crates/smelt-logical/src/maintenance/locality.rs` `#[cfg(test)]`:

- `route2_derived_admits_with_no_declaration` — key-derived `pdate`, `declared_functional_
  dependencies` empty ⇒ `LocalitySlice::DeltaValues`.
- `route2_derived_outranks_the_extremal_refusal` — `MAX(<key col>) AS pdate` admits `DeltaValues`
  despite the `Monotone::Value` discriminant.
- `route2_extremal_over_a_non_key_column_still_refuses_route2` — the existing extremal refusal is
  unchanged (falls through to route 3).
- `route2_declared_fd_still_admits_when_underivable` — non-key `CAST(d AS DATE)` + declared FD ⇒
  `DeltaValues` (fallback preserved).
- `route2_derived_still_obeys_the_structural_preconditions` — a key-derived projection that is
  not NOT-NULL-provable / wrong granularity still refuses.

Conformance pool — `crates/smelt-maintenance-testkit` + `crates/smelt-cli/tests/maintenance_conformance/`:

- `composed_key_derived_recipe_admits_route2_with_no_declaration` — the new
  `ComposedRoute::KeyDerived` recipe renders no `functional_dependencies:` entry and is admitted
  by the real gate with a `DeltaValues` slice (via `assert_composed_admitted_with_expected_route`).
- The generative composed family samples `KeyDerived` and drives its merge mechanics to equality
  with the full-refresh oracle on the same channel `KeyDetermined` uses.

End-to-end fixture — `crates/smelt-runtime/tests/locality_route2_derived.rs` (new):

- `key_derived_partition_model_admits_and_runs` — stage a keyed model
  (`unique_key: [id, d]`, `pdate = CAST(d AS DATE)`, `timeseries.partition_column: pdate`) with
  **no** `functional_dependencies:`, derive the plan through the real Salsa path, and drive it
  as far as the pipeline accepts, asserting the stored table equals the full-refresh oracle.

## Tasks

1. Edit the spec sections above (spec-first).
2. Add `crates/smelt-logical/src/analysis/key_derived.rs`: a pure, CST-based leaf classifier
   `key_derived_partition_verdict(sql, unique_key, partition_column) -> KeyDerivedVerdict`
   (`Derived` / `NotDerived(reason)`), reusing `analysis::analyze_select` for the select item and
   the existing nondeterminism predicate; doc-comment it as a leaf classifier invoked over one
   already-bounded node's own text, per the property-composition-walk rule. Red-green the unit
   tests.
3. Wire it into `establish_locality`'s route 2 **before** the extremal check: on `Derived`,
   return `LocalitySlice::DeltaValues`; otherwise keep today's extremal refusal → declared-FD
   verdict order untouched, and fold the `NotDerived` reason into route 2's captured local
   refusal message so a model that reaches the three-route message still gets the specific
   diagnosis.
4. Add `ComposedRoute::KeyDerived` to `crates/smelt-maintenance-testkit/src/recipe.rs`
   (`composed_route_name`, `arb_composed_route`, `unique_key`, `partition_column`,
   `functional_dependency` → `None`, `matrix_cell_id`) and render its body
   (`SELECT id, d, CAST(d AS DATE) AS pdate, SUM(val) AS total ... GROUP BY id, d`); follow the
   compile errors through `render.rs` and `families/gate_composed.rs`.
5. Establish whether the new recipe/fixture is executable through `execute_project`; if
   `classify_cumulative`'s grammar refuses it (as it does `KeyDetermined`'s), use the
   `run_windowed_keyed_maintenance` channel and record the exact refusal reason in the phase
   summary rather than dropping the leg.
6. Land the end-to-end fixture test, then reconcile the two spec bullets against what actually
   runs.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-logical maintenance::locality`
- `cargo test -p smelt-runtime --test locality_route2_derived`
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb`
- `cargo test -p smelt-cli --test partition_residue_probes --features duckdb`

## Commit message

`feat(incremental): route 2 derives the key-to-partition dependency, declared FD as fallback`
