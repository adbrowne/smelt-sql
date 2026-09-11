# Phase 17 summary — the BigQuery population now *is* the committed fixture

**Executed live** against project `smelt-bq-test-20260816`, dataset `smelt_dogfood`, via ADC
impersonation of `smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com`.

**Result in one line:** 28 days loaded with `scripts/bq-dogfood-loader.sh`'s own emitted SQL,
every day's inserted row count equal to the fixture's expectation on the first attempt; the
75-row 2026-08-04 residue deleted from both tables; and all four D3 acceptance checks pass
with **zero** differences — `githubarchive` and the committed fixture agree on all 64,313
rows, including a full (not sampled) `payload` digest comparison. Total cost **US$0.45**.

## 1 — Dry run first (task 1)

All 28 days, both statements each, `dryRun: true` on the REST `queries` endpoint before a
single byte was spent. `totalBytesProcessed`, per statement and per day:

| Load day | per-statement bytes | day total | running total |
|---|---:|---:|---:|
| 2026-08-07 | 2,615,002,489 | 5,230,004,978 | 5,230,004,978 |
| 2026-08-08 | 3,027,523,846 | 6,055,047,692 | 11,285,052,670 |
| 2026-08-09 | 2,370,685,403 | 4,741,370,806 | 16,026,423,476 |
| 2026-08-10 | 2,081,995,089 | 4,163,990,178 | 20,190,413,654 |
| 2026-08-11 | 2,076,209,658 | 4,152,419,316 | 24,342,832,970 |
| 2026-08-12 | 2,341,996,324 | 4,683,992,648 | 29,026,825,618 |
| 2026-08-13 | 2,362,533,227 | 4,725,066,454 | 33,751,892,072 |
| 2026-08-14 | 2,289,825,540 | 4,579,651,080 | 38,331,543,152 |
| 2026-08-15 | 2,237,956,836 | 4,475,913,672 | 42,807,456,824 |
| 2026-08-16 | 2,191,654,740 | 4,383,309,480 | 47,190,766,304 |
| 2026-08-17 | 1,968,656,293 | 3,937,312,586 | 51,128,078,890 |
| 2026-08-18 | 1,586,218,846 | 3,172,437,692 | 54,300,516,582 |
| 2026-08-19 | 1,126,273,591 | 2,252,547,182 | 56,553,063,764 |
| 2026-08-20 | 655,339,585 | 1,310,679,170 | 57,863,742,934 |
| 2026-08-21 | 825,151,870 | 1,650,303,740 | 59,514,046,674 |
| 2026-08-22 | 1,515,535,426 | 3,031,070,852 | 62,545,117,526 |
| 2026-08-23 | 2,035,759,905 | 4,071,519,810 | 66,616,637,336 |
| 2026-08-24 | 1,902,896,570 | 3,805,793,140 | 70,422,430,476 |
| 2026-08-25 | 1,407,640,963 | 2,815,281,926 | 73,237,712,402 |
| 2026-08-26 | 1,302,516,941 | 2,605,033,882 | 75,842,746,284 |
| 2026-08-27 | 1,287,229,540 | 2,574,459,080 | 78,417,205,364 |
| 2026-08-28 | 1,048,926,298 | 2,097,852,596 | 80,515,057,960 |
| 2026-08-29 | 914,369,045 | 1,828,738,090 | 82,343,796,050 |
| 2026-08-30 | 898,825,222 | 1,797,650,444 | 84,141,446,494 |
| 2026-08-31 | 904,854,113 | 1,809,708,226 | 85,951,154,720 |
| 2026-09-01 | 759,645,787 | 1,519,291,574 | 87,470,446,294 |
| 2026-09-02 | 507,919,893 | 1,015,839,786 | 88,486,286,080 |
| 2026-09-03 | 991,472,162 | 1,982,944,324 | 90,469,230,404 |

**Projected total: 90,469,230,404 bytes = 90.47 GB ≈ US$0.45.** Well under the plan's
250 GB stop gate, so the load proceeded.

The shape phase 10 observed holds: GitHub Archive volume peaks around 2026-08-08 and falls
away through late August, and the two statements of a day are byte-identical in cost because
each `INSERT` re-scans the same base query independently.

## 2 — The load (task 2)

