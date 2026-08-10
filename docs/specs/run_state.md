---
feature: run_state
status: experimental
last_reviewed: 2026-06-13
owners: [andrew]
---

# Run State

> **What this is.** A normative spec for smelt's on-disk run state — the `.smelt/` directory layout, the run-manifest format, run IDs, the interval ledger, deployed-schema snapshots, and (for virtual environments) the fingerprint-keyed snapshot and environment→table map. It defines what smelt persists, when, and how a stateless project avoids persisting anything. Out of scope: the equivalence judgement that keys snapshots (see `output_fingerprint.md`); the environment orchestration that consumes them (see `virtual_environments.md`); incremental interval *semantics* (see `incremental_models.md`); deployed-schema *change classification* (see `schema_evolution.md`). This spec owns the **storage**; those specs own the **meaning**.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences with code/research links.

## Surface

### `.smelt/` directory layout

All run state lives under a single project-local `.smelt/` root (gitignored in example workspaces). The layout is fixed:

```
.smelt/
  meta.json                 # { "state_version": 1 } — layout version marker
  lock                       # advisory single-writer lock, held for a run's duration
  targets/<target>/
    runs/<run_id>.json        # one run manifest per execution
    intervals.json            # cumulative interval coverage across runs
    reconciliation.json       # reconciliation ledger, per plan-managed model (see incremental_models.md)
    landed_deltas.json        # per-source landed-delta intervals, keyed by source address
    snapshots.json            # expanded-logical-SQL / fingerprint snapshots (virtual environments)
    schemas/<model>.json      # deployed schema snapshot per model (see schema_evolution.md)
    reports/<run_id>.json     # run-report artifact (see "Run report" below)
```

State files are never written outside `.smelt/`. A `state.mode: stateless` project does not require this directory to exist. All run-scoped artifacts (manifests, ledgers, snapshots, schemas, reports) live under `.smelt/targets/<target>/`, keyed by the target the run executed against (`smelt.yml` §"Target shape"); `.smelt/meta.json` and `.smelt/lock` are the only project-root files, shared across every target.

### `meta.json` and layout versioning

`.smelt/meta.json` records `{ "state_version": <integer> }`, the version of the on-disk layout described in this spec. Today's layout is version `1`.

- **A version this smelt binary does not recognise (greater than the highest version it knows) is a hard error.** smelt refuses to read or write `.smelt/` rather than guess at an unfamiliar layout; the error names the on-disk version and the highest version this binary supports.
- **A missing `meta.json` denotes the legacy pre-versioning layout** — root-level `runs/`, `intervals.json`, `reconciliation.json`, `landed_deltas.json`, `schemas/` (no `targets/<target>/` nesting). The first locked open of `.smelt/` under a version-aware binary migrates a legacy layout to the current version: existing root-level artifacts move under `targets/<target>/` for the target of the run doing the migration, and `meta.json` is written with the current `state_version`. Migration happens under the advisory lock (see "Locking" below) so a concurrent second process cannot observe or act on a half-migrated layout.
- A `state.mode: stateless` project that has never written `.smelt/` has no `meta.json` and needs none — versioning applies only once state exists on disk.

### Locking

A run acquires an exclusive advisory lock on `.smelt/lock` for its duration, released on completion or error. A second smelt process attempting to acquire the lock while it is held fails loudly — the error names the PID recorded in the lock file (`"state locked by PID <n>"`) — rather than silently interleaving writes with the first process. The lock is project-wide (one lock file at `.smelt/lock`, not per-target), since layout migration and `meta.json` writes are project-wide operations even when a given run only touches one target's subtree. The lock is acquired once for the whole run and held for its entire duration regardless of how many models the run engine executes concurrently (`cli.md` §"Parallel execution with `--jobs`") — concurrent in-process model execution is not concurrent *state-writer* access: every write to a shared, whole-store file under `.smelt/targets/<target>/` (the interval ledger, the landed-delta store, the reconciliation ledger) is additionally serialized within the single locked run so two models finishing at the same moment cannot each load-modify-save the same file and drop one write.

