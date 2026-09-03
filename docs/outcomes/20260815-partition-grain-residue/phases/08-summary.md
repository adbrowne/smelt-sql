# Phase 8 summary — Validate + close out

## Shipped

- `docs/validations/2026-09-04-incremental_shapes.md` — scoped drift report (partition-grain
  residues only; key-grain half deferred to its own blocked outcome).
- Two drift items found and fixed: §References → "The partition grain" was stale (missing
  every code/test/doc path phases 2–7 landed, and the outcome's own plan link); the phase-7
  diagnostic `MaintenancePartitionColumnChanged` was missing from the spec's own partition-grain
  codes table despite `diagnostics.md` naming this spec as owner.
- `docs-site/docs/guide/editor-features.md` gained a paragraph documenting the phase-6
  editor-hover clamp readout (previously shipped, never documented — task 5 of the plan).
- New ratchet: `crates/smelt-cli/tests/partition_residue_probes.rs::partition_grain_residues_stay_closed`
  — parses §"The partition grain" Known Divergences out of the spec, asserts the bullet set is
  exactly the six this outcome does not own. Verified red-first (empty expected set → failed
  naming all six real bullets), then corrected to green.

## Decisions

- Scoped the validate run to the partition grain only, per the phase plan — the key-grain half
  of `incremental_shapes.md` is owned by a separate, currently-`blocked` outcome
  (`docs/outcomes/20260815-keyed-grain-residue`) and running `/smelt:validate` over it would
  produce findings this phase has no mandate to fix.
- Fixed the `MaintenancePartitionColumnChanged` table gap rather than only recording it as a
  finding: it is drift squarely inside a residue phase 7 closed (the diagnostic phase 7 itself
  introduced), so it falls in the plan's bucket (a) — fix now, not bucket (b).
- Confirmed the phase-7 forward-note about a smoother `MaintenancePartitionColumnChanged` remedy
  (something other than deleting the snapshot file) is new scope beyond every success criterion;
  left unactioned per the phase plan, recorded below.

## For the next planner

- No residue-owned work remains open. Success criterion 8 (`/smelt:validate incremental_shapes`
  clean, standing gates green, restricted to this outcome's scope) is met.
- Not actioned (out of scope, recorded as findings, not new phases): a smoother remedy path for
  `MaintenancePartitionColumnChanged` than manually deleting the deployed-schema snapshot file;
  the `CASE`-nested-window gap in `temporal.rs`'s AST walk (advisory-only, matches no residue).
- The six remaining partition-grain Known Divergences bullets are confirmed correctly excluded
  from this outcome (phase-1 audit stands) and are now ratcheted — a future edit reintroducing
  a closed residue's bullet, or dropping/reordering one of the six live ones without updating
  the test, will fail loudly.
- This outcome (`20260815-partition-grain-residue`) is now fully `done`. The backlog's next
  entry after it (`20260815-keyed-grain-residue`) is `blocked`; the loop will fall through to
  `20260815-incremental-spec-closure-confirm`.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test partition_residue_probes --features duckdb` — 4 passed
  (including the new ratchet).
- `cargo test -p smelt-logical --test partition_residue_probes` — 2 passed.
- `cargo test -p smelt-runtime --test statement_parity` — 33 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 75 passed.
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed.
- `cargo test -p smelt-db --test integration diagnostics_catalogue` — 1 passed.
- `cargo test -p smelt-cli --test example_diagnostics` — 119 passed, 1 ignored.
