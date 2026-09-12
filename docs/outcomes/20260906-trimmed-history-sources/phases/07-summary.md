# Phase 07 summary — Keyed-grain coverage for the run-time retention re-evaluation

## Shipped

- `derive_model_retention_plan` (`crates/smelt-runtime/src/execute/retention_admission.rs`)
  now resolves `driving_source_granularity` from the model's own declared source refs
  (`single_clocked_granularity` over each ref's `timeseries.granularity`) instead of hardcoding
  `None`. This mirrors `smelt-db/src/maintenance_refs/plan.rs`'s unconditional candidate-pool
  form — the diagnostics path this derivation must agree with — not `clamp_locality.rs`'s
  `grain: key`-gated form.
- Corrected the function's doc comment, which previously asserted
  `driving_source_granularity` never reaches the retention fold — false, and the reason the
  gap shipped (task 1).
- 3 new unit tests in `retention_admission.rs`'s own test module, staged through
  `ModelDiscovery`/`discover_source_infos` over a tempdir (no backend): a `grain: key` +
  `timeseries:` route-1 model now reaches the retention fold with no
  `Refusal::LocalityNotEstablished`; two differently-granular clocked sources still leave
  resolution `None` (the shared "exactly one else undecided" rule, not a bespoke pick); a
  `grain: partition` model's derivation is byte-for-byte unchanged (regression pin).
- 2 new integration tests in `crates/smelt-runtime/tests/retention_admission.rs`, extending
  the existing DuckDB-backed fixture with a `keyed_agg` model (route 1, `grain: key` +
  `timeseries:`, GROUP BY over the same 45-day-retention `sources.events`): an aged backfill
  window refuses before any statement executes; a near-today window still succeeds.

## Decisions

- **No lookback construct in the keyed integration fixture.** `grain: key` forbids window
  functions (`KeyedForbidsWindowFunctions`) and a self-join of its own driving source
  (`KeyedMultipleDrivingSources` — v1 supports exactly one driving source per model), which
  eliminates both constructs a bounded nonzero reach could come from for a keyed model today.
  Used the identity case instead (`required_lookback: 0`, mirroring
  `locality_route1_slice_pruning.rs`'s fixture) and relied on `retention_refusals_at_age` aging
  `required_lookback` by the run's own window age alone — a window old enough past the retained
  bound still refuses even with zero authoring-time lookback. This is a real instance of the
  rolling re-evaluation, not a weakened proxy for it.
- **Composed-upstream candidate pool (task 5): confirmed unreachable at this call site, left
  uncovered, documented for phase 8.** `smelt-db`'s diagnostics path additionally folds
  composed-upstream-model granularities into the candidate pool for `grain: key` models
  (`ref_model_source_facts`, gated to `resolved_grain == Key`), which needs Salsa `db`/
  `workspace` access. `derive_model_retention_plan` only has `model_file` + `source_infos` (no
  db) — a ref to an upstream *model* (not a declared source) never matches anything in
  `source_infos`, so it's silently absent from `source_refs` and contributes no candidate here,
  regardless of grain. This is the same silent-skip class this phase closed for declared
  sources, now open for composed-upstream ones; not fixed here (task 5's own "cheap or record"
  branch — plumbing db/workspace access into this call site is not cheap).

## For the next planner

- **Phase 8 should add a row for the composed-upstream-granularity gap** described above: a
  `grain: key` model driven by an upstream model's own composed output (not a declared source)
  still resolves `driving_source_granularity: None` at this run-time call site even when the
  diagnostics path would resolve it via `model_source_granularities`. Reachable today only when
  a keyed model's SOLE clocked candidate is a composed upstream (no declared clocked source) —
  narrow, but real, and it's the same silent-under-read shape this outcome exists to close.
- No spec change was needed (delta was "None" as anticipated) — confirmed no doc/spec wording
  scoped the rolling re-evaluation to partition grain.
- `.claude/large-file-baseline.txt` untouched — no touched file regressed.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-runtime --test retention_admission --test retention_full_refresh --test availability_seam --test execute_parity --test statement_parity` — 6 + 4 + 5 + 4 + 41 passed.
- `cargo test -p smelt-logical --test walk_coverage` — 14 passed.
- `cargo test -p smelt-cli --test example_diagnostics` — 128 passed, 1 ignored.
- `bash .claude/scripts/large-file-check.sh` — OK.
- Manually verified RED-before-fix for both the unit test
  (`keyed_model_with_timeseries_reaches_the_retention_fold`) and the integration test
  (`a_keyed_model_backfill_older_than_retention_refuses`) by temporarily reverting
  `driving_source_granularity` to `None` and re-running — both failed with exactly the
  predicted symptom (`Refusal::LocalityNotEstablished` / a silent successful run), then restored.
