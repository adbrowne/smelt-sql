# Phase 5 summary — `supports_fingerprint_sidecar` becomes the sole owner of the sidecar gate

**Shipped:**
- `docs/specs/multi_backend.md` §"The fingerprint sidecar capability": rewritten to name both
  consequences of a flagless target (external-delta-restriction keeps the widened scan;
  repair-family/model-edge group-grain refuses `UnsupportedOnBackend`), state the flag — never
  the dialect — is what every consumer reads, and note the DDL owner is still DuckDB-shaped.
- `crates/smelt-runtime/src/maintenance_driver.rs`: `resolve_live_per_group_recompute_cell` and
  `resolve_live_key_addressed_model_edge_cell` gained a `supports_fingerprint_sidecar: bool`
  parameter, replacing their `dialect != SqlDialect::DuckDB` gates; the four async sidecar entry
  points (`diff_fingerprint_sidecar_changed_keys`, `refresh_fingerprint_sidecar`,
  `diff_repair_group_sidecar_changed_keys`, `refresh_repair_group_sidecar`) now gate on
  `backend.capabilities().supports_fingerprint_sidecar` instead of `backend.dialect() !=
  SqlDialect::DuckDB`. Six doc comments updated to describe the capability gate, not the dialect.
- `crates/smelt-runtime/src/execute.rs`: both resolver call sites now pass
  `backend.capabilities().supports_fingerprint_sidecar` alongside the existing `backend.dialect()`.
- Tests: `repair_lowering.rs` gained
  `snapshot_discovery_refuses_without_the_sidecar_capability` (renamed, now passes
  `BackendCapabilities::spark_delta().supports_fingerprint_sidecar`) and
  `snapshot_discovery_admits_when_the_capability_is_declared` (non-DuckDB dialect + flag `true`
  resolves `SidecarDiff`). `key_addressed_model_edge_lowering.rs` gained the same pair
  (`key_addressed_edge_refuses_without_the_sidecar_capability` /
  `key_addressed_edge_admits_when_the_capability_is_declared`). `fingerprint_sidecar.rs` gained
  `sidecar_entry_points_refuse_without_the_capability` with a new `SidecarLessBackend` stub
  (DuckDB dialect, `supports_fingerprint_sidecar: false`) proving all four entry points refuse.

**Decisions:**
- Left the four `read_observed_delta`/ledger/merge dialect gates (sites 526, 1956, 3486, 3609)
  untouched — they gate a different capability question, not the sidecar.
- The `incremental_models.md` Known Divergences bullet is left as-is; phase 6 owns deleting/
  rewriting closed bullets across the whole outcome.

**For the next planner:**
- Phase 6 should update the `incremental_models.md` "Conditional-maintenance gaps" bullet to
  reflect that the flag, not the dialect, now gates every sidecar consumer, and close out the
  outcome's remaining TODO/spec cleanup across all five gaps.
- Phase 3's blocked generative-pool clause (see `## Blocked` in `outcome.md`) still needs a human
  call on options (a)/(b)/(c) before it can resume — unaffected by this phase's work.

**Gates:**
- `cargo test -p smelt-runtime --test repair_lowering --test key_addressed_model_edge_lowering --test fingerprint_sidecar --test statement_parity --test observed_delta` — 100 tests, all green.
- `cargo test -p smelt-dialect --test capability_conformance` — green (matrix unchanged).
- `cargo test -p smelt-cli --test maintenance_conformance` — 79 tests, all green.
- `cargo fmt --all -- --check` — clean.
- `bash .claude/scripts/clippy-gate.sh` (both feature sets) — zero warnings.
- `cargo test -p smelt-runtime -p smelt-dialect -p smelt-logical --quiet` — all green (one
  transient `execute_parity` failure traced to concurrent `cargo test` processes from an
  unrelated background verify-phase.sh run contending over shared DuckDB temp files; re-run
  alone was green).
- `cargo test -p smelt-cli --test example_diagnostics --quiet` — 120 passed, 1 ignored.
- Full-workspace `verify-phase.sh` was launched but exceeded the 10-minute foreground budget and
  was moved to background by the tool; the equivalent scoped checks above were run directly and
  are all green.
