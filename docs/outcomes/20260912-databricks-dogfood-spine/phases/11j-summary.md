# Phase 11j summary

## Shipped

- `smelt_state::intervals::seed_interval(project_dir, target, model_name, model_hash, start, end)`
  (`crates/smelt-state/src/intervals.rs`) — a thin wrapper over `FileStore::new` (permissive
  posture, gated by the same `Intervals` artifact check a real run's write goes through) +
  `IntervalStore::get_or_create(...).record_interval(...)` + `save_intervals`. Refuses when
  `start >= end`.
- `smelt state seed-interval --target <t> --model <m> --start <YYYY-MM-DD> --end <YYYY-MM-DD>`
  CLI subcommand (`crates/smelt-cli/src/commands/state.rs`, wired into `main.rs`'s new
  `Commands::State { command: StateCommands }` / `StateCommands::SeedInterval`). Compiles the
  project through the sanctioned `CompilerRegistry::compile_with_sql_and_ephemerals` path (no
  backend connection), hashes the compiled SQL via `compute_model_hash`, and calls
  `seed_interval`. Refuses with a named error if `--model` isn't in the project.
- `docs-site/docs/reference/cli.md` `## smelt state` / `### smelt state seed-interval` section.

## Decisions

- Reused the plan's own downplay of hash exactness: the CLI subcommand still derives the hash
  from the model's real compiled SQL (not a placeholder) since `CompilerRegistry` was already
  the sanctioned no-backend compile path other offline commands (`explain --show-sql`, `check`)
  use — no harder to do right than a placeholder would have been.
- Ephemeral-model handling mirrors `check.rs`'s pattern (collect ephemeral models in exec order,
  build one `EphemeralResolver`) rather than wiring `UpstreamSchemas` — unnecessary for a hash
  that only needs to be "reasonably current," and keeps the tool's dependency surface small.
- New `commands/state.rs` module (not reusing `commands/seed.rs`, which is the unrelated "load
  seed CSV files" command) and a new top-level `State` subcommand group, following the existing
  `Docs { command }` nested-subcommand precedent in `main.rs`.

## For the next planner

- 11k (queued next) can now: query the schema's real ingestion frontier, run `seed-interval`
  once per model against `databricks_job`, `databricks fs cp` the resulting `intervals.json` to
  the Volume, then resume 11i's live legs.
- The `--auto` cloud-target reconciliation question (should `--auto` ever read backend-resident
  state instead of only `.smelt/`?) stays open — tracked in `docs/specs/run_state.md` §"Known
  Divergences / Open Questions" and not reopened here per the plan's "Decided here" note.
- Housekeeping: bumped `.claude/hardening-baseline.txt`'s `smelt-cli println` ratchet 188 → 189
  (one new success-message `println!` in `state.rs`, same pattern as `smelt init`), with a
  dated sign-off note; no other ratchet moved.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-state --quiet` — all green (20 tests across the crate's suites, 13 in
  `intervals::tests` including the 5 new ones).
- `cargo test -p smelt-cli --test databricks_bundle --quiet` — all green (26 tests, including
  the 2 new `seed_interval_*` tests).
- No ratchet lowered; `smelt-cli println` baseline raised 188 → 189 with a sign-off note (see
  Decisions).
