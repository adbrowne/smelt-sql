# CLI Commands Reference

The `smelt` command-line interface provides commands for running models, inspecting schemas, managing incremental state, and more.

## Top-Level Flags

These flags are accepted at the root command and work without a project (no `smelt.yml` required):

| Flag | Short | Description |
|------|-------|-------------|
| `--help` | `-h` | Print usage and exit. Per-subcommand `--help` prints that subcommand's flag table. |
| `--version` | `-V` | Print the installed package version (`smelt 0.x.y`) and exit. |

`smelt --version` and `smelt --help` are the only invocations that succeed without a subcommand.

## Common Flags

The following flags appear on most subcommands:

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--project-dir` | path | `.` | Path to the smelt project root (directory containing `smelt.yml`) |
| `--database` | path | _(from smelt.yml)_ | Override the DuckDB database file path |
| `--target` | string | `dev` | Target environment name as defined in `smelt.yml` |
| `--scope` | string | _(cwd-derived)_ | Dot-path prefix for expanding bare model names. Pass `--scope ''` to disable auto-scoping. |
| `--log-format` | `text` \| `json` | `text` | Log line format. `json` emits one parseable JSON object per tracing line, for orchestrator/log-aggregator consumption. Global — set at the root command, applies to every subcommand. |

---

## Exit codes

Every `smelt` subcommand follows the same exit-code contract, so orchestrators (cron, Airflow, CI) can branch on it without parsing stdout:

| Code | Meaning |
|------|---------|
| `0` | Success. Includes a `warn`-severity `smelt check` violation and an empty-but-valid selection — a build that ran nothing because there was nothing to do is not a failure. |
| `1` | Detected failure. A failed model build, a failed `smelt test` case, an `error`-severity `smelt check` violation, `smelt diff` detecting a schema change, or a check referencing a model not built in the target. |
| `2` | Usage error. Malformed CLI arguments, a malformed or missing `smelt.yml`, or an unresolvable project root. |

`1` means the command ran correctly and found a problem in the data or models — investigate the pipeline. `2` means the command could not run at all because its own inputs were invalid — fix the invocation. Retrying a `2` without changing the command is never useful.

---

## State lock errors

Any command that mutates run state (`smelt run`, `smelt build`, `smelt backbuild`) takes an exclusive lock on `.smelt/lock` for its duration and releases it on completion or error. A second invocation started while the first is still running fails immediately with:

```
Error: state locked by PID <n>
```

rather than silently interleaving writes with the in-flight run. This is expected when two runs are launched concurrently against the same project (e.g. an overlapping cron schedule, or a manual run started while a CI job is still going) — it is not a corruption signal. Wait for the process named by `<n>` to finish, then re-run.

The lock is an OS-level advisory file lock (`flock`), not a hand-rolled PID file: if the holder process is killed or its container crashes, the operating system releases the lock automatically when the process's file descriptors close, so a stuck lock left behind by a dead process is not expected. If a `state locked by PID <n>` error persists after confirming `<n>` is no longer running, that indicates the lock file lives on a filesystem that doesn't honor advisory locks (e.g. certain network mounts) rather than a normal stale-lock condition.

See `docs/specs/run_state.md` §"Locking" for the full semantics.

---

## State isolation per target

Run state lives under `.smelt/targets/<target>/`, keyed by the `--target` a command ran against (default `dev`). A run against `dev` and a run against `prod` never share interval coverage, reconciliation ledgers, deployed-schema snapshots, or run history — each target has its own closed, disjoint state store. This means:

- `smelt run --target prod` and `smelt run --target dev` can each be resumed, inspected, and reasoned about independently; a `dev` backfill can never mask a coverage gap in `prod`.
- `smelt status`, `smelt history`, and `smelt diff` accept `--target` (default `dev`) and report on that target's state only — pass the target you actually care about, especially in CI where the default `dev` is rarely the one that matters.
- `.smelt/meta.json` (the layout-version marker) and `.smelt/lock` (the advisory single-writer lock) are the only files shared across every target — see `docs/specs/run_state.md` §"`.smelt/` directory layout".

See `docs/specs/run_state.md` §"`.smelt/` directory layout" for the full on-disk shape.

---

## Argument resolution and `--scope`

Every command that takes an entity identifier — a model name in `--select`, a positional model argument to `smelt type`, `smelt table`, `smelt status`, etc. — resolves it using a three-shape input rule.

### The three input shapes

| Shape | Example | What happens |
|-------|---------|--------------|
| **Full path** | `silver.events_parsed` | Resolved as-is against the project. Always works. |
| **Scoped shorthand** | `events_parsed` (with scope `silver`) | Expanded to `silver.events_parsed` and resolved. Falls back to the bare argument if the expanded form does not exist. |
| **No-scope bare leaf** | `events_parsed` (no scope set) | Resolved as a full path. Errors if no entity with that exact path exists, even if a same-named entity exists in a sub-namespace. |

All smelt output — model lists, type signatures, `smelt explain --json` keys, log lines — uses the full canonical dot-path (e.g. `silver.events_parsed`) regardless of how you typed the identifier. Scope changes what you type; it never changes what smelt prints.

### Scope sources (highest precedence first)

1. **`--scope <prefix>` flag.** Pass a dot-path such as `silver` or `marts.daily`. Whitespace and the literal `smelt.` prefix are rejected.
2. **Working-directory derivation (auto).** When your current directory is inside a project's scan root (`models/` by default), smelt derives the scope from the path components between the scan root and your cwd. For example, `models/silver/` auto-scopes to `silver`; `models/marts/daily/` auto-scopes to `marts.daily`. Being at or above the scan root produces no auto-scope.
3. **No scope.** The argument must be a full path. Bare leaves error unless they are themselves full paths.

`--scope ''` (empty string) forces no scope regardless of cwd. Use this in scripts and CI where the working directory should not influence resolution.

### Worked examples — `web_analytics` project

The `web_analytics` example has a `models/silver/events_parsed.sql` model. All three forms below resolve to the same entity and produce identical output:

```bash
# 1. Full path — works from anywhere inside the project
smelt type silver.events_parsed

# 2. Explicit scope flag — same result without typing the namespace
smelt --scope silver type events_parsed

# 3. Cwd auto-scope — smelt derives "silver" from the working directory
cd models/silver
smelt type events_parsed --project-dir <project-root>
```

All three print the canonical path in their output:

```
silver.events_parsed:
  (raw_events: {...})
  -> {event_id: BIGINT, device_id: INTEGER, ...}
```

### Bare-leaf error and the `did you mean` hint

Running `smelt type events_parsed` from the project root (no auto-scope, no `--scope` flag) errors:

```
Error: Model 'events_parsed' not found. Did you mean 'silver.events_parsed'?
```

When the leaf matches multiple entities (e.g. both `silver.events_parsed` and `bronze.events_parsed` exist), the error lists all candidates and suggests using `--scope` or the full path.

### Selectors and `--scope`

`--select` and `--exclude` values are expanded through the same resolution rule. A bare name like `--select events_parsed` with scope `silver` active is expanded to `--select silver.events_parsed` before the selector engine runs. Tag selectors (`tag:staging`, `tag:revenue+`, etc.) contain a `:` and are passed through unchanged — they are not entity identifiers.

---

## smelt init

Non-interactively scaffold a minimal, working smelt project.

**Usage:**

```
smelt init [DIR]
```

`smelt init` writes a `smelt.yml`, a `models/` directory containing one example model, one seed CSV, and a `.gitignore` excluding `.smelt/` and the database file, to `DIR` (default `.`, created if it doesn't exist). Every file it writes is a fixed, deterministic template — there are no interactive prompts and no flags that change what gets scaffolded beyond the target directory. The scaffolded project builds successfully against DuckDB with no further edits (`smelt build` inside it exits `0`).

`smelt init` refuses to run against a directory that already contains a `smelt.yml`: it exits `2` with a message naming the conflicting file, rather than overwriting or merging. There is deliberately no `--force` flag to override this — run `smelt init` in a fresh directory, or remove the conflicting `smelt.yml` and re-run.

**Exit codes:**

- Exits `0` on a successful scaffold.
- Exits `2` if `DIR` already contains a `smelt.yml` (usage error — the fix is a different or empty directory, not a retry).

**Examples:**

```bash
# Scaffold a new project in ./my-project
smelt init my-project

