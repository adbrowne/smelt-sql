# The self-directed scheduler (2026-09-05)

Research into the next big focus area after the incremental-models and definition-deltas
programme. Written at Andrew's request after a review of `docs/ROADMAP.md`, the specs, and the
runtime as of 2026-09-05 (main at `b7740de4`; the outcome loop's branch
`outcome-loop-20260904-programme-hygiene` ahead of it with `state-residency` complete).

This is a research doc, not a spec or a plan: it frames the decision surface so that the spec
diff can be written and human-reviewed before any loop touches the run loop. The 2026-08-16
handoff flagged this item as the highest design risk in the programme and asked for exactly
that review.

## The claim

smelt's orchestration layer is split down the middle. The **execution** half — given a set of
`(model, region)` pairs, run them correctly — is well built. The **decision** half — given what
exists in the warehouse and what has been recorded, decide which `(model, region)` pairs must
run — does not exist as a product surface. Every run is told its work by a human or by an
external scheduler that knows nothing about the DAG.

The delta-signature programme built the whole decision machinery (typed edges, forward
propagation, backward resolution, per-source landed-delta records, observed output deltas, the
reconciliation ledger) and stopped one step short of connecting it to the run loop. Closing
that step is what converts the last year of incremental work into a user-visible capability:
`smelt run` with no window flags does the right thing.

## Where the two halves stand

### Execution half — built

| Capability | Where | Status |
|---|---|---|
| Wavefront scheduler, `--jobs` bound | `smelt-runtime/src/execute.rs` | shipped; `tests/parallel_execution.rs` |
| Bounded retry with backoff, transient-only | `execute.rs` `RetryPolicy`/`retry_backend_call` | shipped; `tests/retry.rs` |
| `--resume` keyed on outcome + definition hash | `execute.rs`, `run_state.md` §"`--resume` semantics" | shipped |
| Project-wide advisory lock, atomic state writes | `smelt-state/src/file_store.rs` | shipped |
| Run manifest + run report, grouped failure summary | `smelt-state`, `cli.md` §"Failure summary" | shipped |
| Check failure skips the downstream closure | `execute.rs` skip-set | shipped |
| Deferral-contract run skipping | `execute.rs`, `tests/contract_deferral_*.rs` | shipped |
| Three-way exit-code contract | `cli.md` §"Exit codes" | shipped |
| Cancellation | `CancellationToken` threaded through `execute_project` | shipped |
| Cron / Airflow guidance | `docs-site/docs/guide/orchestration.md` | shipped |

### Decision half — machinery built, entry point missing

| Piece | Where | What exists | What is missing |
|---|---|---|---|
| Forward propagation | `smelt-runtime/src/propagation.rs` `plan_since_upstream*` | derives the dirty `(model, region)` set from declared per-source deltas over typed edges, incl. keyed dirt-sets and column-group scope | its input is `--source X --landed a..b` typed by the operator |
| Backward resolution | `propagation.rs` `resolve_build_plan` | `smelt build <model> --period --include-upstreams` | none for this shape; it is the "I want this output" verb and is fine as is |
| Per-source landed-delta record | `smelt-state/src/landed_deltas.rs`; written by `execute.rs` after every batch | records what each consumed source's window covered; interval-diffs append-only sources | nothing **reads** it to discover a delta; nothing compares it to what the source now holds |
| Source completeness marker | `sources.md` `watermark.complete_through`; `smelt-core/src/sources.rs` | parsed; drives the `SourceWatermarkViolated` probe | never consulted to size a run |
| Observed output deltas on model edges | `maintenance_driver::read_observed_delta`, consumed by `plan_since_upstream_with_observed_deltas` | recorded per run, narrows model-edge propagation | only reachable via the explicit `--since-upstream` path |
| Reconciliation ledger | engine-resident `_smelt_ledger` (state-residency, done) | per `(region × column-group)` processed-input vector / frontier watermark | not a scheduling input; it is the fold's never-fold-twice guard |
| `--auto` | `smelt-cli/src/commands/run_setup.rs` `compute_auto_time_range` | max date in the interval ledger → today, one window for the whole run | ignores edges, propagation, keyed dirt, per-model clocks; date-keyed |
| Interval ledger | `smelt-state/src/intervals.rs` | per-model covered intervals | keyed by calendar date; spec says RFC3339 instants (`run_state.md` §Known Divergences; triage item 15) |
| Cumulative reprocessing | `smelt-runtime/src/cumulative.rs` | refuses a reprocessed window via the ledger | no scheduling-side watermark, so `--auto` is blind for idempotent keyed models (triage items 9, 10) |

