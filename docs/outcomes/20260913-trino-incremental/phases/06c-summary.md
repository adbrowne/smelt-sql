# Phase 6c summary — the repair family's own sidecar requirement

**Shipped:**
- `smelt_logical::maintenance::repair::is_repair_admitted` — the single-owner positive
  discriminator for a repair-admitted `PerGroupRecompute` cell (`repair.rs`).
- `required_state_structure`'s `PerGroupRecompute` arm now requires `FingerprintSidecar` for a
  repair-admitted cell, not only a key-addressed one (`availability/state_structure.rs`).
- `resolve_availability` clears `scans` when the downgraded `original` technique was
  `PerGroupRecompute`, keeping `has_repair_family_lowering`'s discriminator exact
  (`availability/mod.rs`).
- `smelt_runtime::maintenance_driver::resolve_repair_state_downgrade`, mirroring
  `resolve_keyed_fold_state_downgrade`, folded into `execute/project/mod.rs`'s existing
  whole-target-rebuild arm via `.or(...)` (one dispatch route serves both downgrade shapes).
- `docs/specs/state.md` §"The degradation contract" and `docs/specs/multi_backend.md`'s Trino
  table updated: the `PerGroupRecompute`/no-`key_scope` row split into repair-admitted
  (downgraded) and downgrade-reached (reachable) rows.
- Tests: 5 new `smelt-logical` unit tests, 1 new `smelt-runtime` seam test, 1 new
  `smelt-runtime` DuckDB-oracle offline equivalence test, 1 new `smelt-cli` explain test, 1 new
  spec-freshness test, plus `degraded_routes.rs`'s test 1 rewritten to assert the downgrade.

**Decisions:**
- The requirement is derived from the cell's own shape (`is_repair_admitted`), not a new
  `PlanCell` field — matches `has_repair_family_lowering`'s existing discriminator vocabulary.
- BigQuery's landing-state change (repair cells there now downgrade instead of hard-refusing,
  since BigQuery also lacks the sidecar) is recorded but escalated to
  `20260906-bigquery-correctness`, not absorbed here.
- The two downgrade-producing resolvers (`resolve_keyed_fold_state_downgrade`,
  `resolve_repair_state_downgrade`) are combined with `.or(...)` into one dispatch arm rather than
  a second branch, per the plan's explicit instruction.

**For the next planner:**
- **Test 9 (`per_group_recompute_matches_full_refresh_on_trino`) is still not achieved live** —
  for a SECOND, unrelated, newly-discovered reason distinct from the sidecar gap this phase fixed.
  The fixture's Form B band needs an `INTERVAL` literal to discharge obligation 4 (bounded
  per-group read), but no spelling works end to end on live Trino: `INTERVAL '3 days'` (quoted,
  DuckDB-native) parses in `smelt-parser` but Trino's engine rejects it
  (`TypeNotFoundException: Unknown type: interval`); `INTERVAL '3' DAY` fails to even parse in
  `smelt-parser`; `INTERVAL 3 DAY` parses but `source_bounds::parse_quoted_interval` (the Form B
  classifier) doesn't recognise the bare-numeric spelling, so obligation 4 fails closed before the
  sidecar question is ever reached. Full writeup and candidate fixes in outcome.md's Blocked log,
  2026-09-15 "phase 6c". This is NOT a sidecar-fix defect — the fix is proven 5 independent ways
  without needing this specific live path (see Blocked log).
- Test 9 is kept `#[allow(dead_code)]` again in `degraded_routes.rs`, with an updated doc comment.
- Row 6c is `blocked` (matching phase 6's own precedent for the same unresolved live test),
  though everything else the plan asked for (spec, fix, 9 of 10 listed tests, live explain proof)
  is done and green.
- `.claude/large-file-baseline.txt` was updated (`--update`) for two files that grew from this
  phase's additions: `crates/smelt-runtime/src/execute/project/mod.rs` (5251→5278) and
  `crates/smelt-runtime/tests/repair_lowering.rs` (1886→1992). Sign-off: growth is test/doc-comment
  volume from this phase's own additions, not a design smell — no split warranted.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — fmt/clippy/shellcheck PASS; full `cargo test` initially
  FAILED on the large-file ratchet (fixed via baseline update above and reformatted with
  `cargo fmt --all`), then re-run clean (exit 0); `example_diagnostics` PASS (129 passed, 1 ignored).
- `cargo test -p smelt-logical --test maintenance_availability --test walk_coverage` — PASS (42 + 14).
- `cargo test -p smelt-runtime --test availability_seam --test repair_lowering --test execute_parity --test statement_parity` — PASS (11 + 24 + 4 + 46).
- `cargo test -p smelt-cli --test explain_maintenance --test trino_incremental_spec_freshness` — PASS (57 + 7).
- Live tier (`scripts/trino-up.sh` / `source scripts/trino-env.sh`):
  `cargo test -p smelt-cli --test trino_incremental_families -- --test-threads=1` — 17/18 PASS
  (`per_group_recompute_matches_full_refresh_on_trino` parked, see above); `scripts/trino-down.sh` run.
