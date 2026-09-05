# Phase 3 summary — route 2's derived key-derived-expression sub-route

## Shipped

- `crates/smelt-logical/src/analysis/key_derived.rs` (new): the leaf classifier
  `key_derived_partition_verdict(sql, unique_key, partition_column) -> KeyDerivedVerdict`
  (`Derived`/`NotDerived(reason)`) — CST-based over `analysis::analyze_select`, no raw-text
  scan. Admits a deterministic expression (`CAST`, `MIN`/`MAX`, …) whose column references are
  entirely `unique_key` columns and which calls no run-/row-nondeterministic function. 6 unit
  tests.
- `crates/smelt-logical/src/maintenance/locality.rs`: route 2 now consults the derived
  sub-route **before** the extremal-fold check; on `Derived` it returns `LocalitySlice::
  DeltaValues` immediately, outranking the extremal refusal exactly when every column reference
  is a key column. The `NotDerived` reason is folded into the extremal-fold and declared-FD-
  refused refusal messages (not into the generic three-route fallthrough, to avoid pre-empting
  it). 5 new named tests plus the module doc comment updated.
- `docs/specs/incremental_shapes.md` §"Key temporal locality": route 2's sentence now states
  the derived-then-declared sub-route order and the outranking condition; the "admits only a
  declared FD" Known Divergence bullet is deleted, and the "no runnable end-to-end route-2
  fixture" clause is removed (a fixture now exists).
- `crates/smelt-maintenance-testkit/src/recipe.rs`, `render.rs`: new `ComposedRoute::KeyDerived`
  recipe (`unique_key: [id, d]`, `pdate = CAST(d AS DATE)`, no declared FD), wired into
  `arb_composed_route` (admission-rate sampler now samples 4 routes) and
  `render_composed_model_body`.
- `crates/smelt-maintenance-testkit/src/families/gate_composed.rs` and
  `crates/smelt-cli/tests/maintenance_conformance/gate.rs`: admission-match arm for the new
  route; the CLI gate additionally gained a full direct-driver equivalence leg
  (`composed_derived_classification`/`_delta_sql`/`_oracle_sql`/`_suppression`,
  `drive_composed_derived_and_assert`, per-slice + whole-table equivalence assertions) wired
  into `composed_keyed_pool_upholds_equivalence`'s route match.
- `crates/smelt-runtime/tests/locality_route2_derived.rs` (new): the plan's end-to-end fixture
  — stages a `KeyDerived` recipe with no `functional_dependencies:`, confirms real-Salsa
  admission via `LocalitySlice::DeltaValues`, then drives two windows through
  `run_windowed_keyed_maintenance` directly against a real DuckDB backend and asserts equality
  with the full-refresh oracle after each.

## Decisions

- Derived sub-route consulted before the extremal-fold refusal (spec-mandated): a `MAX`/`MIN`
  over a key column is the key itself, so it must not be refused as an extremal fold. Confirmed
  by test `route2_derived_outranks_the_extremal_refusal`.
- `KeyDetermined`'s existing recipe (`unique_key: [id]`, `pdate = CAST(d AS DATE)`) is genuinely
  the **declared** sub-route's flagship, not the derived one — `d` is not part of its own
  `unique_key`, so the derived classifier can't decide it and it still needs the FD. `KeyDerived`
  needed a distinct recipe (`unique_key: [id, d]`) to actually exercise the derived path.
- `KeyDerived`'s body (`CAST(d AS DATE) AS pdate` alongside `GROUP BY id, d`) is still refused by
  `classify_cumulative`'s runtime grammar (not a literal GROUP BY text, not COALESCE) —
  independent of locality admission, same pre-existing gap `KeyDetermined` already hits. Both
  the conformance leg and the end-to-end fixture drive it via `run_windowed_keyed_maintenance`
  directly, per the plan's own contingency instruction.
- `KeyDerived` excluded from `gate_composed.rs`'s (target-parametrized) equivalence drive for
  the same `Grade::Additive`-ledger-gap reason `KeyEmbedded`/`KeyDetermined` already are — only
  admission sampling, not equivalence, runs there for it.

## For the next planner

- Not done: phase 4 (`KeyedRecurrenceDeclarationMismatch`, order-independent key-set comparison)
  is still `pending` and unblocked by this phase.
- Out of scope, not touched: Route 2's `IN (SELECT DISTINCT …)` DuckDB MERGE-binder limitation
  (both `KeyDetermined` and `KeyDerived` drive with `slice: None`, same as before) — still a
  live gap in `incremental_shapes.md` §"Locality machinery gaps".
- No new gaps surfaced beyond what the outcome already tracks.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test walk_coverage` — 8 passed.
- `cargo test -p smelt-logical --lib maintenance::locality` / `key_derived` — 37 / 6 passed.
- `cargo test -p smelt-runtime --test locality_route2_derived` — 1 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb composed` — 4 passed.
- `cargo test -p smelt-cli --test partition_residue_probes --features duckdb` — 4 passed.
