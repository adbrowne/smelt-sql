# Phase 10 — Validate and close

## Objective

Retire every stale "not yet built" claim about the succession grain across the six anchored
specs, point their §References blocks at the code and tests phases 2–9 landed, and prove the
whole outcome green: `/smelt:validate` clean for `incremental_shapes`, `model_properties`, and
`diagnostics`, and every standing gate passing. This closes success criterion 10 and is the
last evidence needed for criteria 1–9, whose implementations already landed.

## Spec delta

No user-visible behaviour changes. The edits are status/divergence corrections plus reference
freshness — the spec files this phase rewrites:

- `docs/specs/incremental_shapes.md` §Known Divergences "The succession grain" — delete the three
  stale bullets ("No implementation exists yet", "The generative conformance recipes … do not
  exist yet", "The conformance pool has no arrival-partitioned source recipe"); keep the
  `__tombstones` reserved-suffix collision bullet (still true); add the one genuine residual:
  Spark, BigQuery, and `state.warehouse_tables: none` have no tombstone-ledger builder and take
  the recorded `MaintenanceStateDowngraded` full refresh. §References §"The succession grain"
  gains **Code** and **Tests** bullets and a **Plans (history)** line citing this outcome.
- `docs/specs/model_properties.md` — the "Keyed-succession classification" declarations-table row
  Status `not-yet` → `built`; §References Code/Tests gain
  `crates/smelt-logical/src/analysis/succession.rs` and its per-rule unit tests.
- `docs/specs/model_transforms.md` — the "Succession-patch keyed `MERGE`" row's last column
  `unbuilt` → `built`.
- `docs/specs/diagnostics.md` — the "All twelve succession-grain codes are specified and
  unimplemented" §Known Divergences bullet is deleted (all twelve are `DiagnosticCode` variants
  with live dispatch); if any residual remains it is rewritten in behaviour terms.
- `docs/specs/sources.md` §Known Divergences "Declared profiles license almost nothing yet" —
  amend the "the fold/ledger techniques … are the unbuilt machinery" clause to except the
  succession grain, whose `append_only` admission is load-bearing and whose posture probe now
  dispatches on the succession window-forward path (phase 6c).
- `docs/specs/state.md` — confirm the "Tombstone ledger (succession grain)" residency row and the
  degradation contract read as built; keep `state_docs_freshness::spec_references_are_live` green.
- `docs/specs/incremental_models.md` §Limitations "SCD2 recognition is bounded to the
  keyed-succession pattern" — verify it reads as shipped behaviour; correct any residual
  "not yet" phrasing found by the sweep in task 2.

## Tests

New, in `crates/smelt-cli/tests/succession_docs_freshness.rs` (all red before the spec edits):

1. `succession_spec_references_are_live` — every backticked `crates/…`, `docs/…`, `examples/…`
   path cited in `incremental_shapes.md` §References §"The succession grain" exists on disk
   (same shape as `state_docs_freshness::spec_references_are_live`, scoped to that subsection so
   it cannot trip on the out-of-scope repo-wide path drift).
2. `succession_divergences_are_not_stale` — `incremental_shapes.md`'s §Known Divergences "The
   succession grain" subsection contains none of the retired phrases ("No implementation exists
   yet", "do not exist yet", "has no arrival-partitioned source recipe"), and `diagnostics.md`
   carries no "specified and unimplemented" succession bullet.
3. `succession_status_rows_read_built` — `model_properties.md`'s keyed-succession declarations
   row and `model_transforms.md`'s succession-patch row both end in a `built` status cell.

## Tasks

1. Write the three tests above; confirm each fails against the current specs.
2. Sweep the six anchored specs for stale succession claims:
   `rg -n -i 'succession|tombstone' docs/specs/{incremental_shapes,model_properties,model_transforms,diagnostics,sources,state,incremental_models}.md`
   and list every line asserting absence of implementation.
3. Apply the `incremental_shapes.md` §Known Divergences rewrite and the §References Code/Tests/
   Plans bullets (cite `analysis/succession.rs`, `maintenance/emit.rs`, the runtime succession
   driver, `smelt-state`'s ledger DDL, the conformance `families/gate_succession.rs`,
   `statement_parity`, `examples/scd2_succession/`, this outcome directory).
4. Apply the `model_properties.md`, `model_transforms.md`, `diagnostics.md`, `sources.md`,
   `state.md`, and `incremental_models.md` edits named in the spec delta.
5. Green the three tests.
6. Run `/smelt:validate incremental_shapes`, `/smelt:validate model_properties`, and
   `/smelt:validate diagnostics`; fix any drift the reports raise that is inside this outcome's
   scope, and record anything genuinely out of scope as a residual §Known Divergences bullet
   (behaviour terms, no phase vocabulary — timeless-oracle rule).
6a. Record the three drift reports' verdicts in `phases/10-summary.md`; do not commit the raw
   reports as separate files.
7. Run every standing gate listed under Verification and record each result in the summary.
8. Update `docs/ROADMAP.md` with the succession grain as complete, dated 2026-09-07.

## Verification

```bash
bash .claude/scripts/verify-phase.sh
cargo test -p smelt-cli --test succession_docs_freshness --test state_docs_freshness
cargo test -p smelt-logical --test walk_coverage --test maintenance_plan_conformance
cargo test -p smelt-runtime --test statement_parity --test execute_parity --test projection_dialect_invariance
cargo test -p smelt-cli --test maintenance_conformance
cargo test -p smelt-db --test diagnostics_catalogue
cargo test -p smelt-lsp --test example_workspaces
cargo test -p smelt-cli --test explain_maintenance --test cli_docs_coverage
bash .claude/scripts/large-file-check.sh
cd docs-site && uv run mkdocs build
```

## Commit message

`docs(succession): close the spec divergences and validate the succession grain`
