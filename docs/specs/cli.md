---
feature: cli
status: experimental
last_reviewed: 2026-07-11
owners: [andrew]
---

# CLI

> **What this is.** A normative spec for the `smelt` command-line interface — exit codes, command semantics, the `smelt explain --json` schema, and the `smelt build` lifecycle. Flag enumerations are in `docs-site/docs/reference/cli.md`; this spec covers the **behavior** those flags control.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No plan-phase headings (`### Phase A — …`), no inline phase labels (`Meta list (Phase A)`), no plan-vocabulary status callouts (`[deferred to Phase E1]`) in §Surface, §Semantics, §Design, or §Constraints. Implementation status that needs naming goes in §Known Divergences (describe behaviour, link the plan; phase numbers tolerated only when paired with a plan link) or §References → Plans (history) (link plan files; do not describe their phase structure). See the Timeless-oracle rule in `CLAUDE.md` for the full rule and good/bad examples.

## Surface

### Commands

| Command | Purpose |
|---------|---------|
| `smelt init [DIR]` | Non-interactively scaffold a minimal working project in `DIR` (default `.`) |
| `smelt run` | Execute models in topological order |
| `smelt build` | Seed then run (convenience wrapper) |
| `smelt backbuild` | Rebuild a model and its upstreams over a time range |
| `smelt seed` | Load CSV seeds into the target database |
| `smelt test` | Run unit tests against in-memory DuckDB |
| `smelt check` | Run data-quality checks against built data in the configured target |
| `smelt diff` | Report pending schema changes (offline) |
| `smelt table <model>` | Show inferred column schema for a model (offline) |
| `smelt type [model]` | Show model function signature (offline) |
| `smelt status [model]` | Show incremental interval coverage and gaps |
| `smelt history [model]` | Show past run records |
| `smelt list` | List discovered project entities (models, seeds, sources, tests, checks) with kind and materialization (offline) |
| `smelt clean` | Remove build artifacts under `target/` (compiled docs, catalog output); never touches state (`.smelt/`) or the target database |
| `smelt explain` | Output model graph as JSON for orchestrators |
| `smelt bakeoff <model>` | Measure per-cell technique cost against a replayed window of real data; `--pin` emits the winning choice |
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

Every CLI command that takes an entity identifier (a model, function, seed, source, or test) accepts a **dot-path argument** matching the universal addressing scheme defined in `architecture.md` §"Resolution: `smelt.<path>` is the universal addressing scheme". The argument is normally written without the leading `smelt.` prefix — that prefix is implicit at the CLI surface — but a leading `smelt.` is **accepted and stripped** so that any identifier the CLI prints (which is always the full canonical `smelt.<path>` form) can be copy-pasted straight back into a command. Both `silver.events_parsed` and `smelt.silver.events_parsed` resolve identically. After the optional prefix is stripped, the remainder is the same path tuple the resolver matches against.

Three input shapes are accepted:

| Shape | Example | Resolution |
|-------|---------|------------|
| **Full path** | `silver.events_parsed` (or `smelt.silver.events_parsed`) | A leading `smelt.` is stripped if present, then resolved as-is against the workspace. Always works. |
| **Scoped shorthand** | `events_parsed` (with scope `silver`) | Expanded to `<scope>.<arg>` (`silver.events_parsed`) and resolved. There is **no fall-through** to the bare argument: if `<scope>.<arg>` does not resolve, the command errors (it does not silently retry `<arg>`). To address an entity outside the active scope, pass its full path. |
| **No-scope bare leaf** | `events_parsed` (no scope set) | Resolved as a full path. Errors if no entity at that exact path exists, even if a same-named entity exists in a sub-namespace. |

**Scope sources, in precedence order:**

1. **`--scope <prefix>` flag** (explicit). The flag value is a dot-path (e.g. `silver`, `marts.daily`); whitespace and the literal `smelt.` prefix are rejected.
2. **Working-directory derivation** (auto). The auto-scope is the address segments of the current working directory — that is, the cwd's path relative to the project root, with any matching `paths:` strip-prefix removed (the same address derivation the resolver applies to files, per `architecture.md` §"Resolution: `smelt.<path>` is the universal addressing scheme"). If cwd is `<project>/<prefix>/<segs...>` where `<prefix>` is a `paths:` entry, the auto-scope is `<segs joined by .>`; if cwd is under a directory **not** named in `paths:`, those directory names remain part of the scope. If cwd is the project root, above the project, or is exactly a `paths:` strip-prefix directory (so the remaining segments are empty), no auto-scope applies.
3. **No scope.** The argument must be a full path; bare leaves resolve only against entities whose full path is exactly the leaf.

`--scope ''` (empty string) forces "no scope" regardless of cwd. This is the explicit opt-out for scripts that want to be cwd-insensitive.

**Disambiguation rules:**

- **No scope-expansion fall-through.** When a scope is active, a shorthand argument resolves **only** as `<scope>.<arg>`. If that exact path does not resolve, the command errors; it never silently retries the bare `<arg>` or searches up the scope hierarchy. This is what keeps a passing command stable: adding a top-level entity later can never change which entity a scoped shorthand resolved to. To reach an entity outside the active scope, pass its full path (full-path arguments are honored regardless of scope — see below).
- **Ambiguity at no-scope resolution.** When the user passes a bare leaf (e.g. `events_parsed`) with no scope and the leaf matches multiple entities (`silver.events_parsed`, `bronze.events_parsed`), the command exits `2` (usage error; see §"Exit codes") with a diagnostic listing all matches and a hint to use `--scope` or the full path. The CLI does not silently pick one.
- **Cross-scope full paths.** A full-path argument (`bronze.raw_events`) is honored regardless of the active scope. Scope narrows input, never output or cross-references.