Per D1, the SQL was `scripts/bq-dogfood-loader.sh --emit-sql --date <D>` with phase 10's one
substitution, `` `raw. `` → `` `smelt_dogfood. ``, applied at deploy time only. No load SQL
was authored for this phase. The two `INSERT`s were submitted as separate jobs (rather than
one multi-statement script) purely so that `numDmlAffectedRows` is readable per statement —
the SQL text is the loader's, unchanged.

Days ran in strict calendar order, 2026-08-07 → 2026-09-03. Before each next day, both
statements' affected-row counts were checked against the fixture's expectation

    expected(D) = fixture_rows(D) + fixture_rows(D-1 where MOD(CAST(id AS BIGINT),50)=0)

A mismatch would have stopped the sequence. **None did** — all 56 statements returned
exactly the expected count.

| Load day | expected | rows (events) | rows (arrival) | billed bytes, per statement |
|---|---:|---:|---:|---:|
| 2026-08-07 | 2,388 | 2,388 | 2,388 | 2,615,148,544 |
| 2026-08-08 | 4,132 | 4,132 | 4,132 | 3,028,287,488 |
| 2026-08-09 | 2,679 | 2,679 | 2,679 | 2,370,830,336 |
| 2026-08-10 | 2,955 | 2,955 | 2,955 | 2,082,471,936 |
| 2026-08-11 | 4,310 | 4,310 | 4,310 | 2,077,229,056 |
| 2026-08-12 | 3,778 | 3,778 | 3,778 | 2,342,518,784 |
| 2026-08-13 | 3,691 | 3,691 | 3,691 | 2,363,490,304 |
| 2026-08-14 | 3,421 | 3,421 | 3,421 | 2,290,089,984 |
| 2026-08-15 | 4,063 | 4,063 | 4,063 | 2,238,709,760 |
| 2026-08-16 | 3,516 | 3,516 | 3,516 | 2,192,572,416 |
| 2026-08-17 | 2,899 | 2,899 | 2,899 | 1,969,225,728 |
| 2026-08-18 | 2,127 | 2,127 | 2,127 | 1,586,495,488 |
| 2026-08-19 | 767 | 767 | 767 | 1,127,219,200 |
| 2026-08-20 | 566 | 566 | 566 | 655,360,000 |
| 2026-08-21 | 748 | 748 | 748 | 825,229,312 |
| 2026-08-22 | 1,687 | 1,687 | 1,687 | 1,516,240,896 |
| 2026-08-23 | 2,778 | 2,778 | 2,778 | 2,036,334,592 |
| 2026-08-24 | 1,564 | 1,564 | 1,564 | 1,903,165,440 |
| 2026-08-25 | 1,146 | 1,146 | 1,146 | 1,408,237,568 |
| 2026-08-26 | 2,434 | 2,434 | 2,434 | 1,303,379,968 |
| 2026-08-27 | 2,190 | 2,190 | 2,190 | 1,287,651,328 |
| 2026-08-28 | 1,369 | 1,369 | 1,369 | 1,049,624,576 |
| 2026-08-29 | 951 | 951 | 951 | 915,406,848 |
| 2026-08-30 | 823 | 823 | 823 | 899,678,208 |
| 2026-08-31 | 1,584 | 1,584 | 1,584 | 904,921,088 |
| 2026-09-01 | 843 | 843 | 843 | 760,217,600 |
| 2026-09-02 | 51 | 51 | 51 | 508,559,360 |
| 2026-09-03 | 145 | 145 | 145 | 991,952,896 |

Load order was strictly increasing this time, unlike phase 10's deliberate reversal — so the
D-1 redelivery arm always lands *after* the real day it duplicates, which is the ordering the
DuckDB replay driver also uses.

## 3 — The deletion (task 3)

The only deletion in this phase, by explicit predicate on each table. No table was dropped or
truncated.

```
DELETE FROM `smelt_dogfood.github_events`          WHERE CAST(created_at AS DATE) = DATE '2026-08-04'
  -> {"rows":75,"billed":0,"job":"job_pLok_BRiyQ1lOI-aLut7cDOHvppA"}
DELETE FROM `smelt_dogfood.github_events_arrival`  WHERE CAST(created_at AS DATE) = DATE '2026-08-04'
  -> {"rows":75,"billed":10485760,"job":"job_YUv_-P_J-cn7nk7eAu5_Lids2sP-"}
```

75 removed from each, as predicted. Read back from `INFORMATION_SCHEMA.PARTITIONS`: the
`20260804` partition is gone from `github_events` entirely, the day range now starts at
`20260805`, and nothing else changed shape.

