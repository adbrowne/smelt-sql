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
