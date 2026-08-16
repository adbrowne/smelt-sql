# Phase 10 plan — `smelt explain`: per-column guarantee ledger + pre-execution refusal surfacing

## Objective

Close the second half of success criterion 5: `smelt explain` prints a **per-column guarantee
ledger** (each output column's effective equivalence contract × its derived settle bound, with a
volatile column's determinism exemption printed in place of the contract) and **surfaces the
model's refusals up front**, ahead of the plan, so an operator sees what will be refused before
executing anything. Both derivations are single-owned in `smelt-logical` and read identically by
the text and `--json` renderings, matching phase 9's headline precedent.

## Spec delta (made first, by the implementer)

1. `docs/specs/incremental_models.md` §Surface "CLI":
   - Move the **per-column guarantee ledger** out of the "Per cell" bullet into its own
     model-level bullet, and state its row shape: one row per output column carrying its column
     group, its effective contract point label, and its settle bound; a column whose settle bound
     is not derivable prints `settle: not derived` rather than a fabricated interval (the
     §"Delta signatures" never-fabricate rule); a column whose determinism verdict is not `Clean`
     prints its determinism exemption in place of the equivalence contract (§"The determinism
     scope").
   - Add a **Refusals** bullet: every derived refusal is printed immediately after the headline,
     before the plan body, naming the refusal's diagnostic code and its rendered reason — the
     pre-execution surface. An empty refusal set prints nothing there.
2. `docs/specs/cli.md` §"`smelt explain --json` output schema": add `guarantees` (array of
   `{column, group, contract | determinism_exemption, settle}`) and `refusals` (array of
   `{code, message}`) to the per-model maintenance report.
3. Known Divergences in `incremental_models.md`: narrow the "`smelt explain` does not yet print
   the per-column guarantee summary or surface a pre-execution refusal" bullet — delete it if
   nothing remains; and drop the "`smelt explain` does not print the determinism exemption in
   the per-column guarantee ledger" clause from the determinism-scope bullet (its runtime half —
   pinning, oracle exemption, technique gates — stays).

## Tests (red-green)

`crates/smelt-logical/src/maintenance/ledger.rs` (new module, unit):
1. `ledger_row_per_output_column_carries_its_group_contract` — every output column yields exactly
   one row, labelled with the effective contract of the group that owns it (two groups with
   different points ⇒ different labels on their columns).
2. `ledger_settle_bound_reads_established_locality` — a route-1 locality's `SettleBound` renders
   as the derived margin on every row; a route-2 (`Never`) locality renders `never`.
3. `ledger_settle_bound_not_derived_without_locality` — no `key_locality` ⇒ `not derived`, never a
   fabricated or zero interval.
4. `ledger_volatile_column_prints_determinism_exemption` — a column whose determinism verdict is
   `Run`/`Row` carries the exemption instead of a contract label; a `Clean` sibling is unaffected.
5. `refusal_summary_names_code_and_reason` — each `Refusal` variant renders as
   `<DiagnosticCode>: <reason>`, never `{:?}` of the enum.

`crates/smelt-db/tests/maintenance_ledger.rs` (new, integration):
6. `plan_report_populates_column_determinism` — `maintenance_plan_report` fills the new
   per-column determinism field from the walk's property vector; the non-report constructors
   default it empty.

`crates/smelt-cli/tests/explain_maintenance.rs`:
7. `explain_prints_per_column_guarantee_ledger` — text output contains a `Guarantees:` block with
   one row per output column carrying contract + settle.
8. `explain_surfaces_refusals_before_the_plan` — for a refusing model, the refusal block's byte
   offset is after the `emits:` headline and before `Maintenance plan:`.
9. `explain_json_guarantees_match_text` — `--json`'s `guarantees` and `refusals` arrays carry
   field values byte-equal to the text rendering (same single-owner values, per phase 9's
   headline test).

## Tasks

1. Land the three spec edits above (spec-first).
2. New `smelt-logical/src/maintenance/ledger.rs`: `ColumnGuarantee` (contract label or determinism
   exemption), `SettleLabel`, `GuaranteeRow { column, group, guarantee, settle }`, pure
   `derive_guarantee_ledger(column_groups, contract_cfg, key_locality, determinism)`; and pure
   `render_refusal(&Refusal) -> RefusalSummary { code, message }`. Contract points come from
   `smelt_logical::contract::effective_contract` (never a local ladder); settle from
   `KeyLocality::settle_bound` (never re-derived); export from `maintenance/mod.rs`.
3. `smelt-db`: add `column_determinism: Vec<ColumnDeterminism>` to `MaintenancePlanResult`,
   populated only in `maintenance_plan_report` from the `model_property_vector` it already
   computes (phase 9's `own_output_delta` precedent — other construction sites default empty).
4. `smelt-cli/src/explain.rs`: print the refusal block right after the headline (replacing the
   bottom `Refusals:` list, so there is exactly one refusal rendering) and a `Guarantees:` block
   after it; both read `derive_guarantee_ledger`/`render_refusal` output verbatim — the CLI
   formats no labels itself.
5. `--json`: `ExplainGuaranteeJson`/`ExplainRefusalJson` built from the SAME derived values, added
   to `ExplainMaintenanceJson`.
6. Update `docs-site/docs/reference/cli.md` prose + its golden explain fixture, and regenerate the
   web-analytics tutorial fixture (`python3 examples/web_analytics/generate_tutorial.py`) if its
   explain output moved.
7. Write `phases/10-summary.md`.

## Verification

- `cargo test -p smelt-logical --test walk_coverage --quiet`
- `cargo test -p smelt-db --test maintenance_ledger --test maintenance_signature --quiet`
- `cargo test -p smelt-cli --test explain_maintenance --test explain_model --test explain_show_sql --quiet`
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md docs/specs/cli.md` — no matches
- `bash .claude/scripts/verify-phase.sh`

## Commit message

`feat(explain): print the per-column guarantee ledger and surface refusals before the plan`