The spec already says this in its own words: "smelt does not currently discover what is new
on its own: every run is told its window" (`incremental_models.md` §Surface "Run flags"), and
lists "Automatic, watermark-diffed `--since-upstream`" under Future Extensions. Research
`20260811` §6 put "scheduler consumes delta types" as step 1 of what remained. It has never
started.

## Why now

- **State-residency is done.** The 2026-09-04 review said not to start the scheduler until the
  ledger moved engine-resident, because a per-source watermark that drives correctness belongs
  under the residency rule. That gate is open on the loop branch (all eleven phases done,
  `docs/validations/2026-09-05-state.md` clean).
- **The alternative queue items do not move the product.** The roadmap's "What's Next" items
  1–4 (collation, total output-schema resolution, safety-overrides review) are type-system
  polish. The four remaining 2026-09-04 outcomes are closure work. None of them changes what a
  user can do with `smelt run`.
- **It is the payoff of the delta work.** Without it, a user of the typed-edge graph must
  hand-compute what landed upstream and type it on the command line — precisely the bookkeeping
  dbt users do with `is_incremental()` and a `max(updated_at)` subquery, which smelt set out to
  derive instead.

## What "self-directed" has to mean

Three questions, each with a decision surface.

### 1. What is the delta discovery input?

A bare `smelt run` (no window flags) needs, for every source the selected models read, a
statement of what landed since the source's delta was last consumed. Candidates, in
widen-never-narrow order:

| Source shape | Discovery | Cost | Trust posture |
|---|---|---|---|
| Append-only clocked, `watermark.complete_through` declared | read the marker; delta = `[covered_through, complete_through)` diffed against the landed record | one cheap query | declared and checked: the existing `SourceWatermarkViolated` probe already falsifies it |
| Append-only clocked, no marker | `max(partition_column)` on the source; delta = the same interval diff | one scan of the partition column (often metadata-answerable) | derived; can see partial partitions — the settle bound (`source_lateness`) already exists to hold the trailing edge open |
| `change_feed` | the feed's offsets since the recorded offset | one read of the feed's position | native |
| `mutable_snapshot` with fingerprint sidecar | the sidecar diff (already built for the delta-restricted recompute) | one digest pass | derived |
| `mutable_snapshot` without sidecar, unclocked | whole-table (`LandedDelta::WholeTable`) | none | the declared cost of the conservative posture; propagates as whole-model dirt exactly as today |

The point of the table: **every row already has its machinery**. The landed-delta store
records coverage; the sidecar computes snapshot diffs; the propagation planner consumes
`SourceDelta`s. The missing piece is a pure function
`discover_source_deltas(sources, landed_store, backend_observations) -> Vec<SourceDelta>` and
one query per source to gather the observations.

**Open decision (a):** does the "current state of the source" read happen against the live
backend inside `smelt run`, or must it come from a recorded observation (an explicit
`smelt observe` / landing hook that upstream loaders call)? The spec's Future Extensions entry
scopes live freshness querying *out* ("live backend freshness querying stays out of scope"),
but that sentence was written when the ledger was file-resident. Recommendation: **both**, as
the explicit and automatic forms already coexist for `--landed`. A live read is the default
for clocked sources (it is one `max()` query and smelt already runs probes against the source);
a recorded landing is the path for loaders that know what they wrote and for sources smelt
cannot cheaply scan. Neither invents a new graph-layer input: both produce `SourceDelta`s.

**Open decision (b):** where does the per-source "last propagated through" watermark live? It
is not the reconciliation ledger (that is per-model correctness state) and not the interval
ledger (that is per-model coverage). The landed-delta store is the natural home — it is
already keyed by source address and already interval-diffs — but today it is under `.smelt/`,
so under `state.mode: stateless` it is absent. Under the degradation contract that is
acceptable: a missing watermark widens to whole-table, which is correct and visible. The
question is whether a *shared* watermark (several CI runners, one warehouse) needs it
engine-resident like the ledger. Recommendation: keep it in run state for v1, record the
degradation in `smelt explain`, and let the pluggable-observability-store extension
(`state.md` §Future Extensions) answer sharing when it is felt.

