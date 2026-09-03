# Phase 15 summary — Observed-delta consumption, read side

**Shipped:**
- `smelt run --since-upstream` now reads the recorded `_smelt_observed_delta` table live: a new
  `crates/smelt-runtime/src/propagation.rs::load_observed_delta_lookup` loads one row per
  model-address delta origin (skipping raw-source origins) and feeds
  `plan_since_upstream_with_observed_deltas` directly from `commands/run.rs`'s
  `run_since_upstream`. `plan_since_upstream` (empty-lookup wrapper) is now consumed only by the
  testkit/conformance harness, not the CLI.
- New shared decoder `maintenance_driver::read_observed_delta` decodes both `changed_keys` and
  `partitions` `VARCHAR[]` columns in one read; `read_observed_delta_changed_keys` is now a thin
  wrapper over it (no behaviour change, same `None`/`Some(&[])` distinction).
- Backward resolution (`smelt build --include-upstreams` / `resolve_build_plan`) is documented and
  pinned as a stated non-goal for observed-delta consumption — it answers an existence question a
  change record can't soundly narrow (spec paragraph + doc comment + regression test).
- `docs/specs/incremental_models.md`: new paragraph under "Backward resolution — what must exist"
  (the non-goal), extended "Observed deltas on model edges" paragraph (live read behaviour,
  DuckDB-scoped), and the "Observed-delta consumption is partial" Known Divergences bullet
  narrowed to only the write-side clauses phase 16 owns.
- `docs-site/docs/guide/incremental-models.md`: replaced the stale "`explain` never opens a
  backend connection ... reading the live delta is `--since-upstream`'s job" closer with a "What a
  recorded delta narrows" paragraph stating the absent/empty/non-empty behaviour and the backward-
  resolution non-goal.

**Decisions:**
- Backward resolution does NOT consume observed deltas (already recorded in outcome.md's decision
  log 2026-09-02, phase 15, before this implementation step) — landed in the spec verbatim.
- `load_observed_delta_lookup`'s eligibility filter is "origin name is a known model" (the CLI's
  full model set), not "origin is locality-admitted" — the narrower locality-admitted filter
  already lives inside `plan_since_upstream_with_observed_deltas` itself (via `key_locality_slice`),
  so over-populating the lookup for a model-address delta that isn't locality-admitted is harmless
  (never consulted) and keeps the loader simple/pure with respect to locality derivation.

**For the next planner:**
- **Real bug found and fixed in this phase, not a residue**: the CLI's original wiring created the
  observed-delta lookup backend connection and kept it alive for the entire `run_since_upstream`
  function body (including the run loop below, which opens its *own* backend connection to the
  same DuckDB file). Two independent live connections to one DuckDB file from the same process
  corrupted/lost writes non-deterministically — 3 of 12 `since_upstream.rs` tests failed with
  "Table with name gold does not exist" on the first run, including two **pre-existing, unmodified**
  tests (`model_address_landed_delta_propagates`, `composed_model_address_landed_delta_propagates`).
  Fixed with an explicit `drop(lookup_backend)` right after the lookup load, before the run loop.
  Nothing else in the CLI currently opens two live connections to the same target this way, but if
  a future phase adds another pre-run backend probe (e.g. for `--auto` staleness), it must drop its
  connection before entering the execute loop, or reuse the SAME connection somehow.
- Phase 16 (write side: keyed-fold + staged-candidate recording, settle-bound × observed-delta
  "delta empty" leg) is next and unblocked by this phase — `read_observed_delta` is the shared
  decode site it should read through if it needs the full `ObservedDelta`, not just changed keys.
- No other residue surfaced; `resolve_build_plan` was already fully pure (never touched the
  backend) before this phase, so pinning it as a non-goal required no code change, only docs/tests.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-runtime --test since_upstream_propagation --test observed_delta` — 27
  passed.
- `cargo test -p smelt-cli --features duckdb --test since_upstream --test include_upstreams` — 16
  passed.
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance` — 74 passed (DAG
  families still call `plan_since_upstream`; empty-lookup semantics preserved).
- `/smelt:validate incremental_models` spot-check on the edited sections: manually re-read
  "Backward resolution — what must exist", "Observed deltas on model edges", and the rewritten
  Known Divergences bullet against the shipped code — consistent (not run as the full slash
  command; that's phase 24's job).
