# Outcome: State residency — implement the state-ownership doctrine

**Created:** 2026-08-16
**Status:** active
**Source:** `docs/handoffs/2026-08-16-delta-signature-closure-programme.md` (programme outcome 1)
**Spec anchors:** `docs/specs/state.md`, `docs/specs/run_state.md`,
`docs/specs/incremental_models.md`

## The outcome

`docs/specs/state.md` stops being pure intent and becomes the implemented state-ownership
doctrine. `state.mode` is consulted by the runtime instead of merely parsed, so the optionality
rule holds: a project declares how much state it carries and the runtime writes (and depends on)
exactly that. The reconciliation ledger moves out of `.smelt/` into an engine-resident table
transactional with the fold it guards, closing the flagship gap — deleting `.smelt/` can never
again corrupt a keyed additive fold. Plan derivation gains the two-step
ideal-then-availability-resolution: an additive-graded cell on a backend without a ledger builder
downgrades to the recompute family with a recorded, explain-visible `MaintenanceStateDowngraded`
instead of failing loudly, and a declared contract that *requires* state refuses with
`DeclaredContractRequiresState`. A state-deletion conformance leg in the generative suite proves
the residency rule end to end — only possible once the ledger is in its final home, which is why
this outcome runs first in the programme.

## Success criteria (checkable)

1. **`state.mode` is consulted.** `execute_project` threads `StateMode` from config through to
   every state write/read; each mode behaves per `state.md`'s optionality rule (no unconditional
   `.smelt/` store creation regardless of mode). Closes `state.md` "The runtime ignores
   `state.mode` entirely."
2. **The reconciliation ledger is engine-resident.** Both gradings live in a backend table
   transactional with the fold, not `.smelt/reconciliation.json`; the additive grade's
   never-fold-twice check no longer rides on `.smelt/`. Closes `state.md` "The reconciliation
   ledger is `.smelt/`-resident" and the matching bullets in `run_state.md` /
   `incremental_models.md` §Known Divergences.
3. **Availability resolution exists in derivation.** The two-step ideal-then-availability
   derivation lands; an additive-graded cell on a ledger-less backend downgrades with a recorded
   `MaintenanceStateDowngraded` (visible in `smelt explain` and diagnostics), and
   `DeclaredContractRequiresState` refuses when the declared contract cannot be honoured without
   the unavailable state. Both §Surface diagnostic codes are implemented. Closes `state.md` "No
   availability-resolution step exists in derivation."
4. **Absent-state behaviour is specified everywhere the optionality rule requires.** Schema
   snapshots, source postures, and probe baselines each get their one-sentence absent-state
   behaviour in their owning specs (spec-first), and the implementation matches. Closes
   `state.md` "Structure-level degradation behaviours are unevenly specified."
5. **A state-deletion conformance leg exists**: the generative maintenance-conformance suite
   deletes `.smelt/` (and separately starts from a fresh clone) mid-sequence and equivalence
   still holds for every maintained model, including keyed additive folds.
6. All standing gates green (`verify-phase.sh`, `maintenance_conformance`, `statement_parity`,
   `walk_coverage`); `/smelt:validate state` reports no drift; every Known Divergences bullet
   this outcome claims is actually removed from the owning spec.

## Out of scope

- **Warehouse-bookkeeping opt-out knob** (`state.mode` refusing all smelt-authored objects in
  the target schema) — explicitly tagged Open Question in `state.md`; owned by the decision
  track, not this outcome.
- **A Spark-dialect ledger builder.** The downgrade path (criterion 3) is the required
  behaviour on ledger-less backends; whether to build a Spark ledger before a real workload
  demands it stays an open question per `incremental_models.md`.
- Scheduler consumption of delta signatures, per-source watermarks, `smelt explain` signature
  headline — programme outcome 2 (`scheduler-delta-signatures`).
