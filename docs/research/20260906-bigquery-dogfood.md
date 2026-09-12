# Dogfooding smelt on BigQuery: the GitHub-activity pipeline

**Date:** 2026-09-06
**Status:** Design — agreed in session, not yet planned
**Author:** Andrew Browne, with Claude

## The decision

smelt's next stretch optimises for **reach: a first real user**, and that user is
Andrew, running a real pipeline on BigQuery with GCP-native orchestration. The bar
is not "a stranger could adopt this" — it is **"it runs unattended against a real
dataset and I trust the numbers."**

This is deliberately narrower than `20260905-dbt-replacement-gaps.md`'s top-five.
That list is ranked by *how many prospective adopters a gap blocks*; with a single
named user who is the maintainer, most of it evaporates. See §"Out of scope".

## Why this, and why now

Three facts make dogfooding the highest-value next move rather than more substrate:

1. **BigQuery is far more built than the project notes suggest.** It is merged to
   `main` with a leg in all eight fixed-recipe parity suites passing live, a live
   type oracle with a divergence registry (a 512-case sweep over 285 live-compared
   columns found exactly one unregistered divergence class — a smaller surface than
   DuckDB's or Spark's), and all 21 cases of the generative maintenance-conformance
   gate passing against the real warehouse.
2. **`state-residency` landed `done`.** Correctness state lives in engine-resident
   tables, and `state.mode: stateless` writes nothing under `.smelt/`. A stateless
   container against BigQuery is close to the natural deployment already.
3. **The live leg is the only thing that finds a whole class of defect.** Three
   product-side bugs it caught were invisible to every offline gate: `^` computing
   bitwise XOR instead of power on GoogleSQL, the output-schema cast wrap rounding
   exact medians, and the keyed `MERGE` emitting `INSERT *` on every backend. A real
   pipeline is a wider instance of that same oracle.

Point 3 was confirmed again while writing this document — see §"Findings already
banked".

## The programme

**D0 — Provision the dogfood project.** Its own GCP project, no 24h table
expiration, a real budget. The existing `smelt-bq-test-20260816` is correct as a
test harness and fatal as a pipeline meant to accumulate history.

**Decided:** the dogfood project gets a Claude-reachable credential path. This is a
deliberate departure from the test harness's design, where `.claude/settings.json`
denies `gcloud`/`bq` outright. That denial exists to keep a Claude session away from
a *shared* credential; a dedicated, dataset-scoped, budget-capped dogfood project is
a different risk calculation. The test project's isolation is unchanged.

**D1 — Land, then model.** A separate loader copies GitHub Archive shards into one
partitioned table smelt owns; smelt's source is that table. Then the models. This
phase generates the rest of the backlog — see §"Sequencing".

**D2 — Numbers you can trust.** The BigQuery correctness punch-list, *driven by what
D1 actually hits* rather than swept speculatively.

**D3 — Unattended on GCP.** Workload identity, Cloud Run Job, Cloud Scheduler,
Cloud Logging over the run report W2 already emits.

### Sequencing: models first, punch-list second

Issue #179 records 42 BigQuery registry entries with no emission verdict. Building
all 42 speculatively is waste; building the ones the pipeline hits is not. The same
argument applies to techniques, grains, and capability rows. **Let the real models
generate the punch-list.** The exceptions are defects wrong for *any* model, which
are unconditional — §"Findings already banked" holds the first.

### Orchestration: Cloud Run Job, not Composer

Managed Airflow (Cloud Composer) carries a floor around US$300/month and buys
nothing that smelt's own DAG derivation does not already provide. The
build-vs-rent rule (`20260906-build-vs-rent-boundary.md`) puts scheduling
*execution* firmly on the rent side, so the thinnest thing that works is right:
a container on a cron. Cloud Workflows is the fallback if cross-target fan-out
is ever needed.

## The example project

Lands as `examples/github_activity/`, alongside `examples/web_analytics/`, so it
doubles as documentation and earns CI coverage. The BigQuery legs cannot run in
normal CI; the DuckDB-runnable shapes can.

It deliberately exercises the same problem-space as the web-analytics series in a
different domain, so the two can be compared:

