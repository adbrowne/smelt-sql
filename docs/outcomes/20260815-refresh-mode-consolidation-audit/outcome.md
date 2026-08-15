# Outcome: Audit and resolve the fate of `refresh: latest_value` / `versioned` / `materialized_view`

**Created:** 2026-08-15
**Status:** queued
**Source:** `docs/plans/20260704-model-updates.md` rows D1/D2/D3;
`docs/outcomes/20260815-definition-delta-migrate/outcome.md` §"Out of scope"
**Spec anchors:** `docs/specs/incremental_shapes.md`, `docs/specs/incremental_models.md`,
`docs/specs/materialized_view.md`

## The outcome

`docs/plans/20260704-model-updates.md` rows D1 (`refresh: latest_value`), D2 (`refresh:
versioned`), and D3 (`refresh: materialized_view` classifier/execution) target spec files
(`cumulative_aggregate.md`, `latest_value_models.md`, `versioned_models.md`) that were deleted and
consolidated into `keyed_models.md` and then `incremental_shapes.md`/`incremental_models.md`.
Whether each row's scope survived that consolidation under a different name (e.g. `latest_value`
folded into the key grain's order-monotone-overwrite family), was dropped as a decided non-surface,
or is still genuinely wanted is audited first, mode by mode, and recorded as an explicit decision —
then whatever is still wanted actually ships.

## Success criteria (checkable)

1. For each of D1 (`latest_value`), D2 (`versioned`), D3 (`materialized_view`), the audit records
   one of three findings in the decision log: **already covered** (names the current spec section
   it maps to), **decided non-surface** (states why it's not needed), or **still wanted** (scopes
   what remains to build).
2. `docs/plans/20260704-model-updates.md`'s D1/D2/D3 rows are updated to reflect the finding
   (closed, superseded-with-pointer, or still-pending-with-new-tracker) so the historical plan
   file stops silently pointing at deleted spec files.
3. For every mode found "still wanted," a spec diff lands in its owning spec (`incremental_shapes.md`
   if it's a shape-profile variant, `materialized_view.md` if it's that mode) before implementation
   (spec-first rule).
4. Every "still wanted" mode's classifier and execution path ship, backed by generative-conformance
   coverage against a full-refresh oracle the same way the key and partition grains are.
5. `/smelt:validate` reports no drift for every spec this outcome touches. All standing gates
   green.

## Out of scope

- None until the audit (phase 1) narrows scope — this outcome is deliberately audit-first because,
  per `20260815-definition-delta-migrate`'s own note, "that's a real question, not a mechanical
  rename sweep."

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Audit D1 (`latest_value`), D2 (`versioned`), D3 (`materialized_view`) against current spec state; record findings | pending |
| 2 | Spec diff for whichever modes the audit finds still-wanted | pending |
| 3 | Implementation: first still-wanted mode's classifier + execution | pending |
| 4 | Implementation: second still-wanted mode's classifier + execution (if any) | pending |
| 5 | Implementation: third still-wanted mode's classifier + execution (if any) | pending |
| 6 | Generative conformance coverage for each shipped mode | pending |
| 7 | Validate + close out: `/smelt:validate` clean for touched specs, `20260704-model-updates.md` rows updated, standing gates green | pending |

## Decision log

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
