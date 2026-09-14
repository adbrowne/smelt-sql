# Phase 4 summary — the emulated delete-and-insert window covers exactly what it writes

**Shipped:**
- `TrinoBackend::insert_overwrite` (`crates/smelt-backend-trino/src/backend.rs`) delegates to
  `delete_and_insert_transactional` instead of refusing by name (BigQuery's precedent).
- `TrinoBackend::delete_and_insert_transactional` override targets the catalog-qualified,
  double-quoted three-part name (`self.qualified_name`), mirroring Spark's/BigQuery's overrides;
  the emitted text still comes from `emit_delete_insert` — no authoring in this crate.
- `TrinoBackend::delete_partitions`'s refusal message rewritten to say it is unreached (no
  runtime path calls it) rather than pointing at this outcome.
- `emit_delete_insert` unit test (`crates/smelt-logical/src/maintenance/emit/recompute.rs`) —
  the DELETE predicate is byte-identical to the region's own predicate.
- `insert_overwrite_is_emulated_not_refused` unit test (`crates/smelt-backend-trino/src/
  backend.rs`) — no live coordinator needed, proves the by-name refusal is gone.
- `statement_parity::trino::delete_insert_parity_on_trino` (`crates/smelt-runtime/tests/
  statement_parity/trino.rs`) — executed DELETE+INSERT byte-identical to a direct
  `emit_delete_insert` call; the DELETE's two literals proven identical to the INSERT body's
  injected clamp literals, recovered from the executed text.
- Three live CLI-level tests in `crates/smelt-cli/tests/trino_incremental_families.rs`: exact
  no-wider/no-narrower coverage under a source mutation, out-of-order window application vs a
  `--full-refresh` oracle, and repeated-window idempotence.
- Spec deltas: `docs/specs/incremental_shapes.md` §"First-run and backfill" and
  `docs/specs/multi_backend.md` §"Incremental & schema evolution per backend" state the
  non-atomicity of the DELETE+INSERT pair on Trino/Iceberg (autocommit-only writes) and the
  recovery property that makes it safe.

**Decisions:**
- `Backend::insert_overwrite`/`delete_and_insert_transactional` have **no production caller**
  either — the real `DeleteInsert` dispatch (`execute_model_incremental_with_bookkeeping`'s
  `IncrementalStrategy::DeleteInsert` arm) builds the group via `build_delete_insert_group` with
  a bare `schema.table` name and never routes through this method. Implemented it anyway per the
  plan's BigQuery precedent (it's public `Backend` API, exercised by `smelt-maintenance-testkit`'s
  Link-C harness and each backend's own live tests); the live CLI tests assert the *actual*
  bare-name dispatch path instead. Recorded in the outcome's Decision log.
- A fixture needing a between-run source mutation must be a **declared external source**, not a
  first-class inline model — `stage_int_partition_project`'s `seed_events` is rebuilt from its
  static `VALUES` body on every run, silently undoing a raw mutation. Fixed by staging the
  mutation test over a declared `models/sources/events.yml` instead.

**For the next planner:**
- Phase 5 (merge-less conditional write + column-scoped merge) and phase 6 (degraded routes) are
  next per the outcome's phase table; nothing from phase 4 blocks them.
- The two DeleteInsert-dispatch-path discovery (bare vs qualified name) is generic — not
  Trino-specific — and may be worth a follow-up note on whether `insert_overwrite`/
  `delete_and_insert_transactional` should eventually become the one production dispatch path,
  but that is out of this phase's and this outcome's scope (no family/technique work is licensed
  here per the outcome's "Out of scope" section).

**Gates:**
- `bash .claude/scripts/verify-phase.sh`-equivalent run manually: `cargo fmt --all -- --check`
  (clean), `bash .claude/scripts/clippy-gate.sh` (clean, both feature sets), `cargo test --quiet`
  (full workspace, exit 0), `cargo test -p smelt-cli --test example_diagnostics` (129 passed, 1
  ignored), `bash .claude/scripts/shellcheck-gate.sh` (clean) — all green.
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` (no live tier): the
  Trino leg skips cleanly.
- `cargo test -p smelt-cli --test trino_ci_wiring`: green.
- Live tier (`bash scripts/trino-up.sh` / `source scripts/trino-env.sh`):
  `cargo test -p smelt-cli --test trino_incremental_families -- --test-threads=1` (10/10),
  `cargo test -p smelt-runtime --test statement_parity -- --test-threads=1` (43/43),
  `cargo test -p smelt-backend-trino --test backend_live -- --test-threads=1` (15/15). Torn down
  with `bash scripts/trino-down.sh`.
