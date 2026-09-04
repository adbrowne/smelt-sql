# Phase 6 summary — retire the whole-SQL flat-scan bound floor

**Shipped:**
- `derive_model_bounds` (`crates/smelt-logical/src/analysis/source_bounds.rs`) no longer merges
  every context source's walk verdict with a whole-text flat-scan floor. The per-source
  `merge`-with-flat-scan loop `20e74879` added is gone.
- Added a test-only `derive_model_bounds_inner(sql, ctx, unsupported_fallback: bool)` seam so the
  walk-only derivation could be compared against the fallback-covered one before deleting anything
  (tasks 1–3 of the plan).
- `ReachTransfer::leaf` (same file) now also resolves a ctx key with the "sources." prefix
  stripped — the maintenance-plan subsystem's bare-name convention
  (`resolve_table_ref_source_name` + `.strip_prefix("sources.")`, already used by
  `join_shape::find_join_for_source` and half a dozen other bare-name resolvers) that the walk's
  leaf matcher had never picked up. This was a real, previously-latent gap: with the floor gone,
  every `SourceFacts`-keyed maintenance-plan test that reads `smelt.sources.X` while naming its
  source bare (`"X"`) went from `NotDerivable`/absent to correctly `Bounded` again.
- One narrow leaf classifier survives inside `derive_model_bounds_inner`: a context source the
  walk never finds as a FROM leaf *anywhere* in the tree (regardless of `Unsupported`) gets a
  per-source whole-text fallback. The one live shape is a source passed only as an argument to a
  table-valued function call (`smelt.functions.sessionize(source => smelt.silver.x, …)`) —
  `normalize_table_ref` resolves the call itself to an opaque `Table` leaf, never descending into
  its arguments, so the argument's own `smelt.<path>` is structurally invisible to the walk. This
  is `examples/web_analytics/models/silver/sessions.sql`'s real shape (reads
  `silver.events_deduped` through `smelt.functions.sessionize`).
- `derive_bound_for_source`'s doc comment now names every construct that still normalizes to
  `Unsupported` (table function in FROM, `RECURSIVE` CTE, unnamed/non-SELECT CTE body, non-SELECT
  derived table body, unrecognised set-op operator) — the two shapes phase 1 fixed
  (redundantly-parenthesised derived table, parenthesised join group) are called out as no longer
  surviving.
- `docs/specs/model_properties.md` §Known Divergences gained one line naming the function-call-
  argument gap (tracked to this outcome).
- 22 pre-existing `source_bounds.rs` unit tests and 2 `composed_output_as_clocked_source.rs`
  integration tests had unrealistic fixture SQL (a `FROM` clause using a bare/mismatched table name
  that never matched the `BoundContext` key under test) that only ever passed because the retired
  floor was blind to table identity. Fixed to use `smelt.<ctx-key>` FROM references, matching how
  every real caller builds `ModelInputs`/`BoundContext` — not walk gaps, not test-behavior changes.
- 3 new regression tests in `source_bounds.rs`: the exact `20e74879` motivating shape (now walk-only,
  no floor needed), a table-driven corpus asserting walk-only == floored across every SQL shape this
  module already exercises, and `flat_floor_survives_for_source_hidden_in_a_function_call_argument`
  pinning the one surviving leaf classifier (fails loudly if the walk ever learns to trace into call
  arguments, naming it as the signal to delete the classifier).

**Decisions:**
- Kept the leaf-absent top-up **unconditional** (applies to both the walk-only and
  fallback-covered call), distinct from the `Unsupported`-tree fallback's `unsupported_fallback`
  gate — it is a separate mechanism (a per-source gap, not a whole-tree one) and the walk-only
  comparison tests still hold since both sides of the comparison get it identically.
- Did not attempt to make the walk trace into `smelt_path_call` arguments (would require a new
  `InputItem`/children-slice shape mirroring `expr_scopes` for call arguments) — out of scope for
  a floor-retirement phase; recorded as a spec Known Divergence instead.

**For the next planner:**
- The `ReachTransfer::leaf` "sources." fix is a genuine correctness improvement beyond this
  phase's own scope — it was silently masked by the floor for as long as the floor existed. Worth
  a mention if any downstream audit asks "was this bound derivation ever actually wrong before
  today."
- The function-call-argument gap (new Known Divergences line) is real production surface
  (`examples/web_analytics`'s flagship composed model uses it) but is now correctly covered by the
  named leaf classifier — no further action needed unless the walk gains call-argument tracing,
  at which point `flat_floor_survives_for_source_hidden_in_a_function_call_argument` will fail and
  name the cleanup.
- Phase 7 (delete divergence bullets, `/smelt:validate model_properties`) should NOT delete the new
  bullet this phase added — it documents a live gap, not a closed one. Only MP-03/05/11/13 are
  phase 7's targets.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --quiet` — 0 failures across all 67 test binaries.
- `cargo test -p smelt-logical --test walk_coverage --quiet` — 6 passed.
- `cargo test -p smelt-logical --test expr_scope_inline_equivalence --quiet` — 15 passed.
- `cargo test -p smelt-runtime --test statement_parity --quiet` — 37 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — 78 passed.
- `cargo test -p smelt-cli --test e2e --features duckdb events_deduped_redelivery_equivalence` — 3
  passed (this is the test that caught the function-call-argument gap).
- `rg -n 'derive_bound_for_source\(sql' crates/smelt-logical/src` — 2 whole-`sql` callers survive:
  the task-5 leaf-absent classifier and the `has_unsupported` legacy fallback. No other caller.
