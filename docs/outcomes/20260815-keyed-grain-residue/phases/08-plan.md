# Phase 8 plan — Validate + close out

## Objective

Close the outcome by proving criterion 6: `/smelt:validate incremental_shapes` reports no drift for
any bullet phases 1, 2, 4, 5 and 7 closed, and every standing gate is green. Any drift the
validation surfaces that is *attributable to this outcome's own changes* is fixed here; drift owned
by other outcomes is reported, not fixed. Criterion 3 stays unmet by design — phase 3 is blocked on
a recorded human decision — and this phase records that honestly rather than papering over it.

## Spec delta

No user-visible behaviour changes here. Two bookkeeping spec edits only, both in
`docs/specs/incremental_shapes.md`:

- **§References → "The key grain"** — add the code/test entries this outcome introduced that the
  block does not yet name: the `KeyedRetractableContribution` classifier seam in
  `smelt-logical/src/maintenance/derive.rs`, `Backend::execute_write_with_bookkeeping`
  (`smelt-backend/src/lib.rs` + the DuckDB override), `rules::cumulative::execution_postures`,
  `RunReporter::state_structure_unavailable` (`smelt-runtime/src/reporter.rs`), and the tests
  `keyed_frontier_bookkeeping`, `execution_postures`, `arb_once_write_null_schedule`.
- **Front matter** — bump `last_reviewed` to the validation date (currently `2026-09-03`).

Only add entries that are actually absent; do not restate.

## Tests

No new product tests — this phase adds no behaviour. The "tests" are the validation checks
themselves, each of which must be run and its output read:

- `smelt:validate incremental_shapes` — full drift report; the oracle for this phase.
- `grep -nE 'Phase [A-Z0-9]' docs/specs/incremental_shapes.md docs-site/docs/reference/state.md` —
  timeless-oracle check over the files this outcome touched; must be empty.
- Every `docs/specs/incremental_shapes.md` §References path resolves (`ls`/`test -e` per path) —
  catches references rotted by phases 1-7's renames.

## Tasks

1. Invoke `Skill smelt:validate` with `incremental_shapes`; capture the full drift report.
2. Triage each drift item into: (a) caused by this outcome's phases 1/2/4/5/7 → fix here;
   (b) pre-existing and owned elsewhere → leave, and name the owning outcome/plan in the summary;
   (c) criterion 3 / phase 3 residue → leave, cite the Blocked entry.
3. Fix every category-(a) item.
4. Apply the §References additions and the `last_reviewed` bump described under Spec delta.
5. Run the timeless-oracle grep and the References-path existence check; fix what they surface.
6. Run the standing gates listed under Verification and record each result verbatim.
7. Write `phases/08-summary.md`: per-criterion verdict for criteria 1-6 (criterion 3 explicitly
   unmet, pointing at the Blocked entry and its three candidate options), the category-(b) drift
   list with owners, and the gate results — this summary is the evidence the close-out step judges
   the Success criteria against.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt + clippy both feature sets + `cargo test` +
  `example_diagnostics`)
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --test
  keyed_frontier_bookkeeping --test projection_dialect_invariance --test dialect_seam`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo check -p smelt-maintenance-testkit --features spark,bigquery --all-targets` (phase 6's
  gated twin)
- `cargo test -p smelt-lsp --test example_workspaces`

A gate that fails for a reason this outcome introduced is fixed here; a pre-existing failure is
recorded in the summary with evidence that it predates the outcome (re-run against the base commit).

## Commit message

`docs(20260815-keyed-grain-residue): close out — validate incremental_shapes, sync spec references`
