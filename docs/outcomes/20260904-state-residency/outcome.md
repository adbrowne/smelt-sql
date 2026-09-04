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
| 3 | Wire the ledger reset into the delta-restricted write path; statement-parity and keyed-frontier tests cover the ledger statements (incl. transactional rollback); conformance gate green with the file ledger gone | done |
| 4 | Availability resolution as a pure `smelt-logical` step: `StateStructure` inventory, per-technique requirement, recompute-family downgrade recorded on the cell; `state.warehouse_tables` parsed (`smelt_yml.md` row) and expressible as an availability input | done |
| 5 | Wire resolution into the plan-derivation seam: every runtime consumer reads a resolved plan; the non-DuckDB ledger skip becomes a recorded downgrade instead of a `state_structure_unavailable` reporter call | planned |
| 6 | Surface: `smelt explain` prints downgrades (text + `--json`); LSP/CLI warning diagnostic; `DeclaredContractRequiresState` validation refusal; replace the keyed-grain `state_structure_unavailable` skip with the recorded downgrade | pending |
| 7 | `state.mode` honoured in `execute_project`: per-posture write set, `stateless` writes nothing, `--resume`/propagation degrade per spec; per-posture tests | pending |
| 8 | Conformance-gate leg: `.smelt/` deletion interleaved between run steps for every maintained recipe | pending |
| 9 | Close the keyed-grain residue outcome (amend criterion 3, mark phase 3 and the outcome done); docs-site state pages | pending |
| 10 | Validate + close out: `/smelt:validate state` clean, `state.md` divergences rewritten, all gates green | pending |

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

- 2026-09-04 (plan 03): phase 3's row widened to include wiring the `_smelt_ledger` recompute
  reset into the delta-restricted / column-scoped-merge write path
  (`execute_delete_insert_with_delta_restriction`), the gap phase 2 surfaced. That path currently
  records no reconciliation entry at all — a regression against criterion 1's "the ledger's fold
  and its never-fold-twice check execute against an engine-resident table in the same transaction
  as the maintained write" — so it is criterion-serving work and cannot be deferred out. No new
  phase: the fix is two call sites plus two parameters, and the phase's own parity tests are its
  natural coverage.
- 2026-09-04 (plan 03): reading `RecordingBackend` confirmed it does not override
  `execute_write_with_bookkeeping`, so the trait default routes ledger DDL/DML through the
  recorded `execute_sql`/`execute_statement_group` — the statement-parity leg needs no new
  recording seam. The "same transaction" half of criterion 1 is therefore proven separately, by a
  rollback test driving `DuckDbBackend::execute_write_with_bookkeeping` with a failing write group.

- 2026-09-04 (phase 3 implement): landed — `execute_delete_insert_with_delta_restriction` gained
  `ensure_sqls`/`pre_write_sqls` params routing through `execute_write_with_bookkeeping`; the
  ledger reset construction in `execute.rs` is hoisted once per batch above all three DuckDB
  DeleteInsert dispatch arms (model-edge restricted, external-sidecar restricted, plain), closing
  phase 2's surfaced gap where the two restricted branches recorded no reset at all. Six new/
  extended tests lock byte-parity, the rollback-under-failure guarantee, and the documented
  non-DuckDB skip (a red test for phase 4's `MaintenanceStateDowngraded` to turn green). All
  gates green (`verify-phase.sh`, `statement_parity`, `execute_parity`, `keyed_frontier_
  bookkeeping`, `maintenance_conformance`); `rg` confirms no production `reconciliation.json`
  reader/writer remains. See `phases/03-summary.md`.

- 2026-09-04 (plan 04): phase 4 split in two. The old row bundled the pure derivation step with
  "fed in", but the plan is derived at ~10 `smelt-db` `derive_model_maintenance_plan{,_with_edges}`
  call sites in `smelt-runtime`'s maintenance driver, none of which carry backend/config facts
  today — wiring is a distinct, mechanical, separately-reviewable change. New phase 4 = the pure
  function plus the config key; new phase 5 = the wiring seam (which also retires phase 3's
  documented non-DuckDB `state_structure_unavailable` skip). Old phases 5-9 renumber to 6-10; no
  criterion-serving work left the outcome.
- 2026-09-04 (plan 04): `state.warehouse_tables` is user-visible `smelt.yml` surface, so phase 4
  carries a small spec delta after all — `smelt_yml.md` §"Top-level keys" row for `state` lists
  only `mode:` today. `state.md` §"Opting out of warehouse bookkeeping" is already normative
  (phase 1), so the delta is one table row pointing at it, not a new normative statement.
- 2026-09-04 (plan 04): the downgrade record lands as a `state_downgrade: Option<StateDowngrade>`
  field on `PlanCell` (spec: "recorded on the cell"), which costs a mechanical
  `state_downgrade: None` in the ~38 existing `PlanCell` literals. A side-table on
  `MaintenancePlan` was rejected: it would let a consumer read a cell's technique without its
  downgrade, which is exactly the silent substitution §"The degradation contract" forbids.

- 2026-09-04 (phase 4 implement): landed the pure availability-resolution step —
  `crates/smelt-logical/src/maintenance/availability.rs` (`StateStructure`,
  `required_state_structure`, `StateAvailability`, `recompute_equivalent`,
  `resolve_availability`), `PlanCell::state_downgrade`, and `state.warehouse_tables` parsing in
  `smelt-core`. All 10 tests green; `verify-phase.sh` and the unchanged-gates checks
  (`walk_coverage`, `statement_parity`, `execute_parity`) all green. No consumer calls
  `resolve_availability` yet — phase 5's wiring seam. Also fixed an unrelated pre-existing
  `verify-phase.sh` failure (`partition_grain_residues_stay_closed` stale after the 2026-09-04
  decision track's `data_latency` retirement) since it blocked the mandatory gate for every
  phase; see `phases/04-summary.md` for detail and the decision-residue follow-up note.

- 2026-09-04 (plan 05): no phase reshape. Phase 4's summary flagged that no backend
  "realisable structures" enumeration exists; phase 5 adds it as an exhaustive
  `realisable_state_structures(SqlDialect)` table in `smelt-logical`'s `availability.rs` rather
  than a `Backend` trait query — the mapping is pure data (maintenance-plan purity) and
  `smelt-state`, where the only ledger builders live (`ddl_duckdb.rs`), cannot host it because it
  does not depend on `smelt-logical`.
- 2026-09-04 (plan 05): the ~12 `derive_model_maintenance_plan{,_with_edges}` call sites in
  `smelt-runtime` are routed through one new seam module (`maintenance_availability.rs`) with an
  `rg`-based structural gate, instead of calling `resolve_availability` at each site — a
  per-site call is exactly the "consumer re-derives the plan" shape the maintenance-plan-purity
  invariant forbids.
- 2026-09-04 (plan 05): `smelt-db`'s own derivation sites (diagnostics, `lib.rs:2111`) stay
  unresolved in this phase. They are analysis-time, where the target dialect is not known, and
  the diagnostic channel for a downgrade is phase 6's work; criterion 3 speaks of runtime
  consumers.

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->
