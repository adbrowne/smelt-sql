# Phase 9 summary — `.smelt/` is not correctness-bearing on Trino

## Shipped

- `crates/smelt-cli/tests/trino_state_residency.rs` (live-gated, `SMELT_TRINO_URL`): a two-model
  `materialization: table` project (`base_table` + `downstream_table` reading it via
  `FROM smelt.base_table`) proves deleting `.smelt/` between two live runs changes neither
  table's value, that the second run actually reran (not a silent no-op), that
  `state.mode: stateless` writes no `.smelt/` on Trino, and that a stateless run produces the
  same table values as an `intervals` run of the same project. 5/5 passing against the local
  Docker tier.
- `crates/smelt-cli/tests/trino_posture_plan_invariance.rs` (offline, placeholder target): proves
  `smelt explain <model> --json`'s `cells` are byte-identical under `state.mode: intervals` and
  `state.mode: stateless`, for both `lifetime_spend` (`grain: key`, `KeyedFold`) and `cs_merged`
  (`grain: partition`, `ColumnScopedMerge`) — the two incremental shapes that cannot yet complete
  a live run on Trino (no `MaintenanceDialect` variant). Test 6 guards against vacuous pass by
  asserting both fixtures actually carry a `state_downgrade` cell.
- `docs-site/docs/guide/targets.md` Trino §Limitations: one added sentence stating `.smelt/` is
  safe to delete and `state.mode: stateless` is fully supported, alongside the existing
  phase-8 locking sentence.

## Decisions

- **`region_partitioned` (a plain `grain: partition` cell with no key-addressed edge) does not
  carry a `state_downgrade` at all** — its ideal technique is already `DeleteInsert`, which
  needs no state structure (`docs/specs/state.md` §"The degradation contract": "a plain,
  clamp-bounded `PerGroupRecompute` cell requires nothing"). Test 6 would have been vacuous
  against it, so the offline fixture reuses `cs_merged` (`ColumnScopedMerge`, requires the
  transactional merge ledger) from `trino_explain_downgrade.rs`'s four-shapes fixture instead —
  confirmed by probing `smelt explain --json` directly before writing the assertion.
- Test 5 compares per-model `explain --json`'s `cells` array, not the whole-project
  `explain --json` (no positional model name), because the whole-project shape (`models.<name>`,
  keyed by model, carrying `incremental`/`source_bounds`) doesn't expose `state_downgrade` at
  all — only the per-model report does.

## Fixed in this phase (found by its own test, not pre-existing red)

`crates/smelt-backend-trino/src/backend.rs`: **every second `smelt run` against a
`materialization: table` model on Trino failed**, independent of `.smelt/` — not specific to
this phase's delete scenario, just never exercised before (no prior test ran `smelt run` twice
against the same live table). `execute_model_default` (`smelt-backend/src/lib.rs`) always drops
the *other* materialization kind before creating the requested one (`Materialization::Table` →
`drop_view_if_exists` then `drop_table_if_exists` then `create_table_as`). Trino's
`DROP VIEW IF EXISTS` only suppresses "does not exist" when *nothing* exists under that name —
if a table already exists there (from the first run), it raises `"View 'x' does not exist, but
a table with that name exists"` instead of a silent no-op. Added `drop_if_exists_tolerant`,
swallowing that specific cross-kind-mismatch message (still means "no view of that name
exists", which is exactly what `IF EXISTS` asks), applied to all six `DROP TABLE/VIEW IF EXISTS`
call sites in the file (`create_table_as`, `create_view_as`, `drop_table_if_exists`,
`drop_view_if_exists`, and `load_table`'s pair). Verified: `trino_state_residency.rs` 5/5,
`trino_lock_versioning.rs` 4/4, `trino_explain_downgrade.rs` 8/8, `smelt-backend-trino`'s own
suite 12/12 + 27/27, all still green after the fix.

## For the next planner

- This phase closed the loop on criteria 1–5 from the outside as scoped. Nothing else was
  deferred: the plan's reachability constraint (live half = table models only, offline half =
  incremental shapes via plan-invariance) was followed exactly, and no incremental model turned
  out to complete live on Trino (still blocked on the missing `MaintenanceDialect`, per phase
  6 — that finding did not change).
- The rerun bug just fixed was a real gap in general Trino re-run correctness, not specific to
  `.smelt/` deletion — every live-run test elsewhere in the suite ran `smelt run` only once
  against a given table before this phase, so it was previously unreached. Worth a note for
  whoever picks up `20260913-trino-incremental`: the same `execute_model_default` drop-both
  pattern will matter again once incremental techniques start writing through Trino.
- Phase 10 (the keyless staged emitter's `CREATE TEMP TABLE` sentinel) and phase 11 (surface and
  close) are next; nothing here changes their scope.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full `cargo test` workspace, example_diagnostics)
- `bash scripts/trino-up.sh && source scripts/trino-env.sh && cargo test -p smelt-cli --test trino_state_residency` — 5/5 (includes the vacuous-pass guard)
- `cargo test -p smelt-cli --test trino_posture_plan_invariance` — 2/2, offline
- `cargo test -p smelt-cli --test trino_spec_freshness --test state_docs_freshness` — 9/9
- `cargo test -p smelt-core --test trino_docs_freshness` — 6/6
- `bash .claude/scripts/large-file-check.sh` — OK
- Regression check post-fix: `trino_lock_versioning.rs` 5/5, `trino_explain_downgrade.rs` 4/4,
  `smelt-backend-trino` full suite green
- `bash scripts/trino-down.sh` — tier torn down
