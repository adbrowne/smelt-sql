---
feature: cli
status: experimental
last_reviewed: 2026-05-05
owners: [andrew]
---

# CLI

> **What this is.** A normative spec for the `smelt` command-line interface — exit codes, command semantics, the `smelt explain --json` schema, and the `smelt build` lifecycle. Flag enumerations are in `docs-site/docs/reference/cli.md`; this spec covers the **behavior** those flags control.

## Surface

### Commands

| Command | Purpose |
|---------|---------|
| `smelt run` | Execute models in topological order |
| `smelt build` | Seed then run (convenience wrapper) |
| `smelt backbuild` | Rebuild a model and its upstreams over a time range |
| `smelt seed` | Load CSV seeds into the target database |
| `smelt test` | Run unit tests against in-memory DuckDB |
| `smelt diff` | Report pending schema changes (offline) |
| `smelt table <model>` | Show inferred column schema for a model (offline) |
| `smelt type [model]` | Show model function signature (offline) |
| `smelt status [model]` | Show incremental interval coverage and gaps |
| `smelt history [model]` | Show past run records |
| `smelt explain` | Output model graph as JSON for orchestrators |
| `smelt ui` | Start a local web UI for the model graph |
| `smelt docs generate` | Generate a data catalog (markdown or JSON) |
| `smelt docs list` | List embedded documentation topics |
| `smelt docs show <topic>` | Print embedded documentation topic to stdout |
| `smelt docs path` | **Stub.** Prints a message indicating docs are embedded in the binary and suggests using `smelt docs list` / `smelt docs show` instead. Does not print a usable filesystem path — there is none; docs are compiled into the binary. Future feature. |

### Top-level flags

| Flag | Description |
|------|-------------|
| `--help`, `-h` | Print usage and exit 0. Per-subcommand `--help` prints that subcommand's flag table. |
| `--version`, `-V` | Print the package version (`CARGO_PKG_VERSION`) and exit 0. |

`--help` and `--version` succeed in any directory — they do not require `smelt.yml` to be present.

### Common flags (all commands)

| Flag | Default | Description |
|------|---------|-------------|
| `--project-dir` | `.` | Root directory containing `smelt.yml` |
| `--target` | `dev` | Named target from `smelt.yml` |
| `--database` | from config | Override DuckDB database file path (DuckDB targets only) |

### Exit codes

| Code | Meaning |
|------|---------|
| `0` | Success — all selected models built/tested/seeded without error |
| `1` | Execution failure — at least one model failed, test failed, or schema diff detected changes |
| Non-zero | Configuration error, missing smelt.yml, selector parse error, YAML parse error |

**`smelt diff` specifics:** exits `0` if no schema changes are detected; exits `1` if any changes are found (including new or removed models). This makes it suitable as a CI gate.

**`smelt test` specifics:** exits `0` if all tests pass; exits `1` if any test fails.

### `smelt build` flags

| Flag | Description |
|------|-------------|
| `--verbose` / `-v` | Log compiled SQL for each model before execution. Only executed models produce output; skipped models produce none. |
| `--show-plan` | Requires a positional model-file path argument (e.g. `smelt build --show-plan models/marts/customers.sql`). Without the positional argument the command errors. There is no project-wide `--show-plan` mode. |
| `--select` / `-s` | Repeatable. Each `--select <value>` is one selector. Space-separated values inside a single `--select` are treated as one literal selector (not parsed as multiple). |
| `--exclude` / `-e` | Repeatable; same selector grammar as `--select`. |
| `--event-time-start` / `--event-time-end` | ISO-8601 date or timestamp. End is exclusive. Both required together for incremental execution. |

`smelt build` also accepts the schema-evolution flags `--allow-column-removal` and `--allow-full-refresh`; see `schema_evolution.md` §"Evolution flags" for semantics. The same flags are accepted by `smelt run` (which delegates to the same evolution-handling path).

`--dry-run` does **not** exist on `smelt build`. Use `smelt run --dry-run` to parse and validate without executing.

**`smelt explain` excludes test models.** `smelt explain` (with or without `--json`) filters out all `materialization: test` models from its output via the `is_test()` predicate applied to every discovered model. Test models never appear in `models`, `execution_order`, or the physical plan section. This filtering is not flag-controlled; it is always active.

### `smelt explain --json` output schema

