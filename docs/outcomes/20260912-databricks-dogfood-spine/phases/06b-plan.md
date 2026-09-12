# Phase 6b plan — unblock the Databricks run path (dialect-dispatched hash; bare-host reconciliation)

**[live]** — this phase ends with a live full refresh. If `scripts/dbx-dogfood-env.sh`
cannot reach the workspace, land tasks 1–5 (all offline, all gated) and emit
`<<PHASE_BLOCKED>>` rather than claiming the live leg.

## Objective

Phase 6's full refresh completed 1 of 16 models: the maintenance layer's baseline-snapshot
fingerprint spells `sha256(...)` as literal dialect-unaware text, and Spark/Databricks has
only `sha2(expr, 256)`. Make that spelling dialect-dispatched behind one owner with a
structural gate, reconcile the wizard's scheme-bearing `SMELT_DBX_HOST` against the
`type: databricks` target's bare-hostname contract, and re-run the full refresh to a clean
baseline. Advances criterion 6 (a run that completes at all) and unblocks 7 and 8, which are
vacuous over a single model.

## Spec delta

None. The fingerprint expression is internal maintenance-layer SQL, not user-visible surface,
and the `host:` contract already reads "bare hostname, no scheme" — this phase conforms the
tooling to the spec rather than changing it. The `SMELT_DBX_HOSTNAME` export is script-level,
documented in the script header, not spec surface.

## Tests

Red-green, in this order:

1. `crates/smelt-logical/tests/emit_statements.rs::spark_baseline_snapshot_uses_sha2_not_sha256`
   — the Spark-dialect append-only baseline snapshot contains `sha2(` and no `sha256(`.
2. `…emit_statements.rs::spark_row_fingerprint_uses_sha2_not_sha256` — the per-column and
   whole-row fingerprint expressions on Spark likewise.
3. `…emit_statements.rs::duckdb_and_bigquery_hash_spellings_are_unchanged` — pins the exact
   existing DuckDB (`sha256(`) and BigQuery (`TO_HEX(SHA256(`) shapes byte-for-byte, so the
   refactor cannot silently move the two engines already proven live.
4. `crates/smelt-logical/tests/maintenance_dialect_blindness.rs::hash_spelling_has_one_owner`
   — structural gate: no production file under `src/maintenance/` other than the new
   `emit/hash.rs` mentions a hash-function spelling (`sha256(`, `SHA256(`, `sha2(`), and in
   `hash.rs` every such mention sits on a `MaintenanceDialect::… =>` dispatch line.
5. `…maintenance_dialect_blindness.rs::the_hash_scan_flags_a_planted_spelling` — planted-source
   negative control, mirroring the existing planted-hardcode test.
6. `crates/smelt-cli/tests/github_activity_databricks.rs::databricks_target_host_is_bare` —
   `examples/github_activity/smelt.yml`'s `databricks` target config-loads with a
   scheme-bearing `SMELT_DBX_HOST` in the environment (i.e. it reads the bare variable), and
   still refuses a scheme if one reaches `host:`.
7. `crates/smelt-cli/tests/dbx_dogfood_provision.rs::env_script_exports_bare_hostname` —
   `scripts/dbx-dogfood-env.sh` exports `SMELT_DBX_HOSTNAME` as `SMELT_DBX_HOST` stripped of
   scheme and any trailing slash, leaving `SMELT_DBX_HOST` itself untouched (its URL consumers
   `dbx-auth.sh` and `dbx_dogfood_query.py` still need the scheme).

## Tasks

1. Add `crates/smelt-logical/src/maintenance/emit/hash.rs`: `hash_digest_expr(expr, dialect)`
   (inner/per-column: DuckDb `sha256(x)`, Spark `sha2(x, 256)`, BigQuery `SHA256(x)` — keeping
   BigQuery's current lowercase-accepted BYTES-returning shape) and `hash_hex_expr(expr,
   dialect)` (outer/string-returning: DuckDb `sha256(x)`, Spark `sha2(x, 256)`, BigQuery
   `TO_HEX(SHA256(x))`). Doc-comment why Spark needs the bit-length argument.
2. Route every hash spelling in `emit/fingerprint.rs` and `emit/probes.rs` through those two
   helpers; thread `dialect` into `column_fingerprint_expr` / `concat_varchar_expr_typed`,
   which currently take only the cast-type string.
3. Extend `maintenance_dialect_blindness.rs` with the hash-owner scan and its planted control.
4. Export `SMELT_DBX_HOSTNAME` (bare) from `scripts/dbx-dogfood-env.sh`; point
   `examples/github_activity/smelt.yml`'s `databricks` target `host:` at it; update the four
   test call sites that stub `SMELT_DBX_HOST` to stub the new variable too
   (`github_activity_support::smelt_run`, `github_activity_replay.rs` ×2,
   `list_external_step.rs`).
5. Run the offline gates (see Verification). Everything to here is workspace-free.
6. **Live, only if reachable:** `source scripts/dbx-dogfood-env.sh` with no shell workaround,
   confirm reachability with `scripts/dbx-query.sh` before anything else, then re-run the
   phase-6 full refresh verbatim:
   `smelt run --target databricks --full-refresh --allow-full-refresh --event-time-start
   2026-08-05 --event-time-end 2026-08-07`.
7. Read back `SHOW TABLES IN workspace.smelt_dogfood` and a `count(*)` per materialised model;
   commit the run report under `.smelt/targets/databricks/reports/`.
8. Write `phases/06b-summary.md`: the model-by-model outcome table, and every *remaining*
   failure recorded in phase 6's format (model, smallest reproducing SQL, live error, root
   cause located in source, blast radius) — **recorded, not fixed**. The record-don't-fix brief
   is back in force now that a run completes; the only admissible further fix is one without
   which no run completes at all, and it must say so explicitly.
9. Note for row 10's findings handoff: the maintenance layer hand-spells hash functions outside
   `BuiltinRegistry`'s `Signature::emission` table, which is why this reached a live engine as
   an `AnalysisException` rather than a compile-time `UnsupportedOnBackend`. This phase fixes
   the spelling, not the ownership. Do **not** attempt the registry migration here.

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be all green (fmt, clippy both feature sets,
  shellcheck, full `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test emit_statements --test maintenance_dialect_blindness
  --test probe_execution --quiet`
- `cargo test -p smelt-logical --test walk_coverage --quiet` and
  `cargo test -p smelt-runtime --test statement_parity --quiet` — the maintenance-emitter
  invariant gates.
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — the equivalence gate over
  the real DuckDB pipeline, proving the DuckDB hash shape did not move.
- `cargo test -p smelt-cli --test github_activity_databricks --test dbx_dogfood_provision
  --test github_activity_replay --test example_diagnostics --quiet`
- `cargo build -p smelt-cli --features databricks`
- Live: the committed run report plus the read-back counts, quoted in the summary.

## Commit message

`outcome(databricks-dogfood-spine): phase 6b dialect-dispatches the maintenance hash and reconciles the target host`
