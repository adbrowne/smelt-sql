# Phase 3b — Typed ANSI partition literals from the single `partition_literal` owner

## Objective

Close gap 1 of phase 3's block report: a calendar-axis partition literal renders as a typed
ANSI literal (`DATE '2026-01-01'` / `TIMESTAMP '2026-01-01 00:00:00'`) rather than a bare
quoted string, on **every** dialect, from the one owner `smelt_logical::maintenance::emit::
partition_literal`. This unblocks criterion 2 (the reachable families execute end-to-end on
Trino, whose `date <= varchar(10)` refusal is what surfaced it) and criterion 7 (the generative
gate runs calendar-axis recipes through the real pipeline), without any consumer acquiring a
dialect branch (criterion 6's shape).

Scope discipline: this phase changes *literal rendering only*. It does not touch axis
resolution (3a, done), the `ColumnScopedMerge` downgrade (3c), or any live Trino family proof
(3d).

## Spec delta (spec-first — the implement step makes this edit before code)

- `docs/specs/incremental_shapes.md` §"Validation rules" rule **8a**, the sentence beginning
  "A partition literal is rendered **in the axis's own domain** everywhere a run emits one":
  replace "quoted and escaped (`'2026-01-01'`) on the calendar axis" with the typed-ANSI rule —
  a calendar-axis literal renders as `DATE 'YYYY-MM-DD'` for a date-shaped value and
  `TIMESTAMP 'YYYY-MM-DD HH:MM:SS[.fff]'` for a timestamp-shaped one; the integer axis stays
  bare (`7`); a calendar value matching neither shape is a hard refusal, not a fallback to an
  untyped string. State the reason in one clause: the typed literal type-checks on a strict
  engine (Trino) and is accepted unchanged by DuckDB, Spark and BigQuery, so one spelling
  serves every dialect and no caller learns a dialect.
- No other spec section states the literal spelling; if the implementer finds one
  (`rg -n "quoted and escaped" docs/specs/`), it is edited in the same commit.

## Tests (red-green)

1. `smelt-logical/tests/emit_statements.rs::partition_literal_renders_typed_calendar_literals`
   — `Calendar, "2026-01-01"` → `DATE '2026-01-01'`; `Calendar, "2026-01-01 00:00:00"` and the
   `T`-separated / fractional-seconds forms → `TIMESTAMP '…'`; `Integer, "7"` → `7` unchanged.
2. `…::partition_literal_refuses_a_non_calendar_shaped_value` — replaces today's
   `"it's"` → `'it''s'` escaping assertion: a calendar value parsing as neither shape is `Err`,
   and the message names the offending value (fail-loud, matching the integer arm).
3. `…::region_predicate_uses_typed_calendar_literals` — `Region::for_axis(Calendar, …)`
   `.predicate(..)` renders `col >= DATE '…' AND col < DATE '…'`.
4. `smelt-runtime/tests/transformer_tests.rs` (or the existing home of the injection tests):
   `injected_source_filter_uses_typed_calendar_literal` and
   `output_clamp_uses_typed_calendar_literal` — the injected scan-window predicate and the
   `_smelt_output_clamp` predicate both carry `DATE '…'`.
5. `smelt-runtime/tests/partition_axis_windowing.rs::real_run_batch_sql_still_quotes_calendar_axis_bounds`
   — 3a's regression fence, retargeted to assert the typed form (it is the test that pins the
   calendar side of the axis fix and must not be left asserting the old spelling).
6. `smelt-cli/tests/trino_incremental_families.rs::calendar_axis_incremental_model_runs_on_trino`
   — live tier, mirroring 3a's integer-axis test: seed → backfill → steady-state re-run of a
   calendar-`partition_column` model through `smelt run --target trino`. This is the test that
   proves the gap is actually closed. **If `scripts/trino-env.sh` cannot reach the coordinator,
   emit `<<PHASE_BLOCKED>>` — never skip green.**

## Tasks

1. Make the spec edit above.
2. Rewrite `partition_literal`'s `Calendar` arm in `crates/smelt-logical/src/maintenance/emit/types.rs`
   as a shape classifier: date → `DATE '…'`, timestamp (space or `T` separator, optional
   fractional seconds, optional trailing `Z`) → `TIMESTAMP '…'`, otherwise `Err`. Parse with
   `chrono` (already a dependency) rather than ad hoc string slicing; keep the escaping of the
   inner text.
3. Update the doc comment on `partition_literal`, on `Region`, and on
   `smelt-backend/src/types.rs:94` to state the typed-literal rule.
4. Fix the two `unwrap_or_else` fallbacks in `crates/smelt-runtime/src/transformer.rs`
   (~140, ~149) so the unreachable-fallback spelling matches the new rule instead of
   reconstructing a bare quoted string.
5. Run `cargo test --workspace` and repair every fixture pinning the old spelling — these are
   test/spec/doc assertions over *rendered SQL*, not stored state (per the 2026-09-14 ruling's
   survey). Expected areas: `smelt-logical/tests/emit_statements.rs`,
   `smelt-runtime/tests/*`, `smelt-cli/tests/*` explain and dry-run snapshots,
   `smelt-backend-{duckdb,spark,bigquery}` sql tests.
6. Regenerate the web-analytics tutorial pages (`python3 examples/web_analytics/generate_tutorial.py`)
   and edit `examples/web_analytics/tutorial_pages/` templates — never the generated
   `docs-site/docs/examples/web-analytics/` output — until `tutorial_freshness` is green.
7. Sweep hand-written user docs for the old spelling
   (`docs-site/docs/guide/incremental-models.md`, `migrations.md`) and update the shown SQL.
8. **Escalation check (not an absorb):** if any site *persists* a partition literal into the
   ledger or a deployed-schema snapshot rather than rendering it into a predicate, stop and
   record it — the ruling's survey says none exists; a counter-example is an escalation.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, shellcheck, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test tutorial_freshness --test explain_snapshots` (doc-freshness
  and explain-output pins).
- `cargo test -p smelt-runtime --test partition_axis_windowing --test dry_run_statements --test execute_parity`.
- `cargo test -p smelt-logical --test emit_statements --test walk_coverage`.
- Live tier: `bash scripts/trino-up.sh && source scripts/trino-env.sh`, then
  `cargo test -p smelt-cli --test trino_incremental_families` and
  `cargo test -p smelt-backend-trino --test backend_live`; tear down with `scripts/trino-down.sh`.

## Commit message

`fix(partitions): render calendar partition literals as typed ANSI DATE/TIMESTAMP literals`