### 2. What is the scheduler's currency?

Today's currency is a whole day-interval per model. The typed-edge graph already carries
`(delta shape × addressing × column set)` vectors and keyed dirt-sets, and the runtime already
runs keyed cells — but a node reached only through the keyed channel is scheduled as a
whole-table run because there is no interval to bound it by, and keyed dirt-sets carry key
*columns* and provenance, not key *values*.

Decisions here are sequencing, not design; the spec already states the target:

1. **Instant-keyed intervals** (triage item 15, agreed). Every place the day ordinal is the
   unit — `intervals.rs`, `landed_deltas.rs`, `iso_floor`/`iso_ceil` in `propagation.rs`, the
   run-window rendering — moves to RFC3339 instants aligned outward to the receiving axis's
   declared grain. Hourly models stop coarsening to days. This is the cheapest moment because
   the scheduler is rebuilding the same bookkeeping.
2. **Key-valued dirt-sets** where discovery admits them. The affected-key discovery routes exist
   (`model_properties.md` §"Affected-key discovery"); the scheduler dispatches the already-
   derived key-addressed cells with the discovered key set instead of a whole-table run.
3. **Deferral and frozen-horizon as scheduling inputs**, not just run-time skips. A model with
   `deferral: D` whose inputs have moved less than `D` is dropped from the dirty set *before*
   dispatch and reported as such; today the skip happens inside the run.

### 3. What is the external-orchestrator contract?

The roadmap's "Orchestrator Integration" item imagines a Dagster/Airflow adapter over
`smelt explain --json`. That is the wrong layer while the decision half is missing: an adapter
that maps one model to one Airflow task still has to be told each task's window, and it will
re-derive the DAG that smelt already derives.

The contract that falls out of self-direction is smaller and stronger:

- **One idempotent command per unit of work.** `smelt run --select <model>` with no window
  discovers its own delta, runs it, records it. Re-running it is a no-op. An orchestrator can
  schedule the whole project as one task (simplest, already documented) or one task per model
  with `--select` and `--jobs 1`, and get the same result either way because the dirty set is
  derived from state, not from task ordering.
- **`smelt explain --json` for the DAG** — already there — plus the dirty set as a dry-run
  output: `smelt run --dry-run` prints the discovered per-source deltas and the propagated
  `(model, region)` set exactly as `--since-upstream` prints its report today, so an
  orchestrator that wants to fan out can ask smelt what would run.
- **Exit codes unchanged.** `0` with an empty dirty set is "nothing landed", not a failure.

A Dagster asset factory becomes a thin consumer of those two outputs and can be a separate,
later, optional package. It should not be in scope for this work.

## What does not belong in this work

- **Virtual environments / plan-apply-promote.** Same crates, different question (definition
  deltas across environments, not data deltas over time). Its Known Divergences are honest and
  its gating work (cross-model column lineage) is unrelated. Keep it queued behind this.
- **External models as DAG participants.** Real need, but it is a declaration-format design
  with no machinery behind it yet. The self-directed scheduler makes it *easier* later (an
  external model is a source with a declared clock and a recorded landing), so sequence it
  after.
- **A Spark or BigQuery engine-resident ledger.** Triage item 12: build when a workload demands
  it; the downgrade path is the intended steady state.
- **Live freshness for never-modeled raw sources.** Nothing smelt never touched has a landed
  record; a first run is a full run. That is the existing first-run rule, not a gap.

## Enabling refactor: split `execute.rs`

`execute.rs` is 6,590 lines and `maintenance_driver.rs` is the same order. The ratchet-paydown
outcome explicitly excludes the split as "judgment-heavy, belongs to a fork-level implementer".
Every phase of this work lands in that file (the discovery step sits before the wavefront; the
dispatch changes sit inside it; the recording changes sit after each batch). Doing the split
first, as a move-only refactor with `execute_parity`, `statement_parity`, and the conformance
gate as the safety net, is not hygiene — it is what makes the scheduler phases reviewable.
Suggested seams, following the existing module boundaries the file already comments:
`schedule.rs` (wavefront + skip-set + resume closure), `record.rs` (interval / landed-delta /
ledger writes under `state_io_lock`), `batch.rs` (per-model batch loop), leaving `execute.rs`
as the `execute_project` entry and request/response types.

