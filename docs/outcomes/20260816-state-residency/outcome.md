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
| 2 | Repair the pre-existing `contract_lattice_spec` heading-lookup regression (phase 1's Blocked entry, option (b)), then thread `StateMode` through `execute_project`: `FileStore` carries the project posture and each observability write is gated to exactly the families `state.md` §"`state.mode` and what each posture provides" assigns it; `--resume` refuses by name under `stateless` | done |
| 3 | Repair the second pre-existing red-gate class (`output_delta_spec` / `typed_edge_spec` duplicate-`### The graph layer` lookup + the `General` verdict-name judgment call), then absent-state runtime behaviours (criterion 4's "implementation matches" half): `ProbeBaselineUnavailable` emitted for absent source-posture and frozen-band baselines, absent-schema-snapshot degradation per `schema_evolution.md` | done |
| 4 | Move the reconciliation ledger engine-resident: backend table transactional with the fold, migration/read path for existing `.smelt/reconciliation.json`, never-fold-twice check rides the table | planned |
| 5 | Two-step ideal-then-availability derivation with recorded downgrades: `MaintenanceStateDowngraded` + `DeclaredContractRequiresState`, explain-visible | pending |
| 6 | State-deletion conformance leg: `.smelt/` deletion and fresh-clone steps in the generative suite, asserted against the oracle | pending |
| 7 | Docs-site update for state modes and residency; `/smelt:validate state`; remove closed Known Divergences bullets across `state.md`/`run_state.md`/`incremental_models.md` | pending |
| 8 | Close-out: full standing-gate sweep, outcome status flip | pending |

## Decision log

- **2026-08-16 (phase 4 plan).** No reshape of the remaining rows. Planning phase 4 established
  that `state.md` §Known Divergences overstates the gap: the **additive** grading is already
  engine-resident (`_smelt_ledger`, whose `PRIMARY KEY` *is* the never-fold-twice key, committed
  with the fold by `Backend::fold_ledger_delta`'s DuckDB transactional override). Only the
  **idempotent/frontier** grading still lives in `.smelt/reconciliation.json`, and nothing in
  production reads that file — phase 4 moves a write, not a decision. Criterion 2's residual work
  is therefore exactly the frontier record's move plus the legacy-file import; the phase 4 plan
  also corrects the false Known Divergences bullets rather than leaving them for phase 7's sweep
  (a bullet that is false the moment the code lands is drift, not deferred work).
- **2026-08-16 (phase 4 plan).** Two engine tables, not one graded table: the frontier record gets
  its own `_smelt_frontier` rather than a `grade` column on `_smelt_ledger`. Adding a column would
  require a warehouse-side migration of every existing `_smelt_ledger`, and both paths key the
  whole-row group `{*}`, so a frontier reset's intersecting-region `DELETE` would otherwise wipe
  additive delta-identity rows. Also: no `state_version` bump for the removal of
  `reconciliation.json` — the file is consumed and deleted as a legacy artifact rather than being a
  layout version difference, and no binary in either direction ever read it for a decision.
- **2026-08-16 (phase 4 plan).** On a dialect with no ledger builder (everything but DuckDB; a
  Spark builder is out of scope for this outcome) phase 4 skips the frontier record with a
  `tracing::warn!` and leaves any legacy file in place. That interim say-so becomes phase 5's
  recorded, explain-visible `MaintenanceStateDowngraded` — kept inside the outcome, not deferred.

- **2026-08-16 (phase 3 implement).** Phase 3 landed: the second pre-existing
  red-gate class is fixed (`section_body` in `output_delta_spec.rs`/
  `typed_edge_spec.rs` now searches only after `## Semantics`, so it cannot
  match the Overview primer's restated heading), and `incremental_models.md`
  §"The graph layer" now names `KeyedUpsert`/`General` explicitly alongside
  its lowercase prose — the judgment call from phase 2's discovery landed as
  a spec edit naming the owning verdict type, not a test weakening.
  `ProbeRecordOutcome::BaselineEstablished` plus `RunReporter::probe_advisory`
  are wired at both absent-baseline sites (source posture, frozen band) and
  at `smelt diff`'s absent-schema-snapshot path; `execute.rs`'s per-model
  `EventSink` event buffer needed a `ProbeAdvisory` variant too, or advisories
  from concurrent model execution were silently dropped on replay — caught by
  a debug test before shipping. All phase-3 gates green, including the full
  `verify-phase.sh` sweep. `.claude/hardening-baseline.txt`'s `smelt-cli
  println` count moved 161→163 via the gate's own `--update` path (both new
  lines are intentional user-facing output; the ratchet's substring match
  also counts the new `eprintln!`). See `phases/03-summary.md` for details.
- **2026-08-16 (phase 3 plan).** Folded phase 2's newly-discovered red-gate class
  (`smelt-logical --test output_delta_spec` / `--test typed_edge_spec`, duplicate
  `### The graph layer` headings in `incremental_models.md` plus the lowercase `general` vs
  `General` verdict-name question) into phase 3 as task 1, rather than opening a new row or
  deferring it. Criterion 6 requires every standing gate green, so it cannot leave the outcome;
  and phase 3 is the first phase after the discovery that may touch `docs/specs/` (its own spec
  delta is a `run_state.md` edit), which is what phase 2's constraint forbade. Unlike phase 2's
  mechanical repair, this one carries a genuine judgment call — the plan names the decision rule
  rather than pre-deciding it.
- **2026-08-16 (phase 3 plan).** Phase 3 needs a small spec delta of its own: the run-manifest
  probe-record `outcome` vocabulary in `run_state.md` §"Run manifest" is currently
  `"dispatched" | "skipped"`, and an established-not-compared probe is neither. Spec-first, so
  the vocabulary gains `"baseline_established"` before the code does. This is inside criterion 4
  (the absent-state behaviour has to be *reported*, and the manifest is where a run's probe
  outcomes are durably reported), not new scope.

- **2026-08-16 (phase 2 implement).** Phase 2 landed: `FileStore` now carries `StateMode`,
  gates every observability family per `state.md`'s consequence table, and `--resume` refuses
  by name under `stateless`. The `contract_lattice_spec` repair (phase 1's Blocked entry,
  option (b)) is done and verified. Discovered — but deliberately did not fix, to stay inside
  the "no `docs/specs/` edits" constraint — a second, distinct pre-existing red-gate class in
  `smelt-logical --test output_delta_spec` and `--test typed_edge_spec`: `incremental_models.md`
  has two `### The graph layer` headings post PR #166, and even the correct section's prose
  uses lowercase `general` (delta-signature verdict) where the test expects capitalized
  `General` (output-delta profile verdict) — a spec-content judgment call, not a mechanical
  pointer fix. See `phases/02-summary.md` "For the next planner" for the full analysis; a
  follow-up phase/task should resolve it.
- **2026-08-16 (phase 2 plan).** Split the old row 2 into two rows (posture threading; absent-state
  runtime behaviours) and renumbered the tail 4–8. The old row bundled two independently testable
  changes with different oracles — posture gating is verified by "which files exist after a run",
  absent-state degradation by "which diagnostic is emitted" — and neither leaves the outcome:
  criterion 1 is row 2, criterion 4's implementation half is row 3.
- **2026-08-16 (phase 2 plan).** Adopted option (b) of phase 1's Blocked entry: the
  `contract_lattice_spec::constraint_and_claude_md_state_the_lattice_invariant` regression is
  repaired as phase 2's first task. Phase 2 already touches `crates/`, and no standing gate may
  stay red while this outcome adds work behind it. The repair is test-side: post-redraft the
  lattice-point invariant is a §Constraints & Invariants *bullet*, not a `###` subsection, so the
  test's `section_body("### The contract, plan, and graph layer")` lookup becomes an
  `h2_section_body("## Constraints & Invariants")` lookup asserting the same two substrings. No
  spec text changes; the invariant's strength is unchanged.
- **2026-08-16 (phase 2 plan).** Phase 1's row stays `blocked` (its deliverables all landed and
  were verified per `phases/01-summary.md`; only the unrelated red gate blocked it). Phase 8's
  close-out judges criterion 4 against phases 1 and 3 together.
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
