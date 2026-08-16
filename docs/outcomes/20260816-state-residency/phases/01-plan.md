# Phase 1 — Spec deltas: absent-state behaviour for the three unspecified structures

**Outcome:** `docs/outcomes/20260816-state-residency/outcome.md`
**Advances:** success criterion 4 (spec half), and pre-clears the surface criteria 2/3 need.

## Objective

Give schema snapshots, source postures, and frozen-band probe baselines the one-sentence
absent-state behaviour the optionality rule demands, in their owning specs. Each must land on
one side of `state.md` §"The optionality rule"'s disjunction — degrade-and-say-so, or refuse
loudly by name — never silence. This phase is **spec-only**: no crate changes; the
implementation of these sentences lands in phase 2.

## Spec delta (this phase *is* the delta)

1. **`docs/specs/schema_evolution.md` §Semantics "Stored schemas"** — extend the existing
   "If `.smelt/schemas/` does not exist…" sentence into the full absent-state rule:
   **degrade-and-say-so**. With no snapshot for a model (never written, deleted, or excluded
   by `state.mode: stateless`), `smelt diff` reports it `new` and a migration proceeds as a
   first deployment; smelt never refuses and never infers a migration from an absent
   snapshot. Add the posture note: under `stateless` no snapshot is written, so every run
   sees every model as new — this changes what `smelt diff` can *tell* you, never what the
   deployed table equals. Cite `state.md` §"The optionality rule".

2. **`docs/specs/sources.md`** (the append-only posture-probe paragraph in §Semantics;
   cross-referenced from `model_properties.md` §"Probe obligation") — absent posture baseline
   is **degrade-and-say-so**: with no recorded baseline for a source partition the probe
   cannot compare, so the run *establishes* the baseline and reports the partition as
   unverified this run rather than asserting the declared posture held. Under
   `stateless` no baseline persists, so every run is an establishing run and the
   `append_only` declaration is never runtime-verified — reported, not silent.

3. **`docs/specs/incremental_models.md` §"The contract lattice"** (frozen-horizon paragraph) —
   generalise the existing first-run sentence into the absent-state rule: an absent frozen-band
   baseline (first run, deleted `.smelt/`, or a posture that excludes it) degrades the probe to
   baseline-establish-only and is reported as unverified; `ContractLateArrivalOutsideHorizon`
   cannot fire without a baseline. State the split explicitly: `frozen_horizon` degrades
   because its probe rides an **observability** baseline, whereas `contract.deferral` is
   `DeclaredContractRequiresState` because its lag is measured from the **correctness**-class
   frontier, which no posture can withhold — only a backend with no ledger builder can.

4. **`docs/specs/state.md`** — §Surface Diagnostics: add `ProbeBaselineUnavailable` (advisory,
   run-time: a declared fact's probe had no baseline to compare against and only established
   one; names the source/model, the partition set, and why the baseline was absent — absent
   posture or first observation). This is the single "say so" vehicle shared by deltas 2 and 3.
   §Known Divergences: rewrite the "Structure-level degradation behaviours are unevenly
   specified" bullet gap-first — the behaviours are now specified; the remaining gap is that
   the runtime does not yet honour them (`ProbeBaselineUnavailable` unimplemented, baselines
   written unconditionally), tracked by this outcome. Do **not** delete the bullet.

5. **`docs/specs/diagnostics.md`** — register `ProbeBaselineUnavailable` in the catalogue with
   its severity and owning spec, matching the existing entry format.

## Tests

Spec-only phase; the oracles are lint-shaped, run as commands under Verification rather than
new `#[test]`s. No red-green code test is appropriate here — inventing one would test the
markdown, not the feature. The implementation red-green for these sentences is phase 2's.

## Tasks

1. Read `state.md` §"The optionality rule" + §"Declarations stay fail-loud" as the oracle.
2. Apply delta 1 to `schema_evolution.md`.
3. Apply delta 2 to `sources.md`; add a one-line pointer from `model_properties.md`
   §"Probe obligation" if that row currently implies unconditional verification.
4. Apply delta 3 to `incremental_models.md` §"The contract lattice".
5. Apply delta 4 to `state.md` (§Surface table row + Known Divergences rewrite).
6. Apply delta 5 to `diagnostics.md`.
7. Grep-check every new `§"…"` cross-reference resolves to a real heading in the named file.

## Verification

- `bash .claude/scripts/verify-phase.sh` (no code changed; must stay green).
- Timeless-oracle lint on each edited spec:
  `rg -n 'Historical name|pre-cut|ratified|category error|Phase [A-Z0-9]' docs/specs/{state,sources,schema_evolution,incremental_models,diagnostics}.md`
  → no hits.
- Every `§"…"` introduced by this phase resolves: for each reference, `rg -n '^#{2,4} .*<name>' docs/specs/<file>.md` hits.
- `docs/specs/state.md` §"The state-structure inventory" still lists exactly the structures
  named in the deltas, with unchanged classes (this phase reclassifies nothing).
- Confirm no `crates/` file is touched: `git diff --stat -- crates/` is empty.

## Commit message

`spec(state): absent-state behaviour for schema snapshots, source postures, probe baselines`
