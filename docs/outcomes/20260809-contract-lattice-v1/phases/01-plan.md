# Phase 1 — Spec: the lattice framing

## Objective

Land the normative framing for the contract lattice in `docs/specs/incremental_models.md`
before any code exists: the equivalence invariant becomes the **default point**, two v1
relaxations (`frozen_horizon`, `deferral`) are declared surface, and each point is defined by
the three-part admission rule settled in the Decision log (declaration schema + pure oracle
transform + probe emitter, single-owned in `smelt-logical`). Advances success criteria 3
(spec defines the lattice) and sets the contract the later phases discharge for 1, 2, 4, 5.
Spec + a standing spec gate only — no production code.

## Surface decision made by this phase

Relaxations do **not** live under `maintenance:` — that block is defined as never widening what
admission allows, and a relaxation does exactly that. v1 introduces a sibling top-level
`contract:` block:

```yaml
contract:
  frozen_horizon: '90 days'      # partition grain only
  deferral: '6 hours'
  cells:                          # optional per-cell refinement, addressed like maintenance.cells
    - columns: [<col>, ...]
      on: <source-address> | backfill
      deferral: '1 day'
```

Model-level values are the default for every cell; a `cells[]` entry refines one cell. Absent
`contract:` = the default point (strict equivalence). `horizon_ceiling:` is unchanged and stays
a warning threshold on the *derived* horizon — orthogonal to `frozen_horizon:`, which is a
contract relaxation that clamps writes.

## Spec delta (the phase's main work; spec-first)

`docs/specs/incremental_models.md`:

1. **§Semantics — new `### The contract lattice`**, immediately after §"The equivalence
   invariant". States: the invariant is the default point; a lattice point is admissible only as
   a complete triple (declaration schema, pure oracle transform, probe emitter) single-owned in
   `smelt-logical`; users pick and parameterise points, never define them; the effective contract
   is per cell. Then one subsection per v1 point with its restated oracle and probe:
   - **frozen horizon (H)** — oracle: strict equivalence over `S_H = { i ∈ S : partition(i)`
     within `H` of the run that scanned `i` }; writes outside `H` are clamped **by contract**;
     probe: a late-arrival count over scanned rows whose natural partition falls outside `H`,
     raising `ContractLateArrivalOutsideHorizon`.
   - **deferral (D)** — oracle: `∃ S' ⊆ S` such that every input in `S \ S'` arrived within the
     last `D`, and `incremental_state(S) == full_refresh(S')`; licenses skipping a run whose
     pending inputs are all inside the window, and subsuming a pending small run into a larger
     scheduled one; probe: ledger-derived lag (cell frontier vs input frontier) raising
     `ContractDeferralExceeded` when lag > `D`.
2. **§Surface — new `### Contract relaxations (`contract:`)`** after §"Maintenance overrides":
   the block above, the partition-grain-only restriction on `frozen_horizon`, cell addressing
   reused from `maintenance.cells`, and the rule that a relaxation is always printed by
   `smelt explain` (never silent).
3. **§"Windowed maintenance and the horizon"** — the derived-horizon paragraph gains a
   cross-reference: a declared `frozen_horizon` narrows write eligibility *by contract* and
   turns the silent-late-arrival behaviour into a diagnosed one; the "silently excluded"
   sentence is scoped to *the default point* (its deletion for declared cells lands in phase 3).
4. **§Diagnostics — new `**Contract-lattice codes.**` table**: `ContractFrozenHorizonInvalid`
   (unparseable/negative interval, or declared on a non-partition-grain model),
   `ContractLateArrivalOutsideHorizon`, `ContractDeferralInvalid` (unparseable/negative, or a
   cell with no clock to measure lag against), `ContractDeferralExceeded`.
5. **§Constraints & Invariants** — the lattice-point single-owner rule as a numbered constraint,
   plus a bullet in `CLAUDE.md`'s architectural-invariants list naming the standing gate.
6. **§Known Divergences** — one entry: the lattice is specified and unimplemented (no loader
   acceptance, no oracle transform, no probe emitter, conformance not yet parameterised),
   landing `docs/outcomes/20260809-contract-lattice-v1/outcome.md`.

`docs/specs/diagnostics.md`: the four codes catalogued, with a paragraph in the
specified-and-unimplemented note (rows may precede variants — existing posture).

## Tests

New standing gate `crates/smelt-logical/tests/contract_lattice_spec.rs`, in the shape of
`output_delta_spec.rs` (section extraction + table-row assertions over the spec files):

- `lattice_section_states_default_point_and_two_v1_points` — §"The contract lattice" exists and
  names the default plus exactly `frozen_horizon` and `deferral`.
- `each_point_states_its_oracle_transform_and_probe` — both point subsections state a restated
  oracle and a named probe; neither is declared-but-unchecked.
- `admission_rule_is_the_single_owner_triple` — the section states the three-part rule and
  locates ownership in `smelt-logical`.
- `surface_catalogues_the_contract_block` — §Surface documents `contract:`, the
  partition-grain-only restriction on `frozen_horizon`, and per-cell refinement.
- `horizon_section_scopes_silent_exclusion_to_the_default_point` — the silent-late-arrival
  wording is qualified and cross-references the frozen-horizon point.
- `diagnostics_tables_carry_the_four_lattice_codes` — both `incremental_models.md` §Diagnostics
  and `diagnostics.md` list all four codes.
- `constraint_and_claude_md_state_the_lattice_invariant` — the numbered constraint exists and
  `CLAUDE.md` carries a matching invariant bullet.
- `known_divergence_tracks_the_unimplemented_lattice` — the divergence entry exists and links
  the outcome file.

## Tasks

1. Write `contract_lattice_spec.rs` first; confirm it fails red against today's spec.
2. Edit `docs/specs/incremental_models.md` §Semantics: add §"The contract lattice".
3. Edit §Surface: add §"Contract relaxations (`contract:`)".
4. Edit §"Windowed maintenance and the horizon": scope the silent-exclusion sentence.
5. Edit §Diagnostics: add the contract-lattice code table.
6. Edit §Constraints & Invariants and `CLAUDE.md`: the single-owner lattice-point rule.
7. Edit §Known Divergences: the specified-and-unimplemented entry.
8. Edit `docs/specs/diagnostics.md`: catalogue the four codes + unimplemented note.
9. Re-run the gate green; run the verification gates.

## Verification

- `cargo test -p smelt-logical --test contract_lattice_spec`
- `cargo test -p smelt-db --test integration diagnostics_catalogue`
- `bash .claude/scripts/verify-phase.sh`

## Commit message

`spec(contract-lattice): default point, frozen horizon, and deferral with per-point oracles and probes`