### Atomic writes

Every write under `.smelt/` (a run manifest, the interval ledger, the reconciliation ledger, landed deltas, a schema snapshot, a run report, `meta.json` itself) is atomic: the new content is written to a temporary file in the same directory and renamed into place, so a process killed mid-write leaves either the old file intact or the new one — never a truncated or partially-written file.

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
      "batch_safety": "BoundedSafe",        // omitted if absent
      "outcome": "success",                 // "success" | "failed" | "skipped"
      "definition_hash": "a1b2c3d4",        // hash of the model's compiled definition at run time
      "error": "Conversion Error: ...",     // omitted unless outcome is "failed"
      "retry_count": 0,                     // number of retry attempts made before the final outcome
      "probes": [                           // omitted if empty; absent entirely on older manifests
        { "fact": "key_recurrence", "probe": "KeyedRecurrenceBoundViolated", "outcome": "dispatched" }
      ],
      "subsumed": {                         // omitted unless this run subsumed a prior deferral skip
        "maintained_exclusive": "2026-01-02",
        "input_inclusive": "2026-01-07"
      }
    }
  }
}
```

Every model smelt attempted or considered in a run has an entry keyed by `outcome`: `success` (completed without error), `failed` (the model's execution raised an error), or `skipped` (not attempted — upstream failure, selector exclusion, `--resume` short-circuit, or a `contract.deferral` skip). `definition_hash` is recorded for every entry regardless of outcome; it is what `--resume` compares against to decide whether a `success` from a prior run still applies (see "`--resume` semantics" below). `error` carries the failure's display text for every `failed` entry; `retry_count` records how many retry attempts (`docs/specs/architecture.md`, `RunReporter::model_retrying`) were made for that model before its final outcome, `0` if it succeeded or failed on the first attempt. `probes` records, per declared-fact probe this model's run consulted, the fact (`model_properties.md` §"Probe obligation" registry key), the probe's named diagnostic code, and whether the project's `probes:` cadence policy (`smelt_yml.md` §"Top-level keys") actually dispatched it (`"dispatched"`) or skipped it this run (`"skipped"` — the declaration was trusted, not verified). Defaulted to empty for manifests written before probe dispatch was wired in. `strategy` additionally carries two deferral-specific values for a `skipped` entry: `"skipped_deferral"` (this cell's own measured lag licensed the skip, `incremental_models.md` §"The contract lattice") and `"skipped_deferral_upstream"` (a selected dependent of a deferral-skipped cell, skipped with it). `subsumed` is present only on a `success` entry whose own write range proved it folded a window a prior run had recorded `skipped_deferral` for — `maintained_exclusive`/`input_inclusive` are that pending window's dated bounds (`incremental_models.md` §"The contract lattice").

**Abort semantics: the in-flight wave finishes, every failure is recorded, then the run aborts.** A run executes selected models in topologically-ordered waves (`docs/specs/architecture.md` §"Run Pipeline Parity"); models within one wave may run concurrently. When a model's execution raises an error, smelt does not abort the instant the first error is observed — it lets every model already dispatched in that wave finish, and every one that also errors gets its own `failed` entry with its own `error` text. Only once the wave has fully drained does the run stop dispatching further waves. This means a run where two independent models fail concurrently in the same wave never silently downgrades the second failure to `skipped` — both are `failed`, each with its own recorded error. Every other selected model that never got a manifest entry (a later wave that never started, or a model mid-flight when the abort happened) is recorded `skipped`.

### Run ID

A run ID is `<UTC-timestamp>-<hex-suffix>`, formatted `%Y%m%d-%H%M%S-<6 hex>` (e.g. `20260604-141233-a1b2c3`). It is timestamp-sortable and unique for non-concurrent runs.

### Interval ledger (`intervals.json`)

Cumulative per-model interval coverage as string date keys (`"2026-01-01"`, half-open `[start, end)`). Interval arithmetic (merge, gap detection) operates on calendar dates. Consumed by incremental backfill and gap detection (`incremental_models.md`).

### Run report (`reports/<run_id>.json`)

A run report is a summary artifact written alongside the run manifest at `.smelt/targets/<target>/reports/<run_id>.json`, sharing the manifest's `run_id`. Where the manifest is the durable per-model record consumed by `--resume` and history queries, the report is the human/tooling-facing summary of one run: counts of models by outcome (`success`/`failed`/`skipped`), total duration, and per-model error messages for any `failed` entry. It is derived entirely from the manifest and carries no information the manifest lacks; a report can always be regenerated from its manifest.

```jsonc
{
  "run_id": "20260604-141233-a1b2c3",
  "started_at": "2026-06-04T14:12:33Z",
  "completed_at": "2026-06-04T14:12:41Z",   // null for an incomplete (cancelled/aborted) run
  "duration_ms": 8123,                      // 0 when completed_at is null
  "outcome_counts": { "success": 5, "failed": 2, "skipped": 1 },
  "failures": [                             // one entry per `failed` model, empty if none
    { "model": "bad_a", "error": "Conversion Error: ...", "retry_count": 0 },
    { "model": "bad_b", "error": "Conversion Error: ...", "retry_count": 1 }
  ]
}
```

A report is written at every point a manifest is persisted — successful completion, cancellation, and abort — so a partial report (derived from an incomplete manifest, `completed_at: null`) is available immediately after a failed or cancelled run, not only after a subsequent successful one. Per "Abort semantics" above, `failures` names every model that failed in the aborting wave, never just the first.

### `--resume` semantics

`--resume` re-runs a previously-interrupted or partially-failed selection while skipping models that do not need to run again. A model is skipped when **both** hold: its outcome in the most recent *incomplete* run (the latest manifest with `completed_at: null`, or whose selection overlaps the current one and ended with at least one non-`success` outcome) is `success`, **and** its `definition_hash` in that manifest entry matches the model's current compiled-definition hash. A model whose prior outcome was `failed` or `skipped`, or whose definition has changed since, always re-runs under `--resume` — and so does every downstream dependent of any such model, since a dependent's own prior `success` said nothing about inputs that have since been rebuilt out from under it. `--resume` **refuses** (a hard error, not a warning) when there is no incomplete run to resume from — the most recent run for the target completed successfully, or no run manifest exists at all — rather than silently falling back to a full run: a typo'd or stale `--resume` invocation must never be mistaken for "nothing needed doing" (`architecture.md` §"Fail-loud discipline").

### Snapshot and environment store (virtual environments)

Under `state.mode: environments`, run state additionally records, per model: the **expanded logical SQL** that built each physical table, the **output fingerprint** of that SQL, and a map from `(environment, model)` to the physical table currently backing it. These drive fingerprint-keyed reuse and promotion (`virtual_environments.md`).

### Relationship to the reconciliation ledger

The run-state intervals this spec owns and the maintenance plan's **reconciliation ledger** — the `(output-region × column-group)` bookkeeping that records each region-group's processed-input vector, frontier watermarks for idempotent groups, and delta identities for additive groups (`incremental_models.md` §"The reconciliation ledger") — are **not the same mechanism and do not substitute for each other**. Run-state intervals are project-wide observability: they exist to answer "what has this project run, and where are the gaps" for humans and tooling, and a project may run at `state.mode: stateless` and forgo them entirely. The reconciliation ledger is **required correctness structure** for every `grain: key` (and `key_per_partition`) model maintained under a derived plan — it exists whenever the plan does, independent of `state.mode`, because it is what lets a fold-family technique detect a re-run (never fold a delta already in the entry's processed set) and lets a crashed run resume exactly. Neither reads the other: the reconciliation ledger is per-model, keyed by `(region × column-group)`, and today stored under `.smelt/reconciliation.json` for both storage gradings; an additive group's delta-identity grade is intended to move to warehouse-resident state, transactional with the fold, once a genuine keyed-fold execution path consumes it (`incremental_models.md` §"The reconciliation ledger", §Known Divergences). The interval ledger here is project-wide observability state, also under `.smelt/`. This spec also owns the **per-source landed-delta record** that forward propagation consumes (`sources.md` §"Landed-delta (derived, recorded)"; `incremental_models.md` §"The graph layer") — which partition intervals of a source landed, keyed by source address, recorded in run state alongside the interval ledger, not in the reconciliation ledger. A project could run `state.mode: stateless` with `grain: key` models and still get correct, ledger-enforced reprocessing refusal — the reconciliation ledger's presence is a property of the *plan*, not of the project's state posture. This spec continues to own the run-state **storage and serialisation** (`.smelt/` layout, manifest format, run IDs, landed-delta intervals); the reconciliation ledger's structure, grading, and operations are owned by `incremental_models.md`.

## Semantics

- **Stateless writes nothing.** Under `state.mode: stateless` (the default), no manifest, interval, snapshot, or environment record is written; `.smelt/` need not exist. State is created only when a higher posture is opted into.
- **A manifest is written per run.** `started_at` and `run_id` are set at run start; `completed_at` is set on successful completion. A manifest with `completed_at: null` denotes an interrupted run.
- **Recovery is idempotent re-run.** smelt does not checkpoint mid-run; recovery is re-running the same selection/range, which converges because each committed unit is idempotent (`incremental_models.md` §"Failure mode"). The interval ledger lets a re-run skip already-covered ranges and surface gaps.
- **Manifest evolution is backward-compatible.** Readers must tolerate historical manifests: every new field is `Option`al or `#[serde(default)]`. A required new field is a breaking change to stored state and is not permitted.
- **Snapshot reuse is keyed by fresh fingerprints, not stored hashes.** The persisted artifact for reuse is the **expanded logical SQL**, not its fingerprint. Equivalence is decided by fingerprinting the stored SQL and the current SQL with the *current* compiler at decision time (`output_fingerprint.md` §Design). The stored fingerprint, if recorded, is a cache/diagnostic, never the source of truth.
- **State is single-writer.** Only one smelt process may hold `.smelt/lock` at a time; a second acquisition attempt fails loudly naming the holder's PID rather than interleaving writes or corrupting an in-progress run's artifacts.
- **Every write is atomic.** No reader ever observes a partially-written state file — writes go to a temp file in the same directory, then rename.
- **Layout version is checked before any read or write.** An on-disk `state_version` greater than the current binary's highest known version is a hard error; a missing `meta.json` triggers the one-time legacy-layout migration to per-target paths under the lock, never a silent reinterpretation of root-level files as the new layout.
- **`--resume` decisions are keyed off outcome and definition hash together.** A `success` outcome alone is not sufficient to skip a model under `--resume` — the model's definition must also be unchanged since that run, so an edited model always re-executes even if its last recorded outcome was `success`.

