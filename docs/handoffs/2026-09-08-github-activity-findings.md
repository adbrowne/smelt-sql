# GitHub-activity pipeline findings — both halves

**Status:** complete. **Both halves have landed.** The offline half — the four root causes,
the divergence registry and the requirements handed to the two downstream feature outcomes —
was banked 2026-09-08 from `examples/github_activity/`'s DuckDB replay over the committed
30-day Parquet fixture. The **live-BigQuery addendum is dated 2026-09-12** and begins at
"## The live BigQuery half": it carries the compile refusals, runtime failures,
cross-target comparison and full-refresh-oracle result of phases 10–14 and 17 run against
`smelt-bq-test-20260816.smelt_dogfood`. Everything above that heading was produced with no
live warehouse; everything below it was produced by runs that were.

**Source of every claim below:** `docs/outcomes/20260906-bigquery-dogfood-spine/phases/`
`0{2,3,4,6,8,9}-summary.md` for the offline half, and `10-`, `11-`, `12-`, `17-`, `13-`
and `14-summary.md` for the live half, plus that outcome's own "## Decision log" and
`examples/github_activity/README.md`. Nothing here is re-derived or re-measured; each
number is traceable to one of those.

## The four root causes

1. **Succession naming-tie fold** — `silver.repo_naming` and `silver.actor_naming`.
   The incremental window-forward patch loop addresses the presented table by
   `(key, clock)` (its `MERGE ... ON` clause), so a redelivered duplicate or a genuine
   same-second tie whose payload agrees converges to one presented row. `--full-refresh`
   re-runs the model's raw compiled `SELECT` (`emit_succession_full_rebuild`) with no such
   addressing, keeping every physically duplicated row. Measured exactly: 139 extra rows
   for `repo_naming`, 145 for `actor_naming`, matching the fixture's own same-second tie
   counts (`phases/03-summary.md`). `marts.naming_history` is unaffected — its `LAG` filter
   drops the duplicate identically on both legs. **Latent, unmeasured**: the rebuild path
   never runs the clock-tie probe at all, so a content-*disagreeing* tie (not just a
   byte-identical one) would silently corrupt a full refresh; this fixture measured zero
   disagreeing ties, so the gap stays undetected rather than exercised
   (`phases/03-summary.md`, `crates/smelt-cli/tests/github_activity_oracle.rs`'s
   `DIVERGENCE_REGISTRY` doc comment).

   **Fixed** by `docs/outcomes/20260906-bigquery-correctness/phases/03-plan.md`:
   `emit_succession_full_rebuild` folds its presented rebuild on `(key_cols, clock_col)`
   with a `MAX` aggregate over every other output column, and runs the clock-tie probe
   over its own whole-source scope before the presented write — closing both the row-count
   divergence and the previously-latent gap in the same paragraph above.

2. **Enrichment freeze** — `gold.events_enriched`. No `UpstreamMutation(gold.repo_dim)`
   maintenance cell is ever derived
   (`crates/smelt-logical/src/maintenance/derive/model_edge.rs::append_model_edge_cells`):
   the key-addressed route needs the *downstream's own* declared `unique_key`, which
   `gold.events_enriched` (`grain: partition`) has none of by construction, and the
   clock-based route needs the upstream to declare `timeseries:`, which `gold.repo_dim`
   (clockless) does not. Concretely, renaming a repo never refreshes
   `current_repo_name` on already-written rows through any tracked technique — silent,
   surfacing only via `smelt explain --json` (`RepairKeysNotDiscoverable`), never at `run`
   time. Measured over the full 30-day fixture (`every_window_deep_sweep`,
   `phases/08-summary.md`): the stale-row count is strictly non-decreasing across all 30
   days — `1, 1, 2, 5, 5, 8, 13, 15, 16, 22, 22, 22, 28, 28, 28, 29, 29, 29, 31, 36, 36,
   37, 37, 37, 37, 38, 38, 38, 39` — zero rows ever heal. This corrects an earlier, wrong
   phase 6 claim ("self-heals") that turned out to be a row-count check, not a content
   check.

   **Fixed** by `docs/outcomes/20260906-bigquery-correctness/phases/04-plan.md` and
   `phases/05-plan.md`: a new enrichment-keyed route in `append_model_edge_cells` derives
   an `UpstreamMutation(gold.repo_dim)` / `Technique::ColumnScopedMerge` cell addressed by
   the join key the downstream itself carries (phase 4), and the run path dispatches it
   once per run over the model's unwindowed output (phase 5) — `gold.events_enriched`'s
   `current_repo_name` now heals and the stale-row count reaches zero.

3. **Oracle windowing gap** — `silver.actor_sessions`. `compute_calendar_windows`
   (`crates/smelt-runtime/src/windowing.rs`) applies the Form-B forward-reach rebase only
   at the two *outer* edges of a single multi-day invocation, never at an interior chunk
   boundary. A wide `--full-refresh` is exactly such an invocation, so the
   **full-refresh oracle itself under-counts** a cross-midnight session — the incremental
   leg is correct (confirmed against a from-scratch raw-SQL recomputation,
   `phases/08-summary.md`). Scope: any single invocation of a Form-B model spanning more
   than one partition chunk is affected, not just `--full-refresh` — a very wide ordinary
   incremental backfill window could show the same undercount.

   **Fixed** by `docs/outcomes/20260906-bigquery-correctness/phases/06-plan.md`: two
   changes, not one. `compute_calendar_windows` now folds the skew into every interior
   batch's own `filter_start`/`filter_end` (clamped to the invocation's outer scan
   envelope, `docs/specs/incremental_shapes.md` §"Execution model (DuckDB)"), but those
   fields turned out to have no consumer on the real execute path — `derive_batch_filtered_sql`
   (`crates/smelt-runtime/src/execute/sources.rs`) built its per-source scan pushdown from
   the batch's own unwidened output range (`run_range`, i.e. `partition_start`/
   `partition_end`) instead, the actual mechanism the divergence traced to. The fix threads
   a second, separate `scan_range` parameter (sourced from `IncrementalBatch::filter_start`/
   `filter_end`) through `derive_batch_filtered_sql` and its three call sites, so the
   per-source pushdown widens by the model's own skew on an interior chunk while the output
   clamp stays exact. The two legs now compare exactly equal on `silver_actor_sessions` —
   no `DIVERGENCE_REGISTRY` entry remains for it.

