---
feature: cli
status: experimental
last_reviewed: 2026-05-27
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
| `--scope` | cwd-derived | Dot-path prefix used to expand bare argument names into full `smelt.<path>` addresses. See §"Argument resolution and `--scope`". `--scope ''` disables auto-scoping. |

### Argument resolution and `--scope`

Every CLI command that takes an entity identifier (a model, function, seed, source, or test) accepts a **dot-path argument** matching the universal addressing scheme defined in `architecture.md` §"Resolution: `smelt.<path>` is the universal addressing scheme". The argument never carries the leading `smelt.` prefix — that prefix is implicit at the CLI surface — but the remainder is the same path tuple the resolver matches against.

Three input shapes are accepted:

| Shape | Example | Resolution |
|-------|---------|------------|
| **Full path** | `silver.events_parsed` | Resolved as-is against the workspace. Always works. |
| **Scoped shorthand** | `events_parsed` (with scope `silver`) | Expanded to `<scope>.<arg>` (`silver.events_parsed`) and resolved. Falls back to the bare argument if the scoped expansion does not exist. |
| **No-scope bare leaf** | `events_parsed` (no scope set) | Resolved as a full path. Errors if no entity at that exact path exists, even if a same-named entity exists in a sub-namespace. |

**Scope sources, in precedence order:**

1. **`--scope <prefix>` flag** (explicit). The flag value is a dot-path (e.g. `silver`, `marts.daily`); whitespace and the literal `smelt.` prefix are rejected.
2. **Working-directory derivation** (auto). If the process's current working directory is under `<project>/<scan_root>/<segs...>` for some scan root in `paths:`, the auto-scope is `<segs joined by .>`. If `cwd` is the project root, above the project, or inside a `scan_root` itself, no auto-scope applies.
3. **No scope.** The argument must be a full path; bare leaves resolve only against entities whose full path is exactly the leaf.

`--scope ''` (empty string) forces "no scope" regardless of cwd. This is the explicit opt-out for scripts that want to be cwd-insensitive.

**Disambiguation rules:**

- **Scope expansion fall-through** is silent: if `<scope>.<arg>` resolves, use it; otherwise try `<arg>`. The fall-through is one-shot — there is no recursive search up the scope hierarchy.
- **Ambiguity at no-scope resolution.** When the user passes a bare leaf (e.g. `events_parsed`) with no scope and the leaf matches multiple entities (`silver.events_parsed`, `bronze.events_parsed`), the command exits non-zero with a diagnostic listing all matches and a hint to use `--scope` or the full path. The CLI does not silently pick one.
- **Cross-scope full paths.** A full-path argument (`bronze.raw_events`) is honored regardless of the active scope. Scope narrows input, never output or cross-references.

**Canonical-display rule.** Every CLI command's output uses the full canonical `smelt.<path>` form for every entity it names — model lists, type signatures, diagnostics, JSON output keys, log lines. Scope changes how the user *types* an identifier; it never changes how the CLI *prints* one. Copy-pasting any printed identifier back into a `--select`, into a model `FROM` clause, or into another command must produce the same resolution.

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

`smelt run` and `smelt build` also accept `--allow-downgrade` (see below).

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
      "owner": "<string>",                  // omitted if unset
      "origin": {                           // omitted when the model is hand-authored
        "type": "generated",
        "generator_file": "<workspace-relative path>",
        "generator_name": "<string>"        // ModelDef.name that produced this model
      }
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

### No-op rebuild output

When `smelt build` or `smelt run` completes with no models executed (either because `--select` matched nothing, or because no models required re-running), smelt must emit a human-readable line to **stderr** to avoid silent ambiguity:

```
smelt: nothing to rebuild
```

When `--select` matched no models, the message is:

```
smelt: no models matched the selector(s)
```

Both messages are emitted to **stderr** so they do not pollute stdout-parsed output. Neither message is emitted on a successful build that ran at least one model.