| web-analytics problem | GitHub-events analogue |
|---|---|
| at-least-once feed, ~2% duplicates | loader is at-least-once by design; `event.id` is a real natural dedup key rather than a synthetic one |
| 3-day late arrival | shards publish hourly with lag; events near an hour boundary land in the next file — bounded and derivable |
| 30-minute-gap sessionization crossing midnight | **contribution sessions** — an actor's burst of activity separated by a gap. Same shape, different domain |
| identity stitching (device → user) | **repo and actor renames** — `repo.id` is stable, `repo.name` is the name *at event time*. This is where SCD2 lives |
| *(not exercised)* | **a genuinely large source**, so partition pruning has to keep the bill down. The web-analytics data was synthetic and small |

### First pass: the spine

Six models, chosen to prove the hard shapes end to end before investing in breadth.

**Sources**
- `raw.github_events` — produced by the loader. Append-only, clocked on `created_at`,
  day-partitioned, **trimmed to N days**, at-least-once.

**Models**
- `bronze.events` — typed passthrough, JSON payload retained.
- `silver.events_deduped` — dedup on `id` plus the derived late-arrival window.
- `silver.actor_sessions` — gap-based sessionization; the web-analytics session analogue.
- `silver.repo_naming` — **SCD2**, keyed succession over rename observations.
- `marts.naming_history` — "what was this repo called in March?", so the SCD2 output
  is visible as a product rather than only as a technique.

### The full sketch (the target, not the first pass)

Recorded so the spine's boundaries are deliberate rather than accidental.

- **Sources:** `raw.github_events`; `manual.repo_watchlist` — a small hand-edited
  mutable snapshot with a declared `unique_key`. This one is deliberate:
  `docs/TODO.md` records that `Technique::ColumnScopedMerge` has **no reachable
  shipped shape**, and a `unique_key`-declaring dimension left-joined into a fact is
  precisely the shape that makes it reachable.
- **Silver fan-out:** `push_events`, `pr_events`, `issue_events`, `star_events` —
  typed payload extraction per event type. One upstream, several grains; a shape
  web-analytics does not exercise.
- **Second succession instance:** `silver.actor_naming`, so the succession grammar
  is exercised twice rather than once.
- **Gold:** `events_enriched` (session identity + current repo name + watchlist
  tier — the `ColumnScopedMerge` shape); `repo_activity_daily`.
- **Marts:** `daily_active_contributors`, `repo_leaderboard`, `star_growth`.

## Two features this surfaces

Both are Andrew's calls from the session, wanted independently of this pipeline.

### Black-box steps in the DAG

The loader should not be smelt's code, but it should be a **node in smelt's DAG**:
an external job smelt schedules and orders but does not author, with a `source`
declaration as the contract for what it produces and where. This generalises well
past ingest — any externally-produced relation gets the same treatment.

Open: whether the contract is exactly today's source YAML plus a `produced_by:`
key, or a distinct declaration kind.

### Trimmed-history sources

A source whose retained history is **bounded, with the bound moving forward** as old
partitions age out. This is materially different from a static bound: a model whose
maintenance needs to read further back than the source still retains must refuse or
degrade, and the point at which that becomes true arrives on its own.

## Two tensions to hit deliberately

Named here so they are stressed on purpose rather than discovered in week three.

### 1. The SCD2 shape may not fit the succession grammar

`docs/outcomes/20260906-scd2-keyed-succession` requires row-local columns plus
`LEAD(t) OVER (PARTITION BY k ORDER BY t)` over **one `append_only`, clocked
source**, with a **row-local** pre-filter.

Every GitHub event observes `(repo_id, repo_name)`, so succession applied directly
to the event stream yields one tiny interval per event. Collapsing consecutive
identical names into one interval is `LAG`-based change detection, which is *not*
row-local — and whose output is a model, not a source. So either the grammar
stretches, or there is an intermediate step it cannot see through.

Unresolved. This is the single most valuable thing the dogfood stresses, and it
should be confronted in the spine rather than deferred.

### 2. Trimmed history versus SCD2 lifetime

