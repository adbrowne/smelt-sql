# Phase 9a summary — the Databricks equivalence-oracle harness, offline

## Shipped

- `examples/github_activity/smelt.yml`: `databricks_oracle` target (Unity Catalog
  `workspace.smelt_dogfood_oracle`), off the no-`--target` default.
- `models/sources/raw/github_events{,_arrival}.yml`: `databricks_oracle:` name-map entries
  pointing at the shared `smelt_dogfood.*` tables (anti-vacuity — without them the oracle
  would read a table nothing creates).
- `crates/smelt-cli/tests/parity_support/mod.rs`: extracted the equivalence-manifest shape
  (`EquivalenceCheckpoint`/`EquivalenceManifest`, with an optional `source_days_loaded` field
  used only by the Databricks leg), the `violating_pair`/`synth_db` negative-control fixtures,
  and `assert_source_covers_window` — the per-checkpoint measurement that replaces BigQuery's
  unbounded-refresh exemption list on a target whose source lands one day at a time.
  `github_activity_bq_oracle.rs` now consumes the shared shapes instead of restating them (no
  behaviour change — its 14 tests stay green).
- `crates/smelt-cli/tests/github_activity_dbx_oracle.rs` (new, 13 tests, all offline): the
  anti-vacuity gate, the no-`--target`-default-unmoved gate, the registry-consulting sweep's
  negative controls, the empty-exemption-list assertion, the source-ran-ahead-of-its-window
  failure, the script/suite checkpoint-schedule lockstep check, the report-totality gate
  (loud-skips until 9b commits a report), and the live sweep
  `databricks_incremental_matches_its_oracle_at_every_window` (skips without
  `SMELT_DBX_DOGFOOD_LIVE=1`, fails loudly rather than skipping when live and the manifest is
  missing).
- `scripts/dbx-dogfood-oracle.sh` (new, shellcheck-clean, allow-listed alongside
  `dbx-dogfood-parity.sh` in `.claude/settings.json`): `duck-types` (delegates to
  `dbx-dogfood-parity.sh duck`), `window <n>`, `oracle <n>`, `snapshot <n>`, `manifest`,
  `report`. Default schedule: `START_DATE=2026-08-05`, `CHECKPOINTS=9,10,11` — windows 9-11
  land the three new fixture days (2026-08-13/14/15) on top of the eight phases 5-7b already
  loaded.

## Decisions

- 2026-09-13: kept the per-suite thin wrapper (`check_equivalence` calling
  `check_agreement_against`) rather than extracting it — it's 6 lines of glue over a registry
  and side-labels constant that differ per suite, and `github_activity_dual_target.rs` already
  establishes this as the convention (`check_targets_agree`, `check_databricks_agree`).
- 2026-09-13: `assert_source_covers_window` asserts strict equality (`source_days_loaded ==
  window`), not `<=`. The decision log's premise is "the source's day count equals the window
  number" — a loader that ran behind would be just as invalid an oracle as one that ran ahead,
  so equality is the correct measurement, not merely an upper bound.
- 2026-09-13: `EquivalenceCheckpoint` unifies the two suites' manifest shape by giving
  `source_days_loaded` a serde default of 0, so BigQuery's manifest (which never sets it) is
  unaffected. This is the "no behaviour change to the BigQuery suite" the plan required, and
  it's asserted directly by the regression run below rather than by inspection.

## For the next planner (phase 9b)

- 9b's job per the plan: run the live sequence (`dbx-dogfood-oracle.sh duck-types`, then
  `window`/`oracle`/`snapshot` for 9, 10, 11, then `manifest`, then the live cargo test, then
  `report`), commit `phases/09b-equivalence.json` at
  `EQUIVALENCE_REPORT_PATH` in `github_activity_dbx_oracle.rs`, and flip
  `the_equivalence_report_covers_every_model_at_every_checkpoint`'s loud-skip branch to a hard
  failure (delete the `let Some(report) = ... else { skip }` early return) now that a report
  exists. It should also add a `the_committed_equivalence_report_shows_no_violation`-style hard
  gate and a `equivalence_registry_entries_are_all_live`-style two-sided ratchet, mirroring
  `github_activity_bq_oracle.rs`'s committed-report gates — 9a did not add these because there
  is nothing to check them against yet.
- 9b needs a live OAuth token (one hour) and phases 5-7b's eight already-landed fixture days
  in place; if the workspace is unreachable it must emit `<<PHASE_BLOCKED>>` per the outcome's
  driver note, not skip green.
- 9c depends on 9b's result: if `silver_actor_naming` still duplicates on the full-refresh
  oracle leg, that is evidence the defect is shared with the full refresh rather than being an
  incremental-write-path-only bug (see outcome.md criterion 7's blocker and `## Blocked`).
- Not done in 9a, out of scope for it: nothing else surfaced. The extraction into
  `parity_support` was verified to disturb neither `github_activity_bq_oracle.rs` (14/14) nor
  `github_activity_dual_target.rs` (23/23).

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets,
  shellcheck, full `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-cli --test github_activity_dbx_oracle` — 13/13 passed.
- `cargo test -p smelt-cli --test github_activity_bq_oracle` — 14/14 passed (regression).
- `cargo test -p smelt-cli --test github_activity_dual_target` — 23/23 passed (regression).
- No live workspace touched; nothing in this phase requires one.