- The definition-delta vertical — programme outcome 3.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec deltas first: one-sentence absent-state behaviour for schema snapshots, source postures, probe baselines in their owning specs; sharpen `state.md` §Surface where wiring needs it | blocked |
| 2 | Thread `StateMode` through `execute_project`: each mode gates exactly the state families `state.md` assigns it; red-green per mode. Includes implementing phase 1's absent-state behaviours (`ProbeBaselineUnavailable`, absent-snapshot degradation) so criterion 4's "implementation matches" half is met | pending |
| 3 | Move the reconciliation ledger engine-resident: backend table transactional with the fold, migration/read path for existing `.smelt/reconciliation.json`, never-fold-twice check rides the table | pending |
| 4 | Two-step ideal-then-availability derivation with recorded downgrades: `MaintenanceStateDowngraded` + `DeclaredContractRequiresState`, explain-visible | pending |
| 5 | State-deletion conformance leg: `.smelt/` deletion and fresh-clone steps in the generative suite, asserted against the oracle | pending |
| 6 | Docs-site update for state modes and residency; `/smelt:validate state`; remove closed Known Divergences bullets across `state.md`/`run_state.md`/`incremental_models.md` | pending |
| 7 | Close-out: full standing-gate sweep, outcome status flip | pending |

## Decision log

- **2026-08-16 (phase 1 plan).** Criterion 4 has two halves — spec sentence and matching
  implementation. Phase 1 is spec-only, so the implementation half is folded into phase 2's row
  (where posture gating already touches every baseline write site) rather than deferred out. No
  new phase row needed; phase 2's description widened.
- **2026-08-16 (phase 1 plan).** Resolved the frozen-horizon/deferral asymmetry that phase 1's
  spec delta 3 would otherwise leave ambiguous: `contract.frozen_horizon` **degrades** when its
  baseline is absent (the baseline is observability-class, and the spec already tolerates a
  baseline-establishing first run), while `contract.deferral` stays
  `DeclaredContractRequiresState` because its lag is measured from the correctness-class
  frontier, which no posture can withhold. Consistent with `state.md` §"Declarations stay
  fail-loud" naming deferral as the *one* exception.
- **2026-08-16 (phase 1 plan).** Added one advisory diagnostic, `ProbeBaselineUnavailable`, as
  the shared "say so" vehicle for absent probe baselines (source postures + frozen band). The
  optionality rule requires degradation be reported; without a code, delta 2 and delta 3 would
  specify silent degradation, which the rule forbids.

## Blocked

- **2026-08-16 (phase 1).** All five spec deltas landed (schema_evolution.md, sources.md,
  incremental_models.md §"The contract lattice", state.md §Surface Diagnostics +
  Known Divergences, diagnostics.md catalogue), and every phase-1-scoped verification passed
  (timeless-oracle lint, `§"…"` cross-reference resolution, state-structure-inventory
  unchanged, zero `crates/` diff, `cargo fmt --check`, `cargo clippy`, `example_diagnostics`).
  `bash .claude/scripts/verify-phase.sh` (full mode) is red for an unrelated, pre-existing
  reason confirmed via `git stash`: `cargo test -p smelt-logical --test contract_lattice_spec
  constraint_and_claude_md_state_the_lattice_invariant` fails looking for a
  `"### The contract, plan, and graph layer"` heading in `incremental_models.md` that the prior
  `spec-redraft-incremental-models` merge (PR #166, commit `14fa9e14`) removed without updating
  this standing gate — the failure reproduces identically on the pre-phase-1 commit, so it
  predates and is independent of this outcome's work. Fixing it means editing
  `crates/smelt-logical/tests/contract_lattice_spec.rs` (and/or restoring an
  `incremental_models.md` Known-Divergences heading), which this phase's own Verification
  explicitly forbids (`git diff --stat -- crates/` must be empty). Candidate options: (a) a
  tiny standalone fix — update the test's `section_body` lookup to the post-redraft heading
  structure (likely folded into `### Per-cell write addressing` / the plan matrix's Known
  Divergences prose) or restore an equivalent heading; (b) fold that fix into phase 2 (which
  already touches crates/) as a zero-scope-creep prerequisite step before phase 2's own red-green
  work; (c) open it as a standalone fast-follow outside this outcome. Spec work itself
  (docs/specs/{state,sources,schema_evolution,incremental_models,diagnostics}.md) is committed
  and sound regardless of which option is chosen.
