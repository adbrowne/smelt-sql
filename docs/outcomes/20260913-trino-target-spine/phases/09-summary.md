# Phase 9 summary — end-to-end on the real pipeline

**Shipped:**
- `dialect_and_capabilities` (`crates/smelt-runtime/src/compile.rs`) is infallible again: `TrinoCapabilitiesUnmeasured` deleted, Trino now returns `(SqlDialect::Trino, BackendCapabilities::trino_iceberg())`.
- `examples/trino_spine/` — `dev` (DuckDB) + `trino` targets, `spine_table.sql` (table) and `spine_view.sql` (view) over literal rows, no seed/source needed. Listed in `examples/README.md`.
- `crates/smelt-cli/tests/trino_smoke.rs` — live-gated: materializes both models on a real Trino/Iceberg tier via `smelt run`, reads rows back, asserts a run report + manifest naming both models `Success`, plus the vacuous-pass guard (`SMELT_TRINO_URL` unset ⇒ `trino_env()` is `None`, never a fabricated `Some`).
- `common::TargetKind::Trino { schema }` is now real: `targets_to_run_with_trino`, `fetch_rows`/`execute_sql_on` and `targets_yaml` grow real Trino arms; every other TargetKind match across the parity suites (merge/schema-evolution/incremental/pipe/lowering/dual-target/source-seed) grows a loud `unreachable!` arm naming why (dead today; `targets_to_run` stays Trino-free by design).
- `materialization_parity.rs`'s `view_and_table_materialize_consistently_on_both` now runs a live Trino leg via `targets_to_run_with_trino`.
- `example_diagnostics::trino_spine_no_diagnostics` and `smelt-lsp`'s `example_workspaces::trino_spine` — both clean offline.
- `docs/specs/multi_backend.md` §Known Divergences gains the array-decode-to-Arrow gap line (phase 8 discovery, no tracking plan yet).

**Decisions:**
- The plan's test 2 (`trino_compile_refuses_qualify`) rested on a wrong premise: `QUALIFY` with `supports_qualify = false` is *rewritten* to a subquery on every dialect that sets it false (Spark, BigQuery, now Trino), never refused — confirmed against `smelt-dialect/tests/snapshots.rs`'s existing `qualify_rewrite_spark`. Swapped for `trino_compile_refuses_materialized_view_without_native_ivm`, which exercises the one capability flag (`supports_native_ivm = false`) that actually produces a hard compile-time refusal, mirroring the existing DuckDB test.
- Found and fixed two real bugs blocking the walking skeleton, both squarely full-refresh-path bugs, not `trino-incremental`'s incremental-maintenance scope:
  1. The full-refresh probe call sites in `execute/project/mod.rs` eagerly resolved a `MaintenanceDialect` (`smelt_backend::maintenance_dialect(...)?`) before checking whether the model declared *any* probe at all — so a plain model with zero declared probes still hard-failed on Trino (no `MaintenanceDialect::Trino` variant exists, deliberately, per `20260913-trino-incremental`'s scope). Fixed by adding dialect-free predicates (`model_probes::any_declared_probe`, `source_probes::any_append_only_posture_probe`) and only resolving the dialect when one of them is true.
  2. Nothing ever called `Backend::ensure_schema` for Trino — `create_backend`'s Trino arm never did (deliberately: `TrinoBackend::new` must stay network-call-free, pinned by `crates/smelt-backends/tests/create_backend.rs::factory_constructs_a_trino_backend_from_a_target`), and Spark/BigQuery cover it inside their own (already-networked) constructors. Fixed by adding a generic `requires_schema_init` gate in `execute_project` right after backend construction — runs for every backend with the flag set (DuckDB/Spark/BigQuery/Trino), calling an idempotent `CREATE SCHEMA IF NOT EXISTS`-shaped `ensure_schema`; redundant-but-harmless for backends whose constructor already does it.
- Large-file baseline (`compile.rs`, `execute/project/mod.rs`) updated via `--update` — modest, in-scope growth from new tests and the two fixes above.

**For the next planner:**
- Phase 8's array-decode gap (Trino `array(...)` result columns don't decode to Arrow) is now recorded in the spec but has no owning plan — worth a line item somewhere in the Trino programme.
- The double `ensure_schema` call (constructor + `execute_project`'s new gate) for Spark/BigQuery/DuckDB is a minor, accepted inefficiency — one idempotent no-op statement per run. Not worth chasing unless it shows up as a real cost.
- Phases 10 (CI wiring) and 11 (close-out: docs-site, hardening baseline, divergences) are next in this outcome.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (`SMELT_TRINO_URL` unset; every live leg printed its skip reason).
- `bash scripts/trino-up.sh && source scripts/trino-env.sh` then `cargo test -p smelt-cli --test trino_smoke --test materialization_parity --test seed_parity` — all live legs ran, 0 skipped, all green.
- `cargo test -p smelt-backend-trino` (live) — green.
- `cargo test -p smelt-lsp --test example_workspaces trino_spine` — green.
- `cargo test -p smelt-runtime --test dialect_seam` — green.
- `bash scripts/trino-down.sh` — clean teardown.
