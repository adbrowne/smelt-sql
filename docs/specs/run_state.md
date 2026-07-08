---
feature: run_state
status: experimental
last_reviewed: 2026-06-13
owners: [andrew]
---

# Run State

> **What this is.** A normative spec for smelt's on-disk run state — the `.smelt/` directory layout, the run-manifest format, run IDs, the interval ledger, deployed-schema snapshots, and (for virtual environments) the fingerprint-keyed snapshot and environment→table map. It defines what smelt persists, when, and how a stateless project avoids persisting anything. Out of scope: the equivalence judgement that keys snapshots (see `output_fingerprint.md`); the environment orchestration that consumes them (see `virtual_environments.md`); incremental interval *semantics* (see `batched_models.md`); deployed-schema *change classification* (see `schema_evolution.md`). This spec owns the **storage**; those specs own the **meaning**.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences with code/research links.

## Surface

### `.smelt/` directory layout

All run state lives under a single project-local `.smelt/` root (gitignored in example workspaces). The layout is fixed:

```
.smelt/
  runs/<run_id>.json      # one run manifest per execution
  intervals.json          # cumulative interval coverage across runs
  schemas/<model>.json    # deployed schema snapshot per model (see schema_evolution.md)
```

State files are never written outside `.smelt/`. A `state.mode: stateless` project does not require this directory to exist.

### Run manifest (`runs/<run_id>.json`)

A run manifest is a JSON document recording what one execution did:

```jsonc
{
  "run_id": "20260604-141233-a1b2c3",
  "started_at": "2026-06-04T14:12:33Z",
  "completed_at": "2026-06-04T14:12:41Z",   // null until the run finishes
  "models": {
    "<model_name>": {
      "strategy": "incremental",            // materialization strategy used
      "time_range": { "start": "2026-01-01", "end": "2026-02-01" }, // omitted if absent
      "partitions_updated": ["2026-01-01", "..."],                  // omitted if empty
      "row_count": 12345,
      "duration_ms": 812,
      "batch_safety": "BoundedSafe"         // omitted if absent
    }
  }
}
```

### Run ID

A run ID is `<UTC-timestamp>-<hex-suffix>`, formatted `%Y%m%d-%H%M%S-<6 hex>` (e.g. `20260604-141233-a1b2c3`). It is timestamp-sortable and unique for non-concurrent runs.

### Interval ledger (`intervals.json`)

Cumulative per-model interval coverage as string date keys (`"2026-01-01"`, half-open `[start, end)`). Interval arithmetic (merge, gap detection) operates on calendar dates. Consumed by incremental backfill and gap detection (`batched_models.md`).

### Snapshot and environment store (virtual environments)

Under `state.mode: environments`, run state additionally records, per model: the **expanded logical SQL** that built each physical table, the **output fingerprint** of that SQL, and a map from `(environment, model)` to the physical table currently backing it. These drive fingerprint-keyed reuse and promotion (`virtual_environments.md`).

### Relationship to the reconciliation ledger