## Proposed sequence

Each step independently shippable and independently valuable.

1. **Spec diff, human-reviewed.** `incremental_models.md`: move "Automatic, watermark-diffed
   `--since-upstream`" from Future Extensions into §Surface as the default behaviour of a
   window-less `smelt run`; state the discovery table above as normative (widen-never-narrow
   over discovery inputs); define `--dry-run`'s dirty-set output. `run_state.md`: the
   per-source watermark's home and its degradation. `sources.md`: `watermark.complete_through`
   becomes a scheduling input, with the trust rule unchanged. `cli.md`: `--auto` is retired
   into the default; the explicit `--source/--landed` form stays as the override.
2. **Split `execute.rs`** (fork-level, move-only, gates green).
3. **Discovery as a pure function** in `smelt-logical` or `smelt-runtime`'s propagation module:
   `discover_source_deltas` over `(source facts, landed store, observations)`; unit-tested
   against every row of the discovery table; no backend yet.
4. **Observations from the backend**: one `max(partition_column)` / marker read per source,
   through the `Backend` trait; sidecar and change-feed positions through their existing seams.
5. **Wire the window-less run**: `smelt run` with no flags = discover → propagate → dispatch →
   record. `--dry-run` prints the set. Conformance gate gains a "no-flag run" step kind whose
   oracle is full refresh of everything that landed — the research §6 step-1 exit criterion
   ("a keyed-upstream → partitioned-downstream chain incrementally with no command-line
   delta").
6. **Instant-keyed intervals** across `intervals.rs`, `landed_deltas.rs`, propagation rendering.
7. **Key-valued dirt-set dispatch; deferral as a pre-dispatch filter.**
8. **Docs-site**: rewrite `guide/orchestration.md` around "smelt decides, the scheduler ticks";
   retire the `--auto` guidance.

Steps 3–8 fit the outcome loop once step 1 has been reviewed. Step 2 does not: it is the
fork-level refactor the ratchet outcome already identified.

## Open questions for the spec review

1. Live read vs recorded landing as the default discovery input (decision (a) above).
2. Watermark residency: run state with recorded degradation (recommended) or engine-resident.
3. Does a window-less `smelt run` on a project with **no** landed record for a source run
   everything (first-run rule) or refuse and ask for an explicit window? Recommendation: run
   everything and say so — a refusal would make the zero-state case worse than today.
4. Whether `--auto` is removed or kept as a deprecated alias for one release. Recommendation:
   remove; no backward-compatibility constraint applies and the flag's semantics are being
   replaced, not extended.

## Pointers

- Review that set the sequence: `docs/research/20260904-incremental-state-review.md`.
- Original sequencing: `docs/research/20260811-delta-signatures-and-definition-deltas.md` §6.
- Handoff that flagged the design risk: `docs/handoffs/2026-08-16-delta-signature-closure-programme.md`.
- Triage decisions consumed here: `docs/research/20260816-open-questions-triage.md` items 8, 9, 10, 12, 15.
- Spec anchors: `incremental_models.md` §"The graph layer", §Surface "Run flags", §Future
  Extensions "Automatic, watermark-diffed `--since-upstream`"; `sources.md` §"Landed-delta
  (derived, recorded)", `watermark` row; `run_state.md` §"Interval ledger", §Known Divergences;
  `state.md` §"The degradation contract", §Future Extensions.
- Code anchors: `crates/smelt-runtime/src/propagation.rs` (`SourceDelta`,
  `plan_since_upstream_with_observed_deltas`, `resolve_build_plan`);
  `crates/smelt-state/src/landed_deltas.rs` (`record_landing`);
  `crates/smelt-cli/src/commands/run_setup.rs` (`compute_auto_time_range`);
  `crates/smelt-cli/src/commands/run.rs` (`run_since_upstream`).
