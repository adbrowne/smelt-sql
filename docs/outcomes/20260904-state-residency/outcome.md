# Outcome: State residency — no correctness state outside the engine

**Created:** 2026-09-04
**Status:** queued
**Source:** `docs/specs/state.md` §Known Divergences (all five bullets); `docs/research/20260904-incremental-state-review.md` §"Recommended next sequence" items 2 and 3; `docs/handoffs/2026-08-16-delta-signature-closure-programme.md` (the `state-residency` outcome it put first); `docs/outcomes/20260815-keyed-grain-residue/outcome.md` §Blocked (phase 3)
**Spec anchors:** `docs/specs/state.md`, `docs/specs/run_state.md` §"Relationship to the reconciliation ledger", `docs/specs/incremental_models.md` §Known Divergences (ledger residency), `docs/specs/incremental_shapes.md` §"The transactional frontier write (merge ledger)"

## The outcome

Deleting `.smelt/` never changes what a maintained model computes. The reconciliation ledger
(both gradings) lives in a backend table transactional with the fold it protects, not in
`.smelt/reconciliation.json`. `state.mode` is honoured: `stateless` writes nothing under
`.smelt/`, and every state structure degrades exactly as `state.md` §"The degradation contract"
specifies. Plan derivation has an availability-resolution step: a cell whose technique needs a
structure the target backend cannot realise is downgraded to its recompute-family equivalent and
the downgrade is recorded as `MaintenanceStateDowngraded`, printed by `smelt explain`; a declared
contract point that needs an unavailable structure refuses with `DeclaredContractRequiresState`.
`state.warehouse_tables: none` is parsed and feeds that resolution. The conformance gate proves
the invariant by deleting `.smelt/` between run steps.

## Success criteria (checkable)

1. `crates/smelt-state` no longer owns a `.smelt/reconciliation.json`; the reconciliation
   ledger's fold and its never-fold-twice check execute against an engine-resident table in the
   same transaction as the maintained write, on DuckDB (statement-parity gate extended to cover
   the ledger statements).
2. `StateMode` is consulted by `execute_project`: under `stateless` no file is created under
   `.smelt/`; under each other posture only the structures `state.md` §"`state.mode` and what
   each posture provides" lists are written. A test per posture asserts the on-disk set.
3. Availability resolution exists as a pure function in `smelt-logical`'s maintenance layer
   (maintenance-plan purity); on a backend without a ledger realisation an additive-graded cell
   downgrades to its recompute-family equivalent with a `MaintenanceStateDowngraded` record,
   instead of failing loudly or skipping. The keyed-grain residue outcome's
   `state_structure_unavailable` reporter event is replaced by this recorded downgrade.
4. `MaintenanceStateDowngraded` is printed by `smelt explain` (text and `--json`) and surfaced
   as a warning-level diagnostic; `DeclaredContractRequiresState` refuses a `contract.deferral`
   declaration on a target that cannot supply the ledger. Both codes are in
   `docs/specs/diagnostics.md`.
5. `state.warehouse_tables` is parsed; `none` makes every engine-resident structure unavailable
   to resolution, with the two diagnostics above as the only consequences.
6. `cargo test -p smelt-cli --test maintenance_conformance` has a leg that interleaves `.smelt/`
   deletion between run steps for every maintained recipe and asserts the equivalence oracle
   still holds (the "Conformance gate leg for state deletion" Future Extension, now built).
7. `docs/outcomes/20260815-keyed-grain-residue/outcome.md` criterion 3 is amended to "a
   ledger-less backend takes a recorded, explain-visible downgrade" (option 1 of its blocked
   entry, decided 2026-09-04), phase 3 is closed against this outcome's criterion 3, and that
   outcome's Status becomes `done`.
8. All five `state.md` §Known Divergences bullets are deleted or rewritten to a residual gap;
   `/smelt:validate state` clean; docs-site `guide/` pages that describe `.smelt/` state are
   updated; all standing gates green (`verify-phase.sh`, `statement_parity`, `execute_parity`,
   `maintenance_conformance`, `walk_coverage`).

## Out of scope

- A Spark or BigQuery ledger builder. `docs/research/20260816-open-questions-triage.md` item 12
  decides those backends take the recorded downgrade path; that is the intended steady state.
- The pluggable OLTP observability store (`state.md` §Future Extensions).
- Instant-keyed intervals and the scheduler's typed delta currency
  (`run_state.md` §Known Divergences bullet 1; `scheduler-delta-signatures`, which needs a
  human-reviewed first plan).
- The virtual-environments snapshot/environment store (`run_state.md` §Known Divergences).

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec delta: make `state.md` §Surface/§Semantics the sole normative statement of ledger residency and availability resolution; align `run_state.md`/`incremental_models.md`/`incremental_shapes.md` cross-references; add both codes to `diagnostics.md` | pending |
| 2 | Engine-resident reconciliation ledger on DuckDB: table DDL/DML emitted from `smelt-logical`'s maintenance layer, fold + never-fold-twice check transactional with the write; delete `.smelt/reconciliation.json` | pending |
| 3 | Statement-parity and keyed-frontier tests cover the ledger statements; conformance gate green with the file ledger gone | pending |
| 4 | Availability resolution as a pure derivation step: `MaintenanceStateDowngraded` record on the cell, recompute-family downgrade, `state.warehouse_tables` parsed and fed in | pending |
| 5 | Surface: `smelt explain` prints downgrades; LSP/CLI warning; `DeclaredContractRequiresState` validation refusal; replace the keyed-grain `state_structure_unavailable` skip with the recorded downgrade | pending |
| 6 | `state.mode` honoured in `execute_project`: per-posture write set, `stateless` writes nothing, `--resume`/propagation degrade per spec; per-posture tests | pending |
| 7 | Conformance-gate leg: `.smelt/` deletion interleaved between run steps for every maintained recipe | pending |
| 8 | Close the keyed-grain residue outcome (amend criterion 3, mark phase 3 and the outcome done); docs-site state pages | pending |
| 9 | Validate + close out: `/smelt:validate state` clean, `state.md` divergences rewritten, all gates green | pending |

## Decision log

- 2026-09-04 (scaffold): Andrew accepted the review's option 1 for the keyed-grain residue
  outcome's blocked phase 3 — a ledger-less backend takes a recorded, explain-visible downgrade
  rather than gaining a ledger builder. This outcome owns that closure (criterion 7).

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->
