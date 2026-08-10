# Phase 7 plan — Surface: `explain` contract rendering + docs-site

## Objective

Make the effective contract per cell visible: `smelt explain <model>` prints, for every cell,
whether it sits at the default point or a relaxed one and with which parameters, in both the text
report and the `--json` per-model report. This closes success criterion 4 and deletes the last
"still missing" clause of the contract-lattice Known Divergence; the user-facing docs gain the
`contract:` block. Criterion 6 (all standing gates green) is re-established at the end.

## Spec delta (spec-first — the implement step makes these edits before code)

1. `docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan report" — add one paragraph:
   each cell's block additionally prints its **effective contract** — `default` when no `contract:`
   applies, otherwise the applicable relaxations with their declared intervals
   (`frozen_horizon: 90 days` — model-level, every cell of a partition-grain model; `deferral: 6
   hours` — model-level default, or a `contract.cells[]` refinement, which prints the narrower
   value and labels its origin). The `--json` per-model report carries the same as a per-cell
   `contract_point` object; absent relaxations are omitted, never rendered as null. Note that a
   per-cell `deferral` refinement is printed as declared even though it is not yet scheduled
   (existing Known Divergence).
2. `docs/specs/incremental_models.md` §Known Divergences, the contract-lattice bullet — delete the
   final "Still missing: `smelt explain` rendering the effective contract per cell." sentence and
   replace it with a statement that the effective contract is printed per cell in both renderings;
   keep the per-cell-`deferral`-not-scheduled clause.
3. `docs-site/docs/guide/incremental-models.md` — new `## Contract relaxations` section after
   §"Declaring a horizon ceiling (warning only)": the `contract:` block, the two points, what each
   licenses, the two runtime diagnostics by name, and the `smelt explain` line showing the
   effective contract. `docs-site/docs/reference/cli.md` §`smelt explain` — one line in the
   maintenance-plan report description mentioning the effective-contract row.

## Tests (red → green)

- `crates/smelt-logical/src/contract/mod.rs` unit tests
  - `effective_contract_defaults_to_the_default_point` — no `contract:` → `EffectiveContract`
    with both relaxations `None`.
  - `effective_contract_applies_model_level_frozen_horizon_to_every_cell` — model-level `H`
    reaches a cell regardless of its trigger/columns.
  - `effective_contract_cell_deferral_overrides_the_model_default` — a `contract.cells[]` entry
    matching the cell's `(columns × on)` address wins over the model-level `deferral`, and its
    origin is reported as cell-level.
  - `effective_contract_non_matching_cell_entry_keeps_the_model_default` — an entry addressing a
    different column group / trigger does not apply.
- `crates/smelt-cli/tests/explain_maintenance.rs`
  - `explain_prints_default_contract_point_per_cell` — an ordinary maintained model's report has a
    `contract: default` row on every cell.
  - `explain_prints_frozen_horizon_contract_point` — the phase-2 `contract: frozen_horizon`
    example fixture renders `frozen_horizon: <interval>` on its cells.
  - `explain_json_carries_contract_point_per_cell` — `--json` cells carry `contract_point` with
    the declared interval; a default cell omits the relaxation keys.
- `crates/smelt-logical/tests/contract_lattice_spec.rs`
  - `explain_contract_rendering_is_single_owned` — structural: the CLI explain path resolves the
    effective contract through `smelt_logical::contract::effective_contract` and contains no local
    model-vs-cell ladder over `ContractConfig`.

## Tasks

1. Make the three spec/doc edits above (spec-first), including the docs-site section.
2. Add `EffectiveContract` + `effective_contract(cfg: Option<&ContractConfig>, trigger_address,
   group_columns)` to `smelt-logical/src/contract/mod.rs`, with a `matching_contract_cell` matcher
   mirroring `maintenance::choice::matching_cell`'s addressing semantics (narrower wins), plus a
   `render_label()`-style one-line description used by both renderings.
3. Thread the model's `ContractConfig` into `build_maintenance_plan_report` and
   `build_maintenance_plan_json` in `crates/smelt-cli/src/explain.rs` (the caller in
   `commands/explain.rs` already loads `ModelMetadata`; read `metadata.contract`).
4. Render the text row per cell (`      contract:  default` / `      contract:  frozen_horizon
   90 days, deferral 6 hours (cell)`) beside the existing `corner:`/`technique:` rows.
5. Add `contract_point: ExplainContractPointJson` to `ExplainCellJson` with
   `#[serde(skip_serializing_if)]` on each optional relaxation (append-stable, per `cli.md`
   §Constraints item 5).
6. Extend `contract_lattice_spec.rs` with the structural single-owner assertion.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test contract_lattice_spec`
- `cargo test -p smelt-cli --test explain_maintenance --test explain_show_sql --test explain_model`
- `cargo test -p smelt-cli --test example_diagnostics`

## Commit message

`feat(contract-lattice): explain renders the effective contract per cell`