## Design

**Opt-in, file-based, gitignored.** State is a posture a project opts into (`virtual_environments.md` §`state.mode`), not a baseline requirement. Storing it as plain JSON files under `.smelt/` (rather than a required embedded database) keeps a stateless project zero-cost and keeps state human-inspectable and easy to delete — a half-broken state store must never be harder to recover from than dropping a directory. Rationale: `incremental_models.md` §"Partition-grain design" "smelt does not own state"; research §6.

**Persist the SQL, treat the fingerprint as ephemeral.** Storing the expanded logical SQL that built each table — and recomputing fingerprints fresh on both sides at decision time — makes the fingerprint algorithm free to change between releases with no migration code and no version-stable-form contract. A stored hash would force a versioned normal form and golden cross-version tests; storing the SQL makes the comparison apples-to-apples by construction. Rationale: `output_fingerprint.md` §Design; research §5.6, Open Question 15.

**Run ID is timestamp-first.** A sortable `%Y%m%d-%H%M%S` prefix makes `runs/` browsable and history queries cheap; the hex suffix disambiguates same-second runs. Uniqueness assumes one smelt process at a time — concurrent runs are not a supported posture today.

**Per-target partitioning is the minimal environments answer.** A project running against both `dev` and `prod` targets must never have a `dev` run's interval ledger or reconciliation state answer a question about `prod` — that would let a developer's local backfill silently mask a gap in production, or a production reconciliation ledger entry silently satisfy a developer's `grain: key` model when it shouldn't. Nesting every run-scoped artifact under `.smelt/targets/<target>/` gives each target a closed, disjoint state store using the existing JSON-files mechanism, with no new storage technology — the deliberately minimal version of the fuller `virtual_environments.md` orchestration layer, which this spec's snapshot/environment store section still anticipates for the `state.mode: environments` posture.

