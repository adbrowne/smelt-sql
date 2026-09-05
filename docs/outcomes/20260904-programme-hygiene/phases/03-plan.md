# Phase 03 plan — absent-state sentences for the three unspecified structures

## Objective

Close success criterion 4: give deployed-schema snapshots, source postures, and probe
baselines each one sentence of absent-state behaviour in their owning spec, in one of the two
shapes `state.md` §"The optionality rule" permits (degrade-and-say-so, or refuse-by-name), then
delete `state.md` §Known Divergences' "Structure-level degradation behaviours are unevenly
specified" bullet. Docs-only; no crate changes.

## The three calls (made here, written by the implement step)

Each reuses a diagnostic already owned by `state.md` §Surface; no new code, no new machinery.

1. **Deployed-schema snapshots** — *degrade and say so.* Absent (`state.mode: stateless`, or the
   directory deleted), `smelt diff` reports every model as `new` and no `AlterTable` migration is
   planned: the model goes through its ordinary create/replace materialization. The degradation is
   visible in the diff output; smelt never reads a missing snapshot as `NoChange`.
2. **Source postures** — *degrade and say so.* The `append_only` verification mechanisms that need
   a cross-run recorded baseline (watermark-monotonicity row counts, frontier checksum) cannot run
   under a posture that persists nothing, so the narrowing declaration's fold licence is withheld:
   admission falls back to the *undeclared* row of §"`mutation_profile`"'s licence table and the
   downgrade is recorded (`MaintenanceStateDowngraded`). Scan-window-scoped probes (`unique_key`,
   `delta_identity` uniqueness) are unaffected — they verify within the consuming run.
3. **Probe baselines (frozen-band)** — *refuse by name.* `contract.frozen_horizon` sells a
   relaxation of the equivalence invariant paired with the late-arrival probe, and that probe is
   baseline-comparative across runs. Declaring it under a posture that does not persist probe
   baselines is `DeclaredContractRequiresState` — the same call `state.md` already makes for
   `contract.deferral`, for the same reason: silently dropping the check turns a declared
   guarantee into an unverified hope. (The first-run "establishes the baseline" case is unchanged.)

## Spec delta

- `docs/specs/schema_evolution.md` §Semantics "Stored schemas" — extend the existing
  "If `.smelt/schemas/` does not exist…" line into call 1, citing `state.md` §"The optionality rule".
- `docs/specs/sources.md` §Semantics item 4 ("Verification mechanisms") — append call 2.
- `docs/specs/incremental_models.md` §"The contract lattice", the `frozen_horizon` probe paragraph
  (~line 815, after "so it only establishes the baseline") — append call 3.
- `docs/specs/state.md` §Known Divergences — delete the "Structure-level degradation behaviours
  are unevenly specified" bullet (4 lines).
- No `run_state.md` edit: it already states "Stateless writes nothing" and defers structure
  semantics to owner specs. Add a one-clause cross-ref there only if the implement step finds
  `run_state.md` asserting a *different* absent-snapshot behaviour.

## Tests

Docs-only phase; the red-green oracle is a grep assertion per call, written and observed failing
before the edit.

- `absent_schema_snapshot_stated` — `rg -n 'optionality rule' docs/specs/schema_evolution.md`
  hits, in the "Stored schemas" section.
- `absent_source_posture_stated` — `rg -n 'MaintenanceStateDowngraded' docs/specs/sources.md` hits.
- `absent_probe_baseline_stated` — `rg -n 'DeclaredContractRequiresState' docs/specs/incremental_models.md`
  hits, inside the contract-lattice `frozen_horizon` text.
- `state_divergence_bullet_gone` — `rg -n 'Structure-level degradation behaviours' docs/specs/state.md`
  exits 1.
- `no_phase_vocabulary` — `rg -n 'Phase [A-Z0-9]' docs/specs/{state,sources,schema_evolution,incremental_models}.md`
  surfaces no new hit (timeless-oracle rule).

## Tasks

1. Run the five checks above; confirm the first four fail as expected.
2. Read `docs/specs/state.md` §"The optionality rule" and §"The degradation contract" so each
   sentence's wording matches the doctrine's vocabulary (degrade-and-say-so / refuse-by-name).
3. Write call 1 into `schema_evolution.md` §"Stored schemas".
4. Write call 2 into `sources.md` §Semantics item 4.
5. Write call 3 into `incremental_models.md` §"The contract lattice" `frozen_horizon` paragraph.
6. Delete the `state.md` §Known Divergences bullet; check nothing else in `state.md` refers back
   to it (`rg -n 'unevenly specified' docs/specs/`).
7. Re-run the five checks; all green.
8. Write `phases/03-summary.md` (what each sentence says, where it landed, any wording the specs
   forced a change to, and anything phase 5's validate step should expect).

## Verification

- `bash .claude/scripts/verify-phase.sh` (doc-sync gates included).
- The five `rg` checks above.
- `rg -n 'schema snapshots|source postures|probe baselines' docs/specs/state.md` — the inventory
  table rows still stand; only the divergence bullet is gone.

## Commit message

`docs(programme-hygiene): specify absent-state behaviour for schema snapshots, source postures and probe baselines`
