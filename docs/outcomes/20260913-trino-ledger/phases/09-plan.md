# Phase 9 plan — `.smelt/` is not correctness-bearing on Trino

## Objective

Make the residency rule's falsifiable form executable on the Trino target: deleting `.smelt/`
between runs changes no maintained table's value, and `state.mode: stateless` writes nothing
under `.smelt/` while leaving every maintained table equal to what the stateful posture
produced. Advances success criterion 10, and closes the loop on criteria 1–5 by proving from
the outside what those phases established from the inside — Trino keeps no correctness state
anywhere, so there is nothing on disk for a delete to lose.

## Reachability constraint (settles the test seam before writing it)

Phase 6's summary is load-bearing here: Trino has **no `MaintenanceDialect` variant**, so no
`refresh: incremental` model can complete a live `execute_project` run on Trino today — the
downgraded `PerGroupRecompute` cell still emits through the maintenance path and hard-errors
with `UnsupportedMaintenanceDialect`. That is `20260913-trino-incremental`'s subject, not this
phase's. So the phase proves the claim in two halves:

- **Live half** — over what actually runs on Trino today: `materialization: table` models
  (including a downstream `smelt.ref()` so the DAG has more than one node). Real rows, real
  delete, real second run.
- **Offline half** — over maintained models that cannot yet run live: the derived maintenance
  plan (`smelt explain --json`) must be **byte-identical** under `state.mode: intervals` and
  `state.mode: stateless`. That is the optionality rule's own statement — the posture may
  change what smelt can tell you, never what the plan computes — and it reaches the
  incremental shapes the live half cannot.

If the implementer finds an incremental model *does* complete live on Trino, that is a
finding: extend the live half to cover it and record it in the summary rather than leaving
coverage on the table.

## Spec delta

No normative change — this phase proves existing claims (`state.md` §"The residency rule",
§"The optionality rule"). One doc edit only: `docs-site/docs/guide/targets.md`, Trino section
— state that `.smelt/` is safe to delete on Trino and that `state.mode: stateless` is fully
supported, alongside phase 8's locking sentence, so the `✗`-heavy limitations list is not read
as "state is broken here". Re-run the two freshness gates after editing.

## Tests

New file `crates/smelt-cli/tests/trino_state_residency.rs` (live-gated on `SMELT_TRINO_URL`,
skipping green with an `eprintln!` when unset, following `trino_lock_versioning.rs`'s shape and
its `TRINO_ENV_GUARD` static; read rows with `common::fetch_trino_rows`):

1. `deleting_smelt_between_runs_changes_no_trino_table` — run the two-model project, snapshot
   both tables' rows, `fs::remove_dir_all(.smelt/)`, run again, assert both tables' rows are
   equal to the snapshots (and non-empty, so a two-sided-empty pass is impossible).
2. `the_second_run_really_reran_after_the_delete` — after the delete, `.smelt/` is recreated
   and the second run's exit status is success; guards test 1 against passing because the run
   silently no-opped.
3. `stateless_mode_writes_nothing_under_smelt_on_trino` — same project with
   `state.mode: stateless`: the run succeeds and `.smelt/` does not exist afterwards.
4. `stateless_mode_changes_no_trino_table_value` — the rows the stateless run produced equal
   the rows the `intervals` run produced for the same models.

New file `crates/smelt-cli/tests/trino_posture_plan_invariance.rs` (offline, no coordinator —
placeholder target like `trino_explain_downgrade.rs`):

5. `maintenance_plan_is_byte_identical_across_state_modes_on_trino` — `smelt explain --json`
   over a fixture carrying the incremental shapes (`grain: key` and `grain: partition`, reusing
   `trino_explain_downgrade.rs`'s model bodies) returns identical JSON under
   `state.mode: intervals` and `state.mode: stateless`.
6. `the_fixture_actually_carries_downgraded_cells` — the same JSON contains at least one
   `MaintenanceStateDowngraded` cell, so test 5 cannot pass vacuously over a plan with no
   state-dependent cells in it.

## Tasks

1. Read `trino_lock_versioning.rs` and `common::{trino_env, trino_schema, trino_target_block,
   fetch_trino_rows}`; confirm `fetch_trino_rows`' sort/normalisation is deterministic enough
   for a rerun-equality assertion (it sorts; if not, sort in the test).
2. Write `trino_state_residency.rs` with a `stage_residency_project(tmp, schema, state_mode)`
   helper parameterised on the posture; two models — a base `table` model and a downstream one
   selecting from it via `smelt.ref()`.
3. Red-green tests 1–4 against a live tier (`bash scripts/trino-up.sh && source
   scripts/trino-env.sh`); drop the schema afterwards with `common::drop_trino_schema`.
4. Write `trino_posture_plan_invariance.rs`; red-green tests 5–6 offline.
5. Edit the Trino section of `docs-site/docs/guide/targets.md` per the spec-delta note.
6. `bash scripts/trino-down.sh`; run the verification gates; hand-edit
   `.claude/large-file-baseline.txt` only if a *tracked* file regressed (never `--update` —
   phase 7's note).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `bash scripts/trino-up.sh && source scripts/trino-env.sh && cargo test -p smelt-cli --test trino_state_residency` — 4/4 against a live coordinator
- `cargo test -p smelt-cli --test trino_posture_plan_invariance` — 2/2, offline
- `cargo test -p smelt-cli --test trino_spec_freshness --test state_docs_freshness`
- `cargo test -p smelt-core --test trino_docs_freshness`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`test(state): .smelt/ proved non-correctness-bearing on Trino — delete-between-runs equality and a stateless posture that writes nothing`
