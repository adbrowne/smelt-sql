---
feature: cli
status: experimental
last_reviewed: 2026-05-03
owners: [andrew]
---

# CLI

> **Scope.** Normative spec for the `smelt` command-line interface — top-level flags, subcommand surface, and the user-visible contracts the CLI is responsible for. The detailed per-command flag tables live in the user-facing reference (`docs-site/docs/reference/cli.md`); this spec captures the load-bearing rules and the divergences uncovered by the smelt-loop runs (`docs/plans/20260502-smelt-loop-findings.md`). It is a stub — sections may be brief — but every section must say something concrete.

## Surface

### Top-level flags

| Flag | Behaviour |
|------|-----------|
| `--help`, `-h` | Print usage and exit 0. Per-subcommand `--help` prints that subcommand's flag table. |
| `--version`, `-V` | Print the package version (`CARGO_PKG_VERSION`) and exit 0. |

`--version` is a top-level flag (parsed by clap on the root command). It must be accepted even when no subcommand is supplied — `smelt --version` and `smelt --help` are the only invocations that succeed without a subcommand.

### Subcommand catalogue (high level)

The full set is `smelt run | backbuild | build | seed | test | diff | docs | table | type | status | history | explain | ui`. Per-subcommand flag tables and lifecycle prose belong to the reference docs. The spec-level rules below apply across subcommands.

### `smelt build`

Convenience command: `smelt seed` followed by `smelt run` against the same project. Build lifecycle (load config → discover → seed → plan → run) is documented in `docs-site/docs/reference/cli.md`.

`smelt build` flag truth-table — the rules every implementation must uphold:

| Flag | Status | Behaviour |
|------|--------|-----------|
| `--verbose` / `-v` | Implemented | Logs the compiled SQL for each model immediately before execution. A run where every model is up-to-date and skipped produces no extra `--verbose` output, because no model executes. |
| `--show-plan` | Implemented | **Per-model**: requires a positional argument naming a model file path (e.g. `smelt build --show-plan models/marts/customers.sql`). Without the positional argument, the command errors. There is currently no project-wide `--show-plan` mode (see TB-3 below). |
| `--select` / `-s` | Implemented | **Repeatable** flag. Supply each selector as its own `--select <value>`. Space-separated values inside a single `--select` are not parsed as multiple selectors (treated as a single literal selector that will not match anything). Selector grammar (model name, `tag:X`, `+X`, `X+`, `+X+`) is shared with `smelt run`. |
| `--exclude` / `-e` | Implemented | Repeatable; same selector grammar as `--select`. |
| `--dry-run` | **Not** present on `smelt build`. | Use `smelt run --dry-run` for the parse-and-validate-without-executing path. Project-wide compile-only on `build` is an open question (TB-3). |
| `--event-time-start` / `--event-time-end` | Implemented | ISO-8601 (date or full timestamp). End is exclusive. Both required together for any incremental execution. |

### `smelt run --dry-run`

`--dry-run` exists on `smelt run` (and `smelt backbuild`) — parse the project, validate, and print what would execute without touching the database. It is the closest existing analogue to "compile only" today.

### `smelt build --verbose`

The contract:

1. For each model that the run actually executes, emit the compiled SQL string to **stdout** immediately before the backend executes it. The emission is prefixed with a comment line `-- <model_name>` so consumers piping stdout (e.g. `smelt build --verbose | tee compiled.sql`) get a syntactically valid SQL transcript.
2. The emission is **per executed model**, not per discovered model. Models skipped because they are already materialised and unchanged produce no `--verbose` output.
3. The non-`--verbose` summary line (`smelt: built N model(s) in T s`) is unchanged by the flag — `--verbose` adds output, it does not replace.

Pair `smelt run --verbose --dry-run` to see compiled SQL without executing.

## Semantics

1. **Help / version do not require a project.** `smelt --help` and `smelt --version` succeed in any directory; subcommands that read the project (`build`, `run`, …) require `smelt.yml` at `--project-dir` (default `.`). Missing `smelt.yml` is a hard error with a pointer at the expected path.
2. **`--target` defaults to `dev`.** A `--target` value that does not exist in `smelt.yml::targets` is a hard error before any work begins.
3. **Repeatable flags.** `--select`, `--exclude`, and other multi-value flags are accepted as repetitions only; comma- or space-separated values inside a single `--<flag>` are taken as one literal value, never parsed as a list. Implementations may reject space-separated values explicitly with a diagnostic when ambiguity is detected, but must not silently split them.
4. **`--dry-run` is a `smelt run` / `smelt backbuild` flag, not a `smelt build` flag.** The build command goes seed→run; if you want compile-only, drop down to `smelt run --dry-run`. (See Known Divergences for the open question about a build-level dry-run.)
5. **Verbose output is additive.** `--verbose` never suppresses the standard summary; it only adds compiled-SQL output before each executed model.

