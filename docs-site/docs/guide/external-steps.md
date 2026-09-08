# External Steps

Some relations aren't computed by a smelt model — they're landed by a program you run
outside SQL entirely: a loader script pulling from an API, a batch export from another
system, a one-off ETL job. That program's output is still a table your models depend on,
and smelt needs to know when it has to run *before* those models can.

An **external step** declares that dependency explicitly. It's a node in smelt's DAG —
discovered, ordered, and invoked like any other project entity — but smelt never authors
or inspects what it runs. The declaration is the entire contract: what sources the step
produces, how to invoke it, and how often it's expected to run.

## Declaring a step

A step is a `.yml` file, placed anywhere under `paths:` like any other project file,
carrying a top-level `external_step:` block:

```yaml
# models/sources/raw/github_loader.yml
external_step:
  description: >
    Loads the previous two UTC days of githubarchive.day events into
    raw.github_events and its one-day redelivery arm.
  produces:
    - smelt.sources.raw.github_events
    - smelt.sources.raw.github_events_arrival
  command: ["bash", "scripts/bq-dogfood-loader.sh", "--date", "{run_date}"]
  cadence: '1 day'
```

The presence of `external_step:` at the top level makes this file a **step**, never a
source — it declares no `columns:`. The sources it produces are declared exactly as any
other source, in their own `.yml` files with their own `columns:`, `timeseries:`, and
mutation profile; having a step behind them changes nothing about their shape.

| Key | Required | Meaning |
|-----|----------|---------|
| `produces` | yes | Non-empty list of source addresses (`smelt.<path>` form) this step populates. Every entry must resolve to a declared source, and a source may be named by at most one step in the workspace. |
| `command` | yes | Argv list smelt invokes as the step's program. Opaque to smelt — never parsed or type-checked, just run and observed by its exit code. |
| `cadence` | no | How often the producer intends to run, e.g. `'1 day'`. Describes the *producer's* schedule, distinct from a source's own `mutation_profile.lateness`, which describes how far behind the clock a landed row can be. |
| `description` | no | Free-text description, surfaced in LSP hover and `smelt explain`. |

## The `{run_date}` / `{run_end}` placeholders

`command:`'s argv is opaque to smelt except for one closed substitution grammar, applied
per-argv-element before spawning:

- `{run_date}` — the run window's start, ISO `YYYY-MM-DD`.
- `{run_end}` — the run window's exclusive end, same form.
- `{{` and `}}` escape to literal `{` and `}`.

Any other `{name}` in an argv element is rejected at declaration time
(`MalformedExternalStep`) — caught by the LSP and by `smelt list`/`smelt run`'s pre-flight
parse, never deferred to the run that first reaches the step.

## What smelt guarantees

- **DAG ordering.** A run that selects a model downstream of a step's produced sources
  runs the step first, ahead of every consumer.
- **Invocation.** When the run reaches the step, smelt spawns `command:` with the
  placeholders substituted for the run's actual window.
- **Failure propagation.** A non-zero exit is a run failure, naming the step and its exit
  code; every model downstream of the sources it produces is left unbuilt.
- **Reporting.** The run report and manifest record every step a run invoked
  (`command`, `produces`, and duration); `smelt explain` renders a step's declaration.
- **Selection.** `smelt list`, the DAG/graph surfaces, and model selection
  (`+model`-style upstream expansion, direct addressing) reach a step through the same
  selectors as any other node.

## What smelt does not do

- **Authorship.** smelt never writes, generates, or reviews the external program. The
  loader script is yours to write and maintain — see
  `examples/github_activity/load_day.sh` for a worked example that carries its own
  bookkeeping.
- **Parsing or type-checking the command.** `command:`'s argv is an opaque string list;
  smelt does not parse it, does not type-check it, and does not know what the program
  inside it does beyond its exit code.
- **Retries.** A failed step is a run failure like any other — smelt applies no retry
  policy beyond whatever the run's own invocation policy already provides. The step
  itself owns any internal retry logic it needs.
- **Idempotence.** smelt does not guarantee, and does not verify, that running the step
  twice for the same window produces the same result. If the step needs to be safe to
  re-run (for example, after a partial failure), that safety is the step's own
  responsibility to build in.

## Failure modes

| Situation | What happens |
|---|---|
| `external_step:` is malformed — `produces:` missing/empty, an entry that doesn't resolve to a declared source, `columns:` present alongside `external_step:`, a malformed `command:`, an unrecognised `{name}` placeholder, or a malformed `cadence:` | `MalformedExternalStep` — caught at declaration time by the LSP and by `smelt list`/`smelt run`'s pre-flight parse, before any run starts. |
| A run reaches a step but cannot invoke it — no `command:` resolvable in the environment, a dry run, or an environment that categorically cannot execute external commands | `ExternalStepNotInvocable` — the run refuses rather than proceeding against a possibly-stale table. |
| The step's `command:` exits non-zero | `ExternalStepFailed` — the run fails, naming the step and its exit code; every model downstream of its produced sources is left unbuilt. |

`smelt explain <step>` is the one non-refusing preview surface: it prints the step's
declared `produces:`, its literal (unsubstituted) `command:` argv, its cadence and
description, and the models that directly consume its produced sources — without ever
spawning the command.

## When to use a step vs. an out-of-band pipeline

Reach for an external step when the loader is something a run needs to have happened
before it can trust the sources it reads — smelt should order it, fail the run loudly if
it fails, and show it in `smelt list`/`smelt explain`. If the loading pipeline runs on its
own schedule, independent of any smelt run, and the source's freshness is just something
models tolerate (via `mutation_profile`/`source_lateness`), a plain source declaration
with no step behind it is the better fit — you gain nothing from smelt invoking a program
whose completion your run doesn't actually need to wait on.

## Further reading

- [Source YAML Reference](../reference/sources-yml.md#the-external_step-block) for the
  full `external_step:` key table
- [Sources](sources.md) for how the sources a step produces are declared
