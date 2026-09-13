# Phase 9 plan — end-to-end on the real pipeline

## Objective

Make `type: trino` a target a real project can *run*. `dialect_and_capabilities` stops refusing
Trino and hands back `(SqlDialect::Trino, BackendCapabilities::trino_iceberg())`; a committed
example workspace carrying a Trino target checks clean offline; and `execute_project` — driven
through the real `smelt run` CLI path — materializes a table and a view on the live tier, rows
readable back, run report written. Advances criterion 7 (and closes the last blocker the
criterion-4/6 legs were working around). Criteria 8/9 stay for phases 10/11.

## Spec delta

None. The `trino` target shape, the capability column and the connection-security rule were all
written by phases 1 and 8; this phase changes no user-visible surface, only reaches it. One
non-normative edit: `multi_backend.md` §Known Divergences gains a line recording that a Trino
`array(...)` **result** column does not decode to Arrow yet (phase 8 discovery), so a model
projecting an array type fails on read-back — with no claim about when it closes.

## Tests

Offline (run in `verify-phase.sh`):

1. `smelt-runtime` `compile.rs` unit — `dialect_and_capabilities_returns_measured_trino_profile`:
   replaces `dialect_and_capabilities_refuses_trino_until_measured`; a `type: trino` target now
   yields `SqlDialect::Trino` + `BackendCapabilities::trino_iceberg()` and `SqlCompiler::new`
   succeeds.
2. `smelt-runtime` `compile.rs` unit — `trino_compile_refuses_qualify`: a model using `QUALIFY`
   compiled against the Trino target fails with `UnsupportedOnBackend`, proving the measured
   `false` flags reach the compile-path refusal rather than being inert.
3. `smelt-cli` `example_diagnostics` — `trino_spine_no_diagnostics`: `examples/trino_spine`
   checks with zero diagnostics (no live server).
4. `smelt-lsp` `example_workspaces` — `trino_spine`: same workspace clean through the real LSP
   backend.

Live-gated (skip green when `SMELT_TRINO_URL` is unset; each prints its skip reason):

5. `smelt-cli` `trino_smoke.rs` — `trino_smoke_materializes_table_and_view`: stages
   `examples/trino_spine` into a tempdir, injects the live coordinator into `smelt.yml`, runs
   `smelt run --target trino`, asserts exit 0, both relations exist as Iceberg objects, and
   `fetch_trino_rows` returns the expected rows for each.
6. `smelt-cli` `trino_smoke.rs` — `trino_smoke_writes_run_report`: the same run writes a run
   report naming both models with a success status.
7. `smelt-cli` `materialization_parity.rs` — the existing
   `view_and_table_materialize_consistently_on_both` gains a Trino leg via
   `TargetKind::Trino { schema }`, asserting cross-backend row parity against the DuckDB
   reference.
8. `smelt-cli` `trino_smoke.rs` — `trino_legs_skip_not_pass_when_url_unset`: with
   `SMELT_TRINO_URL` removed, `trino_env()` is `None` and `targets_to_run_with_trino` yields no
   Trino leg — the vacuous-pass guard criterion 8 also asks for.

## Tasks

1. Delete `TrinoCapabilitiesUnmeasured` from `crates/smelt-runtime/src/compile.rs`; make
   `dialect_and_capabilities` infallible again and drop the `?` at its one call site.
2. Rewrite the refusal unit test as test 1; add test 2.
3. Add `examples/trino_spine/` — `smelt.yml` with a `dev` duckdb target plus a `trino` target
   (placeholder `host`/`port`/`user`/`catalog`/`schema`, password via `${ENV}` or absent), and
   two models: `spine_table.sql` (`materialization: table`) and `spine_view.sql`
   (`materialization: view`), both over literal rows so no seed or source is needed.
   `examples/README.md` gains its line.
4. Wire tests 3 and 4 into `example_diagnostics/smoke_and_migration.rs` and
   `smelt-lsp/tests/example_workspaces.rs`.
5. Add `TargetKind::Trino { schema }` to `crates/smelt-cli/tests/common/mod.rs`; give it real
   arms in `fetch_rows` (→ `fetch_trino_rows`) and the drop helper (→ `drop_trino_schema`),
   reusing phase 7's helpers. Add `targets_to_run_with_trino(label)` — `targets_to_run(label)`
   plus the Trino leg when `trino_env()` is `Some` — and leave `targets_to_run` itself
   Trino-free (see the decision-log entry: the incremental/merge/schema-evolution parity legs
   belong to the sibling outcomes, not here).
6. Give every other suite matching `TargetKind` a loud Trino arm that panics naming the sibling
   outcome that owns its leg — dead today because `targets_to_run` never yields it, but
   fail-loud rather than a wildcard the next variant would slip through.
7. Switch `materialization_parity.rs` to `targets_to_run_with_trino`, add its Trino arm and the
   `trino:` block to `stage_mat_workspace`'s `smelt.yml` (via `trino_target_block`), and drop
   the suite's Trino schema at the end.
8. Add `crates/smelt-cli/tests/trino_smoke.rs` (tests 5, 6, 8), modelled on `spark_smoke.rs` but
   asserting rather than collecting breaks.
9. Delete the now-stale "Trino is deliberately NOT a `TargetKind` variant" comment block in
   `common/mod.rs` and the phase-8-blocked notes in `seed_parity.rs`.
10. Add the array-decode Known Divergence line to `docs/specs/multi_backend.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt + clippy both feature sets + tests +
  example_diagnostics), with `SMELT_TRINO_URL` **unset** — every live leg must skip green.
- `bash scripts/trino-up.sh && source scripts/trino-env.sh` then:
  - `cargo test -p smelt-cli --test trino_smoke` — all live legs run, 0 skipped.
  - `cargo test -p smelt-cli --test materialization_parity` — Trino leg runs.
  - `cargo test -p smelt-cli --test seed_parity` — unchanged, still green.
  - `cargo test -p smelt-backend-trino` — unchanged.
  - `bash scripts/trino-down.sh`
- `cargo test -p smelt-lsp --test example_workspaces trino_spine`
- `cargo test -p smelt-runtime --test dialect_seam` — the compile-path refusal gate.

## Commit message

`feat(trino): run a table and a view end-to-end on the Trino target via execute_project`
