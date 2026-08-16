# Phase 6 summary — `smelt migrate` reachable mid-incremental-history + migrate-driven recovery leg

## Shipped

- `crates/smelt-runtime/src/execute.rs`: `record_first_deployment_definition` — the first-deployment
  schema-baseline save, extracted into one helper and called from all three completion sites a
  maintained model can return through (the ordinary windowed fall-through, the keyed cumulative
  arm, and the non-keyed key-addressed-dispatch arm). The gate is now just `!already_stored` —
  the old `plan.incremental.is_some()` gate was dropped (see Decisions: it's always `None` for
  `grain: key` models, which would have silently skipped every keyed model forever).
- `crates/smelt-runtime/src/migrate.rs`: `derive_migration_plan_for_model` now strips frontmatter
  from both the recorded ("before") and current ("after") SQL before parsing — a real,
  previously-undetected bug (see Decisions) that made every frontmatter-bearing model's diff
  collapse to `Opaque`/"not a plain SELECT statement".
- `crates/smelt-maintenance-testkit/src/schedule_gen.rs`: `ConformanceStep::MigrateApply` — drives
  the real `smelt migrate <model>` / `--apply` subcommands as subprocesses, no accompanying run;
  excluded from `is_permutable`; not drawn by any generator (hand-pinned only). Unit test
  `migrate_apply_step_is_not_permutable`.
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs`: `drive_and_assert` now returns
  `(STracker, usize, Vec<i32>)` — the third element is each `MigrateApply` step's observed
  `--apply` exit code, in order. Two new pinned gate tests:
  `migrate_apply_recovers_equivalence_after_payload_column_add` (PassThrough + AddPayloadColumn,
  recovers via `--apply` alone) and `migrate_refuses_skeleton_change_then_full_refresh_recovers`
  (AdditiveAgg + AddGroupingColumn, `--apply` refuses exit 3, `FullRefreshRun` recovers).
- `crates/smelt-cli/tests/maintenance_conformance_spark/gate_spark.rs`: `MigrateApply` arm panics
  naming the variant — DuckDB-CLI-only, never part of the Spark pool.
- `crates/smelt-cli/tests/maintenance_conformance/registry.rs`: pruned the
  `known_bug_incremental_path_skips_schema_snapshot` `known_bug_still_reproduces` match arm (dead
  code — the id was already absent from `registry()`); doc comment updated to record the closure.
- `crates/smelt-runtime/tests/definition_recording.rs` (new): four tests —
  `windowed_run_records_deployed_definition`, `cumulative_run_records_deployed_definition`,
  `key_addressed_run_records_deployed_definition`, and
  `windowed_run_after_rewrite_keeps_the_old_recorded_definition`.
- `crates/smelt-cli/tests/migrate.rs`: `migrate_is_reachable_after_incremental_build` — windowed
  incremental build, then `smelt migrate` after an added-column edit exits `3` with a printed plan
  (no `NoRecordedDefinition`).
- `docs/specs/definition_deltas.md` §Detection: added the normative first-deployment recording
  rule. §Known Divergences: added the pending-delta-run-refusal-not-implemented bullet.

## Decisions

- Dropped the `plan.incremental.is_some()` gate from the extracted helper. That field is populated
  only for a `grain: partition` model with a resolved window-batch plan;
  `Config::get_incremental_with_metadata` returns `None` unconditionally when grain isn't
  `Partition`, so a `grain: key` model's `plan.incremental` is *always* `None`. Keeping the old
  gate at the new cumulative-arm call site would have made phase 6 a no-op for every keyed model.
  Every call site already guarantees "non-full-refresh maintenance route" by construction, so
  `!already_stored` alone is the correct (and sufficient) guard.
- Fixed a real bug in `smelt-runtime/src/migrate.rs` discovered by the first pinned gate test: the
  backbuild diff parser (`File::select_stmt()`) resolves only when the SQL body is the file's own
  top-level statement, so a leading `---\n...\n---\n` frontmatter block — which every
  `refresh: incremental` model carries — made `definition_diff` always return `Opaque`. This wasn't
  caught by phases 1-3's CLI tests because their `orders.sql` fixture carried no frontmatter at
  all. Frontmatter is now stripped from both diff sides right before parsing; the raw
  (frontmatter-bearing) form is preserved everywhere else. This is squarely this phase's own
  acceptance target (its own pinned test requires factoring to work for a real, frontmatter-bearing
  incremental fixture), not a pre-existing unrelated red — fixed per the plan's "expected, proceed"
  guidance rather than deferred.
- Test fixtures need `state:\n  mode: intervals\n` in `smelt.yml` for `FileStore` to persist
  anything at all (default `StateMode::Stateless` writes no `.smelt/` directory) — cost real
  debugging time; noted here for the next planner writing a fresh incremental fixture by hand.
- `MigrateApply`'s exit-code recording widened `drive_and_assert`'s return type to a 3-tuple. Only
  two call sites destructured the old 2-tuple (`harness_self_check.rs`, `contract_points.rs`) —
  both updated to bind (and ignore) the new third element; every other call site already discarded
  the whole `Result`, so no other churn was needed.

## For the next planner

- The frontmatter-stripping fix in `migrate.rs` is a real bugfix beyond this phase's listed scope
  — it makes `smelt migrate` actually work for realistic (frontmatter-bearing) models for the first
  time. Worth a note in the outcome-level validate pass (phase 10) that this wasn't a phase 1-3
  regression, just a gap phases 1-3's frontmatter-less test fixtures never exercised.
  `crates/smelt-cli/tests/migrate.rs`'s `ORDERS_SQL_V1` fixture still carries no frontmatter —
  worth adding a frontmatter-bearing fixture there too at some point, though the new
  `migrate_is_reachable_after_incremental_build` test and the gate.rs pinned tests already cover
  the frontmatter-bearing path end to end.
- Phase 7 (atomicity divergence) and phase 8 (diagnostic rename) are next per the outcome table;
  nothing from this phase's scope leaked into either.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test migrate --features duckdb --quiet` — 14 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb --quiet` — 83 passed.
- `cargo test -p smelt-maintenance-testkit --quiet` — 35 passed.
- `cargo test -p smelt-runtime --test definition_recording --quiet` — 4 passed.
- `cargo check -p smelt-cli --tests --features spark` — clean.
