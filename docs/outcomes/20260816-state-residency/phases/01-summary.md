# Phase 1 summary — spec deltas: absent-state behaviour

## Shipped

- `docs/specs/schema_evolution.md` §Semantics "Stored schemas" — absent-snapshot rule
  generalised to the full degrade-and-say-so statement, with the `stateless` posture note.
- `docs/specs/sources.md` §Semantics item 4 — absent posture-baseline rule (degrade-and-say-so),
  naming `ProbeBaselineUnavailable`.
- `docs/specs/model_properties.md` §"Probe obligation" registry row for the append-only posture
  probe — one-sentence pointer to the absent-baseline behaviour so the row no longer implies
  unconditional verification.
- `docs/specs/incremental_models.md` §"The contract lattice" frozen-horizon paragraph —
  generalised first-run sentence into the absent-baseline rule, plus the explicit
  frozen_horizon-degrades / deferral-refuses split (observability vs correctness baseline).
- `docs/specs/state.md` §Surface "Diagnostics" — new `ProbeBaselineUnavailable` row (advisory).
  §Known Divergences — "Structure-level degradation behaviours" bullet rewritten gap-first: the
  behaviours are now specified, the remaining gap is runtime non-conformance, tracked by this
  outcome.
- `docs/specs/diagnostics.md` — new `### State` catalogue section registering
  `ProbeBaselineUnavailable`.

## Decisions

- Frozen-horizon degrades, deferral refuses — cited to `state.md`'s
  observability/correctness class split, not asserted freshly (already logged in outcome.md's
  decision log from planning).
- `ProbeBaselineUnavailable` cross-references use the bare `file.md §"Heading"` convention
  already established in `incremental_models.md`'s existing `state.md` citations, not the
  `§Surface "Heading"` convention seen in some other specs — picked for local consistency
  within this citation cluster.

## For the next planner

- **Pre-existing, unrelated standing-gate regression discovered and NOT fixed (out of this
  phase's spec-only scope):** `cargo test -p smelt-logical --test contract_lattice_spec
  constraint_and_claude_md_state_the_lattice_invariant` fails looking for a
  `"### The contract, plan, and graph layer"` heading that the `spec-redraft-incremental-models`
  merge (PR #166 / commit `14fa9e14`) removed from `incremental_models.md` without updating this
  test. Reproduced independent of this phase via `git stash`. See outcome.md's Blocked entry for
  full detail and candidate fixes (a standalone crates/-touching fix, folded into phase 2, or a
  separate fast-follow). This must be scheduled — it is a real hole in a standing CI gate.
- Phase 2's plan should double-check whether restoring/relocating the missing heading also
  serves phase 2's own work (phase 2 already touches `crates/`), which would make option (b)
  from the Blocked entry free.

## Gates

- `bash .claude/scripts/verify-phase.sh --fast` — PASS (fmt, clippy, example_diagnostics).
- `bash .claude/scripts/verify-phase.sh` (full) — FAIL: `cargo test` workspace run fails on
  `contract_lattice_spec::constraint_and_claude_md_state_the_lattice_invariant` — confirmed
  pre-existing via `git stash` (fails identically on the pre-phase-1 commit).
- Timeless-oracle lint (`rg -n 'Historical name|pre-cut|ratified|category error|Phase
  [A-Z0-9]' docs/specs/{state,sources,schema_evolution,incremental_models,diagnostics}.md`) —
  clean (only self-referential matches inside each file's own Timeless-oracle-rule preamble,
  pre-existing).
- All new `§"…"` cross-references resolve (`rg -n '^#{2,4} .*<name>'` per target file) — verified
  for `state.md §"Diagnostics"`, `state.md §"The optionality rule"`,
  `incremental_models.md §"The contract lattice"`.
- `docs/specs/state.md` §"The state-structure inventory" — unchanged (`git diff` shows no hunk
  in that table).
- `git diff --stat -- crates/` — empty.
