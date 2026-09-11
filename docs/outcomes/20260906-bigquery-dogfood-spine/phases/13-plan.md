# Phase 13 plan — dual-target parity: DuckDB and BigQuery over the same rows

**Advances:** criterion 6 ("The two targets agree"), completing it. It also produces the
relation-level snapshots phase 14 consumes, so it is the phase that has to settle the
comparison basis, the comparator and the divergence vocabulary — phase 14 only adds the
oracle leg on top.

**Human-gated.** Runs live BigQuery against `smelt-bq-test-20260816.smelt_dogfood` using
ADC impersonation of `smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com`. A
headless iteration with no credential must emit `<<PHASE_BLOCKED>>` rather than skip green.

## Spec delta

None. No user-visible feature behaviour changes. The deliverables are a comparison
harness, its offline gates, and a measured parity report.

## What the live dataset holds today (verified this session, not assumed)

`SELECT table_id, row_count FROM smelt_dogfood.__TABLES__`:

```
_smelt_ledger 9          github_events 6053          silver_actor_naming 6053
_smelt_observed_delta 1  github_events_arrival 6053  silver_actor_naming__tombstones 0
bronze_events 6053       gold_events_enriched 5990   silver_events_deduped 5990
gold_repo_activity_daily 799                         silver_issue_events 2
gold_repo_dim 709        marts_naming_history 2      silver_pr_events 5
marts_repo_leaderboard 709  marts_star_growth 2      silver_push_events 5710
                                                     silver_repo_naming 6053
                                                     silver_repo_naming__tombstones 0
```

Two things follow that the phase-12 summary's "What phase 13 can assume" section no longer
states correctly, and which this plan supersedes:

1. **Fourteen model tables exist, not ten** — `gold_repo_dim`, `gold_events_enriched`,
   `marts_repo_leaderboard` and `marts_star_growth` are materialised, because
   `20260906-bigquery-correctness` closed the two emitted-SQL defects (2026-09-11 entries).
   Only `silver.actor_sessions` and `marts.daily_active_contributors` stay out, refused at
   *compile* time on GoogleSQL for the INTERVAL `RANGE` frame.
2. **BigQuery now carries engine-resident bookkeeping** — `_smelt_ledger`,
   `_smelt_observed_delta` and the two `__tombstones` tables exist, landed by that same
   outcome's phases 12–15. They are excluded from the parity comparison for the reason the
   DuckDB oracle already excludes them: they record *how* a run happened, not model state.
3. `probes: { cadence: off }` is **gone** from `smelt.yml` and must not come back — one
   committed configuration now serves both targets.

## D1 — the comparison basis: one population, the committed fixture

Criterion 6 says "over the same rows". The two populations differed by construction, and
**phase 17 closes that upward**: the fixture's whole 30-day range is loaded into
`smelt_dogfood.github_events` / `github_events_arrival` by the real loader, and the stray
2026-08-04 slice — which the fixture cannot contain — is deleted. Read `phases/17-summary.md`
for the measured acceptance checks before running anything here; if its per-day counts,
id-set equality and redelivery-slice checks did not all pass, this phase has no basis and
must stop rather than compare.

The consequence for this phase is a simplification: **there is no mirror artifact and no
staging of a synthetic DuckDB source.** The DuckDB leg is the committed, already-gated
30-day replay (`examples/github_activity/run_incremental.py`, the same replay
`github_activity_oracle.rs` runs), and the BigQuery leg runs the same 30 windows over the
same rows.

| | rows | event-time days |
|---|---|---|
| `seeds/github_events_sample.parquet` | 64,313 real events | 2026-08-05 … 2026-09-03 |
| `smelt_dogfood.github_events` after phase 17 | the same 64,313, plus each day's 2% redelivery of the day before | the same 30 days |

## D2 — the schedule: the fixture's thirty windows

Both legs run one window per fixture day — the schedule `run_incremental.py` already
defines (`DEFAULT_START = 2026-08-05`, `DEFAULT_DAYS = 30`), with the first day a full
refresh and the remaining twenty-nine incremental. This supersedes phase 12's four-step
schedule, which existed only because the BigQuery source held three days.

Clearing the ground on BigQuery means dropping the 14 model tables, the two `__tombstones`
tables, `_smelt_ledger`, `_smelt_observed_delta`, and
`examples/github_activity/.smelt/targets/bigquery/` — the model tables are stale relative to
the widened source anyway. **`github_events` and `github_events_arrival` are never dropped,
never truncated, never reloaded, and `githubarchive` is never touched** — phase 17 owns the
source and this phase only reads it. Drop by explicit name from a list the script prints
first; no wildcard drop.