The run-state intervals this spec owns and the maintenance plan's **reconciliation ledger** — the `(output-region × column-group)` bookkeeping that records each region-group's processed-input vector, frontier watermarks for idempotent groups, and delta identities for additive groups (`maintenance_plan.md` §"The reconciliation ledger") — are **not the same mechanism and do not substitute for each other**. Run-state intervals are project-wide observability: they exist to answer "what has this project run, and where are the gaps" for humans and tooling, and a project may run at `state.mode: stateless` and forgo them entirely. The reconciliation ledger is **required correctness structure** for every `grain: key` (and `key_per_partition`) model maintained under a derived plan — it exists whenever the plan does, independent of `state.mode`, because it is what lets a fold-family technique detect a re-run (never fold a delta already in the entry's processed set) and lets a crashed run resume exactly. Neither reads the other: the reconciliation ledger is per-model and backend-resident, keyed by `(region × column-group)`; the interval ledger here is project-wide observability state under `.smelt/`. This spec also owns the **per-source landed-delta record** that forward propagation consumes (`sources.md` §"Landed-delta intervals (derived, recorded)"; `maintenance_plan.md` §"The graph layer") — which partition intervals of a source landed, keyed by source address, recorded in run state alongside the interval ledger, not in the reconciliation ledger. A project could run `state.mode: stateless` with `grain: key` models and still get correct, ledger-enforced reprocessing refusal — the reconciliation ledger's presence is a property of the *plan*, not of the project's state posture. This spec continues to own the run-state **storage and serialisation** (`.smelt/` layout, manifest format, run IDs, landed-delta intervals); the reconciliation ledger's structure, grading, and operations are owned by `maintenance_plan.md`.

## Semantics

- **Stateless writes nothing.** Under `state.mode: stateless` (the default), no manifest, interval, snapshot, or environment record is written; `.smelt/` need not exist. State is created only when a higher posture is opted into.
- **A manifest is written per run.** `started_at` and `run_id` are set at run start; `completed_at` is set on successful completion. A manifest with `completed_at: null` denotes an interrupted run.
- **Recovery is idempotent re-run.** smelt does not checkpoint mid-run; recovery is re-running the same selection/range, which converges because each committed unit is idempotent (`batched_models.md` §"Failure mode"). The interval ledger lets a re-run skip already-covered ranges and surface gaps.
- **Manifest evolution is backward-compatible.** Readers must tolerate historical manifests: every new field is `Option`al or `#[serde(default)]`. A required new field is a breaking change to stored state and is not permitted.
- **Snapshot reuse is keyed by fresh fingerprints, not stored hashes.** The persisted artifact for reuse is the **expanded logical SQL**, not its fingerprint. Equivalence is decided by fingerprinting the stored SQL and the current SQL with the *current* compiler at decision time (`output_fingerprint.md` §Design). The stored fingerprint, if recorded, is a cache/diagnostic, never the source of truth.

## Design

**Opt-in, file-based, gitignored.** State is a posture a project opts into (`virtual_environments.md` §`state.mode`), not a baseline requirement. Storing it as plain JSON files under `.smelt/` (rather than a required embedded database) keeps a stateless project zero-cost and keeps state human-inspectable and easy to delete — a half-broken state store must never be harder to recover from than dropping a directory. Rationale: `batched_models.md` §Design "smelt does not own state"; research §6.

**Persist the SQL, treat the fingerprint as ephemeral.** Storing the expanded logical SQL that built each table — and recomputing fingerprints fresh on both sides at decision time — makes the fingerprint algorithm free to change between releases with no migration code and no version-stable-form contract. A stored hash would force a versioned normal form and golden cross-version tests; storing the SQL makes the comparison apples-to-apples by construction. Rationale: `output_fingerprint.md` §Design; research §5.6, Open Question 15.

**Run ID is timestamp-first.** A sortable `%Y%m%d-%H%M%S` prefix makes `runs/` browsable and history queries cheap; the hex suffix disambiguates same-second runs. Uniqueness assumes one smelt process at a time — concurrent runs are not a supported posture today.

## Constraints & Invariants

- **Stateless requires no `.smelt/`.** Enabling no state posture must leave a project's on-disk footprint and behaviour exactly as today.
- **Fixed layout.** State is confined to `.smelt/runs/`, `.smelt/intervals.json`, and `.smelt/schemas/`. New artifact kinds extend this layout under `.smelt/`, never outside it.
- **Forward-compatible manifests.** Stored JSON must remain readable by later smelt versions; new fields are optional/defaulted.
- **No stored-hash dependence for reuse.** Reuse correctness must not depend on a previously stored fingerprint value; it is always recomputed.

## Known Divergences / Open Questions

- **Interval-ledger key granularity is date-only, pending the incremental rewrite.** The interval ledger (§"Interval ledger") keys coverage by calendar-date string (`"2026-01-01"`), but incremental models routinely filter on sub-day (hourly/second) event-time boundaries. Whether the ledger keys move to RFC3339 instants (sub-day capable) is **coordinated with the incremental_models rewrite** rather than changed here, so the ledger granularity and the incremental cadence model land together. Tracked in `docs/plans/20260322-incremental-model-support.md` (`batched_models.md`).
- **Snapshot / environment store is unbuilt.** Today `smelt-state` persists run manifests, the interval ledger, and deployed schemas. The expanded-logical-SQL snapshot, the recorded output fingerprint, and the `(environment, model) → table` map are **specified here but not implemented**; they arrive with the virtual-environments orchestration layer. Tracking: `docs/research/20260601-virtual-environments.md` §8.
- **JSON files vs. embedded store.** Research §8 sketched an embedded `.smelt/state.db`; the implementation uses JSON files (`runs/*.json`, `intervals.json`, `schemas/*.json`). The current normative layout is the JSON form; whether to move to an embedded store as the snapshot/environment map grows is open.
- **Concurrency / parallelism.** Run IDs and the file layout assume a single smelt process at a time; concurrent runs against one `.smelt/` are not specified. Parallel *model* execution within one run is owned by `smelt-runtime`, not by this layout.
- **Manifest format completeness.** The fields above reflect `smelt-state` today; the full manifest contract (e.g. per-model error capture, retries) is still settling and may add optional fields.
- **"No change to correctness" no longer holds unconditionally once forward propagation lands.** §"Relationship to the reconciliation ledger" describes run-state intervals as observability a project can forgo with no correctness impact; that was true while every run recomputed its own dirty set from scratch. Forward propagation (`smelt run --since-upstream`, `maintenance_plan.md` §"The graph layer") instead computes what must run from **recorded per-source landed deltas**, so a project that opts out of persisting them loses the input forward propagation needs and falls back to a full recompute rather than the derived dirty set. This is a known divergence until forward propagation and the reconciliation ledger land (`docs/plans/20260707-maintenance-plan-impl.md` phases MP14/MP15).

## References

- **Code**: `crates/smelt-state/src/lib.rs` (`RunManifest`, `ModelRunRecord`, `TimeRangeRecord`, `generate_run_id`), `src/file_store.rs` (`.smelt/` reader/writer), `src/intervals.rs` (`IntervalStore`), `src/schema_tracking.rs` (`DeployedSchema`), `src/history.rs` (history queries)
- **Tests**: `crates/smelt-state/tests/`
- **User docs**: none yet (CLI surfaces `smelt status` / `smelt history` over this state — see `cli.md`)
- **Plans (history)**: none yet — predecessor research is `docs/research/20260601-virtual-environments.md`
- **Related specs**: `batched_models.md` (interval semantics, idempotent recovery), `keyed_models.md` (the transactional merge ledger — required correctness structure, distinct from this spec's opt-in observability), `schema_evolution.md` (deployed-schema snapshots and migration), `virtual_environments.md` (the snapshot/environment consumers), `output_fingerprint.md` (the equivalence key), `architecture.md` (`state.mode` surface, `smelt-state` crate)
