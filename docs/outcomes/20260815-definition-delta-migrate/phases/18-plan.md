# Phase 18 — Consume the declared guardrail/preference config

## Objective

Two pieces of declared `maintenance:` surface parse today but never reach a consumer:
`scan_bounds.on_violation: warn` (every guardrail violation is an Error) and the
`prefer`/`technique` choice ladder on the ordinary region path
(`resolve_incremental_strategy` reads `cell.technique` directly, bypassing
`resolve_cell_choice`). Wire both, and state the absent-cost-model fallback preference order in
the spec. Advances success criterion 15 (three of the five "Plan-consumer gaps" clauses) and
criterion 20 (the bullet is narrowed in `incremental_models.md`, not just fixed in code).

## Spec delta (made first, by the implement step)

- `docs/specs/incremental_models.md` §"Partition-local maintenance (the K8 guardrail)": state
  that `on_violation: warn` **admits** the derived plan and reports the violation as a Warning
  (`MaintenanceScanUnbounded`, Warning severity); `error` (the default) refuses. The guardrail
  stays check-only either way — it never modifies a derived clamp.
- `docs/specs/incremental_models.md` §Design: add "Absent a cost model: the fixed preference
  order" — validated `cells[].write` pin > hard `cells[].technique` pin (refuses loudly when
  the resolvable set does not contain it) > soft `prefer` > the cell's own admitted-and-live
  technique > region recompute. Applies uniformly to every dispatch route, including the
  ordinary windowed/partition-grain region path.
- `docs/specs/incremental_models.md` §Future Extensions: the cost model between two admissible
  techniques (moved out of §Known Divergences).
- `docs/specs/incremental_models.md` §Known Divergences, "**Plan-consumer gaps**": delete the
  `prefer`/`scan_bounds.on_violation` clause and the "cost model … is unbuilt" clause; the
  bullet retains only the horizon-clamped-quadrant, mutation-vs-rederivation, and `AppendOnly`
  clauses (phase 19's scope).
- `docs-site/docs/` — the page documenting `maintenance.scan_bounds` and the one documenting
  `maintenance.defaults.prefer`: one sentence each for the warn semantics and the preference
  order.

## Tests (red-green)

`crates/smelt-db/tests/maintenance_diagnostics.rs`
1. `scan_bounds_on_violation_warn_admits_and_warns` — a model whose scan cannot be
   partition-bounded, with `on_violation: warn`, emits exactly one `MaintenanceScanUnbounded`
   at `DiagnosticSeverity::Warning` and the plan still admits a creation cell.
2. `scan_bounds_on_violation_error_still_refuses` — the same fixture with the field absent (and
   with `error` explicit) still emits the Error and refuses. Guards the default.
3. `scan_bounds_warn_is_per_model_over_project` — a project-level `on_violation: error` with a
   model-level `warn` resolves to Warn (narrower wins, mirroring `require`).

`crates/smelt-runtime/tests/region_choice_ladder.rs` (new)
4. `region_path_honours_prefer_recompute` — a partition-grain model whose creation cell admits
   `DeleteInsert`, with `defaults.prefer: recompute`, resolves to `backend_default` rather than
   `IncrementalStrategy::DeleteInsert`.
5. `region_path_refuses_unadmitted_technique_pin` — the same model with `cells[].technique:
   fold` (unadmitted here) returns `Err` naming the resolvable set, not a silent `DeleteInsert`.
6. `region_path_unchanged_without_overrides` — no `maintenance:` overrides declared: byte-for-
   byte the pre-phase verdict for both the model-edge-driven and the first-`NewData` branches.

## Tasks

1. Spec delta above (all four `incremental_models.md` edits + the two docs-site sentences).
2. `smelt-db/src/queries/maintenance.rs`: make `effective_scan_bounds` return
   `(allow_full_scan, require, ScanBoundsViolation)`, resolving `on_violation` with the same
   narrower-wins ladder as `require` (default `Error`).
3. At the `maintenance_plan` source-assembly call site (~line 804): when the resolved severity
   is `Warn` and the source was not otherwise accepted, pass `allow_full_scan: true` into
   `source_facts` and record the source name in a new `scan_bounds_warnings: Vec<String>` field
   on `MaintenancePlanDiagnostics`.
4. `smelt-db/src/lib.rs`: after the existing refusal loop, emit one
   `MaintenanceScanUnbounded` Diagnostic at `Warning` severity per `scan_bounds_warnings` entry,
   anchored at `body_start`, message naming the source and `scan_bounds.on_violation: warn`.
5. `smelt-runtime/src/maintenance_driver.rs::resolve_incremental_strategy`: replace both
   direct `cell.technique` reads with `resolve_cell_choice(cell, &trigger,
   &effective_override(...), None, backend_supports_column_scoped_merge)`; map
   `ChosenTechnique::Admitted(Technique::DeleteInsert)` → `IncrementalStrategy::DeleteInsert`,
   `Admitted(_)` and `RegionRecompute` → `backend_default` (preserving today's verdict when no
   override is declared), and propagate `ChoiceRefusal` as an `anyhow` error the way
   `resolve_live_column_scoped_cell` already does. Thread the backend's
   column-scoped-MERGE capability from the existing call site in `execute.rs` rather than
   hardcoding it.
6. Update the doc comments on `effective_scan_bounds` (drop "not yet consumed here") and on
   `resolve_incremental_strategy` (name the ladder it now consults).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-db --test maintenance_diagnostics`
- `cargo test -p smelt-runtime --test region_choice_ladder --test statement_parity --test execute_parity`
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance --test maintenance_pins --test example_diagnostics`
- `cargo test -p smelt-lsp --test example_workspaces`

## Commit message

`feat(maintenance): consume scan_bounds.on_violation: warn and the prefer/technique ladder on the region path`
