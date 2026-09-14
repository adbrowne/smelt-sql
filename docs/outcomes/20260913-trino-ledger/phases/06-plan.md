# Phase 6 plan — the three lattice points on Trino: `deferral` refuses, `frozen_horizon` and `retain_departed` proceed

## Objective

Settle what each contract-lattice point does on a dialect that realises no correctness structure.
`contract.deferral`'s semantics *are* a statement about state, so it fails loudly with
`DeclaredContractRequiresState`; `frozen_horizon` and `retain_departed` are statements about the
model's own SQL, so they stay admitted, their clamps and probes still apply, and no new lattice
point is introduced. Advances the success criteria on the degradation contract (an absence is a
downgrade or a named refusal, never a silent skip) and on "declarations stay fail-loud".

## Findings this plan acts on (verified while planning, not assumptions)

- The analysis-time `DeclaredContractRequiresState` derivation
  (`smelt-db/src/queries/maintenance/diagnostics.rs`) is already fully dialect-driven —
  `backend_dialect_for("trino")` → `realisable_state_structures(Trino)` (empty) → the refusal.
  Expected to be green on arrival; the tests below are the standing gate that it stays so.
- `retain_departed::emit_departed_key_probe` is dialect-free ANSI SQL and needs no
  `MaintenanceDialect`; `probe_plan.rs` already lists it among the rows that need no dialect.
- **Real gap:** `execute/project/mod.rs:3554` resolves
  `smelt_backend::maintenance_dialect(backend.dialect())?` *unconditionally* inside the
  `if let Some(end_date)` arm, before `frozen_horizon_probes` is asked whether the model declares
  anything at all. On Trino that `?` returns `UnsupportedMaintenanceDialect` and hard-errors the
  run — for any clocked model, declaration or not. Phase 4 already established the lazy-resolution
  posture for exactly this shape in `probe_plan.rs`; this site was missed.

## Spec delta (make this edit first)

`docs/specs/state.md` §"Declarations stay fail-loud" — extend the existing exception paragraph
with the converse rule it implies but does not state: a declaration the SQL upholds stays valid
even where its *verification probe* has no emission on the target dialect; the run proceeds and the
skipped verification is reported as a run-time warning naming the model, the dialect and the probe,
following the §"The degradation contract" precision clause's recording rule. Name `frozen_horizon`'s
late-arrival probe on a dialect with no `MaintenanceDialect` as the instance, and restate that
`contract.deferral` remains the sole refusing declaration. Add the matching one-liner to
`docs/specs/multi_backend.md` §"Incremental & schema evolution per backend" under Trino.

## Tests (red-green)

1. `smelt-db` `maintenance_diagnostics/status_and_contract.rs`
   `deferral_on_trino_refuses_declared_contract_requires_state` — the existing Spark fixture with
   `smelt_yml_single_target("trino")`: exactly one Error naming `contract.deferral` and the
   reconciliation ledger.
2. same file, `cell_level_deferral_on_trino_refuses` — the `contract.cells[].deferral` twin.
3. same file, `frozen_horizon_on_trino_is_admitted` — a `contract.frozen_horizon: 90 days` model on
   a trino-only target raises **no** `DeclaredContractRequiresState` and no Error at all.
4. same file, `retain_departed_on_trino_is_admitted` — a keyed model declaring
   `contract.retain_departed` on a trino-only target raises no `DeclaredContractRequiresState`.
5. `smelt-logical` `contract/point.rs` tests: `required_state_structure_is_dialect_free` — the
   point → structure map is a property of the point alone (no dialect argument), so no new lattice
   point is needed for Trino; asserts the three-variant exhaustive map is unchanged.
6. `smelt-runtime` new `crates/smelt-runtime/tests/trino_contract_points.rs`
   `frozen_horizon_probe_dialect_is_resolved_only_when_declared` — the RED test for the 3554 gap:
   a clocked model with **no** `contract.frozen_horizon`, compiled/executed against a Trino target
   stub, must not produce an `UnsupportedMaintenanceDialect` error.
7. same file, `frozen_horizon_declared_on_trino_skips_its_probe_with_a_warning` — with the
   declaration present, the run proceeds (no error), emits zero frozen-band probe statements, and
   logs one `tracing::warn!` naming the model, the dialect and the skipped probe (capture via
   `tracing_subscriber`, mirroring `maintenance_driver/membership/execute.rs`'s twin site).
8. `smelt-cli` `trino_explain_downgrade.rs` `explain_on_trino_reports_the_deferral_refusal` — a
   staged project declaring `contract.deferral` on trino surfaces the refusal through the CLI
   (non-zero / refusal text naming the declaration), proving it is not diagnostics-only.

## Tasks

1. Land the `state.md` + `multi_backend.md` spec edits above.
2. Write tests 1–4 and run them; expect green (standing gates). Fix `diagnostics.rs` only if a test
   actually fails — do not pre-emptively touch it.
3. Write test 5; keep `required_state_structure` exhaustive over `ContractPoint` with no new variant.
4. Write tests 6–7 (RED), then make `execute/project/mod.rs`'s frozen-horizon arm resolve the
   maintenance dialect lazily: build the probe set only when the model declares
   `contract.frozen_horizon`, and where the target has no `MaintenanceDialect`, skip the probe with
   the `tracing::warn!` shape `maintenance_driver/membership/execute.rs` uses (model, dialect,
   structure/probe named) instead of propagating `UnsupportedMaintenanceDialect`.
5. Audit the sibling `maintenance_dialect(backend.dialect())?` sites in `execute/project/mod.rs`
   for the same unconditional shape **on the paths these three declarations reach**; fix any that
   a Trino run of the test fixtures actually hits, and record the rest (with line numbers) in the
   phase summary for phases 7–10 rather than widening this phase.
6. Write test 8 and run the CLI leg.
7. Update `docs/specs/diagnostics.md` only if a message string changed.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, shellcheck, full test,
  example_diagnostics) — must be green with no baseline bumped.
- `cargo test -p smelt-db --test maintenance_diagnostics`
- `cargo test -p smelt-logical --test maintenance_availability` and `cargo test -p smelt-logical --lib contract::`
- `cargo test -p smelt-runtime --test trino_contract_points --test availability_seam`
- `cargo test -p smelt-cli --test trino_explain_downgrade --test trino_spec_freshness`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`feat(state): the contract lattice on Trino — deferral refuses, frozen_horizon and retain_departed proceed with the probe skip named`
