# Phase 1 summary — Diagnostics catalogue + spec cross-refs

## What shipped

- Added `PropertyDowngrade` (Warning) and `PropertyDiffBaselineUnavailable` (Error, CLI-only,
  exit 2) to `DiagnosticCode` in `crates/smelt-db/src/diagnostics_types.rs`, doc-commented with
  their `property_diff.md` triggers.
- Added both to the `docs/specs/diagnostics.md` catalogue (new "Property diff" section) and a
  "specified and unimplemented" Known Divergences entry pointing at this outcome.
- Added a "property diff" group mention + `smelt-explain.md` link to
  `docs-site/docs/reference/diagnostics.md`.
- Added one-liner cross-refs: `docs/specs/cli.md` (`smelt explain <model>` section, pointing
  `--diff` to `property_diff.md`) and `docs/specs/lsp.md` (`Code Lens` capability row +
  Diagnostics section note pointing `PropertyDowngrade` to `property_diff.md` §"Editor").
- `map_metadata_error_to_diagnostic` in `smelt-db/src/lib.rs` untouched (matches `MetadataError`,
  not `DiagnosticCode` — unaffected).

## Discovery for phase 2's planner

`crates/smelt-lsp/src/backend.rs::diagnostic_code_str` is a second exhaustive match over
`DiagnosticCode` (wire code-string mapping), separate from `map_metadata_error_to_diagnostic`.
Adding a `DiagnosticCode` variant requires an arm here too (`"property-downgrade"`,
`"property-diff-baseline-unavailable"`) or the workspace fails to compile. No wildcard arm —
exhaustiveness here is deliberate. Future `DiagnosticCode` additions in this outcome should grep
for `match code {` / `match diagnostic.code` in `smelt-lsp` before assuming only the catalogue
gate applies.

## Gate status

- `cargo test -p smelt-db --test integration diagnostics_catalogue`: red before the doc edit
  (missing both codes), green after.
- `bash .claude/scripts/verify-phase.sh`: see phase return message for full result.
