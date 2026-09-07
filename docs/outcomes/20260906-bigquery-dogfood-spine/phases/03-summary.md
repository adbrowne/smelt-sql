# Phase 3 summary — succession on the real rename stream

## Shipped

- `examples/github_activity/models/sources/raw/github_events_arrival.yml` — the
  arrival-partitioned twin source (`raw.github_events_arrival`, same columns plus
  `ingested_date`).
- `examples/github_activity/models/silver/repo_naming.sql` — succession grain over
  `raw.github_events` (event-time-partitioned posture).
- `examples/github_activity/models/silver/actor_naming.sql` — same shape over
  `raw.github_events_arrival` (arrival-partitioned posture).
- `examples/github_activity/models/marts/naming_history.sql` — `LAG`-derived real renames,
  `refresh: full`.
- `setup_sources.sql` / `run_incremental.py` / `crates/smelt-cli/tests/
  github_activity_replay.rs`'s harness extended to stage and replay the arrival relation
  alongside the event-time one.
- 5 new tests in `github_activity_replay.rs`: grain recognition (both postures), the
  redelivery fold-once invariant, the fixture's measured same-second ties (139
  repo / 145 actor), and `naming_history`'s 34 renamed repos / 4 renamed actors / the
  owner-change row / the 2 reused repo names. `full_refresh_matches_incremental_replay`
  extended to cover all three new models.
- `examples/github_activity/README.md` — new "The rename stream" section.

## Decisions

- **`created_at` must be projected verbatim, not aliased.** The succession-patch
  executor (`crates/smelt-runtime/src/maintenance_driver/succession/execute.rs`) resolves
  the clock column's type from the model's own output schema by name; aliasing it away
  (as `examples/scd2_succession/models/customer_history.sql` does with `effective_ts AS
  valid_from`) fails at run time with "clock column has no resolved output type". Fixed
  in this phase's models, not in the shared executor or the pre-existing example — see
  "For the next planner".
- **Redelivery clause needed its own arrival-relation variant.** Reusing the event-time
  `UNION ALL` clause verbatim inside the arrival-relation INSERT produced a column-count
  mismatch (the arrival relation has one extra column). Both the Python driver and the
  Rust test harness now carry a separate `redelivery_arrival` clause that adds
  `ingested_date` to the redelivered leg too.

## For the next planner

- **A genuine full-refresh/incremental divergence for succession models with ties,
  recorded not fixed.** `silver.repo_naming`/`silver.actor_naming` do not satisfy
  criterion 7's equivalence at the raw row-count level: the incremental window-forward
  patch loop folds same-`(key, clock)` rows (its `MERGE ... ON` addressing), but
  `--full-refresh`'s `emit_succession_full_rebuild` re-runs the model's raw compiled
  `SELECT` with no such addressing, keeping every physically duplicated row. Measured
  exactly: 139 extra rows for `repo_naming`, 145 for `actor_naming`, matching the
  fixture's own same-second tie counts. `marts.naming_history` is unaffected (its `LAG`
  filter drops the duplicate identically on both legs), so this is confined to the two
  silver tables' row counts, not to any business-facing answer. I explored fixing the
  emitter (`SELECT DISTINCT *` wrapping the model SELECT before the presented `CREATE
  TABLE AS`) and found it insufficient: `LEAD`/`LAG` computed over an un-deduped source
  gives tied rows *different* computed values (arbitrary tie-break in the window's
  `ORDER BY`), so exact-row dedup only catches byte-identical duplicates (this alone
  closed 50 of 139 repo rows) — the general fix needs to fold on `(key_cols, clock_col)`
  with an aggregate (e.g. `MAX`) over every other column, which needs the full output
  schema threaded into `emit_succession_full_rebuild` and touches its statement-parity
  test fixtures (`crates/smelt-runtime/tests/statement_parity/succession.rs`) and every
  other succession consumer. That is real, well-scoped work belonging to
  `docs/outcomes/20260906-scd2-keyed-succession`'s decision log, not this pipeline — I
  reverted the partial fix rather than ship an incomplete production change. Also worth
  noting: this rebuild path has never run the clock-tie probe at all, so a
  content-*disagreeing* tie would silently corrupt a full-refresh today; the fixture
  measured zero disagreeing ties, so this stays undetected rather than exercised.
- **The pre-existing `examples/scd2_succession/models/customer_history.sql` would hit the
  same "clock column has no resolved output type" failure if ever executed for real** — it
  aliases `effective_ts AS valid_from` and is currently only exercised through
  `explain_maintenance/succession.rs` (a static plan-report check) and the generative
  `maintenance_conformance` suite (which projects the clock column unaliased, per
  `SuccessionRecipe::new_lead`'s `(source.clock_column, source.clock_column)` projection),
  never through an actual `smelt run`. Worth an explicit real-execution test or a fix to
  either the example or the executor.
- Criterion 9's remaining un-probed case — the owner-change row
  (`mikiKG45/noob-devops-project` → `guslariR45/noob-devops-project`) — is now covered:
  `naming_history_surfaces_the_real_renames` asserts it appears.

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test github_activity_replay` — 9/9 pass.
- `cargo test -p smelt-lsp --test example_workspaces github_activity` — pass.
- `cargo test -p smelt-cli --test example_diagnostics` — pass (`github_activity_no_diagnostics`).
- `python3 examples/github_activity/run_incremental.py` — full 30-day replay + `smelt test`, pass.
