# Phase 10 summary — deploy the loader, run it, measure cost

**Deployed against:** project `smelt-bq-test-20260816`, dataset `smelt_dogfood` (US,
no default table expiration, unchanged), via ADC impersonation of
`smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com`.

## A drift found and fixed before spending anything

`scripts/bq-dogfood-loader.sh --emit-ddl` declared 9 columns; `--emit-sql` selected 10 —
phase 5 re-pinned `sample.sql` to project `payload`, and the phase-9 DDL was written from
the pre-`payload` schema. Verified before touching BigQuery:

```
$ bash scripts/bq-dogfood-loader.sh --emit-ddl | grep -i payload      # (no output)
$ bash scripts/bq-dogfood-loader.sh --emit-sql --date 2026-09-03 | grep -i payload
  payload
  payload
```

Fixed: `payload STRING` added to both `raw.github_events` and `raw.github_events_arrival`
in `--emit-ddl`, in the same position `sample.sql`'s projection puts it (last column,
before the arrival twin's `ingested_date`).

**Gating test added** (red-green): `ddl_columns_match_the_sample_projection` in
`crates/smelt-cli/tests/github_activity_loader.rs`. It parses `sample.sql`'s projection
list (column name = the `AS alias` when present, else the bare identifier) and the DDL's
column list for both tables, and asserts they match exactly (plus `ingested_date` as the
arrival table's one extra trailing column). Run red against the pre-fix DDL first:

```
left:  ["id", "type", "created_at", "actor_id", "actor_login", "repo_id", "repo_name", "org_id", "public"]
right: ["id", "type", "created_at", "actor_id", "actor_login", "repo_id", "repo_name", "org_id", "public", "payload"]
```

then green after the fix:

```
$ cargo test -p smelt-cli --test github_activity_loader
running 12 tests
... (all 12 pass, including the new test)
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## Qualified-name resolution: `raw.` → `smelt_dogfood.`

The loader's emitted SQL says `` `raw.github_events` `` (a two-part `dataset.table`
BigQuery identifier — `raw` names a dataset, not a schema prefix inside one dataset). The
live dogfood dataset is named `smelt_dogfood`, and the service account cannot create a
second dataset (`WRITER` on `smelt_dogfood` only, no dataset-create permission at the
project level) — so a dataset literally named `raw` does not and must not exist here.

**Chosen resolution:** at deploy time (not in the script — the script stays a generic,
project-agnostic derivation of `sample.sql`), I textually substituted `` `raw. `` →
`` `smelt_dogfood. `` in the emitted DDL/SQL before submitting each statement to
BigQuery. This mirrors the same mapping the DuckDB leg already makes for the identical
problem: `models/sources/raw/github_events.yml` is DuckDB-side flattened to
`main.sources_raw_github_events` (one physical schema, path-prefixed table name) rather
than a real `raw` schema. `raw.github_events` in the loader's own output is the *logical*
name smelt's source declaration uses; which physical dataset that logical name resolves
to is an environment/deployment concern, not something the loader script should hard-code
— exactly like the DuckDB target's `schema: main` in `smelt.yml`. I did not invent a
second dataset, and I did not edit the script to hard-code `smelt_dogfood` (that would
wire one specific project into a script meant to be reusable); the substitution lived only
in the one-off deploy commands run against this specific project. A future phase 11+ that
wires `produced_by:`/real orchestration will need to make this same substitution (or an
equivalent target-mapping mechanism) explicit and permanent — flagged as a finding below.

## What was created and loaded

`bq query --project_id=smelt-bq-test-20260816 --use_legacy_sql=false` ran the
`raw.`→`smelt_dogfood.`-substituted `--emit-ddl` output:

```
Created smelt-bq-test-20260816.smelt_dogfood.github_events
Created smelt-bq-test-20260816.smelt_dogfood.github_events_arrival
```

Two separate loader invocations, each dry-run first (REST `queries` endpoint,
`dryRun: true`) to confirm cost before spending:

| Load day (`--date`) | Suffix range scanned | Per-statement dry-run bytes | Rows inserted (each table) | Job | `totalBytesBilled` | `totalSlotMs` |
|---|---|---|---|---|---|---|
| 2026-08-06 | 0805–0806 | 1,734,232,098 (~1.61 GiB) | 2,777 | `dogfood-load-20260806` | 3,468,689,408 | 209,004 |
| 2026-08-05 | 0804–0805 | 1,827,433,334 (~1.70 GiB) | 3,276 | `dogfood-load-20260805` | 3,655,335,936 | 120,309 |

(`totalBytesBilled`/`totalSlotMs` are the whole two-statement script's job stats — BigQuery
runs a multi-statement `bq query` as one parent job with two child jobs; the two INSERTs
scan the same base query independently so per-statement dry-run bytes roughly double for
the combined actual bill.)

I deliberately ran 2026-08-06 before 2026-08-05 (reverse chronological) rather than two
strictly increasing days: later days in the pinned range (checked 2026-08-07: 2.615 GiB/
statement, ~5.23 GB combined; 2026-08-08: ~3.03 GiB/statement) cost measurably more —
GitHub Archive volume trends up through the range — so I picked the two cheapest known-good
days from dry-run measurements rather than the first two calendar days. Every combined
per-day dry run stayed under the ~5 GB stop threshold except 2026-08-07 (~5.23 GB), which I
skipped rather than running. Running day-06 before day-05 means day-06's own redelivery arm
(2% of day-05, `MOD(id,50)=0`) landed *before* day-05's real rows did — this creates real
duplicate rows once day-05's full load lands (see below), which is exactly the intentional
at-least-once behaviour the pipeline's dedup layer exists for, not a bug, but it is a
different code path than "always load in day order" and worth a future run doing the
latter for completeness.

## Verification

**Row counts per partition** (`smelt_dogfood.github_events`, grouped by `DATE(created_at)`):

```
+--------------+-----------+---------------------+
| created_date | row_count | null_payload_count  |
+--------------+-----------+---------------------+
|   2026-08-04 |        75 |                   0 |
|   2026-08-05 |      3264 |                   0 |
|   2026-08-06 |      2714 |                   0 |
+--------------+-----------+---------------------+
```

`payload` arrived non-null on every row across all three partitions.

`INFORMATION_SCHEMA.PARTITIONS` confirms real BigQuery day-partitions (not just a
`GROUP BY`):

```
+-----------------------+--------------+------------+
|      table_name       | partition_id | total_rows |
+-----------------------+--------------+------------+
| github_events         | 20260804     |         75 |
| github_events         | 20260805     |       3264 |
| github_events         | 20260806     |       2714 |
| github_events_arrival | 20260805     |       3276 |
| github_events_arrival | 20260806     |       2777 |
+-----------------------+--------------+------------+
```

`github_events` partitions on `DATE(created_at)` (event time — 2026-08-04 exists only
because day-05's redelivery arm reaches back one day); `github_events_arrival` partitions
on `ingested_date` (arrival time — only the two calendar days the loader actually *ran*
on, 08-05 and 08-06, ever appear, regardless of which event day the rows describe), which
is exactly the event-time-vs-arrival-time partition posture split the outcome's spec
anchors this to.

**Redelivery slice present**, checked directly with the modulus the outcome specifies
(`MOD(id, 50) = 0` of the prior day) rather than inferred from totals:

```sql
SELECT id, COUNT(*) AS copies FROM `smelt_dogfood.github_events`
WHERE DATE(created_at) = DATE '2026-08-05' AND MOD(CAST(id AS INT64), 50) = 0
GROUP BY id HAVING COUNT(*) > 1;
-- 10+ rows returned, each copies = 2
```

Table-wide: 6,053 total rows, 5,990 distinct `id`s → 63 duplicates, which is exactly the
size of day-06's `MOD(id,50)=0` redelivery slice of day-05 (2,777 − 2,714 = 63). The
arithmetic closes: day-05 run inserted 3,276 rows (3,201 real day-05 + 75 redelivered
day-04); day-06's later redelivery of day-05 added 63 more day-05-dated rows
(3,201 + 63 = 3,264, matching the observed count). At-least-once redelivery is real,
present, and exactly the declared modulus — not a hand-wave.

**Retention read back from table metadata** (REST `tables.get`, not assumed from the DDL
text):

```
github_events:          timePartitioning.expirationMs = 3888000000  (= 45.0 days)
github_events_arrival:  timePartitioning.expirationMs = 3888000000  (= 45.0 days)
expirationTime (table-level, both tables): unset
```

`3,888,000,000 ms / 86,400,000 ms/day = 45` — matches `README.md`'s documented
`partition_expiration_days = 45` exactly (the same number the DDL test parses from the
README, now verified against the *deployed* table, not just the emitted DDL text). No
table-level `expirationTime` is set on either table, preserving criterion 1's "no default
table expiration" as a dataset-only property. Configuration read-back only — no partition
was aged out or forced to expire.

## Cost per run

| Job | `totalBytesBilled` | GB | Cost @ $5/TB |
|---|---|---|---|
| `dogfood-load-20260806` | 3,468,689,408 | 3.469 | $0.0173 |
| `dogfood-load-20260805` | 3,655,335,936 | 3.655 | $0.0183 |
| **Average per run** | — | 3.562 | **$0.0178** |

Extrapolated to one run/day for a month: **$0.0178 × 30 ≈ $0.53/month** at these two
sampled days. Using the heaviest day dry-run observed in the pinned range instead
(2026-08-08, ~3.03 GiB/statement, ~6.06 GB combined, ~$0.030/run) as a conservative
upper bound: **~$0.91/month**. Either figure is a small fraction of the project's AUD
25/month budget (≈ USD 16–17 at current rates) — the loader alone does not threaten the
budget even before its 1 TiB/month BigQuery on-demand free tier is applied (which would
make the realized bill $0 in practice at this volume).

## Findings for `docs/outcomes/20260906-bigquery-correctness`

1. **The `raw.` → `smelt_dogfood.` (or any real dataset) qualified-name mapping has no
   home yet.** Today it's a manual `sed` substitution I ran by hand at deploy time. Before
   phase 11's live model run or any real orchestration, this needs a defined mechanism —
   most likely the same target-mapping smelt already does for the DuckDB `schema: main`
   case, extended to BigQuery's project/dataset pair — rather than continuing as an
   undocumented manual step. Not fixed here per this outcome's "Out of scope."
2. **`--emit-ddl`'s `payload` omission was silent until this phase.** The phase-9 test
   suite proved the *INSERT* projection matched `sample.sql` byte-for-byte but never
   checked the DDL's column list against it. Fixed here with
   `ddl_columns_match_the_sample_projection`, but flagging the underlying pattern
   (multiple artifacts independently deriving a schema from one source of truth, only one
   of them gated) as worth a general sweep — are there other DDL/projection pairs in this
   repo with the same silent-drift shape?
3. **Load order matters more than the loader's own tests exercise.** Running day-06 before
   day-05 (chosen here purely for cost) produced correct, expected duplicate rows via the
   redelivery mechanism, but the loader's per-PR test suite (phase 9) only exercises
   `--emit-sql` for a single `--date` in isolation — nothing asserts what happens across
   *two* invocations in either order. The live run here is reassuring evidence but not a
   gate; consider a phase-12-adjacent test that stages two loader invocations (in both
   orders) against a scratch dataset, if that becomes cheap enough to run per-PR.

## Gates

- `cargo test -p smelt-cli --test github_activity_loader` — 11 passed.

**Orchestrator follow-up (not the implementer's work):** the count is 11 rather than the
12 this phase left behind, because `loader_script_is_shellcheck_clean` was **deleted**.
Its "shellcheck isn't on this box, skipping" is out of date — shellcheck is pinned in
`mise.toml`'s `[tools]` as of 2026-09-09 and the implementer simply ran `cargo test`
outside `mise exec`. More to the point the test was strictly weaker than what replaced
it: it linted one script and returned **green** when the tool was missing, which reads
exactly like a pass. `.claude/scripts/shellcheck-gate.sh` lints all 62 scripts (this one
among them), fails rather than skips when shellcheck is absent, and runs in both
`verify-phase.sh` and the CI Lint job.
- `bash .claude/scripts/verify-phase.sh` — see the session's final report for the tail
  (fmt, clippy both feature sets, full workspace tests, example_diagnostics, and the new
  shellcheck-over-`scripts/`+`.claude/scripts/` gate).
- Manual, live BigQuery: DDL applied, two loader runs executed, row counts / partitions /
  retention / redelivery all verified by direct query against `smelt_dogfood` as shown
  above.

## Not done / blocked

Nothing was blocked. Everything in the phase brief's task list (1–6) was completed. Day
2026-08-07 was dry-run-priced and deliberately skipped as a *load* day (combined ~5.23 GB,
over the phase's own ~5 GB stop-and-report threshold) — reported here rather than treated
as a blocker, since two cheaper days from the same pinned range satisfied "at least two
days loaded" without it.
