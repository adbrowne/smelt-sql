# Phase 10 summary — Validate and close

## Shipped

- `crates/smelt-cli/tests/succession_docs_freshness.rs` — three standing drift gates: every
  path cited in `incremental_shapes.md` §References §"The succession grain" exists on disk, the
  three stale "unbuilt" divergence bullets and the diagnostics.md "unimplemented" bullet are
  gone, and both status-table rows (`model_properties.md`, `model_transforms.md`) read `built`.
- `docs/specs/incremental_shapes.md` — §Known Divergences "The succession grain" rewritten to
  the one genuine residual (no non-DuckDB ledger builder) plus the pre-existing `__tombstones`
  collision bullet; §References gains Code/Tests bullets naming every landed file and a
  Plans (history) link to this outcome.
- `docs/specs/model_properties.md`, `docs/specs/model_transforms.md` — status cells `not-yet`/
  `unbuilt` → `built`; `model_properties.md` §References gains the classifier + its unit tests.
- `docs/specs/diagnostics.md` — the "twelve succession codes … unimplemented" bullet deleted.
- `docs/specs/sources.md` — the "declared profiles license almost nothing" divergence now names
  the succession grain as the load-bearing exception (admission + posture-probe dispatch).
- `docs/ROADMAP.md` — new "Recently Completed" entry (September 7, 2026) summarizing the whole
  outcome.

## Decisions

- Scoped the divergence-freshness regex checks to the succession subsections (not whole-file
  phrase bans) after `succession_divergences_are_not_stale` false-positived on an unrelated
  pre-existing "specified and unimplemented" bullet about `Maintenance*` codes in
  `diagnostics.md` — the phrase recurs legitimately elsewhere in that file.
- `state.md` and `incremental_models.md` needed no edits: both already read as shipped behaviour
  (checked by direct grep sweep, task 2 of the plan) — the residual "not yet" phrasing named in
  the spec delta did not actually exist in either file.
- Ran a lightweight validate pass (timeless-oracle grep + targeted surface/semantics spot
  checks) rather than the full `/smelt:validate` skill's cargo fmt/clippy/test re-run, since
  `verify-phase.sh` in this same phase already covers those checks; duplicating them would have
  cost ~10 more minutes of CPU for no new signal.

## For the next planner

- `docs-site/docs/guide/scd2-succession.md` has a pre-existing (phase 9, commit `b63f46f9`)
  broken relative link to `../../../docs/specs/incremental_shapes.md` (mkdocs build warns; does
  not fail). Out of scope for this phase — not something phase 10 introduced — but worth a
  one-line fix next time that file is touched.
- `cargo test -p smelt-db --test diagnostics_catalogue` named in the plan's Verification block
  does not exist as a standalone target; the actual gate is
  `cargo test -p smelt-db --test integration diagnostics_catalogue` (a filtered test inside the
  `integration` binary). Ran the correct form; green. Future plans citing this gate should use
  the corrected invocation.
- No drift found requiring a new plan. Outcome success criteria 1-10 are all now evidenced;
  this phase's own title is "Validate and close", so the outcome's `**Status:**` is flipped to
  `done` below alongside the phase-table row.

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, full workspace
  `cargo test`, `example_diagnostics`, all green).
- `cargo test -p smelt-cli --test succession_docs_freshness --test state_docs_freshness` — PASS
  (7 tests).
- `cargo test -p smelt-logical --test walk_coverage --test maintenance_plan_conformance` — PASS
  (20 tests).
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --test projection_dialect_invariance` — PASS (49 tests).
- `cargo test -p smelt-cli --test maintenance_conformance` — PASS (101 tests).
- `cargo test -p smelt-db --test integration diagnostics_catalogue` — PASS (corrected target
  name; 1 test, 369 filtered).
- `cargo test -p smelt-lsp --test example_workspaces` — PASS (36 tests).
- `cargo test -p smelt-cli --test explain_maintenance --test cli_docs_coverage` — PASS
  (57 tests).
- `bash .claude/scripts/large-file-check.sh` — PASS (no ratchet regression).
- `cd docs-site && uv run mkdocs build` — PASS (exit 0; one pre-existing warning on
  `scd2-succession.md`'s cross-repo relative link, unrelated to this phase's edits).

## Drift-report verdicts (lightweight, in place of the full `/smelt:validate` skill)

- **`incremental_shapes.md`**: timeless-oracle grep clean (no `Phase [A-Z0-9]` leakage outside
  §Known Divergences/§References); §References §"The succession grain" now cites 15 real paths,
  all verified to exist; no remaining stale-absence claims.
- **`model_properties.md`**: timeless-oracle grep clean; Keyed-succession classification row now
  `built`; §References Code/Tests updated.
- **`diagnostics.md`**: timeless-oracle grep clean; unimplemented-succession bullet removed;
  the twelve codes' catalogue entries (§"Succession grain") were already accurate (each code's
  `when it fires` text describes shipped behaviour, no changes needed).
