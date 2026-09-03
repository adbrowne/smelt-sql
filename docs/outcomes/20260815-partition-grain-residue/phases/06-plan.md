# Phase 6 plan — per-source clamp observability

## Objective

Finish success criterion 6: `smelt explain --json` resolves each bounded source's
run-relative scan window when a concrete run window is supplied, and editor hover on a
`smelt.<path>` reference in a partition-grain model shows the same derived clamp beside the
existing schema readout. Closes the "Per-source clamp observability is partly emitted"
Known Divergences bullet (whose "specified ahead of a tracking plan" claim is also stale —
`docs/plans/20260704-model-updates-l4-batched.md` BL8 tracks it).

## Spec delta (spec-first — the implement step makes these edits)

`docs/specs/incremental_shapes.md`:

1. §"Observing the per-source clamp" — name the resolved surface concretely: a `Bounded`
   entry carries `partition_col`, the symbolic `before`/`after` offsets, **and**, when a
   concrete run window is supplied (`smelt explain --json --period <start>..<end>`, bounds
   read in the model's own axis domain), a resolved `scan_start`/`scan_end` pair
   `[run_start − before, run_end + after)` rendered in that axis. An offset with a
   non-uniform unit (month/year) resolves to no window and reports `scan_unresolved`
   naming the unit rather than guessing a day count. State that the resolved window is the
   *same* derivation the run's pushdown filter uses, never a second arithmetic. State that
   editor hover on a `smelt.<path>` reference inside a partition-grain model renders the
   same per-source verdict (`Bounded`/`Unbounded`/`NotDerivable`).
2. §Known Divergences → "The partition grain" — remove the "Per-source clamp observability
   is partly emitted (Open Question)" bullet.

## Tests (red first)

- `resolve_scan_window_matches_injected_filter` (`smelt-runtime::transformer`) — the shared
  resolver returns exactly the `filter_start`/`filter_end` `inject_source_filters` pushes
  down, for a day-offset calendar bound and a zero-offset integer bound.
- `resolve_scan_window_refuses_symbolic_offset` (`smelt-logical` or `smelt-runtime`, wherever
  the resolver lands) — a `Offset::Symbolic("month")` bound yields the unresolved verdict
  naming the unit, never a coerced day count (fail-loud discipline).
- `calendar_time_filter_is_byte_identical` (`smelt-runtime::transformer`, existing) — must
  stay green after `inject_source_filters` is refactored onto the resolver.
- `explain_json_period_resolves_scan_window` (`smelt-cli/tests/explain_show_sql.rs`) —
  `explain --json --period 2026-01-01..2026-01-08` on a 3-day-lookback model emits
  `scan_start: "2025-12-29"` / `scan_end: "2026-01-08"`; without `--period`, neither field is
  present and `before`/`after` are unchanged ISO-8601 durations.
- `probe_explain_json_run_relative_source_bounds` (`smelt-cli/tests/partition_residue_probes.rs`)
  — **inverted** from its current "must NOT look resolved" assertion into a positive
  assertion on the resolved calendar dates.
- `model_source_clamps_derives_upstream_bounds` (`smelt-db`) — the new Salsa query returns a
  `Bounded` verdict naming the upstream's own `partition_column` and its derived offsets for
  a partition-grain model, and an empty map for a non-partition-grain model.
- `hover_text_for_source_clamp_renders_each_verdict` (`smelt-lsp`, pure formatter) —
  `Bounded(c, 3d, 0)`, `Unbounded`, and `NotDerivable` each render a distinct one-line
  readout; a source with no verdict renders nothing (`None`).

## Tasks

1. Extract the pushdown window arithmetic out of `inject_source_filters`
   (`crates/smelt-runtime/src/transformer.rs:95-135`) into one shared resolver keyed on
   `PartitionAxis` + a `before`/`after` `Offset` pair, returning either a resolved
   `(start, end)` string pair or an unresolved verdict naming the reason. Put it beside
   `Offset` in `crates/smelt-logical/src/analysis/source_bounds.rs` (single owner of offset
   semantics; `PartitionAxis` already lives in `smelt-logical` since phase 5a); have
   `inject_source_filters` call it, wrapping its `before_secs`/`after_secs` as
   `Offset::Seconds`. Calendar output must stay byte-identical (ceiling-to-whole-days
   preserved).
2. Relax `--period` on `ExplainArgs` (`crates/smelt-cli/src/main.rs:505`) so it no longer
   `requires = "show_sql"`; update its doc comment to describe it as the run window used
   both for `--show-sql` literals and `--json` scan-window resolution. `--show-sql`'s
   behaviour is unchanged.
3. Add `scan_start`/`scan_end` (both `skip_serializing_if = "Option::is_none"`) and
   `scan_unresolved` to `SourceBoundJson::Bounded` (`crates/smelt-cli/src/explain.rs:70`);
   thread the parsed `--period` window + its `axis_implied_by_literal_form` axis into
   `compute_source_bounds` and fill them via the task-1 resolver. Absent `--period`, the
   JSON is byte-identical to today.
4. Add a Salsa query `model_source_clamps(db, workspace, file)` to `smelt-db` — a thin
   wrapper (Salsa purity rule) that builds the `BoundContext` from the model's own
   `smelt.<path>` refs (resolve each via `resolve_ref_path`, take the upstream's
   `timeseries.partition_column` + `defining_expr_siblings`, mirroring `ref_model_edge`'s
   pattern at `crates/smelt-db/src/lib.rs:1630`) and calls the pure `derive_model_bounds`.
   Returns an empty map when the hovered file's own model is not partition-grain.
   Re-export `BoundResult`/`Offset` from `smelt-db` so `smelt-lsp` needs no new dependency.
5. Add the pure formatter `hover_text_for_source_clamp(source_name, &BoundResult) ->
   Option<String>` to `crates/smelt-lsp/src/hover.rs` and export it from `lib.rs`.
6. Wire it into the `smelt.models.*` path-ref hover branch
   (`crates/smelt-lsp/src/backend.rs:3720-3800`): append the clamp line under the existing
   schema table when the query returns a verdict for that source.
7. Make the spec-delta edits; invert the phase-6 probe; sweep `examples/` for any doc or
   golden output that pins the old `source_bounds` JSON shape.
8. Write `phases/06-summary.md` (shipped / decisions / for the next planner / gates).

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt + clippy both feature sets + full test +
  example_diagnostics).
- `cargo test -p smelt-runtime --test statement_parity` — the injected pushdown filter must
  not shift by one byte.
- `cargo test -p smelt-cli --test rebuild_dry_run` and
  `cargo test -p smelt-cli --test maintenance_conformance` — the shared resolver is on the
  live run path.
- `cargo test -p smelt-cli --test partition_residue_probes probe_explain_json_run_relative_source_bounds`
  — green after inversion.
- `cargo test -p smelt-lsp --test example_workspaces` — hover wiring must not regress
  example diagnostics.

## Commit message

`feat(observability): run-relative scan window in explain --json and editor hover clamp`