4. **Mart repair gap** — `marts.daily_active_contributors`. This Form-A downstream
   aggregate has no rebase of its own and never revisits an already-written partition, so
   it never learns when `actor_sessions`'s own (correct) Form-B rebase rewrites an earlier
   partition. `total_events` is frozen at first-write time — a strict subset of the
   oracle. Not a missing-repair-edge shape like root cause 2's, despite the earlier
   resemblance: the maintenance cell for this edge already exists (it is clocked, so
   `append_model_edge_cells`' existing clock route derives it) — the gap is that no run
   ever dispatches that cell over the partitions the upstream actually rewrote, because
   `build_model_plans` gave every model the invocation's requested run window verbatim,
   never learning that a Form-B upstream selected in the same run rebased a wider window
   (`phases/08-summary.md`; mechanism confirmed by inspection in
   `docs/outcomes/20260906-bigquery-correctness/phases/07-plan.md`).

   **Fixed** by `docs/outcomes/20260906-bigquery-correctness/phases/07-plan.md`: a model's
   run window is now the union of the requested window and the derived output window of
   every upstream maintained model selected in the same invocation
   (`docs/specs/model_transforms.md` §Semantics "The derived output window propagates
   within a run."), applied in `build_model_plans` before the `frozen_horizon` clamp. A
   `[D, D+1)` run over `marts.daily_active_contributors` now widens to
   `actor_sessions`'s own rebased `[D-1, D+2)` output window, so the mart re-runs over the
   same partitions the upstream rewrote and the two legs compare exactly equal —
   no `DIVERGENCE_REGISTRY` entry remains for it.

## The registered divergences

Root cause 1 (`silver_repo_naming` / `silver_actor_naming`) is **fixed**, not registered:
`docs/outcomes/20260906-bigquery-correctness/phases/03-plan.md` folded
`emit_succession_full_rebuild`'s presented rebuild on `(key_cols, clock_col)`, the same
addressing the patch loop's `MERGE ... ON` clause uses, so the two legs now compare exactly
equal on both relations — no `DIVERGENCE_REGISTRY` entry remains for either. The rebuild
path also now runs the clock-tie probe (previously latent — see below) before its
presented write.

Root cause 2 (`gold_events_enriched`) is also **fixed**, not registered:
`docs/outcomes/20260906-bigquery-correctness/phases/04-plan.md` and `phases/05-plan.md`
derive and dispatch the enrichment-keyed `UpstreamMutation(gold.repo_dim)` cell, so
`current_repo_name` now heals and the two legs compare exactly equal on this relation too —
no `DIVERGENCE_REGISTRY` entry remains for it.

Root cause 3 (`silver_actor_sessions`) is also **fixed**, not registered:
`docs/outcomes/20260906-bigquery-correctness/phases/06-plan.md` threads the model's own
skew into every interior chunk's source-scan pushdown (`derive_batch_filtered_sql`'s new
`scan_range` parameter), so the two legs now compare exactly equal on this relation too —
no `DIVERGENCE_REGISTRY` entry remains for it.

Root cause 4 (`marts_daily_active_contributors`) is also **fixed**, not registered:
`docs/outcomes/20260906-bigquery-correctness/phases/07-plan.md` widens a downstream
model's run window to cover every in-run upstream's derived output window, so a run
touching `actor_sessions`'s rebased partitions also re-runs this mart over them — the two
legs now compare exactly equal on this relation too — no `DIVERGENCE_REGISTRY` entry
remains for it.

**`DIVERGENCE_REGISTRY` is now empty** — all four root causes are fixed, not registered.
`an_unregistered_divergence_fails` still enforces exact equality on any future genuine
divergence directly against every materialised relation, not via a lookup into this
(now-empty) registry, so it does not pass vacuously. This is checked, not just asserted:
`docs/outcomes/20260906-bigquery-correctness/phases/08-plan.md` added
`assert_matches_oracle_fails_closed_on_an_empty_registry` (drives the
registry-consulting comparator itself over a perturbed relation and checks it reports
the mismatch), `check_bound_accepts_a_holding_bound` /
`check_bound_rejects_a_leading_side` / `check_bound_rejects_divergence_outside_the_
licensed_columns` (exercise `check_bound`'s `MonotoneDivergence` arm and both `Side`
variants, otherwise dead on an empty registry), and `no_relation_diverges_unexplained`
(names the zero-unexplained-count claim directly over the full 30-day fixture). An empty
`DIVERGENCE_REGISTRY` table here means "measured and found nothing to register," not
"never measured."

## Latent, unmeasured

Fixed by `docs/outcomes/20260906-bigquery-correctness/phases/03-plan.md`:
`emit_succession_full_rebuild` now runs the clock-tie probe over its own whole-source scope
before its presented write, so a content-*disagreeing* tie (two rows sharing `(key, clock)`
whose other columns differ) is refused rather than silently folded. This fixture still
measures zero disagreeing ties, so the refusal path itself remains unexercised by this
fixture, even though the mechanism is now wired.

## Requirements handed to `20260906-external-dag-steps`

`scripts/bq-dogfood-loader.sh` is external to smelt by this outcome's own "Out of scope"
section: smelt's `raw.github_events` / `raw.github_events_arrival` source declarations are
the contract the loader is trusted against, but nothing in the smelt graph knows the
loader exists or runs on a schedule (`phases/09-summary.md`). What this costs today: a
reader of the smelt project cannot see, from smelt alone, that these two sources are
populated by an external scheduled query rather than by another smelt model or an
ungoverned manual load — the dependency is documented only in the source YAMLs'
`description:` prose and in this outcome's docs, not in anything smelt's graph, `explain`,
or lineage tooling can traverse. A `produced_by:` declaration on a source would need to
express, at minimum: (a) an external identifier for the producing job (the loader's own
derivation is a per-day `_TABLE_SUFFIX` slice plus a previous-day redelivery arm — see
`scripts/bq-dogfood-loader.sh::emit_sql`), so lineage tooling has something to point at;
(b) enough of the producer's own cadence (day-partitioned, one run per day) to let a
future staleness check compare "when did this source's data last land" against "when did
the declared producer last run" — which is exactly the axis this pipeline has no
column for today (see the `lateness: '6 hours'` placeholder in root cause discussion,
`phases/02-summary.md`); and (c) the redelivery arm's shape (`MOD(CAST(id AS BIGINT), 50)
= 0` over day D-1) as a *declared* fact rather than a comment the loader script and the
source YAML's `key_recurrence.window: '0 days'` must be kept in sync by hand.

## Requirements handed to `20260906-trimmed-history-sources`

The loader declares a **45-day** `partition_expiration_days` bound
(`scripts/bq-dogfood-loader.sh`, parsed from `README.md`'s "The BigQuery loader" section),
derived from the 30-day fixture range plus backfill headroom (`phases/09-summary.md`).
This is unrelated to and inconsistent with the pre-existing, inert
`retention: '90 days'` field on both `examples/github_activity/models/sources/raw/
github_events.yml` and `github_events_arrival.yml` — that field is parsed into
`smelt-core`'s `SourceDefinition` today but consumed by no maintenance logic (confirmed by
grep, no reader outside test fixtures). This outcome's phase 9 fixed the stale claim in
`github_events.yml`'s comment (it previously and incorrectly said the two numbers match)
but left the value itself alone, since owning the actual trimmed-history mechanism and
reconciling 45 vs. 90 belongs to `trimmed-history-sources`. That outcome must either: make
`retention:` a real, enforced bound and pick one number (with a stated reason for
diverging from the loader's 45, if it does), or otherwise formally connect the two so a
future reader cannot again find them silently disagreeing.

## Punch-list for `20260906-bigquery-correctness`

1. **Done** (`docs/outcomes/20260906-bigquery-correctness/phases/03-plan.md`). The
   succession full-refresh rebuild path (`emit_succession_full_rebuild`) now folds on
   `(key_cols, clock_col)` with an aggregate over every other column, closing the
   `silver_repo_naming` / `silver_actor_naming` divergence — provoked by `silver.repo_naming`
   and `silver.actor_naming`. (An exploratory `SELECT DISTINCT *` wrap was tried and found
   insufficient — see `phases/03-summary.md` — the real fix needed the full output schema
   threaded into the emitter.)
2. **Done** (`docs/outcomes/20260906-bigquery-correctness/phases/04-plan.md`,
   `phases/05-plan.md`). A new enrichment-keyed route in `append_model_edge_cells` derives
   the missing `UpstreamMutation(gold.repo_dim)` cell for a `grain: partition` downstream
   reading a clockless keyed-model dimension, and the run path dispatches it once per run
   over the model's unwindowed output — `gold.events_enriched`'s `current_repo_name` now
   heals and the stale-row count reaches zero. `Refusal::RepairKeysNotDiscoverable` also
   gained a real `DiagnosticCode` for the fail-closed leg that survives the new route.
3. **Done** (`docs/outcomes/20260906-bigquery-correctness/phases/06-plan.md`).
   `compute_calendar_windows`'s interior-chunk-boundary forward-reach loss for Form-B
   models is fixed — provoked by `silver.actor_sessions`, and by any other Form-B model
   materialized in one invocation spanning multiple partition chunks. See root cause 3's
   own **Fixed** paragraph above for the two-part mechanism.
4. **Fix-or-register** the missing repair edge from a Form-B model's own self-rebase to a
   Form-A downstream aggregate that reads it verbatim — provoked by
   `marts.daily_active_contributors`. Worth checking whether the fix for item 2 (a
   general "downstream must learn an upstream rewrote an already-materialised row"
   mechanism) naturally covers this too, or whether they need separate maintenance-cell
   work.

## Criterion 6 — the two known live conformance failures

Both `dags_bigquery::diamond_propagation_suffices_on_bigquery` and
`gate_composed_bigquery::composed_keyed_pool_upholds_equivalence_on_bigquery` are already
**fixed and live-confirmed** in the repo record — neither was open when this outcome's phase 9
looked. What phase 9 added is the durable, offline half: a gate that would have caught the
diamond mechanism before any live sweep ever needed to run, plus the retirement of the stale
"uncharacterised"/"not yet re-confirmed" wording that still sat in the BigQuery conformance
binary's own doc comments.

| Test | Mechanism | Fix | Live evidence |
|---|---|---|---|
| Test: `diamond_propagation_suffices_on_bigquery` | `diamond_dag`'s `ParityFilter` body renders `WHERE id % 2 = 0`; GoogleSQL has no infix `%` (`400 Syntax error: Expected ")" but got "%"`, measured live 2026-08-19). Chasing it found the worse sibling: infix `^` is bitwise XOR on GoogleSQL, so it returns a *different number* rather than erroring. | `7a2eb89d0` (`%`→`MOD`), `af972abe0` (`^`→`POWER`) | targeted re-run 2026-08-19 (231.65s, pass); whole sweep 2026-08-21 (21/21); concurrent sweep 2026-08-22 (22 cases) |
| Test: `composed_keyed_pool_upholds_equivalence_on_bigquery` | No mechanism of its own — collateral from three already-closed gaps it reached in one case: the keyed-fold `MERGE`'s not-matched arm hardcoding `INSERT *` (`build_cumulative_merge_sql` took no dialect), `Backend::execute_model`'s unconditional `DROP VIEW`/`DROP TABLE` across an object-type mismatch, and the composed route-3 delta query's hand-rolled `FROM (VALUES …) AS t(...)` row set. | `0178e6bd4`, `d84320a44`, `e028596e3`/`aee113753` | confirmed live 2026-08-19 sweep (14/21, this case in the passing set); same two sweeps above |

**Offline gates now holding each mechanism**, so a regression in either fails before any live
sweep is needed:

- `cargo test -p smelt-maintenance-testkit --test googlesql_render` — new this phase. Parses
  and prints every `DagBody` variant's rendered body (all six DAG recipes, all eight `DagBody`
  variants) and the composed keyed pool's rendered bodies (all four `ComposedRoute`s) under the
  BigQuery dialect, and asserts no refused construct (infix `%`, infix `^`, `MEDIAN(`,
  `VARCHAR`, `DOUBLE`, `EXCEPT ALL`, `FROM (VALUES`) survived. This is the gate that would have
  caught `diamond_propagation_suffices` offline, before it ever reached a live warehouse. A
  negative control (`the_refused_construct_scan_is_not_vacuous`) and a fail-loud check on an
  unparseable body (`a_body_that_does_not_parse_fails_loud`) guard the scanner itself against
  passing vacuously.
- `cargo test -p smelt-dialect --test modulo_lowering --test power_lowering` — the `%`→`MOD`
  and `^`/`**`→`POWER` lowerings `googlesql_render` depends on.
- `cargo test -p smelt-backend --test merge_columns_guard` (`require_merge_columns`) and
  `no_family_hardcodes_a_backend_dialect` (`crates/smelt-maintenance-testkit/src/families/mod.rs`,
  `dags.rs`) — hold the composed-pool collateral fixes' own dialect-awareness.

**The one genuinely unrunnable item**: re-confirming green at today's HEAD. The live leg needs
credentials this loop does not have, and phases 1-8 of this outcome touched maintenance
emitters since the last live sweep (2026-08-22). This is recorded as a dated, named debt —
`docs/specs/multi_backend.md` §"Known Divergences" "The BigQuery conformance leg's live
evidence has a date" — rather than skipped green; it belongs to the
`20260906-bigquery-dogfood-spine` outcome's blocked live half (its phase 16).

## Close-out (2026-09-08)

`docs/outcomes/20260906-bigquery-correctness` closed its own phase 10 by verifying every
success criterion's evidence at HEAD rather than re-deriving it. One row per criterion,
naming the artifact and the gate that holds it:

| # | Criterion | Artifact | Holding gate |
|---|---|---|---|
| 1 | The unconditional fix | `emit_fingerprint_digest_select` threads `dialect` to `row_fingerprint_expr` (phase 1) | per-dialect unit tests in `crates/smelt-logical/src/maintenance/emit/fingerprint.rs` |
| 2 | Punch-list harvested, not invented | This handoff's four punch-list items, carried into rows 3-7 verbatim (phase 3 planning) | none needed — a process check, not a code gate |
| 3 | Every fixed construct is gated | Phases 1-9's per-construct tests, plus the new structural scan for the defect class itself | `cargo test -p smelt-logical --test maintenance_dialect_blindness` (new, phase 10) |
| 4 | Ratchets move the right way | `.claude/dialect-gaps-baseline.txt` / `.claude/parser-gaps-baseline.txt`, held with a dated note (phase 10) | `cargo test -p smelt-db --test dialect_audit -- gap_count_ratchet` |
| 5 | Cross-target agreement | `DIVERGENCE_REGISTRY` emptied by phases 3-7; fail-closed proof added in phase 8 | `cargo test -p smelt-cli --test github_activity_oracle` (`assert_matches_oracle_fails_closed_on_an_empty_registry`, `no_relation_diverges_unexplained`) |
| 6 | The two known live conformance failures characterised | §"Criterion 6" table above (phase 9) | `cargo test -p smelt-maintenance-testkit --test googlesql_render`, `-p smelt-dialect --test modulo_lowering --test power_lowering` |
| 7 | Gates green | This close-out's own run (phase 10) | `verify-phase.sh` + the six named crate-level gates below |

**Stayed unverified, deliberately, per Out of scope:**

- The BigQuery **value** leg (`scripts/bigquery-dialect-audit.sh`) — needs a live warehouse
  this loop does not have. Already a dated, named debt in `docs/specs/multi_backend.md`
  §"Known Divergences".
- The 42 no-verdict `BuiltinRegistry` entries tracked by issue #179 — no spine model
  reaches them (the spine's live-BigQuery half, its phase 16, never ran), so giving any of
  them a verdict here would be speculation criterion 2 forbids. They stay on #179.

**Gates run at phase 10 HEAD** (see `phases/10-summary.md` for verbatim counts):
`verify-phase.sh`; `smelt-logical --test maintenance_dialect_blindness` (new); `smelt-cli
--test github_activity_oracle`; `smelt-db --test dialect_audit`; `smelt-dialect --test
emission_ownership`; `smelt-runtime --test dialect_seam --test
projection_dialect_invariance`; `smelt-maintenance-testkit --test googlesql_render`;
`large-file-check.sh`.

## Close-out of the reopening (2026-09-11)

`docs/outcomes/20260906-bigquery-correctness` reopened on 2026-09-10 for the T5
ledger-substrate gap, and its phase 16 closed it by running `examples/github_activity`
live on BigQuery against `smelt-bq-test-20260816.smelt_dogfood`. One row per criterion,
naming the artifact and the gate:

| # | Criterion | Artifact | Holding gate |
|---|---|---|---|
| 3 | Every fixed construct is gated | The live run's four defects, each with a gate that would have caught it (phase 16) | `cargo test -p smelt-runtime --test maintenance_sql_dialect_purity` (new, 8 tests); `cargo test -p smelt-backend-bigquery --test ledger_gate` (new); the conflict classifier's pair in `smelt-backend-bigquery/src/sql.rs` |
| 4 | Ratchets move the right way | `dialect_gaps_bigquery` **held** at 42 with a dated note; `duckdb_seed_gaps 0` unchanged; large-file baseline unchanged (two files split, neither raised) | `cargo test -p smelt-db --test dialect_audit -- gap_count_ratchet`; `large-file-check.sh` |
| 5 | Cross-target agreement | All four defects **fixed**, none registered — each was smelt emitting DuckDB SQL to BigQuery, a bug with a right answer rather than a difference between engines | `cargo test -p smelt-cli --test github_activity_oracle` (registry still empty and still fails closed) |
| 7 | Gates green | Phase 16's own run | `verify-phase.sh` + the crate gates in `phases/16-summary.md` |
| 8 | Plan and run layers agree about state | Unchanged from phase 11; no guard was added or removed | `cargo test -p smelt-runtime --test state_guard_census` |
| 9 | BigQuery runs the pipeline's real plan | 14 models, twice, at default `--jobs`, with observed deltas, both ledgers and the tombstone ledger all realised and read back from the warehouse | the live run itself; `example_diagnostics` holds the no-downgrade claim offline |

**The four defects** (detail in `phases/16-summary.md`): a hardcoded `VARCHAR` in the
driver's changed-key projection; a typed `DATE '…'` literal in the succession window
predicate; a DuckDB ledger `CREATE TABLE` reached from `execute/project`; and BigQuery
cancelling concurrent transactions over `_smelt_ledger`, which made a parallel run lose
models at random — the same write-conflict detection that makes the never-fold-twice
refusal sound there.

**The eight inherited checks** — four verified live, four not exercised:

| # | Check | Verdict |
|---|---|---|
| 1 | Untyped `NULL` coercion in the domain union | verified (the union type-checks; no delete rows in this source) |
| 2 | The patch `MERGE` as a whole | verified (both succession models, twice) |
| 3 | The transactional rebuild is really rejected | not exercised — `SourceRetentionExceeded` refuses `--full-refresh` while stored output exists |
| 4 | `SMELT_LEDGER_ALREADY_REFLECTED` survives the error envelope | not exercised — no `Grade::Additive` cell in this model set |
| 5 | `@@row_count = 0` on a repeat `MERGE` | not exercised — same reason |
| 6 | First-run DDL inside the bookkeeping transaction | not exercised — every target already existed |
| 7 | Arrow list shape for a `REPEATED STRING` | verified (decoded; the decoder now errors rather than returning empty) |
| 8 | `ARRAY<STRING>` accepts a fully-suppressed window's row | verified (read back: one row, both arrays empty) |

Checks 3–6 are each blocked by a property of a *long-lived* dataset (stored output exists;
every target exists), not by a defect. The cheap route for a future phase is the
integration suite's ephemeral dataset.

**Still not proven, and it is not this outcome's:** dual-target **value** parity. The
BigQuery leg holds three days and the DuckDB fixture thirty, so equal row counts are not
expected; comparing the two populations is the spine's phase 13, which now has 14
comparable models.

## References

- `docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md` — outcome header, criteria,
  and "## Decision log" (all dated 2026-09-08 unless noted).
- `docs/outcomes/20260906-bigquery-dogfood-spine/phases/0{2,3,4,6,8,9}-summary.md`.
- `crates/smelt-cli/tests/github_activity_oracle.rs` — `DIVERGENCE_REGISTRY` (the
  machine-checked source of truth for the five divergences' exact predicates).
- `examples/github_activity/README.md` — "Trusting the numbers", "The BigQuery loader".

## Findings handed back from 20260906-external-dag-steps (2026-09-08)

`docs/outcomes/20260906-external-dag-steps` closed its own phase 9 by verifying all seven
success criteria's evidence at HEAD (see that outcome's `phases/09-summary.md` for the full
criterion → evidence table) and hands the following back for the spine's blocked
live-BigQuery phases to pick up:

**(a) The loader contract as it actually shipped.** The GitHub-activity day-loader is no
longer three drifting copies — it is one script,
`examples/github_activity/load_day.sh`, declared as a black-box step by
`examples/github_activity/models/sources/raw/github_loader.yml`'s `external_step:` block,
producing both `raw.github_events` and `raw.github_events_arrival`. It is idempotent per
day via its own `main._loader_days` ledger (a day already loaded is a no-op), and is
invoked by `smelt run`/`smelt build` whenever a run selects a model reached from either
produced source — the run orders the step ahead of its consumers and fails the run (naming
the step, downstream models left unbuilt) on a non-zero exit. When the spine's live-BigQuery
phases (10-14) stand this pipeline up against the dogfood project, this is the step that
must actually execute the `bq query`/`gcloud` invocation named in `command:` — smelt orders
and invokes it but never authors or inspects its SQL.

**(b) `smelt list --format json` hard-fails on all three example workspaces, pre-existing,
unrelated to external steps.** `ListError::ParseErrors` fires on `examples/github_activity`,
`examples/web_analytics`, and `examples/retail_analytics` alike: `load_workspace` discovers
root-level utility scripts (`sample.sql`, `setup_sources.sql`) as SQL models project-wide,
and `smelt list` treats a parse error anywhere in the discovered set as fatal rather than
scoped to the selected models. Verified pre-existing (reproduces with this outcome's changes
stashed out). Two candidate fixes, neither attempted here: scope `ListError::ParseErrors` to
the selected set, or give `load_workspace` callers a way to exclude non-model root SQL from
discovery. Recorded in `docs/TODO.md`. The spine's live-run phases should use
`smelt explain --json` (unaffected) rather than `smelt list --format json` if they need a
machine-readable node listing before this is fixed.

**(c) A dry run, or any embedder setting `invoke_external_steps: false`, refuses rather than
runs stale.** `ExecuteRequest::invoke_external_steps` defaults to `true` (a live `smelt
run`/`smelt build` invokes steps normally); setting it `false`, or `dry_run: true`, makes a
run that reaches a step refuse with a named diagnostic instead of reading a possibly-stale
produced table. The spine's live-BigQuery phases will hit this refusal the moment they preview
a plan (e.g. `--dry-run`) before actually loading `raw.github_events` — that is by design, not
a bug: the whole point of the black-box-step contract is that smelt will not read a source a
step produces without either running the step or being told explicitly it is safe not to.

**(d) The UI plan-preview opt-out named in phase 4's planning decision log was never built.**
`smelt-ui/src/run_manager.rs::to_runtime_request` always sets `invoke_external_steps: true`
because it only drives the live-run path today — no UI call site sets it `false`. If a UI
plan-preview endpoint is built later (for the spine's dogfood-project UI work or otherwise),
it must set `invoke_external_steps: false` (or `dry_run: true`, which also refuses) to avoid
silently invoking a step — for example, a `bq query` against the dogfood project — from what
looks like a read-only preview action.

## The live BigQuery half

*Added 2026-09-12. Everything in this section and below it was produced by runs against
`smelt-bq-test-20260816.smelt_dogfood`, under ADC impersonating
`smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com`.*

What was provisioned, loaded, widened and run, in phase order — each row's numbers are the
phase summary's own, read from BigQuery job metadata rather than estimated:

| phase | what happened | cost |
|---|---|---|
| 7 | `smelt_dogfood` created in the existing project, `defaultTableExpirationMs` **absent** (read back from the API, not inferred from a successful create); `smelt-dogfood@` holds `roles/bigquery.jobUser` project-wide plus `WRITER` on that one dataset; a project-scoped AUD 25/month budget; a query job created in the dogfood project and **refused** in both of the human's other projects | — |
| 10 | the loader deployed: `github_events` and `github_events_arrival` created day-partitioned on `created_at` / `ingested_date`, both `timePartitioning.expirationMs = 3888000000` (45 days) read back from `tables.get`; two days loaded, 6,053 rows, `payload` non-null on every one | US$0.0173 + US$0.0183 (≈ US$0.018/run, ≈ US$0.53/month at one run/day) |
| 11 | first live run: `bronze.events` and the first write of the three silver keyed models succeeded, then a hard stop at `silver.events_deduped` on the T5 observed-delta gap | 94 MB billed, ≈ US$0.0005 |
| 12 | a full refresh plus **three consecutive incremental windows**, each with a run report, each exit 0, **10 of 16 models**; `silver.events_deduped` reached 5,990 rows — the source's exact distinct-`id` count — so the deliberate redelivery folded once, live; the empty W3 correctly changed nothing | 1.111 GB over 258 jobs, ≈ US$0.0056 |
| 17 | the population widened to the committed fixture's thirty days: 28 days loaded with `scripts/bq-dogfood-loader.sh --emit-sql`'s own output, every one of the 56 `INSERT`s returning the fixture's expected count on the first attempt; the 75-row 2026-08-04 residue deleted by explicit predicate. `smelt_dogfood.github_events` holds **65,583 rows over 30 partitions with 64,313 distinct ids**, byte-identical to the committed fixture on `(id, created_at, type, actor_id, repo_id, repo_name, payload length, payload MD5)` — compared in full, zero rows differing, all 28 redelivery slices matching. `githubarchive` has **not** drifted from the fixture | 90.60 GB, US$0.45 |
| 13 | the fixture's **thirty** windows on both targets over that one shared population; at the final window all **fourteen** compared relations byte-equal in both directions of a whole-row `EXCEPT ALL`; thirteen of fourteen equal at *every* compared checkpoint; `silver.events_deduped` reached 64,313 rows across twenty-nine window boundaries | 58.46 GB over 2,160 jobs, US$0.29 |
| 14 | the full-refresh oracle leg on a third target (`bigquery_oracle`, dataset `smelt_dogfood_oracle`, created and dropped inside the phase): at the final window all **fourteen** relations byte-equal to their own full refresh, empty divergence registry; eight of fourteen equal at every one of the seven checkpoints | 12.0 GB over 1,438 jobs, US$0.06 |

The whole live programme — provisioning through the oracle leg — cost **≈ US$0.84**, of
which US$0.45 is phase 17's one-off widening scan of `githubarchive`.

**Criteria 5, 6 and 7 are met, and this is what they rest on.**

- **Criterion 5** (live BigQuery: a full refresh then at least three consecutive incremental
  windows, run reports captured) — phase 12, four runs, all exit 0, reports quoted verbatim
  in `phases/12-summary.md`. It was met at 10 of 16 models; phases 13 and 14 later ran the
  same pipeline at 14.
- **Criterion 6** (the two targets agree) — phase 13. Fourteen relations byte-equal at the
  final window, gated per-PR over the committed `13-parity.json` by
  `the_two_targets_agree_at_the_final_window`.
- **Criterion 7** (each incremental state equals a full refresh over the inputs seen so far,
  on both targets) — phase 14 on BigQuery, plus `every_window_matches_the_full_refresh_oracle`
  on DuckDB (30 windows, 16 models, 317.6 s).

**Coverage is stated, not implied.** The BigQuery half is **14 of 16** models — two,
`silver.actor_sessions` and its only downstream `marts.daily_active_contributors`, are
refused at compile time on GoogleSQL over an INTERVAL `RANGE` lookback frame, and are
excluded from both legs so the relation sets are equal by construction. It is checked at
**7 of 30** windows (1, 2, 3, 5, 10, 20, 30; the set phase 13 measured and phase 14 reused,
so both phases' claims are about the same state). The DuckDB half is 16 models at all 30
windows.
Six of the fourteen are additionally exempt from the *intermediate* oracle comparisons, each
with a checkable proof recorded per row in `14-equivalence.json` — see live finding 1, which
is why. Nothing is exempt at the final window, and a gate fails if anything ever is.

There is **no BigQuery CI tier**, by standing decision: the live sweeps write committed
reports (`13-parity.json`, `13-parity-attribution.json`, `14-equivalence.json`) and it is
those that are read per-PR.

## Live-BigQuery findings

Every row is harvested from a committed phase summary. "closed" means the defect is fixed
and gated today, not that it was dismissed.

| finding | provoking model | provoking statement | classification | owner |
|---|---|---|---|---|
| `--event-time-end` does not bound a full refresh's source scans, so a full refresh is **not** a window-bounded oracle for six of fourteen relations: their oracle at window 1 is byte-identical to their oracle at window 30 (phase 14) | `gold.repo_dim`, `silver.repo_naming`, `silver.actor_naming`, `marts.naming_history`, `gold.events_enriched`, `marts.repo_leaderboard` | `smelt run --target bigquery_oracle --full-refresh --event-time-start 2026-08-05 --event-time-end <day k+1>` | open — a product question, measured rather than argued; invisible on DuckDB, whose oracle stages a truncated source | `20260906-bigquery-correctness` |
| Cost is jobs, not rows: ≈5.1 jobs/model/run and a ~5 s per-job floor, so a model writing 2 rows can cost six times another model writing 2 rows | `silver.issue_events` (`deleteinsert`, 2 rows, 33 s) vs `marts.star_growth` (`full_refresh`, 2 rows, 5.5 s) | the per-model execution recorded across phase 13's thirty per-window `smelt run --target bigquery` reports (derivation in `phases/16-plan.md`) | open — **derived** from those reports, not measured against `INFORMATION_SCHEMA.JOBS` (the dogfood SA lacks `bigquery.jobs.list`) | `20260906-bigquery-unattended` |
| The shared `_smelt_ledger` serialises every model's bookkeeping — BigQuery aborts a transaction mutating a table another in-flight transaction is mutating, so a parallel run loses models at random; every ledger access is already model-scoped | every maintained model; surfaced through `silver.repo_naming` in a parallel run | the per-model bookkeeping transaction's write into `smelt_dogfood._smelt_ledger` | open — filed as [#203](https://github.com/adbrowne/smelt-sql/issues/203) with its evidence; not restated here | issue #203 |
| An empty incremental window costs as much as a full one — every model still re-scans its inputs | `gold.repo_activity_daily` | W3, `smelt run --target bigquery --start 2026-08-07 --end 2026-08-08` (230,686,720 bytes billed, identical to W2, which landed 2,714 events) | open — operational economics for a scheduled idle run | `20260906-bigquery-unattended` |
| The run window need not match partition granularity (`docs/specs/incremental_shapes.md` §"Run window vs partition granularity"), so the thirty daily windows were a schedule *choice*, not a requirement — with the caveat that the saving is batch-safety-class-dependent | `gold.repo_activity_daily` | the thirty per-window `smelt run --target bigquery --start D --end D+1` schedule | operational note | `20260906-bigquery-unattended` |
| `stage_workspace` copied the gitignored `.smelt/` into staged workspaces, so a staged run inherited a developer's local posture baseline and failed where a fresh clone and CI could not reproduce it | `silver.repo_naming` (the append-only posture probe) | the staged-workspace copy in `crates/smelt-cli/tests/github_activity_support/mod.rs::copy_dir_all` | **closed** (phase 13): `target/` and `.smelt/` are now skipped by name, with the reason inline. Recorded because the class recurs | closed — this outcome |
| A concurrent `cargo test -p smelt-cli` rebuilt `target/debug/smelt` **without** `--features bigquery` mid-run and killed a live leg at window 3; the refusal happens at backend construction, so no partial window was written | the whole thirty-window leg (the refusal precedes model execution) | `Error: BigQuery backend not available. Rebuild with --features bigquery` | **closed** operationally: `scripts/bq-dogfood-parity.sh` honours `SMELT_BIN` and `PARITY_RESUME_FROM` | closed — `scripts/bq-dogfood-parity.sh` |
| Dataset creation is outside the dogfood SA's grant — deliberate, from phase 7's provisioning design; the grant was not widened | the `bigquery_oracle` target's dataset (no model) | `CREATE SCHEMA smelt_dogfood_oracle` → `Access Denied: … does not have bigquery.datasets.create permission` | boundary, working as intended | closed — dataset lifecycle lives in `scripts/bq-dogfood-oracle-dataset.sh`, under the human credential |
| T5 observed-delta recording was DuckDB-only and refused unconditionally, stopping the whole model set (`silver.events_deduped` is upstream of everything) | `silver.events_deduped` | `maintenance_driver/driver.rs:634-641`, `Feature not supported by BigQuery: observed-delta recording for a change-suppressed keyed fold (T5)` | **closed** — reconciled into a recorded downgrade rather than a `bail!` | `20260906-bigquery-correctness` (closed) |
| The append-only posture probe could not be planned by BigQuery at all: the baseline is grouped by the **raw** partition column rather than the declared `granularity: day` (5,797 "partitions" for a 3-day source), and BigQuery's inline row set is one subquery per row, producing a 692,597-character statement | `silver.repo_naming` | the `SourceMutationProfileViolated` probe → `Resources exceeded … too many subqueries or query is too complex` | **closed**; the granularity half was wrong on every backend, DuckDB's `VALUES` just never complained | `20260906-bigquery-correctness` (closed) |
| `FILTER (WHERE …)` reached the warehouse instead of being refused at compile time — GoogleSQL has no such clause and no `BackendCapabilities` flag covered it | `gold.repo_dim` | `MAX(repo_name) FILTER (WHERE is_current) AS current_repo_name` → `400 Syntax error: Expected ")" but got "("` | **closed** | `20260906-bigquery-correctness` (closed) |
| An INTERVAL `RANGE` window frame reached the warehouse; GoogleSQL allows only numeric offsets | `silver.actor_sessions`, and its only downstream `marts.daily_active_contributors` | `LAG(created_at) OVER (PARTITION BY actor_id ORDER BY created_at RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW)` | **partly closed**: it is now an actionable compile-time `UnsupportedOnBackend` refusal rather than a warehouse error — but the two models still cannot run on BigQuery, which is why the live half is 14 of 16. A window-frame lowering seam is its own piece of work | `20260906-bigquery-correctness` (the lowering seam) |
| A dialect-blind `VARCHAR` spelling hid behind the frame above — a *missed site* of a known bug class, not a new one | `silver.actor_sessions` | `LAG(CAST(NULL AS VARCHAR))` | **closed** | `20260906-bigquery-correctness` (closed) |
| The precision half of the degradation contract is invisible at run time: nothing in the console output or the run report says an observed delta was skipped, and `smelt explain` still takes no `--target` | `silver.events_deduped` | the downgraded T5 write path, on all four phase-12 runs | open — the spec says the degradation is "recorded, explain-visible"; for this class on this backend it is neither | `20260906-bigquery-correctness` |
| An `external_step:` has no target-awareness: it cannot be told, or scoped to, the target a run is invoking it for, so the loader step reported `success` on the BigQuery target while writing to a local DuckDB file | `sources.raw.github_loader` | `command: ["bash", "load_day.sh", "--date", "{run_date}"]` | open | `20260906-external-dag-steps` |
| `default_target` falls back to the **alphabetically-first** target, so merely declaring a `bigquery` target silently re-pointed every no-`--target` invocation and downgraded `ColumnScopedMerge` to `PerGroupRecompute` project-wide; nothing announced it | every model in `examples/github_activity` under a no-`--target` invocation | `crates/smelt-runtime/src/profile.rs::default_target` with targets `bigquery` and `dev` declared | open — a fail-loud violation; `target: dev` is pinned here as the local fix and does nothing for the next project | `20260906-bigquery-correctness` |
| `--emit-ddl` declared 9 columns while `--emit-sql` selected 10 — two artifacts deriving a schema from one source of truth with only one of them gated | `raw.github_events` / `raw.github_events_arrival` | `bash scripts/bq-dogfood-loader.sh --emit-ddl` vs `sample.sql`'s projection | **closed** (phase 10) by `ddl_columns_match_the_sample_projection`, written RED against the pre-fix DDL. The general shape is worth a sweep | closed — this outcome |
| Loader invocation order is not gated: phase 10 deliberately loaded 08-06 before 08-05 and the redelivery arithmetic closed correctly (63 duplicate ids, 3,201 + 63 = 3,264), but nothing asserts what two invocations do in either order | `raw.github_events` | two `scripts/bq-dogfood-loader.sh --emit-sql --date D` loads in reverse calendar order | open — test gap, evidenced but not gated | `20260906-external-dag-steps` |
| `bronze_events` differs between targets at every intermediate checkpoint (62,382 → 59,605 → 57,217 → 50,406 → 32,251 → 11,536 → 0), always with the DuckDB side a strict subset — arrival order, not either engine | `bronze.events` | its whole-source rebuild (`materialization: table`, no incremental strategy) under a warehouse-resident source vs a day-by-day loaded one | **closed**: registered as one `TARGET_DIVERGENCE_REGISTRY` entry under an `ArrivalLag` bound that still refuses a row lost from inside the lagging leg's own loaded range, and removed entirely by `run_incremental.py --preload-source` | closed — nothing escalated |

## Recorded, not BigQuery findings

Backend-agnostic items the live runs surfaced, kept here so a future reader does not
re-file them against a backend:

- **`SourceRetentionExceeded` on a repeated `--full-refresh`.** A second `--full-refresh`
  over a retention-bounded source, while stored output already exists, refuses without an
  explicit license — `docs/specs/sources.md` §Semantics 5, implemented in
  `smelt-logical`/`smelt-runtime` with no backend crate involved. It would reproduce
  identically on DuckDB given the same sequence. An operational fact for anyone running this
  pipeline live, not a defect.
- **The posture-baseline granularity half of the probe defect.** Grouping the baseline by the
  raw partition column rather than the declared granularity is wrong on **every** backend;
  BigQuery merely made it fatal, because DuckDB's `VALUES` row set absorbs 5,797 branches
  without complaint.
- **The `retention: '90 days'` field on both source YAMLs is still inert and still disagrees
  with the loader's 45.** Unchanged by the live half — see "Requirements handed to
  `20260906-trimmed-history-sources`" above. What the live half *did* add is that the 45 is
  no longer only a DDL claim: it was read back from live table metadata as
  `timePartitioning.expirationMs = 3888000000` on both tables (phases 10 and 17).
- **`smelt list --format json` hard-fails on all three example workspaces.** Pre-existing and
  unrelated to any backend — see finding (b) above. `smelt explain --json` is unaffected and
  is what the live phases used.
- **The `bronze_events` cross-target difference is about arrival order**, which is a fact
  about the example (it is the pipeline's only whole-source rebuild), not about DuckDB or
  BigQuery.

## Operational notes for the next live run

**Build and credential recipe.** None of this is written down anywhere but the phase
summaries:

```bash
cargo build -p smelt-cli --features bigquery     # the default build has no BigQuery backend
bash scripts/bigquery-venv.sh                    # the pinned Python client venv
source scripts/bq-dogfood-env.sh                 # dataset=smelt_dogfood, expiry UNSET, token UNSET
export SMELT_BQ_ACCESS_TOKEN=$(gcloud auth application-default print-access-token)
smelt run --target bigquery --start D --end D+1  # always pass --target; never unpin `target: dev`
```

`python/smelt/bigquery_adapter.py` never falls back to ADC itself, which is why the token has
to be minted explicitly and handed over. ADC here is **already** an
`impersonated_service_account` credential targeting `smelt-dogfood@`, so the printed token is
the service account's — an explicit `--impersonate-service-account` flag on top of it is
redundant.

**Costs, measured.** The loader is ≈ US$0.018/run (≈ US$0.53/month at one run/day); a
thirty-window incremental leg is US$0.29; a seven-checkpoint oracle sweep over the same
state is US$0.06; a one-off thirty-day re-widening of the source from `githubarchive` is
US$0.45. Most per-job billing is BigQuery's 10 MB per-table minimum-billing floor applied
many times over, not data volume — the inputs are 65,583 rows.

**The 2026-09-19 partition-expiration deadline is live.** Both source tables declare
`partition_expiration_days = 45` and the oldest partition in each is 2026-08-05, so it ages
out on 2026-09-19. Phases 13 and 14 completed on 2026-09-11/12, a week inside it. **Any
later live comparison must first re-load the missing days** — after that date, a comparison
against the committed fixture is invalid rather than merely late.

**Two hazards worth not rediscovering.** Run a live leg against a *pinned copy* of the
binary (`SMELT_BIN`): a concurrent `cargo test` in the same checkout will rebuild
`target/debug/smelt` without `--features bigquery` under a running leg's feet. And an
impersonated token outlives less than a thirty-window leg does (~61 minutes against a
one-hour token), so `scripts/bq-dogfood-parity.sh` re-mints before every window and every
snapshot via `PARITY_TOKEN_CMD` rather than exporting one token up front.

**Levers.** `PARITY_CHECKPOINTS=1,2,3,…,30` widens the comparison set from the declared
seven (the reduction was measured, not assumed: 1,481 rows/s through the real landing path
puts a thirty-checkpoint sweep at ≈7.2M rows ≈81 min, against ~51 min of model execution).
`PARITY_RESUME_FROM` resumes an interrupted leg with **absolute** window numbers, so a
checkpoint keeps its label. Neither changes what is compared, only how much.

**Scheduling shape.** The thirty daily windows were a choice; the run window need not match
partition granularity, and a wider window is one engine query with one partition-aligned
DELETE and one INSERT. But an empty window costs what a full one does, so an idle daily
schedule pays a working day's scan for nothing.

## Final punch-list

Ordered, for `20260906-bigquery-correctness` to consume. Everything the DuckDB half handed
over is already closed — see "Punch-list for `20260906-bigquery-correctness`" above, whose
items 1-3 are done and whose item 4 was closed by the run-window widening described in root
cause 4's **Fixed** paragraph. These are what the *live* half adds:

1. **Decide whether `--event-time-end` should bound a full refresh's source scans.** The
   consequential one: today "full refresh" and "the oracle at window *k*" are not the same
   operation against a static source, so six of fourteen relations cannot be compared at an
   intermediate checkpoint. Owner: `20260906-bigquery-correctness`.
2. **Give BigQuery a per-model ledger, or otherwise stop serialising every model's
   bookkeeping through one `_smelt_ledger` table.** Owner:
   [#203](https://github.com/adbrowne/smelt-sql/issues/203).
3. **A window-frame lowering seam**, so `silver.actor_sessions` and
   `marts.daily_active_contributors` run on GoogleSQL and the live half becomes 16 of 16.
   Owner: `20260906-bigquery-correctness`.
4. **Surface the precision half of the degradation contract at run time** — a console line, a
   run-report field, or `smelt explain --target`. Owner: `20260906-bigquery-correctness`.
5. **Stop inferring the default target from sort order.** Owner:
   `20260906-bigquery-correctness`.
6. **Target-awareness for `external_step:`** — per-target scoping, a `{target}` placeholder,
   or a trust-this-source-is-fresh escape hatch. Owner: `20260906-external-dag-steps`.
7. **Gate the loader's two-invocation orderings**, which the live run evidenced but nothing
   checks. Owner: `20260906-external-dag-steps`.
8. **Reconcile `retention: '90 days'` with the loader's enforced 45.** Owner:
   `20260906-trimmed-history-sources`.
9. **Cost shape for a scheduled run**: ≈5.1 jobs/model/run with a ~5 s per-job floor, and an
   empty window costing a full one. Owner: `20260906-bigquery-unattended`.
