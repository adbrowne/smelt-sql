# Phase 3a — the succession diagnostics surface

## Objective

Turn phase 3's `Refusal::SuccessionNotRecognized` (and the classifier's advisory, which
phase 3 currently drops on the floor) into real user-visible diagnostics: eleven
`Succession*` `DiagnosticCode` variants, mapped in the two single-owner mapping functions
and folded into `check_file_diagnostics` so LSP and CLI see the same set, each exercised by
an `examples/broken` fixture. Advances success criterion 2 in full, and unblocks criterion 9's
diagnostics-reference page.

## Spec delta

None. `docs/specs/diagnostics.md` §"Succession grain" already specifies all twelve codes
normatively (severity, condition, what each names). The §Known Divergences bullet that calls
them "specified and unimplemented" is rewritten by phase 10, not here — `SuccessionClockTie`
is still unimplemented after this phase (it is runtime, phase 5).

## Tests

Red-green, in this order:

1. `smelt-logical` `refusal_code_tests::succession_reasons_each_name_their_own_code` — the ten
   `NotSuccessionReason` variants map to ten distinct `Succession*` names, none `None`.
2. `smelt-logical` `refusal_code_tests::every_refusal_is_classified` (existing) — drop
   `SuccessionNotRecognized` from `none_variants`; must still pass.
3. `smelt-logical` `maintenance::succession::tests::advisory_does_not_change_the_derived_plan` —
   `derive_succession_plan` over the same `Recognized` verdict with and without the advisory
   yields byte-identical `plan` and `output`; only `advisories` differs.
4. `smelt-db` `refusal_codes::refusal_code_names_are_real_variants` (existing gate) — extended
   with the ten succession `(Refusal, MaintenanceRefusal)` pairs; both directions agree.
5. `smelt-db` `diagnostics_catalogue::every_diagnostic_code_is_catalogued` (existing) — green
   with the eleven new variants (the spec table already names them).
6. `smelt-db` `queries::maintenance` unit tests: `succession_refusal_projects_to_maintenance_refusal`
   (each reason survives the `Refusal` → `MaintenanceRefusal` projection, none filtered to `None`)
   and `succession_advisory_reaches_plan_diagnostics`.
7. `smelt-cli` `example_diagnostics::broken_workspace_succession_codes` — for each of the eleven
   fixtures, exactly its own `Succession*` code fires at that file and at no other file in
   `examples/broken/`; the advisory fixture's code is `Warning` severity and its file also
   reports no `Succession*` Error.
8. `smelt-cli` `example_diagnostics` (existing, all clean workspaces) — no clean example gains
   a succession diagnostic; `smelt-lsp` `example_workspaces` likewise.

## Tasks

1. Add the eleven unit variants to `DiagnosticCode` (`crates/smelt-db/src/diagnostics_types.rs`),
   grouped under a `Succession grain` doc block; do **not** add `SuccessionClockTie` (phase 5).
2. `smelt-logical`'s `refusal_code`: replace the blanket `SuccessionNotRecognized => None` arm
   with an exhaustive inner match on `NotSuccessionReason` returning the ten code names; delete
   the phase-3a TODO comment and the `none_variants` entry in its test.
3. Carry the advisory: add `advisories: Vec<SuccessionAdvisory>` to `SuccessionDerivation` and
   populate it from the `Recognized` verdict — the `MaintenancePlan` itself stays untouched, so
   admission provably cannot depend on it (test 3).
4. `crates/smelt-db/src/queries/maintenance.rs`: add
   `MaintenanceRefusal::SuccessionNotRecognized { reason: NotSuccessionReason }`; map it in
   `diagnostic_for_refusal` to the matching `(Error, DiagnosticCode::Succession*, message)` per
   reason, each message carrying the reason's own detail string; replace the `=> None` filter in
   the `Refusal` → `MaintenanceRefusal` projection with the real projection.
5. Thread the advisory to the surface: carry `succession_advisories` on `MaintenancePlanResult`
   (populated only on the succession branch) and on `MaintenancePlanDiagnostics`, mirroring
   `state_downgrades`' shape.
6. `crates/smelt-db/src/file_check.rs`: fold `plan_diags.succession_advisories` into a
   `Warning`/`SuccessionPreFilterNegatesFlag` diagnostic anchored at `body_start`, alongside the
   existing `state_downgrades` loop. Refusals need no new fold — they already flow through
   `diagnostic_for_refusal`.
7. Add `examples/broken/models/sources/succession_changes.yml`: `mutation_profile: append_only`,
   a `timeseries.event_time_column: changed_at`, columns `customer_id`/`changed_at`/`is_deleted`
   all `nullable: false` plus one nullable column and one payload column, so each fixture can
   isolate exactly one refusal.
8. Add eleven fixtures `examples/broken/models/succession_<code_suffix>.sql`, each
   `refresh: incremental` with no `timeseries:`/`unique_key:`/`grain:`, each a minimal edit away
   from the recognised shape, with a header comment naming its code and the spec rule. The
   `SuccessionSingleSourceOnly` and `SuccessionDrivingSourceNotAppendOnly` fixtures need a second
   source; reuse `maintenance_orders`.
9. Write test 7's helper (mirror `broken_workspace_maintenance_scan_unbounded`'s
   filter-to-a-code-set + expected-file shape) and the per-fixture assertions.
10. Run the clean-workspace gates and reconcile: any clean example that now reports a succession
    refusal is a real finding — record it in the summary and fix the example or the classifier,
    never by widening the test's filter.

## Verification

- `bash .claude/scripts/verify-phase.sh` (the `join_context_reach` failure the phase-3 summary
  documented is expected to remain red — phase 3b owns it; every other leg must be green).
- `cargo test -p smelt-db --test integration refusal_codes diagnostics_catalogue`
- `cargo test -p smelt-logical --lib refusal_code_tests --lib maintenance::succession`
- `cargo test -p smelt-cli --test example_diagnostics`
- `cargo test -p smelt-lsp --test example_workspaces`

## Commit message

`feat(smelt-db): raise the eleven succession-grain diagnostics from the plan's refusal`
