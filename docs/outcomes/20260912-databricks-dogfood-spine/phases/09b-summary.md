# Phase 9b summary — the equivalence invariant, measured live on Databricks

## Shipped

- `docs/outcomes/20260912-databricks-dogfood-spine/phases/09b-equivalence.json` and `.md`:
  the committed report over checkpoints w09/w10/w11 (`2026-08-13/14/15`), 16 relations each.
- `crates/smelt-cli/tests/github_activity_dbx_oracle.rs`: `equivalence_report()` now hard-fails
  on a missing report (loud-skip branch deleted); two new gates land per the plan —
  `the_committed_report_proves_the_oracle_read_the_shared_source` (per-checkpoint
  `source_days_loaded == window` plus non-zero rows both sides) and
  `the_final_window_compares_every_relation_with_nothing_exempt` (window 11, all 16 relations,
  `scope == "compared"`). 15/15 offline.
- `scripts/dbx-dogfood-oracle.sh`: two bugs fixed to make the sequence runnable at all —
  `stage_window` passed the loader `--day`, which the loader has never accepted (it takes
  `--date`); `stage_oracle` needs `--allow-full-refresh` because, unlike the BigQuery oracle
  (which drops and recreates a scratch dataset), this oracle re-runs `--full-refresh` over the
  SAME `smelt_dogfood_oracle` tables every checkpoint, so from the second checkpoint on
  `docs/specs/sources.md` §Semantics 5's retention gate refuses the whole-table recompute
  without an explicit operator license (harmless here — 11 days deep is far inside the
  sources' 45-day retention).

## Decisions

- 2026-09-13: fixed both script bugs above rather than recording them — the sequence could
  not complete a single window without them, matching the outcome's own "only fix what's
  needed for a run to complete at all" exception.
- 2026-09-13: cleaned a stale `examples/github_activity/.smelt/` and `target/` (gitignored,
  local-only) before `duck-types` would run — a prior local DuckDB replay's recorded source
  postures conflicted with a fresh `--first-full-refresh` over 11 days. This is purely local
  observability state (`docs/specs/run_state.md`: `intervals.json`/`landed_deltas.json` are
  project-wide *observability*, never correctness structure — the reconciliation ledger that
  IS correctness structure is engine-resident, in Unity Catalog, untouched by this). Deleting
  it lost nothing live and cost nothing to regenerate.
- 2026-09-13: per the plan's contingency, did **not** register `gold_events_enriched`'s
  divergence in this suite's own `EQUIVALENCE_DIVERGENCE_REGISTRY` even though it is the same,
  already root-caused defect row 8 registered in the dual-target suite. The plan's contingency
  text is written generally ("if any relation violates... tests 5-6 move into 9c's scope"),
  not scoped to `silver_actor_naming` alone, so tests 5/6 were left off this phase and the
  decision of whether/how to register is left to 9c or a later phase.

## Measured result (the deliverable)

All three checkpoints, 16 relations each:

- **`silver_actor_naming` matches exactly at every checkpoint** (`incr_only=0, oracle_only=0`
  at w09/w10/w11) — **this is the decisive evidence phase 9c needs.** Databricks' own
  full-refresh oracle does **not** duplicate `(actor_id, created_at)` rows the way row 8's
  dual-target sweep found the *incremental* leg duplicating against DuckDB (520 extra rows).
  Since the oracle (a `--full-refresh`, not the succession-patch technique) agrees with the
  incremental leg exactly, the 520-row duplication is **not** reproduced by a full refresh on
  the same engine — it is specific to the incremental write path
  (`crates/smelt-runtime/src/maintenance_driver/succession/execute.rs`'s tombstone mechanism),
  not a defect in the model's SQL under SparkSQL. This points 9c at `## Blocked`'s options 1 or
  3 (a write-path bug), not option 2 (an accepted, both-legs-see-it duplication) or a model-SQL
  fix.
- **`gold_events_enriched` violates at every checkpoint**: w09 `9/9`, w10 `10/10`, w11 `16/16`
  (`incr_only`/`oracle_only`, growing as more fixture days carry more repo renames). This is
  the SAME divergence row 8 root-caused and registered in the dual-target suite as
  `DivergenceBound::UnorderedColumnDivergence` on `current_repo_name` — the `EnrichmentKeyed`→
  `DeleteInsert` downgrade (phase 7b) sacrifices the unwindowed heal a full refresh always
  performs. Not new; not investigated further here.
- Every other relation (`bronze_events`, `gold_repo_activity_daily`, `gold_repo_dim`, both
  `marts_*` families, every other `silver_*`) matches exactly at all three checkpoints.

Per the plan's contingency (not clean → do not register or fix, commit evidence, land tests
2-4 only, block, point at 9c): a `## Blocked` entry is added to `outcome.md` naming both
violating relations and this evidence.

## Free Edition cost/latency observations (for criterion 4's facts sheet)

- Three full-refresh-with-`--allow-full-refresh` runs (oracle 9/10/11) took 84s/111s/141s;
  three incremental runs (window 9/10/11) took 84s/67s/62s — oracle cost grows with
  accumulated data as expected, incremental cost does not.
- `snapshot`'s exports each hit one `[INVALID_HANDLE.SESSION_CLOSED]` warning (non-fatal,
  matches phase 4c's own observation of single-digit-second serverless session teardown) —
  the export script recovers and reads correctly regardless.

## For the next planner (phase 9c)

- 9c's decisive question is answered: `silver_actor_naming`'s duplication is confined to the
  incremental write path (succession-patch tombstone mechanism), not shared with the full
  refresh. Route 1 or 3 in `## Blocked`, not route 2.
- `gold_events_enriched`'s divergence recurs here identically to row 8; 9c (or a later phase)
  should decide whether to also register it in `EQUIVALENCE_DIVERGENCE_REGISTRY` here, since
  it is understood and bounded, not an open question — this phase deliberately left that
  decision open rather than making the call itself.
- Not done, out of scope for this phase: root-causing `succession/execute.rs`'s write path
  itself (9c's job), and the Catalog-Commits/cross-table-transaction research question this
  outcome's 2026-09-13 research note already flagged for planner triage.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets,
  shellcheck, full `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-cli --test github_activity_dbx_oracle` — 15/15 (tests 2-4 landed per
  plan; 5-6 deliberately not landed — sweep not clean).
- `cargo test -p smelt-cli --test github_activity_bq_oracle` — 14/14 (regression, unchanged).
- `cargo test -p smelt-cli --test github_activity_dual_target` — 23/23 (regression,
  unchanged).
- `bash .claude/scripts/shellcheck-gate.sh` — PASS (76 scripts, zero findings).
- Live workspace touched: 3 windows loaded/run, 3 oracle full refreshes, 3 read-only snapshot
  exports, all against `workspace.smelt_dogfood`/`smelt_dogfood_oracle`.
