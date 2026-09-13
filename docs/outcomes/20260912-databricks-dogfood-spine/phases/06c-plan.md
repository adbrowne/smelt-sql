# Phase 6c plan — recognise Databricks' drop-type-mismatch error, then land a clean full refresh

## Objective

Close criterion 6's first half, which is still open: the model set has never completed a full
refresh on Databricks. Phase 6b's re-run failed 3 of 16 models (12 skipped) because
`SparkBackend::drop_view_if_exists` / `drop_table_if_exists` only recognise vanilla-OSS-Spark
error shapes, and Unity Catalog returns a third one (`[DROP_COMMAND_TYPE_MISMATCH]`) for the
identical condition. Fix the recognition behind one pure, tested predicate per direction, then
re-run the full refresh live to a clean baseline and record whatever remains. This is the
outcome's named "only fix what's needed for a run to complete at all" exception: every
incremental window in row 7 re-hits this on the same three self-referential models.

## Spec delta

None. This is a backend error-recognition fix, not a change to user-visible behaviour — the
documented semantics of "drop whatever object occupies the name" are unchanged; only the set of
engine error messages recognised as that condition widens.

## Tests

Offline (`crates/smelt-backend-spark/src/tests.rs`, against the two new pure predicates):

1. `drop_view_mismatch_recognises_databricks_message` — the live UC text
   (`[DROP_COMMAND_TYPE_MISMATCH] Cannot drop a table with DROP VIEW. Use DROP TABLE instead. SQLSTATE: 42809`)
   is recognised as "name is a table".
2. `drop_view_mismatch_recognises_oss_spark_messages` — both existing shapes
   (`WRONG_COMMAND_FOR_OBJECT_TYPE`, `DROP VIEW requires a VIEW`) still recognised — no regression.
3. `drop_table_mismatch_recognises_databricks_message` — the symmetric UC text
   (`[DROP_COMMAND_TYPE_MISMATCH] Cannot drop a view with DROP TABLE. Use DROP VIEW instead.`)
   is recognised as "name is a view".
4. `drop_table_mismatch_recognises_oss_spark_messages` — `WRONG_COMMAND_FOR_OBJECT_TYPE` and
   `is a VIEW` still recognised.
5. `unrelated_error_is_not_swallowed` — a permission/table-not-found message is recognised by
   neither predicate, so a genuine failure still propagates (guards against a broad
   `DROP_COMMAND` substring match).

## Tasks

1. Verify live reachability first: `bash scripts/dbx-query.sh "SELECT current_user()"`. If it
   cannot reach the workspace, stop and emit `<<PHASE_BLOCKED>>` — never skip green.
2. Red: add the five tests above against two not-yet-existing pure functions in
   `crates/smelt-backend-spark/src/lib.rs`, `is_table_not_view_error(msg: &str) -> bool` and
   `is_view_not_table_error(msg: &str) -> bool` (module-private, `pub(crate)` if the test module
   needs it).
3. Green: implement the two predicates with the three message shapes each, and rewrite
   `drop_view_if_exists` / `drop_table_if_exists` to call them instead of inline `msg.contains`
   chains, so there is exactly one owner per direction. Keep the existing comments' intent.
4. Run the offline gates; confirm the Spark parity tier is untouched by inspection (no change to
   `spark_adapter.py`, `sql.rs`, or any emitted statement).
5. Live: `source scripts/dbx-dogfood-env.sh` then
   `smelt run --target databricks --full-refresh --allow-full-refresh --event-time-start 2026-08-05 --event-time-end 2026-08-07`
   against the dogfood schema. Capture the run report under
   `examples/github_activity/.smelt/targets/databricks/reports/` and commit it.
6. Record the result in `phases/06c-summary.md`: per-model outcome + row counts, and for every
   remaining failure the model, the statement, the live error text, the root cause, and a fix
   candidate — recorded, not fixed, unless a failure again blocks *any* model completing.
7. If the refresh is clean (16/16 or only registered-divergence failures), say so explicitly in
   the summary: row 7's windows start from this baseline, and the "For the next planner" section
   must name the exact command and event-time window row 7 should advance to.
8. Do NOT attempt the incremental windows here — that is row 7.

## Verification

- `cargo test -p smelt-backend-spark --lib --quiet` — the five new tests green.
- `cargo test -p smelt-cli --features databricks --test github_activity_databricks --test dbx_dogfood_provision --quiet`
- `bash .claude/scripts/verify-phase.sh` — must be all green; no ratchet lowered.
- Live: the full-refresh run above, with its committed run report as the evidence artifact.

## Commit message

`outcome(databricks-dogfood-spine): phase 6c recognises Unity Catalog's drop-type-mismatch error and lands a clean full refresh`
