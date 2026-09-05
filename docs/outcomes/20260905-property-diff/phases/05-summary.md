# Phase 5 summary — `smelt explain --diff`

Shipped: `smelt explain --diff [<ref>]` (text/JSON), `--fail-on {downgrade,any}`, `--select`
(narrows reported set only, Δ2), full clap exclusivity with `<model>`/`--show-sql`/`--period`/
`--technique` (exit 2). Spec deltas Δ1–Δ3 landed first. Rendering lives in a new pure
`smelt_logical::analysis::diff_render` (`change_line`/`reason_line`/`model_block`/`text_report`)
over a new `DiffReport`/`BaselineInfo` envelope whose `Serialize` is the JSON schema — no second
sort, no second message assembly, ready for Phase 6/7 reuse.

Carried items: **C1** — text report now renders corner/technique/refusals from the
`PropertyProfile` (new `build_model_profile` extraction shared with `build_model_diagnostics`),
not the raw plan. **C2** — an added/removed entry's `cause.reason` carries the derivation-failure
text via a pure, unit-tested `apply_failure_reasons` helper. **C3** — new standalone fixture
`crates/smelt-cli/tests/fixtures/state_downgrade/` gives `resolve_availability`/`state_downgrade`
a live producer. **C4** — `profiles_for_workspace` now resolves each model's dialect/availability
from its own target (`execute::sql_dialect_for_target`), verified by a new dual-target fixture
(`crates/smelt-runtime/tests/fixtures/dual_target_dialect/`) proving a Spark-targeted model shows
a `state_downgrade` a DuckDB-targeted twin does not. **C5** — two stale comments removed.

## Criterion-4 fixture — deviation from the plan (ruling R3)

The plan's literal `SUM(amount) → MAX(amount)` edit on `user_daily_spend` was verified BY HAND
against `examples/timeseries` and produces **zero shift anywhere** — not even in the edited model
itself (only a neutral `discriminant` metadata change). Root cause: the only combiner-sensitive
cell is a `NewData` fold over the append-only `raw.transactions` source, which never needs a
correction/`UpstreamMutation` cell, so `KeyedFold`'s forward-fold admission is insensitive to
invertibility. Confirmed with `MAX`, `AVG`, and `SUM(DISTINCT ...)` (all stay `KeyedFold`,
zero propagation); only a truly holistic combiner (`MEDIAN`) loses the cell outright, and even
then nothing propagates downstream.

The substituted edit — adding a join to the unclocked `raw.users` dimension inside
`user_daily_spend` — reproduces the actual phenomenon: `user_daily_spend` shows `cause.kind ==
"edited"` with a `cell_technique` downgrade (`KeyedFold → DeleteInsert`, via broken row identity/
`fan_out_join`), and its direct downstream **`user_spend_running_total`** (not `user_spend_rollup`,
which only passes `total_amount` through untouched) shows `cause.kind == "downstream"`,
`of == ["user_daily_spend"]`. This is `a_join_induced_downgrade_propagates_to_the_named_downstream_model`
in `crates/smelt-cli/tests/property_diff_cli.rs`, plus `--fail-on`/`--select`/JSON-schema tests
reusing the same edit. Recommend flagging in the outcome's Known Divergences or a follow-up issue
that `examples/timeseries` has no fixture demonstrating a pure-combiner-driven downgrade.

## Gate

fmt clean; clippy clean on both feature sets; `smelt-logical` (lib + tests), `smelt-runtime`
(`profile_workspace`), `smelt-cli` (`property_diff_cli`, `property_profile_parity`,
`explain_maintenance`, `explain_model`, `explain`, `explain_docs_freshness`,
`cli_docs_coverage`, `exit_codes`) all pass. `hardening_budget` baseline bumped by exactly one
legitimate `println!` (smelt-cli 174→175, the `--json` branch); zero new unwrap/expect (two
candidates converted to `?`/anyhow instead). Did not run `cargo test --workspace` or
`example_diagnostics` per instruction.

## For Phase 6 (Markdown)

