# Outcome: Every defect the real pipeline hits on BigQuery is fixed, and DuckDB and BigQuery agree

**Created:** 2026-09-06
**Status:** done (2026-09-11; reopened 2026-09-10, closed by phase 16's live run)
**Driver:** outcome loop (`.claude/outcome-backlog`). Phases 1-10 were loop-ground and are
`done`. Of the reopening, phases 11-15 are loop-grindable — the emitters and the structural
gate are provable offline, and phases 12-15 must prove their SQL by unit test and by
`cargo check -p smelt-cli --features bigquery`, never by reaching a warehouse. Phase 16 is
**human-gated**: it re-runs `examples/github_activity` live against the dogfood dataset, so
it must emit `<<PHASE_BLOCKED>>` rather than attempt it — the gate was given on 2026-09-11
and the phase ran.
**Source:** `docs/research/20260906-bigquery-dogfood.md` §"The programme" (D2), §"Sequencing: models first, punch-list second", §"Findings already banked"
**Spec anchors:** `docs/specs/multi_backend.md` §"Operator lowering", §"Statement-level lowering", §"Output-schema type conformance", §"Cross-engine emission audit"; `docs/specs/architecture.md` §"Constraints & Invariants" item 14; `docs/reference/dialect-coverage.md`; `docs/specs/state.md` §"The state-structure inventory", §"The degradation contract"; `docs/specs/incremental_shapes.md` §"The transactional frontier write (merge ledger)", §"The tombstone ledger (hidden state)"; `docs/specs/incremental_models.md` §"The graph layer"

## The outcome

The BigQuery defects that a real pipeline actually reaches are fixed, and each fix is held
by a gate that would have caught it. The unconditional ones — wrong for any model on any
run — are fixed first and without waiting for evidence: `emit_fingerprint_digest_select`
ignores its `dialect` parameter and hardcodes DuckDB's hash spelling on every backend.
Everything after that is driven by
`docs/outcomes/20260906-bigquery-dogfood-spine`'s findings handoff: only the registry
entries, techniques, grains and capability rows the spine's models genuinely hit are
built, and each one lands with its emission verdict, its ledger row, and its coverage in
the published dialect table. Where DuckDB and BigQuery disagree on the same rows, the
difference is either fixed or registered with a reason — never tolerated silently.

The reopening (2026-09-10) adds the defect the spine's live run actually hit, which is a
layering fault rather than a missing spelling: the availability layer claims BigQuery and
Spark realise state structures whose only emitters are DuckDB's, so the run refuses where
it should have downgraded and said so. The fix reconciles the two layers first and gates
the reconciliation structurally, then gives BigQuery a real realisation of the ledger
substrate — observed deltas, the merge and reconciliation ledgers, never-fold-twice, and
the tombstone ledger — so the cross-target comparison weighs one plan on two engines
instead of two plans. Spark's realisation is refused, not deferred: Delta has no
cross-table transaction, and that absence is recorded in the spec rather than left to be
rediscovered.

## Success criteria (checkable)

1. **The unconditional fix.** `emit_fingerprint_digest_select`
   (`crates/smelt-logical/src/maintenance/emit.rs`) threads its `dialect` through to
   `row_fingerprint_expr` instead of passing `MaintenanceDialect::DuckDb`; a unit test per
   dialect asserts the emitted expression (BigQuery gets `TO_HEX(SHA256(…))`, since
   GoogleSQL's `SHA256` returns `BYTES` and the value feeds a `STRING_AGG`). Whether the
   path is reachable on a live `mutable_snapshot` run is answered in the decision log
   either way — the fix does not depend on the answer.
2. **Punch-list harvested, not invented.** Phase 1 reads the spine's findings handoff and
   rewrites this outcome's remaining phase rows from it. A row whose only justification is
   "issue #179 lists it" and which no spine model reaches is recorded under Out of scope
   with that rationale, per §"Sequencing".
3. **Every fixed construct is gated.** Each defect fixed here gains coverage in the gate
   that owns its class — a `signatures.rs` emission verdict plus a `dialect_audit` probe
   for a spelling, a `ledger.rs` row for a registered mismatch, a `dialect_seam` case for
   a compile-time refusal, a `projection_dialect_invariance` case for a projection bug —
   so a regression fails offline. No fix lands with only a manual sweep behind it.
4. **Ratchets move the right way.** `.claude/dialect-gaps-baseline.txt` and
   `.claude/parser-gaps-baseline.txt` fall or hold; neither is raised. Registry entries
   this outcome gives verdicts to leave `#179`'s unverified count lower, and
   `docs/reference/dialect-coverage.md` is regenerated so the doc-sync gate is green.
5. **Cross-target agreement.** Every divergence the spine registered under its criterion 6
   is resolved here: fixed, or promoted to a permanent, reasoned entry in the divergence
   registry with the engines and the construct named. The count of unexplained differences
   is zero.
6. **The two known live conformance failures are characterised.**
   `dags_bigquery::diamond_propagation_suffices` and
   `gate_composed_bigquery::composed_keyed_pool_upholds_equivalence` are each either fixed
   or explained in the handoff with the mechanism named — not left uncharacterised.
7. **Gates green.** `bash .claude/scripts/verify-phase.sh`, plus
   `cargo test -p smelt-dialect --test emission_ownership`,
   `cargo test -p smelt-runtime --test dialect_seam`,
   `cargo test -p smelt-runtime --test projection_dialect_invariance` and
   `cargo test -p smelt-db --test dialect_audit` (DuckDB legs in-process). The BigQuery
   value leg is a manual sweep (`scripts/bigquery-dialect-audit.sh`) — a phase that needs
   it and cannot run it emits `<<PHASE_BLOCKED>>` rather than skipping green.

8. **The plan layer and the run layer never disagree about state.** For every dialect,
   `realisable_state_structures` names exactly the structures that dialect can actually
   build: a structure it claims is realisable has emitters and a backend seam behind it,
   and a structure it does not claim produces a *recorded* downgrade rather than a
   `bail!`. Held structurally, not by inspection — every `dialect != SqlDialect::DuckDB`
   guard under `crates/smelt-runtime/src/maintenance_driver/` must correspond to a
   structure the availability layer declares unrealisable for that dialect, so flipping a
   dialect's row on without deleting its guard fails, and deleting a guard without
   backing it fails.
9. **BigQuery runs the pipeline's real plan, not a coarsened one.** `examples/
   github_activity` completes on the `bigquery` target with observed-delta recording, the
   merge and reconciliation ledgers, the additive never-fold-twice refusal and the
   tombstone ledger all realised — so the cross-target comparison of criterion 5 and the
   spine's criterion 7 compare one plan on two engines rather than two plans. Where a
   guarantee has no sound realisation on a dialect, that dialect's row says so and the
   reason is in `docs/specs/state.md` — an honest absence, never a silent one.

## Out of scope

- Building the 42 no-verdict BigQuery registry entries of issue #179 speculatively. Only
  entries a spine model reaches are built here; the rest stay on #179.
- Snowflake, Redshift and Postgres emission work.
- Any new model, source or feature — this outcome only fixes what the spine surfaced.
- Widening the sample, the model set, or the pipeline's scope (the spine owns that).
- Retiring the PostgreSQL emission dialect (tracked separately by the
  dialect-emission-vocabulary outcome).
- **Closing the LSP-diagnostics / `smelt explain` divergence for model-edge refusals**
  (found by phase 4, recorded in the decision log). `smelt-db`'s `maintenance_plan` Salsa
  query never threads model edges, so no model-edge refusal — the pre-existing
  `ReachNotDerivable` included — reaches `file_diagnostics()`. It is an editor-surface gap
  that predates this outcome and is not a defect the real pipeline hits on BigQuery, so it
  serves none of the success criteria; it needs its own outcome.
- **Any BigQuery-only *emission* defect not already in hand.** The spine's live half has
  since run (its phase 11, 2026-09-10) and surfaced one hard stop — the T5 ledger-substrate
  gap this reopening owns — but no new registry entry, grain or capability row. Building
  any of issue #179's 42 no-verdict entries would still be speculation, which criterion 2
  forbids; they stay on #179.
- **The Spark realisation of the ledger substrate.** Delta gives per-table atomicity only
  and has no cross-table transaction, so a Spark ledger write and its data write cannot be
  made atomic and the never-fold-twice refusal has no sound Delta realisation without a
  different soundness argument. Phase 11 corrects Spark's `realisable_state_structures`
  row to say so and records the reason in `docs/specs/state.md`; phases 12-15 are
  BigQuery-only. Spark's honest downgrade is in scope; Spark's realisation is not.
- **Any change to what the downgraded plan computes.** A recorded downgrade already
  preserves the equivalence invariant by construction (`recompute_equivalent`); this
  outcome makes the downgrade *honest and then unnecessary* on BigQuery, and never widens
  what a downgraded cell is allowed to do.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | The unconditional fix: thread `dialect` through `emit_fingerprint_digest_select` to `row_fingerprint_expr`, per-dialect unit tests, and answer in the decision log whether the path is reachable on a live `mutable_snapshot` run | done |
| 2 | The rest of the dialect-blind fingerprint SQL: `key_expr_for_columns`' hardcoded `CAST(... AS VARCHAR)` and `emit_repair_group_digest_select`'s DuckDB-only `bit_xor(hash(...))` + `VARCHAR` cast — fix per-dialect or refuse loudly, with the capability gate held by a test | done |
| 3 | Punch-list 1 — `emit_succession_full_rebuild` folds on `(key_cols, clock_col)` with a per-column aggregate over the model's own output schema, and runs the clock-tie probe it has never run; closes the `silver_repo_naming` / `silver_actor_naming` divergence | done |
| 4 | Punch-list 2a — derive the missing `UpstreamMutation(gold.repo_dim)` cell: a new **enrichment-keyed** route in `append_model_edge_cells` for a clockless keyed upstream read in value-enrichment position by a partition-addressed downstream, plus a real `MaintenanceRepairKeysNotDiscoverable` diagnostic so the remaining fail-closed leg is loud at `build`/`run` rather than only `explain` | done |
| 5 | Punch-list 2b — make that cell live on the run path: thread model edges into `resolve_live_column_scoped_cell`/`maintenance_availability::derive_resolved` and the mutation gate, so `gold.events_enriched`'s already-written `current_repo_name` heals and the `github_activity` stale-row count reaches zero | done |
| 6 | Punch-list 3 — `compute_calendar_windows`' interior-chunk-boundary forward-reach loss for Form-B models, which makes the full-refresh oracle itself undercount a cross-midnight session | done |
| 7 | Punch-list 4 — the missing repair edge from a Form-B model's own self-rebase to a Form-A downstream aggregate that reads it verbatim; first check whether phases 4-5's mechanism already covers it | done |
| 8 | Resolve every divergence the spine registered (`github_activity_oracle.rs`'s `DIVERGENCE_REGISTRY`): each residual entry fixed or promoted to a reasoned permanent entry naming the engines and the construct, unexplained count zero — and, if phases 3-7 have emptied the registry, prove the unregistered-divergence sweep still fails closed on an empty registry rather than passing vacuously | done |
| 9 | Characterise or fix the two known live conformance failures (`diamond_propagation_suffices`, `composed_keyed_pool_upholds_equivalence`) | done |
| 10 | Close: regenerate `docs/reference/dialect-coverage.md`, move the gap ratchets down, update issue #179 with what was verified, all standing gates green | done |
| 11 | Make the plan layer and the run layer agree before any new SQL: correct `realisable_state_structures`' BigQuery/Spark rows, express the observed-delta recording requirement in the availability layer so the three T5 `bail!` sites become recorded downgrades, and add the structural gate tying every `dialect != DuckDB` guard under `maintenance_driver/` to a structure declared unrealisable | done |
| 12 | Observed deltas on BigQuery: `ddl_bigquery` sidecar emitters plus a `record_observed_delta_with_write` override on a BigQuery multi-statement transaction; flip the row back on, which the phase-11 gate then forces the driver guard to be deleted for | done |
| 13 | Merge ledger and reconciliation ledger on BigQuery: `ON CONFLICT DO NOTHING` re-expressed as `MERGE … WHEN NOT MATCHED`, plus the `execute_write_with_bookkeeping` override | done |
| 14 | Additive never-fold-twice on BigQuery without an enforced `PRIMARY KEY`: re-express the constraint-violation refusal transactionally, red-green on a repeat-fold test | done |
| 15 | Tombstone ledger on BigQuery, so the succession-patch technique runs live rather than downgrading to `DeleteInsert` | done |
| 16 | Close the reopening: re-run `examples/github_activity` live on BigQuery, regenerate coverage, move the ratchets, extend the findings handoff | done |

## Decision log

- 2026-09-12 (inbound, from `20260906-bigquery-dogfood-spine` phase 16): **the criterion-8
  findings handoff is complete and is this outcome's input.**
  `docs/handoffs/2026-09-08-github-activity-findings.md` now carries both halves — its live
  section ("## The live BigQuery half" onwards) banks phases 10–14 and 17, and its
  "## Final punch-list" names nine items with an owner each. Five are this outcome's.

  **The primary input is punch-list item 1: `--event-time-end` does not bound a full
  refresh's source scans on BigQuery.** Six of fourteen relations' oracle at window 1 is
  byte-identical to their oracle at window 30, so "full refresh" and "the oracle at window
  *k*" are not the same operation against a static source. It is invisible on DuckDB, where
  the oracle stages a truncated source. Whether the bound *should* reach those scans is the
  product question this outcome takes a view on; the spine deliberately recorded the
  measurement and did not answer it. The other four inbound items are the window-frame
  lowering seam (which is what keeps the live half at 14 of 16 models), the run-time
  invisibility of the degradation contract's precision half, the alphabetical
  `default_target` fallback, and — filed separately as
  [#203](https://github.com/adbrowne/smelt-sql/issues/203) — the shared `_smelt_ledger`
  serialising every model's bookkeeping.

- 2026-09-11 (phase 16, the live run that closes the reopening): **BigQuery runs the
  pipeline's real plan — 14 models, twice, at full parallelism — and the run found four more
  defects every offline gate had passed.** Eight live runs against `smelt_dogfood`; the
  headline pair is `smelt run --target bigquery --start 2026-08-06 --end 2026-08-07 -e
  silver.actor_sessions -e marts.daily_active_contributors`, 14 success / 0 failed / 0
  skipped both times, the second against the first's committed state. That matches the
  2026-09-11 baseline exactly, and `silver.actor_sessions` still refuses at compile time over
  `LAG`/`MAX` under an INTERVAL `RANGE` frame, unchanged. Total billing, eight runs and every
  verification query: well under a cent.

  **All four defects were in the maintenance/bookkeeping layer, and that is why the BigQuery
  gap ratchet holds rather than falls.** Not one of them is a registry spelling. Three are the
  same species — smelt emitting DuckDB SQL to BigQuery: `CAST(<key> AS VARCHAR)` in the
  driver's changed-key projection (GoogleSQL has no `VARCHAR`, so *every* keyed model with a
  suppressed write failed its observed-delta record); `<col> >= DATE '<start>'` in the
  succession window predicate (DuckDB widens `DATE` to `TIMESTAMP` implicitly, GoogleSQL
  refuses, and both succession models' clock-tie probes died on it); and two direct
  `ddl_duckdb::generate_ledger_*` calls in `execute/project/mod.rs` — unreachable-and-harmless
  while the reconciliation ledger was DuckDB-only, DuckDB SQL in a BigQuery job the instant
  phase 13 declared it realisable there. That third one is the **third** phase to take the
  same fix (13 took two call sites, 15 took two more), which is the argument for gating the
  class instead of the instance.

  **The fourth defect is the one nothing offline could have predicted, and it is the
  guarantee's own shadow.** BigQuery cancels a transaction that mutates a table another
  in-flight transaction is also mutating, and every maintained model's bookkeeping transaction
  mutates the one `_smelt_ledger` table — so a parallel run lost models at random to
  "Transaction is aborted due to concurrent update". The write-conflict detection doing it is
  *precisely* what phase 14's decision log named as the soundness argument for the additive
  never-fold-twice refusal on a dialect whose `PRIMARY KEY`s are `NOT ENFORCED`. The guarantee
  and the failure are one fact, so the fix does not weaken the isolation: a concurrent-update
  abort becomes `BackendError::TransactionConflict`, classified **transient** (the engine rolls
  the transaction back before cancelling it, so re-issuing the identical group is the engine's
  own documented remedy — the only error class in `is_transient`'s exhaustive match where that
  is true by definition), and `BigQueryBackend::ledger_gate` serialises *this process's* ledger
  transactions so a run does not race itself. Retry alone was measured insufficient; the gate
  alone leaves a second writer unhandled; both were needed. The visible cost is honest and
  stated in `docs/specs/state.md`: 108s where the coarsened baseline took 38s.

  **What is proven live.** The tombstone ledger and the patch `MERGE` (phase 15) — both
  succession models executed it for the first time anywhere, `silver_repo_naming`/
  `silver_actor_naming` hold 6,053 rows each and their `__tombstones` tables exist; the
  untyped `NULL` in the domain union's tombstone arm, which GoogleSQL coerces exactly as phase
  15 read it would (a set operation type-checks at plan time, so the probe *running* is the
  measurement); the merge and reconciliation ledgers, read back as 9 rows with no duplicate
  after two runs of the same window — the idempotent `MERGE … WHEN NOT MATCHED` record works
  live; and phase 12's observed deltas, read back as one present-and-empty row
  (`n_keys=0, n_parts=0`) for the fully-suppressed repeat, which is both halves of the
  empty-versus-absent argument and the `ARRAY<STRING>`-without-`NOT NULL` write at once.

  **What remains proven offline only, with the reason.** Four of the eight inherited checks
  were not exercised, and none of them because of a defect: the transactional rebuild (check 3)
  is unreachable because `SourceRetentionExceeded` refuses a whole-table recompute while stored
  output exists; the `already_reflected` sentinel and `@@row_count = 0` (checks 4 and 5)
  because **no cell in this model set grades `Grade::Additive`** — proven, not assumed, since
  run B replayed run A's window against A's ledger and an additive cell would have bailed
  `KeyedReprocessedWindow`; and the non-atomic creating write (check 6) because every target
  already existed. All four are blocked by properties of a *long-lived* dataset, and all four
  are cheaply reachable on the integration suite's ephemeral one. That is the honest shape of
  what a dogfood dataset can and cannot show, and it is recorded rather than glossed.

  **What the spine still owns.** Dual-target **value** parity — its phase 13. The BigQuery leg
  holds three days and the DuckDB fixture thirty, so equal row counts are not expected and
  nothing here compares the populations. What changed for it is that the comparison now weighs
  one plan on two engines: 14 comparable models, no downgraded cell, no coarsened technique.

  **One residual, named rather than quietly carried.** `repair_keys_literal_select` escapes a
  string literal DuckDB-style (`'` → `''`), which does not continue a GoogleSQL string while a
  backslash does escape there — the same portability trap phase 13 fixed for the ledger. The
  path is not reached by `github_activity` and was not exercised live, so fixing it under a
  closing row would have been untested new behaviour; it wants the one-line
  `escape_string_literal` treatment in a phase that can test it.

  **Two pre-existing tests encoded the old claim and were corrected, not deleted** —
  `repair_keys_literal_select_empty_keys_is_dialect_independent` asserted a hardcoded `VARCHAR`
  on all three dialects (false, and a live failure waiting for the first BigQuery repair with no
  affected keys), and `statement_parity::succession`'s fixture restated the typed `DATE '…'`
  predicate instead of calling its owner.

  **Gates:** `verify-phase.sh` ALL GREEN; `dialect_audit` (coverage table regenerated, no
  diff); `statement_parity` + `state_guard_census`; `cargo check -p smelt-cli --features
  bigquery` clean; `large-file-check.sh` OK with **no baseline raised** — `execute/project/
  mod.rs` shrank by 35 lines into a new `ledger_reset.rs`, and `observed_delta/main.rs` by 130
  into a new `rules.rs`, rather than either growing past its cap.

- 2026-09-11 (phase 15, implementation): **The tombstone ledger is realised on BigQuery, the
  row flipped, and the census is now empty.**
  `realisable_state_structures(BigQuery)` is
  `vec![MergeLedger, ReconciliationLedger, ObservedOutputDeltas, TombstoneLedger]`
  (`crates/smelt-logical/src/maintenance/availability/state_structure.rs:67-72`). §5's
  honest-refusal path was **not** taken: every construct the two blocked emitters emit
  either has a GoogleSQL realisation or has one that is provably equivalent, and the two
  whose acceptance could not be settled offline were *removed* rather than guessed at.

  **Three constructs needed a dialect branch and one of them was a genuine correctness
  trap.** `touched_keys_predicate`'s `(k…) IN (SELECT k…)` is a syntax error in GoogleSQL,
  which has no row constructor — but a *single*-key model would have parsed, so the failure
  would have appeared only when someone declared two key columns. It is a correlated
  `EXISTS` over an aliased relation on BigQuery now
  (`crates/smelt-logical/src/maintenance/emit/succession/mod.rs:113-131`), with the aliases
  (`__smelt_presented`, `__smelt_tombstones`) introduced by `build_domain_cte` on that path
  only, since the `IN` form needs none. The other two — `QUALIFY` in the dedup relation and
  a `WITH` inside the `MERGE`'s `USING` — both *exist* in GoogleSQL; what could not be
  settled offline is their acceptance in these exact positions. Rather than ship a guess,
  BigQuery gets nested derived tables with an explicit `ROW_NUMBER() … WHERE
  __smelt_dedup_rn = 1`, which denotes the same relation, is accepted everywhere, and is
  the shape `emit_succession_full_rebuild`'s own fold already used. The fourth branch is
  `DELETE FROM <ledger>` → `… WHERE TRUE`, which GoogleSQL requires.

  **What did *not* need a branch is asserted, not assumed.** The idempotent tombstone insert
  — `INSERT … SELECT … WHERE <flag> AND NOT EXISTS (correlated)` — is byte-identical across
  the two dialects, and `the_tombstone_insert_needed_no_dialect_branch` asserts that
  equality *and* asserts the `MERGE` differs, so the claim cannot pass vacuously. The
  `MERGE` itself, the three-arm `UNION ALL`, `LEAD`/`LAG`, and the whole clock-tie probe are
  unchanged. The probe was fixed for free: the union is built by the private
  `build_domain_cte`, not by the `emit_succession_union_relation` the plan named (no such
  function exists), so making it dialect-plural fixed the probe and the patch at once.

  **The two `assert!`s became one typed refusal, and it names only Spark.**
  `emit_succession_patch`/`emit_succession_full_rebuild` return
  `Result<StatementGroup, UnsupportedSuccessionDialect>`; Spark's absence is stated as
  permanent (no cross-table Delta transaction) rather than pending, matching
  `smelt_state::ledger`'s own wording. The two `#[should_panic]` tests were corrected into
  refusal tests that also assert both realising dialects succeed.

  **The purity rule split the work across two layers, and the split is the interesting
  part.** The tombstone *table*'s DDL is bookkeeping and went to `smelt-state`
  (`src/ddl_bigquery/tombstone.rs`, dispatched by the new `src/tombstone.rs` — third
  instance of the `ledger.rs` pattern, Spark an error by name). Every *statement* stayed
  single-owned in `smelt-logical` and became dialect-plural there, which is what
  `statement_parity`'s structural no-authoring leg proves. `bigquery_type_sql`'s `Result`
  propagates end to end: `TombstoneDdlError::UnmappableColumn` reaches
  `succession/execute.rs`'s `ensure_sqls` via `?`, so a `MAP` key column fails naming the
  column instead of being substituted.

  **Two DuckDB-only call sites were live bugs the moment the guard came down.**
  `succession/execute.rs` still called `ddl_duckdb::generate_ledger_table_ddl` and
  `generate_ledger_upsert_sql` directly — unreachable off DuckDB while the `!= DuckDB` bail
  stood, and DuckDB SQL in a BigQuery job the instant it did not. Both now route through
  `smelt_state::ledger`. Phase 13 deliberately left them; this is the phase that had to take
  them.

  **The census is empty, and proving that still fails closed took real work.** Both
  `STATE-GUARD` bails are replaced by `maintenance_driver::realises_tombstone_ledger`
  (`crates/smelt-runtime/src/maintenance_driver/ledger.rs`), so no raw guard remains
  anywhere under `src/maintenance_driver/`. The verdict logic was extracted into a pure
  `judge(Vec<Guard>) -> Verdicts` and a new control,
  `an_empty_census_still_fails_closed_on_a_planted_guard`, drives it on three planted
  guards — unannotated, unknown-structure, and one annotated with a structure a dialect now
  realises. A stubbed `judge` returning `Verdicts::default()` passes the real test and fails
  the control, which is the whole point.

  **The legible result is criterion 9 stated as an absence.**
  `examples/github_activity`'s two `SuccessionPatch` cells stop downgrading on the
  `bigquery` target, so `github_activity_no_diagnostics` is
  `check_workspace_no_diagnostics` again — the workspace is diagnostic-clean for the first
  time since the `bigquery` target was added, and the cross-target comparison now weighs one
  plan on two engines with no downgrade to discount.

  **What is not proven offline, named rather than glossed.** (a) The domain union's
  tombstone arm projects a bare `NULL AS <payload col>`; GoogleSQL's coercion rules say an
  untyped `NULL` takes the set operation's supertype, but the emitter holds payload column
  *names* only, so `CAST(NULL AS <t>)` would need a new input — if BigQuery rejects it, the
  fix is to thread payload types, not to change the relation. (b) The patch `MERGE` as a
  whole has never executed; its two riskiest sub-constructs were removed rather than
  guessed, but that is an argument, not a measurement. (c) The rebuild group is marked
  `transactional` and opens with `CREATE TABLE … AS`, so
  `write_with_bookkeeping_plan` runs its three statements unbound on BigQuery. That is a
  *degradation* here rather than phase 14's refusal, and the asymmetry is the argument: the
  rebuild is a pure function of the whole retained source, so a partial application is
  repaired by re-running and no bookkeeping row can outlive a write that did not happen.
  Stated in `docs/specs/state.md` and in the emitter's doc comment; phase 16 confirms it
  against the engine.

  **A gate went red for a reason phase 12 did not warn about.** Splitting `succession.rs`
  into `emit/succession/{mod.rs,tests.rs}` (to stay under the 1500-line cap) left
  `maintenance_dialect_blindness` scanning a test module as production code — its
  `#[cfg(test)] mod ... { ... }` stripper cannot reach a module in its own file. Fixed the
  way `state_guard_census` and `statement_parity` already do it: skip `tests.rs` and
  `tests/` in the walk. `statement_parity` itself stayed green, because
  `smelt-logical/src/maintenance/emit/` was already a directory exclusion.

- 2026-09-11 (phase 12, implementation): **BigQuery records observed output deltas, and the
  translation that made it possible was not the one the plan predicted.**
  `realisable_state_structures(BigQuery)` is now
  `vec![MergeLedger, ReconciliationLedger, ObservedOutputDeltas]`
  (`crates/smelt-logical/src/maintenance/availability/state_structure.rs:61-66`). Phase 11 had
  already retired the three T5 write guards and the read guard in favour of the derived
  `records_observed_deltas`, so the row flip *is* the switch: `state_guard_census` is unchanged,
  and nothing was deleted. The row's other clause — "a `record_observed_delta_with_write`
  override" — needed nothing either: that method is
  `Backend::execute_conditional_write_and_record_observed_delta`
  (`crates/smelt-backend/src/lib.rs:697-711`), a thin delegation to the
  `execute_write_with_bookkeeping` seam phase 13 already overrode
  (`crates/smelt-backend-bigquery/src/lib.rs:604`). Both were verified by reading and are now
  asserted rather than re-implemented.

  **The load-bearing translation has four parts, not three.** The plan named `IGNORE NULLS` (no
  `FILTER` clause in GoogleSQL, and `ARRAY_AGG` *raises* on a NULL element rather than yielding
  a NULL array), the `COALESCE` (`ARRAY_AGG` over zero rows is `NULL` on BigQuery too, so it
  stays, spelled `ARRAY<STRING>[]` because a bare `[]` has no element type to unify against),
  and the `MERGE` in place of `ON CONFLICT … DO UPDATE`. The fourth was found by reading the
  caller rather than the DuckDB text: **each element needs `CAST(… AS STRING)`**. GoogleSQL
  coerces no array element type on write where DuckDB folds an `INTEGER[]` into a `VARCHAR[]`
  column, and `changed_keys_select` emits a literal `NULL AS delta_partition` — an INT64-typed
  NULL — for every model with no partition axis
  (`crates/smelt-runtime/src/maintenance_driver/column_scoped.rs:304`). Without the cast the
  `COALESCE` is a type error on *every bare keyed model*, not an edge case. The statement is
  `crates/smelt-state/src/ddl_bigquery/observed_delta.rs`, dispatched by the new
  `crates/smelt-state/src/observed_delta.rs` (`SqlDialect`-keyed, exhaustive, Spark an error by
  name — phase 13's `ledger.rs` pattern, second instance).

  **Empty-versus-absent survives BigQuery's array flattening because it never depended on a
  column value.** BigQuery cannot distinguish a NULL `ARRAY` from an empty one — a NULL written
  to an `ARRAY` column reads back empty — which would be fatal if *absent* meant a NULL column.
  It means **no row for the window**: the upsert's source is one un-grouped aggregate `SELECT`,
  so exactly one row lands per recorded window even over zero input rows, and
  `read_observed_delta` returns `None` iff the row count is zero. Stated as a property of the
  guarantee in `docs/specs/state.md` and pinned by
  `ledger_dialect::on_bigquery_empty_and_absent_are_separated_by_row_presence`. Consequence: the
  two `ARRAY<STRING>` columns carry **no** `NOT NULL` — BigQuery cannot store a NULL array
  anyway, so the constraint is redundant at best and a live-only DDL rejection at worst.

  **§0, the defect phase 14 handed over, was resolved by degrading rather than refusing — and
  the reasons the two cases differ are the whole argument.**
  `sql::write_with_bookkeeping_plan` bound the write group into its transaction
  unconditionally, and a first run's write group is a `CREATE TABLE … AS`, which GoogleSQL
  forbids inside one. Phase 14 refused the analogous shape because its record protects a
  *correctness* guarantee (an unrefused repeat fold double-counts); this record is bookkeeping,
  and refusing a first run over bookkeeping costs a capability for nothing. So the plan now
  returns `BookkeepingPlan { statements, atomicity }`
  (`crates/smelt-backend-bigquery/src/sql.rs:258,300,309`), and a creating write group takes
  `BookkeepingAtomicity::NonAtomicCreatingWrite`: no transaction, **write first and record
  after**, reported at `warn!` by the backend (`lib.rs:616`) and recorded in
  `docs/specs/state.md`. The reordering is sound for a specific reason, not by convenience —
  record-before-write exists because a record reads the target's *pre-write* state, and a write
  that creates the target has none to read (the record's own query would reference a
  nonexistent table); and the surviving exposure is the harmless direction, a created table
  with an unrecorded window (a redundant re-run) rather than a record claiming a write that
  never happened. Detection is a leading `CREATE`, excluding `TEMP`/`TEMPORARY`
  (`creates_a_permanent_entity`, `sql.rs:329`), because `create_group` is the only DDL the
  driver ever puts in a write group; any *other* DDL reaching one would still be bound and
  rejected by the engine, which is noted in the doc comment rather than silently pre-empted.

  **What is not proven offline, and what phase 16 inherits.** The Arrow list type BigQuery's
  adapter returns for an `ARRAY<STRING>` column. `python/smelt/bigquery_adapter.py:88-94` goes
  `result.to_arrow()` → `to_batches()` → `RecordBatch::from_pyarrow_bound`, and
  google-cloud-bigquery conventionally maps a `REPEATED STRING` to `list<…: string>` — but the
  storage-API path and the client version are outside this repo, so that is a reading, not a
  proof. Rather than claim it, the failure mode was removed: `decode_string_list_column`
  (`crates/smelt-runtime/src/maintenance_driver/observed_delta.rs:76`) now accepts both list
  widths over both string widths and **errors** on any other shape or a missing column, where
  it previously early-returned an empty vector. That silent empty was harmless with one
  in-process producer and is a silent-*narrowing* hazard with an adapter: a consumer cannot tell
  an empty decode from an empty delta, so an unrecognised shape would restrict a downstream
  recompute to no keys instead of widening — precisely the class this outcome exists to catch.
  Phase 16 also inherits the live check that the `ARRAY<STRING>` columns without `NOT NULL`
  accept the write, and that a fully-suppressed window lands one present-and-empty row.

  **One gate earned its keep during the split.** `ddl_bigquery.rs` was at its 1007-line
  baseline, so it became `ddl_bigquery/{mod,ledger,observed_delta}.rs` (split, not raised —
  the only baseline change is `--update` dropping the orphaned row). That immediately turned
  `statement_parity`'s no-authoring gate red, because its exclusion was a *file* suffix and the
  owner is now a directory; the fix moves it to `EMITTER_MODULE_DIR_EXCLUSIONS`
  (`crates/smelt-runtime/tests/statement_parity/structural_and_ledger.rs:359-378`), the same
  treatment `smelt-logical/src/maintenance/emit/` already has. Worth recording because it is the
  second time a file split has tripped this gate (phase 14 was the first) — a per-dialect
  renderer owner spread over a directory is now the expected shape, not the exception.

- 2026-09-11 (phase 14, implementation): **BigQuery refuses a repeat fold, and the refusal is a
  statement's effect rather than a storage constraint.** `realisable_state_structures(BigQuery)`
  is now `vec![MergeLedger, ReconciliationLedger]`. The user-visible result is again one line
  gone from a fixture: `examples/github_activity`'s `KeyedFold` cell on `raw.github_events` no
  longer emits `MaintenanceStateDowngraded` on the `bigquery` target, so the workspace is down
  to two diagnostics, both tombstone-ledger (phase 15's), and both remaining cells keep their
  planned technique.

  **What the refusal actually is.** On DuckDB the guarantee *is* the enforced `PRIMARY KEY`
  (`smelt-backend-duckdb/src/lib.rs:748-755`). BigQuery's is `NOT ENFORCED`, so the mechanism
  is re-expressed in three pieces: `smelt_state::ledger::ledger_fold_record_sql` (new) is the
  record whose zero-effect outcome *is* the refusal — a plain `INSERT` on DuckDB, the
  phase-13 `MERGE … WHEN NOT MATCHED` on BigQuery, deliberately the same builder as the
  idempotent upsert so the two can never disagree about what "recorded" means;
  `smelt_backend_bigquery::sql::fold_ledger_delta_script` wraps record + `IF @@row_count = 0
  THEN RAISE` + action in one `BEGIN TRANSACTION … COMMIT TRANSACTION` under an
  `EXCEPTION WHEN ERROR THEN ROLLBACK TRANSACTION; RAISE …` handler; and
  `BigQueryBackend::fold_ledger_delta` overrides the trait default, which is explicitly
  **not** used — its `exists` → `insert` → `action` across three jobs is the check-then-act
  race this row exists to prevent, and shipping it would have satisfied the census while
  reintroducing the defect. The sentinel (`SMELT_LEDGER_ALREADY_REFLECTED`) is emitted and
  matched in one module, `contains`-matched because the adapter wraps it in a job-error
  envelope, with the pairing asserted in a single test so the two halves cannot drift.

  **Decision 2 checked, and the answer is stronger than the plan assumed.** BigQuery's
  "Multi-statement transactions" documentation gives snapshot isolation *and* — the
  load-bearing sentence — "If a transaction mutates (updates or deletes) rows in a table, then
  other transactions or DML statements that mutate rows in the same table cannot run
  concurrently. Conflicting transactions are cancelled." Both folds of one delta mutate
  `_smelt_ledger`, so they cannot both commit: the loser is cancelled loudly, and a later
  re-run reads the committed row and refuses. The soundness argument therefore rests on
  write-conflict detection on one table, not on read-snapshot reasoning — written into the
  emitter's doc comment and into `docs/specs/state.md`.

  **Decision 3, and a capability that was simply wrong.** The same docs say DDL creating or
  dropping *permanent* entities is not supported inside a transaction. The plan's option (b)
  — prove the additive path never produces DDL — is provably false at the call site:
  `driver.rs`'s `action_group` is `create_group` (an `emit_create_table_as`) whenever the
  target does not exist. So the first step is refused, before any backend call, naming the
  construct and the remedy (`--full-refresh` materialises the target outside the
  window-forward loop, after which every step is a merge). The condition is the capability,
  never a dialect: `BackendCapabilities::bigquery()` declared `supports_transactional_ddl:
  true`, which was untrue, and is now `false` with the quote in the comment and the
  `capability_conformance` cell corrected — inert today, since BigQuery does not override
  `execute_statement_group` and the default ignores `StatementGroup::transactional`.
  Decision 4: `exists_sql` is dead on this dialect and is bound `_exists_sql` with a paragraph
  saying why, rather than issued and ignored.

  **The guard left the census rather than being re-annotated**, the second phase running to
  phase 13's precedent: `driver.rs`'s `!= SqlDialect::DuckDB` became
  `maintenance_driver::realises_reconciliation_ledger`, derived from the availability layer, so
  `state_guard_census` now covers two guards (both `TombstoneLedger`) and `ReconciliationLedger`
  joins `ObservedOutputDeltas` and `MergeLedger` in the derive-don't-compare list. The census's
  own directory walk had to learn that a unit-test module can be a `tests/` directory and not
  only a `tests.rs` file — that split was forced by the large-file ratchet
  (`maintenance_driver/tests.rs` 1083 → 1205), and the directory form was chosen because
  `statement_parity`'s no-authoring gate skips `tests/` by that same convention and had flagged
  a flat `ledger_tests.rs`'s fixture `MERGE INTO` text. No baseline was raised.

  **The red was verified to be the right red.** Pointing BigQuery's fold record back at
  `generate_ledger_insert_sql` makes the headline test fail with "a repeat fold must be
  refused, got Committed" — the double-count itself — because the test's fake warehouse models
  an unenforced key honestly: a duplicate `INSERT` simply lands again. Nine pre-existing tests
  encoded the old claim and were corrected, not deleted, including `RecordingBackend::
  capabilities`, which was pinned to `duckdb()` regardless of dialect and would have made the
  new refusal untestable. Gates: `verify-phase.sh` ALL GREEN, `state_guard_census` 3/3,
  `availability_seam` 6/6, `maintenance_availability` 22/22, `maintenance_dialect_blindness`
  3/3, `ledger_dialect` 7/7, `never_fold_twice` 3/3 (new),
  `cargo check -p smelt-cli --features bigquery` clean, `large-file-check.sh` OK.

  **Not proven offline, and one of them is a live bug this phase chose not to fix.** Phase 16
  inherits: that a live repeat really refuses and the sentinel survives the adapter's error
  envelope; that `@@row_count` after a `MERGE … WHEN NOT MATCHED` really is `0` on a repeat.
  And phase 13's open DDL-in-transaction question is now **answered from the docs** — it is not
  permitted — which means `driver.rs`'s `Grade::Idempotent` arm still hands the first
  (table-creating) step's `CREATE TABLE … AS` to `execute_write_with_bookkeeping`, where
  `write_with_bookkeeping_plan` puts it inside the transaction. On the docs' reading the engine
  will reject that script: loudly, on a first run only, and with a fix local to that plan
  function. It is named here rather than left to be discovered live, and deliberately left out
  of this row — different grade, different seam, and changing it here would have meant
  untested new behaviour on a path this row does not touch.

- 2026-09-11 (phase 13, implementation): **BigQuery realises the merge ledger, and the guard
  that refused it is gone from the census rather than merely annotated.**
  `realisable_state_structures(BigQuery)` is now `vec![MergeLedger]`, backed by five GoogleSQL
  builders in `crates/smelt-state/src/ddl_bigquery.rs` and a real transaction seam in
  `smelt-backend-bigquery`. The most legible result is one line deleted from a fixture:
  `examples/github_activity`'s `UpstreamMutation`/`ColumnScopedMerge` cell no longer emits
  `MaintenanceStateDowngraded` on the `bigquery` target
  (`example_diagnostics/smoke_and_migration.rs`), and its **absence** is now the asserted
  claim. A real cell on a real workspace stopped being coarsened.

  **The row's named defect and two the plan did not name.** `ON CONFLICT DO NOTHING` has no
  GoogleSQL form, so the idempotent record is a `MERGE` into the backticked two-part
  `` `<schema>._smelt_ledger` ``, sourced from a one-row `SELECT` of the six literals, matched
  on the four key columns, `WHEN NOT MATCHED THEN INSERT` — `SELECT`, not
  `UNNEST([STRUCT(…)])`, because phase 12's finding 1b is about a row *set* and this source is
  always one row. Found while porting: (a) `PRIMARY KEY` is a syntax error without
  `NOT ENFORCED`, and (b) **string escaping is not portable** — `ddl_duckdb::
  escape_sql_literal` doubles the quote, but `''` does not continue a GoogleSQL string and a
  backslash IS an escape character there, so a model name or partition value carrying either
  would have been silently corrupted. `ddl_bigquery::escape_string_literal` uses the backslash
  form, with its own test.

  **One dispatch point, and a derived gate rather than a comparison.** `smelt_state::ledger`
  (new) is the single `SqlDialect`-keyed `match` — exhaustive, so a new dialect is a compile
  error, and Spark is an error naming the dialect rather than DuckDB SQL it cannot run.
  `driver.rs`'s ledger sites route through it. The guard at `driver.rs:611` was not
  re-annotated but **replaced** by `maintenance_driver::realises_merge_ledger`, derived from
  `realisable_state_structures` the way `records_observed_deltas` is, so it left the census
  entirely: `state_guard_census` now covers three guards, and `MergeLedger` joins
  `ObservedOutputDeltas` in the list of structures whose gate must be derived, never compared.
  The census's own module doc named this as the preferred fix; this is the first phase to take
  it.

  **`ReconciliationLedger` stayed off, and the plan's reason survived contact with the code.**
  `driver.rs:485`'s `Grade::Additive` arm gets its never-fold-twice refusal from
  `fold_ledger_delta` returning `AlreadyReflected`, which on DuckDB *is* the `PRIMARY KEY`
  violation (`smelt-backend-duckdb/src/lib.rs:748-755`). An unenforced key raises nothing, so
  the identical statements would double-count. Its guard stays, with the comment rewritten to
  say the ledger text now exists and only the enforced refusal is missing. Its three builders
  were routed through the dispatch anyway, so row 14 has nothing left but the refusal itself.

  **Not proven, and it is the interesting half.** `BigQueryBackend::
  execute_write_with_bookkeeping` runs `ensure_sqls` as separate jobs then
  `pre_write_sqls` + the write group in one `BEGIN TRANSACTION … COMMIT TRANSACTION` script,
  wrapped in `BEGIN … EXCEPTION WHEN ERROR THEN ROLLBACK TRANSACTION; RAISE …; END` because
  BigQuery does not unwind a script's transaction on its own. Whether BigQuery accepts the
  first step's `CREATE TABLE … AS` *inside* that transaction cannot be settled offline — the
  write group genuinely can be DDL — so it is stated here rather than assumed, and phase 16
  owns it. The contract that IS proven is the ordering and the boundary, asserted against the
  pure `sql::write_with_bookkeeping_plan`'s statement list rather than a warehouse. Where
  there is no record to bind (`pre_write_sqls` empty), no transaction is opened at all.

  **Five pre-existing tests encoded the old claim and were corrected, not deleted** —
  `realisation.rs`'s `has_emitters` (now per `(dialect, structure)`) and its positive
  expectation, `succession.rs`'s `a_ledger_less_dialect_realises_no_ledger` (the merge-ledger
  half is Spark-only now), the census's known-guard count, and the `github_activity`
  diagnostics fixture. Gates: `verify-phase.sh` ALL GREEN, `state_guard_census` 3/3,
  `availability_seam` 6/6, `maintenance_availability` 21/21,
  `maintenance_dialect_blindness` 3/3, new `smelt-state --test ledger_dialect` 5/5,
  `cargo check -p smelt-cli --features bigquery` clean, `large-file-check.sh` OK
  (`ddl_bigquery.rs` 607 → 967, well under the 1500 default cap — no split needed, no ratchet
  raised). No warehouse was reached. **Row 12 is now unblocked on its transactional half**:
  `execute_conditional_write_and_record_observed_delta` delegates to the seam this phase
  overrode, so it needs only its emitters and the row flip.

- 2026-09-11 (live verification of the phase-12 fixes): **proven against the real engine, and
  the run found two more defects that every offline gate had passed.** Three live runs against
  `smelt_dogfood`; the pipeline now builds **14 models** where phase 12 built 10, with gold and
  marts materialising on BigQuery for the first time. Run report
  `20260911-065009-e8436e.json`: 14 success, 0 failed, 0 skipped, 38s.

  **The measurement that was the whole point.** `raw.github_events`' recorded posture baseline
  now holds **3 partitions**, not 5,797 — read back from the state file smelt wrote, against the
  same source and the same fixture phase 12 measured. The arrival twin still holds 2, unchanged.
  Probes ran at their default cadence with no workaround.

  **Defect A — grouping by a bucket expression is not enough on GoogleSQL.** The first live run
  failed both probe-carrying models with `400 SELECT list expression references column created_at
  which is neither grouped nor aggregated` (job 8a576599). GoogleSQL does not match a grouped
  expression referenced from *inside a wrapping expression* in the SELECT list, so
  `SELECT CAST(TIMESTAMP_TRUNC(c, DAY) AS STRING) … GROUP BY TIMESTAMP_TRUNC(c, DAY)` is rejected
  even though the inner expression is grouped. The snapshot emitter now repeats the projection
  verbatim in its `GROUP BY`, wrapping cast included — the same partitioning either way, since the
  cast is injective over the bucket's values, and legal on every dialect. Nothing offline could
  have caught this: DuckDB accepts both spellings.

  **Defect B — the compile-path refusal had a hole the size of a function body.** The second run
  shipped `RANGE BETWEEN INTERVAL '2 days' PRECEDING` to BigQuery (job d0434f7f) *with the
  refusal for that exact construct already in place*. A `smelt.define` call is opaque in the
  model's own CST: the body is inlined by the printer, so the emission walk never saw it. This was
  never specific to window frames — a `//`, a `MEDIAN`, any `Emission::Unsupported` verdict inside
  a function body had the same free pass, which makes it a hole in the
  `dialect_seam` guarantee rather than a missed case. `print_checked` now also walks the
  **expanded source** tree (still smelt SQL, pre-lowering — the same expanded-source pass the
  lookback-bound deriver already makes, not a re-parse of printed output), deduplicated by
  (name, reason) so the model-tree occurrence keeps the span that points at the user's file.
  Regression test: `dialect_seam::refusals::a_refused_construct_inside_a_function_body_is_refused_at_compile_time`,
  asserted for both the frame and `//`.

  **What the refusals look like now.** `silver.actor_sessions` fails at compile time with
  `UnsupportedOnBackend` naming `LAG` and `MAX`, the dialect's limit, and the numeric rewrite —
  before any warehouse round trip. That is the designed outcome, not a regression: the model is
  still unrunnable on BigQuery, but it now says so in the compiler with an actionable message
  instead of costing a job and returning a syntax error. It and its one downstream
  (`marts.daily_active_contributors`) are the two models outside the 14.

  **Cost:** the three runs plus the verification queries billed well under a cent; the heaviest
  read was 19,824 bytes. `githubarchive` was never touched and the loader was not re-run.

  **Still not proven:** dual-target *value* parity. The BigQuery leg holds three days (6,053 rows)
  and the DuckDB fixture holds thirty (64,313), so equal row counts are not expected and nothing
  here compares the two populations — that is the spine's phase 13, and it now has 14 comparable
  models rather than 10.

- 2026-09-11 (phase 12's findings, fixed ahead of the ledger-substrate phases): **four of the
  five findings the first live incremental run produced are closed; the fifth dissolved.**
  Taken out of order deliberately — none of them needed a BigQuery ledger substrate, and two of
  them were wrong on *every* backend, so making phases 12-15 wait on them would have been
  backwards. Commits `ca62743e8` and `4d90ef2d0`.

  1. **The posture baseline is bucketed onto the declared grid** (finding 1a). It grouped by the
     raw partition column, so a TIMESTAMP column under `granularity: day` recorded one
     "partition" per *second* — 5,797 for a three-day, 6,053-row source — and the closed-partition
     reasoning behind the append-only late-arrival classification ran at the wrong unit on every
     backend. DuckDB never complained; a DATE partition column (the dogfood pipeline's arrival
     twin) hides it entirely, which is why it took a TIMESTAMP source on a strict planner to
     surface. New `smelt_logical::classify_partition_bucket` decides from the column's declared
     type and granularity; week buckets are Monday-based on every dialect, matching
     `align_output_start` (GoogleSQL's bare `WEEK` is Sunday-based, so `WEEK(MONDAY)` is explicit).

  2. **GoogleSQL's inline row set is one operand, not one per row** (finding 1b). The chained
     `SELECT … UNION ALL SELECT …` is valid and does not scale; `SELECT * FROM UNNEST([STRUCT(…),
     …])` is GoogleSQL's own form and has no per-row planning cost. Asserted as a property at
     5,797 rows, the size the live run actually refused.

  3. **`FILTER (WHERE …)` and `INTERVAL` RANGE frames are refused at compile time** (findings 2
     and 3), as dialect facts rather than registry verdicts, since neither clause belongs to any
     one function. Deliberately *not* auto-lowered: the `CASE WHEN` form is equivalent only for
     NULL-ignoring aggregates (it would change `ARRAY_AGG`) and that property is not yet registry
     data, and the interval frame's GoogleSQL form needs the `OVER` clause's `ORDER BY` rewritten
     in a printer seam that does not exist. Both limits are stated in `multi_backend.md`
     §"Clause-level dialect refusals" rather than left as folklore.

  4. **Finding 5 dissolved rather than being fixed.** It was "no single committed config serves
     both targets", and the whole reason was finding 1 — the `probes: cadence: off` workaround.
     With 1a and 1b fixed the workaround is deleted from `examples/github_activity/smelt.yml` and
     the DuckDB negative control keeps its teeth.

  **Every residual gap is now tracked as an issue**, not only as spec prose: #200 (the `FILTER`
  lowering, waiting on a registry null-input disposition), #201 (the `INTERVAL` frame lowering,
  waiting on a window-spec dialect seam) and #202 (finding 4's structured half). Each is also a
  Known Divergence in its owning spec.

  **What is left of finding 4, stated rather than quietly dropped.** The precision-downgrade
  skip sites now log at `warn!` instead of `debug!`, so an operator sees them without raising the
  log level. The *structured* half is not done: no per-model record in the run manifest or report,
  and `smelt explain` still takes no `--target`, so there is no offline way to ask what a target
  would give up and no machine-readable record after the fact. The availability layer already
  knows the answer statically, so this is plumbing rather than derivation. Now a Known Divergence
  in `docs/specs/state.md`; it wants its own phase with a spec diff, because it adds surface.

  **Not yet proven live.** Every fix above is held by offline tests and the full gate
  (`verify-phase.sh` green), but no BigQuery run has exercised them — the six models findings 2
  and 3 blocked, and the probe path finding 1 blocked, are still unproven on the real engine.
  That proof belongs to the spine's phase 13, or to phase 16 here.

- 2026-09-10 (phase 11, implementation): **both layers now agree, and the agreement is held
  structurally rather than by inspection.** `realisable_state_structures` returns `vec![]`
  for BigQuery and SparkSQL: `ObservedOutputDeltas` and `FingerprintSidecar` were both false
  claims. The sidecar was the open question the reopening left for this phase, and it
  resolved as a *second* instance of the same lie — it also contradicted a source of truth
  already in the tree, `BackendCapabilities::supports_fingerprint_sidecar`, `true` for DuckDB
  alone and gated on by every consumer in `maintenance_driver/sidecar.rs`. That contradiction
  is now its own test.

  The three T5 `bail!`s are gone. Rather than annotate the hardcoded comparisons, they route
  through one predicate — `maintenance_driver::records_observed_deltas(dialect)`, *derived
  from* `realisable_state_structures` — so phase 12 flipping BigQuery's row on retires its own
  guard and the two cannot drift apart again. Where the structure is unrealisable the write
  still happens and only the record is skipped; the read side has always treated an absent
  delta as a legal widen-never-narrow trigger, so the cost is downstream precision, never
  correctness.

  Criterion 8's gate is `cargo test -p smelt-runtime --test state_guard_census` (3 tests):
  every remaining `SqlDialect::DuckDB` comparison under `maintenance_driver/` must carry
  `// STATE-GUARD: <StateStructure>` and that structure must be unrealisable off DuckDB. Four
  remain (two `TombstoneLedger`, one each `MergeLedger`/`ReconciliationLedger`), all
  reachable only if the plan layer failed to downgrade first. The contradiction leg was
  verified by temporarily re-adding `MergeLedger` to BigQuery's row — it failed naming file,
  line, structure and dialect — then reverted; the scanner's annotation/prose/fixture
  discrimination has its own synthetic controls.

  **Three pre-existing tests encoded the lie and were corrected, not deleted**:
  `maintenance_availability/succession.rs`'s `a_ledger_less_dialect_realises_no_ledger`,
  `availability_seam`'s intersection test (which gained a DuckDB comparison so its claim is
  still about the dialect rather than `warehouse_tables`), and — found only after the first
  verify run — `observed_delta.rs`'s `keyed_fold_suppressed_recording_refuses_a_non_duckdb_backend`,
  now `..._degrades_on_a_non_duckdb_backend`. That third one was hidden behind `cargo test`'s
  fail-fast: the large-file ratchet failed first and the run never reached `smelt-runtime`.
  Worth remembering — a ratchet failure can mask real test failures in later crates.

  `crates/smelt-runtime/tests/observed_delta.rs` (1331 lines, exactly at its cap) became the
  directory target `observed_delta/{main.rs,degradation.rs}`, matching the existing
  `availability_seam/`/`dialect_seam/` pattern; the fake backend's ~110 lines of trait stubs
  moved out with the degradation test. The baseline changed by one line and **downward**
  (1331 → 1197) — the sanctioned `--update` for an orphaned entry after a split, not a raised
  ratchet, so criterion 4 is satisfied by a fall. The fake's `get_row_count` had to stop being
  `unimplemented!()` because the run now completes instead of refusing before any write —
  itself evidence of the behaviour change, and commented as such.

  Spec delta landed: `docs/specs/state.md` gained §"Which dialects realise which structure"
  (BigQuery "not yet", Spark "**no**" with the Delta cross-table-transaction reason) plus the
  two binding rules, and §"The degradation contract" now separates *losing a technique* from
  *losing precision* — the distinction whose absence let a precision loss be treated as a
  refusal. Gates: `verify-phase.sh` ALL GREEN (fmt, clippy both feature sets, workspace
  `cargo test`, `example_diagnostics`), `state_guard_census` 3/3, `observed_delta` 14/14,
  `availability_seam` 6/6, `maintenance_availability` 20/20,
  `maintenance_dialect_blindness` 3/3.

  **The spine's phases 12-14 are unblocked**: `silver.events_deduped` no longer refuses on
  BigQuery. It will run the downgraded (`PerGroupRecompute`) plan until phases 12-15 land the
  real substrate, which is the coarser-but-equivalence-preserving posture the reopening
  planned for.

- 2026-09-10 (reopening): **the outcome reopens for the T5 gap, and the defect is not the
  one the spine filed.** The spine recorded "BigQuery has no observed-delta bookkeeping".
  The actual defect is a contradiction between two layers that are each internally
  consistent: `crates/smelt-logical/src/maintenance/availability/state_structure.rs:23-26`
  declares that BigQuery and Spark **do** realise `StateStructure::ObservedOutputDeltas`
  ("every dialect realises the sidecar/output-delta structures, which have no per-dialect
  builder gate"), while every writer of that structure is `smelt_state::ddl_duckdb` behind
  a hard `bail!` on any non-DuckDB dialect — `maintenance_driver/driver.rs:636`,
  `column_scoped.rs:357`, `membership/execute.rs:57`. Believing the structure available,
  `resolve_availability` records no downgrade, and the run dies where the *merge ledger*
  block twelve lines above (`driver.rs:610`) — correctly declared unrealisable — skips with
  a `tracing::debug!` and lets the cell's own `state_downgrade` be the user-visible
  channel. That is the mechanism the T5 path was supposed to use and does not.

  Two facts that shaped the fix. The technique downgrade *did* fire correctly
  (`KeyedFold`→`PerGroupRecompute` via the absent `ReconciliationLedger` — the four
  `MaintenanceStateDowngraded` diagnostics the spine saw); T5 recording is orthogonal to
  technique and bit the already-downgraded plan anyway. And the read side already tolerates
  absence — `observed_delta.rs:82` returns `Ok(None)` on any non-DuckDB dialect, which
  `since_upstream.rs` and `delta_restriction` document as "always a legal fallback
  trigger", i.e. conservative over-propagation. So the downgrade path was never blocked on
  a missing fallback; it was blocked on the availability layer's claim.

  **Decision: realise the substrate on BigQuery (not merely downgrade), across the whole
  `!= DuckDB` family, with Spark carved out.** Scoping evidence gathered before choosing:
  `ddl_bigquery.rs` and `ddl_spark.rs` exist but hold schema-evolution DDL only — all five
  state structures live solely in `ddl_duckdb.rs`, so this is the substrate's second
  implementation, not a spelling. DuckDB is the only backend overriding any transactional
  seam (`smelt-backend-duckdb/src/lib.rs:631,726,786`); BigQuery overrides only
  `delete_and_insert_transactional`, Spark none, so bookkeeping and write are sequential
  and non-atomic on both today. The four guarantees are not equally portable: BigQuery has
  real multi-statement transactions (`BEGIN TRANSACTION … COMMIT` as one query job, DDL not
  permitted inside — matching the trait's existing "keep the `IF NOT EXISTS` DDL outside
  the transaction" precedent), but its `PRIMARY KEY`s are declared-and-unenforced, so the
  additive never-fold-twice refusal — today a PK constraint violation caught as
  `already_reflected` (`smelt-backend-duckdb/src/lib.rs:748-755`) — has to be re-expressed
  transactionally. That is the load-bearing phase: get it wrong and an additive fold
  double-counts. Delta has per-table atomicity and no cross-table transaction at all, so
  Spark's realisation is refused here rather than attempted; its row is corrected to tell
  the truth and the reason lands in the spec.

  **Ordering is the deliberate part: the honesty fix goes first, not last.** Phase 11
  reconciles the two layers and adds the structural gate *before* a line of new SQL, which
  (a) unblocks the spine's phases 12-14 immediately, without waiting on phases 12-15, and
  (b) converts every later phase into ratchet-down work — flipping a dialect's row back on
  forces its driver guard to be deleted, and deleting a guard without an emitter behind it
  fails. Neither half can be satisfied by a promise. Spec-first obligations: `docs/specs/
  state.md` §"The state-structure inventory" and §"The degradation contract" (per-dialect
  realisation becomes a stated property rather than a code detail) and
  `docs/specs/incremental_shapes.md` (never-fold-twice becomes dialect-plural). No conflict
  with maintenance-plan purity — `CLAUDE.md` already excludes `smelt-state` ledger DDL/DML
  as bookkeeping. Open and left to phase 11 rather than assumed: whether
  `StateStructure::FingerprintSidecar`, claimed realisable on both non-DuckDB dialects, is
  actually backed, or is a second instance of the same lie.

- 2026-09-08 (phase 10 implementation, close): **outcome done.** All success criteria
  verified at HEAD as the planning entry laid out. New structural gate
  (`cargo test -p smelt-logical --test maintenance_dialect_blindness`, 3/3) closes
  criterion 3's remaining gap; `handoff_claimed_relations()` in
  `crates/smelt-cli/tests/github_activity_oracle.rs` is now scoped to `## The registered
  divergences` (fixing the false-positive phase 9 flagged), with two new tests proving
  both the scoping and its non-vacuity. `docs/handoffs/2026-09-08-github-activity-findings.md`
  gained a `## Close-out (2026-09-08)` section (one row per criterion, artifact + gate) and
  `.claude/dialect-gaps-baseline.txt` gained a dated hold note —
  `dialect_gaps_bigquery` stays 42, `duckdb_seed_gaps 0` confirmed untouched.
  `SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test dialect_audit
  the_coverage_table_matches_the_registry` reconfirmed `docs/reference/dialect-coverage.md`
  byte-identical (`git status` clean). Issue #179 got a comment
  (github.com/adbrowne/smelt-sql/issues/179#issuecomment-5581444485) naming what was fixed
  nearby and that its 42 entries are untouched — not closed. All gates green:
  `verify-phase.sh`, `maintenance_dialect_blindness` (3/3), `github_activity_oracle` (18
  passed, 1 ignored), `dialect_audit` (61/61), `emission_ownership` (11/11), `dialect_seam`
  + `projection_dialect_invariance` (18/18 + 4/4), `googlesql_render` (4/4),
  `large-file-check.sh`. Row 10 flipped to `done`; outcome Status flipped to `done`.

- 2026-09-08 (phase 10 planning): **no reshape; row 10 is the last row and its "move the
  ratchets down" clause resolves to *hold*, with the reason written into the baseline
  file.** Measured at HEAD: `cargo test -p smelt-db --test dialect_audit` is 61/61 green,
  so `the_coverage_table_matches_the_registry` already passes (the regeneration is a
  no-op to be confirmed, not a pending edit) and `gap_count_ratchet` already matches
  `dialect_gaps_bigquery 42`. That count cannot fall here: every fix phases 1-9 landed was
  in `smelt-logical`'s maintenance emitters, not in `BuiltinRegistry`, and giving the 42
  no-verdict entries verdicts speculatively is forbidden by criterion 2 and already
  recorded under Out of scope. Criterion 4's "fall or hold; neither is raised" is therefore
  satisfied by holding, and the phase's job is to make that legible rather than to move a
  number. Two pieces of real work remain and are in the phase: (a) criterion 3 is still
  short one gate — the defect class this outcome opened on (an emitter taking a `dialect`
  parameter and hardcoding `MaintenanceDialect::DuckDb` anyway) is held only by per-emitter
  unit tests, so a *new* emitter could reintroduce it silently; a structural scan over
  `crates/smelt-logical/src/maintenance/` with a planted-needle non-vacuity control closes
  that. (b) `handoff_claimed_relations()` in `github_activity_oracle.rs` scans the whole
  handoff for any `` | ` ``-leading row, which phase 9's summary flagged as a false-positive
  trap — and phase 10 must itself append a close-out section to that handoff, so scoping
  the scan to the divergence table is required work, not cleanup. Issue #179 gets a
  comment, not a close: its 42 entries are untouched.

- 2026-09-08 (phase 9 implementation): **the plan's citation table verified exactly as
  written; the durable half is now landed and no live re-run is needed.** All six commits
  (`7a2eb89d0`, `af972abe0`, `0178e6bd4`, `d84320a44`, `e028596e3`, `aee113753`) confirmed via
  `git show`; `modulo_lowering`/`power_lowering` both pass at HEAD unchanged. New
  `crates/smelt-maintenance-testkit/tests/googlesql_render.rs` (4 tests: the two positive
  scans over every `DagBody` variant across all six DAG recipes and all four `ComposedRoute`s,
  a non-vacuity negative control over all seven refused-construct needles, and a fail-loud
  check on an unparseable body) — none of the scans found a live needle, so no fix was needed,
  only the gate. `crates/smelt-cli/tests/maintenance_conformance_bigquery/backend.rs` gained
  `bigquery_oracle_relation_issues_no_ddl_and_returns_an_inline_subquery` against the REAL
  `BigQueryConformanceBackend` (not a stand-in fake) and a real in-memory-equivalent
  `DuckDbBackend`, gated `#[cfg(feature = "duckdb")]` (on by default alongside `bigquery`) so
  it needs no warehouse and no `SMELT_BQ_PROJECT`. `main.rs`'s doc comment and
  `gate_composed_bigquery.rs`'s doc comment both retired their stale
  "uncharacterised"/"not yet re-confirmed" wording in favour of the plan's table plus the
  2026-08-21/2026-08-22 sweep results. Spec delta landed
  (`docs/specs/multi_backend.md` §"Known Divergences": "The BigQuery conformance leg's live
  evidence has a date", naming the 2026-08-22 sweep and the five offline gates that stand in
  for a live re-run between sweeps). `docs/handoffs/2026-09-08-github-activity-findings.md`
  gained a §"Criterion 6" section carrying the table verbatim; its leading-cell format had to
  change from `` | `name` `` to `` | Test: `name` `` after `findings_handoff_names_no_unknown_relation`
  (a pre-existing generic scan for any `` | ` `` -leading markdown row, not specific to the
  divergence-registry table) flagged both new rows as stale registered-divergence claims — a
  real, if narrow, false positive in a gate this phase didn't own, fixed by reformatting rather
  than touching the gate. All gates green: `verify-phase.sh`, `googlesql_render` (4/4),
  `modulo_lowering`+`power_lowering` (11/11), `maintenance_conformance --features duckdb`
  (101/101, including `dags::diamond_propagation_suffices` and the `composed_pool` family),
  `cargo check -p smelt-cli --features bigquery --tests`, the new BigQuery-gated test (1/1,
  `SMELT_BQ_PROJECT` unset), `large-file-check.sh`. Row 10 is unchanged.

- 2026-09-08 (phase 9 planning): **no reshape, and no block — the two failures are already
  characterised AND fixed in the repo record, so phase 9 is offline forensics plus the gate that
  makes the characterisation durable.** Criterion 6 reads as open only because this outcome's row
  9 inherited the 2026-08-16 handoff's "uncharacterised" wording. The record since disagrees:
  `diamond_propagation_suffices_on_bigquery` is `WHERE id % 2 = 0` reaching GoogleSQL unlowered
  (`400 Syntax error: Expected ")" but got "%"`, measured live 2026-08-19), fixed by `7a2eb89d0`
  (`%`→`MOD`) plus `af972abe0` (`^`→`POWER`, the worse silent-wrong-number sibling found chasing
  it); `composed_keyed_pool_upholds_equivalence_on_bigquery` had no mechanism of its own and was
  collateral from three already-closed gaps (`INSERT *` in the keyed-fold MERGE `0178e6bd4`, the
  `DROP` object-type mismatch `d84320a44`, the hand-rolled `FROM (VALUES …)` row set
  `e028596e3`/`aee113753`), confirmed live in the 2026-08-19 sweep. Both then passed the
  whole-sweep measurements of 2026-08-21 (21/21, 2190.85s) and 2026-08-22 (22 cases, 621.61s
  concurrent). So the phase does not need the live leg and must not emit `<<PHASE_BLOCKED>>`
  under criterion 7. What it does need is the durable half, which does not exist: the diamond
  mechanism is gated at the printer (`modulo_lowering`, `power_lowering`) but nothing ties the
  *testkit's own rendered recipe bodies* to those lowerings, which is exactly the seam that let a
  `%` reach a live warehouse in the first place — hence the new `googlesql_render` gate over
  every `DagBody` and the composed pool's rendered bodies, with a non-vacuity control in phase
  8's shape. The one genuinely unrunnable item — re-confirming green at today's HEAD, after
  phases 1-8 touched maintenance emitters — is recorded as a dated, named debt in the spec and
  the handoff rather than skipped green; it belongs to the spine's blocked live half. Row 10 is
  unchanged.

- 2026-09-08 (phase 8 implementation): **fail-closed proof landed as planned; criterion
  5's cross-target half registered nothing because the spine never ran live BigQuery.**
  `check_matches_oracle` (a `Result`-returning split of `assert_matches_oracle`) plus five
  new tests (`assert_matches_oracle_fails_closed_on_an_empty_registry`,
  `check_bound_accepts_a_holding_bound`, `check_bound_rejects_a_leading_side`,
  `check_bound_rejects_divergence_outside_the_licensed_columns`,
  `no_relation_diverges_unexplained`) now exercise the registry-consulting comparator, both
  `check_bound` arms/`Side` variants, and criterion 5's zero-unexplained-count claim
  directly, sharing a new `perturbed_one_day_pair()` staging helper with
  `an_unregistered_divergence_fails`. `registry_entries_are_all_live` gained the same
  direct check on an empty registry so its own loop cannot pass vacuously either. The
  `#[allow(dead_code)]` attributes on `Bound` and `Side` are gone — both are now
  constructed by real tests. `docs/handoffs/2026-09-08-github-activity-findings.md`'s
  divergence section now names these five tests, so "empty registry" reads as "measured
  and found nothing," not "never measured." Criterion 5's **cross-target**
  (DuckDB-vs-BigQuery) half registered nothing to resolve: the spine
  (`docs/outcomes/20260906-bigquery-dogfood-spine`) is `blocked` with its live-BigQuery
  half (its phase 16) never run, so it produced no dual-target divergence at all — already
  covered by this outcome's Out of scope bullet on BigQuery-only defects; recorded again
  here so criterion 5 does not read as half-checked. All gates green: `verify-phase.sh`,
  `github_activity_oracle` (16 passed, 1 ignored measurement sweep, 115s), and
  `github_activity_replay` (17 passed, 57s). File grew from 975 to 1151 lines, under the
  1500-line default cap with no baseline entry needed.

- 2026-09-08 (phase 8 planning): **no reshape; row 8's content is now the fail-closed
  proof, and criterion 5's cross-target half is answered rather than left open.** Phases
  3-7 fixed all five registered divergences instead of promoting any, so
  `DIVERGENCE_REGISTRY` is empty and criterion 5's "unexplained count zero" holds — but
  three of the registry's own gates (`succession_divergence_is_exactly_tied_row_
  multiplicity`, `registry_entries_are_all_live`, `every_registry_entry_is_named_in_the_
  findings_handoff`) now iterate an empty slice and pass by construction, and neither
  `check_bound`'s `MonotoneDivergence` arm nor `assert_matches_oracle`'s unregistered
  branch has any live test: the existing negative control
  (`an_unregistered_divergence_fails`) stops at `compare_databases` and never reaches the
  registry-consulting comparator. Phase 8 is therefore exactly that proof, plus one
  directly-named zero-unexplained-count assertion. The **cross-target** reading of
  criterion 5 (the spine's criterion 6 is DuckDB-vs-BigQuery dual-target parity) has no
  residual work: the spine is `blocked` with its live-BigQuery half never run, so it
  registered no dual-target divergence at all — already covered by this outcome's Out of
  scope bullet on BigQuery-only defects, and recorded again by phase 8's task 9 so
  criterion 5 does not read as half-checked. Rows 9 and 10 are unchanged.

- 2026-09-08 (phase 7 implementation): **the mechanism worked on the first try; the tutorial
  freshness gate did not move.** Task 1's inspection confirmed the plan's diagnosis exactly
  (the clocked cell already exists; the run-window widening was the missing piece), so
  `IncrementalWindows::output_window()`, the pure `widen_run_window_for_upstream_outputs`
  helper, and threading both through `build_model_plans` (recording each model's output
  window in a map keyed by name, consulted by name via `refs` before the `frozen_horizon`
  clamp) closed `marts_daily_active_contributors`'s divergence on the first run of the full
  30-day oracle — no second-attempt fix was needed, unlike phases 3 and 6.
  `cargo test -p smelt-cli --test tutorial_freshness --features duckdb` passed unmodified
  (no regeneration needed): the web-analytics tutorial's directive commands apparently never
  select a Form-B upstream and its Form-A downstream together in one invocation the way the
  plan's task 8 anticipated, so the widening never triggers there. `DIVERGENCE_REGISTRY` is
  now empty; `findings_handoff_names_no_unknown_relation`'s "claimed non-empty" assertion had
  to be loosened to accept an empty table when the registry itself is empty (a fixed-in-phase
  consequence of the registry emptying now rather than in phase 8, not a new mechanism) — see
  `phases/07-summary.md`.
- 2026-09-08 (phase 7 planning): **no reshape of the phase order; row 8 reworded, and the
  handoff's open question is answered inside phase 7 rather than by a row of its own.**
  Reading the code settled punch-list item 4's mechanism: the edge from `silver.actor_sessions`
  to `marts.daily_active_contributors` is *clocked*, so `append_model_edge_cells`' clock route
  already derives a `NewData` / `RecomputeRegion` / `DeleteInsert` cell for it — the maintenance
  cell is not missing. What is missing is the window it is ever dispatched over: `build_model_plans`
  gives every model the invocation's requested run window verbatim, so a Form-A downstream never
  learns that its Form-B upstream rebased `[D-1, D+2)` on a `[D, D+1)` run. Phases 4-5's
  enrichment-keyed route therefore does **not** subsume this (it is key-addressed value enrichment
  for a *clockless* upstream; this read is membership-sensitive and clocked), which is the handoff's
  question answered — phase 7 confirms it by inspection as its first task rather than carrying a
  separate row. The fix is one rule, stated in `docs/specs/incremental_models.md` §"Forward
  propagation" for an explicit landed delta but never applied to an ordinary windowed run: a model's
  run window is the union of the requested window and every in-run upstream's derived output window.
  Row 8 is reworded because phases 3-7 are expected to leave `DIVERGENCE_REGISTRY` empty, at which
  point its real content is proving the unregistered-divergence sweep still fails closed rather than
  passing vacuously — criterion 5 stays owned by a row either way.
- 2026-09-08 (phase 7 planning): **phase 6's suggested extra `statement_parity` lookback+skew+chunking
  fixture does not get a row.** The regression it names is already gated at its source by
  `windowing_form_b_chunking.rs::lookback_and_skew_widen_independently_never_summed`; a second fixture
  asserting the same property further from the code would duplicate, not widen, coverage, so criterion 3
  is satisfied without it. Recorded here rather than under Out of scope because no work is leaving the
  outcome — it was never in it.

- 2026-09-08 (phase 6 implementation): **the plan's diagnosis targeted a dead field; the real
  fix needed a second layer.** `IncrementalBatch::filter_start`/`filter_end` — what the plan's
  formula computes — turned out to have zero consumers anywhere in the real execute path
  (`rg`-confirmed): `derive_batch_filtered_sql` (`crate::execute::sources`) widens each bounded
  source's scan from `run_range` (`batch.partition_start`/`partition_end`, unwidened) plus a
  *per-source*, independently-derived lookback/lookahead bound (`per_model_source_bounds`) —
  never from `filter_start`/`filter_end` at all. So the phase 6 plan's formula, applied only in
  `windowing.rs`, was inert against the actual defect; the `github_activity` oracle test still
  failed after it (a real cross-midnight session still truncated at the interior chunk
  boundary). Fixed by threading a new `scan_range` parameter through `derive_batch_filtered_sql`
  and its three call sites (`execute/project/mod.rs`, `execute/project/dry_run.rs`,
  `smelt-cli/explain.rs`), sourced from a **new** `IncrementalBatch::scan_start`/`scan_end` field
  pair — deliberately not a repurposing of `filter_start`/`filter_end`. The first attempt reused
  `filter_start`/`filter_end` (already skew-widened) as `scan_range`, which passed the
  `github_activity` oracle but **double-widened** the lookback component for any model with a
  nonzero SQL-inferred lookback: `derive_batch_filtered_sql` still adds `per_model_source_bounds`'
  own lookback on top, so a model carrying both a real lookback and a chunked run got its scan
  literal widened twice. Caught by `web_analytics_tutorial_pages_are_fresh` (the doc-freshness
  gate), not by any windowing-crate test, because none of them combine a nonzero lookback with
  chunking and a literal-text assertion — see phases/06-summary.md "For the next planner" for the
  gap this leaves. `scan_start`/`scan_end` carries skew alone (clamped to the outer envelope,
  same clamp as the plan's original formula); `filter_start`/`filter_end` keeps its pre-existing,
  lookback-only meaning untouched.

- 2026-09-08 (phase 6 planning): **no reshape; the fix is scan-side and clamped to the existing
  outer envelope.** Reading `compute_calendar_windows` confirmed the row's diagnosis and pinned the
  mechanism: the Form-B relation has *two* inversions — a write-side one (run window → output window
  `[start − after, end + before)`, implemented, applied once per invocation) and a scan-side one (to
  write partitions `[bs, be)` the scan must cover driving dates `[bs − before, be + after)`, never
  implemented). A single-chunk invocation covers the scan side incidentally because its batch bounds
  *are* the output-window bounds; every interior boundary loses it. The fix folds the skew into the
  per-batch filter but clamps it to the invocation's existing outer scan envelope, so single-chunk
  literals stay byte-identical and no existing statement-parity fixture moves — the narrower choice,
  taken deliberately: whether the outermost chunk should also read past the run window's trailing edge
  is a data-availability question this defect does not raise. Both specs already assert the correct
  rule ("each sized from its own chunk's reach"; scan "relative to the derived output window"), so the
  spec delta is a clarifying sentence per file naming the skew inversion and the property it buys —
  output invariant under chunk count. The integer axis needs no change (nonzero skew is already refused
  fail-closed there). Phase 7's `marts_daily_active_contributors` entry is expected to *shift* under
  this fix (its upstream now writes fuller sessions), so phase 6 re-measures and re-registers it
  without fixing it.

- 2026-09-08 (phase 5 implementation): **the enrichment-keyed cell is live;
  `github_activity`'s stale-row count reaches zero.** `resolve_live_column_
  scoped_cell` gained a `model_edges` parameter and switched to
  `derive_resolved_with_edges` when non-empty; `decide_column_merge_dispatch`
  excludes an `EnrichmentKeyed` cell from per-batch dispatch; a new
  `execute/enrichment_heal.rs` dispatches it once per run over the model's
  unwindowed output. The mutation gate's existing `None`-on-missing-
  `SourceInfo` behaviour already implements the plan's "fails open to
  dispatch" posture for an edge trigger — no new gating code was needed, only
  tests and a spec sentence naming the property. `execute/project/mod.rs`
  grew 52 lines past its large-file baseline (the two call sites' own
  ~20-argument lists, irreducible without moving locals); bumped with a
  sign-off note rather than left red. Full 30-day replay
  (`enrichment_heal_repairs_rows_written_before_the_rename`,
  `gold_events_enriched_matches_the_full_refresh_oracle`) confirms zero stale
  rows; the plan's `statement_parity` isolation test (test 5) was not added —
  see `phases/05-summary.md` "For the next planner".

- 2026-09-08 (phase 5 planning): **no reshape to the phase rows; one item moved to Out of
  scope.** Reading the run path confirmed phase 4's split was right and phase 5's scope is
  exactly as written — the resolver takes no edges, so the derived cell is invisible to both
  dispatch branches. One design question the row did not name is settled in the plan: an
  enrichment-keyed cell's write is addressed by the join key, not by a partition interval,
  so the existing per-batch `ColumnMergeDispatch::Full` arm (which MERGEs the *window-
  filtered* compiled SQL) cannot heal rows written on earlier days and would leave the stale
  count non-zero. The cell is therefore excluded from the per-batch dispatch and dispatched
  once per run over the model's unwindowed output, licensed by the edge's declared
  `allow_full_scan`. Phase 4's LSP-diagnostics finding is recorded under Out of scope: it is
  a pre-existing editor-surface gap serving none of this outcome's success criteria.

- 2026-09-08 (phase 4 implementation): **found, did not fix, a pre-existing LSP-diagnostics
  gap for every model-edge refusal.** `smelt-db`'s LSP-facing `maintenance_plan` Salsa query
  (`maintenance_plan_diagnostics`, what `file_diagnostics()` calls) is wired to the
  source-only `derive_model_maintenance_plan`, never `..._with_edges` — so no model-edge
  refusal (not just the new `RepairKeysNotDiscoverable`; `ReachNotDerivable` has the
  identical, already-documented gap) has ever reached `file_diagnostics()`/the editor; only
  `smelt explain` (`maintenance_plan_report`) sees them. Phase 4's own diagnostics test was
  rewritten against `plan_for` (the `explain` query) with the gap named inline rather than
  silently expanding this phase to also thread edges into the LSP query — see
  `phases/04-summary.md` "For the next planner" for the follow-up.
- 2026-09-08 (phase 4 implementation): **the enrichment-keyed route needed a guard the plan
  didn't spell out — restricted to an actual JOIN.** Without checking that the edge resolves
  via `enrichment_join_clause` at all, the route also fired for a plain `FROM smelt.<edge>`
  driving relation (no join, i.e. the edge IS the sole source) — not value enrichment at all —
  and broke `keyed_model_edge.rs::consumer_not_carrying_upstream_keys_is_refused` (a `ScanUnbounded`
  refusal instead of the expected `RepairKeysNotDiscoverable`). Fixed by returning `Ok(None)`
  early when the edge is not resolvable as an enrichment join; full workspace gate green after.

- 2026-09-08 (phase 4 planning): **reshape — punch-list item 2 splits into a derivation
  phase (4) and a run-path phase (5); old rows 5-9 shift to 6-10.** Reading the code for the
  missing `UpstreamMutation(gold.repo_dim)` cell showed the work is two separable layers, not
  one. (a) *Derivation*: `append_model_edge_cells` today offers a clockless `KeyedUpsert` edge
  only the key-addressed `PerGroupRecompute` route, whose two discovery legs (upstream-keyed,
  grain-over-upstream) both need the DOWNSTREAM's grain to resolve against the upstream
  relation — impossible for a `grain: partition` downstream like `gold.events_enriched`. But
  the shape is not a per-group recompute at all: it is the value-enrichment shape
  (`Technique::ColumnScopedMerge`) smelt already derives for a declared `mutation_profile:
  mutable_snapshot` dimension, and its write addressing is the *join* key carried in the
  downstream's own output (`repo_id`), which IS discoverable. So the fix is a third,
  enrichment-keyed route — parity between a mutable-snapshot source dimension and a clockless
  keyed model dimension — not a widening of the existing two. (b) *Run path*: the runtime's
  live-cell resolver (`resolve_live_column_scoped_cell` →
  `maintenance_availability::derive_resolved`) calls the source-only
  `derive_model_maintenance_plan`, never `..._with_edges`, so it cannot see a model edge at
  all; the mutation gate and the dimension-`unique_key` lookup likewise search `source_infos`
  only. Making the derived cell actually dispatch is its own chunk of work with its own gate
  (the `github_activity` stale-row count reaching zero). Neither half is deferred out of the
  outcome — both are rows. The row's "or a refusal surfaced at `run`/`build`" alternative is
  kept as well, not instead: phase 4 also gives `Refusal::RepairKeysNotDiscoverable` a real
  `DiagnosticCode` (its catalogue row in `docs/specs/diagnostics.md` already exists with no
  variant behind it), so the fail-closed leg that survives the new route is loud at
  `build`/`run` rather than visible only through `smelt explain --json`.

- 2026-09-08 (phase 3 implementation): **the plan's `MAX`-per-column fold was insufficient;
  fixed to a `ROW_NUMBER()`-ranked whole-row pick instead.** Running the full 30-day
  `every_window_matches_the_full_refresh_oracle` gate exposed two bugs a per-column `MAX`
  aggregate over the model's compiled `SELECT` output cannot avoid: (1) `LEAD`/`LAG` computed
  over physically-duplicated tied rows produces genuinely different derived-column values per
  physical row (one row's `LEAD` self-references its tied sibling; `MAX` prefers that artifact
  over the correct `NULL`), and (2) the fold's `SELECT` list must preserve the model's own
  output column order (not force key-first), since the patch loop's bootstrap shell always
  uses model order and position-based `EXCEPT ALL` comparisons broke under a reordered fold.
  Both fixed; the full 30-day oracle (`crates/smelt-cli/tests/github_activity_oracle.rs`) and
  the day-by-day replay (`crates/smelt-cli/tests/github_activity_replay.rs`) now pass with
  `silver_repo_naming`/`silver_actor_naming` comparing exactly equal — see
  `phases/03-summary.md`.

- 2026-09-08 (phase 3 planning): **the harvest happened at plan time, and row 3 became
  the first real punch-list item.** Row 3 as scaffolded was a meta-phase whose entire
  content — "read the handoff and rewrite the remaining rows" — is exactly what the
  outcome loop's plan step does under its own reshape rule, so running it as an implement
  iteration would have burned a step producing only a table edit. The handoff
  (`docs/handoffs/2026-09-08-github-activity-findings.md`) is final for loop purposes: the
  spine's `**Status:**` is `blocked` and its live-BigQuery half will not land unattended,
  so waiting for a richer input is waiting for something no loop iteration can produce.
  Rows 3-6 are now its four punch-list items verbatim, in its order (item 4 explicitly
  after item 2 because the handoff asks whether item 2's mechanism subsumes it); old rows
  5-7 shift to 7-9. Nothing from the handoff was dropped: its two "requirements handed
  to" sections address the other two backlog outcomes, not this one, and its "latent,
  unmeasured" clock-tie item folds into row 3 where the same emitter is already open.

- 2026-09-08 (phase 2 implementation): **`supports_fingerprint_sidecar` stays
  DuckDB-only after phase 2.** Phase 2 proved the fingerprint/repair-group digest
  SQL well-formed per dialect (BigQuery, Spark), pinned by 8 new tests including
  a loud-refusal gate (`sidecar_capability_is_declared_only_where_the_digest_sql_
  is_verified` in `crates/smelt-runtime/tests/fingerprint_sidecar.rs`) that fails
  offline if the capability flag moves without a live value-leg sweep for that
  backend. No such sweep ran this phase — the flag is unchanged.

- 2026-09-08 (phase 1 implementation): **the fixed path is not reachable on a live
  `mutable_snapshot` run today.** `crates/smelt-dialect/src/dialect.rs` declares
  `supports_fingerprint_sidecar: true` only for DuckDB (line 218; `spark()`/
  `spark_delta()`/`spark_parquet()` and `bigquery()` all declare `false`, lines 260, 295,
  354 in the file as read for this phase). Every runtime entry point in
  `crates/smelt-runtime/src/maintenance_driver/sidecar.rs` —
  `diff_fingerprint_sidecar_changed_keys` (line 133),
  `refresh_fingerprint_sidecar` (line 242), `diff_repair_group_sidecar_changed_keys`
  (line 389), and `refresh_repair_group_sidecar` (line 493) — checks
  `backend.capabilities().supports_fingerprint_sidecar` and returns
  `BackendError::unsupported` before ever calling `emit_fingerprint_digest_select` or
  `emit_repair_group_digest_select`. So today a BigQuery (or Spark) target never reaches
  the previously-wrong DuckDB-hardcoded digest SQL at all — the bug was latent, not
  live-hit. It would become reachable the moment `bigquery()`'s (or a Spark variant's)
  `supports_fingerprint_sidecar` flips to `true`, which is presumably future work this
  outcome's punch-list (harvested in phase 3) or a follow-on outcome would drive. The fix
  still lands now, unconditionally, per criterion 1 and the outcome's framing — it removes
  a landmine ahead of that capability ever being turned on, rather than waiting for a
  spine model to trip it.

- 2026-09-08 (phase 1 planning): **reshape — a new row 2 for the remaining dialect-blind
  fingerprint SQL.** Reading the emitter for criterion 1 surfaced two siblings with the
  same defect class: `key_expr_for_columns` hardcodes `CAST(... AS VARCHAR)` (GoogleSQL
  has no `VARCHAR` at all) and `emit_repair_group_digest_select` hardcodes both that cast
  and DuckDB's `bit_xor(hash(...))`. Fixing only the digest expression would leave the
  same emitted statement invalid on BigQuery, so this serves criterion 1's substance and
  criterion 3 and is not deferred out. Old rows 2-6 shift to 3-7.

- 2026-09-08 (bigquery-dogfood-spine phase 15): **the interim findings handoff now
  exists** at `docs/handoffs/2026-09-08-github-activity-findings.md` — the four measured
  root causes, the five registered divergences, and this outcome's punch-list, all
  DuckDB-half only. Its live-BigQuery half lands in that outcome's phase 16; until then,
  this document is the phase-2 planner's rewrite input, not the final one.
- 2026-09-06 (scaffold): **deliberately near-empty.** Phase 3 is a placeholder the phase-2
  planner rewrites. This is the outcome loop's just-in-time planning used as intended, and
  it is the mechanism by which §"Sequencing"'s "let the real models generate the
  punch-list" is enforced rather than merely intended.
- 2026-09-06 (scaffold): **phase 1 runs before the spine finishes.** The fingerprint-dialect
  defect is wrong for any model on any backend, so it is not gated on evidence. It is
  ordered first so the loop has real work the moment this outcome is reached, even if the
  spine is still mid-flight.
- 2026-09-06 (scaffold): the BigQuery value leg cannot run in the loop's environment. A
  phase that needs it must block rather than skip — the same rule the active-plan pointer
  states for the Spark legs, for the same reason (a silently skipped live leg is a hole
  that reads as green).

## Blocked

(none)