**Canonical-display rule.** Every CLI command's output uses the full canonical `smelt.<path>` form for every entity it names — model lists, type signatures, diagnostics, JSON output keys, log lines. Scope changes how the user *types* an identifier; it never changes how the CLI *prints* one. Copy-pasting any printed identifier — including its leading `smelt.` prefix — back into a `--select`, into another command argument, or (minus the prefix) into a model `FROM` clause must produce the same resolution. Because entity arguments strip a leading `smelt.` (see above), the printed `smelt.<path>` form round-trips without edits.

### Exit codes

This is the normative exit-code contract for every `smelt` subcommand. Every other mention of exit codes in this spec (the `smelt check` exit paragraph below, the no-op/unresolvable-selector table in §"No-op vs unresolvable selector", the `smelt build` lifecycle §"Check" step, and Constraints & Invariants item 6) refers back to this section rather than restating it.

| Code | Meaning |
|------|---------|
| `0` | Success. Includes a `warn`-severity `smelt check` violation and an empty-but-valid selection (§"No-op rebuild output") — a build that ran nothing because there was nothing to do is not a failure. |
| `1` | Detected failure. A failed model build, a failed `smelt test` case, an `error`-severity `smelt check` violation, `smelt diff` detecting a schema change, or `CheckTargetNotBuilt` (a check referencing a model not built in the target). |
| `2` | Usage error. Malformed CLI arguments (clap-detected), an unresolvable or ambiguous selector/entity argument, a malformed or missing `smelt.yml`, or an unresolvable project/target. |

Codes `1` and `2` are deliberately distinct: `1` means the command ran correctly and *found* a problem in the data or models; `2` means the command could not run at all because its own inputs (flags, config, project structure) were invalid. An orchestrator should treat `1` as "investigate the pipeline" and `2` as "fix the invocation" — retrying a `2` without changing the command is never useful.

**`smelt diff` specifics:** exits `0` if no schema changes are detected; exits `1` if any changes are found (including new or removed models). This makes it suitable as a CI gate.

**`smelt test` specifics:** exits `0` if all tests pass; exits `1` if any test fails.

**`smelt check` specifics:** exits `0` if every `error`-severity check passes (zero violating rows); exits `1` if any `error`-severity check has violations. `warn`-severity checks never affect the exit code — a check with `severity: warn` and violations reports `WARN` and the command still exits `0`. A check whose referenced model is not built in the target fails with `CheckTargetNotBuilt` (exit `1`), never a silent pass.

**`smelt init` specifics:** exits `0` on a successful scaffold. Exits `2` if the target directory already contains a `smelt.yml` (usage error — the fix is a different/empty directory, not a retry of the same command).

**`smelt list` specifics:** exits `0` if discovery and parsing succeed, including when the (possibly selector-narrowed) result set is empty. Exits `2` on a parse error or an unresolvable/ambiguous selector, per the general selector-resolution rule above.

**`smelt clean` specifics:** exits `0` whether or not `target/` existed to remove (removing nothing is not a failure). Exits `1` if `target/` exists but cannot be removed (e.g. a permissions error) — the command ran but failed to do its job, not a malformed invocation.

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

**`smelt explain` excludes tests.** `smelt explain` (with or without `--json`) filters out all `smelt.test` declarations from its output via the test-kind predicate applied to every discovered entity. Tests never appear in `models`, `execution_order`, or the physical plan section. This filtering is not flag-controlled; it is always active.

### `smelt ui`

| Flag | Default | Description |
|------|---------|-------------|
| `--host` | `127.0.0.1` | Address to bind the UI server to. Loopback addresses (`127.0.0.1`, `::1`, `localhost`) require no further opt-in. |
| `--port` | `3000` | Port to bind the UI server to. |
| `--allow-remote` | off | Required to bind `--host` to a non-loopback address. |

`smelt ui` has no authentication and no HTTPS — it is designed to be reached from the machine running it. Binding to a non-loopback host without `--allow-remote` is a hard error naming the flag; smelt never silently falls back to a loopback bind. Passing `--allow-remote` proceeds and logs a startup warning that the server is reachable from other hosts. The CORS policy allows only the server's own origin (`http://{host}:{port}`, plus `http://localhost:{port}` when bound to loopback) — no other origin can read its API responses from a browser.

### `smelt explain <model>` maintenance-plan report

`smelt explain` accepts an optional positional model-name argument. When given, it prints that
model's maintenance plan (`incremental_models.md` §Surface "The plan (derived, reported)") instead
of the whole-project graph: every cell (its trigger, corner, technique, and `ledger_catch_up`
flag), the derived per-source scan clamps, the per-source partition-locality verdict, any
admission refusals, the model's own **Relation Contract** fill (`models.md` §"The Relation
Contract": its clock, identity, and derived `grain` label), and one contract block per inbound
edge (upstream dependency) — a declared source or an upstream maintained model, rendered through
the identical `clock:` / `identity:` / `derived grain:` rows and labelled `(source)` or `(model)`
so the reader knows which provider filled them; a row prints `(none)` when that provider declares
neither fact. The report is read-only and plain text. `--select` and `--json` are ignored when a
model-name argument is given, except `--json` combined with `--show-sql` (below).

