# Phase 9b — Trust the numbers: three live oracle checkpoints on Databricks

## Objective

Run the equivalence-oracle sequence 9a built, live: land fixture days `2026-08-13/14/15` as
windows 9–11 on the `databricks` target, full-refresh each window into
`workspace.smelt_dogfood_oracle` on the `databricks_oracle` target, and compare. Commit the
measured report and turn 9a's loud-skipping report gates into hard gates. This closes
criterion 8 (the equivalence invariant on a third engine) and produces the evidence row 9c
needs to root-cause `silver_actor_naming`'s Databricks-only duplication.

## Spec delta

None. This phase measures existing behaviour; it changes no user-visible surface. If the
sweep surfaces a construct that must be fixed for a run to complete at all, that fix carries
its own spec delta and is recorded in the summary.

## Live-phase precondition

Before anything else: `source scripts/dbx-dogfood-env.sh && bash scripts/dbx-auth.sh` then a
reachability probe (`bash scripts/dbx-verify.sh`, or the cheapest `dbx-query.sh` the wrapper
set offers). If the workspace is unreachable or the token cannot be minted, emit
`<<PHASE_BLOCKED>>` with the error — never skip green (outcome driver note).

## Tests

Red-green, all in `crates/smelt-cli/tests/github_activity_dbx_oracle.rs` unless noted.

1. `databricks_incremental_matches_its_oracle_at_every_window` (exists, live) — the sweep
   itself; runs under `SMELT_DBX_DOGFOOD_LIVE=1` and writes the report. Red until the three
   checkpoints' snapshots exist.
2. `the_equivalence_report_covers_every_model_at_every_checkpoint` — delete the
   `let Some(report) = … else { skip }` early return so an absent report is a hard failure;
   every checkpoint names the same relation set, no blank cell.
3. `the_committed_report_proves_the_oracle_read_the_shared_source` (new) — anti-vacuity over
   the committed evidence: every checkpoint records `source_days_loaded == window` and a
   non-zero row count on both sides, so an empty-vs-empty comparison cannot read as success.
   Mirrors `github_activity_bq_oracle.rs`'s gate of the same name.
4. `the_final_window_compares_every_relation_with_nothing_exempt` (new) — at window 11 the
   report compares every relation the project declares, with `DBX_UNBOUNDED_REFRESH_RELATIONS`
   still empty.
5. `the_committed_equivalence_report_shows_no_violation` (new) — every relation at every
   checkpoint has `incr_only == 0 && oracle_only == 0` unless a registry entry licenses it.
   **Only landed if the measured sweep is clean** (see Contingency).
6. `equivalence_registry_entries_are_all_live` (new) — the two-sided ratchet: an entry naming
   a relation the report shows agreeing fails, and a relation the report shows violating with
   no entry fails. **Only landed if the measured sweep is clean** (see Contingency).

## Tasks

1. Build `target/debug/smelt`; source the env; mint the token; probe reachability (above).
2. `bash scripts/dbx-dogfood-oracle.sh duck-types` — the DuckDB type-reference databases for
   windows 9–11.
3. For `n` in 9, 10, 11 in order: `oracle.sh window $n` (loader lands the day, then the
   incremental `smelt run` on `databricks`), `oracle.sh oracle $n` (full refresh into
   `smelt_dogfood_oracle`), `oracle.sh snapshot $n` (read-only NDJSON export of both sides).
   Capture each run report; record any compile refusal or runtime failure verbatim rather
   than fixing in place.
4. `bash scripts/dbx-dogfood-oracle.sh manifest`, then run test 1 live with
   `EQUIVALENCE_MANIFEST` and `EQUIVALENCE_REPORT_OUT` pointing at `phases/09b-equivalence.json`.
5. `bash scripts/dbx-dogfood-oracle.sh report` to render `phases/09b-equivalence.md`.
6. Land tests 2–4 (and 5–6 iff clean); delete the loud-skip branch and the "until phase 9b
   commits one" hedges from the module and gate doc comments.
7. Write `phases/09b-summary.md`: the measured per-relation table, every live failure, the
   `silver_actor_naming` verdict stated explicitly for 9c (does the full-refresh oracle leg
   duplicate too?), and the Free Edition cost/latency observations for criterion 4's facts
   sheet.

## Contingency (the sweep is not clean)

If any relation violates — `silver_actor_naming` is the expected candidate — do **not**
register it away and do **not** fix the write path here (that is 9c's decision, with three
routes already recorded in `## Blocked`). Instead: commit `09b-equivalence.json` and `.md` as
the evidence regardless, land tests 2–4 only, state in the summary exactly which relations
violated and in which direction at which checkpoint, add a dated `## Blocked` entry naming
the violating relations and pointing at row 9c, and emit `<<PHASE_BLOCKED>>`. The report is
the deliverable either way; tests 5–6 move into 9c's scope.

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be green at the end (this is why tests 5–6
  are contingent: a red hard gate over a known-violating report is not a green suite).
- `cargo test -p smelt-cli --test github_activity_dbx_oracle` — offline legs, all pass.
- `cargo test -p smelt-cli --test github_activity_bq_oracle` and `--test
  github_activity_dual_target` — regression, unchanged.
- `bash .claude/scripts/shellcheck-gate.sh` if any script changes.

## Commit message

`outcome(databricks-dogfood-spine): phase 9b measures the equivalence invariant on Databricks over three live windows`