**Arrival order differs between the legs, and that is a variable to control, not ignore.**
On BigQuery the source is fully populated before the first `smelt run`, so the windows
advance over a static table; the DuckDB replay loads day D at window D. If smelt's windowing
is correct the two produce identical state, and any difference is a real finding — but it
would be a finding about *arrival order*, not about the engine. So: run the natural pair
first, and if any relation diverges, attribute it by re-running the DuckDB leg with the
source pre-loaded (pre-populate both source tables from the fixture and pre-seed
`main._loader_days` with all thirty days, which makes `load_day.sh`'s existing per-day
idempotence turn the declared external step into a no-op — no change to `load_day.sh` is
needed and none should be made). Report which of the two the divergence survives.

**Both legs exclude the same two models** — `-e silver.actor_sessions -e
marts.daily_active_contributors` — so the relation sets are equal by construction. Their
absence from BigQuery is not a value divergence to register; it is a compile-time refusal,
recorded in the summary as a structural exclusion with its `UnsupportedOnBackend` message
quoted.

**Comparison points.** Compare after **every** window, as the DuckDB oracle already does —
not only at the end. Thirty comparison points × 14 relations is the claim; if the wall-clock
cost of exporting 14 relations from BigQuery thirty times proves prohibitive, reduce the
*export* frequency explicitly and say so in the summary (e.g. every window for the small
relations, every fifth plus the final for the 64k-row ones), never silently.

## D3 — the comparator: whole-row multiset difference, not stringify-and-sort

`crates/smelt-cli/tests/common/mod.rs:556` `batches_to_sorted_rows` (stringify every cell,
sort rows) is the repo's only existing cross-target value comparator. Do not reach for it
here: it collapses type differences into formatting differences and gives an unactionable
diff on a 64k-row relation.

Use instead the primitive the oracle already gates on
(`github_activity_oracle.rs:92` `relation_diff`): `EXCEPT ALL` in both directions over
`SELECT *`, run **inside DuckDB**, with the BigQuery side pulled local. Per relation:

1. Export `SELECT * FROM smelt_dogfood.<relation>` with
   `scripts/bq_dogfood_export.py` (typed NDJSON — it decodes BigQuery's all-strings REST
   encoding using the result schema).
2. Load it into a scratch DuckDB database, casting each column to the **DuckDB leg's own**
   declared type for that column, read from `information_schema.columns`. A column whose
   BigQuery value will not cast is a finding, not a tolerance.
3. `SELECT * FROM duck.<relation> EXCEPT ALL SELECT * FROM bq.<relation>` and the mirror;
   report both counts plus up to five `to_json` sample rows per side.

**Relation-set totality first.** Discover relations on each side generically — DuckDB from
`information_schema.tables`, BigQuery from `INFORMATION_SCHEMA.TABLES` — apply the same
`EXCLUDED_PREFIXES = ["sources_", "_smelt_"]` plus a `__tombstones` suffix exclusion, and
fail if a relation exists on one side only. Discovery must not be a hardcoded model list;
that is what makes the sweep non-vacuous.

**Normalisation is declared, not implicit.** The only admissible normalisation is the
type cast in step 2 (INT64→BIGINT, FLOAT64→DOUBLE, NUMERIC→DECIMAL, TIMESTAMP→TIMESTAMP
at UTC, DATE→DATE, STRING→VARCHAR, BOOL→BOOLEAN). Any tolerance beyond exact value
equality — float epsilon, timestamp truncation, string trimming — is a **registered
divergence with a reason**, never a quiet comparator setting. Write the policy into the
comparator's doc comment.

## D4 — the divergence vocabulary

Copy the shape that already works in `github_activity_oracle.rs:174` rather than inventing
one: a `const TARGET_DIVERGENCE_REGISTRY: &[TargetDivergence]` whose entries name the
relation, the reason, and a checkable bound; an unregistered non-zero diff fails; and a
two-sided liveness gate (an entry naming a relation that no longer diverges is an error
telling you to delete it, exactly as `registry_entries_are_all_live` does).

If the sweep comes back empty — the two targets agree on all 14 relations at every window — that is the best outcome and the registry stays empty. It is then **mandatory**
to prove the sweep still fails closed on an empty registry, per the precedent at
`github_activity_oracle.rs:913`; an empty registry plus a vacuous sweep is indistinguishable
from success and must not be shipped as one.

## Tests

Offline, per-PR, no credential — new target `crates/smelt-cli/tests/github_activity_dual_target.rs`:

1. `relation_set_mismatch_fails` — two synthetic DuckDB databases, one missing a relation;
   the totality check returns `Err` naming the relation and the side. RED first.
