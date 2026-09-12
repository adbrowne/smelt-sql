# Phase 6f plan — `LAG`/`LEAD` window frames elided on SparkSQL/Databricks

## Objective

`silver.actor_sessions` is the last model failing the live full refresh: its `LAG(...)` calls
carry `RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW` and Spark refuses any frame on
`lag`/`lead` (`Cannot specify window frame for lag function`), skipping three dependents. Make the
frame *elided at emission* on the SparkSQL dialect through registry data — not a printer name
match — then re-run the full refresh to a clean 16/16. Advances criterion 6 (the whole model set
full-refreshes on Databricks) and unblocks criteria 7, 8 and row 7, which are each stated over
"the same model set".

**Why elision is semantics-preserving, not a narrowing.** Per the SQL standard `LAG`/`LEAD` are
offset functions that ignore the window frame, and DuckDB agrees — measured 2026-09-12:

```
rows at 2024-01-01, 2024-01-10, 2024-01-10 01:00 (one partition)
LAG(ts) OVER (… ORDER BY ts RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW)
  → 2024-01-01 at the 2024-01-10 row — nine days back, i.e. outside the frame
identical to the unframed LAG on every row.
```

So DuckDB and a frameless Spark `lag` compute the same thing; nothing is lost for criterion 7.
The frames stay in the **source** (they are `sessionize.sql`'s load-bearing `max_lookback`
declaration the planner derives the bound from) — only the printed SparkSQL drops them.

## Spec delta

`docs/specs/multi_backend.md` §"Operator lowering" (beside "Statement-level lowering"): a new
short subsection **"Frame elision on offset functions"** — an offset built-in that the SQL
standard defines to ignore its window frame may be declared, per dialect and per position, to
emit with the frame removed; the elision is planned from the source CST before printing, never
recovered from printed SQL, and the source frame is untouched so a derived lookback bound is
unaffected. Name `LAG`/`LEAD` on SparkSQL as the entry this exists for.

## Tests (red first)

- `smelt-types` `registry_coverage`: `lag_and_lead_elide_frame_on_sparksql` — the `LAG`/`LEAD`
  rows carry `Emission::Rewrite(RewriteId::ElideWindowFrame)` at `(DialectId::SparkSql,
  Position::Window)` and stay `Native` on DuckDB/BigQuery.
- `smelt-dialect` new `frame_elision` test file:
  - `lag_prints_without_frame_on_spark` — compiles the `sessionize` `LAG(...) OVER (… RANGE
    BETWEEN INTERVAL '2 days' PRECEDING …)` shape; SparkSQL output has the `PARTITION BY`/
    `ORDER BY` intact and no `RANGE BETWEEN`; DuckDB output is byte-identical to the source.
  - `frame_on_a_non_offset_window_fn_is_kept_on_spark` — the same frame on `MAX(...)` is
    untouched on every dialect (the elision is registry-scoped, not blanket).
  - `named_window_frame_is_elided_too` or, if the grammar's `NAMED_WINDOW` path makes that
    ill-defined for a shared window, an explicit test that a named-window `LAG` is *refused*
    rather than silently emitted with the frame.
- `smelt-dialect` `emission_ownership` — unchanged gate, must stay green: the elision touches no
  `WINDOW_SPEC` inside `printer/`, and `ElideWindowFrame` is dispatched.
- `smelt-db` `dialect_audit`: the `LAG`/`LEAD` SparkSQL probes (coverage totality names any
  registry entry with no probe) — schema leg on DuckDB in-process, Spark leg nightly.
- `smelt-db` `type_property_tests`-adjacent smoke (or a plain DuckDB unit test in the new file):
  `duckdb_lag_ignores_its_frame` — pins the measured fact above, so a DuckDB change that made
  the frame load-bearing would fail here rather than silently diverging on the parity leg.
- `smelt-cli --features databricks` `github_activity_databricks`:
  `actor_sessions_compiles_without_window_frame` — `--dry-run` against the `databricks` target,
  compiled SQL for `silver.actor_sessions` contains no `RANGE BETWEEN` on a `lag(` call.

## Tasks

1. Spec edit first: `docs/specs/multi_backend.md` §"Frame elision on offset functions".
2. `smelt-types/src/signatures/emission/position_rewrite.rs`: add `RewriteId::ElideWindowFrame`
   with a doc comment stating the standard-ignores-frame justification and the DuckDB measurement.
3. `smelt-types/src/signatures/builtins/window.rs`: declare the verdict on `LAG` and `LEAD` for
   SparkSQL at `Position::Window`; leave every other dialect `Native`.
4. New `crates/smelt-dialect/src/frame_elision.rs` (sibling of `restructure.rs`, **outside**
   `printer/`): pure `plan(root, registry-resolved verdicts, dialect) -> Vec<TextRange>` over the
   source CST, returning the `WINDOW_FRAME` ranges to drop — the only place that knows the
   `WINDOW_SPEC`/`WINDOW_FRAME` node shapes.
5. `PrintContext` gains `elided_frames: &'a [TextRange]`, mirroring `settled_emissions`; the
   printer skips a node whose range is listed (a range lookup — no kind match, no name, no
   dialect arm) and emits its trailing trivia so the `)` does not glue.
6. Dispatch `RewriteId::ElideWindowFrame` in `registry_emit`'s match: the *call* prints natively
   (the verdict addresses the window, not the call text), with a comment saying so.
7. Wire the pass at every compile entry point that builds a `PrintContext`, beside the existing
   `restructure::plan` call, so `resolve_refs_in_sql`-style callers with no plan pass `&[]`.
8. Register the `LAG`/`LEAD` SparkSQL probes in the dialect-audit probe set; regenerate
   `docs/reference/dialect-coverage.md` (doc-sync gate).
9. Live: re-run the full refresh and record the result — a clean 16/16, or the next construct at
   the same failure point, recorded not fixed.
10. Write `phases/06f-summary.md`; flip row 6f to `done`.

## Verification

- `bash .claude/scripts/verify-phase.sh` (run the five gates individually if the bundle exceeds
  the foreground budget, as phase 6e did).
- `cargo test -p smelt-types --test registry_coverage`,
  `cargo test -p smelt-dialect --test emission_ownership --test template_emission --test frame_elision`,
  `cargo test -p smelt-db --test dialect_audit`,
  `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance`,
  `cargo test -p smelt-cli --features databricks --test github_activity_databricks`.
- Live: `source scripts/dbx-dogfood-env.sh` then `smelt run --target databricks --full-refresh
  --allow-full-refresh --event-time-start 2026-08-05 --event-time-end 2026-08-07`; capture the
  run id and the success/failed/skipped counts. If the workspace is unreachable, emit
  `<<PHASE_BLOCKED>>` — never skip green.

## Commit message

`outcome(databricks-dogfood-spine): phase 6f elides LAG/LEAD window frames on SparkSQL`
