# Phase 6 summary — live observed-delta consumption

**Shipped:**
- `maintenance_driver::read_observed_delta` — reads one `(model, window)` row back off
  `_smelt_observed_delta`, decoding both `changed_keys` and `partitions` into an `ObservedDelta`
  (`None` = never recorded, `Some` with both empty = present-and-empty). `read_observed_delta_changed_keys`
  is now a delegating wrapper.
- `propagation::observed_delta_keys_to_read` (pure) — the exact `ObservedDeltaKey` list a live
  resolver should read, derived from the same `derive_clamp_and_locality` `key_locality_slice`
  the planner itself consults.
- New `smelt-runtime::propagation_live` module — `resolve_observed_delta_lookup(backend, schema,
  keys)` reads every key live and assembles an `ObservedDeltaLookup`, omitting absent keys
  (never fabricating present-and-empty for them).
- `smelt-cli/src/commands/run.rs::run_since_upstream` now creates the real backend, resolves the
  live lookup, and calls `plan_since_upstream_with_observed_deltas` instead of the always-empty
  `plan_since_upstream`. Runs under `--dry-run` too; a backend-creation failure is a named error.
- `docs/specs/incremental_models.md` §Known Divergences narrowed: dropped "doesn't read the
  recorded delta table live" and the settle-bound × observed-delta "no live delta-empty leg"
  clause from "Observed-delta consumption is partial"; dropped the observed-delta clause from
  the scheduler-currency bullet's live-resolution parenthetical (the key-value half stays,
  tracked for row 7).

**Decisions:**
- Live resolver lives in a new `propagation_live.rs` file rather than inline in `propagation.rs`
  — keeps `propagation.rs` pure (no backend/async), matching that module's own doc comment.
- The CLI creates a dedicated backend connection just for the observed-delta read (dropped before
  the run's own backend is created later in the same function) rather than threading one shared
  connection through — simplest correct shape; DuckDB tolerates sequential (non-concurrent)
  opens of the same file.

**For the next planner:**
- Row 7 (live keyed-seed resolution) is next — `observed_delta_keys_to_read`'s "ask the planner
  module" pattern is a precedent worth reusing for whatever asks the sidecar which origins to
  diff.
- The CLI test records its present-and-empty row directly via `generate_observed_delta_upsert_sql`
  against the target DuckDB file (no write family in this fixture's technique records one yet) —
  matches the plan's own fallback instruction; the write-family recording gap stays in the
  narrowed divergence bullet's surviving residue ("the keyed-fold and staged-candidate write
  families record nothing").
- Nothing else deferred out of this phase's scope.

**Gates:**
- `cargo test -p smelt-runtime --test since_upstream_propagation --test observed_delta` — 24 + 6 passed
- `cargo test -p smelt-cli --test since_upstream` — 10 passed
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity` — 4 + 23 passed
- `cargo test -p smelt-cli --test maintenance_conformance` — 76 passed
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md` — no matches
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, full workspace test, example_diagnostics)
