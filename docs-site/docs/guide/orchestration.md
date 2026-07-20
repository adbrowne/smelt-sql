# Orchestration

This page covers running smelt under a scheduler -- cron, Airflow, or anything that shells out and checks an exit code: how to interpret exit codes, how retries and `--resume` interact, what happens when a run is interrupted partway through, where to find the run report, and how to capture logs. See [Production Deployment](deployment.md) for installing smelt and configuring targets/secrets first.

## Exit codes

Every `smelt` subcommand exits one of three codes, and an orchestrator should branch on them rather than parsing stdout:

| Code | Meaning |
|------|---------|
| `0` | Success. Includes a `warn`-severity `smelt check` violation and an empty-but-valid selection -- a build that ran nothing because there was nothing to do is not a failure. |
| `1` | Detected failure. A failed model build, a failed `smelt test` case, an `error`-severity `smelt check` violation, `smelt diff` detecting a schema change, or a check referencing a model not built in the target. |
| `2` | Usage error. Malformed CLI arguments, a malformed or missing `smelt.yml`, or an unresolvable project/target. |

The distinction between `1` and `2` matters for retry logic: `1` means the command ran correctly and *found* a problem in the data or models -- worth investigating, and often worth retrying if the cause is transient (a flaky upstream source, a backend hiccup). `2` means the command couldn't run at all because its own inputs were invalid -- retrying a `2` without changing the invocation (fixing a typo'd flag, a missing target, a broken `smelt.yml`) is never useful. See the [CLI reference](../reference/cli.md#exit-codes) for the full contract, including per-command specifics (`smelt diff`, `smelt test`, `smelt check`).

## Scheduling with cron

A cron entry runs `smelt build` (or `smelt run`) against a named target on a fixed schedule:

```cron
0 * * * *  cd /srv/my-project && smelt build --target prod >> /var/log/smelt/build.log 2>&1
```

Because every state-mutating run acquires an exclusive advisory lock on `.smelt/lock` for its duration (see [Production Deployment](deployment.md#locking)), an overlapping second invocation -- the previous hour's run still in flight when the next one fires -- fails loudly rather than interleaving writes with the first, naming the PID that holds the lock. This means you do not need an external lockfile (`flock`) around the cron entry for correctness; a run that finds the lock held exits non-zero and the next scheduled tick tries again. If overlap is expected to be common (a run that occasionally exceeds its interval), wrap the cron entry in `flock -n` to skip cleanly instead of paying for a failed invocation:

```cron
0 * * * *  flock -n /srv/my-project/.smelt/lock.cron smelt build --target prod --project-dir /srv/my-project
```

## Scheduling with Airflow

A `BashOperator` (or any operator that shells out) maps naturally onto smelt's exit-code contract -- Airflow's own task-retry mechanism handles a `1`, while a `2` should not be retried blindly since it indicates a broken invocation rather than a transient failure:

```python
from airflow.operators.bash import BashOperator

smelt_build = BashOperator(
    task_id="smelt_build",
    bash_command="cd /srv/my-project && smelt build --target prod",
    retries=2,
    retry_exponential_backoff=True,
)
```

A retried Airflow task should pass `--resume` on retry attempts after the first, so a re-run does not redo work that already succeeded:

```python
smelt_build = BashOperator(
    task_id="smelt_build",
    bash_command=(
        "cd /srv/my-project && "
        "smelt build --target prod "
        "$(test \"$AIRFLOW_CTX_TRY_NUMBER\" -gt 1 && echo --resume)"
    ),
    retries=2,
)
```

## Bounded automatic retry vs. `--resume`

smelt retries a **transient backend failure within a single model's execution** automatically and internally -- a bounded number of attempts with backoff, applied only to errors the backend classifies as transient (a dropped connection, a lock-contention error), never to a deterministic failure like a type mismatch or a constraint violation. This is not configurable via a CLI flag; it happens inside one model's statement-group execution and is invisible to the orchestrator except as a longer wall-clock time and a non-zero `retry_count` recorded against that model in the run report.

`--resume` is a different mechanism, invoked explicitly, that operates **across an entire run** rather than within one model:

```bash
smelt run --target prod --resume
```

`--resume` re-runs a previously-interrupted or partially-failed selection while skipping models that don't need to run again. A model is skipped only when both hold: it succeeded in the most recent incomplete run, and its compiled definition hasn't changed since. A model that failed, was skipped, or whose SQL has changed always re-runs -- and so does every downstream dependent of any such model, since a dependent's prior success said nothing about inputs that have since been rebuilt out from under it. This makes `--resume` a pure optimization over a full re-run, never a correctness trade-off: the worst case (a hash mismatch) is always "run it again," never "skip it incorrectly."

`--resume` refuses with a hard error -- not a silent full run -- when there's nothing to resume from: the most recent run for the target already completed successfully, or no run manifest exists at all. This is deliberate: a stale or typo'd `--resume` invocation must never be mistaken for "nothing needed doing." An orchestrator that always passes `--resume` on retry attempts needs its retry step to tolerate this refusal on the first attempt (when there is no prior incomplete run) by falling back to a plain run, as in the Airflow example above.

## Partial-failure walkthrough

A run executes selected models in topologically-ordered waves; models within one wave may run concurrently. When a model's execution errors, smelt does not abort the instant the first error is observed -- every model already dispatched in that wave finishes, and each failure gets recorded independently. Only once the wave has fully drained does the run stop dispatching further waves. Concretely, for a run selecting `[a, b, c, d]` where `c` depends on `a` and `d` depends on `b`, and `b` fails:

- `a` -- in the same wave as `b`, already dispatched -- finishes normally: `success`.
- `b` -- fails: `failed`.
- `c` -- depends only on `a`, which succeeded, so it runs in the next wave: `success`.
- `d` -- depends on `b`, which failed -- never dispatched: `skipped`.

Nothing downstream of a failure silently appears to succeed, and nothing independent of a failure is held back waiting for it. Re-running with `--resume` after fixing whatever caused `b` to fail skips `a` and `c` (both `success`, unchanged definitions) and re-runs `b` and `d`.

## Run report

Every run writes a run report to `.smelt/targets/<target>/reports/<run_id>.json`, alongside the run manifest, sharing its `run_id`. The report is the human/tooling-facing summary: outcome counts, total duration, and per-model error messages for anything that failed.

```jsonc
{
  "run_id": "20260604-141233-a1b2c3",
  "started_at": "2026-06-04T14:12:33Z",
  "completed_at": "2026-06-04T14:12:41Z",
  "duration_ms": 8123,
  "outcome_counts": { "success": 5, "failed": 2, "skipped": 1 },
  "failures": [
    { "model": "bad_a", "error": "Conversion Error: ...", "retry_count": 0 },
    { "model": "bad_b", "error": "Conversion Error: ...", "retry_count": 1 }
  ]
}
```

A report is written at every point a manifest is persisted -- successful completion, cancellation, and abort -- so a partial report (`completed_at: null`) is available immediately after a failed or cancelled run, not only after a subsequent successful one. Point log shipping or an observability pipeline at `.smelt/targets/<target>/reports/` to pick up each run's outcome without parsing stdout; `retry_count` on a `failures` entry tells you whether a failure exhausted the bounded automatic retry described above or failed outright on the first attempt.

## `smelt explain` for DAG introspection

`smelt explain --json` prints the whole-project dependency graph as JSON -- model names, their upstream/downstream edges, and configuration -- for an orchestrator to consume when it needs to generate its own task graph from a smelt project rather than shelling out to a single `smelt build`:

```bash
smelt explain --json --project-dir /srv/my-project > graph.json
```

Pass a model name to instead print that one model's maintenance-plan report (its materialization cells, clamps, and locality verdicts); this is a debugging tool for a single model, not something an orchestrator typically parses. See the [CLI reference](../reference/cli.md) for the full flag table.

## Log capture

By default smelt logs human-readable text. Pass `--log-format json` (a global flag, valid on every subcommand) to instead emit one parseable JSON object per line, suited to a log aggregator:

```bash
smelt build --target prod --log-format json >> /var/log/smelt/build.jsonl 2>&1
```

Log verbosity is controlled the same way as any `tracing`-based Rust binary: the standard `RUST_LOG` environment variable.

```bash
RUST_LOG=debug smelt build --target prod
RUST_LOG=smelt_runtime=debug,smelt_backend_duckdb=info smelt build --target prod
```

Without `RUST_LOG` set, smelt logs at a sensible default level -- enough to see per-model progress without debug-level backend chatter. Set it per-invocation (as above) rather than exporting it globally in a scheduler's environment, so a one-off debugging run doesn't silently change the verbosity of every other scheduled job sharing that environment.
