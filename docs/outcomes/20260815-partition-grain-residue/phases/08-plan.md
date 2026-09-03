# Phase 8 plan — Validate + close out

## Objective

Run `/smelt:validate incremental_shapes` end-to-end and repair every drift item it surfaces that
belongs to a residue this outcome closed (phases 2–7), so success criterion 8 is met on evidence
rather than assertion. Refresh the spec's §References for the partition grain (the code, tests and
user-doc paths phases 2–7 created are absent from it) and pin the closed residues with a ratchet so
a future edit cannot quietly re-open a bullet this outcome removed.

## Spec delta

No behaviour change, so no §Surface/§Semantics edit. Two **status-only** edits to
`docs/specs/incremental_shapes.md`:

1. §References → "The partition grain" → **Code / Tests / User docs**: add the paths phases 2–7
   landed — `smelt-logical/src/analysis/temporal.rs` (subquery/CTE descent),
   `smelt-logical/src/{windowing axis types, resolve_scan_window}`,
   `smelt-runtime/src/windowing.rs` (`PartitionAxis`/`PartitionPoint`),
   `smelt-logical/src/maintenance/derive.rs` (`partition_column_changed`),
   `smelt-state/src/schema_tracking.rs` (`DeployedSchema::partition_column`),
   `smelt-db/src/lib.rs` (`model_source_clamps`) — plus the tests
   (`crates/smelt-cli/tests/partition_residue_probes.rs`,
   `crates/smelt-logical/tests/partition_residue_probes.rs`) and the user-doc pages
   `docs-site/docs/reference/smelt-explain.md`, `docs-site/docs/reference/timeseries.md`.
2. §References → "The partition grain" → **Plans (history)**: add
   `docs/outcomes/20260815-partition-grain-residue/outcome.md` (same posture the key-grain
   divergences already use for `docs/outcomes/20260809-rung2-state-shapes`).

Anything else the validate run flags as *behaviour* drift is a finding to report in the summary,
not a licence to widen this phase — fix it only if it is one of the seven closed residues.

## Tests

- `crates/smelt-cli/tests/partition_residue_probes.rs::partition_grain_residues_stay_closed` —
  new **ratchet** (green on arrival, red on regression): parses §"The partition grain" Known
  Divergences out of `docs/specs/incremental_shapes.md` and asserts its bullet set is exactly the
  six this outcome does **not** own (per-column `data_latency`, non-deterministic
  row-set-membership, schema-evolution-is-a-definition-delta, `PartitionGrainForbidsMetrics`,
  sub-`g_part` suggestion, `NOW()`/`CURRENT_*` pinning), failing with the offending bullet's text
  if any other bullet appears. Red first by construction: write the assertion against a
  deliberately-wrong expected set, watch it fail naming the real bullets, then correct it.
- Re-run, unchanged, the five surviving inverted probes as the close-out oracle — they are the
  per-criterion evidence, not new work.

## Tasks

1. Execute the `/smelt:validate incremental_shapes` process (steps 1–6 of
   `.claude/commands/smelt/validate.md`) in this session; write the drift report to
   `docs/validations/2026-09-04-incremental_shapes.md`.
2. Triage every drift item into: (a) belongs to a residue phases 2–7 closed → fix now;
   (b) belongs to one of the six unowned bullets or another grain → record in the summary as a
   finding, leave alone.
3. Apply the two §References edits above; bump `last_reviewed` only if the file's content changes.
4. Verify the timeless-oracle check on the spec body and on the partition-grain user-doc pages
   (`grep -nE "Phase [A-Z0-9]+"`) — fix any leak in a page phases 2–7 touched.
5. Check `docs-site/docs/guide/editor-features.md` documents the phase-6 clamp readout on
   `smelt.<path>` hover; add it if missing (that page is the hover surface's user doc).
6. Write the ratchet test (red-first per above), then correct its expected set.
7. Re-run the five inverted probes plus the outcome's standing gates; record each verdict verbatim
   in the summary.
8. Write `phases/08-summary.md`: per-success-criterion evidence (criterion → the probe/test/gate
   that proves it), the drift report path, and every item triaged into bucket (b).

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be ALL GREEN.
- `cargo test -p smelt-cli --test partition_residue_probes --features duckdb`
- `cargo test -p smelt-logical --test partition_residue_probes`
- `cargo test -p smelt-runtime --test statement_parity`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-db --test integration diagnostics_catalogue`
- `cargo test -p smelt-cli --test example_diagnostics`

## Commit message

`docs(incremental_shapes): validate partition-grain residue close-out and pin the closed bullets`
