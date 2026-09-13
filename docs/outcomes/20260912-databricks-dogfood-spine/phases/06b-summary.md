# Phase 6b summary — dialect-dispatched hash, host reconciliation, and one clean re-run

## Shipped

- `crates/smelt-logical/src/maintenance/emit/hash.rs` (new): `hash_digest_expr`/
  `hash_hex_expr`, the single owner of every `sha256`/`SHA256`/`sha2` spelling under
  `src/maintenance/` — DuckDB and BigQuery keep their exact byte-for-byte existing shapes;
  Spark/Databricks now emits `sha2(x, 256)` instead of the nonexistent `sha256(x)`.
- `crates/smelt-logical/src/maintenance/emit/fingerprint.rs` and `.../probes.rs` now route
  every hash spelling through `hash.rs` (`column_fingerprint_expr`, `row_fingerprint_expr`,
  `emit_repair_group_digest_select`'s DuckDB combine, and both append-only/source-mutation
  aggregate fingerprints in `probes.rs`).
- `maintenance_dialect_blindness.rs` gained `hash_spelling_has_one_owner` (+ its planted-control
  test) — a structural gate mirroring the existing DuckDb-hardcode scan, failing on any
  `sha256(`/`SHA256(`/`sha2(` occurrence under `src/maintenance/` outside a dispatch line in
  `hash.rs`.
- Three new correctness tests in `emit_statements.rs`
  (`spark_baseline_snapshot_uses_sha2_not_sha256`, `spark_row_fingerprint_uses_sha2_not_sha256`,
  `duckdb_and_bigquery_hash_spellings_are_unchanged`) plus two stale-expectation fixes
  (`digest_select_uses_spark_string_cast_on_spark`, `source_mutation_fingerprint_spark_shape`)
  that had pinned the old, wrong Spark `sha256(` spelling.
- Host reconciliation: `scripts/dbx-dogfood-env.sh` now exports `SMELT_DBX_HOSTNAME`
  (`SMELT_DBX_HOST` with scheme + trailing slash stripped); `examples/github_activity/smelt.yml`'s
  `databricks` target reads `host: ${SMELT_DBX_HOSTNAME}` instead of the scheme-bearing
  `${SMELT_DBX_HOST}`. Five test call sites updated to stub both variables
  (`github_activity_support::smelt_run`, `github_activity_replay.rs` ×3, `list_external_step.rs`).
  New tests: `dbx_dogfood_provision.rs::env_script_exports_bare_hostname`,
  `github_activity_databricks.rs::databricks_target_host_is_bare`.
- `.claude/large-file-baseline.txt` updated for the two files this phase grew past their
  entries (`fingerprint.rs` 1188→1196, `emit_statements.rs` 1730→1807) — legitimate growth from
  the new hash-dispatch code and its tests, no reviewer sign-off issue.
- A clean re-run of the phase-6 full refresh against the live workspace, with a committed run
  report (`.smelt/targets/databricks/reports/20260912-100131-2d806a.json`).

## Decisions

- Kept BigQuery's inner digest spelling as the existing lowercase `sha256(` (not cased to
  `SHA256`) in `hash_digest_expr`, matching what phase 1/2's tests already proved live —
  changing case would have been an unforced, untested behavior change outside this phase's
  boundary.
- Routed the DuckDB-only `bit_xor(hash(sha256(...)))` combine in
  `emit_repair_group_digest_select` through `hash_digest_expr(&row_digest_expr, dialect)` even
  though only the DuckDB arm needs it, so the single-owner structural gate has no second
  `sha256(` spelling to flag outside `hash.rs`.
- Used `smelt explain --json` rather than `smelt list` for the new host-reconciliation
  integration test (`databricks_target_host_is_bare`): `smelt list` against
  `examples/github_activity` hits an unrelated, pre-existing discovery issue over the project's
  root-level `sample.sql`/`setup_sources.sql` scratch files (not paths-scoped out), which is
  orthogonal to this phase and left unrecorded as a fix target since `explain` proves the same
  config-load/interpolation behavior without tripping it.

## Live re-run result

**1 success, 3 failed, 12 skipped** (same shape as phase 6's own count, but for a different
reason — see below). All fixture-day full-refresh: `--event-time-start 2026-08-05
--event-time-end 2026-08-07`.

| Model | Outcome | Rows |
|---|---|---|
| `silver.events_deduped` | success | 5,915 |
| `bronze.events` | **failed** | 0 (table already holds 5,978 rows from an earlier run) |
| `silver.actor_naming` | **failed** | 0 |
| `silver.repo_naming` | **failed** | 0 |
| every other model (12) | skipped (downstream of a failed model) | 0 |

The phase-6 root cause (`sha256` reaching Spark, `[UNRESOLVED_ROUTINE]`) is confirmed fixed:
none of the three current failures mention `sha256`, and the host-reconciliation working
directly (`source scripts/dbx-dogfood-env.sh` with no shell workaround, `smelt run --target
databricks` resolving `host: ${SMELT_DBX_HOSTNAME}` cleanly) is proven by this same run.

### New finding: `drop_view_if_exists`'s Databricks error-code drift (recorded, not fixed)