**Locking and atomic writes prioritize "never lie" over throughput.** A `.smelt/` that reports incomplete or corrupted state after a crash is worse than one that refuses a second concurrent writer outright — a project's confidence in its interval ledger and reconciliation ledger depends on every recorded write being a fully-committed, non-interleaved one. Advisory locking (rather than, say, file-level OS locks that some filesystems don't support) and rename-based atomic writes are the cheapest mechanism that gives that guarantee without requiring an embedded database (see "Opt-in, file-based, gitignored" above).

**`meta.json` versioning is a trapdoor, not a migration framework.** The layout has changed exactly once so far (root-level files → `targets/<target>/`), and the spec commits to only one migration rule: missing `meta.json` means the one known legacy shape, and a version above the highest the binary understands is a hard stop. This is deliberately not a general N-to-N migration chain; if the layout changes again, that migration gets its own explicit rule rather than a generic version-diff engine, because the state directory is small, human-inspectable, and (per "Opt-in, file-based, gitignored" above) always safe to delete and let a project regenerate from a full run.

**`--resume` compares definition hash, not just outcome, to stay correct under edits.** A model's prior `success` says nothing about whether it should be skipped if its SQL changed since that run — skipping it would silently leave a stale table in place under a `--resume` that looks like it "succeeded". Recording `definition_hash` per model, per manifest entry, and requiring it to match the model's current compiled hash keeps `--resume` a pure optimization over a full re-run rather than a correctness trade-off: worst case (hash mismatch) is always "run again", never "skip incorrectly".

