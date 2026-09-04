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
- The `Config.targets` iteration-order fix (`resolve_default_target` is a stopgap; the real fix is
  a "no target declared and 2+ targets" diagnostic or an ordered map). Flagged by phase 6; no
  success criterion depends on it, and it is a repo-wide config change.
- Confirming/removing the now-likely-dead ledger-less `else` branch in
  `maintenance_driver.rs::run_windowed_keyed_maintenance` (phase 6's flag). Availability
  resolution already makes it unreachable in production, so no criterion depends on the cleanup.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec delta: make `state.md` §Surface/§Semantics the sole normative statement of ledger residency and availability resolution; align `run_state.md`/`incremental_models.md`/`incremental_shapes.md` cross-references; add both codes to `diagnostics.md` | done |
| 2 | Engine-resident reconciliation ledger on DuckDB: the region-recompute reset joins the already-engine-resident fold on `_smelt_ledger`, transactional with the batch write; delete `.smelt/reconciliation.json` and its `smelt-state` file-store API | done |
| 3 | Wire the ledger reset into the delta-restricted write path; statement-parity and keyed-frontier tests cover the ledger statements (incl. transactional rollback); conformance gate green with the file ledger gone | done |
| 4 | Availability resolution as a pure `smelt-logical` step: `StateStructure` inventory, per-technique requirement, recompute-family downgrade recorded on the cell; `state.warehouse_tables` parsed (`smelt_yml.md` row) and expressible as an availability input | done |
| 5 | Wire resolution into the plan-derivation seam: every runtime consumer reads a resolved plan; the non-DuckDB ledger skip becomes a recorded downgrade instead of a `state_structure_unavailable` reporter call | done |
| 6 | Run/explain surface: `smelt explain` prints the recorded downgrade (text + `--json`); the keyed-grain merge-ledger skip becomes the recorded downgrade and `RunReporter::state_structure_unavailable` is retired | done |
| 7 | Analysis surface: `MaintenanceStateDowngraded` warning diagnostic and `DeclaredContractRequiresState` validation refusal as `DiagnosticCode` variants emitted from the pure `maintenance_plan_diagnostics` owner (LSP + CLI) | done |
| 8 | `state.mode` honoured in `execute_project`: per-posture write set, `stateless` writes nothing, `--resume`/propagation degrade per spec; per-posture tests | done |
| 9 | Conformance-gate leg: `.smelt/` deletion interleaved between run steps for every maintained recipe | done |
| 10 | Close the keyed-grain residue outcome (amend criterion 3, mark phase 3 and the outcome done); docs-site state pages | done |
| 11 | Validate + close out: `/smelt:validate state` clean, `state.md` divergences rewritten, all gates green | done |

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
- 2026-09-05 (phase 5 implement): landed the single `smelt-runtime` availability-resolution seam
  (`maintenance_availability.rs`) and threaded `availability: &StateAvailability` through all 9
  `maintenance_driver.rs` resolvers plus `propagation.rs`'s graph walk (`StateAvailability::all()`
  there — that consumer never reads `technique`/`state_downgrade`). The `execute.rs` ledger-reset
  site now gates on `availability.contains(ReconciliationLedger)` instead of a raw DuckDB dialect
  check, and no longer calls `reporter.state_structure_unavailable(...)`. See `phases/05-summary.md`
  — phase 6 still owns the keyed-grain caller of that reporter method and the `smelt explain`
  surface.

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

- 2026-09-05 (plan 06): phase 6 split in two. The old row bundled four separable
  deliverables across three crates; the two diagnostic codes are analysis-time work with a single
  owner (`smelt-db`'s pure `maintenance_plan_diagnostics`, which already receives
  `active_backends`) and their own gates (`diagnostics_catalogue`, `example_diagnostics`), while
  the explain rendering and the reporter retirement are runtime/CLI surface. New phase 6 = the
  run/explain surface + retiring `RunReporter::state_structure_unavailable`; new phase 7 = the two
  `DiagnosticCode` variants and the `DeclaredContractRequiresState` refusal. Old phases 7-10
  renumber to 8-11; no criterion-serving work left the outcome (criteria 4 and 5 are now served
  jointly by phases 6 and 7).