# Scaffold into the current directory
smelt init
```

---

## smelt run

Run models and materialize them in the target database. This is the primary command for executing your data pipeline.

**Usage:**

```
smelt run [OPTIONS]
```

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--database` | | path | _(from config)_ | DuckDB database file path |
| `--target` | | string | `dev` | Target environment from smelt.yml |
| `--show-results` | | bool | `false` | Display query results after execution |
| `--verbose` | `-v` | bool | `false` | Show compiled SQL for each model |
| `--dry-run` | | bool | `false` | Print the maintenance statements that would run — without executing (see below) |
| `--show-plan` | | bool | `false` | Print the resolved execution plan (model names + strategies) and exit, without executing. Combine with `--dry-run` to see the plan without any execution side effects. |
| `--event-time-start` | | string | | Start of event time range for incremental models (ISO 8601: YYYY-MM-DD). Requires `--event-time-end`. |
| `--event-time-end` | | string | | End of event time range for incremental models (exclusive, ISO 8601: YYYY-MM-DD). Requires `--event-time-start`. |
| `--start` | | string | | Alias for `--event-time-start` |
| `--end` | | string | | Alias for `--event-time-end` |
| `--select` | `-s` | string[] | | Select models to run (repeatable). Supports: `model_name`, `tag:X`, `+tag:X`, `tag:X+`, `+tag:X+` |
| `--exclude` | `-e` | string[] | | Exclude models from the run (repeatable). Same syntax as `--select`. |
| `--batch-size` | | integer | | Override batch size in days for backfill chunking |
| `--per-partition` | | bool | `false` | Force per-partition execution (one query per granularity period) |
| `--auto` | | bool | `false` | Auto mode: process only uncovered intervals since last run |
| `--allow-column-removal` | | bool | `false` | Allow column removal during schema evolution (otherwise blocked for safety) |
| `--allow-full-refresh` | | bool | `false` | Allow full table refresh when schema changes cannot be handled with ALTER TABLE (e.g., incompatible type changes, or unsupported operations on Spark+Parquet). See [Schema Evolution](../guide/schema-evolution.md). |
| `--allow-downgrade` | | bool | `false` | Allow incremental models that fail the safety classifier to fall back to full-table refresh instead of being refused at planning time. A temporary escape hatch while fixing the model SQL, not a normal-operation flag. |
| `--since-upstream` | | bool | `false` | Forward propagation: run exactly the partitions dirtied by the declared per-source deltas below, computed through the maintenance-plan propagation graph. See [Forward propagation with `--since-upstream`](#forward-propagation-with---since-upstream). |
| `--source` | | string[] | | A source **or upstream maintained-model** address whose landed delta is declared via the paired `--landed` flag, or resolved from a persisted watermark when `--landed` is omitted for it (repeatable). Only meaningful with `--since-upstream`. |
| `--landed` | | string[] | | The landed interval for the paired `--source`: bare `<start>..<end>` (pairs positionally, the Nth `--landed` with the Nth `--source`) or `<address>=<start>..<end>` (pairs by address) — ISO `YYYY-MM-DD`, end exclusive. Repeatable; see `--source`. |
| `--jobs` | `-j` | integer | _(available parallelism)_ | Maximum number of models to execute concurrently. `--jobs 1` forces strictly serial execution — one model at a time, in the same order as every prior `smelt` release. See [Parallel execution with `--jobs`](#parallel-execution-with---jobs). |
| `--resume` | | bool | `false` | Resume a previously partially-failed run: skip any model that succeeded last time with an unchanged definition, and rerun everything else. See [`--resume` — continue after a partial failure](#--resume--continue-after-a-partial-failure). |

### Parallel execution with `--jobs`

The run engine (shared by `smelt run`, `smelt build`, and `smelt backbuild`) dispatches models as a topological **wavefront**: a model starts only once every one of its upstream dependencies in the current run has finished, but models with no dependency relationship to each other may run concurrently. `smelt run --jobs` bounds how many models are in flight at once:

- Omitted (the default) — resolves to the host's available parallelism (`std::thread::available_parallelism()`), typically the number of logical CPUs.
- `--jobs 1` — strictly serial: one model at a time, in `execution_order`. This is the pre-`--jobs` behavior and remains available as an explicit opt-out.
- `--jobs N` (`N > 1`) — up to `N` models run concurrently, always subject to the dependency graph: an edge `A -> B` guarantees `A` fully completes (including any `smelt.check`s in a `smelt build`) before `B` starts, regardless of `N`.

Progress output, the per-run manifest, and every other run-report artifact are identical regardless of `--jobs` — the run engine buffers each model's progress events internally and replays them in the same deterministic `execution_order` sequence a `--jobs 1` run would have produced, whichever models actually finished first. A failure stops the scheduler from starting further models; any already in flight are allowed to finish, and every model that completed (successfully or not) before the run stopped is still recorded in the manifest.

Concurrency helps most when a project's DAG is wide (many independent models per layer) and when a meaningful share of a run's wall-clock time is spent on work other than the backend query itself (SQL compilation, schema-evolution checks, `smelt.check` execution) — a single-connection backend (e.g. DuckDB) still serializes concurrent query execution against that one connection, so `--jobs` primarily shortens a run by overlapping that surrounding work, and by overlapping models assigned to *different* targets.

### Retrying transient backend failures

`smelt run`, `smelt build`, and `smelt backbuild` automatically retry a model's write step when it fails with a **transient** backend error — a dropped connection, a connection-pool timeout, or similar environmental failure that a fresh attempt against the same input is likely to clear. A flaky connection partway through a long run does not have to fail the whole run.

A retry always re-runs the model's *entire* write step for the attempt that failed — the full drop-and-recreate for a table, one incremental batch's complete DELETE+INSERT, a column-scoped MERGE, or a keyed model's create-or-merge partition write — never a partial slice of it, so a retried model never leaves a half-applied write behind. Coverage is uniform across every write technique a model can dispatch to, including the delta-restricted recompute a model-edge creation trigger can take. Retries use exponential backoff between attempts; the delay is derived deterministically from the run and model identity rather than real-clock jitter, so repeated runs behave predictably.

Only transient failures are retried. A deterministic failure — invalid SQL, a type mismatch, a constraint violation, a missing table, an unsupported dialect feature — fails the model on the first attempt, since retrying cannot change the outcome.

By default, up to 3 attempts are made per write step before the model is reported as failed. To disable retry entirely (fail immediately on the first transient error, matching pre-retry behavior), set `retry_max: 0` on the run request. There is currently no dedicated CLI flag for this; it is available to programmatic consumers of the run engine (e.g. the UI) via the `retry_max`/`retry_backoff_ms` fields on the run request.

### Run report and failure summary

Every `smelt run`/`smelt build`/`smelt backbuild` invocation against a stateful project writes a **run report** alongside its run manifest, at `.smelt/targets/<target>/reports/<run_id>.json` (`docs/specs/run_state.md` §"Run report"). Where the manifest is the durable per-model record `--resume` reads, the report is the human/tooling-facing summary: counts of models by outcome, total duration, and per-model error text for anything that failed. A report is written whether the run succeeds, is cancelled, or aborts, so a partial report is available immediately after a failed run.

When independent models fail in the same run — even concurrently, in the same `--jobs`-scheduled wave — every one of them gets its own recorded error; a second or third failure is never silently downgraded to "skipped". At the end of a failed run, `smelt` prints a failure summary naming every failed model with its first error line and a one-line hint toward the likely next action:

```
smelt: run 20260720-120001-a1b2c3 failed — 2 model(s) failed:
  - bad_a: Conversion Error: Could not convert string 'not_a_number' to INT32
    hint: re-run with -v for the full backend error, or `smelt run --show-plan` to inspect the plan
  - bad_b: Conversion Error: Could not convert string 'also_not_a_number' to INT32
    hint: re-run with -v for the full backend error, or `smelt run --show-plan` to inspect the plan
```

The hint is chosen from a coarse classification of the error text — a compile-time failure (parse/type/reference resolution) points at the model's SQL; a backend execution failure points at `-v`/`--show-plan`; a check/constraint failure points at `smelt check`. Classification is best-effort text matching, not a structured error code, since the underlying error is already flattened to a string by the time the report captures it.

### `--resume` — continue after a partial failure

`smelt run --resume` re-runs a selection that previously failed partway through, skipping models that already succeeded and don't need to run again. A model is skipped when **both** hold: it succeeded in the most recent partially-failed run, and its definition hasn't changed since (same compiled SQL). A model that failed, was skipped, or whose SQL was edited always re-runs — and so does every model downstream of it, since a downstream model's own prior success said nothing about inputs that have since been rebuilt.

```bash
# A run fails partway through — some models succeeded, one failed, the rest
# never started.
smelt run
# ... "silver.sessions" fails ...

# Fix the underlying issue (bad data, a transient outage, a bug in the
# model), then resume: already-succeeded upstream models are skipped;
# "silver.sessions" and everything downstream of it re-run.
smelt run --resume
```

The run resumed from is the latest one that either never finished (interrupted by an error or a cancellation) or that did finish but still recorded at least one non-success outcome for a model overlapping the current selection — for example a check failure that skipped that model's downstream dependents without aborting the whole run.

`--resume` refuses — a hard error, not a warning — when there is nothing to resume from: the most recent run for the target completed with every model successful, or no run manifest exists yet at all. This is deliberate: a stale or mistaken `--resume` must never be silently reinterpreted as a full run. Run `smelt run` without `--resume` (or remove `.smelt/`) to start fresh.

A resumed-away model's materialized table and interval-ledger bookkeeping are left completely untouched — `--resume` only decides which models to skip *executing*, it never rewrites or re-derives state for a model it isn't running.

**Selector syntax:**

The `--select` and `--exclude` flags support graph-aware selection:

- `model_name` -- select a single model by name (subject to scope resolution; see [Argument resolution and `--scope`](#argument-resolution-and-scope))
- `silver.model_name` -- select by full canonical path (always unambiguous)
- `tag:analytics` -- select all models with the `analytics` tag
- `+model_name` -- select the model and all its upstream dependencies
- `model_name+` -- select the model and all its downstream dependents
- `+model_name+` -- select the model, its upstreams, and its downstreams

Tag selectors (`tag:...`) are not subject to scope expansion and are passed through unchanged.

**Examples:**

```bash
# Run all models in the project
smelt run

# Run with incremental time range
smelt run --start 2026-01-01 --end 2026-01-08

# Run a specific model by full canonical path
smelt run --select silver.events_parsed

# Run with scope shorthand (equivalent to the above when scope is silver)
smelt --scope silver run --select events_parsed

# Run only models with the "staging" tag, showing compiled SQL
smelt run --select tag:staging --verbose

# Dry run to validate without executing
smelt run --dry-run

# Auto mode: process only new intervals
smelt run --auto
```

### `--dry-run` — inspect the maintenance statements before they run

`smelt run --dry-run` and `smelt backbuild --dry-run` print, for every model the invocation would execute, the **maintenance statements** the run would execute — the region `DELETE`+`INSERT` pair (or keyed `MERGE`, etc.) a maintained model rebuilds its window with — not merely the compiled `SELECT` body. The statements are the output of the same statement emitters a real run consumes, so what you see is what would run. Region bounds are **real**: they come from the invocation's own `--event-time-start`/`--event-time-end` window, never symbolic placeholders. A transactional group is bracketed by `BEGIN`/`COMMIT` lines to show its atomicity. Nothing is executed and no backend connection is opened.

`smelt backbuild --dry-run` additionally reflects the **chunking** a real backbuild performs: when a model's batch-safety classification (or an explicit `--batch-size`/`--per-partition`) splits the range, the statements print once per chunk, each introduced by a boundary line naming its `[start, end)` window and position — `-- chunk 2/4: [2026-03-08, 2026-03-15)` — in the order a real backbuild would execute them. An auto-chunked backfill is thereby fully inspectable before it runs.

Division of labour with [`smelt explain <model> --show-sql`](#smelt-explain): `--show-sql` is the no-window, single-model plan-inspection surface (symbolic bounds unless `--period` is given); `--dry-run` is the "exactly what would **this invocation** do" surface — real window, real selection, real chunking.

### Forward propagation with `--since-upstream`

Every `refresh: incremental` model with a declared `grain:` derives a maintenance plan whose cells carry a derived scan clamp per input — the same window the maintenance SQL itself reads (`smelt explain <model>` prints it). `--since-upstream` composes those clamps into a propagation graph and walks it forward from **caller-declared** per-source deltas: for each `--source <address> --landed <start>..<end>` pair, the delta reflects through every downstream edge, dirtying exactly the partitions that delta can affect, recursively through the dependency chain. The dirty set is printed before anything runs, then `smelt` runs exactly those `(model, region)` pairs — never a partition outside the propagated set.

`--source` and `--landed` are repeatable. A `--landed` value pairs one of two ways: bare `<start>..<end>` pairs positionally (the first `--source` with the first `--landed`, and so on — requires equal counts), or address-qualified `<address>=<start>..<end>` pairs by address, with no positional constraint. The two spellings must not be mixed in one invocation. A `--source` with no paired `--landed` resolves its delta from a persisted **watermark** — the point a prior completed run already propagated that source through (a field on the same per-source landed-delta record, written once every model consuming the source finishes a run with a window end) — as the span `watermark → now`, refined live wherever a recorded observed-delta exists. A `--source` with neither a paired `--landed` nor a persisted watermark fails the run with a named error identifying the source and the missing watermark — there is no implicit whole-table fallback and no silent skip. The runner (or an external poller that watches the real upstream systems) is responsible for telling `smelt` what landed for a source no prior run has covered; a cron tick is only the trigger to ask.

An unclocked source's delta dirties the whole downstream model for every consumer sensitive to it — never a silent no-op, since that cell was only ever admitted under an explicit full-scan acceptance. A source address may be given as a bare name (`bronze`), with its `sources.` breadcrumb (`sources.bronze`), or with the full `smelt.` prefix (`smelt.sources.bronze`) — all three resolve identically.

`--source` accepts any **clocked provider** as the delta origin — a declared source, or an upstream maintained model, whether ordinary (`grain: partition`/`key_per_partition`) or a **locality-admitted composed model** (`grain: key` plus an admitted `timeseries:` block — see [the composed shape](../guide/incremental-models.md#the-composed-shape-key-time)). A model origin's landed delta is the output window a completed run wrote for it. The delta reflects through that model's downstream edges exactly as a source delta does (the model-to-model edge is derived from the same scan clamp `smelt explain` reports for it — a composed origin's edge carries its admitted route's own margin), and the origin model itself is never re-run. The address is resolved against the workspace: an address that is neither a declared source nor a maintained model is a named error, not a silent no-op. A **bare** keyed model (no admitted `timeseries:`) still refuses fail-loud as a `--source` origin, the same as anywhere else in the propagation graph — it has no partition axis for interval dirt to propagate over.

```bash
# silver.events_parsed finished a run over Jan 3; propagate that landed
# window to everything downstream of it (the origin model is not re-run).
smelt run --since-upstream \
  --source silver.events_parsed --landed 2026-01-03..2026-01-04
```

A model whose dependency graph contains a cycle, a self-reference, or a **bare** keyed-grain node (no admitted time axis, so no partition axis for interval dirt to propagate over) refuses the whole `--since-upstream` invocation with a named error rather than guessing. A locality-admitted composed node is not refused — it participates in propagation like any other clocked node, both as an intermediate stage and as the `--source` origin itself.

```bash
# Two sources landed data since the last propagation; run exactly the
# partitions each delta can affect.
smelt run --since-upstream \
  --source sources.raw.events --landed 2026-01-03..2026-01-04 \
  --source sources.raw.users --landed 2026-01-07..2026-01-08
```

---

## smelt backbuild

Rebuild a target model and all its upstream dependencies for a specified time range. Useful for backfilling historical data or repairing a specific model and everything it depends on.

`smelt backbuild` handles both `grain: partition` and `grain: key` incremental models uniformly. For `grain: partition` models, it applies the DELETE+INSERT (or append/insert-overwrite) strategy over the requested window. For `refresh: incremental` + `grain: key` table models, it dispatches the per-partition merge loop: each partition in the window is merged into the key-grain table without discarding earlier partitions, so accumulated state from outside the requested window is preserved.

**Usage:**

```
smelt backbuild [OPTIONS] <SELECTOR> --start <DATE> --end <DATE>
```

**Arguments:**

| Argument | Required | Description |
|----------|----------|-------------|
| `<SELECTOR>` | yes | Target model selector (e.g., `+marts.daily_revenue`, `silver.events_parsed`). Bare names are subject to scope resolution; see [Argument resolution and `--scope`](#argument-resolution-and-scope). |

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--start` | | string | _(required)_ | Start of time range (ISO 8601: YYYY-MM-DD) |
| `--end` | | string | _(required)_ | End of time range (exclusive, ISO 8601: YYYY-MM-DD) |
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--database` | | path | _(from config)_ | DuckDB database file path |
| `--target` | | string | `dev` | Target environment from smelt.yml |
| `--show-results` | | bool | `false` | Display query results after execution |
| `--verbose` | `-v` | bool | `false` | Show compiled SQL for each model |
| `--dry-run` | | bool | `false` | Print the maintenance statements that would run, with per-chunk boundaries — without executing ([details](#dry-run-inspect-the-maintenance-statements-before-they-run)) |
| `--batch-size` | | integer | | Override batch size in days for backfill chunking |
| `--per-partition` | | bool | `false` | Force per-partition execution (one query per granularity period) |
| `--allow-downgrade` | | bool | `false` | Allow incremental models that fail bound derivation to fall back to full-table refresh instead of being refused at planning time. A temporary escape hatch while fixing the model SQL, not a normal-operation flag. |

**Examples:**

```bash
# Backbuild a model and all its upstreams for January (canonical path form)
smelt backbuild +marts.daily_revenue --start 2026-01-01 --end 2026-02-01

# Same using scope shorthand (equivalent when scope is marts)
smelt --scope marts backbuild +daily_revenue --start 2026-01-01 --end 2026-02-01

# Preview the maintenance statements — one block per auto-derived chunk —
# without executing anything
smelt backbuild +marts.daily_revenue --start 2026-01-01 --end 2026-02-01 --dry-run

# Backbuild with per-partition execution
smelt backbuild +marts.daily_revenue --start 2026-01-01 --end 2026-01-08 --per-partition
```

---

## smelt build

Seed the database with CSV files and then run all models. This is a convenience command that combines `smelt seed` followed by `smelt run`.

**Lifecycle.** A single `smelt build` performs these steps in order:

1. **Load `smelt.yml`** and validate the requested `--target` exists.
2. **Discover** seed CSVs, per-entity source YAMLs, SQL models, Python models, and `smelt.define` function files — all under the directories listed in `paths:`.
3. **Seed** — for each CSV, smelt's own parser reads and type-infers the file, converts the rows to typed Arrow batches, and loads them via `Backend::load_table`. Seeds are loaded sequentially in deterministic (sorted) order. Schemas are auto-created.
4. **Plan** — build the logical graph from discovered models, apply planner rules, and produce the physical execution graph. Models are executed in topological order so each model's upstreams are materialised first.
5. **Run** — for each model, materialise according to its `materialization` (`table`, `view`, `materialized_view`, or inlined for `ephemeral`). Backends that support it use `CREATE OR REPLACE TABLE` / `CREATE OR REPLACE VIEW` for atomic replacement.

**Idempotency.** `smelt build` is safe to re-run on the same database. Seeds and non-incremental models replace their existing tables/views each run; incremental models advance their interval state and append new partitions. Re-running with the same inputs converges on the same final state.

**`--show-results`.** When set, prints a small Arrow-formatted preview after each materialised model finishes (the same renderer used by DuckDB's CLI). Use it for quick correctness spot-checks during development; it is not a substitute for `smelt test`.

**`--verbose`.** For each model that the run actually executes, prints a `-- <model_name>` comment line followed by the compiled SQL string to **stdout** immediately before the backend executes it. Output is per executed model — models skipped because they are already up-to-date produce no extra `--verbose` output. The standard `smelt: built N model(s) in T s` summary line is still printed; `--verbose` adds output, it does not replace it. Pair with `--dry-run` on `smelt run` if you want to see compiled SQL without touching the database.

### `smelt build` flag truth-table

The flags below have surprised users in practice; the table records what each one actually does on `smelt build`.

| Flag | Status | Behaviour |
|------|--------|-----------|
| `--verbose` / `-v` | implemented | Prints `-- <model_name>` + the compiled SQL to stdout immediately before each executed model. No extra output when all models are up-to-date and skipped. |
| `--show-plan` | per-model only | Requires a positional argument naming a model file path (e.g. `smelt build --show-plan models/marts/customers.sql`). There is no project-wide `--show-plan` mode — a bare `smelt build --show-plan` errors. **Output format:** success prints an `ExpandedCall fn_id="<name>"` node with the inlined function body; an unresolved function call prints `error: Unknown function \`smelt.functions...\`` to stderr and exits non-zero. |
| `--select` / `-s` | repeatable | Supply each selector as its own `--select <value>`. Space-separated values inside a single `--select` are taken as one literal selector and will not match anything; use repetition. |
| `--exclude` / `-e` | repeatable | Same selector grammar and repetition rule as `--select`. |
| `--dry-run` | **not on `smelt build`** | Use `smelt run --dry-run` for parse-and-validate-without-executing. There is no project-wide compile-only flag on `build` today. |
| `--event-time-start` / `--event-time-end` | implemented | ISO-8601 (`2026-03-20` or `2026-03-20T00:00:00Z`). End is exclusive. Both required together for incremental execution. |
| `--allow-downgrade` | implemented | Allow incremental models that fail the safety classifier to fall back to full-table refresh instead of being refused at planning time. A temporary escape hatch while fixing the model SQL. |

**Usage:**

```
smelt build [OPTIONS]
```

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--database` | | path | _(from config)_ | DuckDB database file path |
| `--target` | | string | `dev` | Target environment from smelt.yml |
| `--show-results` | | bool | `false` | Display query results after execution |
| `--verbose` | `-v` | bool | `false` | Show compiled SQL for each model |
| `--event-time-start` | | string | | Start of event time range for incremental models (ISO 8601: YYYY-MM-DD). Requires `--event-time-end`. |
| `--event-time-end` | | string | | End of event time range for incremental models (exclusive, ISO 8601: YYYY-MM-DD). Requires `--event-time-start`. |
| `--select` | `-s` | string[] | | Select models to run (repeatable). Same syntax as `smelt run`. |
| `--exclude` | `-e` | string[] | | Exclude models from the run (repeatable). Same syntax as `--select`. |
| `--period` | | string | | Backward resolution: the target output period, `<start>..<end>` (ISO `YYYY-MM-DD`, end exclusive). Requires `--include-upstreams` and a positional target model. See [Backward resolution with `--include-upstreams`](#backward-resolution-with---include-upstreams). |
| `--include-upstreams` | | bool | `false` | Resolve and build the target model's required upstream slices for `--period` instead of the ordinary seed+run-everything build. Requires `--period`. |
| `--allow-downgrade` | | bool | `false` | Allow incremental models that fail the safety classifier to fall back to full-table refresh instead of being refused at planning time. A temporary escape hatch while fixing the model SQL, not a normal-operation flag. |

**Examples:**

```bash
# Seed and run everything
smelt build

# Seed and run with incremental time range
smelt build --event-time-start 2026-01-01 --event-time-end 2026-01-08

# Seed and run only selected models (canonical path form)
smelt build --select marts.daily_revenue --select marts.transactions

# Same with scope shorthand
smelt --scope marts build --select daily_revenue --select transactions

# Bounded test/validation build: resolve and build exactly what marts.daily_revenue
# needs for January
smelt build marts.daily_revenue --period 2026-01-01..2026-02-01 --include-upstreams
```

### Backward resolution with `--include-upstreams`

`smelt build <model> --period <start>..<end> --include-upstreams` answers the dual question to `--since-upstream`: given a target model and a requested output period, which upstream slices must exist for that period to be correct? It walks the target's ancestor sub-DAG backward through the SAME propagation graph `--since-upstream` assembles, applying each edge's derived scan clamp directly (`[s, e)` downstream requires `[s − before, e + after)` upstream), and resolves, for every ancestor, the partition interval that must exist — a data prerequisite for a raw source, a build region for an intermediate model — plus the build order those models must run in (ancestor-first, target last).

The resolved-slices report — one `STAGE <source>: <interval>` or `BUILD <model>: <interval>` line per ancestor, then a `Build order: ...` line — is printed before anything runs. `smelt` then builds exactly that set, ancestor models first, the target last, through the same execution path every other run/build command uses. Raw sources are reported (so you know what must already be staged) but never built by `smelt` — sources are external data.

An ancestor whose partition axis can't be sliced (an unclocked lookup/dimension source, or a model with no declared `timeseries:`) resolves to the whole table — printed as `whole table` rather than an interval — since there is no interval structure to bound it against.

This is the bounded test/validation build: staging exactly the resolved slices and building bottom-up produces the same result, over the requested period, as a full build over complete history.

```bash
# Resolve and build exactly what marts.daily_revenue needs for January,
# printing the required upstream slices and build order first.
smelt build marts.daily_revenue --period 2026-01-01..2026-02-01 --include-upstreams
```

---

## smelt seed

Load CSV seed files into the database. Seed files are CSV files placed in the directories listed under `paths:` in `smelt.yml` (default: `["models"]`).

**Usage:**

```
smelt seed [OPTIONS]
```

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--database` | | path | _(from config)_ | DuckDB database file path |
| `--target` | | string | `dev` | Target environment from smelt.yml |
| `--show-results` | | bool | `false` | Display loaded data after seeding |
| `--select` | `-s` | string[] | | Select specific seeds to load (by name or schema.name) |

**Examples:**

```bash
# Load all seed files
smelt seed

# Load specific seeds
smelt seed --select customers --select products

# Load and display the data
smelt seed --show-results
```

---

## smelt test

Run model tests and report results. Tests are `smelt.test` declarations in `.sql` files, placed in a directory listed in `paths:` (typically `tests/`).

Each test defines mock input data and expected output for a model or CTE. smelt compiles the test into a standalone SQL query, executes it against an in-memory DuckDB instance, and compares the result.

**Usage:**

```
smelt test [OPTIONS]
```

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--select` | `-s` | string[] | | Filter tests by name (repeatable, substring match) |
| `--verbose` | `-v` | bool | `false` | Show compiled SQL for each test |
| `--show-all` | | bool | `false` | Show passing tests in output (default: only failures shown) |
| `--target` | | string | `dev` | Target environment from smelt.yml — only consulted by singular tests that query real data |
| `--database` | | path | _(from smelt.yml)_ | DuckDB database file path (overrides smelt.yml) |
| `--seed` | | integer | | Random seed for property-based tests, for reproducibility |
| `--json` | | bool | `false` | Output results as JSON for editor integration. Always exits `0`, regardless of test status — a caller must inspect the JSON for pass/fail, not the exit code. |

**Output:**

Test results are printed as PASS/FAIL lines with timing. A summary line shows total counts. The command exits with code 1 if any test fails — except with `--json`, which always exits `0` so an editor or CI step can parse the JSON body regardless of pass/fail status.

```
smelt test

  PASS test_cohort_sizes (mart_cohort_retention::cohort_sizes)     0.02s
  FAIL test_user_activity (user_activity)                          0.03s

  1 passed, 1 failed, 2 total (0.05s)
```

**Examples:**

```bash
# Run all tests
smelt test

# Run tests matching "cohort"
smelt test --select cohort

# Run with compiled SQL output
smelt test --verbose

# Show all results including passes
smelt test --show-all
```

See the [Testing guide](../guide/testing.md) for how to write tests.

---

## smelt check

Run data-quality checks against built pipeline data. Checks are `smelt.check` declarations in `.sql` files, placed in a directory listed in `paths:` (typically `checks/`).

Each check is a **failing-rows query**: it returns the rows that violate an invariant. The check passes when the query returns zero rows and fails when it returns one or more. Unlike `smelt test` — which runs against mock data in an in-memory DuckDB — `smelt check` compiles each check's `smelt.<path>` references to the **real materialized relations** in the configured target and executes against the data the pipeline actually produced.

**Usage:**

```
smelt check [OPTIONS]
```

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--select` | `-s` | string[] | | Filter checks by name (repeatable, substring match) |
| `--target` | | string | `dev` | Target environment from `smelt.yml` |
| `--database` | | path | | DuckDB database file path (overrides `smelt.yml`) |
| `--verbose` | `-v` | bool | `false` | Show compiled SQL for each check |

**Output:**

Each check is reported as a `PASS`, `FAIL`, or `WARN` line. A violation includes the violating row count and a capped inline sample of the offending rows for debugging; violating rows are not persisted to the warehouse. A summary line shows total counts.

```
smelt check

  PASS  daily_revenue_non_negative
  FAIL  amount_must_exceed_500 — 3 violating row(s)
    {"order_id": "7", "amount": "120.00"}
  WARN  amount_above_threshold — 1 violating row(s)

  1 passed, 1 failed, 1 warned, 3 total
```

**Exit codes:**

- Exits `0` when every `error`-severity check passes (zero violating rows).
- Exits `1` when any `error`-severity check has violations.
- `warn`-severity checks never affect the exit code — a `severity: warn` check with violations reports `WARN` and the command still exits `0`.
- A check whose referenced model has not been built in the target fails with `CheckTargetNotBuilt` (exit `1`), never a silent pass on an absent relation.

**Examples:**

```bash
# Run all checks against the dev target
smelt check

# Run checks matching "revenue"
smelt check --select revenue

# Run against a specific target
smelt check --target prod

# Show the compiled SQL for each check
smelt check --verbose
```

See the [Testing guide](../guide/testing.md) for how to write checks.

---

## smelt list

List every entity `smelt` discovers in the project — models, seeds, sources, tests, and checks — one per line, in canonical `smelt.<path>` form, alongside its kind and, for models, its materialization. Offline: discovery and parsing only, no database connection.

**Usage:**

```
smelt list [OPTIONS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--project-dir` | path | `.` | Path to smelt project root |
| `--select` / `-s` | string (repeatable) | | Narrow the listed model set. Same selector syntax as `smelt run`/`smelt build` |
| `--exclude` / `-e` | string (repeatable) | | Exclude models from the listed set. Same syntax as `--select` |
| `--format` | string | `text` | Output format: `text` or `json` |

**Examples:**

```bash
# List everything in the project
smelt list

# Narrow to models under a tag
smelt list --select tag:staging

# Machine-readable output
smelt list --format json
```

Exits `0` on success, including an empty (selector-narrowed) result set. Exits `2` on a parse error or an unresolvable/ambiguous selector.

---

## smelt clean

Remove `target/` — the directory `smelt docs generate` and other artifact-producing commands write to. Never touches `.smelt/` state (run manifests, deployed-schema snapshots) or the configured target database.

**Usage:**

```
smelt clean [OPTIONS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--project-dir` | path | `.` | Path to smelt project root |

**Examples:**

```bash
smelt clean
```

Exits `0` whether or not `target/` existed to remove. Exits `1` if `target/` exists but cannot be removed.

---

## smelt diff

Show pending schema changes between model definitions and deployed state. Compares the inferred schema (from SQL parsing and type inference) against the last deployed schema (stored in `.smelt/schemas/`).

This command does **not** require a database connection — it works entirely offline, making it fast and suitable for CI pipelines.

**Usage:**

```
smelt diff [OPTIONS]
```

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--target` | | string | `dev` | Target environment from `smelt.yml`. Deployed schemas are recorded per target (see "State isolation per target" below), so `diff` compares against this target's recorded state. |
| `--select` | `-s` | string[] | | Select models to diff (repeatable). Same selector syntax as `smelt run`. |
| `--exclude` | `-e` | string[] | | Exclude models from diff (repeatable). Same syntax as `--select`. |
| `--json` | | bool | `false` | Output as JSON for machine consumption |

**Exit codes:**

- `0` — no schema changes detected
- `1` — schema changes detected (or new/removed models found)

**Output:**

For each model with changes, smelt shows the specific column-level changes and a risk assessment:

```
smelt diff

Model: daily_revenue
  ADD COLUMN email VARCHAR NULL
  ALTER COLUMN amount TYPE INTEGER -> BIGINT
  -> Safe: ALTER TABLE (no data loss)

Model: user_sessions
  DROP COLUMN legacy_id
  ADD COLUMN session_type VARCHAR NOT NULL
  -> Requires: --full-refresh (NOT NULL column added)

Model: new_model
  + New model (not yet deployed)

Summary: 2 changed, 1 new, 0 removed, 5 unchanged
```

Change types detected:

- **ADD COLUMN** — column exists in model SQL but not in deployed schema
- **DROP COLUMN** — column exists in deployed schema but not in model SQL
- **ALTER COLUMN TYPE** — column type changed (e.g., INTEGER → BIGINT)
- **ALTER COLUMN nullability** — column changed between NULL and NOT NULL

Risk assessment:

- **Safe: ALTER TABLE** — changes can be applied with ALTER TABLE statements (no data loss)
- **Requires: --full-refresh** — destructive changes that need a full table rebuild (e.g., adding NOT NULL column, unsafe type narrowing)
- **Requires: --allow-column-removal** — column removals detected (blocked by default for safety)

**Examples:**

```bash
# Show all schema changes
smelt diff

# Show changes for a specific model (canonical path form)
smelt diff --select marts.daily_revenue

# Show changes using scope shorthand
smelt --scope marts diff --select daily_revenue

# Show changes for all models with a tag
smelt diff --select tag:staging

# JSON output for CI
smelt diff --json

# Use in CI: fail if any schema changes pending
smelt diff --json || echo "Schema changes detected!"
```

**JSON output format:**

```json
{
  "models": [
    {
      "name": "daily_revenue",
      "status": "changed",
      "changes": [
        { "type": "add_column", "column": "email", "data_type": "VARCHAR", "nullable": true }
      ],
      "risk": {
        "requires_full_refresh": false,
        "has_column_removals": false,
        "migration_action": "alter_table",
        "statements": ["ALTER TABLE main.daily_revenue ADD COLUMN email VARCHAR"]
      }
    },
    { "name": "new_model", "status": "new" }
  ],
  "summary": { "changed": 1, "new": 1, "removed": 0, "unchanged": 5 }
}
```

---

## smelt docs generate

Generate a static data catalog from your project's model metadata. Exports model schemas, column lineage, descriptions, tags, and dependency information as browsable documentation.

**Usage:**

```
smelt docs generate [OPTIONS]
```

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--format` | | string | `markdown` | Output format: `markdown` or `json` |
| `--output` | `-o` | path | `target/docs` | Output directory |
| `--select` | `-s` | string[] | | Select models to include (repeatable). Same selector syntax as `smelt run`. |

**Output formats:**

**Markdown** (default) generates a directory with:

- `index.md` — project overview with model table and tag index
- `models/<name>.md` — per-model page with columns, lineage, and configuration

**JSON** generates a single `catalog.json` with all metadata in a structured format.

**What's included per model:**

- Name, description, owner, tags, materialization
- Columns with inferred types, nullability, descriptions, and column-level tests
- Column lineage (source tracking: from which upstream model or external table)
- Upstream and downstream dependencies
- Incremental configuration (if applicable)

**Examples:**

```bash
# Generate markdown docs
smelt docs generate

# Generate JSON catalog
smelt docs generate --format json

# Generate docs for specific models
smelt docs generate --select tag:marts

# Custom output directory
smelt docs generate --output docs/catalog
```

---

## smelt docs list

List the user-facing documentation topics shipped with this binary. Documentation is embedded at build time, so it works offline and is pinned to the installed version of smelt.

**Usage:**

```
smelt docs list
```

Each line is a topic path you can pass to `smelt docs show`.

**Example:**

```bash
smelt docs list
# concepts/how-it-works
# concepts/project-structure
# getting-started/installation
# getting-started/quickstart
# guide/incremental-models
# ...
```

---

## smelt docs show

Print the markdown contents of a documentation topic to stdout.

**Usage:**

```
smelt docs show <topic>
```

The `<topic>` argument is a path from `smelt docs list`, with or without the `.md` suffix.

**Examples:**

```bash
smelt docs show getting-started/quickstart
smelt docs show guide/incremental-models
smelt docs show reference/smelt-yml | less
```

If the topic isn't found, the error message lists near matches.

---

## smelt docs path

Explain where the embedded docs live (they are inside the binary itself, not on disk). Useful when you're scripting around the docs and want to know whether to grep through `smelt docs show` output or look for files on the filesystem.

**Usage:**

```
smelt docs path
```

---

## smelt table

Show column names and types for a model. The schema is inferred by the smelt type checker without executing the model.

**Usage:**

```
smelt table [OPTIONS] <MODEL_NAME>
```

**Arguments:**

| Argument | Required | Description |
|----------|----------|-------------|
| `<MODEL_NAME>` | yes | Name of the model to inspect |

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--project-dir` | path | `.` | Path to smelt project root |
| `--format` | string | `table` | Output format: `table` (human-readable) or `json` |

**Type caveat:** The types shown by `smelt table` are smelt's **compile-time inferred types** — they are derived from the SQL source without executing the model. They may differ from the physical column types that DuckDB stores after a `smelt build`. To see the physical DuckDB column types after building, use:

```bash
duckdb my-project.duckdb -c 'DESCRIBE <model>'
```

**Typed functions:** For columns whose values come from typed `smelt.define` calls — functions annotated with a `-> Expr<T>` return type — `smelt table` correctly reflects the declared return type. For example, a column fed by a `-> Expr<Double>` function shows as `DOUBLE` in `smelt table` output, and downstream aggregates (such as `SUM`) also use that declared type for their inferred result.

**Examples:**

```bash
# Show column types for a model (canonical path form)
smelt table marts.daily_revenue

# Using scope shorthand
smelt --scope marts table daily_revenue

# Output as JSON
smelt table marts.daily_revenue --format json

# Inspect a model in a different project
smelt table silver.users --project-dir ./my-project
```

---

## smelt type

Show the function type signature of models, displaying their input references and output columns. When called without a model name, shows signatures for all models in the project.

**Usage:**

```
smelt type [OPTIONS] [MODEL_NAME]
```

**Arguments:**

| Argument | Required | Description |
|----------|----------|-------------|
| `[MODEL_NAME]` | no | Name of a specific model to inspect (omit to show all) |

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--project-dir` | path | `.` | Path to smelt project root |

**Examples:**

```bash
# Show type signatures of all models
smelt type

# Show type signature of a specific model (canonical path form)
smelt type silver.events_parsed

# Using scope shorthand
smelt --scope silver type events_parsed

# All output uses canonical paths regardless of how you typed the name:
# silver.events_parsed:
#   (raw_events: {...})
#   -> {event_id: BIGINT, device_id: INTEGER, ...}
```

---

## smelt status

Show interval coverage and gaps for incremental models. Reports which time intervals have been materialized and identifies any gaps in coverage.

**Usage:**

```
smelt status [OPTIONS] [MODEL_NAME]
```

**Arguments:**

| Argument | Required | Description |
|----------|----------|-------------|
| `[MODEL_NAME]` | no | Specific model to show status for (omit for all incremental models) |

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--project-dir` | path | `.` | Path to smelt project root |
| `--target` | string | `dev` | Target environment from `smelt.yml`. State is partitioned per target (see "State isolation per target" below), so `status` reports coverage for this target's state only. |
| `--since` | string | | Start of query range for gap detection (ISO 8601: YYYY-MM-DD) |
| `--until` | string | | End of query range for gap detection (ISO 8601: YYYY-MM-DD, default: today) |

**Examples:**

```bash
# Show status of all incremental models
smelt status

# Show status for a specific model (canonical path form)
smelt status silver.sessions

# Using scope shorthand
smelt --scope silver status sessions

# Check for gaps in a specific time range
smelt status silver.sessions --since 2026-01-01 --until 2026-03-01
```

---

## smelt history

Show run history for the project. Displays past execution records including timestamps, durations, and which models were run.

**Usage:**

```
smelt history [OPTIONS] [MODEL_NAME]
```

**Arguments:**

| Argument | Required | Description |
|----------|----------|-------------|
| `[MODEL_NAME]` | no | Specific model to show history for (omit for all runs) |

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--target` | | string | `dev` | Target environment from `smelt.yml`. Run manifests are recorded per target (see "State isolation per target" below), so `history` reports runs for this target only. |
| `--limit` | `-l` | integer | `10` | Number of runs to show |

**Examples:**

```bash
# Show recent run history
smelt history

# Show last 20 runs
smelt history --limit 20

# Show history for a specific model (canonical path form)
smelt history silver.sessions
```

---

## smelt explain

Output model graph and configuration as JSON for orchestrator integration. Produces a machine-readable representation of the project's dependency graph, model configurations, and physical execution plan.

**Usage:**

```
smelt explain [MODEL_NAME] [OPTIONS]
```

**Arguments:**

| Argument | Type | Description |
|----------|------|-------------|
| `MODEL_NAME` | string, optional | Name of a single model to print the maintenance-plan report for instead of the whole-project graph. |

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--json` | | bool | `false` | Output as JSON (required for machine consumption). With `MODEL_NAME`, emits the per-model maintenance-plan report as JSON — with or without `--show-sql`, same schema either way. |
| `--select` | `-s` | string[] | | Select models to include (repeatable). Same selector syntax as `smelt run`. Ignored when `MODEL_NAME` is given. |
| `--show-sql` | | bool | `false` | With `MODEL_NAME`, additionally print the maintenance statements each cell executes. Never connects to a backend. |
| `--period` | | `<start>..<end>` | | With `--show-sql`, use these real literal date bounds (`YYYY-MM-DD..YYYY-MM-DD`, end exclusive) for the printed statements' region. Without it, the symbolic placeholders `{{window_start}}`/`{{window_end}}` stand in. |
| `--technique` | | string | | Requires `--show-sql`. Render a named technique's own preview statements instead of the admitted one's, for every cell — including a cell where the technique is not applicable, whose reason is printed rather than silently skipped. Accepts `delete_insert`, `keyed_fold`, `column_scoped_merge`, `in_place_update`, `per_group_recompute`, `recompute` (`recompute` and `delete_insert` both resolve to the same DELETE+INSERT / region-recompute technique). Doesn't affect `--json`, whose `technique_previews` array always carries every technique regardless of this flag. |

Without a `MODEL_NAME`, the output includes both the **logical graph** (models as written) and the **physical graph** (execution plan with ephemeral models inlined, strategies resolved). See [Two-Graph Architecture](../developing/architecture.md#two-graph-architecture) for details.

With a `MODEL_NAME`, `smelt explain` instead prints that model's **maintenance plan**, led by a
one-line **delta-signature headline**: what the model emits (`keyed upsert over [...]`,
`append-only within a window`, or `general change` naming what degraded it), how that shape is
addressed (`key-addressed`, `window-addressed by <axis>`, or `whole-table-addressed`), the
derived `grain` label, and the derived **run shape** — `window-forward` or `snapshot-reconcile`
for `grain: key`, the window sweep over the partition axis for `grain: partition`. A model whose
shape isn't derivable at all prints no headline line. Immediately after the headline, before
anything else, the report prints every admission **refusal** — `<DiagnosticCode>: <reason>`
per refusal, or `Refusals: (none)` — so you see what will be refused before the plan body.
Then: every cell (trigger, corner, technique), the `ledger_catch_up` flag (whether the cell
routes through the
[reconciliation ledger](../guide/incremental-models.md#the-reconciliation-ledger)), the derived
per-source scan clamps, each source's partition-locality verdict, a model-level **Guarantees**
ledger (one row per output column: its group, its effective equivalence contract or —
for a volatile column — its determinism exemption, and its derived settle bound, `not derived`
when no key-temporal-locality slice was established), the
model's own **Relation Contract** (its clock, identity, and derived `grain` label), and one
contract block per **inbound edge**. This only applies to incremental models (`refresh:
incremental` with a `grain:` declared) — other models print a one-line notice instead.

Each cell also prints a `contract:` row — its effective [contract relaxation](../guide/incremental-models.md#contract-relaxations)
(`default`, or `frozen_horizon`/`deferral` with their declared intervals); `--json` carries the
same information per cell as a `contract_point` object.

A `ColumnScopedMerge` cell's block additionally prints an `observed-delta recording:` line —
the only technique family recording is wired for today (`KeyedFold` and the staged-candidate
write family do not record yet, so their cells print no such line at all). The line still says
`yes` or `no`: recording only actually fires when the cell's matched arm can suppress the write
for unchanged rows, which needs a proven per-row identity (`region key:` must be `Key(...)`,
never `WholeRow`) over columns all proven comparable across runs — a cell that fails either
check falls back to an unconditional rewrite at runtime and has nothing to record, so its line
reads `no`. For a composed (key + time)
model, the `Key temporal locality:` block prints an `observed-delta projection:` line alongside
its route and settle bound: `exact (key-embedded)` / `exact (key-determined)` for locality
routes 1–2, `` widened by `r` + margins `` for route 3, since a key's stored partition can move
under that route. A bare keyed model (no established locality) prints no `Key temporal
locality:` block and no projection line — see [Observed deltas and no-op
cascades](../guide/incremental-models.md#observed-deltas-and-no-op-cascades). Both lines are
static facts about the derived plan, not about a specific past run: `smelt explain` never opens
a backend connection, so it reports what a cell's technique *would* record and how its route
*would* project.

Every cell also prints an `admissible write patterns:` line — the physical addressing patterns
(`region`, `keyed`, `column`, `update`, `full_rebuild`, and any backend-contributed pattern) the
cell's own declared facts and target backend admit — and a `write pin:` line showing the
[`maintenance.cells[].write` pin](smelt-yml.md#cellswrite--the-physical-addressing-pin), if one is
set (`(none)` otherwise). A `ColumnScopedMerge`/`KeyedFold` cell additionally prints a
`write variant:` line naming whether that cell's matched arm resolves suppressed or unconditional
and why — `preference` (the structural steady-state-vs-first-build default), `first-build posture`,
or a `technique:`/`prefer:` pin's own name — see [Steering: prefer /
technique](../guide/incremental-models.md#steering-prefer--technique).

An inbound edge is either a declared source (`sources.*`) or an upstream maintained model —
both render through the identical `clock:` / `identity:` / `derived grain:` rows, labelled
`(source)` or `(model)` so it's clear which provider filled them. A row prints `(none)` when
that provider declares neither fact — a source with no `timeseries:` and no `unique_key:` is
legal and simply has nothing to summarize, never an error.

Each edge additionally prints a `delta type:` row — the shape of change that edge's own upstream
emits: `append-only within window` (every change lands as new rows within a bounded window,
never revising an already-emitted row), `keyed upsert` (a change instead revises the row
identified by a key set), or `general` (neither addressing holds, naming the construct or
world-fact that degraded it — an unregistered operator such as a window function, a
row-multiplying join, or a source declaring no `mutation_profile`). A source edge is typed by its
own declared mutation profile; a model edge is typed by the upstream model's own derived verdict.
An edge with no derivable verdict (e.g. the upstream isn't itself `refresh: incremental`) prints
no `delta type:` row at all, rather than a fabricated one. A per-group recompute cell reading a
`keyed upsert` upstream model edge (key-addressed, no partition clamp of its own) prints the
group-grain fingerprint-sidecar diff **over the upstream's own output table** as its
affected-key discovery mechanism, alongside the clamped current-source scan and the
`mutable_snapshot` source sidecar diff.

Add `--show-sql` to also print, after each cell's block, the maintenance statements that cell
executes — the output of the same pure emitters a run executes. Each cell's SELECT body is
compiled through the same compiler a real run uses, including the real ephemeral resolver (so a
referenced ephemeral model is CTE-inlined, not shown as a bare table reference) and the real
upstream column types (so aggregates over a `smelt.ref()` column cast correctly), so the printed
SQL matches what a run would compile. Statements print in execution order; a transactional group
(e.g. a paired region `DELETE`+`INSERT`) is bracketed by `BEGIN`/`COMMIT` lines to show its
atomicity. `--show-sql` never connects to a backend or executes anything — it is a pure
compile-and-render step. Combine with `--period <start>..<end>` to see the real literal bounds a
run over that window would use; without it, the symbolic placeholders
`{{window_start}}`/`{{window_end}}` stand in so the emitted shape is inspectable without choosing
a window. Combined with `--json`, the per-model report is emitted as JSON with a `statements`
array per cell (`{"sql": "<statement>", "transactional_group": <int>}`) — the machine-liftable
form for documentation generators or other tooling.

One narrow case still diverges from a real run's *types* (not its shape): a column aggregated
directly off an ephemeral ref (e.g. `SUM(rate)` where `rate` comes straight from a joined
ephemeral model) casts to the `BIGINT` default rather than its real type — this is a compile-order
limitation in the shared compiler that a real run hits identically, so `--show-sql` still matches
what a run executes, casting quirk included. See `docs/specs/cli.md` Known Divergences.

A second, currently wider divergence: for a `ColumnScopedMerge`/`KeyedFold` cell whose write is
[conditionally suppressed](../guide/incremental-models.md#conditional-writes) at run time, `--show-sql`
always renders the unconditional matched arm (`WHEN MATCHED THEN UPDATE SET *`/the plain fold), never
the `IS DISTINCT FROM`-guarded suppressed form the live run actually executes — the report hasn't been
wired to the same suppression check yet. The cell's `region key:` row (`WholeRow` vs. a named key) is
still a reliable signal for one half of the admission rule: a `WholeRow` region key means that cell
never suppresses, regardless of what `--show-sql` prints.

Add `--technique <name>` (alongside `--show-sql`) to inspect a technique smelt *didn't* pick — every
technique smelt knows an emitter for, previewed against each cell's own contract/identity/column data,
labelled with whether that technique is actually sound here. This renders the requested technique's
own statements in place of the admitted one's, per cell, together with the cell's admissibility verdict
for it: `Admitted` (this is the technique the plan actually resolved), `InterchangeableAlternative`
(proven sound for this cell, but not the one the plan resolved — region recompute is always this when
it isn't itself admitted), or `NotApplicable` with a reason (the technique's structural preconditions
aren't met for this cell — printed, never silently skipped). `--technique` always uses the symbolic
`{{window_start}}`/`{{window_end}}` placeholders — it's a display-only illustration of a cell's shape,
not a `--period`-bound dry run. `--json` (with or without `--technique`) always carries the full
`technique_previews` array per cell — one entry per known technique, not just the admitted one — plus
a top-level `properties` object with the model's derived property set (columns, grain, functional
dependencies, per-column determinism/comparability/discriminants, row identity, source bounds).

**Examples:**

```bash
# Show the explain output
smelt explain

# Output as JSON for scripting
smelt explain --json

# Explain only selected models and their dependencies (canonical path form)
smelt explain --select +marts.daily_revenue --json

# Using scope shorthand
smelt --scope marts explain --select +daily_revenue --json

# Print one incremental model's maintenance plan
smelt explain daily_events

# Also print the maintenance statements each cell executes
smelt explain daily_events --show-sql

# ...with a real window instead of the {{window_start}}/{{window_end}} placeholders
smelt explain daily_events --show-sql --period 2024-01-01..2024-01-08

# Machine-readable statements array per cell
smelt explain daily_events --show-sql --json

# See what the keyed-fold technique would look like on a model that doesn't admit it,
# and why it's not applicable there
smelt explain daily_events --show-sql --technique keyed_fold
```

```text
$ smelt explain daily_events
emits: general change, whole-table-addressed (expression has no column reference to attribute an output-delta shape to (a constant literal, COUNT(*), or an opaque function call), forced by column group {event_count}), grain: partition, run shape: window sweep over event_date

Refusals: (none)

Maintenance plan: daily_events

Cells (2):
  - group {*} on trigger NewData { source: "raw.events" }
      corner:    RecomputeRegion
      technique: DeleteInsert
      ledger_catch_up: false
      locality:  NOT partition_local (source: raw.events, why: unclocked source is read in full on every recompute)
      scan clamps: (none)
  - group {*} on trigger Backfill
      corner:    RecomputeRegion
      technique: DeleteInsert
      ledger_catch_up: false
      locality:  NOT partition_local (source: raw.events, why: unclocked source is read in full on every recompute)
      scan clamps: (none)

Guarantees:
  - event_count (group {event_count}): default, settle: not derived

Relation contract:
  clock:    event_time_column=event_timestamp partition_column=event_date granularity=Day
  identity: (none)
  derived grain: partition

Inbound edges: sources.raw.events
  - sources.raw.events (source)
      clock:    (none)
      identity: event_id
      derived grain: key
      delta type: general (degraded by: source 'raw.events' is append_only but declares no clock/axis column)
```

---

## smelt bakeoff

Measure the wall-clock cost of every admissible maintenance technique for a model's cells against
a replayed window of real data, and optionally emit the winning technique as a ready-to-paste
frontmatter pin. This is the same [`maintenance.cells[].technique`/`prefer` override
ladder](smelt-yml.md#maintenance-configuration) a live run consults — `smelt bakeoff` measures
what that ladder would cost under each candidate, it doesn't change what a run does until you
paste the emitted pin yourself.

**Usage:**

```
smelt bakeoff <MODEL_NAME> [OPTIONS]
```

**Arguments:**

| Argument | Type | Description |
|----------|------|-------------|
| `MODEL_NAME` | string, required | The incremental model to measure. |

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--project-dir` | path | `.` | Path to smelt project root |
| `--target` | string | `dev` | The declared target to clone for scratch measurement runs. |
| `--cells <col>@<source>,...` | string[] | every cell with ≥2 admissible techniques | Narrow measurement to specific cells. Repeatable and/or comma-separated. A named cell with only one admissible technique errors — there's nothing to compare. |
| `--runs N` | u32 | `3` | Splits the driving source's event-time extent into `N` sequential windows and replays them, in order, once per candidate technique. Each replay is a real `execute_project` run against the project's own data, not a synthetic sample. |
| `--keep` | bool | `false` | Retain the scratch schemas (`smelt_bakeoff_<model>_<technique>`) and their per-target state directories after measurement instead of dropping them. |
| `--pin` | bool | `false` | Print the winning `cells[]` entry (or a full `maintenance:` block when the model has none) as YAML to stdout. Emit-only — never writes the model's `.sql` file. |

Every cell under measurement runs against a disposable **scratch target**: the chosen `--target`
is cloned in memory under a synthetic name with schema `smelt_bakeoff_<model>_<technique>` — no
runtime schema seam is needed, since schema already flows from `config.targets[target].schema` —
and the scratch schema (plus its state directory) is dropped after measurement unless `--keep`.
The real target and its state are never touched. After each replayed window, every pair of
measured techniques' output is cross-checked with `EXCEPT ALL` in both directions; a mismatch
fails the whole run loudly rather than reporting a cost for a technique whose output diverged.

A model with no cell that has 2+ admissible techniques prints a "nothing to measure" report and
exits `0` — there is nothing to bake off.

**Examples:**

```bash
# Measure every multi-technique cell of daily_events_enriched over 3 replayed windows
smelt bakeoff daily_events_enriched

# Narrow to one cell, replay 5 windows, keep the scratch schemas for inspection
smelt bakeoff daily_events_enriched --cells user_name@users --runs 5 --keep

# Measure and print the winning technique as a ready-to-paste frontmatter pin
smelt bakeoff daily_events_enriched --pin
```

```text
$ smelt bakeoff daily_events_enriched --runs 2 --pin
smelt bakeoff report for `daily_events_enriched` (target=dev, runs=2)

cell: columns=["user_name"] on=users trigger=UpstreamMutation
  - fold             total=   842ms per-run=[421, 421] rows=100000 schema=smelt_bakeoff_daily_events_enriched_fold
  - recompute        total=  1930ms per-run=[965, 965] rows=100000 schema=smelt_bakeoff_daily_events_enriched_recompute
  equivalence: OK (EXCEPT ALL empty both directions)

to pin this choice, add to `daily_events_enriched.sql` frontmatter:
maintenance:
  cells:
    - columns: [user_name]
      on: users
      technique: fold
```

The report lists, per measured cell, every admissible technique's total and per-window
wall-clock cost, the resulting row count, and the scratch schema it ran in; the `equivalence:`
line confirms every pair of measured variants agreed exactly. With `--pin`, the winning
technique — lowest total wall-clock across the replayed windows — is printed as YAML you can
paste directly into the model's `maintenance:` block; see [`cells[].technique` /
`prefer`](smelt-yml.md#maintenance-configuration) for what pinning it then does at execution. A
tie keeps the model's current default choice and says so in the report rather than picking
arbitrarily.

---

## smelt ui

Start a local web UI for visualizing the model graph and project structure.

**Usage:**

```
smelt ui [OPTIONS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--project-dir` | path | `.` | Path to smelt project root |
| `--port` | integer | `3000` | Port to serve the UI on |
| `--host` | string | `127.0.0.1` | Host address to bind to. Non-loopback addresses require `--allow-remote`. |
| `--allow-remote` | flag | off | Required to bind `--host` to a non-loopback address |

**Network exposure:** `smelt ui` has no authentication or HTTPS. It is meant to be reached at `http://127.0.0.1:<port>` from the machine running it. Binding to a non-loopback host without `--allow-remote` fails loudly rather than silently binding to loopback instead; with the flag, the server starts and logs a startup warning that it is reachable from other hosts. CORS is restricted to the server's own origin.

**Examples:**

```bash
# Start the web UI on default port
smelt ui

# Start on a custom port
smelt ui --port 8080

# Bind to all interfaces (for remote access) — requires the opt-in
smelt ui --host 0.0.0.0 --port 3000 --allow-remote
```