```json
{
  "models": {
    "<model_name>": {
      "dependencies": ["<upstream_model_name>", ...],
      "materialization": "table" | "view" | "ephemeral" | "materialized_view" | "test",
      "incremental": {                      // omitted if not incremental
        "granularity": "day" | "hour" | ...,
        "partition_column": "<col>",
        "event_time_column": "<col>",
        "unique_key": ["<col>", ...],       // omitted if empty
        "batch_safety": "FullyBatchSafe" | "BoundedSafe" | "PerPartitionOnly"
      },
      "tags": ["<tag>", ...],               // omitted if empty
      "owner": "<string>"                   // omitted if unset
    }
  },
  "execution_order": ["<model_name>", ...],
  "physical": {                             // omitted unless --show-physical or similar
    "execution_order": ["<model_name>", ...],
    "nodes": {
      "<model_name>": {
        "strategy": "<string>",
        "materialization": "<string>",
        "target": "<target_name>",
        "logical_origins": ["<model_name>", ...]  // omitted if single-origin
      }
    },
    "ephemerals": ["<model_name>", ...],
    "transformations": ["<string>", ...]    // omitted if empty
  }
}
```

- `models` keys are in alphabetical (BTreeMap) order.
- `dependencies` lists only direct upstream dependencies, not transitive.
- `execution_order` is a valid topological sort of all included models.

## Semantics

### `smelt build` lifecycle

A single `smelt build` performs these steps, in order:

1. **Load** `smelt.yml` from `--project-dir`. Fail if absent.
2. **Validate** that the requested `--target` exists in the config.
3. **Discover** all project files under `paths:` and the dedicated scan paths (functions, sources, tests). The resolver classifies each file by format and content per `architecture.md` §"Resolution": `.sql` files become models, `smelt.define`s, or tests; `.csv` files become seeds; per-entity `.yml` files (a `users.yml` next to a `users.csv`, or alone in `sources/`) become seed sidecars or sources respectively.
4. **Seed** — for each CSV file (in deterministic sorted order):
   - Drop any existing table or view with the same qualified name.
   - `CREATE TABLE <schema>.<name> AS SELECT * FROM read_csv_auto('<path>')`.
   - Schemas are auto-created if absent.
5. **Plan** — build the logical dependency graph; apply planner rules; produce the physical execution graph. Models execute in topological order.
6. **Run** — for each model in topological order, materialize according to its effective materialization:
   - `table` / `materialized_view`: `CREATE OR REPLACE TABLE` (atomic replacement)
   - `view`: `CREATE OR REPLACE VIEW`
   - `ephemeral`: inlined as CTE — no DDL emitted

`smelt build` is idempotent. Re-running with the same inputs and the same time range produces the same final database state.

### `smelt run` vs `smelt backbuild`

`smelt run` executes the selected models for the requested time range. Incremental models receive a DELETE+INSERT for the given `[start, end)` window.

`smelt backbuild` additionally traverses upstream of the selector target(s) and rebuilds the full dependency chain. It uses the model's batch-safety classification to determine whether the range can be processed in a single query or must be split into per-partition or batched chunks.

### `--verbose` behavior

1. For each model that executes, emit the compiled SQL string to stdout immediately before execution. The emission is prefixed with `-- <model_name>`.
2. Only executed models produce `--verbose` output. Models skipped because they are already up-to-date produce no output.
3. The standard summary line (`smelt: built N model(s) in T s`) is unaffected — `--verbose` adds output, it does not replace it.

Pair `smelt run --verbose --dry-run` to see compiled SQL without executing.

### `smelt diff` — offline operation

`smelt diff` does **not** require a live database connection. It compares:
- **Inferred schema**: derived from SQL parsing and type inference on the current model files
- **Deployed schema**: stored in `.smelt/schemas/` (written by previous `smelt run`/`smelt build` invocations)

If `.smelt/schemas/` does not exist, all models appear as "new".

### `smelt table` and `smelt type` — offline operation

Both commands derive output from SQL parsing and type inference only. No database connection is required. Results match what the LSP and `smelt diff` see; they may differ from the live database schema if the last `smelt run` used a different model version.

### `smelt test` isolation

`smelt test` compiles each test into a standalone SQL query and executes it against an **in-memory DuckDB instance** using mock data declared in the test's frontmatter. No connection to the project's target database is made. Tests always run on DuckDB regardless of the project's configured target(s).

### `smelt docs generate` output

With `--format markdown` (default):
- Output directory: `target/docs/` (overridable with `--output`)
- `target/docs/index.md` — project overview table with all models, tags, and owners
- `target/docs/models/<model_name>.md` — per-model page with columns, lineage, configuration, and descriptions

With `--format json`:
- `target/docs/catalog.json` — all model metadata as a single JSON object

### `smelt docs list` / `smelt docs show`

Documentation is embedded in the binary at build time. `smelt docs list` enumerates available topic paths. `smelt docs show <topic>` prints the markdown for a topic. Topics match the relative paths under `docs-site/docs/` with or without the `.md` suffix.

## Design

**`smelt build` = seed + run.** Combining seeding and model execution into one command is the most common development workflow. Keeping them as separate commands (`smelt seed`, `smelt run`) allows targeted re-seeding or model re-runs without the full lifecycle.

**`smelt diff` is offline.** Schema change detection does not require a live database connection. This enables CI checks that run without database credentials and without executing models. Deployed schemas are stored in `.smelt/schemas/` as part of the project's state.

