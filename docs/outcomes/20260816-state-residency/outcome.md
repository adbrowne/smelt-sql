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
| 4 | Move the reconciliation ledger engine-resident: backend table transactional with the fold, migration/read path for existing `.smelt/reconciliation.json`, never-fold-twice check rides the table | done |
| 5 | Two-step ideal-then-availability derivation: ideal plan preserved, availability resolution pass, recorded explain-visible `MaintenanceStateDowngraded` | done |
| 6 | `DeclaredContractRequiresState`: fail-loud validation for a declared contract point whose semantics require an unavailable state structure (`contract.deferral` ↔ the frontier) | done |
| 7 | Fuse the frontier reset into the region-recompute's own write transaction (phase 4's flagged gap; closes criterion 2's "transactional with the fold" wording the specs already claim) | done |
| 8 | State-deletion conformance leg: `.smelt/` deletion and fresh-clone steps in the generative suite, asserted against the oracle, for keyed additive folds *and* idempotent-graded region-recompute models | done |
| 9 | Backend-aware downgrade visibility: `smelt explain` (text + `--json`) resolves the model's real target dialect into `StateAvailability` instead of `all()`, so a `MaintenanceStateDowngraded` cell is actually visible for a real project; same for the remaining `maintenance_driver`/`propagation` resolvers that can reach a target | done |
| 10 | Docs-site update for state modes and residency; `/smelt:validate state`; remove/narrow closed Known Divergences bullets across `state.md`/`run_state.md`/`incremental_models.md` (incl. the now-stale "the runtime ignores `state.mode` entirely") | planned |
| 11 | Close-out: full standing-gate sweep (including one live-Spark execution of the `maintenance_conformance_spark` edits phase 9 could only compile-check, or a recorded reason it was not possible), criteria-vs-summaries judgment, outcome status flip | pending |

## Decision log

- **2026-08-16 (phase 10 plan).** Minimal reshape: row 11 now explicitly carries the live-Spark
  execution of phase 9's `maintenance_conformance_spark` edits (compile-checked only in that
  phase's sandbox) or a recorded reason it could not run — leaving it implicit would let the
  close-out sweep call criterion 6 green over an unexecuted suite. Row 10 stays a docs+spec
  phase with no runtime behaviour change; its three doc-sync tests exist so the sweep is a
  ratchet rather than one-shot prose. Also noted for row 11's judgment: **row 1 is marked
  `blocked` but its content actually landed** — all five spec deltas are committed, and the
  blocker (the pre-existing `contract_lattice_spec` heading-lookup regression) was fixed inside
  phase 2 per option (b). Row 11 should judge criterion 4's spec half against phase 1's
  summary, not against the row's `blocked` status.
- **2026-08-16 (phase 10 plan).** The docs sweep narrows rather than deletes two bullets: the
  DuckDB-only ledger-substrate bullet (`state.md`, `incremental_models.md`) survives as a
  dialect-coverage gap even though its "fails loudly" half is closed, and per phase 9's summary
  the two remaining `StateAvailability::all()` call sites are documented as intentional, not as
  residue — recording them as a gap would misdescribe deliberate design.