## Design

This is a stub spec; the CLI surface is large and the deep design rationale will land alongside follow-up implementation plans. Decisions worth recording now:

**`--version` is a top-level flag, not a subcommand.** The conventional Unix shape (`tool --version` / `tool --help`) is what users probe first; making `version` a subcommand would force `smelt version` and break muscle memory. A top-level flag costs one line of clap configuration and matches every comparable tool (cargo, dbt, docker).

**`--show-plan` is per-model in v1.** Whole-project planning is a different operation: it produces a graph view, not a single-model plan, and the right output format (DAG + per-node SQL? per-node summary? something else?) has not been chosen. Keeping `--show-plan` per-model in v1 leaves the design space open while serving the most common need (inspect what one model compiled to). See Known Divergences TB-3.

**`--verbose` logs per executed model, not per discovered model.** Logging every discovered model would flood the output with redundant SQL on incremental runs where most models are skipped. The execution-path emission means `--verbose` output scales with work done, which is the user's mental model.

## Constraints & Invariants

1. `--help` and `--version` must succeed in environments without `smelt.yml` (no project context required).
2. Multi-value flags (`--select`, `--exclude`) are repetition-based; the parser must not silently split internal whitespace into multiple values.
3. `--dry-run` does not exist on `smelt build`; adding it requires a deliberate spec change (see TB-3).
4. `--show-plan` requires a positional model-file argument on `smelt build`; absence is a hard error, not a silent fall-through to project-wide mode.
5. The compiled-SQL output of `--verbose` is for the model immediately before its execution — emission and execution must not be reordered.

## Known Divergences / Open Questions

- **TB-3 — No project-wide compile-only flag.** `smelt build --dry-run` is rejected by clap; `smelt build --show-plan` requires a positional model-file argument. There is no single command to "compile every model and show the plan without executing." Two candidate resolutions (open question):
  1. Extend `--show-plan` to accept "no positional argument means whole project," emitting a graph + per-node plan.
  2. Add a fresh `smelt build --dry-run` that mirrors `smelt run --dry-run` semantics but spans the seed→run lifecycle.
  Tracked in the smelt-loop findings plan as deferred until the direction is chosen.
- **TB-4 — `smelt --version` is not a recognised flag.** The clap root command does not declare `--version`, so `smelt --version` errors with `unexpected argument`. Trivial fix tracked in `docs/plans/20260502-smelt-loop-findings.md` Phase 5.
- **`--select` whitespace handling is unspecified.** When a user writes `--select "a b"`, the literal `"a b"` becomes one selector that will not match any model — the call silently produces an empty selection. Whether this should be a hard error or a warning is open; the implementation today is silent. Recorded so a future plan can choose deliberately.

## References

- **Code**: `crates/smelt-cli/src/main.rs` (clap definitions), `crates/smelt-cli/src/commands/build.rs` (`--show-plan` dispatch), `crates/smelt-cli/src/commands/seed.rs`, `crates/smelt-cli/src/seed.rs`
- **Tests**: `crates/smelt-cli/tests/` (CLI integration tests)
- **User docs**: `docs-site/docs/reference/cli.md` — the canonical per-flag reference; this spec is the rulebook the reference must agree with.
- **Plans (history)**: `docs/plans/20260502-smelt-loop-findings.md` — the source plan for this stub; Phases 4 and 5 close TB-1 and TB-4, Phase 6 reconciles the user-facing reference.
- **Related specs**:
  - `architecture.md` — pipeline stages the CLI orchestrates.
  - `incremental_models.md` — `--event-time-start` / `--event-time-end` semantics, batch safety classification, `backbuild` behaviour.
  - `functions.md` — `smelt build` plans function expansion as part of the build lifecycle.
  - `smelt_yml.md` — `targets:`, `model_paths:`, `seed_paths:` keys consumed by the CLI.