**`--show-sql`** additionally prints, after each cell's report block, the maintenance statements
that cell executes — the output of the same pure emitters a run executes
(`incremental_models.md` §"Statement emission (single owner)"). Each cell's SELECT body is compiled
through the same `CompilerRegistry` apparatus a real run uses — the real discovered project's
ephemeral resolver (so a `smelt.<ephemeral>` ref is CTE-inlined, not resolved as a physical table
reference) and the real upstream column typing derived from static type inference (so `SUM`/`AVG`
over a `smelt.ref()` column casts to that column's actual type instead of the `BIGINT` default) —
so the printed SQL matches what a run would compile for the same model and inputs (see Known
Divergences for the one residual gap: a column aggregated directly off an ephemeral ref). Statements print in execution order; a transactional group is bracketed
by `BEGIN`/`COMMIT` lines in the printout to show its atomicity (the backend supplies the real
transaction mechanics at run time). Region literals come from `--period <start>..<end>` when
given; without `--period`, the symbolic placeholders `{{window_start}}`/`{{window_end}}` stand
in, so the emitted shape is inspectable without choosing a window. `--show-sql` never connects
to a backend and never executes anything. With `--json` alongside `--show-sql` (the one case
where `--json` is honored together with a model-name argument), the per-model report is emitted
as JSON with a `statements` array per cell
(`{"sql": "<statement>", "transactional_group": <int>}`) — the machine-liftable form
documentation generators embed.

A model with no maintenance plan (not `refresh: incremental`, or no `grain:` declared) prints a
one-line notice rather than an error, and exits `0`.

When the plan's column-group derivation could not distinguish per-column provenance (an
unqualified column ambiguous between two joined sources — `model_properties.md` §"Per-column
mutation-sensitivity / column provenance"), the report calls out the resulting whole-model
collapse in plain language rather than printing an indistinguishable single-group plan.

Omitting the model-name argument keeps the existing whole-project graph behavior described below,
unchanged.

### `smelt bakeoff <model>` flags

| Flag | Default | Description |
|------|---------|-------------|
| `--cells <col>@<source>,...` | every cell with ≥2 admissible techniques | Repeatable/comma-separated. Narrows measurement to the named cells. A named cell with only one admissible technique errors — there is nothing to compare. |
| `--runs N` | `3` | Splits the driving source's event-time extent into `N` sequential windows and replays each window, in order, once per candidate technique. Each replay is a real `execute_project` run against the project's own data. |
| `--target <name>` | active target | The declared target to clone for scratch measurement runs. |
| `--keep` | off | Retain the scratch schemas (`smelt_bakeoff_<model>_<technique>`) after measurement instead of dropping them. |
| `--pin` | off | Print the winning `cells[]` entry (or a full `maintenance:` block when the model has none) as YAML to stdout. Emit-only — never writes the model's `.sql` file. |

`smelt bakeoff` reports, per measured cell, each admissible technique's wall-clock cost and row
count across the `--runs` windows, and cross-checks every pair of candidate techniques' output
per window with `EXCEPT ALL` in both directions, failing loud on a mismatch rather than reporting
a cost for a technique whose output diverged. See `incremental_models.md` §"CLI" for the full
scratch-target and pin semantics.

### `smelt explain --json` output schema

```json
{
  "models": {
    "<model_name>": {
      "dependencies": ["<upstream_model_name>", ...],
      "materialization": "table" | "view" | "ephemeral" | "materialized_view",
      "refresh": "full" | "incremental" | "materialized_view",     // omitted when "full" (default)
      "grain": "partition" | "key" | "key_per_partition" | null,  // present iff refresh == "incremental"
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

An empty result set is **not** the same as an unresolvable selector. A selector that names an entity which does not resolve to anything is a hard error (see §"Argument resolution algorithm" step 4 and §"No-op vs unresolvable selector" below); the no-op messages here apply only when every selector **resolved** but the resulting working set is legitimately empty.

When `smelt build` or `smelt run` completes with no models executed — every selector resolved, but either the resolved set selected nothing or no models required re-running — smelt must emit a human-readable line to **stderr** to avoid silent ambiguity and exits **`0`** (an empty-but-valid selection is a success, not a failure):

```
smelt: nothing to rebuild
```

When the (valid) selectors matched no models, the message is:

```
smelt: no models matched the selector(s)
```

Both messages are emitted to **stderr** so they do not pollute stdout-parsed output. Neither message is emitted on a successful build that ran at least one model.

### No-op vs unresolvable selector

The two empty-output cases are distinct and have different exit codes (per §"Exit codes"):

| Case | Example | Behaviour | Exit code |
|------|---------|-----------|-----------|
| **Unresolvable selector** — an entity-name selector resolves to no entity of any kind | `--select typo_name` (no such model) | Hard "not found" diagnostic (§"Argument resolution algorithm" step 4) | `2` (usage error) |
| **Valid but empty selection** — every selector resolved, but the matched set is empty | `--select tag:nonexistent`, or a valid selector whose models are all up-to-date | `smelt: no models matched the selector(s)` / `smelt: nothing to rebuild` to stderr | `0` |

A typo'd entity name fails loudly rather than silently building nothing; a `tag:`/`generator_file:` selector that legitimately matches no models (per `model_selection.md` §"Tag matching" and `model_selection.md` §"Selection methods") is a quiet no-op.

### `smelt build` lifecycle

A single `smelt build` performs these steps, in order:

1. **Load** `smelt.yml` from `--project-dir`. Fail if absent.
2. **Validate** that the requested `--target` exists in the config.
3. **Discover** all project files by walking every non-excluded subdirectory under the project root (discovery is project-wide; `paths:` only strips address prefixes — there are no per-kind dedicated scan paths, per `architecture.md` §"Resolution: `smelt.<path>` is the universal addressing scheme"). The resolver classifies each file by format and content: `.sql` files become models, `smelt.define`s, tests, or checks; `.csv` files become seeds; per-entity `.yml` files (a `users.yml` next to a `users.csv`, or alone) become seed sidecars or sources respectively.
4. **Seed** — run the seed lifecycle per `seeds.md` for each non-ephemeral CSV seed (in deterministic sorted order): smelt parses and type-infers the CSV itself and ingests it via `Backend::load_table(...)` — not a backend-specific `read_csv_auto` recipe. Ephemeral seeds are skipped (they inline as CTEs at compile time); sources are never loaded. Schemas are auto-created if absent.
5. **Plan** — build the logical dependency graph; apply planner rules; produce the physical execution graph. Models execute in topological order.
6. **Run** — for each model in topological order, materialize according to its effective materialization:
   - `table` / `materialized_view`: `CREATE OR REPLACE TABLE` (atomic replacement)
   - `view`: `CREATE OR REPLACE VIEW`
   - `ephemeral`: inlined as CTE — no DDL emitted
7. **Check** — after a model materializes, run every `smelt.check` that references it against the just-written data (per `testing.md` §"Check execution model"). An `error`-severity violation marks every model **downstream of the checked model** as skipped for the remainder of the build (bad data does not propagate) and makes the build exit `1`; a `warn`-severity violation is reported and the build continues. Checks run within the same `build` invocation only — `smelt run` does not run checks.

`smelt build` is idempotent. Re-running with the same inputs and the same time range produces the same final database state.

### `--exclude` and inconsistent working sets

`--exclude` removes models from the working set after all `--select` expansions complete (`model_selection.md` §"Selection algorithm"). When the excluded selector carries an upstream `+` operator (`--exclude +model`), it removes the model **and its transitive upstreams**. If any removed upstream is still required by a model that remains in the working set, smelt refuses to run an inconsistent set: it emits a diagnostic naming the retained model and the missing upstream dependency rather than executing a model against an absent input. The user must either narrow the exclusion (drop the `+`) or also exclude the dependent model.

### `smelt run` vs `smelt backbuild`

`smelt run` executes the selected models for the requested time range. Incremental models receive a DELETE+INSERT for the given `[start, end)` window.

`smelt backbuild` additionally traverses upstream of the selector target(s) and rebuilds the full dependency chain. It uses the model's batch-safety classification to determine whether the range can be processed in a single query or must be split into per-partition or batched chunks.

### Failure summary

`smelt run` and `smelt build` both print a grouped failure summary to stderr at the end of a failed run, naming every model that failed — not just the first. Independent models that fail in the same `--jobs`-scheduled wave each get their own entry; a second or third failure is never silently downgraded to "skipped" (per `run_state.md` §"Run report"). Each entry carries the model's first error line and a one-line hint toward the likely next action:

```
smelt: run <run_id> failed — <N> model(s) failed:
  - <model>: <first line of the recorded error>
    hint: <next action>
```

The hint is chosen from a coarse classification of the recorded error text into one of three causes — compile (parse/type/reference resolution failure: points at the model's SQL), execute (the backend rejected the compiled SQL: points at `-v`/`--show-plan`), or check (a constraint/data-quality violation: points at `smelt check`). The classification is a best-effort text match, not a structured error code — nothing upstream of the run report currently tags a failure with its originating stage. A successful run prints no failure block. The failure summary is presentation only: it never changes the run's exit code (`smelt run`/`smelt build` still exit per §"Exit codes").

### `--dry-run` prints the maintenance statements

`smelt run --dry-run` and `smelt backbuild --dry-run` print, for every model the invocation
would execute, the maintenance statements the run would execute — the output of the same pure
emitters a real run consumes (`incremental_models.md` §"Statement emission (single owner)") — not
merely the compiled SELECT body. Region literals are **real**: they come from the invocation's
resolved `--event-time-start`/`--event-time-end` window, never symbolic placeholders.
Transactional groups are bracketed by `BEGIN`/`COMMIT` lines, exactly as in
`smelt explain <model> --show-sql`.

`smelt backbuild --dry-run` additionally reflects the chunking a real backbuild performs: when
the batch-safety classification splits the range, statements print once per chunk, each chunk
introduced by a boundary line naming its `[start, end)` window and its position
(`-- chunk 2/5: [2026-03-21, 2026-03-22)`), in the order a real backbuild would execute them. An
auto-chunked backfill is thereby inspectable in full before it runs.

`--dry-run` never executes a statement against the target. The division of labour with
`smelt explain <model> --show-sql`: `--show-sql` is the no-window, single-model plan-inspection
surface (symbolic bounds unless `--period` is given); `--dry-run` is the "exactly what would
*this invocation* do" surface — real window, real selection, real chunking.

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

0. **Strip an optional `smelt.` prefix.** If `arg` begins with the literal `smelt.`, remove it before any further processing, so a copy-pasted canonical identifier resolves to the same entity as its bare form.
1. **Determine the active scope.** If `--scope` was passed, use its value (treating `--scope ''` as "no scope"). Otherwise compute the cwd-derived scope per §"Argument resolution and `--scope`". The active scope is either a `Vec<String>` of path segments or `None`.
2. **Build the candidate path tuple.**
   - If the active scope is `Some(s)`, the only candidate is `s ++ arg_segs`. There is no bare-`arg_segs` fall-through candidate — a scoped shorthand resolves exclusively under its scope; a full path must be passed to reach an entity elsewhere.
   - If `None`, the only candidate is `arg_segs`.
3. **Resolve the candidate** against the workspace via `resolve_ref_path` (the same resolver `smelt.<path>` references use inside model SQL — `architecture.md` §"Resolution: `smelt.<path>` is the universal addressing scheme"). It either resolves or it does not.
4. **No candidate resolves** → emit a "not found" diagnostic that lists every entity whose leaf segment matches `arg_segs.last()`. If exactly one such entity exists, the diagnostic includes `did you mean '<full path>'?`.
5. **No candidate resolves and the arg itself is a bare leaf with no scope active**, but multiple entities have that leaf → emit the "ambiguous" diagnostic listing all matches.

Selectors passed to `--select` / `--exclude` are expanded through the same algorithm: each **entity-name** selector value (a `model_name` selector, after any leading/trailing `+` graph operators are stripped — see below) is treated as a bare identifier and substituted with its resolved full path before the selection engine runs. An entity-name selector that resolves to no entity is a **hard "not found" error** (step 4), not a silent empty match — the same fail-loud behaviour as a bare command argument. Selectors that use a method prefix (`tag:`, `generator_file:`) are not entity identifiers and are passed through to the selection engine unchanged; a method selector that matches no models is a valid empty selection, not an error (see §"No-op vs unresolvable selector").

**Graph operators are stripped before resolution.** A `model_name` selector may carry leading and/or trailing `+` graph operators (`+events_parsed`, `events_parsed+`, `+events_parsed+`, per `model_selection.md` §"Selector syntax"). The `+` markers are removed before the bare identifier is resolved, and re-attached to the resolved full path afterwards: `+events_parsed` resolves `events_parsed` to `silver.events_parsed` and yields the selector `+silver.events_parsed`. The `+` operators never participate in entity resolution.

### Cwd-derived scope computation

The cwd-derived scope is the **address** of the current working directory — the cwd's path relative to the project root, with any matching `paths:` strip-prefix removed. This is identical to how a file's `smelt.<path>` address is derived (`architecture.md` §"Resolution: `smelt.<path>` is the universal addressing scheme"): `paths:` is a list of strip-prefixes, **not** a discovery gate, and there are no per-kind dedicated scan paths. Given the resolved project root `<project>` and the configured strip-prefix list `paths:` (which defaults to `["models"]`):

1. Compute `rel = cwd.strip_prefix(<project>)`. If `cwd` is not under the project root, no auto-scope applies.
2. For each strip-prefix in `paths:`, in declaration order, check whether `rel.starts_with(<prefix>)`. The first match wins; the matched prefix is removed from `rel`. (If no prefix matches, `rel` is used unchanged — directories not named in `paths:` keep their names as scope segments, exactly as they would as address segments.)
3. The auto-scope segments are the remaining components of `rel` (after any prefix strip), joined by `.`. An empty result — `cwd` is the project root, or is exactly a `paths:` strip-prefix directory — is "no scope".

The cwd-derived scope is informational at command start and does not change mid-run. It is also surfaced on the first line of the `--verbose` log so users can see which scope is active.

### `smelt test` isolation

`smelt test` compiles each test into a standalone SQL query and executes it against an **in-memory DuckDB instance** using mock data declared in the test's frontmatter. No connection to the project's target database is made. Tests always run on DuckDB regardless of the project's configured target(s).

`smelt test --select` uses the **full selector syntax** (`model_selection.md` §"Selector syntax") — the same `model_name` / `tag:` / `generator_file:` methods and `+` graph operators every other command accepts, resolved through the §"Argument resolution algorithm". It is **not** a substring match on test names. A test model matches a `model_name` selector by its canonical `smelt.<path>` (test models are addressable, per `architecture.md`), and matches `tag:` by its effective tag set. This makes test selection consistent with `smelt run`/`smelt build` selection.

### `smelt check` — data-quality assertions against built data

`smelt check` executes each `smelt.check` declaration's failing-rows query against the project's **configured target** (not in-memory DuckDB — a check asserts on the real materialized data; see `testing.md` §"Check execution model"). A check passes iff its query returns **zero rows**; returned rows are violations. The command reports `PASS`/`FAIL`/`WARN` per check with the violation row count and a capped inline sample of violating rows, and exits per §"Exit codes" (`error`-severity violations → `1`; `warn`-only → `0`). A check that references an unbuilt model fails with `CheckTargetNotBuilt` rather than passing silently.

`smelt check --select` is a **substring match on the check name** (repeatable; a check runs if any `--select` value is a substring of its name). It does not use the full selector syntax — no `tag:`/`generator_file:` methods, no `+` graph operators — and a selection that matches no check prints `No checks matched the selection.` and exits `0` rather than hard-erroring. Unlike the build-integrated check pass (`smelt build` step 7), standalone `smelt check` runs against whatever is currently materialized and applies no downstream skip-cascade — it is a pure validation pass.

### `smelt init` — non-interactive scaffolder

`smelt init [DIR]` writes a minimal, working smelt project to `DIR` (default `.`): a `smelt.yml`, a `models/` directory containing one example model, one seed CSV, and a `.gitignore` that excludes `.smelt/` and the database file. It takes no interactive prompts — every file it writes has a fixed, deterministic template; there is no wizard and no flag that changes what gets scaffolded beyond the target directory. The scaffolded project builds successfully against DuckDB with no further edits (`smelt build` inside the scaffold exits `0`).

`smelt init` refuses to run against a directory that already contains a `smelt.yml`: it exits `2` (usage error) with a message naming the conflicting file, rather than overwriting or merging. There is no `--force` flag to override this — the guidance is to run `smelt init` in a fresh directory, or to remove the conflicting file first and re-run. `DIR` is created if it does not exist; an existing empty or non-project directory is populated in place.

### `smelt list` — enumerate discovered entities

`smelt list` prints every entity `smelt` discovers in the project — models, seeds, sources, tests, and checks — one per line, in canonical `smelt.<path>` form (§"Canonical-display rule"), alongside its kind and, for models, its materialization. `smelt list` is **offline**: it performs discovery and parsing only, the same project-wide scan `smelt explain` uses, and makes no database connection. It accepts the same `--select`/`--exclude` selector flags as `smelt run`/`smelt build` (`model_selection.md`) to narrow the listed set, and respects `--scope` for shorthand selector arguments exactly as every other command does.

### `smelt clean` — remove build artifacts

`smelt clean` removes `target/` — the directory `smelt docs generate` and other artifact-producing commands write to. `smelt clean` **never touches state**: it does not delete `.smelt/` (run manifests, deployed-schema snapshots consumed by `smelt diff`, or any other versioned state directory), and it does not connect to or modify the configured target database. Only regenerable build output is in scope for removal — the same distinction `incremental_models.md`'s maintenance state draws between *derived, disposable output* and *state a subsequent run depends on to behave correctly*. `smelt clean` is safe to run at any time without affecting incremental correctness or losing run history.

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

**`smelt check` runs against the target; `smelt test` does not.** The two assertion commands deliberately split on execution model: a test asserts on model *logic* and runs in-memory on mock data (fast, no connection), a check asserts on materialized *data* and must run against the configured target where that data lives. `smelt check` is a separate verb (rather than only a `smelt build` phase) so operators can re-validate current data without rebuilding; build-time check execution (`smelt build` step 7) additionally blocks downstream propagation on an `error`-severity failure, matching dbt's `build` skip-cascade and SQLMesh's blocking audits. The full design rationale for the `smelt.check` kind lives in `testing.md` §Design.

**`--show-plan` is per-model in v1.** Whole-project planning is a different operation (it produces a graph view, not a single-model plan) and the right output format has not been chosen. Keeping `--show-plan` per-model leaves the design space open while serving the most common need.

**`--verbose` logs per executed model, not per discovered model.** Logging every discovered model would flood the output on incremental runs where most models are skipped. Emission scaling with work done matches the user's mental model.

**Scope is input shorthand, never resolution policy.** The CLI accepts shorthand identifiers because typing `silver.events_parsed` from inside `models/silver/` is tedious for the common iteration loop. But shorthand stops at the CLI boundary: the resolver inside model SQL always sees the canonical `smelt.<path>`, the dependency graph and run manifest key on the canonical path, and command output always prints canonical paths. This keeps two invariants intact at once — the spec's "identity falls out of structure" principle (model bodies don't depend on where the user happened to be standing) and the requirement that any printed identifier round-trips cleanly back into any other command or into a `.sql` file.

**Cwd auto-scope, not config-driven default.** A workspace-config `default_scope:` would tie ergonomics to a config edit and create a fixed convention across all developers on a project; cwd derivation is per-shell-session and matches how `git`, `kubectl`, and most shell-context tools work. The `--scope` flag is the explicit override for scripts and CI where cwd is not a meaningful signal.

**Bare leaves with no scope are a hard error, not a fallback search.** When the user runs `smelt type events_parsed` from the project root (no auto-scope) and the workspace contains `silver.events_parsed`, the command must error rather than silently picking the only candidate. This is what makes ambiguity safe: adding a second `bronze.events_parsed` later will never change the behaviour of a passing command. The error includes a `did you mean '<full path>'?` hint for the single-match case so the failure is one keystroke from a fix.

**Scoped shorthand resolves only under its scope — no fall-through.** With scope `silver` active, `events_parsed` resolves exclusively as `silver.events_parsed`; it never falls back to a bare top-level `events_parsed`. A silent fall-through would be a stability hazard: a command that resolved `events_parsed` to the top-level entity via fall-through would silently retarget the moment a `silver/events_parsed.sql` was added. Removing the fall-through extends the same "adding an entity never changes a passing command" invariant to the scoped case — the only way to address an entity outside the active scope is its full path, which is unambiguous and cwd-independent.

**`paths:` is a strip-list, and the cwd scope is derived the same way.** The auto-scope is just the address of the current directory, so it tracks exactly the addressing model `architecture.md` defines: discovery scans every non-excluded subdirectory and `paths:` only strips address prefixes. There is no separate "scan root" concept in the CLI — the directory a developer stands in maps to a scope segment list by the identical strip-prefix rule that maps a file to its `smelt.<path>`. This keeps the mental model singular: where you stand and what an entity is called are the same coordinate system.

## Constraints & Invariants

1. **`smelt build` is idempotent.** Re-running on the same inputs produces the same final state.
2. **`--help` and `--version` require no project.** They succeed in any directory.
3. **`smelt diff` requires no live connection.** It must work in offline environments (CI without DB credentials).
4. **`smelt test` runs on in-memory DuckDB.** Tests never touch the project's configured target database.
5. **`smelt explain --json` schema is append-stable.** Fields may be added; existing fields must not be renamed or removed without a major version bump.
6. **Exit codes are meaningful.** See §"Exit codes" for the full contract. Scripts should check exit codes, not stdout patterns.
7. **`--dry-run` does not exist on `smelt build`.** It exists on `smelt run` and `smelt backbuild` only.
8. **`--show-plan` requires a positional model-file argument.** Absence is a hard error, not a fallback to project-wide mode.
9. **Multi-value flags are repetition-based.** `--select`, `--exclude`, and similar flags must not silently split internal whitespace into multiple values.
10. **All CLI output is canonical `smelt.<path>`.** Model lists, type signatures, diagnostics, `smelt explain --json` keys, log lines, and any other identifier-bearing output must use the full canonical path. `--scope` adjusts input parsing only.
11. **Argument resolution uses the same resolver as model SQL.** Every entity argument flows through `resolve_ref_path` after scope expansion; there is no parallel leaf-only resolver in the CLI surface. The dependency graph and run manifest are keyed by canonical paths only.
12. **Scoped shorthand has no fall-through.** With a scope active, a shorthand argument resolves only as `<scope>.<arg>`; it never silently retries the bare `<arg>`. Reaching an entity outside the scope requires a full path. Adding a new entity (at any level) never changes which entity a previously-passing command resolved to.
13. **`paths:` is a strip-list, not a scan gate.** Discovery walks every non-excluded subdirectory under the project root; `paths:` only strips address prefixes (`architecture.md` §"Resolution: `smelt.<path>` is the universal addressing scheme"). The cwd-derived scope is computed by the same strip-prefix rule, and the CLI defines no separate per-kind scan paths.
14. **`smelt check` runs against the configured target.** Checks assert on real built data; a check passes iff its failing-rows query returns zero rows. `error`-severity violations set exit `1` and block models downstream of the checked model during `smelt build`; `warn`-severity violations do neither. A check on an unbuilt model is `CheckTargetNotBuilt`, never a silent pass.
15. **`smelt init` never overwrites an existing project.** A target directory already containing a `smelt.yml` is refused (exit `2`) rather than merged or overwritten; there is no `--force` escape hatch.
16. **`smelt list` is offline.** Like `smelt diff`/`smelt table`/`smelt type`, `smelt list` performs discovery and parsing only and makes no database connection.
17. **`smelt clean` never touches state.** It removes only `target/` build artifacts. It must never delete or modify `.smelt/` (run manifests, deployed-schema snapshots) or connect to the configured target database.

## Known Divergences / Open Questions

- **Selector/target usage errors still exit `1`, not `2`.** §"Exit codes" defines `2` for every usage/config error, including an unresolvable or ambiguous selector/entity argument and an unresolvable `--target`. A malformed or missing `smelt.yml` and an unresolvable project root correctly exit `2` — `main` classifies the returned error via its source chain. Selector resolution (`argument_resolution::ResolutionError`) and `--target` validation, however, are erased to an untyped `anyhow` string at each call site before reaching `main`, so they still fall through to the `1` default. Closing this remaining gap needs `ResolutionError` (and an equivalent typed error for unresolvable `--target`) to stay classifiable all the way to `main`. Tracked in `docs/plans/20260719-prod-w1-fail-loud.md`.
- **`smelt test --select` selector-syntax rollout.** `smelt test --select` is specified to use the full selector syntax (§"`smelt test` isolation"), consistent with every other command. Any remaining substring-match behaviour in the implementation is an unlanded gap, not the intended contract.
- **`smelt explain` physical section gating.** The `physical` section of the explain output is documented as present, but the condition that triggers its inclusion (`--show-physical` flag?) is not clearly surfaced in the CLI help or user guide.
- **`.smelt/schemas/` not documented.** The schema state directory that `smelt diff` reads from is not documented in user-facing docs. Its format, update timing, and lifecycle are not specified.
- **`smelt status` reads from live DB.** Gap detection requires a database connection; this is not documented clearly in the command help.
- **No project-wide compile-only flag (TB-3).** `smelt build --dry-run` does not exist; `smelt build --show-plan` requires a positional model-file argument. There is no single command to compile every model and show the plan without executing. Two candidate resolutions: (1) extend `--show-plan` to accept no positional argument for project-wide output, or (2) add `smelt build --dry-run` mirroring `smelt run --dry-run` semantics across the seed→run lifecycle.
- **`--select` whitespace handling is unspecified.** `--select "a b"` produces a single literal selector `"a b"` that silently matches nothing. Whether this should be an error or a warning is open; current behavior is silent.
- **Manifest format and `.smelt/` layout pre-`run_state.md`.** Manifest format, `.smelt/` directory layout, run IDs, parallelism semantics, and failure recovery are not specified. `smelt status` and `smelt history` Surface descriptions in this spec name commands but defer their on-disk format to a future `run_state.md`. Behaviour is implementation-defined until then. (See `architecture.md` §"Specs not yet authored".)
- **The maintenance CLI surface is landed; one technique-resolution gap remains.** `smelt run --since-upstream --source <address> --landed <start>..<end>` (`incremental_models.md` §"CLI") is landed: `RunArgs` accepts the repeatable `--source`/`--landed` pair, forward-propagates the declared per-source deltas through the real per-workspace propagation graph (`smelt_runtime::propagation`), prints the dirty set, and runs exactly the propagated `(model, region)` pairs through `execute_project`. The propagation graph's edges are derived from every model's own `MaintenancePlan` scan clamps — the same clamp the maintenance SQL itself sizes — for both `sources.*` refs and refs to another maintained model in the workspace: the graph builder (`build_forward_graph`) routes a maintained-model upstream through the SAME edge-aware plan derivation (`derive_model_maintenance_plan_with_edges`) that produces the creation cells `smelt explain` reports, so a model-edge clamp in the propagation graph agrees with the clamp `smelt explain` shows for the same edge, and an underivable upstream clock is a `MaintenanceReachNotDerivable` refusal (contributing no walkable edge) rather than a permissive whole-table synthesis. `--source` accepts a maintained-model address as the delta origin (validated through the canonical `resolve_ref_path` resolver — an address that is neither a declared source nor a maintained model is a named error), and the origin model itself is never re-run. What remains is that `execute.rs`'s technique resolution does not yet key off a model-ref cell (`incremental_models.md` §"Known Divergences"). `smelt build <model> --period <start>..<end> --include-upstreams` (backward resolution) is also landed: `BuildArgs` accepts the positional target model plus `--period`/`--include-upstreams`, resolves the required per-ancestor slices and the ancestor-first/target-last build order over the SAME propagation graph (`smelt_runtime::propagation::resolve_build_plan`, backed by `smelt_logical::maintenance::propagate::required_inputs`), prints the resolved-slices report, and builds exactly that set through `execute_project`. `smelt bakeoff <model> [--cells ...]` (per-cell technique cost measurement, with `--pin`) is landed — see §"`smelt bakeoff <model>` flags" above and `incremental_models.md` §"CLI". `smelt explain <model>`'s plan report is landed — see §"`smelt explain <model>` maintenance-plan report" below.
- **The keyed-grain fold-candidate detector admits only a single aggregate projection.** The
  per-model maintenance-plan derivation (`smelt-db`'s `maintenance_plan_report`) resolves a
  `smelt.<path>` ref to another maintained model in the same project into a creation-trigger cell
  clocked by the upstream model's own `timeseries:` declaration, recording a
  `MaintenanceReachNotDerivable` refusal when that clock is underivable
  (`incremental_models.md` §"Upstream model edges"). Separately, a `grain: key` model with two or
  more aggregate columns falls back to `Trigger::Backfill`'s recompute cell with a
  `NoAdmissibleTechnique` refusal recorded for `NewData`, even though the same model's actual
  `refresh: keyed` execution path (`incremental_models.md`) supports arbitrarily many aggregate columns
  via `smelt-planner`'s `classify_cumulative`. Widening the plan-level derivation to match is
  tracked as follow-up work; `smelt explain <model> --show-sql` renders whatever cells the current
  derivation admits — it does not paper over this gap by admitting a cell independently.
- **`--show-sql` casts a column aggregated directly off an ephemeral ref to the `BIGINT`
  default, not its real type.** `smelt-runtime`'s shared compiler
  (`SqlCompiler::compile_with_sql_and_ephemerals`) applies output type casts to a model's SELECT
  body *before* prepending the resolved ephemeral CTEs, so a column like `SUM(rate)` where `rate`
  comes straight off a joined ephemeral model cannot be typed from the real upstream schema at
  cast time regardless of how the caller wires `UpstreamSchemas` — it falls through to the
  `BIGINT` fallback. This is a compile-order limitation in the shared compiler, not an
  `explain`-vs-run divergence: a real `smelt run --dry-run` on the same model produces the
  identical `BIGINT` cast, since both consumers share the one compile path (Run pipeline parity
  rule, `architecture.md`). `--show-sql` therefore still faithfully reproduces what a run would
  execute, casting bug included. A column aggregated off a *non-ephemeral* upstream model ref, or
  an ephemeral ref used outside an aggregate, types correctly. Fixing the underlying ordering is
  tracked as follow-up `smelt-runtime` work; not addressed by
  `docs/plans/20260710-emit-unification.md`.
- **Generator-emitted model `origin` field in `smelt explain --json` is landed.** The `origin` field in §"`smelt explain --json` output schema" surfaces generator emissions distinctly from hand-authored models (per `meta_language.md` §"Multi-model production"). The `ModelOriginKind::Generated { generator_file, generator_name }` enum in `smelt-core/src/origin.rs` is the production type; `ExplainModel.origin` and `CatalogModel.origin` carry it. The `generator_file:<path>` selector parses via `SelectionMethod::GeneratorFile` and resolves against the `emitted_models()` survivor set. The `smelt docs generate` Markdown renderer includes a `**Source**:` line for emitted models. Tracked in `docs/plans/20260509-meta-language-overall.md`.

## References

- **Code**:
  - `crates/smelt-cli/src/main.rs` — command routing, `--target` defaults
  - `crates/smelt-cli/src/explain.rs` — `ExplainOutput`, `ExplainModel`, `ExplainIncremental`, `ExplainPhysical`
  - `crates/smelt-cli/src/commands/` — per-command implementation
  - `crates/smelt-cli/src/commands/build.rs` — `--show-plan` dispatch
  - `crates/smelt-cli/src/logical_graph.rs` — `LogicalGraph::build()`
- **Tests**:
  - `crates/smelt-cli/tests/check_command.rs` — `smelt check` exit codes, severity gating, `--select` substring, unbuilt-target loudness
  - `crates/smelt-cli/tests/build_checks.rs` — `smelt build` check gate: error-severity skip-cascade, warn transparency
- **User docs**:
  - `docs-site/docs/reference/cli.md` — full flag reference
  - `docs-site/docs/guide/model-selection.md` — selector syntax
- **Plans (history)**:
  - `docs/plans/20260502-smelt-loop-findings.md` — TB-1 and TB-4 fixes, TB-3 deferred
  - `docs/plans/20260628-data-checks.md` — `smelt check` command and the `smelt build` check gate
- **Related specs**:
  - `architecture.md` — pipeline stages the CLI orchestrates.
  - `model_selection.md` — `--select` / `--exclude` semantics
  - `models.md` — materialization modes
  - `incremental_models.md` — `--event-time-start` / `--event-time-end` semantics, batch safety classification, `backbuild` behaviour.
  - `functions.md` — `smelt build` plans function expansion as part of the build lifecycle.
  - `schema_evolution.md` — `smelt diff` change classification
  - `testing.md` — `smelt test` and `smelt check` execution
  - `smelt_yml.md` — `targets:` and `paths:` keys consumed by the CLI.
