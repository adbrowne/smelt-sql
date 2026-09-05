# Phase 03 summary — absent-state sentences for schema snapshots, source postures, probe baselines

## Shipped

- `docs/specs/schema_evolution.md` §"Stored schemas" — the existing "If `.smelt/schemas/` does
  not exist…" line now names both trigger conditions (`state.mode: stateless` or a deleted
  directory), states the degrade-and-say-so behaviour explicitly (ordinary create/replace
  materialization, never read as `NoChange`), and cites `state.md` §"The optionality rule".
- `docs/specs/sources.md` §Semantics item 4 ("Verification mechanisms") — appended: the
  watermark-monotonicity probe and frontier checksum need a cross-run baseline a stateless
  posture can't supply, so the narrowing declaration's fold licence is withheld (falls back to
  the *undeclared* row of §"`mutation_profile`"'s licence table) and the downgrade is recorded
  as `MaintenanceStateDowngraded`. Named `unique_key`/`delta_identity`'s scan-window probe as
  unaffected.
- `docs/specs/incremental_models.md` §"The contract lattice", `frozen_horizon` paragraph —
  appended: since the late-arrival probe is baseline-comparative, declaring `frozen_horizon`
  under a posture with no persisted probe baselines is refused by name,
  `DeclaredContractRequiresState`, same call and same reason as `contract.deferral`. First-run
  baseline-establishment is unaffected (no comparison to skip).
- `docs/specs/state.md` §Known Divergences — deleted the "Structure-level degradation
  behaviours are unevenly specified" bullet (4 lines).

## Decisions

All three normative calls were pre-made in `phases/03-plan.md` (plan step); this step only
verified the wording landed in the doctrine's own vocabulary (degrade-and-say-so /
refuse-by-name) and reused the exact diagnostic names `state.md` §Surface already owns
(`MaintenanceStateDowngraded`, `DeclaredContractRequiresState`) rather than inventing new
language.

## For the next planner

- Confirmed pre-existing gap (not fixed here, out of this phase's site list): `§"What the
  composed shape uniquely enables"` is cited from three sites but doesn't exist in either
  incremental spec — already tracked as a fresh `docs/TODO.md` bullet from phase 2.
- No new gaps surfaced during this phase's edits.
- Phase 5 (`/smelt:validate state` + `model_properties`) should find the divergence bullet gone
  and the three owner-spec sentences present; nothing else in `state.md` referenced the deleted
  bullet (`rg -n 'unevenly specified' docs/specs/` is empty).

## Gates

- Five plan-listed `rg` checks: all four "should now hit"/"should now miss" checks pass; the
  `no_phase_vocabulary` sweep shows only the standard timeless-oracle banner text, no new phase
  labels.
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, full test suite,
  example_diagnostics). Docs-only change; no crate code touched.