- 2026-09-05 (plan 06): `smelt explain` resolves availability at its own call site (declared target
  dialect + `state.warehouse_tables`, offline) rather than through `smelt-db`'s Salsa
  `maintenance_plan_report`. Plan 05 recorded that `smelt-db`'s derivation sites stay unresolved
  because analysis time has no single target dialect; explain does know the model's declared
  target, so resolving there uses the single-owner pure `resolve_availability` without pushing a
  dialect into the Salsa query.
- 2026-09-05 (plan 06): the keyed-grain merge-ledger skip (`maintenance_driver.rs`) is replaced
  the same way phase 5 replaced the `execute.rs` ledger-reset skip — a `tracing::debug!` pointing
  at the cell's recorded `state_downgrade`, which is now genuinely user-visible via `smelt
  explain`. With both callers gone, `RunReporter::state_structure_unavailable` and its buffered
  `ReporterEvent` variant are deleted rather than left dead.

- 2026-09-05 (phase 6 implement): landed the explain surface (text row, `--json` field) and
  retired `RunReporter::state_structure_unavailable` entirely (trait, `CliReporter` impl, buffered
  `ReporterEvent` variant + replay + test, three test-local capturing-reporter impls). Also fixed
  a phase-6-exposed pre-existing bug: `smelt explain`'s default-target fallback used
  `HashMap::keys().next()` (randomized per process), which was harmless before this phase but,
  once dialect started feeding availability resolution, made a two-target project's
  ledger-requiring cell nondeterministically Admitted/downgraded across runs — caught as a flaky
  `crates/smelt-cli/tests/explain.rs::technique_flag_renders_named_technique`. New
  `resolve_default_target` (prefers `config.target`, else sorted-first) fixes it; the real fix
  (a "no target declared, 2+ targets" diagnostic, or an ordered `Config.targets` map) is a
  follow-up. See `phases/06-summary.md` for the full account, including a plan deviation (the
  "records no reporter event" test landed as an internal `maintenance_driver.rs` unit test rather
  than in `keyed_frontier_bookkeeping.rs`, which has no dialect-override seam) and a real fix to
  `explain_show_sql.rs`'s BigQuery-median test (scoped its assertions to the `DeleteInsert` cell,
  since a `ColumnScopedMerge` repair cell now correctly downgrades and its `s.*` copy statement
  was never the thing under test).

- 2026-09-05 (plan 07): no phase reshape — phase 6's split already isolated this phase's scope, and
  reading the code confirmed the two codes have exactly one pure owner to come from. Two items
  phase 6 flagged for a follow-up (`Config.targets` ordering; the dead ledger-less branch in
  `run_windowed_keyed_maintenance`) are recorded under "## Out of scope": neither serves a success
  criterion.
- 2026-09-05 (plan 07): both diagnostics are emitted from `maintenance_plan_diagnostics`, including
  the `contract.deferral` refusal — rather than at `check_file_diagnostics`'s existing
  `validate_deferral` sites. That keeps the availability inputs (declared backends +
  `state.warehouse_tables`) assembled once, in one pure function, instead of a second ad hoc
  resolution in the Salsa wrapper.
- 2026-09-05 (plan 07): the analysis-time resolution runs over a **clone** of the derived cells.
  Plan 05 recorded that `smelt-db`'s derivation sites stay unresolved because analysis time has no
  single target dialect; this phase keeps that true for the returned plan/report (which
  `smelt explain` and the runtime resolve themselves) and resolves per declared backend only to
  compute the diagnostics — the same all-declared-backends posture `write_pin_diagnostics` uses.
- 2026-09-05 (plan 07): "which state structure does a contract point require" lands as a pure
  function in `smelt-logical`'s `contract` module, exhaustive over `ContractPoint`, per the
  contract-lattice point single-ownership invariant — a caller must never decide this ad hoc.

- 2026-09-05 (phase 7 implement): landed both `DiagnosticCode` variants
  (`MaintenanceStateDowngraded` Warning, `DeclaredContractRequiresState` Error), the pure
  `contract::required_state_structure`, `parse_warehouse_tables`/`project_warehouse_tables`, and
  `backend_dialect_for`, all folded into `maintenance_plan_diagnostics`. Both catalogue-ahead-of-
  variant divergence bullets deleted (`diagnostics.md`, `state.md`). Real availability resolution
  against `examples/timeseries`'s declared backends surfaced a genuine gap: an illustrative
  `spark` target in that example's `smelt.yml` (never exercised by any test) made three real
  models correctly downgrade/refuse — removed the target and the matching stale README sections
  rather than suppress a correct new diagnostic. All gates green; see `phases/07-summary.md`.

- 2026-09-05 (plan 08): no phase reshape. Reading the write sites confirmed phase 8 is one
  coherent change (a posture gate in `FileStore` plus one `execute_project` wiring point); the
  remaining rows 9-11 are unaffected.
- 2026-09-05 (plan 08): phase 8 carries a small `state.md` spec delta after all. Two structures
  the runtime already writes — the source-mutation baselines and the migration-approval store —
  appear in neither the §"state-structure inventory" nor the posture consequence table, and
  §Surface declares an unclassified structure a spec violation. A per-posture write set must
  enumerate them, so they are classified (both `observability`, both in the `intervals` row) as
  part of this phase rather than left for phase 11.
- 2026-09-05 (plan 08): the posture gate lands inside `FileStore` (a `with_state_mode`
  constructor plus a pure `state_artifacts_written` table), not as ~15 `if mode != Stateless`
  guards at the call sites in `execute.rs`. One seam is checkable against the spec table by a
  single unit test and cannot be bypassed by a new save site.
- 2026-09-05 (plan 08): honouring the default posture means `smelt run` stops writing `.smelt/`
  unless a project opts in, so several in-code test Configs (which get `StateMode::Stateless` by
  default) must declare `state.mode: intervals` to keep asserting manifests/intervals. That
  fixture sweep is listed as a phase task, not treated as a regression.

- 2026-09-05 (phase 8 implement): landed the posture gate inside `FileStore`
  (`StateArtifact` + `state_artifacts_written` + `with_state_mode`), wired
  `execute.rs` to build the run pipeline's store from `config.state.mode`,
  and made `--resume` under `stateless` refuse by naming the posture before
  it reaches history load. No new production logic was needed for
  `contract.deferral` to degrade correctly under `stateless` —
  `run_license`'s existing `None`-frontier fallback already covers it; a new
  test locks that. Discovered mid-phase: honouring `state.mode` for real
  (previously unconditional) broke 9 test files beyond the plan's named
  list, all now fixed by declaring `state.mode: intervals` in their
  fixtures; also discovered `SnapshotStore::save_snapshot_store` has no
  production caller anywhere — virtual environments' distinguishing
  structure is currently never persisted by a real run (pre-existing gap,
  not touched here; flagged for whoever next works `virtual_
  environments.md`). All gates green (`verify-phase.sh`, full workspace
  `cargo test`, `maintenance_conformance`, `execute_parity`/
  `statement_parity`, `smelt-lsp --test example_workspaces`). See
  `phases/08-summary.md`.

- 2026-09-05 (plan 09): no phase reshape. Phase 8's summary surfaced one gap (nothing in
  production writes the virtual-environments snapshot store), but that is virtual-environments
  implementation work already outside this outcome's scope and serves no success criterion; the
  remaining rows 10-11 are unaffected.
- 2026-09-05 (plan 09): the deletion toggle lands as one seam inside
  `LinkCProject::run` (the single point every recipe family's run reaches `execute_project`
  through), not as a new `ConformanceStep` variant — a schedule-step variant would force a match
  arm into every drive loop in `gate.rs` and its sibling modules, and the deletion is a property
  of the *environment* between runs, not of the schedule.
- 2026-09-05 (plan 09): the leg reuses the existing public staging + drive helpers over the
  partition and keyed pools rather than duplicating each of the ~8 pool test bodies. Keyed is
  the load-bearing half — its never-fold-twice check runs against `_smelt_ledger`, so a green
  keyed leg under deletion is the executable proof of criterion 1, where the partition leg
  proves interval state is reconstructible.
- 2026-09-05 (plan 09): case counts default low (3 per pool, `SMELT_STATE_DELETION_CASES`
  override) because this is a standing per-PR gate; an anti-vacuity test on the deletion counter
  is what keeps a small sample from degrading into a no-op assertion.

- 2026-09-05 (phase 9 implement): landed the deletion toggle
  (`LinkCProject::with_state_deletion(StateDeletion::BetweenRuns)`, wired into the single `run`
  seam) and three new `state_deletion.rs` gate tests (partition pool, keyed pool — the
  criterion-1 proof, anti-vacuity), plus a `smelt-maintenance-testkit`-local unit test locking
  the toggle itself. No family needed `.smelt/` continuity to stay equivalent — every admitted
  case in both pools upholds its oracle with `.smelt/` deleted before every run, so no Blocked
  entry was needed. All gates green (`verify-phase.sh`, `maintenance_conformance` full run,
  `execute_parity`/`statement_parity`, `smelt-maintenance-testkit`, and a
  `SMELT_STATE_DELETION_CASES=8` deep sweep). See `phases/09-summary.md`.

- 2026-09-05 (plan 10): no phase reshape. Phase 9's summary reports nothing outstanding, and
  rows 10-11 still map onto criteria 7 and 8. Reading the docs-site pages did widen phase 10's
  *scope* within its own row, without adding work that belongs elsewhere: `docs-site`'s
  `smelt.yml` reference has no `state` top-level key documented at all (so criterion 5's
  user-visible `warehouse_tables` surface has nowhere to live), and `guide/targets.md`'s Spark
  coverage table still asserts a fail-loud refusal for `Additive` keyed folds where phases 4-7
  made it a recorded downgrade. Both are criterion-serving user-doc drift and stay in this row.
- 2026-09-05 (plan 10): the docs-site edits get a real red-green gate
  (`crates/smelt-cli/tests/state_docs_freshness.rs`) rather than being verified by eye — the
  three facts this outcome changed (no `reconciliation.json` file, a documented `state` block,
  a stated deletion invariant) are exactly the kind of user-doc claim that silently rots, and the
  repo's precedent (`cli_docs_coverage`, `tutorial_freshness`) is a standing test.
- 2026-09-05 (plan 10): `state.md` §Known Divergences is deliberately left to phase 11. Phase 10
  propagates already-normative spec statements outward; rewriting the divergence bullets is
  close-out work that should follow `/smelt:validate state`, not precede it.
- 2026-09-05 (implement 10): deviated from the plan-10 note above — phase 10's own Verification
  section runs `rg -n "reconciliation\.json" docs-site/ docs/specs/` with no `docs/specs/`
  exclusion, and two Known-Divergences bullets (`state.md`'s ledger-residency bullet, criterion 1;
  `run_state.md`'s matching bullet) would have failed that literal check. Confirmed both were
  fully landed already (`crates/smelt-runtime/src/execute.rs` consults `StateMode`;
  `crates/smelt-state/src/ddl_duckdb.rs`/`file_store.rs` confirm the ledger is
  `_smelt_ledger`-resident, not file-resident) and deleted them per the spec-craft rule "a
  fully-landed entry is not a divergence — delete it", along with the third bullet
  (`warehouse_tables` unimplemented, criterion 5 — also landed). `state.md` §Known Divergences now
  reads "none currently open"; phase 11 should treat this as already done rather than redoing it.
- 2026-09-05 (implement 10): phase 10 done — all docs-site edits landed, the new
  `state_docs_freshness` gate is green (red before edits), the keyed-grain residue outcome is
  closed (criterion 3 amended, Status `done`), `verify-phase.sh` ALL GREEN. No new gaps
  discovered.

- 2026-09-05 (implement 11): phase 11 done — `/smelt:validate state` run end to end; drift
  report at `docs/validations/2026-09-05-state.md`. Only drift found was §References bookkeeping
  (Code missing the phase-4 `availability.rs` owner and `parse_warehouse_tables`; User docs and
  Plans (history) both still `none yet`) — fixed, backed by a new `spec_references_are_live`
  test (red before the edit, green after) in
  `crates/smelt-cli/tests/state_docs_freshness.rs`. `front-matter last_reviewed` bumped to
  2026-09-05. Gates: `verify-phase.sh` ALL GREEN; `statement_parity`+`execute_parity` 41/41;
  `maintenance_conformance` 78/78; `walk_coverage` 4/4; `state_docs_freshness` 4/4;
  timeless-oracle grep clean; `reconciliation.json` grep shows only the one sanctioned
  `run_state.md` hit. No new gaps surfaced beyond the pre-existing freshness flag phase 6
  already recorded in `docs/TODO.md`.

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->

- 2026-09-05 (plan 11): no reshape — phase 10's summary reports no new gaps, and phase 11 is the
  final row. Plan narrowed on one discovery from that summary: `state.md` §Known Divergences is
  already "none currently open", so phase 11 does not redo it. The residual bookkeeping drift is
  in `state.md` §References, which still reads `User docs: none yet` / `Plans (history): none yet`
  and omits `smelt-logical/src/maintenance/availability.rs`; a new `spec_references_are_live` test
  locks that shut. The drift report persists to `docs/validations/2026-09-05-state.md`.