**`smelt explain --json` for orchestrators.** The JSON output is the integration contract for Dagster, Airflow, and other orchestrators. It must be stable — field additions are allowed, field removals are not. The physical graph is included to allow orchestrators to understand cross-engine topology.

**`smelt test` always in-memory on DuckDB.** Tests do not execute against the project's production target. This guarantees tests are fast, reproducible, and require no external database. The trade-off is that tests on Spark-only projects may miss Spark-specific behavior.

**`--show-plan` is per-model in v1.** Whole-project planning is a different operation (it produces a graph view, not a single-model plan) and the right output format has not been chosen. Keeping `--show-plan` per-model leaves the design space open while serving the most common need.

**`--verbose` logs per executed model, not per discovered model.** Logging every discovered model would flood the output on incremental runs where most models are skipped. Emission scaling with work done matches the user's mental model.

## Constraints & Invariants

1. **`smelt build` is idempotent.** Re-running on the same inputs produces the same final state.
2. **`--help` and `--version` require no project.** They succeed in any directory.
3. **`smelt diff` requires no live connection.** It must work in offline environments (CI without DB credentials).
4. **`smelt test` runs on in-memory DuckDB.** Tests never touch the project's configured target database.
5. **`smelt explain --json` schema is append-stable.** Fields may be added; existing fields must not be renamed or removed without a major version bump.
6. **Exit codes are meaningful.** `0` = success; `1` = detected failure or change; non-zero = error. Scripts should check exit codes, not stdout patterns.
7. **`--dry-run` does not exist on `smelt build`.** It exists on `smelt run` and `smelt backbuild` only.
8. **`--show-plan` requires a positional model-file argument.** Absence is a hard error, not a fallback to project-wide mode.
9. **Multi-value flags are repetition-based.** `--select`, `--exclude`, and similar flags must not silently split internal whitespace into multiple values.

## Known Divergences / Open Questions

- **Exit code standardization incomplete.** Configuration errors, YAML parse failures, and selector parse errors exit with non-zero codes but the exact code is not consistently `2` or any defined value distinct from `1`. Exit code meaning for "user/config error" is not defined.
- **`smelt test --select` is substring match, not selector syntax.** Unlike all other commands, `smelt test`'s `--select` does a simple substring match on test file names rather than using the selector parser. See `model_selection.md` Known Divergences.
- **`smelt explain` physical section gating.** The `physical` section of the explain output is documented as present, but the condition that triggers its inclusion (`--show-physical` flag?) is not clearly surfaced in the CLI help or user guide.
- **`.smelt/schemas/` not documented.** The schema state directory that `smelt diff` reads from is not documented in user-facing docs. Its format, update timing, and lifecycle are not specified.
- **`smelt status` reads from live DB.** Gap detection requires a database connection; this is not documented clearly in the command help.
- **No project-wide compile-only flag (TB-3).** `smelt build --dry-run` does not exist; `smelt build --show-plan` requires a positional model-file argument. There is no single command to compile every model and show the plan without executing. Two candidate resolutions: (1) extend `--show-plan` to accept no positional argument for project-wide output, or (2) add `smelt build --dry-run` mirroring `smelt run --dry-run` semantics across the seed→run lifecycle.
- **`--select` whitespace handling is unspecified.** `--select "a b"` produces a single literal selector `"a b"` that silently matches nothing. Whether this should be an error or a warning is open; current behavior is silent.

## References

- **Code**:
  - `crates/smelt-cli/src/main.rs` — command routing, `--target` defaults
  - `crates/smelt-cli/src/explain.rs` — `ExplainOutput`, `ExplainModel`, `ExplainIncremental`, `ExplainPhysical`
  - `crates/smelt-cli/src/commands/` — per-command implementation
  - `crates/smelt-cli/src/commands/build.rs` — `--show-plan` dispatch
  - `crates/smelt-cli/src/logical_graph.rs` — `LogicalGraph::build()`
- **User docs**:
  - `docs-site/docs/reference/cli.md` — full flag reference
  - `docs-site/docs/guide/model-selection.md` — selector syntax
- **Plans (history)**:
  - `docs/plans/20260502-smelt-loop-findings.md` — TB-1 and TB-4 fixes, TB-3 deferred
- **Related specs**:
  - `architecture.md` — pipeline stages the CLI orchestrates.
  - `model_selection.md` — `--select` / `--exclude` semantics
  - `models.md` — materialization modes
  - `incremental_models.md` — `--event-time-start` / `--event-time-end` semantics, batch safety classification, `backbuild` behaviour.
  - `functions.md` — `smelt build` plans function expansion as part of the build lifecycle.
  - `schema_evolution.md` — `smelt diff` change classification
  - `testing.md` — `smelt test` execution
  - `smelt_yml.md` — `targets:` and `paths:` keys consumed by the CLI.