## Constraints & Invariants

- **Stateless requires no `.smelt/`.** Enabling no state posture must leave a project's on-disk footprint and behaviour exactly as today.
- **Fixed layout.** State is confined to `.smelt/meta.json`, `.smelt/lock`, and, per target, `.smelt/targets/<target>/runs/`, `.smelt/targets/<target>/intervals.json`, `.smelt/targets/<target>/reconciliation.json`, `.smelt/targets/<target>/landed_deltas.json`, `.smelt/targets/<target>/snapshots.json`, `.smelt/targets/<target>/schemas/`, and `.smelt/targets/<target>/reports/`. New artifact kinds extend this layout under `.smelt/`, never outside it.
- **Layout version gates every read and write.** A `state_version` this binary does not recognise is a hard error, never a best-effort read. A missing `meta.json` triggers exactly one migration path (legacy root-level layout → per-target layout), never a silent no-op.
- **Locking is mandatory around every state-mutating run.** `execute_project` acquires `.smelt/lock` before writing any state artifact and releases it on every exit path, including error. No code path writes under `.smelt/targets/` without holding the lock.
- **Writes are atomic or they don't happen.** Every `.smelt/` write (manifest, ledger, snapshot, schema, report, `meta.json`) goes through a temp-file-then-rename path; a direct in-place `write` to a tracked state file is a bug.
- **Forward-compatible manifests.** Stored JSON must remain readable by later smelt versions; new fields are optional/defaulted.
- **No stored-hash dependence for reuse.** Reuse correctness must not depend on a previously stored fingerprint value; it is always recomputed.

## Known Divergences / Open Questions

