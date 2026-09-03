# Phase 6 summary — per-source clamp observability

## Shipped

- `smelt_logical::resolve_scan_window` (`crates/smelt-logical/src/analysis/source_bounds.rs`) —
  the single shared resolver: `PartitionAxis` + `before`/`after` `Offset` → `ScanWindowVerdict`
  (`Resolved{start,end}` or `Unresolved{reason}`, fail-loud on `Offset::Symbolic`).
- `smelt-runtime::transformer::inject_source_filters` now calls the resolver instead of its own
  inline day arithmetic; `calendar_time_filter_is_byte_identical` and the full `statement_parity`
  suite stay green (byte-identical output).
- `smelt explain --json`'s `SourceBoundJson::Bounded` gained `scan_start`/`scan_end`/
  `scan_unresolved` (all `skip_serializing_if`), filled from the resolver when `--period` is
  given. `--period` on `ExplainArgs` no longer `requires = "show_sql"` — it now also drives the
  whole-project `--json` path's scan-window resolution.
- `smelt_db::model_source_clamps(db, workspace, file)` — thin Salsa wrapper (mirrors
  `ref_model_edge`'s resolution pattern) over `derive_model_bounds`, returning `file`'s own
  per-source `BoundResult` map. `BoundResult`/`Offset`/`Seconds` re-exported from `smelt-db`.
- `smelt-lsp::hover_text_for_source_clamp` (pure formatter) wired into the `smelt.models.*`
  hover branch in `backend.rs`: appends a one-line clamp readout under the existing schema
  table when `model_source_clamps` has a verdict for the hovered source.
- Spec: `docs/specs/incremental_shapes.md` §"Observing the per-source clamp" now names
  `scan_start`/`scan_end`/`scan_unresolved` and states the single-derivation guarantee; the
  stale "Per-source clamp observability is partly emitted" Known Divergences bullet is removed.
  `docs-site/docs/reference/smelt-explain.md` documents the new fields with an example.

## Decisions

- `--period`'s run window enters `explain --json` by relaxing the existing flag rather than
  adding a second `--event-time-start`/`--end` pair (plan decision, avoids a second axis parser).
- The scan-window resolver lives in `smelt-logical` beside `Offset`, not in `smelt-runtime` or
  `smelt-cli`, so the filter a run pushes down and the window a report prints can never drift
  (maintenance-plan purity).
- Calendar-axis arithmetic in the resolver uses `chrono::NaiveDate` (smelt-logical already
  depends on chrono) rather than duplicating `transformer.rs`'s Julian-day routine; the
  fail-open pass-through for a non-`YYYY-MM-DD` date string is preserved so calendar output
  stays byte-identical.

## For the next planner

- Phase 7 (`partition_column` rename refusal) and phase 8 (validate + close-out) are next per
  the outcome's phase table; no new residue surfaced by this phase.
- The `probe_explain_json_run_relative_source_bounds` probe was inverted into a positive
  assertion on resolved calendar dates; no other example fixture pinned the old
  `source_bounds` JSON shape (swept — only the two doc pages above referenced it, and neither
  needed a change beyond the new fields).

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-runtime --test statement_parity` — 33 passed.
- `cargo test -p smelt-cli --test rebuild_dry_run` — 4 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 75 passed.
- `cargo test -p smelt-cli --test partition_residue_probes --features duckdb probe_explain_json_run_relative_source_bounds` — passed (inverted).
- `cargo test -p smelt-lsp --test example_workspaces` — 35 passed.