```
table_name             partitions  total_rows  min_p      max_p
github_events                  31       65583   20260805  __NULL__
github_events_arrival          30       65583   20260805  20260903
```

`github_events`'s 31st entry is the `__NULL__` pseudo-partition, holding **0 rows** (checked
directly) — an empty catalogue entry, not data. The arrival twin's oldest partition
(`ingested_date = 20260805`) dropped from 3,276 to 3,201 rows, which is exactly the 75
event-time-08-04 rows that arrived on load day 08-05.

The deletion is reversible: re-running the loader for 2026-08-05 restores those 75 rows.

## 4 — Acceptance checks (task 4, D3)

### (1) Per-day counts, BigQuery vs the fixture — **30/30 days match, none missing, none extra**

The expectation is *not* a plain equality with the fixture's per-day count, because the table
deliberately carries redelivered duplicates:

    bq_rows(D) = fixture_rows(D) + fixture_redeliv(D)   for D in 2026-08-05 … 2026-09-02
    bq_rows(D) = fixture_rows(D)                        for D = 2026-09-03 (no load day 09-04 ran)

and the *distinct* id count per day should equal the fixture's row count per day exactly.

| day | fixture rows | BQ distinct ids | BQ rows | expected BQ rows |
|---|---:|---:|---:|---:|
| 2026-08-05 | 3,201 | 3,201 | 3,264 | 3,264 |
| 2026-08-06 | 2,714 | 2,714 | 2,768 | 2,768 |
| 2026-08-07 | 2,334 | 2,334 | 2,380 | 2,380 |
| 2026-08-08 | 4,086 | 4,086 | 4,168 | 4,168 |
| 2026-08-09 | 2,597 | 2,597 | 2,647 | 2,647 |
| 2026-08-10 | 2,905 | 2,905 | 2,960 | 2,960 |
| 2026-08-11 | 4,255 | 4,255 | 4,339 | 4,339 |
| 2026-08-12 | 3,694 | 3,694 | 3,766 | 3,766 |
| 2026-08-13 | 3,619 | 3,619 | 3,687 | 3,687 |
| 2026-08-14 | 3,353 | 3,353 | 3,412 | 3,412 |
| 2026-08-15 | 4,004 | 4,004 | 4,083 | 4,083 |
| 2026-08-16 | 3,437 | 3,437 | 3,513 | 3,513 |
| 2026-08-17 | 2,823 | 2,823 | 2,877 | 2,877 |
| 2026-08-18 | 2,073 | 2,073 | 2,121 | 2,121 |
| 2026-08-19 | 719 | 719 | 731 | 731 |
| 2026-08-20 | 554 | 554 | 565 | 565 |
| 2026-08-21 | 737 | 737 | 756 | 756 |
| 2026-08-22 | 1,668 | 1,668 | 1,697 | 1,697 |
| 2026-08-23 | 2,749 | 2,749 | 2,817 | 2,817 |
| 2026-08-24 | 1,496 | 1,496 | 1,526 | 1,526 |
| 2026-08-25 | 1,116 | 1,116 | 1,143 | 1,143 |
| 2026-08-26 | 2,407 | 2,407 | 2,449 | 2,449 |
| 2026-08-27 | 2,148 | 2,148 | 2,199 | 2,199 |
| 2026-08-28 | 1,318 | 1,318 | 1,343 | 1,343 |
| 2026-08-29 | 926 | 926 | 935 | 935 |
| 2026-08-30 | 814 | 814 | 828 | 828 |
| 2026-08-31 | 1,570 | 1,570 | 1,596 | 1,596 |
| 2026-09-01 | 817 | 817 | 834 | 834 |
| 2026-09-02 | 34 | 34 | 34 | 34 |
| 2026-09-03 | 145 | 145 | 145 | 145 |

```
days in fixture: 30 days in bq: 30 mismatching days: 0
bq total rows: 65583 bq total distinct-per-day: 64313 fixture total: 64313
```

(2026-09-02's 34 rows and 2026-09-03's 145 are genuinely that thin in `githubarchive` under
the `MOD(repo.id, 1000) = 0` sample — the fixture and the warehouse agree on them, so this is
a property of the upstream shards, not a truncated load.)

### (2) Id sets — **identical in both directions**

```
SELECT COUNT(*) AS rows_total, COUNT(DISTINCT id) AS distinct_ids, COUNTIF(payload IS NULL)
  -> rows_total=65583  distinct_ids=64313  null_payload=0
```

