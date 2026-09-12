# Phase 7 plan — three consecutive incremental windows on Databricks

## Objective

Advance criterion 6's second half: after the full refresh row 6f left, drive **three
consecutive non-empty incremental windows** against `workspace.smelt_dogfood`, capture the run
report for each, and inspect the interval frontier and the engine-resident tables between runs.
Leaves a Databricks side rows 8 (parity) and 9 (oracle) can read, and inventories every
incremental-path refusal for row 7b. Per the outcome's "## Out of scope", a defect is
characterised (model, statement, `file:line`, verbatim error) and recorded — **not fixed**,
unless a run cannot complete at all without it.

## Spec delta

None. This phase runs the product as committed and records what it does.

## The windows, derived from live state rather than assumed

Read back this session (`scripts/dbx-query.sh`, `GROUP BY` per partition column):

| table | partition column | days present (rows) |
|---|---|---|
| `github_events` | `to_date(created_at)` | 2026-08-05 (3,264), 2026-08-06 (2,714) |
| `github_events_arrival` | `ingested_date` | 2026-08-05 (3,201), 2026-08-06 (2,777) |

Unlike the BigQuery spine — where a loader run re-scanned `githubarchive` and the windows were
therefore empty of new rows — **this loader replays a local Parquet fixture**
(`examples/github_activity/seeds/github_events_sample.parquet`, which holds 2026-08-05 through
2026-08-20) through the backend's Arrow path. Landing a further day costs one local read plus
one serverless write, so every window here carries real new rows:

| step | loader | `smelt run` window | new event rows |
|---|---|---|---|
| FR | — | *already done* — run `20260912-124833-91da82`, event time 08-05→08-07, 14 success / 1 failed / 1 skipped | — |
| W1 | `--date 2026-08-07` | `--event-time-start 2026-08-07 --event-time-end 2026-08-08` | 2,334 |
| W2 | `--date 2026-08-08` | `--event-time-start 2026-08-08 --event-time-end 2026-08-09` | 4,086 |
| W3 | `--date 2026-08-09` | `--event-time-start 2026-08-09 --event-time-end 2026-08-10` | 2,597 |

No new full refresh is run: nothing under `crates/` has changed since `20260912-124833-91da82`,
its `intervals.json` coverage is exactly `2026-08-05 → 2026-08-07` on every model, and a
re-refresh would both cost serverless time and risk the `SourceRetentionExceeded` admission the
BigQuery spine hit. `--event-time-end` is exclusive, so each row above is one calendar day and
each window starts exactly where the previous coverage ended.

Each loader run also lands its ~2% redelivery reach-back (previous day's `created_at`, current
`ingested_date`), so the arrival-time models see genuine late arrival inside the window —
whether an event-time window picks those rows up is an **inspection question** for this phase,
recorded either way, not something to fix here.

## Known blocker carried in, not fixed

`gold.events_enriched` fails with `Feature not supported by Spark SQL: key-addressed model-edge
affected-key discovery over a KeyedUpsert upstream (group-grain fingerprint-sidecar diff)` —
`crates/smelt-runtime/src/maintenance_driver/key_addressed/mod.rs:124`, gated on
`BackendCapabilities::supports_fingerprint_sidecar`, which is `false` for Spark/Delta
(`crates/smelt-dialect/src/dialect.rs:295`). `marts.star_growth` skips as its dependent. This
is a maintenance-layer capability gap, on the incremental path as much as the full-refresh one,
so it is expected to recur in all three windows. 14/16 still complete, so the
"only-fix-what-is-needed-to-complete-at-all" exception does **not** apply: row 7b owns it.

## Tests

No new Rust tests — no production code is edited. The evidence is the captured artifacts, and
the assertions this phase makes on them (task 5) are written into the summary with the numbers
that back them.

## Tasks

1. Baseline the live state: `SHOW TABLES IN workspace.smelt_dogfood` with per-model row counts,
   and the two source tables' per-day counts (recorded above; re-confirm immediately before W1).
2. W1 — `bash scripts/dbx-dogfood-loader.sh --date 2026-08-07`, verify the day's count and its
   reach-back slice against the fixture's own DuckDB-computed counts, then
   `smelt run --target databricks --event-time-start 2026-08-07 --event-time-end 2026-08-08`.
3. Inspect after W1: console transcript verbatim; the run report
   (`examples/github_activity/.smelt/targets/databricks/reports/<run_id>.json`) —
   which models `success`/`skipped`/`failed`; how `intervals.json` coverage advanced per model
   (expect `2026-08-05 → 2026-08-08`, and name any model whose coverage did *not* advance);
   `landed_deltas.json`; and the engine-resident table list with row counts, checked against
   what the window's 2,334 new rows predict.
4. Repeat tasks 2-3 for W2 (`--date 2026-08-08`, window 08-08→08-09) and W3 (`--date
   2026-08-09`, window 08-09→08-10).
5. Assert the engine-resident state explicitly rather than by omission: with
   `supports_fingerprint_sidecar: false` on Delta, say which sidecar/ledger/tombstone sibling
   tables exist in `workspace.smelt_dogfood` and which are absent by declaration
   (`docs/specs/state.md` §"Which dialects realise which structure"), so row 9's oracle knows
   what it is comparing against.
6. Record the serverless cost facts this phase consumed (wall time per window, cold-start
   latency) against `free-edition-facts.md`'s quota table — Free Edition carries no bill, so
   time and concurrency are the budget.
7. Characterise every refusal and failure: model, statement, `file:line`, verbatim error. If a
   new one stops all three windows from running at all, stop and mark the phase `blocked` with
   that gate named rather than pressing on.
8. Write `phases/07-summary.md` self-contained per finding (row 10 harvests it), update
   `outcome.md` (row 7 status + dated decision-log entry, and row 7b's wording sharpened with
   what the windows actually recorded), commit and push.

## Verification

- `bash .claude/scripts/verify-phase.sh` — expected green and unchanged; no production code is
  edited, so this is a no-regression check, stated as such.
- Live: three run reports under `.smelt/targets/databricks/reports/`, each with its window's
  model verdicts, and `intervals.json` coverage ending at `2026-08-10`.
- `bash scripts/dbx-verify.sh` green before W1 (already confirmed green this session).

## Commit message

`outcome(databricks-dogfood-spine): phase 7 drives three incremental windows on Databricks`
