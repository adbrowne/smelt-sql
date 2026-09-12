# Phase 14 summary — the numbers are trustworthy on both targets

**Executed live** against project `smelt-bq-test-20260816`, datasets `smelt_dogfood`
(incremental state, read only for export) and `smelt_dogfood_oracle` (the full-refresh
oracle, created and dropped inside this phase), via ADC impersonation of
`smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com`, on 2026-09-12 UTC. Cost
**US$0.06**.

**Result in one line:** at the final window — where "the inputs seen so far" *are* the whole
source, so the oracle is exactly the invariant's full refresh over them — **all fourteen
compared relations are byte-equal to their own full refresh, zero rows in both directions of
a whole-row multiset difference.** Eight of the fourteen are additionally equal at *every*
compared checkpoint. The other six cannot be compared at an intermediate checkpoint against a
static source, for a reason that is measured rather than argued, and each carries a checkable
proof of that reason.

---

## 1 — The oracle definition actually used (D1)

For window *k*, the oracle is a full refresh over the inputs seen so far, materialised into a
separate set of relations and compared relation-by-relation against the incrementally
maintained state at the same point. That is the DuckDB oracle's definition carried over
unchanged (`github_activity_oracle.rs`'s `full_replay_pair`), and it is realised here as:

```
smelt run --target bigquery_oracle --full-refresh \
  --event-time-start 2026-08-05 --event-time-end <day k+1> \
  -e silver.actor_sessions -e marts.daily_active_contributors
```

against a **freshly emptied** `smelt_dogfood_oracle` — every table dropped and
`.smelt/targets/bigquery_oracle/` removed before each checkpoint, so nothing carries between
them, mirroring the DuckDB oracle's fresh workspace per window.

The project gained a third target rather than any run-time mutation:

```yaml
  bigquery_oracle:
    type: bigquery
    project: smelt-bq-test-20260816
    dataset: smelt_dogfood_oracle
    location: US
    schema: smelt_dogfood_oracle
```

### The anti-vacuity gate, written RED first

Both source declarations carry a `bigquery_oracle:` entry pointing at the **shared** physical
tables:

```yaml
name:
  bigquery: smelt_dogfood.github_events
  bigquery_oracle: smelt_dogfood.github_events
```

Without it, `SourceInfo::db_name_for_target` falls back to the default mapping and the oracle
reads `smelt_dogfood_oracle.sources_raw_github_events` — a table nothing creates. The oracle
would refresh over zero rows and the sweep would pass vacuously. That is asserted through the
**real resolver**, not by reading the YAML, and it was written before the YAML and observed
failing on exactly the predicted value:

```
assertion `left == right` failed: the `bigquery_oracle` target must resolve
`sources.raw.github_events` to the shared source table — otherwise the full-refresh oracle
reads an empty table and the equivalence sweep passes vacuously
  left: "smelt_dogfood_oracle.sources_raw_github_events"
 right: "smelt_dogfood.github_events"
```

The gate is doubled at the moment of measurement (every compared relation must have non-zero
oracle rows) and over the committed report (`bronze_events` must carry the source's full
65,583 rows on the oracle side at the final window).

`target: dev` stayed pinned. `adding_the_oracle_target_does_not_move_the_default` resolves the
default the way `smelt-runtime/src/profile.rs` does and requires `dev`, *and* requires that
the alphabetically-first target is not `dev` — so the test cannot go green by the pin becoming
redundant. `example_diagnostics` (128 passed) and `example_workspaces` (37 passed) both read
the committed project and stayed green.

### Dataset lifecycle is a human-credential step, and that is deliberate

`CREATE SCHEMA` under the dogfood service account fails:

```
Access Denied: Project smelt-bq-test-20260816: User does not have
bigquery.datasets.create permission in project smelt-bq-test-20260816.
```

That is not a gap to paper over — `scripts/bq-dogfood-provision.sh` gave the SA
`roles/bigquery.jobUser` plus WRITER on one dataset precisely so that "it writes to one
long-lived dataset and never creates datasets of its own". Keeping that property is worth more
than the convenience, so dataset lifecycle moved to a new sibling script under the human
credential, `scripts/bq-dogfood-oracle-dataset.sh {create,drop}`, and the run itself stayed on
the scoped SA. The script refuses outright if pointed at `smelt_dogfood` or `smelt_test`.

**Created** 2026-09-12, `defaultTableExpirationMs = ABSENT` read back from the API rather than
inferred from a successful create, with WRITER granted to the SA:

```
smelt_dogfood_oracle
defaultTableExpirationMs = ABSENT
  WRITER projectWriters
  WRITER smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com
  OWNER projectOwners
  OWNER adbrowne@gmail.com
  READER projectReaders
```

**Dropped** at the end of the phase and confirmed gone, with `smelt_dogfood` untouched beside
it:

```
smelt_dogfood_oracle is already gone     # the drop ran; this is the confirming re-check
smelt_dogfood
```

## 2 — The DuckDB half is met and was not rebuilt (D2)

Re-run this session, unchanged, and cited rather than duplicated:

```
cargo test -p smelt-cli --features duckdb --test github_activity_oracle \
  every_window_matches_the_full_refresh_oracle
  -> test result: ok. 1 passed; 0 failed  (317.64s)
```

It replays all **thirty** windows of the committed fixture and compares **every** materialised
relation — including `silver.actor_sessions` and `marts.daily_active_contributors` — against a
full-refresh oracle staged over the identical rows seen so far. Since phase 17 widened the
BigQuery source to exactly that fixture, that leg covers the same population BigQuery runs on.
No second DuckDB oracle was written.

**Coverage is not symmetric, and saying so is part of the result.** The DuckDB half covers 16
models over 30 windows; the BigQuery half covers **14 models over 7 windows**. The two missing
models are a compile-time `UnsupportedOnBackend` refusal on GoogleSQL's INTERVAL `RANGE`
lookback frame, not a value divergence, and they are excluded from both legs of the BigQuery
comparison so the relation sets are equal by construction.

## 3 — Comparison points, and why seven (D2)

Windows **1, 2, 3, 5, 10, 20 and 30** — the set phase 13 measured and declared, reused rather
than re-invented, so a phase-13 claim and a phase-14 claim at the same window are about the
same state. `PARITY_CHECKPOINTS` widens them; the measurement did not call for it.

The incremental side of every comparison is phase 13's own exported snapshot at that
checkpoint — not re-run — so the state being judged is literally the state phase 13 judged.
Only the oracle side is new. Its cost is the one that climbs with the window, since each
checkpoint re-derives every model from a growing prefix:

| checkpoint | window end | model execution |
|---|---|---|
| w01 | 2026-08-06 | 68.3 s |
| w02 | 2026-08-07 | 80.1 s |
| w03 | 2026-08-08 | 86.8 s |
| w05 | 2026-08-10 | 112.4 s |
| w10 | 2026-08-15 | 172.6 s |
| w20 | 2026-08-25 | 294.1 s |
| w30 | 2026-09-04 | 415.5 s |

Every one reported `built 14 model(s)`, exit 0; total oracle execution **1,229.8 s (20.5 min)**,
plus the per-checkpoint drop and the 98 relation exports. Thirty checkpoints would have been
roughly four times that, and the plan's declared seven were kept.

**Criterion 7's BigQuery half is therefore checked at seven of thirty windows, not all
thirty** — with the caveat sharpened in §5: at six of those seven the check is partial by
construction, and the final window is the one that is complete.

## 4 — The result (D3)

Whole-row multiset difference — `EXCEPT ALL` in both directions over `SELECT *` — run inside
DuckDB over both BigQuery snapshots landed locally under the **same** declared types, by the
same primitive phase 13 uses. That primitive now lives in one place
(`crates/smelt-cli/tests/bq_parity_support/`) and is consumed by both suites, so the two
claims are made by one comparator rather than by two that might disagree about what "equal"
means.

`=` is zero in both directions; `EX` is exempt at that checkpoint with its proof holding
(§5); `-i/+o` would be *i* rows only on the incremental side, *o* only on the oracle's.

| relation | w01 | w02 | w03 | w05 | w10 | w20 | **w30** |
|---|---|---|---|---|---|---|---|
| `bronze_events` | = | = | = | = | = | = | **=** |
| `gold_events_enriched` | EX | EX | EX | EX | EX | EX | **=** |
| `gold_repo_activity_daily` | = | = | = | = | = | = | **=** |
| `gold_repo_dim` | EX | EX | EX | EX | EX | EX | **=** |
| `marts_naming_history` | EX | EX | EX | EX | EX | EX | **=** |
| `marts_repo_leaderboard` | EX | EX | EX | EX | EX | EX | **=** |
| `marts_star_growth` | = | = | = | = | = | = | **=** |
| `silver_actor_naming` | EX | EX | EX | EX | EX | EX | **=** |
| `silver_events_deduped` | = | = | = | = | = | = | **=** |
| `silver_issue_events` | = | = | = | = | = | = | **=** |
| `silver_pr_events` | = | = | = | = | = | = | **=** |
| `silver_push_events` | = | = | = | = | = | = | **=** |
| `silver_repo_naming` | EX | EX | EX | EX | EX | EX | **=** |
| `silver_star_events` | = | = | = | = | = | = | **=** |

Row counts behind those cells (`incremental/oracle` where they differ, one number where they
are equal):

| relation | w01 | w02 | w03 | w05 | w10 | w20 | w30 |
|---|---|---|---|---|---|---|---|
| `bronze_events` | 65583 | 65583 | 65583 | 65583 | 65583 | 65583 | 65583 |
| `gold_events_enriched` | 3201 | 5915 | 8249 | 14932 | 32758 | 53018 | 64313 |
| `gold_repo_activity_daily` | 399 | 762 | 1165 | 2286 | 5530 | 8529 | 9997 |
| `gold_repo_dim` | 399/5016 | 686/5016 | 956/5016 | 1644/5016 | 3186/5016 | 4438/5016 | 5016 |
| `marts_naming_history` | 1/42 | 2/42 | 3/42 | 10/42 | 26/42 | 35/42 | 42 |
| `marts_repo_leaderboard` | 399 | 686 | 956 | 1644 | 3186 | 4438 | 5016 |
| `marts_star_growth` | 1 | 2 | 3 | 5 | 7 | 12 | 22 |
| `silver_actor_naming` | 3143/64168 | 5856/64168 | 8178/64168 | 14853/64168 | 32647/64168 | 52893/64168 | 64168 |
| `silver_events_deduped` | 3201 | 5915 | 8249 | 14932 | 32758 | 53018 | 64313 |
| `silver_issue_events` | 1 | 1 | 13 | 20 | 47 | 73 | 128 |
| `silver_pr_events` | 2 | 5 | 86 | 99 | 143 | 202 | 366 |
| `silver_push_events` | 3015 | 5639 | 7710 | 13988 | 30680 | 50062 | 60643 |
| `silver_repo_naming` | 3143/64174 | 5856/64174 | 8177/64174 | 14852/64174 | 32651/64174 | 52899/64174 | 64174 |
| `silver_star_events` | 1 | 2 | 10 | 15 | 18 | 32 | 47 |

Machine-readable: `14-equivalence.json`, written by the live sweep and read per-PR by five
gates over it.

**`EQUIVALENCE_DIVERGENCE_REGISTRY` is empty, and stayed empty.** Nothing was registered away.
It fails closed: `the_equivalence_sweep_fails_closed_on_an_empty_registry` drives the real
registry-consulting path over a perturbed pair with the registry passed explicitly as `&[]`,
so it keeps holding whatever the registry comes to contain.

## 5 — Where the oracle is a valid oracle, and where it is not (D4)

Six relations are `EX` at the six intermediate checkpoints. **None of them is a divergence
that was registered away, and none is licensed to be wrong.** The reason is about the
*oracle*, not about the incremental state, and it is the only genuinely new finding of this
phase.

The invariant is `incremental_state(S) == full_refresh(inputs ∈ S)`. On BigQuery the source
tables statically hold all thirty days from before window 1 — phase 13's arrival-order
observation, one level up. A model whose full refresh bounds its own scan to the requested
window therefore has a valid oracle at every checkpoint (that is the eight `=` rows). A model
whose full refresh reads beyond the window does not, at an intermediate one, because its
`inputs` are then a strict superset of what the incremental leg had seen — so the invariant's
antecedent does not hold, and comparing against it would measure arrival order rather than
maintenance, which is exactly the mistake phase 13 controlled for.

**At the final window there is nothing to narrow**, because there the inputs seen so far *are*
the whole source. Every relation is compared normally at w30 and all fourteen agree exactly —
gated by `the_final_window_compares_every_relation_with_nothing_exempt`, which fails if
anything is ever exempt there.

Each exemption carries a **checkable proof**, not a note, and a failing proof fails the sweep:

| relation | proof | mechanism |
|---|---|---|
| `silver_repo_naming` | `oracle_is_the_final_state` | succession cell: a row's `valid_to`/`is_current` is a function of every *later* event for the key, so a full refresh rebuilds the whole history |
| `silver_actor_naming` | `oracle_is_the_final_state` | the same shape on `actor_id` over the arrival-partitioned twin source |
| `gold_repo_dim` | `oracle_is_the_final_state` | declares `maintenance.scan_bounds.per_source.silver.repo_naming.allow_full_scan: true`, so it inherits that upstream's whole-history rebuild |
| `marts_naming_history` | `oracle_is_the_final_state` | derives renames from `silver.repo_naming` pairwise, inheriting the same |
| `gold_events_enriched` | `inherited_columns_only` (`current_repo_name`, from `gold_repo_dim`) | its own row set is window-bounded and matches exactly; the enrichment `LEFT JOIN` is what reaches beyond the window |
| `marts_repo_leaderboard` | `inherited_columns_only` (`current_repo_name`, from `gold_repo_dim`) | groups by the dimension's name column, one level further down |

- `oracle_is_the_final_state` requires the relation's oracle at checkpoint *k* to be
  **byte-identical to its oracle at window 30** — direct evidence that the refresh read the
  whole source rather than the requested window. It holds at every intermediate checkpoint for
  all four, which is why those rows read `399/5016`, `1/42`, `3143/64168`, `3143/64174`: the
  right-hand number never moves.
- `inherited_columns_only` is the tighter of the two: the two sides must agree **exactly** once
  the named columns are projected away. Row counts are already identical at every checkpoint
  (3201/3201, 399/399, …) and the symmetric diffs are small and equal (3/3, 5/5, 8/8, 9/9,
  17/17, 7/7 on `gold_events_enriched`) — the signature of one column differing on a handful of
  renamed repos, and nothing else. A lost row, a duplicated row or any other column moving
  fails the proof. The named column must exist, or the proof is refused as vacuous.

The exemption list is checked against the committed report in both directions
(`the_unbounded_refresh_set_is_exactly_what_the_report_shows`): every exempted row must carry
a named proof and a recorded verdict of `true`, and the set the report exempts must be exactly
the set the code lists.

Worth stating plainly, because it is the difference between this and a registered divergence:
the incremental state of all six is **byte-equal to its own full refresh at the final
window**. Nothing here says a maintained model is stale; it says a full refresh over a static
source is not a window-bounded oracle.

## 6 — Bookkeeping is excluded by decision, not by absence (D3)

`_smelt_ledger`, `_smelt_observed_delta` and the two `__tombstones` tables now exist on
BigQuery — new since phase 12, and a deliberate part of the state posture this outcome landed.
They are excluded from the comparison by prefix and suffix, for the reason the DuckDB oracle
already gives: they record *how* a run happened, and an incremental run's bookkeeping
legitimately differs from a `--full-refresh` run's. Excluding them is a decision, and this is
where it is recorded.

## 7 — Cost (stop gate: US$2)

Read from BigQuery job history (`statistics.query.totalBytesBilled`) under the human
credential — the scoped dogfood SA has `roles/bigquery.jobUser` and cannot query
`JOBS_BY_PROJECT`:

```
{"jobs": 1438, "totalBytesBilled": 12882804736, "GB": 11.998, "usd_at_5_per_TB": 0.0586}
```

**US$0.06** for everything this phase issued: seven full refreshes of fourteen models over a
growing prefix, the per-checkpoint dataset emptying, 98 relation exports, the basis and cost
reads. Two orders of magnitude inside the stop-and-report threshold, and a fifth of phase 13's
US$0.29 — because the expensive half (thirty windows of incremental execution) was reused
rather than re-run.

## 8 — Blast radius

- `github_events` and `github_events_arrival` were **read only** — never dropped, truncated or
  reloaded. Confirmed after the phase: **65,583 rows each**.
- `smelt_dogfood` still holds **20 tables** (14 models at their w30 state, 2 sources, 2
  `__tombstones`, `_smelt_ledger`, `_smelt_observed_delta`) — the phase wrote nothing into it.
- Nothing touched `githubarchive`.
- Writes went only into `smelt_dogfood_oracle`, which no longer exists.
- `target: dev` was never unpinned.

## 9 — Gates

```
cargo test -p smelt-cli --features duckdb --test github_activity_bq_oracle
  -> test result: ok. 12 passed; 0 failed

cargo test -p smelt-cli --features duckdb --test github_activity_dual_target
  -> test result: ok. 16 passed; 0 failed          (unchanged by the comparator extraction)

cargo test -p smelt-cli --features duckdb --test github_activity_oracle \
  every_window_matches_the_full_refresh_oracle
  -> test result: ok. 1 passed; 0 failed           (the DuckDB half, cited not rebuilt)

cargo test -p smelt-cli --test example_diagnostics     -> 128 passed; 0 failed
cargo test -p smelt-lsp  --test example_workspaces     ->  37 passed; 0 failed

SMELT_BQ_DOGFOOD_LIVE=1 cargo test -p smelt-cli --features duckdb \
  --test github_activity_bq_oracle bigquery_incremental_matches_its_oracle_at_every_window
  -> test result: ok. 1 passed; 0 failed           (the live sweep, and the writer of 14-equivalence.json)
```

The twelve offline tests are the six the plan asked for plus six that keep the new vocabulary
honest:

- `the_oracle_target_resolves_sources_to_the_shared_tables` — the D1 anti-vacuity gate,
  through the real resolver. Written RED.
- `adding_the_oracle_target_does_not_move_the_default` — two-sided: `dev` is the default, and
  `dev` is *not* alphabetically first, so the pin cannot silently stop mattering.
- `an_unregistered_equivalence_violation_fails` — a real value difference fails, naming the
  relation and both counts under this suite's own side labels.
- `the_equivalence_sweep_fails_closed_on_an_empty_registry` — the registry-consulting path
  driven over `&[]` explicitly.
- `a_relation_missing_from_the_oracle_is_a_coverage_failure` — coverage totality, which is the
  shape a silently-empty oracle would take.
- `the_equivalence_report_covers_every_model_at_every_checkpoint` — 14 relations at *every*
  checkpoint, every cell present, a scope on every row, the final window present, neither
  excluded model listed.
- `the_committed_equivalence_report_shows_no_violation` — the core claim, gated over the
  committed evidence.
- `the_final_window_compares_every_relation_with_nothing_exempt` — nothing may be exempt at
  w30, which is what makes the final-window claim complete.
- `the_committed_report_proves_the_oracle_read_the_shared_source` — every relation non-empty on
  the oracle side at w30, and `bronze_events` carrying the source's full 65,583 rows.
- `the_unbounded_refresh_set_is_exactly_what_the_report_shows` — the exemption ratchet, both
  directions, plus every entry having a stated mechanism.
- `equivalence_registry_entries_are_all_live` — two-sided over the report.
- `bigquery_incremental_matches_its_oracle_at_every_window` — skips without
  `SMELT_BQ_DOGFOOD_LIVE=1`; with it set and no snapshots, **fails** rather than skipping.

`bash .claude/scripts/verify-phase.sh` output is quoted in the commit for this phase.

## Findings

1. **Criterion 7 is met on both targets.** On BigQuery, after the fixture's full thirty-window
   incremental run, every one of the fourteen compared models' maintained state is byte-equal
   to a full refresh over the whole population — zero rows in both directions, no registered
   divergence standing. On DuckDB the same holds at every one of thirty windows over all
   sixteen models. Nothing was registered away to get there.
2. **A full refresh on BigQuery is not a window-bounded oracle for six of the fourteen
   models** — and this is the phase's substantive finding, for
   `20260906-bigquery-correctness` to take a view on. `smelt run --full-refresh
   --event-time-start X --event-time-end Y` bounds the scan for eight relations and does not
   for the succession cells, the dimension declaring `allow_full_scan`, and what they feed.
   Measured, not inferred: those relations' oracle output at window 1 is **byte-identical to
   their output at window 30**. Whether that is correct (a full refresh should rebuild a
   succession history in full) or a defect (the `--event-time-end` bound should reach the
   source scan) is a product question this phase deliberately does not answer — it records the
   measurement. It is invisible on DuckDB, where the oracle stages a truncated source, so it
   could only have surfaced against a warehouse-resident source.
3. **The two suites now share one comparator.** The whole-row multiset difference, the generic
   relation discovery and exclusions, the divergence-bound vocabulary and the typed NDJSON
   landing seam moved to `crates/smelt-cli/tests/bq_parity_support/`, consumed by both
   `github_activity_dual_target.rs` and `github_activity_bq_oracle.rs`. Phase 13's sixteen
   tests pass unchanged through the move. Two different claims, one primitive — so "equal"
   cannot come to mean two things.
4. **The scoped credential could not create the oracle dataset, and that was the right
   outcome.** The refusal (`bigquery.datasets.create`) is the provisioning design working:
   dataset lifecycle moved to a human-credential script rather than the SA's grant being
   widened.

## Not done / blocked

- Nothing was blocked.
- **Thirty checkpoints were not run**; seven were, per the declared set, and the reduction is
  recorded in §3. `PARITY_CHECKPOINTS=1,2,…,30` is the only change needed.
- **The two compile-refused models were not brought into the BigQuery half.** They stay a
  `20260906-bigquery-correctness` question, exactly as in phase 13.
- **No equivalence violation was fixed, because none was found.** Finding 2 is a measurement
  handed on, not a fix withheld.
- The live sweep is **not** promoted into CI (standing decision: no BigQuery CI tier). Its
  committed report is what runs per-PR.