64,313 distinct ids against the fixture's 64,313 distinct ids. The full id sets were compared
by exporting BigQuery's and diffing locally:

```
ids only in fixture: 0 []
ids only in bq     : 0 []
common ids: 64313
```

### (3) Row content on a checksummed column subset — **0 rows differ**

Rather than sample `payload`, it was compared in **full**: the BigQuery side emitted
`TO_HEX(MD5(payload))` and `LENGTH(payload)` per row, the DuckDB side `md5(payload)` and
`length(payload)`, and the two dumps were diffed row by row on

`(id, created_at, type, actor_id, repo_id, repo_name, payload_len, payload_md5)`

with `created_at` rendered as `%Y-%m-%d %H:%M:%S` UTC on both sides and the integer columns
cast to strings so the comparison is textual and exact. No sampling rule was needed.

```
fixture rows  : 64313
bq rows       : 64313
common ids: 64313
rows differing on the checksummed subset: 0
per-column difference counts: {}
```

This is the strongest form of the check D3 asked for, and it closes the reproducibility
question the plan flagged: **`githubarchive` has not drifted from the committed fixture.**
Nothing to escalate.

### (4) Redelivery slices — **28/28 correct, and the two absences are correct too**

From the arrival twin, `WHERE DATE(created_at) < ingested_date`, grouped by load day:

| load day | event day | rows | fixture `MOD(id,50)=0` count for that event day |
|---|---|---:|---:|
| 2026-08-06 | 2026-08-05 | 63 | 63 |
| 2026-08-07 | 2026-08-06 | 54 | 54 |
| 2026-08-08 | 2026-08-07 | 46 | 46 |
| 2026-08-09 | 2026-08-08 | 82 | 82 |
| 2026-08-10 | 2026-08-09 | 50 | 50 |
| 2026-08-11 | 2026-08-10 | 55 | 55 |
| 2026-08-12 | 2026-08-11 | 84 | 84 |
| 2026-08-13 | 2026-08-12 | 72 | 72 |
| 2026-08-14 | 2026-08-13 | 68 | 68 |
| 2026-08-15 | 2026-08-14 | 59 | 59 |
| 2026-08-16 | 2026-08-15 | 79 | 79 |
| 2026-08-17 | 2026-08-16 | 76 | 76 |
| 2026-08-18 | 2026-08-17 | 54 | 54 |
| 2026-08-19 | 2026-08-18 | 48 | 48 |
| 2026-08-20 | 2026-08-19 | 12 | 12 |
| 2026-08-21 | 2026-08-20 | 11 | 11 |
| 2026-08-22 | 2026-08-21 | 19 | 19 |
| 2026-08-23 | 2026-08-22 | 29 | 29 |
| 2026-08-24 | 2026-08-23 | 68 | 68 |
| 2026-08-25 | 2026-08-24 | 30 | 30 |
| 2026-08-26 | 2026-08-25 | 27 | 27 |
| 2026-08-27 | 2026-08-26 | 42 | 42 |
| 2026-08-28 | 2026-08-27 | 51 | 51 |
| 2026-08-29 | 2026-08-28 | 25 | 25 |
| 2026-08-30 | 2026-08-29 | 9 | 9 |
| 2026-08-31 | 2026-08-30 | 14 | 14 |
| 2026-09-01 | 2026-08-31 | 26 | 26 |
| 2026-09-02 | 2026-09-01 | 17 | 17 |

`mismatches: []`. Two rows are legitimately absent: load day 2026-09-03 redelivered nothing
because 2026-09-02's `MOD(id,50)=0` slice is empty (34 rows, none divisible by 50), and the
fixture's own 2026-09-03 slice of 3 rows was never redelivered because no load day 2026-09-04
ran. Both are properties of where the range ends, not gaps.

The 2026-08-05 → 2026-08-04 redelivery row that existed before this phase is gone, as
intended by the deletion.

## 5 — Cost (task 5)

**Loading**, summed over all 56 `INSERT` jobs' `totalBytesBilled`:

| | bytes | GB | @ US$5/TB |
|---|---:|---:|---:|
| 28 load days, both statements | 90,500,497,408 | 90.500 | **$0.4525** |

Actual billed came in **0.03% above** the dry-run projection (90,500,497,408 vs
90,469,230,404) — dry-run bytes were an accurate predictor at this scale.

**Verification queries** (all on the 10 MB minimum-billing floor or just above it):