> **Implementation note.** The current implementation logs `"No models matched the selectors"` via `info!()`, which is only visible when `RUST_LOG=info` is set. This diverges from the above spec. Until fixed, users observing complete silence should treat it as a no-op (not an error).

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

### `--allow-downgrade` — incremental safety escape hatch

A model that declares `incremental:` but whose SQL fails the safety classifier (contains `OVER`, `HAVING`, `LIMIT`, a subquery, or a non-deterministic function in the outer body) is **refused at planning time** by default. `smelt run` exits non-zero with a diagnostic naming the model and the construct.

`--allow-downgrade` suppresses the hard refusal: the model is treated as a full-table refresh for this run, matching the legacy behaviour. This flag is an explicit opt-in — it must be set every time. It is intended as a temporary escape hatch while the model SQL is being fixed, not as a permanent configuration option.

When `--allow-downgrade` is set, a `warn!` line is emitted for each downgraded model:

```
WARN Incremental safety check failed (falling back to full-table refresh because --allow-downgrade is set): Model '...': ...
```

`--allow-downgrade` has no effect on models that are not incremental, and no effect on models that pass the safety classifier.

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

### Argument resolution algorithm

For each user-supplied entity identifier `arg`:

1. **Determine the active scope.** If `--scope` was passed, use its value (treating `--scope ''` as "no scope"). Otherwise compute the cwd-derived scope per §"Argument resolution and `--scope`". The active scope is either a `Vec<String>` of path segments or `None`.
2. **Build the candidate path tuples.**
   - If the active scope is `Some(s)`, the candidates are `[s ++ arg_segs, arg_segs]` in that order.
   - If `None`, the only candidate is `arg_segs`.
3. **Resolve each candidate** against the workspace via `resolve_ref_path` (the same resolver `smelt.<path>` references use inside model SQL — `architecture.md` §"Resolution"). The first candidate that resolves wins.
4. **No candidate resolves** → emit a "not found" diagnostic that lists every entity whose leaf segment matches `arg_segs.last()`. If exactly one such entity exists, the diagnostic includes `did you mean '<full path>'?`.
5. **No candidate resolves and the arg itself is a bare leaf with no scope active**, but multiple entities have that leaf → emit the "ambiguous" diagnostic listing all matches.

Selectors passed to `--select` / `--exclude` are expanded through the same algorithm: each selector value is treated as a bare identifier and substituted with its resolved full path before the selection engine runs. Selectors that already contain a `:` (selector grammar such as `tag:revenue`, `path:models/silver`) are passed through unchanged — they are not entity identifiers.

### Cwd-derived scope computation

Given the resolved project root `<project>` and the configured scan-root list `paths:` (which defaults to `["models"]`):

1. Compute `rel = cwd.strip_prefix(<project>)`. If `cwd` is not under the project root, no auto-scope applies.
2. For each `scan_root` in `paths:`, in declaration order, check whether `rel.starts_with(scan_root)`. The first match wins.
3. If a match is found, the auto-scope segments are `rel.strip_prefix(scan_root)`'s components, joined by `.`. An empty result (cwd *is* the scan root itself) is "no scope".
4. If no scan root matches, no auto-scope applies.

The cwd-derived scope is informational at command start and does not change mid-run. It is also surfaced on the first line of the `--verbose` log so users can see which scope is active.

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

**Scope is input shorthand, never resolution policy.** The CLI accepts shorthand identifiers because typing `silver.events_parsed` from inside `models/silver/` is tedious for the common iteration loop. But shorthand stops at the CLI boundary: the resolver inside model SQL always sees the canonical `smelt.<path>`, the dependency graph and run manifest key on the canonical path, and command output always prints canonical paths. This keeps two invariants intact at once — the spec's "identity falls out of structure" principle (model bodies don't depend on where the user happened to be standing) and the requirement that any printed identifier round-trips cleanly back into any other command or into a `.sql` file.