- **Interval-ledger key granularity is date-only, pending the incremental rewrite.** The interval ledger (§"Interval ledger") keys coverage by calendar-date string (`"2026-01-01"`), but incremental models routinely filter on sub-day (hourly/second) event-time boundaries. Whether the ledger keys move to RFC3339 instants (sub-day capable) is **coordinated with the incremental_models rewrite** rather than changed here, so the ledger granularity and the incremental cadence model land together. Tracked in `docs/plans/20260322-incremental-model-support.md` (`incremental_models.md`).
- **Snapshot / environment store is unbuilt.** Today `smelt-state` persists run manifests, the interval ledger, and deployed schemas. The expanded-logical-SQL snapshot, the recorded output fingerprint, and the `(environment, model) → table` map are **specified here but not implemented**; they arrive with the virtual-environments orchestration layer. Tracking: `docs/research/20260601-virtual-environments.md` §8.
- **JSON files vs. embedded store.** Research §8 sketched an embedded `.smelt/state.db`; the implementation uses JSON files (`runs/*.json`, `intervals.json`, `schemas/*.json`). The current normative layout is the JSON form; whether to move to an embedded store as the snapshot/environment map grows is open.
- **Concurrency across processes is now specified, not just assumed.** Earlier revisions of this spec left concurrent-process behaviour unspecified; §"Locking" now specifies single-writer advisory locking as the answer — a second process fails loudly rather than interleaving writes. Parallel *model* execution within one run is a separate concern, owned by `smelt-runtime`, not by this layout.
- **Manifest format completeness.** The fields above reflect `smelt-state` today; the full manifest contract (e.g. per-model error capture, retries) is still settling and may add optional fields.
- **"No change to correctness" no longer holds unconditionally once forward propagation lands.** §"Relationship to the reconciliation ledger" describes run-state intervals as observability a project can forgo with no correctness impact; that was true while every run recomputed its own dirty set from scratch. Forward propagation (`smelt run --since-upstream`, `incremental_models.md` §"The graph layer") instead computes what must run from **recorded per-source landed deltas**, so a project that opts out of persisting them loses the input forward propagation needs and falls back to a full recompute rather than the derived dirty set. This is a known divergence until forward propagation and the reconciliation ledger land (`docs/plans/20260707-maintenance-plan-impl.md` phases MP14/MP15).
- **Retry interaction with `outcome` records one entry per model, not one per attempt.** A model that fails N times then succeeds on retry (or exhausts retries and fails) records a single manifest/report entry for that model, with `retry_count` set to the number of retries made before the final outcome — never one entry per attempt.

## References

- **Code**: `crates/smelt-state/src/lib.rs` (`RunManifest`, `ModelRunRecord`, `TimeRangeRecord`, `generate_run_id`), `src/file_store.rs` (`.smelt/` reader/writer), `src/intervals.rs` (`IntervalStore`), `src/reconciliation.rs` (`ReconciliationLedger`, `ReconciliationStore`), `src/landed_deltas.rs` (`LandedDeltaStore`, `SourceLanding`, per-source landed-delta recording), `src/schema_tracking.rs` (`DeployedSchema`), `src/history.rs` (history queries)
- **Tests**: `crates/smelt-state/tests/`
- **User docs**: none yet (CLI surfaces `smelt status` / `smelt history` over this state — see `cli.md`)
- **Plans (history)**: `docs/plans/20260719-prod-w2-operability.md` — locking, atomic writes, versioned/per-target layout, `--resume`, run reports; predecessor research is `docs/research/20260601-virtual-environments.md`
- **Related specs**: `incremental_models.md` (interval semantics, idempotent recovery; the transactional merge ledger — required correctness structure, distinct from this spec's opt-in observability), `schema_evolution.md` (deployed-schema snapshots and migration), `virtual_environments.md` (the snapshot/environment consumers), `output_fingerprint.md` (the equivalence key), `architecture.md` (`state.mode` surface, `smelt-state` crate)
