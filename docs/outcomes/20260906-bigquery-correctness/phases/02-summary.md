# Phase 2 summary — the rest of the dialect-blind fingerprint SQL

**Shipped:**
- `key_expr_for_columns` (`crates/smelt-logical/src/maintenance/emit/fingerprint.rs`)
  now takes `dialect: MaintenanceDialect` and casts to that dialect's own unsized
  string type (`probe_dialect_string_type`) instead of a hardcoded `VARCHAR`, for
  both the single-column literal branch and the multi-column
  `concat_varchar_expr_typed` branch.
- `emit_repair_group_digest_select` has a real per-dialect body: DuckDB
  `bit_xor(hash(sha256(...)))` (unchanged), BigQuery
  `BIT_XOR(FARM_FINGERPRINT(...))`, Spark `bit_xor(xxhash64(...))`, all cast to
  the dialect's own string type.
- `emit_key_addressed_affected_keys_select` threads `dialect` into both its key
  expressions instead of ignoring it.
- Deleted `concat_varchar_expr` (the untyped `VARCHAR`-only wrapper) — no
  remaining callers once `key_expr_for_columns` and
  `emit_repair_group_digest_select` moved to `concat_varchar_expr_typed`.
- Threaded `MaintenanceDialect` through every downstream call site:
  `emit/recompute.rs::emit_per_group_recompute`,
  `smelt-runtime::maintenance_driver::sidecar` (one inline call already had
  `dialect` in scope), and three `smelt-runtime` repair-family helpers
  (`repair_affected_keys_select`, `repair_candidate_select`,
  `repair_slice_predicate` in `maintenance_driver/repair/execute.rs`), each
  gaining a `dialect: MaintenanceDialect` parameter passed from
  `smelt_backend::maintenance_dialect(backend.dialect())` (or an existing local
  `dialect`) at every call site (`diagnostics/preview.rs`,
  `execute/project/mod.rs`, `maintenance_driver/key_addressed/execute.rs`).
- 8 new tests: 7 in `fingerprint.rs`'s `mod tests` pinning the per-dialect SQL
  shapes, plus `sidecar_capability_is_declared_only_where_the_digest_sql_is_verified`
  in `crates/smelt-runtime/tests/fingerprint_sidecar.rs` — the loud-refusal gate
  asserting `supports_fingerprint_sidecar` is `true` for DuckDB only.
- Updated stale doc comments that described the pre-fix DuckDB-only `delta_key`
  shape as a "phase 2 residue" pointer or a permanent design choice.

**Decisions:**
- Kept the runtime `supports_fingerprint_sidecar` capability gate at DuckDB-only
  (per test 8) — this phase proves the emitted SQL is well-formed per dialect,
  not that it's live-verified against a real BigQuery/Spark warehouse. Flipping
  the flag is future work gated on that backend's own value-leg sweep.
- `key_expr_for_columns`'s literal-value contract for a single-column key
  (surfaced downstream as a real predicate value) is preserved unchanged — only
  its cast type became dialect-dependent, matching the plan's design note.

**For the next planner:**
- Phase 3 (harvest from the spine's findings handoff at
  `docs/handoffs/2026-09-08-github-activity-findings.md`) is next; this phase's
  work was scoped narrowly to the two known-dialect-blind emitters named in the
  outcome and did not touch anything from the handoff.
- No live-BigQuery leg was needed or run — every assertion here is over emitted
  SQL text and the capability matrix, matching the plan's verification section.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets,
  full workspace `cargo test`, `example_diagnostics`)
- `cargo test -p smelt-logical --quiet` — PASS
- `cargo test -p smelt-runtime --test statement_parity --quiet` — PASS (41 tests)
- `cargo test -p smelt-runtime --test fingerprint_sidecar --test repair_lowering --test key_addressed_model_edge_lowering --quiet` — PASS (18+13+20 tests)
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — PASS (101 tests, DuckDB output unchanged)
- `.claude/large-file-baseline.txt` bumped (with sign-off note) for mechanical
  growth from the new dialect parameter threading and the 8 new tests.