2. `an_unregistered_target_divergence_fails` — synthetic pair differing in one row; the
   comparator reports it and the sweep fails with the relation and both counts named.
3. `the_sweep_fails_closed_on_an_empty_registry` — the same synthetic mismatch with
   `TARGET_DIVERGENCE_REGISTRY` empty still fails, so an empty registry cannot pass
   vacuously.
4. `registry_entries_are_all_live` — two-sided: every entry names a relation the committed
   parity report lists, and no relation the report marks divergent is missing an entry.
5. `excluded_models_are_exactly_the_compile_refused_pair` — the exclusion set is
   `{silver.actor_sessions, marts.daily_active_contributors}` and nothing else, so a future
   silent widening of the exclusion list fails a test rather than shrinking the comparison.
6. `the_parity_report_covers_every_model_on_both_targets` — string-level over the committed
   report artifact: 14 relations at every compared window, no cell blank.

Live, credential-gated, `#[ignore]`d (or driven only by the script):

7. `duckdb_and_bigquery_agree_on_every_model` — the whole sweep against the live dataset.
   Must **fail**, not skip, when the credential is absent but `SMELT_BQ_DOGFOOD_LIVE=1` is
   set; skip green only when neither is set.

Existing gates that must stay green unchanged: `github_activity_replay` (21),
`github_activity_oracle` (18), `github_activity_loader` (11), `example_diagnostics` (128),
`example_workspaces` — and `bash .claude/scripts/verify-phase.sh` at the end.

## Tasks

1. **Offline first, red-green.** Write `github_activity_dual_target.rs` tests 1–5 and the
   comparator they exercise (relation discovery, totality, `EXCEPT ALL` both ways, registry
   sweep) against synthetic DuckDB databases. No cloud in this task; the whole comparator
   is exercisable with two local `.duckdb` files.
2. **The driver.** `scripts/bq-dogfood-parity.sh` — clears the ground on BigQuery (explicit
   named drops, printed first), runs the thirty-window schedule on both targets with the two
   exclusions, snapshots every relation after each compared window, runs the comparator, and
   writes `phases/13-parity.json` plus a markdown table. The DuckDB leg is the committed
   replay (`run_incremental.py`), not a re-implementation. Shellcheck-clean at `warning`.
3. **Confirm the basis.** Before any comparison, re-read `phases/17-summary.md` and re-verify
   cheaply that the BigQuery source still holds the fixture's thirty days (per-day counts and
   `COUNT(DISTINCT id)`). Partitions expire 2026-09-19 — a comparison run after that date is
   invalid, not merely late.
4. **Run it live.** Build `cargo build -p smelt-cli --features bigquery`; environment is
   `source scripts/bq-dogfood-env.sh` then
   `export SMELT_BQ_ACCESS_TOKEN=$(gcloud auth application-default print-access-token --impersonate-service-account=smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com)`.
   Record per-run wall time, exit code, report id and billed bytes. Thirty windows at ~110s
   each is roughly an hour of wall time; budget for it rather than truncating the schedule.
5. **Register what diverges.** Each difference gets an entry with a *reason* — root-caused
   to the construct and the engine, not "BigQuery rounds differently". A difference that
   cannot be explained is a finding for `20260906-bigquery-correctness`, recorded as such
   in the summary and in the phase-16 handoff input, not papered over with an entry.
6. **Commit the report and the tests**, then `bash .claude/scripts/verify-phase.sh`.
7. **Write `phases/13-summary.md`**: the schedule, the per-relation × per-window table, the
   divergences with reasons, cost, and a "what phase 14 can assume" section that is
   accurate about the population, the schedule and the bookkeeping tables.

## Cost and blast radius

Every query in this phase reads at most the 64k-row source and the model tables derived from
it; thirty windows of pipeline reads plus the per-window exports should still bill only a few
cents, most of it BigQuery's 10 MB metadata-query minimum applied many times over. Measure it
rather than assuming it, and stop and report if the running total passes US$1. Guards: dry-run any statement
before its first live use; never touch `githubarchive`; never drop or reload the two source
tables; never write outside `smelt_dogfood`; never unpin `target: dev`.

## What this phase does not do

- It does not fix anything the comparison surfaces. Fixes belong to
  `20260906-bigquery-correctness` (this outcome's §"Out of scope").
- It does not make `silver.actor_sessions` runnable on BigQuery. The window-spec lowering
  seam is separate work.
- It does not run the full-refresh oracle on either target — that is phase 14.
- It does not add a CI tier for BigQuery (standing decision D2 of
  `docs/plans/20260821-bigquery-remaining.md`: no CI tier).