If the source retains 90 days, the naming dimension's history predates what the
source can still see. The dimension must survive beyond its source's retention — a
deep interaction between the trimmed-history feature and the succession grain, and
one where being right matters.

## Findings already banked

Verified against `main` while writing this document, which is itself evidence for
the thesis:

- **Stale:** the BigQuery handoff's "next step #1" (thread the target dialect into
  `build_cumulative_merge_sql` instead of hardcoding `MaintenanceDialect::DuckDb`)
  is **already fixed** — the function takes a `dialect` parameter and callers pass it.
- **Live defect:** `crates/smelt-logical/src/maintenance/emit.rs:2822`,
  `emit_fingerprint_digest_select` accepts `_dialect` and ignores it, passing
  `MaintenanceDialect::DuckDb` to `row_fingerprint_expr` (line 2837). That function
  genuinely branches — BigQuery needs `TO_HEX(SHA256(…))` because GoogleSQL's
  `SHA256` returns `BYTES`, and the value is fed straight into a `STRING_AGG`. Both
  the hash spelling and the string type are DuckDB's on every backend. This is the
  fingerprint sidecar for `mutable_snapshot` sources — the exact path
  `manual.repo_watchlist` would take. Production code; consequence on a live
  BigQuery run is inferred from the branch, not yet executed.

Other known BigQuery-affecting rows, to be confirmed against what D1 hits: #173
(`%` lowers to `MOD`, which rejects floating-point operands), #174 (`LOG` base
differs per engine), #179 (42 entries with no emission verdict), and the two
uncharacterised live conformance failures the handoff names
(`dags_bigquery::diamond_propagation_suffices`,
`gate_composed_bigquery::composed_keyed_pool_upholds_equivalence`).

## Out of scope

Deliberately excluded, each because it is justified only by a **second** user:

- the dbt manifest importer / migration on-ramp
- slim-CI `state:modified+ --defer`
- a package/macro mechanism
- auth, RBAC, SSO
- reusable parameterized generic tests
- Snowflake, Redshift, Postgres backends
- the self-directed-scheduler daemon (already argued against by the build-vs-rent rule)

If dogfooding shows smelt is worth someone else's time, that case arrives with
evidence behind it.

## Relationship to the existing backlog

This runs **alongside** the eight residue outcomes in `.claude/outcome-backlog`, not
instead of them. Those are substrate-hardening the autonomy loop can grind
unattended; this is human-gated and evidence-generating. They do not compete for the
same attention.

`docs/outcomes/20260906-scd2-keyed-succession` is the one real dependency: the spine's
`silver.repo_naming` needs it, and tension 1 above may reshape it.

## Open questions

1. Is `emit_fingerprint_digest_select`'s ignored dialect reachable on a live
   BigQuery `mutable_snapshot` run, or is that path gated off before it? Determines
   whether it is an unconditional D2 fix or a latent one.
2. Does the black-box step's contract reuse source YAML with a `produced_by:` key,
   or warrant its own declaration kind?
3. How does a trimmed-history bound interact with the equivalence invariant's
   quantifier — is `full_refresh(inputs ∈ S)` taken over retained history or over
   all history that ever existed? The answer decides tension 2.
4. What retention does the loader actually keep, and is it a smelt-visible
   declaration or a property of the loader that smelt merely observes?
5. Does GitHub Archive's late-arrival bound need measuring before it can be written
   into a model, or is the hour-boundary argument enough to state it and let the
   posture probe catch a violation?

## Pointers

- `docs/research/20260905-dbt-replacement-gaps.md` — the gap list this narrows
- `docs/research/20260906-build-vs-rent-boundary.md` — the scope rule applied here.
  **Not on `main` at time of writing**: commit `fdfd69c6` on `worktree-fusion`, unpushed.
  Merge it before this document's references resolve.
- `docs/research/20260816-bigquery-backend.md` — decisions, phase order, provisioning
- `docs/handoffs/2026-08-16-bigquery-backend.md` — live status and punch-list (item 1 now stale)
- `docs/outcomes/20260906-scd2-keyed-succession/outcome.md` — the succession grammar
- `examples/web_analytics/` — the pipeline this mirrors in a different domain