Reuse `diff_render::change_line`/`model_block` for the table/`<details>` rows, and `DiffReport`
directly — do not re-sort `models` or re-derive `cause`/`changes` text. `--markdown` flag was
deliberately NOT added in Phase 5 (kept out of `ExplainArgs` to avoid an undocumented, unimplemented
flag tripping `cli_docs_coverage`); add it alongside the renderer.

## Fix round 1 (controller review)

**Q1 (critical, fixed).** `profiles_for_workspace` skipped any model with no maintenance plan,
so a `refresh: incremental` → `refresh: full` edit made the model vanish from the new map
entirely rather than appearing present-with-empty-cells — routing through `whole_model_changes`
(all-`Neutral`) instead of the matched-both-sides path that fires `maintenance_lost`. Fixed:
every model that classifies as a bare-SELECT `Model` (`smelt_core::resolver::classify`) now gets
a profile via `build_model_profile` with empty cells/refusals/probes when it has no maintenance
plan, or a recorded `failures` entry if even that fails. Scoped to `Model`-classified entities
only — `loaded.sql_files` is a project-wide walk that also carries `smelt.test`/`smelt.check`/
`smelt.define` declarations, none of which are diffable models. Proven end to end against the
real CLI: `losing_incremental_maintenance_reports_a_maintenance_lost_downgrade` in
`property_diff_cli.rs` flips `user_daily_spend`'s `refresh:` and asserts a `maintenance_lost`
downgrade plus `--fail-on downgrade` exiting `1`.

**Q2 (critical, fixed — and a REAL second bug found underneath).** The reviewer's `DELETE FROM …`
repro exposed that `apply_failure_reasons` had the two sides backwards: "added" (present in the
working tree, absent from the baseline) was reading `work_failures` instead of `base_failures`,
and "removed" was reading `base_failures` instead of `work_failures` — so a real derivation
failure never actually reached `cause.reason`, and the three unit tests passed because they
encoded the same swap. Fixed both the logic and the tests (which now assert the correct side);
added `a_body_that_no_longer_derives_a_profile_is_reported_removed_with_a_reason` in
`property_diff_cli.rs` using the reviewer's own repro.

**Q3 — confirmed true.** After Q1, "every model's property profile" in `smelt-explain.md`/
`cli.md` is accurate; no doc change needed.

**Q4 (fixed).** `--fail-on` now has `requires = "diff"`; `smelt explain --fail-on any` exits `2`.
Test: `fail_on_without_diff_is_a_usage_error`.

**Q5 (fixed).** `outcome.md` criterion 4 now names the actual edit (join to `raw.users`) and the
actual downstream model (`user_spend_running_total`), plus the `refresh: full` fixture. Added a
dated Decision-log entry explaining why a combiner swap alone never downgrades a `NewData`-fold
cell over an append-only source.

**Q6 (fixed).** `apply_failure_reasons` and `DiffReport::narrow_to` moved to
`smelt_logical::analysis::diff` (single-owned for Phase 6/7 reuse); `explain_diff.rs` now just
calls them. Their unit tests moved to `diff.rs`'s own test module.

**Q7 (fixed).** `change_line` no longer emits a double space for an empty `subject`
(`▼ maintenance_lost: true → false`, not `▼ maintenance_lost : …`).

**Unplanned fallout from Q1, found and fixed while proving it:** profiling every `Model`-classified
entity surfaced that `examples/timeseries`'s project-wide file walk includes `setup_sources.sql`
(a plain DDL script with no `smelt.` marker, so it default-classifies as `Model`) — a genuine,
expected `PropertySet::derive` failure for a non-analyzable file the classifier cannot distinguish
from a real model. It is symmetric on both sides of any diff (always fails identically), so it
never appears in a reported diff, but it did break `profile_workspace.rs`'s stale
"no per-model derivation failures" assertion, which predated Q1 and assumed the map only ever
touched maintained models. Rewrote that test to assert the real contract (every `Model`-classified
entity is in `profiles` or `failures`, with `setup_sources` allow-listed as the one known,
harmless, structurally-unavoidable exception) rather than relaxing or deleting it.

Gate re-run: fmt clean, clippy clean (both feature sets), `hardening_budget` green (no new
regressions this round), and every previously-listed suite plus the new tests all pass.