- **2026-08-16 (phase 9 plan).** Reshaped the tail into three rows: inserted a new row 9 for
  backend-aware downgrade visibility, pushing the docs/Known-Divergences sweep to 10 and
  close-out to 11. Criterion 3 requires `MaintenanceStateDowngraded` be "visible in `smelt
  explain` and diagnostics", and phase 5's summary flagged that it is not: `smelt explain`'s own
  derivation (`crates/smelt-runtime/src/maintenance_driver.rs:3465`,
  `:3484`) passes `StateAvailability::all()`, so a Spark-targeted project — the only project
  that downgrades at all today — shows no downgrade line, and `build_maintenance_plan_json`
  never carries `state_downgrades` at all. That is criterion-3 work, so it stays inside the
  outcome as a row rather than being swept under row 10's Known Divergences. It runs *before*
  the sweep because the sweep's wording for the availability bullet depends on whether this
  gap is closed.
- **2026-08-16 (phase 9 implement).** `smelt_db::maintenance_plan_report` (the actual function
  feeding `smelt explain`, in `smelt-db/src/lib.rs`) was the real fix site, not the
  `maintenance_driver.rs:3465`/`:3484` resolver named in the plan's row-9 description — that
  resolver (`resolve_live_delta_restriction_facts`) is reached only from live-execution paths in
  `execute.rs`, never from `smelt explain`. `maintenance_plan_report` already called the
  edge-aware `derive_model_maintenance_plan_with_edges` for every model unconditionally, so
  threading its `dialect_name` param closed both the plain and edge-having cases in one fix.
  Also threaded real availability through `maintenance_driver.rs`'s four other resolvers
  (execution-time, not explain-time) since `execute.rs` genuinely has `backend.dialect()` in
  scope at every call site — a duckdb-only no-op for the ungated suite, real for a live Spark
  run. Fixed a latent `HashMap::keys().next()` nondeterminism bug in default-target selection
  (three sites) that this phase's own golden-fixture test flaked on.
- **2026-08-16 (phase 9 plan).** The `all()`-passing call sites split into two classes and only
  one is in scope: the two report/graph resolvers `smelt explain` reaches have a target dialect
  in the caller's hand already (`commands/explain.rs` resolves `dialect` from `config.targets`),
  while `propagation.rs:812` and the four `maintenance_driver` resolvers at `:570`/`:796`/
  `:1030`/`:1156` are reached from paths that may not know their target. Phase 9 threads real
  availability wherever a target is genuinely reachable and leaves a narrowed Known Divergences
  bullet naming any residue, rather than inventing a dialect at a call site that has none.

- **2026-08-16 (phase 8 implement).** Phase 8 landed: `ConformanceStep::DropStateDir`/
  `FreshClone` (excluded from `is_permutable`) plus the shared `StateResidencyOp` enum;
  `LinkCProject` derives `Clone` and gains `fresh_clone` (copies `models/` + `smelt.yml`, never
  `.smelt/`, same `db_path`); `drive_and_assert` holds a reassignable local project handle;
  `drive_keyed_and_assert_with_state_ops` (index-keyed `BTreeMap<usize, StateResidencyOp>`) with
  `drive_keyed_and_assert` delegating an empty map; the Spark twin's match arm `bail!`s on both
  new steps naming the ledger-less-backend downgrade. New `state_deletion.rs` (7 tests, the 6
  planned plus one exercising the keyed hook directly) all pass with NO product-code changes —
  every criterion-5 scenario (redelivery-still-refuses after a ledger-holding drop, generative
  mid-schedule drop/clone, region-recompute frontier survival) held on the first try, confirming
  phases 4/5/7's engine-residency work is actually load-bearing. Anti-vacuity confirmed per the
  plan's Verification step. All gates green including the full `verify-phase.sh` sweep, the
  Spark compile check, and `frontier_residency`/`state_posture`. See `phases/08-summary.md`.
- **2026-08-16 (phase 8 plan).** No reshape (rows 9–10 stand). Phase 8 carries no spec delta: it
  is a test-only leg over already-shipped behaviour, and the Known Divergences sweep is row 9's
  job. The one carve-out is named in the plan — if a residency step exposes a real defect (some
  correctness decision still riding on `.smelt/`), fixing it is inside phase 8, since catching
  exactly that is what criterion 5 exists for.
- **2026-08-16 (phase 8 plan).** Two residency steps, not one: `DropStateDir` and `FreshClone`
  are separately valuable because only the clone changes the project's absolute path — anything
  keyed on the old path (interval-store lookups, model-hash keying, legacy-file import) is caught
  there and nowhere else, while `DropStateDir` alone would leave it invisible.
- **2026-08-16 (phase 8 plan).** Two wiring mechanisms, deliberately: the partition/append-only
  pool gets real `ConformanceStep` enum variants (honest modelling, and it forces the Spark twin
  to state a position — it must `bail!` on residency steps, since a ledger-less backend has no
  engine-resident state to survive the deletion, per phase 5's downgrade), while the keyed pool
  gets an index-keyed `BTreeMap<usize, StateResidencyOp>` parameter on a new
  `drive_keyed_and_assert_with_state_ops` — `KeyedSchedule` is a plain `Vec<KeyedRunWindow>` with
  no step enum, and adding a field would churn every construction site for no test value. The op
  enum itself is single-owned in the testkit and shared by both shapes.
- **2026-08-16 (phase 8 plan).** An explicit anti-vacuity test
  (`drop_state_dir_step_actually_removes_the_directory`) plus a comment-out check in Verification:
  a residency leg whose deletion silently no-ops would pass forever while proving nothing, which
  is the specific failure mode a generative gate over an unobservable step invites.
- **2026-08-16 (phase 7 implement).** Phase 7 landed: `maintenance_driver::
  execute_region_recompute_with_frontier_reset` fuses the ordinary DuckDB `DeleteInsert`
  batch's write with its frontier reset in one transaction (`Backend::
  execute_write_and_reset_frontier`), reusing the SAME `StatementGroup` `emit_delete_insert`
  already built for the run's report; `execute.rs` dispatches through it whenever the target
  already exists, falls back to `execute_model_incremental` for the bootstrap case, and a
  `fused_batch_writes` counter skips the after-the-loop whole-range record only when every
  batch in the run fused. Spec delta: `incremental_models.md` §"The frontier record
  (reconciliation ledger)" now states per-recomputed-batch-region writing, and a new §Known
  Divergences bullet names the three still-unfused paths (bootstrap, delta-restricted
  recompute, column-scoped-merge/in-place-update). Three new tests in
  `frontier_residency.rs` prove per-batch rows, atomic rollback of a failed fused batch, and
  the retained unfused bootstrap path. All gates green, including the full `verify-phase.sh`
  sweep; `.claude/hardening-baseline.txt`'s `smelt-runtime expect` count moved 10→11 (one more
  infallible `.expect()`, same pattern as the existing `column_scoped_cell` one nearby). See
  `phases/07-summary.md` "For the next planner" for row 8's now-available per-batch-fused
  assertion opportunity.
- **2026-08-16 (phase 7 plan).** No reshape (rows 8–10 stand). Planning established the precise
  shape of phase 4's flagged gap: the DuckDB `execute_write_and_reset_frontier` override is
  *already* a real single transaction and its rollback behaviour is already test-covered — the only
  defect is the call site, which passes an empty `write_group` after the batch loop has committed.
  So phase 7 is a call-site move (hand the batch's already-emitted `emit_delete_insert` group to the
  hook), not a backend change, and statement-emission single ownership is preserved because the
  fused write text is the same emitter output the run already reports.
- **2026-08-16 (phase 7 plan).** Fusion is scoped to the ordinary DuckDB `DeleteInsert` batch — the
  region recompute the spec sentence actually names. The bootstrap `CREATE TABLE AS` first
  materialization, the delta-restricted recompute, and column-scoped merge / in-place update keep
  today's after-the-loop record, and phase 7 carries a spec delta narrowing
  `incremental_models.md` §Known Divergences to name them rather than leaving a bullet that reads
  as fully closed. Fusing those paths would mean threading frontier SQL through three more
  `maintenance_driver` helpers with their own retry and probe wiring; not required by criterion 2's
  wording and not silently assumed done.
- **2026-08-16 (phase 7 plan).** The record becomes **per recomputed batch region** rather than one
  whole-range row — that is the necessary consequence of fusing with a per-batch write, it is finer
  and truthful, and the region-intersecting reset `DELETE` keeps a later coarser record collapsing
  the finer ones. Spec-first: `incremental_models.md` gains the sentence before the code lands. The
  after-the-loop whole-range record is retained but runs only when at least one batch was not fused.
- **2026-08-16 (phase 6 implement).** Phase 6 landed: the corrected `state.md` §"Declarations
  stay fail-loud" sentence, the `diagnostics.md` catalogue row,
  `StateStructure::IntervalFrontier`/`StateAvailability::interval_frontier`, the new pure
  `smelt_logical::contract::state_requirements` module (`required_state_structures`,
  `validate_contract_state`), `state_availability_for_project`, the
  `DeclaredContractRequiresState` `DiagnosticCode` (smelt-db + LSP code string), its
  `check_file_diagnostics` wiring (effective posture × every declared backend, deduped by
  declaration), and `examples/timeseries`' `state: mode: intervals`. All phase-6 gates green,
  including the full `verify-phase.sh` sweep, the LSP example-workspace gate, and the new
  runtime e2e test proving the refusal actually blocks `execute_project`. See
  `phases/06-summary.md` "For the next planner" for `smelt explain`'s own non-rendering of this
  refusal, left for a later phase.
- **2026-08-16 (phase 6 plan).** No reshape of the remaining rows (7–10 stand as written).
  Planning phase 6 established that `contract.deferral`'s lag is measured from the
  **`.smelt/`-resident** interval-store and landed-delta frontiers
  (`smelt_runtime::contract_probes::resolve_deferral_frontiers`), not from the engine-resident
  frontier record phase 4 built — so the structure the declaration requires is
  observability-class and the `stateless` posture withholds it. That makes the refusal's real
  trigger the **posture**, correcting phase 1's decision-log assumption ("measured from the
  correctness-class frontier, which no posture can withhold"). Phase 6 therefore carries a small
  spec delta of its own in `state.md` §"Declarations stay fail-loud" (spec-first) rather than
  shipping code that contradicts the spec sentence, plus the `diagnostics.md` catalogue row the
  phase-5 sweep left out. `examples/timeseries` is the live proof of the gap: it declares
  `contract.deferral: '6 hours'` under the default `stateless` posture, so its probe can never
  fire today; the example gains `state.mode: intervals` as part of the phase.
- **2026-08-16 (phase 6 plan).** The refusal rides `file_diagnostics` at Error severity rather
  than a bespoke runtime check: `smelt_runtime::gate::gate_diagnostics` already blocks a run on
  any Error-severity analyzer diagnostic, so the declaration is refused in the editor and at
  build time from one owner. The validator itself stays pure in `smelt-logical`
  (`required_state_structures` / `validate_contract_state`), with `smelt-db` only resolving the
  effective posture and backends — the contract-lattice single-ownership rule.

- **2026-08-16 (phase 5 implement).** Phase 5 landed: the pure
  `smelt_logical::maintenance::availability` module (`StateAvailability`,
  `resolve_state_availability`, `StateDowngrade`), `PlanCell::recompute_fallback` (populated at
  the `KeyedFold` push site via the existing `repair::admit_per_group_recompute`),
  `MaintenancePlanResult::ideal_plan`/`state_downgrades`, the `StateAvailability` parameter on
  both `smelt-db` derive entry points (resolved internally, edges variant resolves after
  appending model-edge cells), `state_availability_for`/`state_downgrade_diagnostics`, the
  warning-severity `MaintenanceStateDowngraded` diagnostic, and its `smelt explain` print. Only
  two runtime call sites (the ones that already carry `dialect: SqlDialect` in scope) pass real
  availability; every other call site — including `smelt explain`'s own report path and five of
  `maintenance_driver.rs`'s seven resolvers — passes `StateAvailability::all()` unchanged, so
  `smelt explain` does not yet show a downgrade for a project's real declared backends. Also
  retired phase 4's `tracing::warn!` frontier-skip site (demoted to `debug!`) and excluded the
  new advisory from the example-workspace zero-diagnostics gates (every `spark`-targeted example
  now legitimately downgrades). All phase-5 gates green, including a re-check of
  `smelt-lsp --test example_workspaces` (not in the plan's own Verification list but broken by
  the same example-fixture effect). See `phases/05-summary.md` "For the next planner" for the
  runtime-wiring gap and `smelt explain`'s own non-backend-awareness, left for a later phase.
- **2026-08-16 (phase 5 plan).** Reshaped the tail into six rows. (a) Split the old row 5 in two:
  the availability-resolution downgrade (advisory, plan-derivation, `MaintenanceStateDowngraded`)
  and the declared-contract refusal (fail-loud, validation, `DeclaredContractRequiresState`) have
  different oracles and different code paths — one rewrites a derived cell, the other rejects a
  frontmatter declaration — so they get a row each. (b) Added a row for phase 4's flagged
  not-achieved item: `run_state.md`/`incremental_models.md` already say the frontier reset
  "commits in the same backend transaction as the recompute's own write", and today it is only
  atomic with itself. That is spec-vs-code drift against criteria 2 and 6, so it stays inside the
  outcome as row 7 rather than leaving as a fast-follow. (c) Widened the conformance leg (row 8)
  to idempotent-graded region-recompute models per phase 4's summary, and folded the stale
  "runtime ignores `state.mode`" bullet into row 9's sweep.
- **2026-08-16 (phase 5 plan).** Availability is resolved **inside**
  `smelt_db::queries::maintenance::derive_model_maintenance_plan(_with_edges)` rather than at each
  of the ~8 `smelt-runtime` call sites: the resolved plan is then what every consumer already
  reads (lowering included), while `MaintenancePlanResult` keeps the ideal plan as its own field —
  which is exactly what `state.md` §"The degradation contract" demands ("the ideal plan must exist
  as a derived object even when it will not run"; early pruning violates the spec). A caller that
  does not know its target passes `StateAvailability::all()`, whose behaviour is byte-identical to
  today's.
- **2026-08-16 (phase 5 plan).** The keyed fold's downgrade target is chosen at derivation, not
  guessed at resolution: the `Technique::KeyedFold` push site calls the existing
  `repair::admit_per_group_recompute` with inputs already in scope and records the result as the
  cell's `recompute_fallback`. No admissible fallback means a fail-loud
  `Refusal::NoAdmissibleTechnique` naming the missing structure — the spec licenses a downgrade
  only to a recompute-family member that preserves the equivalence invariant, never to "run the
  fold anyway". The existing silent `ColumnScopedMerge` → region-recompute drop in
  `choice.rs::resolve_cell_choice` is a *backend-capability* drop, not a state-structure one, and
  is deliberately left as-is: `MaintenanceStateDowngraded` names a missing structure.

- **2026-08-16 (phase 4 implement).** Phase 4 landed: `_smelt_frontier` (new table) plus
  `Backend::execute_write_and_reset_frontier` (DuckDB-transactional override) replace
  `.smelt/reconciliation.json`'s frontier-grading writes; `FileStore::
  take_legacy_reconciliation_store` imports a legacy file's both gradings into the engine
  (`_smelt_frontier`/`_smelt_ledger`) once per run, then deletes it. Not achieved: fusing the
  frontier reset with the model's own data write in one transaction — the actual per-batch
  write already commits earlier in the loop by the time the reset runs, so `write_group` is
  empty at the real call site today; the reset's own delete+insert are atomic with each other,
  not with the write. See `phases/04-summary.md` "For the next planner" for the scoped
  follow-up. All phase-4 gates green, including the full `verify-phase.sh` sweep and
  `maintenance_conformance`. `.claude/hardening-baseline.txt`'s `smelt-backend-duckdb expect`
  count moved 18→19 (one more infallible mutex-lock `.expect(...)`, same pattern as every other
  transactional override in that file).
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