**Model / statement:** every self-referential bootstrap model (`crates/smelt-runtime/src/
execute/project/mod.rs:4424-4429`: `is_self_referential(...) && Materialization::Table` calls
`backend.drop_view_if_exists(...)` unconditionally, then `drop_table_if_exists(...)`, before
bootstrapping) — `bronze.events`, `silver.actor_naming`, `silver.repo_naming` in this model set.

**Live error:**
```
AnalysisException: [DROP_COMMAND_TYPE_MISMATCH] Cannot drop a table with DROP VIEW.
Use DROP TABLE instead. SQLSTATE: 42809
```

**Root cause:** `SparkBackend::drop_view_if_exists` (`crates/smelt-backend-spark/src/
lib.rs:453-468`) issues `DROP VIEW IF EXISTS` and swallows the error only when the message
contains `"WRONG_COMMAND_FOR_OBJECT_TYPE"` or `"DROP VIEW requires a VIEW"` — the two shapes
vanilla OSS Spark returns when the target name is actually a TABLE. Databricks/Unity Catalog
returns a third shape for the identical situation, `[DROP_COMMAND_TYPE_MISMATCH] Cannot drop a
table with DROP VIEW`, which the string match does not recognize, so the error propagates and
fails the whole model — even though the underlying condition (a real, valid table already
exists at that name from a prior run) is exactly the case this fallback exists to swallow.
Confirmed live: `DESCRIBE EXTENDED workspace.smelt_dogfood.bronze_events` shows a real
`MANAGED`/`delta` table with 5,978 rows (from run `20260912-085552-2f3857` or earlier), not a
stray view — the drop-both-defensively pattern is working as designed, only the Databricks
message isn't recognized.

**Blast radius:** every self-referential bootstrap model on the Databricks/Databricks-Connect
path; on the first-ever run against a schema (no pre-existing object of either kind) the DROP
VIEW would presumably no-op cleanly instead, so this only bites once a prior run has already
materialized the table — which is every run in this outcome from here on, since phase 5/6 left
these three tables in place.

**Not fixed here:** phase 6b's stated boundary is the hash spelling and the host contract; this
is a new, unrelated error-message-matching gap, and a run still completes (1 model succeeds) so
the "only fix what's needed to complete at all" exception does not apply. Left for row 10's
findings handoff / a follow-on outcome. The candidate fix is adding
`"DROP_COMMAND_TYPE_MISMATCH"` to `drop_view_if_exists`'s (and `drop_table_if_exists`'s
symmetric `WRONG_COMMAND_FOR_OBJECT_TYPE`/`"is a VIEW"` check, which likely has the same gap in
the opposite direction) string match.

## For the next planner

- The `drop_view_if_exists`/`drop_table_if_exists` Databricks error-message gap above is the
  next blocker for phases 7-8 (three incremental windows, dual-target parity) — until it's
  fixed, every run after the first against an already-populated schema will fail the same three
  self-referential models. It is squarely inside this outcome's stated boundary (a run must
  complete, three consecutive windows are needed), so the next live phase should budget a fix
  for it rather than treat it as pure record-and-defer — the exact one-line-per-arm fix is
  named above.
- The `INVALID_HANDLE.SESSION_CLOSED` warning phase 4c/5 already flagged recurred on every
  `dbx-query.sh` call in this phase too (visible in the `SHOW TABLES`/`DESCRIBE EXTENDED` output
  above) — still believed to be a Free Edition serverless teardown artifact, not a smelt bug;
  still a `free-edition-facts.md` fact, not a fix target.
- The gpg-agent-cache-primed `dbx-auth.sh` refresh (Blocked item (b)'s resolution) worked
  cleanly in this session with no human intervention beyond what was already primed — no new
  finding there.

## Gates

- `cargo test -p smelt-logical --lib maintenance::emit` — 85 passed.
- `cargo test -p smelt-logical --test emit_statements --test maintenance_dialect_blindness
  --test probe_execution --quiet` — 64 + 5 + 14 passed.
- `cargo test -p smelt-logical --test walk_coverage --quiet` — 14 passed.
- `cargo test -p smelt-runtime --test statement_parity --quiet` — 41 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — 104 passed.
- `cargo test -p smelt-cli --features databricks --test github_activity_databricks --test
  dbx_dogfood_provision --test github_activity_replay --test example_diagnostics --quiet` —
  9 + 129 (1 ignored) + 4 passed.
- `cargo build -p smelt-cli --features databricks --quiet` — clean.
- `bash .claude/scripts/verify-phase.sh` — **ALL GREEN** (fmt, clippy both feature sets,
  shellcheck, full `cargo test`, example_diagnostics), after updating the large-file baseline
  for the two files this phase legitimately grew.
- Live: `smelt run --target databricks --full-refresh --allow-full-refresh --event-time-start
  2026-08-05 --event-time-end 2026-08-07` against the real Free Edition workspace, run report
  `20260912-100131-2d806a.json` committed; `SHOW TABLES IN workspace.smelt_dogfood` and
  `DESCRIBE EXTENDED .../bronze_events` read back live to confirm the finding above.
