# State & Recovery

Normative reference for what `.smelt/` records, how `smelt status`/`smelt history` and `--resume` read it, and how to recover from state loss or corruption. See [Production Deployment](../guide/deployment.md#smelt-state-layout) for the directory layout summary and backup/restore procedure, and [Orchestration](../guide/orchestration.md) for how a scheduler consumes this state across runs.

## Inventory

Every artifact `.smelt/` can contain, and what it records:

| Path | Records |
|------|---------|
| `.smelt/meta.json` | `{ "state_version": <int> }` -- the on-disk layout version. |
| `.smelt/lock` | The advisory single-writer lock, held for a run's duration; contains the holding process's PID. |
| `.smelt/targets/<target>/runs/<run_id>.json` | One run manifest per execution: per-model outcome (`success`/`failed`/`skipped`), strategy, row count, duration, `definition_hash`, and `retry_count`. |
| `.smelt/targets/<target>/reports/<run_id>.json` | The run-report artifact -- a summary derived entirely from the matching manifest: outcome counts, total duration, and per-failure error text. Written at every point a manifest is persisted, so a report exists even for an interrupted or aborted run. |
| `.smelt/targets/<target>/intervals.json` | Cumulative interval coverage per incremental model, keyed by calendar date (half-open `[start, end)`); read by `smelt status` for gap detection and by backfill planning. |
| `.smelt/targets/<target>/reconciliation.json` | The reconciliation ledger -- per `grain: key` (and `key_per_partition`) model, the processed-input vector for each `(output-region x column-group)` cell (see "The reconciliation ledger" below). |
| `.smelt/targets/<target>/landed_deltas.json` | Per-source landed-delta intervals -- which partition intervals of each source have landed, keyed by source address; consumed by `smelt run --since-upstream` forward propagation. |
| `.smelt/targets/<target>/schemas/<model>.json` | The deployed schema snapshot for one model -- column names/types/nullability as of its last successful write; consumed by schema-evolution diffing. |
| `.smelt/targets/<target>/snapshots.json` | Fingerprint-keyed reuse snapshots (`state.mode: environments` only). |

Every run-scoped artifact nests under `.smelt/targets/<target>/`; only `meta.json` and `lock` live at the project root, shared across every target. A file is created lazily the first time something is recorded into it -- a project that has never run an incremental model has no `intervals.json`, and one with no `grain: key` models has no `reconciliation.json`.

## Run manifest

`.smelt/targets/<target>/runs/<run_id>.json` records one entry per model that ran this run.
Alongside outcome, strategy, row count, duration, `definition_hash`, and `retry_count` (see
"Inventory" above), a model entry carries a `probes` array — one entry per declared-fact probe
(`docs/specs/model_properties.md` §"Probe obligation") this run consulted for that model:

```json
{ "fact": "functional_dependencies:", "probe": "DeclaredFunctionalDependencyViolated", "outcome": "dispatched" }
```

`outcome` is one of two values: `dispatched` (the probe actually ran and held — a violation would
have failed the run before any write, so a recorded entry is always a clean bill of health, never
a silent pass on a violation) or `skipped` (the project's [`probes:` cadence](smelt-yml.md#probes-configuration)
skipped this probe on this run — the declaration is trusted, not verified, and the manifest says
so explicitly rather than making a skipped check look identical to a checked one). A model
declaring no probe-backed fact has an empty `probes` array. Reading an older manifest with no
`probes` key at all defaults to an empty array, not a parse failure — manifest evolution is
backward-compatible.

## State-schema version and migration

`.smelt/meta.json` records the layout version this store was written with. A version this smelt binary does not recognise (newer than the binary's own highest known version) is a hard error naming both versions -- smelt refuses to read or write rather than guess at an unfamiliar layout. A missing `meta.json` denotes the legacy pre-versioning layout (root-level `runs/`, `intervals.json`, etc. with no `targets/<target>/` nesting); the first locked open under a version-aware binary migrates it to the current layout and writes `meta.json`, under the advisory lock so a concurrent process can never observe a half-migrated store.

## Locking

A run acquires an exclusive advisory lock on `.smelt/lock` for its entire duration and releases it on completion or error. A second smelt process that tries to acquire the lock while it is held fails loudly -- the error names the PID recorded in the lock file -- rather than interleaving writes with the first process. The lock is project-wide, not per-target, and is held for the whole run regardless of how many models execute concurrently under `--jobs`; every write to a shared per-target file (the interval ledger, the landed-delta store, the reconciliation ledger) is additionally serialized within the locked run so two models finishing at the same moment cannot each drop the other's write. Every write under `.smelt/` goes through a temp-file-then-rename path, so a process killed mid-write leaves either the old file intact or the new one -- never a truncated one.

## What `smelt status` and `smelt history` read

`smelt status [MODEL_NAME] --target <target>` reads that target's `intervals.json` and reports covered ranges plus any gap between the last covered date and today (or a `--since`/`--until` window). `smelt history [MODEL_NAME] --target <target> --limit <n>` reads that target's `runs/*.json` manifests, most recent first, showing per-model strategy, row counts, and duration for each run. Both are read-only -- neither command acquires the write lock or mutates `.smelt/`.

## `--resume` semantics

`smelt run --resume` (or `smelt build --resume`) re-runs a previously interrupted or partially-failed selection while skipping models that do not need to run again. A model is skipped when **both** hold: its outcome in the most recent *incomplete* run (a manifest with `completed_at: null`, or one whose selection overlaps the current run and ended with at least one non-`success` outcome) was `success`, **and** its `definition_hash` in that manifest entry matches the model's current compiled-definition hash. A model that previously `failed` or was `skipped`, or whose definition changed since, always re-runs -- and so does every downstream dependent of any such model, since a dependent's prior `success` said nothing about inputs rebuilt out from under it. `--resume` refuses with a hard error (never a silent full run) when there is no incomplete run to resume from -- the most recent run completed successfully, or no run manifest exists at all -- so a stale or typo'd `--resume` invocation is never mistaken for "nothing needed doing."

## The reconciliation ledger

A **frontier** is the record of which typed deltas a cell has absorbed. The reconciliation ledger
is the frontier's warehouse-independent realization: `grain: key` (and `key_per_partition`)
models are maintained through a merge into an existing table rather than a full recompute, and
the reconciliation ledger is the correctness structure that makes repeated, possibly-overlapping
runs safe. (A window-forward keyed model's own merge writes a second realization, the
transactional frontier write, directly into the target table -- see
[Incremental models -- The reconciliation ledger](../guide/incremental-models.md#the-reconciliation-ledger).)
Each ledger entry keys a `(output-region x column-group)` cell to the processed-input vector that
has already been folded into it. Storage is graded by the column-group's algebra:

- **Additive** groups (a running sum, a count) record the **delta identities** already folded in -- re-folding the same delta a second time would double-count, so the ledger's job is to refuse a repeat.
- **Idempotent** groups (a last-write-wins column, a MAX aggregate) record only a **frontier** watermark -- re-applying an already-seen delta is harmless, so the ledger only needs to know how far processing has reached.

Two operations act on the ledger: **fold** (extend an entry with a new delta, refusing if that delta is already in the entry's processed set) and **recompute-reset** (a region recompute resets every intersecting entry to exactly the input it read). For an additive-graded cell, the fold is transactional with the write it accompanies -- on DuckDB targets, the delta identity is folded into a warehouse-resident ledger table in the same transaction as the merge, so a repeat delta violates a primary key and refuses the run before the merge executes a second time, rather than after. On a backend with no ledger substrate, an idempotent-graded model's bookkeeping record is skipped -- the run still succeeds, and it says so on the run's progress output rather than skipping silently.

This ledger is distinct from the interval ledger (`intervals.json`) above: intervals are project-wide observability ("what has this project run, and where are the gaps") that a `state.mode: stateless` project can forgo entirely; the reconciliation ledger is required correctness structure for every plan-managed `grain: key` model, present whenever the plan is, independent of `state.mode`.

## Recovery playbook

**`.smelt/` is lost (deleted, never backed up, new environment).** State is regenerable: an incremental model's interval ledger and reconciliation ledger rebuild from a full re-run over the model's complete history; there is no data loss to the warehouse tables themselves, only a loss of the bookkeeping that lets *future* runs skip already-covered work. Re-run the affected selection with a full time range (or the whole project) to rebuild coverage from scratch.

**`.smelt/` is corrupt (malformed JSON, truncated file, manually edited).** Because every write is atomic (temp file + rename), a file smelt itself wrote cannot be left truncated by a crash -- a corrupt file is evidence of something else (disk-level corruption, an out-of-band edit, a copy taken mid-write by a tool that doesn't respect the rename). smelt fails loudly with a parse or read error naming the file rather than silently discarding the record; there is no automatic quarantine-and-continue. Treat a corrupt `.smelt/` the same as a lost one: remove the affected file (or the whole directory) and let the next run regenerate it. Restore from a known-good backup first if one exists, to avoid losing more coverage history than necessary.

**A run was interrupted mid-flight (process killed, machine restarted).** The run's manifest is left with `completed_at: null`; a matching partial run report is available immediately (see "Inventory" above). Nothing needs manual cleanup: `.smelt/lock` is an OS-level advisory lock (`flock`-style), which the operating system releases automatically when the holding process exits or is killed -- there is no stale-lock file to remove by hand, even after a crash. Re-run with `--resume` to pick up exactly the models that did not reach `success`, or omit `--resume` for an unconditional full re-run of the selection.

**Backup and restore.** See [Production Deployment -- Backup and restore](../guide/deployment.md#backup-and-restore) -- `.smelt/` is plain JSON, safe to copy with any file-level backup tool between runs (or under the same lock your other maintenance tooling uses for a live backup).
