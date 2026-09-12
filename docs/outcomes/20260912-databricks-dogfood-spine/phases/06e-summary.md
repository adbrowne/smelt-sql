# Phase 6e summary — `epoch_us` registered; live run advances to 11/1/4 with a new, unrelated blocker

## Shipped

- `crates/smelt-types/src/functions/{mod,name,category}.rs`: `SqlFunction::EpochUs`
  (`"EPOCH_US"`, `FunctionCategory::DateTime`).
- `crates/smelt-types/src/signatures/builtins/extended_temporal.rs`: the `EPOCH_US` `Signature`
  — `(Timestamp) -> BigInt`, DuckDB keeps the native name, SparkSQL/Databricks emits
  `unix_micros({0})`, BigQuery emits `UNIX_MICROS({0})`.
- `crates/smelt-db/src/type_inference/function_call/registry.rs` +
  `crates/smelt-db/src/type_inference/function_call/legacy.rs`: `EPOCH_US` added to
  `REGISTRY_MIGRATED` (registry-first inference) plus a mirroring legacy match arm for
  `SqlFunction` exhaustiveness.
- Tests: `epoch_us_is_registry_backed` (smelt-types), `epoch_us_infers_bigint` (smelt-db
  registry-inference), `epoch_us_emits_unix_micros_on_spark` (smelt-dialect template-emission,
  DuckDB/Spark/BigQuery), `actor_sessions_compiles_without_epoch_us` (smelt-cli, `--dry-run`
  against the databricks target).
- `docs/reference/dialect-coverage.md` regenerated (`EPOCH_US` row added).
- `.claude/unknown-census.toml`: fixed a stale line number the `legacy.rs` edit shifted
  (Greatest/Least Unknown site, 430 → 435).
- `.claude/large-file-baseline.txt`: `smelt-types/src/signatures/tests.rs` 2467 → 2484 (the new
  test's line count on an already-tracked oversized file).

## Decisions

- Registered `EPOCH_US` registry-first (added to `REGISTRY_MIGRATED`) rather than leaving it
  legacy-only, consistent with `AGE`/`TO_SECONDS`/`DATE_ADD`/`DATE_SUB` — a fixed
  `(Timestamp) -> BigInt` shape has no argument-dependent typing, so it qualifies. The legacy
  match still carries a mirroring arm purely for `SqlFunction` exhaustiveness (the registry path
  always wins at runtime; same pattern the existing temporal entries already use).
- 2026-09-12: updated `.claude/large-file-baseline.txt` rather than splitting
  `signatures/tests.rs` — it's already a registered, deliberately-large file (2467 lines before
  this phase); a 17-line test addition is exactly the in-scope growth the ratchet's own
  instructions say to raise the baseline for.

## For the next planner

**The live full refresh (run `20260912-121546-c86d9f`) is 11 success / 1 failed / 4 skipped —
`epoch_us` is confirmed fixed, but a new, unrelated blocker now sits at the same failure point:**

```
silver.actor_sessions: Execution failed for 'spark sql': AnalysisException:
Cannot specify window frame for lag function.
```

`sessionize.sql`'s `LAG(...)` call carries an explicit window frame clause that Spark refuses —
Spark's `lag`/`lead` (unlike DuckDB's) must run over the implicit default frame, no `ROWS
BETWEEN`/`RANGE BETWEEN` allowed. This is squarely outside phase 6e's own boundary (the
`epoch_us` registry entry) — same shape as 6c → 6d → 6e's own chain, each phase's live run
uncovering the next construct at the identical failure point once the prior one clears. Recorded,
not fixed, per the outcome's boundary. The same three dependents still skip. Row 7 (or a new
inserted row ahead of it, per the 6e reshape precedent) needs to either strip/guard the window
frame on Spark for `LAG`/`LEAD` (a `BackendCapabilities`/emission-shaped fix, since DuckDB's
frame is legitimate there) or exclude `actor_sessions` from criteria 6/7/8 — the former keeps the
model set whole and is the smaller, non-structural change, consistent with how 6e itself was
scoped.

## Gates

- `cargo test -p smelt-types --test registry_coverage --quiet` — 106 passed.
- `cargo test -p smelt-db --test integration registry_consistency --quiet` — 6 passed (plus full
  `integration` binary: 369 passed).
- `cargo test -p smelt-dialect --test emission_ownership --test template_emission --quiet` — 18
  passed.
- `cargo test -p smelt-db --test dialect_audit --quiet` — 61 passed (doc-sync regenerated).
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance --quiet`
  — 24 passed.
- `cargo test -p smelt-cli --features databricks --test github_activity_databricks --quiet` — 6
  passed (includes the new `actor_sessions_compiles_without_epoch_us`).
- `cargo test -p smelt-cli --test cross_engine_types_parity --test github_activity_replay
  --test e2e --quiet` — no inference-churn regressions (176 + 21 passed).
- `bash .claude/scripts/verify-phase.sh` gates run individually (the bundled script exceeded the
  10-minute foreground budget): `cargo fmt --all -- --check` (fixed one formatting diff),
  `clippy-gate.sh` (both feature sets, zero warnings), `shellcheck-gate.sh` (74 scripts, zero
  findings), full `cargo test --quiet` (workspace, exit 0 — includes fixing the
  `unknown_census` stale line number and the `large_file_ratchet` baseline bump above),
  `cargo test -p smelt-cli --test example_diagnostics --quiet` (128 passed, 1 ignored).
- Live: `smelt run --target databricks --full-refresh --allow-full-refresh --event-time-start
  2026-08-05 --event-time-end 2026-08-07` → run `20260912-121546-c86d9f`, **11 success / 1
  failed / 4 skipped** — not yet 16/0/0 (see "For the next planner").