**Cwd auto-scope, not config-driven default.** A workspace-config `default_scope:` would tie ergonomics to a config edit and create a fixed convention across all developers on a project; cwd derivation is per-shell-session and matches how `git`, `kubectl`, and most shell-context tools work. The `--scope` flag is the explicit override for scripts and CI where cwd is not a meaningful signal.

**Bare leaves with no scope are a hard error, not a fallback search.** When the user runs `smelt type events_parsed` from the project root (no auto-scope) and the workspace contains `silver.events_parsed`, the command must error rather than silently picking the only candidate. This is what makes ambiguity safe: adding a second `bronze.events_parsed` later will never change the behaviour of a passing command. The error includes a `did you mean '<full path>'?` hint for the single-match case so the failure is one keystroke from a fix.

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
10. **All CLI output is canonical `smelt.<path>`.** Model lists, type signatures, diagnostics, `smelt explain --json` keys, log lines, and any other identifier-bearing output must use the full canonical path. `--scope` adjusts input parsing only.
11. **Argument resolution uses the same resolver as model SQL.** Every entity argument flows through `resolve_ref_path` after scope expansion; there is no parallel leaf-only resolver in the CLI surface. The dependency graph and run manifest are keyed by canonical paths only.

## Known Divergences / Open Questions

- **Exit code standardization incomplete.** Configuration errors, YAML parse failures, and selector parse errors exit with non-zero codes but the exact code is not consistently `2` or any defined value distinct from `1`. Exit code meaning for "user/config error" is not defined.
- **`smelt test --select` is substring match, not selector syntax.** Unlike all other commands, `smelt test`'s `--select` does a simple substring match on test file names rather than using the selector parser. See `model_selection.md` Known Divergences.
- **`smelt explain` physical section gating.** The `physical` section of the explain output is documented as present, but the condition that triggers its inclusion (`--show-physical` flag?) is not clearly surfaced in the CLI help or user guide.
- **`.smelt/schemas/` not documented.** The schema state directory that `smelt diff` reads from is not documented in user-facing docs. Its format, update timing, and lifecycle are not specified.
- **`smelt status` reads from live DB.** Gap detection requires a database connection; this is not documented clearly in the command help.
- **No project-wide compile-only flag (TB-3).** `smelt build --dry-run` does not exist; `smelt build --show-plan` requires a positional model-file argument. There is no single command to compile every model and show the plan without executing. Two candidate resolutions: (1) extend `--show-plan` to accept no positional argument for project-wide output, or (2) add `smelt build --dry-run` mirroring `smelt run --dry-run` semantics across the seed→run lifecycle.
- **`--select` whitespace handling is unspecified.** `--select "a b"` produces a single literal selector `"a b"` that silently matches nothing. Whether this should be an error or a warning is open; current behavior is silent.
- **Manifest format and `.smelt/` layout pre-`run_state.md`.** Manifest format, `.smelt/` directory layout, run IDs, parallelism semantics, and failure recovery are not specified. `smelt status` and `smelt history` Surface descriptions in this spec name commands but defer their on-disk format to a future `run_state.md`. Behaviour is implementation-defined until then. (See `architecture.md` §"Specs not yet authored".)
- **Generator-emitted model `origin` field in `smelt explain --json` is landed.** The `origin` field in §"`smelt explain --json` output schema" surfaces generator emissions distinctly from hand-authored models (per `meta_language.md` §"Multi-model production"). The `ModelOriginKind::Generated { generator_file, generator_name }` enum in `smelt-core/src/origin.rs` is the production type; `ExplainModel.origin` and `CatalogModel.origin` carry it. The `generator_file:<path>` selector parses via `SelectionMethod::GeneratorFile` and resolves against the `emitted_models()` survivor set. The `smelt docs generate` Markdown renderer includes a `**Source**:` line for emitted models. Tracked in `docs/plans/20260509-meta-language-overall.md`.

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
