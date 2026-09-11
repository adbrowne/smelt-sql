# Phase 17 plan — expand the BigQuery population to the committed fixture's 30 days

**Runs before phases 13 and 14**, which depend on it. Numbered 17 because phase numbers are
historical; the Phases table records the ordering.

**Advances:** criterion 6 ("the two targets agree… over the same rows") by removing the
reason the two legs were not comparable, and extends criterion 3 (the loader, live) from
"at least two days" to the fixture's full range.

**Human-gated.** Runs the real loader against `githubarchive` and writes to
`smelt-bq-test-20260816.smelt_dogfood`. Costs real money — see §Cost.

## Why

`smelt_dogfood.github_events` holds 6,053 rows over 2026-08-04 … 2026-08-06. The committed
DuckDB fixture (`examples/github_activity/seeds/github_events_sample.parquet`) holds 64,313
events over **2026-08-05 … 2026-09-03**, and contains no 2026-08-04 rows at all. Two
consequences:

- The populations are not comparable, so dual-target parity had no honest basis.
- The BigQuery leg exercised three windows where the DuckDB leg exercises thirty, so the
  incremental behaviour the two legs demonstrate is not the same behaviour.

The previous plan closed the gap downward — mirror BigQuery's 3-day slice into DuckDB.
This phase closes it **upward** instead: load the fixture's whole range into BigQuery with
the real loader. That is the better basis, because the shared population is then the
*committed, gated* one rather than a scratch artifact, the DuckDB leg needs no new
machinery at all (`run_incremental.py`'s existing 30-day replay is already the oracle-gated
one), and criterion 3's "the loader produced it" property is preserved and widened rather
than side-stepped.

## D1 — load with the real loader, not by uploading the fixture

Uploading `github_events_sample.parquet` straight into BigQuery would be free and exact.
Reject it: it would make `smelt_dogfood.github_events` a copy of a local file rather than
the output of the scheduled query criterion 3 names, and would quietly retire the one piece
of evidence phases 9–10 exist to produce — that the loader's emitted SQL, gated
byte-identical to `sample.sql`, lands the rows smelt's source declaration is the contract
for. The ~$1 is worth keeping that property.

`scripts/bq-dogfood-loader.sh --emit-sql --date <D>` emits the two `INSERT`s for day D
(real day-D rows plus the deterministic 2% redelivery of day D-1, `ingested_date = D` in
the arrival twin). Phase 10's summary records the one substitution its output needs
(`raw.` → the dataset) and the invocation shape. Reuse that path exactly; do not author new
load SQL.

## D2 — the range, and the one deletion that makes the populations equal

Load days **2026-08-07 … 2026-09-03** (28 days). 2026-08-05 and 2026-08-06 are already
loaded (phase 10, in that reversed order deliberately).

Then **delete the 2026-08-04 rows** — 75 in each of the two tables. They are day-05's
redelivery arm reaching back a day into `githubarchive`, and the fixture has no 08-04 rows
for the DuckDB leg's own day-05 load to redeliver, so they are exactly the residue that
would make the two populations differ. Delete by explicit predicate
(`CAST(created_at AS DATE) = DATE '2026-08-04'` on the event-time table, and the matching
rows on the arrival twin), never by dropping a table. The deletion is reversible — re-running
the loader for 2026-08-05 restores it.

After that, both targets' populations should be, exactly:

- 64,313 real rows, one per fixture event; plus
- a 2% redelivered slice of day D-1 for each D in 2026-08-06 … 2026-09-03.

## D3 — the acceptance check is byte-level, not a row count

Equal row counts are necessary and nowhere near sufficient. Verify:

1. **Per-day counts match**, BigQuery `GROUP BY DATE(created_at)` against the fixture's own
   `GROUP BY created_at::date`, all 30 days, no day missing and none extra.
2. **The id sets match** — `COUNT(DISTINCT id)` on both sides, and the set difference in
   both directions is empty. Compute the BigQuery side in-warehouse and the DuckDB side
   locally; compare by exporting the BigQuery id set (64k short strings, a few MB).
3. **Row content matches on a checksummed column subset** — at minimum
   `(id, created_at, type, actor_id, repo_id, repo_name)`; `payload` is a large JSON string
   and may be compared by length and a sampled digest rather than in full, provided the
   sampling rule is written down.
4. **The redelivery slices are present and correct**: for each D, the count of rows with
   `created_at` on D-1 that arrived with `ingested_date = D` equals the fixture's
   `MOD(CAST(id AS BIGINT), 50) = 0` count for D-1.

Any mismatch is a finding recorded verbatim, not smoothed over. A mismatch in (1) or (2)
means `githubarchive` and the committed fixture have drifted, which would be a significant
finding about the fixture's reproducibility claim and must be escalated, not patched.

## D4 — retention is close enough to matter

Both tables carry `partition_expiration_days = 45` (`--emit-ddl`, parsed from
`README.md`). The fixture's oldest day, 2026-08-05, is 38 days old today (2026-09-12), so
it expires on **2026-09-19**. Phases 13 and 14 must run before then, or the oldest
partitions will vanish mid-comparison and look like a divergence. Record the deadline in
the summary and in the outcome's decision log; do not raise the retention bound to buy time
(it is a declared property the models' composition walk reads).

## Tasks

1. **Dry-run every day first.** For each of the 28 days, `dryRun: true` on both statements;
   record `totalBytesProcessed` per day and the running total. Stop and report if the total
   exceeds **250 GB** (≈ $1.25) rather than pressing on — phase 10 measured ~3.5 GB/day for
   early August and noted GitHub Archive volume trends up through the range.
2. **Load the 28 days in calendar order**, one day at a time, checking the insert row count
   against the fixture's expectation for that day before moving on. A day whose count is
   wrong stops the sequence.
3. **Delete the 2026-08-04 rows** from both tables; confirm 75/75 removed and that the
   partition is empty rather than the table changed in any other way.
4. **Run the D3 acceptance checks** and record every number.
5. **Record cost**: billed bytes per day, the total, and the dollar figure at $5/TB.
6. Write `phases/17-summary.md`; add a row 17 to the outcome's Phases table marked `done`,
   and a 2026-09-12 decision-log entry stating the new basis, the deletion, the retention
   deadline, and the cost.

## Cost

Estimated **$0.55 – $1.00** total (28 days × ~4–7 GB billed at $5/TB), against the dataset's
documented US$25/month cap. Dry-run gate at 250 GB. Every other query in this phase is a
metadata or `COUNT` read on the 64k-row tables — the 10 MB minimum-billing floor.

## Blast radius

- Writes only to `smelt_dogfood.github_events` and `smelt_dogfood.github_events_arrival`.
- Reads `githubarchive.day.2026*` — the only phase since 10 that touches it.
- **No table is dropped or truncated.** The only deletion is the explicit 2026-08-04
  predicate.
- Model tables and bookkeeping tables are not touched; phase 13 clears those on its own
  terms, and it is expected that after this phase the existing model tables are stale
  relative to the widened source. Say so in the summary rather than leaving it implied.
- `target: dev` stays pinned; nothing in `examples/github_activity/` changes except, if
  anything, documentation.

## What this phase does not do

- It does not run any model, on either target.
- It does not compare targets — that is phase 13.
- It does not change the loader, `sample.sql`, the fixture, or the retention bound.
