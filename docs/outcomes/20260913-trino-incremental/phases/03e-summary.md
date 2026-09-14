# Phase 3e summary — live-Trino test isolation

## Shipped

- `trino_schema` (`crates/smelt-cli/tests/common/mod.rs`) and `unique_schema`
  (`crates/smelt-backend-trino/tests/common/mod.rs`, now `pub`) rewritten
  over one naming rule: a process-local `AtomicU64` counter plus a per-call
  entropy suffix (`rand::random()` for `trino_schema`; nanosecond `SystemTime`
  for `unique_schema`, since `smelt-backend-trino` has no `rand` dependency)
  — guarantees no two calls in one process ever collide, closing the root
  cause `capability_probes.rs` hit (14 concurrent tests drawing the same
  `"cap"` suffix within one nanosecond of each other).
- New offline binary `crates/smelt-backend-trino/tests/schema_isolation.rs`:
  1000-call uniqueness and legal-identifier checks for `unique_schema`. Kept
  free of every live-gate marker so `trino_ci_wiring.rs`'s derived census
  classifies it offline.
- `trino_ci_wiring.rs` gained two tests: `trino_schema_names_are_unique_within_a_process`
  (1000-call uniqueness + legality for `trino_schema`, duplicating
  `schema_isolation.rs`'s assertion helper verbatim per the plan) and
  `every_live_trino_test_schema_name_comes_from_the_shared_helper` (the
  anti-regression gate: no live-gated binary may define its own `fn
  unique_schema(`/`fn trino_schema(`).
- That anti-regression gate immediately caught two **existing** private
  duplicates it was written to prevent: `crates/smelt-backend-trino/tests/backend_live.rs`
  (16 live tests, one un-suffixed, un-counted `unique_schema()`) and
  `tests/staged_relation_lifecycle.rs` (same pattern). Both rewritten to
  `mod common;` + the shared `common::live_env_or_skip`/`common::unique_schema`/
  `common::drop_schema`, dropping their private copies entirely.
- `TRINO_ENV_GUARD` widened in `trino_state_residency.rs`, `trino_lock_versioning.rs`,
  and `trino_smoke.rs` (the two lock_versioning/smoke files were the
  "audit" task's finds): each now exposes `resolve_trino_target_block(schema)`,
  which holds the guard across both the `trino_env().is_some()` check and the
  `trino_target_block` read, and every staging function (`stage_residency_project`,
  `stage_lock_project`, `stage_trino_spine`) takes the already-resolved
  target-block string instead of re-deriving it unguarded.
- Stale header claim ("Trino has no `MaintenanceDialect`") was already fixed
  by a prior phase; verified, not re-touched.

## Decisions

- Used a random `u64` (via the crate's existing `rand` dependency) for
  `trino_schema`'s entropy but a nanosecond timestamp for `unique_schema`,
  rather than adding `rand` to `smelt-backend-trino` — the counter alone
  already guarantees intra-process uniqueness; the entropy suffix only needs
  to be *good enough* to avoid inter-process collisions, and nanoseconds
  suffice for that without a new dependency.
- `assert_legal_trino_identifier`/`assert_legal_trino_identifier` is
  duplicated verbatim between `trino_ci_wiring.rs` (smelt-cli) and
  `schema_isolation.rs` (smelt-backend-trino) rather than shared through a
  library, since test code cannot be imported across crate boundaries in
  Rust; the plan's "one shared assertion" is satisfied by the two copies
  being textually identical.

## For the next planner

- **Real regression caught by the plan's own acceptance test**: making
  `trino_schema` non-deterministic broke `materialization_parity.rs`'s
  `view_and_table_materialize_consistently_on_both`, which called
  `trino_schema(BQ_LABEL)` once to build the `smelt.yml` target block and
  relied on `targets_to_run_with_trino(BQ_LABEL)` calling `trino_schema`
  *again* with the same label to independently derive the *same* schema
  name — previously true by coincidence (same label+pid ⇒ same string), now
  false. Fixed by resolving `targets_to_run_with_trino(BQ_LABEL)` once and
  threading its `TargetKind::Trino { schema }` into `stage_mat_workspace`
  instead of re-deriving it. Audited every other `targets_to_run_with_trino`
  call site (only `trino_ci_wiring.rs`'s string-literal census reference) —
  no other file has this pattern, but a future caller building a project yml
  from one `trino_schema()`/`unique_schema()` call and a separately-resolved
  target list should resolve once and thread the value, not call twice.
- 3f–3h and 4–8 (windowed-keyed pushdown typing, `KeyedFold` plan-time
  availability, the keyed-fold `statement_parity` leg, the emulated
  delete-and-insert window, the merge-less conditional write, the degraded
  routes) are unaffected by this phase and remain `pending`/`blocked` as
  before — this phase touched only test harness code, no production path.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both
  feature sets, workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test trino_ci_wiring` — 8/8 passed.
- `cargo test -p smelt-backend-trino --test schema_isolation` — 2/2 passed.
- Live tier (`bash scripts/trino-up.sh` / `source scripts/trino-env.sh`), **3
  consecutive green runs at default parallelism** (no `--test-threads=1`) of:
  `cargo test -p smelt-backend-trino` (44 tests across 5 binaries) and
  `cargo test -p smelt-cli --test trino_ddl_live --test trino_state_residency
  --test trino_lock_versioning --test trino_incremental_families --test
  trino_smoke` (16 tests) and `cargo test -p smelt-cli --test seed_parity
  --test materialization_parity --features duckdb` (4 tests) — all 3 runs
  100% pass, 0 flakes (`bash scripts/trino-down.sh` after).
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity`
  — 41/41 passed (no collateral damage).

Commit: `test(trino): give every live-tier test a guaranteed-unique schema and close the residency env-guard hole`
