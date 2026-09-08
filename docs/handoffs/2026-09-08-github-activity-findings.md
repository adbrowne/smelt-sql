# GitHub-activity pipeline findings — DuckDB half

**Status:** interim — **DuckDB half only**. The live-BigQuery half (compile refusals,
runtime failures, cross-target divergence) lands in phase 16 of
`docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md`, gated on a provisioned GCP
project and credential that do not exist in this worktree (`gcloud auth list` → "No
credentialed accounts", re-checked through phase 9). Everything below is derived from
`examples/github_activity/`'s DuckDB replay over the committed 30-day Parquet fixture —
no live warehouse was queried to produce this document.

**Source of every claim below:** `docs/outcomes/20260906-bigquery-dogfood-spine/phases/`
`0{2,3,4,6,8,9}-summary.md`, that outcome's own "## Decision log", and
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