| query | billed bytes |
|---|---:|
| partition census, before load | 10,485,760 |
| partition census, after load | 10,485,760 |
| edge-partition detail | 10,485,760 |
| per-day counts | 10,485,760 |
| totals + distinct ids | 17,825,792 |
| redelivery slices | 10,485,760 |
| row dump export (64,313 rows) | 22,020,096 |
| `DELETE` on `github_events` | 0 |
| `DELETE` on `github_events_arrival` | 10,485,760 |
| **verification subtotal** | **102,760,448** (0.103 GB, $0.0005) |

All 56 dry runs were free.

**Phase total: 90,603,257,856 bytes = 90.60 GB = US$0.4530**, against the dataset's
documented US$25/month cap — and inside BigQuery on-demand's 1 TiB/month free tier, so the
realised bill is likely $0.

## 6 — Retention deadline (D4)

Read back from table metadata after the load, not assumed from the DDL:

```
{"table":"github_events","expirationMs":"3888000000","field":"created_at","tableExpiry":null,"numRows":"65583"}
{"table":"github_events_arrival","expirationMs":"3888000000","field":"ingested_date","tableExpiry":null,"numRows":"65583"}
```

3,888,000,000 ms = **45 days**, unchanged, with no table-level expiry on either table. The
bound was not raised.

The oldest partition in both tables is **2026-08-05**, which therefore expires on
**2026-09-19**. Phases 13 and 14 must complete before then, or the fixture's oldest day
vanishes from BigQuery mid-comparison and reads as a divergence that is really an expiry.

## What phase 13 can assume

- `smelt_dogfood.github_events` holds **65,583 rows over 30 day-partitions, 2026-08-05 …
  2026-09-03, with 64,313 distinct ids**, and those 64,313 rows are **byte-identical** to
  `examples/github_activity/seeds/github_events_sample.parquet` on
  `(id, created_at, type, actor_id, repo_id, repo_name, payload_len, payload_md5)`. The two
  legs' populations are now the same population. No filter, mirror or scratch source is
  needed on either side.
- The 1,270-row excess over 64,313 is the loader's deliberate at-least-once redelivery:
  a `MOD(CAST(id AS BIGINT), 50) = 0` slice of day D-1 arriving on each load day D, present
  and exactly correct for all 28 boundaries. `silver.events_deduped` must fold it to 64,313.
- `smelt_dogfood.github_events_arrival` holds the same 65,583 rows keyed by
  `ingested_date` over 30 arrival partitions, 2026-08-05 … 2026-09-03 — one per calendar day
  the loader ran, in strictly increasing order.
- There are **no 2026-08-04 rows** in either table.
- The DuckDB leg needs no new machinery: `run_incremental.py`'s existing thirty-window replay
  already runs over exactly these rows, so phase 13's schedule is the fixture's 30 windows.
- **The model tables are now stale.** `bronze_events`, the four `silver_*` fan-out tables,
  `silver_events_deduped`, `silver_repo_naming`, `silver_actor_naming`, the two `gold_*`/
  `gold_repo_*` tables and the three `marts_*` tables all still hold the 6,053-row,
  three-day population (see the partition census above), as do `_smelt_ledger` (9 rows),
  `_smelt_observed_delta` (1 row) and the two empty `__tombstones` tables. That is expected:
  this phase widened the *source* only and ran no model. Phase 13 clears and rebuilds those
  on its own terms.
- Hard deadline **2026-09-19** (see D4).

## Findings

1. **`githubarchive` and the committed fixture agree exactly**, 26 days after the fixture was
   generated, on every column checked including a full `payload` MD5. The fixture's
   reproducibility claim holds against a live re-derivation. Nothing to escalate.
2. Phase 10's finding 1 stands unchanged and unaddressed here by design: the
   `` `raw. `` → `` `smelt_dogfood. `` mapping is still a deploy-time textual substitution
   with no home in the toolchain. This phase made that substitution 28 more times.
3. Phase 10's finding 3 ("load order matters more than the loader's own tests exercise") is
   now covered empirically in the increasing-order direction: 28 consecutive in-order loads,
   each landing its D-1 redelivery arm after the real day, all exact. The reversed-order case
   remains exercised only by phase 10's two days.

## Not done / blocked

Nothing was blocked; tasks 1–6 all completed. Nothing outside the declared blast radius was
written: the only tables modified are `github_events` and `github_events_arrival`, and the
only rows removed are the 150 named by the 2026-08-04 predicate.
