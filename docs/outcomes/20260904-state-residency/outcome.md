# Outcome: State residency — no correctness state outside the engine

**Created:** 2026-09-04
**Status:** active
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
| 1 | Spec delta: make `state.md` §Surface/§Semantics the sole normative statement of ledger residency and availability resolution; align `run_state.md`/`incremental_models.md`/`incremental_shapes.md` cross-references; add both codes to `diagnostics.md` | done |
| 2 | Engine-resident reconciliation ledger on DuckDB: the region-recompute reset joins the already-engine-resident fold on `_smelt_ledger`, transactional with the batch write; delete `.smelt/reconciliation.json` and its `smelt-state` file-store API | done |
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

- 2026-09-04 (plan 01): outcome set `active`; no phase reshape — this is the outcome's first
  phase, there is no prior summary, and reading the five spec anchors confirmed the table's
  shape still holds.
- 2026-09-04 (plan 01): the spec anchors are further along than the outcome assumed —
  `state.md` §Surface, §Semantics and §Diagnostics already state residency, the degradation
  contract and both codes normatively, and `incremental_models.md` line 1389 already defers to
  them. Phase 1 is therefore mostly *de-restatement* in the three consuming specs plus the
  `diagnostics.md` catalogue rows, not a fresh normative draft.
- 2026-09-04 (plan 01): phase 1 widened to strike `reconciliation.json` from `run_state.md`'s
  §Layout tree, locking sentence, atomic-write sentence and **Fixed layout** invariant. Those
  four sites assert `.smelt/` residency as normative layout, which is exactly what criterion 1
  reverses; leaving them would make phase 2's implementation contradict a spec body.
- 2026-09-04 (plan 01): the `diagnostics.md` coverage gate asserts enum → catalogue only, so
  catalogue rows may precede their `DiagnosticCode` variants (the posture already used by the
  `Maintenance*` and contract-lattice rows). Phase 1 adds the two rows now and records the
  catalogue-ahead-of-variant gap; phases 4-5 land the variants.
- 2026-09-04 (phase 1 implement): landed the spec delta — `state.md` cross-reference sentence
  and outcome-linked divergences, `run_state.md` de-restatement (layout tree, locking/atomic-write
  sentences, Fixed-layout invariant, §"Relationship to the reconciliation ledger"),
  `incremental_models.md`/`incremental_shapes.md` divergence-bullet updates, and the new
  `diagnostics.md` §"State residency" catalogue section. Both `rg` sweeps and
  `verify-phase.sh`/`diagnostics_catalogue` gates confirmed clean; see
  `phases/01-summary.md`.

- 2026-09-04 (plan 02): phase 2's row said the ledger DDL/DML would be emitted from
  `smelt-logical`'s maintenance layer. That contradicts the standing maintenance-plan-purity
  invariant, which explicitly excludes "ledger DDL/DML in `smelt-state`" as bookkeeping, and the
  existing `_smelt_ledger` builders already live in `smelt-state/src/ddl_duckdb.rs`. Row reworded
  to keep the ledger builders in `smelt-state`; no new emitter is added to `smelt-logical`.
- 2026-09-04 (plan 02): reading the code showed the ledger's **fold** half is already
  engine-resident (MP12 `_smelt_ledger` + `Backend::fold_ledger_delta`). The only remaining
  production writer of `.smelt/reconciliation.json` is the post-batch-loop region-recompute reset
  in `execute.rs`, so phase 2 is scoped to that one move plus deleting the file-store API. No
  phase added or removed — criterion 1's remaining surface is smaller than the row implied, not
  different.
- 2026-09-04 (plan 02): transactional coupling reuses the existing
  `Backend::execute_write_with_bookkeeping` seam (DuckDB overrides it with a real transaction) via
  a new `execute_model_incremental_with_bookkeeping` default, rather than a second write path —
  the run-pipeline-parity rule keeps the write in one place.

- 2026-09-04 (phase 2 implement): landed the engine-resident region-recompute reset —
  `generate_ledger_recompute_reset_sqls` (smelt-state), `execute_model_incremental_with_bookkeeping`
  (smelt-backend), and the DuckDB DeleteInsert batch-write call site in `execute.rs`;
  `.smelt/reconciliation.json` and its file-store API deleted. Scoped to the plain (non-delta-restricted)
  DeleteInsert branch only, per the plan; the delta-restricted/column-scoped-merge write path
  (`execute_delete_insert_with_delta_restriction`) now has no reconciliation-reset at all, a gap
  surfaced (not fixed) for phase 3 or later to resolve — see `phases/02-summary.md`. All gates
  green (`verify-phase.sh`, `statement_parity`, `execute_parity`, `maintenance_conformance`).

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->
