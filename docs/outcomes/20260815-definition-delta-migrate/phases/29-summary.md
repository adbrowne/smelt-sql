# Phase 29 summary — key-grain frontmatter/CLI validation gaps

## Shipped

- `MetadataError::KeyedForbidsSafetyOverrides` / `DiagnosticCode::KeyedForbidsSafetyOverrides`
  (`crates/smelt-core/src/metadata.rs`, `crates/smelt-db/src/{lib.rs,diagnostics_types.rs}`,
  `crates/smelt-lsp/src/backend.rs`): a key-addressed model (resolved `grain: key`) declaring
  `safety_overrides:` now gets this dedicated, correctly-named refusal instead of
  `PartitionGrainRequiresRefreshIncremental`. Routed through `ModelMetadata::resolved_grain()`
  (not the literal `grain:` field), so a `timeseries:` + `refresh: incremental` model with no
  written `grain:` (an effective partition shape) is still admitted — `validate_timeseries`
  still requires `refresh: incremental` explicitly for both arms (a plain, non-incremental
  model with a folded `batched` block still gets the generic refusal).
- `crates/smelt-runtime/src/execute.rs`: the windowless window-forward keyed arm now refuses
  (`anyhow::bail!`, plain-`bail!` shape mirroring the pre-existing snapshot-reconcile arm) unless
  `request.full_refresh` is set, instead of silently drop+recreating from the whole-source
  SELECT. `--full-refresh` remains the intentional rebuild escape.
- `smelt build` gained a `--full-refresh` flag (`crates/smelt-cli/src/main.rs` `BuildArgs`,
  wired in `commands/build.rs`'s `run_build_with_checks`) — it had none before, so a
  windowless keyed model under `smelt build` had no escape at all once the refusal landed.
- Tests: `crates/smelt-core/tests/refresh_axis.rs` (3 new + 1 retargeted), `crates/smelt-core/src/{config,metadata}.rs` inline tests (2 retargeted), `crates/smelt-db/tests/maintenance_diagnostics.rs` (1 new, CLI+LSP parity via `file_diagnostics()`), `crates/smelt-runtime/tests/keyed_run_window_required.rs` (new file, 4 tests: windowless refusal, one-flag-alone refusal, `--full-refresh` rebuild, snapshot-reconcile regression guard).
- Spec: `docs/specs/incremental_shapes.md` (2 Known Divergences bullets deleted, `KeyedForbidsSafetyOverrides` named in §"Key-grain declaration" and the Surface diagnostics table), `docs/specs/incremental_models.md` §"Run flags" (states the refusal + `--full-refresh` escape), `docs/specs/diagnostics.md` (new row + rewritten retirement paragraph). docs-site: `reference/cumulative-aggregate.md` (rewrote the "falls back" sentence), `reference/cli.md` (`--full-refresh` documented on `smelt build`, including the flag truth-table).

## Decisions

- Kept `refresh: incremental` as an explicit precondition in `validate_timeseries` alongside
  the `resolved_grain()`-derived shape check — using `resolved_grain()` alone (as the plan's
  literal wording suggested) silently admitted a non-incremental model's folded
  `safety_overrides:` block, breaking two pre-existing tests
  (`test_batched_block_without_refresh_batched_errors`,
  `config::tests::test_validate_refresh_keyed_forbids_incremental_via_metadata` after retarget).
  See `finding 2` in the plan — the fix needed both "derived, not literal, grain" AND "still
  requires refresh: incremental", not one or the other.
- Added `--full-refresh` to `smelt build` rather than routing the flagship
  `examples/web_analytics` fixture around the refusal some other way — `smelt build` (seed +
  run everything from scratch) is exactly the "intentional whole-history rebuild" case the CLI
  spec already describes as `--full-refresh`'s purpose, and 12 of the crate's own
  `smelt build`-driving tests needed exactly this escape.

## For the next planner

- `crates/smelt-datagen/tests/example_web_analytics.rs`'s 12 `smelt build` call sites now all
  pass `--full-refresh` uniformly. That's coarser than necessary (it also full-refreshes every
  *other* selected model, not just the keyed one) but matches every other model's existing
  idempotent-rebuild behavior in this fixture, so no test assertion needed changing.
- `crates/smelt-runtime/tests/key_addressed_model_edge_lowering.rs`'s shared `select_request`
  helper and `crates/smelt-runtime/tests/statement_parity.rs`'s `multi_select` closure now
  default `full_refresh: true` (both only ever run the fixture's clocked keyed upstream
  unwindowed) — safe because `request.full_refresh` is consulted nowhere else these tests
  exercise (confirmed via `rg`), but worth knowing if a future test in either file adds a
  *windowed* call through the same helper expecting `full_refresh: false`.
- Not touched: `crates/smelt-cli/src/commands/build.rs`'s `build_include_upstreams` path
  (`--include-upstreams`/`--period`) still hardcodes `full_refresh: false` — backward
  resolution always supplies its own per-ancestor window, so it never reaches the new refusal;
  left as-is, out of this phase's scope.

## Gates

- `cargo test -p smelt-core --test refresh_axis --test config_refresh_axis` — pass
- `cargo test -p smelt-core --quiet` (full crate) — pass, including retargeted inline tests
- `cargo test -p smelt-db --test integration --test maintenance_diagnostics` — pass
- `cargo test -p smelt-db --quiet` (full crate) — pass
- `cargo test -p smelt-runtime --test keyed_run_window_required --test keyed_reprocessed_window_refusal --test execute_parity --test statement_parity` — pass
- `cargo test -p smelt-runtime --quiet` (full crate) — pass
- `cargo test -p smelt-cli --test maintenance_conformance --test example_diagnostics` — pass
- `cargo test -p smelt-cli --quiet` (full crate, incl. `e2e` 175 tests) — pass
- `cargo test -p smelt-datagen --test example_web_analytics` — pass (23 tests)
- `cargo test -p smelt-cli --test cli_docs_coverage` — pass
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full workspace `cargo test`, `example_diagnostics`)
- `rg -n "silently full-refreshes|safety_overrides:\` on a key-addressed model" docs/specs` — no hits
