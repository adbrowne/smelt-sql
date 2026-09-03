# Outcome: Close the key grain's implementation-only residues

**Created:** 2026-08-15
**Status:** active
**Source:** `docs/specs/incremental_shapes.md` §"The key grain" §Known Divergences;
`docs/outcomes/20260815-definition-delta-migrate/outcome.md` §"Out of scope"
**Spec anchors:** `docs/specs/incremental_shapes.md`, `docs/specs/incremental_models.md`

## The outcome

Five key-grain divergences close where the spec has already decided the target behaviour and only
the implementation is missing — no new product decision required. `KeyedRetractableContribution`
gets a real classifier, diagnostic, and test: §"Enrichment joins" and the Diagnostics table already
state exactly when it fires and what it steers toward (`refresh: materialized_view` or DAG
composition), nothing about its semantics is undecided. §"The transactional frontier write (merge
ledger)" already states "every window-forward keyed model maintains a per-model frontier" — not
"every additive-graded one" — so a re-run-tolerant (non-additive) model writing no ledger record
today is a plain conformance gap against that sentence, not an open question; the same section's
"backend-resident and transactional with the write it describes" already implies every backend
must fold the ledger transactionally, so the DuckDB-only override is the gap, not the target.
§"Derived execution postures" already defines order-independence formally ("holds iff every
combiner is order-independent") — the implementation just never computes or prints the verdict it
already specifies. And the generative conformance pool's non-nullable payload type is a test-harness
gap against a family (once-write) whose NULL-preservation obligation is already fully specified in
§"The column-family catalogue".

## Success criteria (checkable)

1. `KeyedRetractableContribution` has a real classifier, a fixture that produces it, and a test —
   matching exactly the semantics already stated in §"Enrichment joins" and the Diagnostics table
   (no new admission rule invented).
2. A re-run-tolerant (non-additive) window-forward model writes a frontier record, matching
   §"The transactional frontier write (merge ledger)"'s unqualified "every window-forward keyed
   model" statement; `--auto` staleness can consult it.
3. The reconciliation ledger's fold is transactional on every shipped backend (matching the
   already-stated "backend-resident and transactional with the write it describes" guarantee), not
   DuckDB-only.
4. Order-independence (and the other derived execution postures already defined in §"Derived
   execution postures") is computed as a real verdict, not assumed sequential by default, and
   `smelt explain` prints it alongside the run shape.
5. The generative conformance pool's row type carries a nullable payload; the once-write family's
   already-specified NULL-preservation obligation is proven by the generated pool, not one
   hand-written test case.
6. `/smelt:validate incremental_shapes` reports no drift for every bullet this outcome closes; all
   standing gates green.

## Out of scope

The following key-grain `(Open Question)` bullets are **not** in this outcome because the spec
itself has not decided the target behaviour — building any of them means choosing new admission
width or a new surface, which is a product call, not an implementation gap. They stay named in
`docs/outcomes/20260815-definition-delta-migrate` §"Out of scope" pending explicit sign-off:
snapshot-reconcile multi-unclocked-source admission, once-write nullability for a key-derived
*expression* (widens the catalogue's four fixed spellings), pattern functions as built-ins vs. a
shipped template, driver granularity below `day`/`week`, `--auto` staleness fidelity beyond
conservative v1, self-referential keyed models, and run-pinning alignment for `NOW()`/`CURRENT_*`
in keyed models (today a deliberate hard refusal, `KeyedForbidsNondeterministic` — relaxing it
changes stated behaviour, not just fills a gap).

Also out of scope, discovered by phase 1: `repair::admit_per_group_recompute` passes an empty
`JoinContext` and never projects a join's own `ON` columns, so per-group repair can never admit
for a source reached only through a JOIN. It is a real limitation, but it belongs to the repair
family's admission width (`docs/outcomes/20260809-repair-family`), not to any of this outcome's
six success criteria — criterion 1 is already met end-to-end without it.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | `KeyedRetractableContribution`: classifier, diagnostic, fixture, test | done |
| 2 | Ledger presence for re-run-tolerant models, matching the spec's unqualified "every window-forward model" statement | done |
| 3 | Transactional ledger fold on every shipped backend | blocked |
| 4 | Derive and print execution postures (order-independence) in `smelt explain` | done |
| 5 | Generative conformance pool: nullable payload, once-write NULL direction covered | done |
| 6 | Validate + close out: `/smelt:validate incremental_shapes` clean, standing gates green | pending |

## Decision log

- 2026-09-03 — Outcome activated. Phase 1 planned with no reshape: no prior phase summary exists
  in this outcome, and the six phase rows still match the success criteria one-for-one. Phase 1's
  derivation seam was fixed to the key-grain `NewData` handler's repair-refusal arm in
  `smelt-logical/src/maintenance/derive.rs` — the only site where both halves of
  `KeyedRetractableContribution`'s stated firing condition (a retractable enrichment-join
  contribution; a repair family that cannot admit a per-group recompute) are already computed, so
  no new admission rule is invented.
- 2026-09-03 — Phase 1 implemented and closed out (all green:
  `.claude/scripts/verify-phase.sh`, `repair_wiring`, `maintenance_diagnostics`,
  `statement_parity`, `technique_lowering`, `maintenance_conformance`, `join_shape` unit tests).
  Discovered (not fixed, out of scope for this phase): `repair::admit_per_group_recompute` always
  passes an empty `JoinContext` to affected-key discovery and never projects a join's own `ON`
  columns, so per-group repair can never admit for a source reached only through a JOIN — flagged
  for the next planner as a candidate follow-up phase.

- 2026-09-03 — Phase 2 planned. No reshape of the remaining rows: phase 1's summary surfaced one
  new limitation (empty `JoinContext` in per-group repair admission), which serves none of the
  six success criteria and is recorded under "## Out of scope" pointing at the repair-family
  outcome. Phase 2's design was fixed to generalising the existing
  `execute_conditional_write_and_record_observed_delta` backend seam into
  `execute_write_with_bookkeeping` (one transactional implementation) rather than adding a
  parallel ledger-write method, and to writing the bookkeeping record with **no** `state.mode`
  gate — `state.md` §"`state.mode` and what each posture provides" already places correctness
  structures in every posture.

- 2026-09-03 — Phase 2 implemented and closed out (all green: `verify-phase.sh`,
  `keyed_frontier_bookkeeping`, `statement_parity`, `execute_parity`, `maintenance_conformance`,
  `smelt-backend-duckdb`, `smelt-state`). Generalised
  `execute_conditional_write_and_record_observed_delta` into `Backend::execute_write_with_bookkeeping`
  as planned, and the `Grade::Idempotent` arm now writes an `ON CONFLICT DO NOTHING` merge-ledger
  record for every step (including the table-creating first one), keyed identically to the `Additive`
  arm. A pre-existing unit test (`sequences_create_then_merge_across_partitions_in_temporal_order`)
  needed its call-count assertions updated — an intentional consequence of this phase, not a
  regression. No new limitations discovered.

- 2026-09-03 — Phase 3 blocked at planning, not implemented. Criterion 3 as worded presumes a
  Spark/BigQuery ledger substrate, but `docs/research/20260816-open-questions-triage.md` item 12
  records an explicit user decision to defer exactly that ("yes - let's put this future
  extensions"), and `incremental_models.md` §Known Divergences cites that record as the reason the
  Spark-dialect ledger builder is *deliberately* deferred. The alternative reading — implement
  `state.md` §"The degradation contract" instead — is a real, unowned feature (availability
  resolution does not exist at all; `state.md` §Known Divergences says so). Either way the choice
  is a product call this outcome cannot make. Rows 4-6 are unaffected and stay workable; criterion
  3 will be unmet at row 6 unless this is resolved first. Full entry under "## Blocked".

- 2026-09-03 — Phase 4 planned. No reshape: phase 2's summary surfaced no new work, phase 3's
  blocked entry already records its own decision, and rows 4-6 still map one-for-one onto criteria
  4, 5 and 6. Phase 4's design was fixed to a single pure derivation in `smelt-logical`
  (`rules::cumulative::execution_postures` over `&[AggregatorColumn]`, beside `state_column_summary`)
  that the runtime's existing `ledger_grade` then *delegates* to — the re-run-tolerance verdict
  exists today only inside `smelt-runtime`, so deriving it there again would violate
  maintenance-plan purity. One spec clarification is decided here rather than deferred: §"Derived
  execution postures"'s qualifying enumeration omits the additive fold, but its own formal rule
  ("holds iff every combiner is order-independent") plus its admission of decomposed fold (whose
  state columns are additive) already decide that `+`/`XOR` are order-independent; the enumeration
  is a partial gloss and is made explicit. Criterion 4's "not assumed sequential by default" is read
  as the *verdict* being derived and printed — actually applying windows out of order stays an
  unused optimisation and is retained honestly in §Known Divergences.

- 2026-09-03 — Phase 4 implemented and closed out (all green: `verify-phase.sh`,
  `execution_postures`/`keyed_families` (46), `smelt-runtime --lib cumulative` (25),
  `explain_maintenance`/`explain_model`/`cli_docs_coverage`, `maintenance_conformance` (74),
  `smelt-lsp --test example_workspaces` (35)). `execution_postures` derived once in
  `smelt-logical`; `WindowedKeyedRule::ledger_grade` now delegates to it. `smelt explain` prints
  an `Execution postures:` block (text and `--json`). The tutorial doc-sync gate required one
  regeneration (`deduplication.md` picked up the new block) — expected, not a regression. No new
  limitations discovered.

- 2026-09-04 — Phase 5 planned. No reshape: phase 4's summary explicitly reports no new
  limitations and states rows 5-6 are unaffected; rows 5 and 6 still map onto criteria 5 and 6.
  Two scoping decisions fixed here. (a) The nullable payload lands as a TYPE change on `GenRow`
  (`val: Option<i64>`) threaded through every testkit and gate site, but `arb_payload_value()` —
  the general append-only pool's draw — keeps producing non-NULL values: criterion 5 names the
  once-write NULL direction, and drawing NULLs across every combiner family would change what the
  additive/idempotent/decomposed families are asserting, which is hardening beyond this outcome
  rather than work its criteria require. (b) The proof moves into a dedicated world-fact-preserving
  generator (`arb_once_write_null_schedule`) driven through the existing `STracker` oracle, and the
  hand-written case is retained as a pinned minimal witness with its now-false "GenRow::val is
  non-nullable" rationale rewritten.

- 2026-09-04 — Phase 5 implemented and closed out (all green: `verify-phase.sh`,
  `smelt-maintenance-testkit` (56), `smelt-cli --test maintenance_conformance` (75),
  `cargo check -p smelt-cli --tests` clean). `GenRow::val` is now `Option<i64>`, threaded through
  every construction/read site plus a new `arb_once_write_null_schedule` generator and four new
  tests. `incremental_shapes.md`'s "generative conformance pool cannot stage NULL payloads" bullet
  is deleted. Discovered (not fixed, out of scope for this phase): `cargo check -p smelt-cli --tests
  --features smelt-cli/spark` (and the BigQuery twin) fail to compile on a PRE-EXISTING, unrelated
  bug in `smelt-maintenance-testkit/src/families/gate_composed.rs`'s call to
  `run_windowed_keyed_maintenance` (arg-count/closure-type mismatch against the function's current
  signature) — confirmed pre-existing by re-running the same check against the base commit with
  this phase's changes stashed out (identical errors, none about `GenRow`/`Option`/`val`). Flagged
  for the next planner as a candidate short follow-up phase; it blocks that specific gated-twin
  compile check for any change until fixed.

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->

- 2026-09-03 — **Phase 3, "Transactional ledger fold on every shipped backend".**

  **Decision needed:** what "transactional on every shipped backend" means for the merge ledger,
  given that a recorded user decision already rules out the reading criterion 3 assumes.

  **Why the outcome cannot answer it.** The outcome frames criterion 3 as a pure conformance gap
  ("the DuckDB-only override is the gap, not the target"), inferred from
  `incremental_shapes.md` §"The transactional frontier write (merge ledger)"'s phrase
  "backend-resident and transactional with the write it describes". But
  `docs/research/20260816-open-questions-triage.md` item 12 asks this exact question ("A
  transactional ledger for Spark") and records the decision "yes - let's put this future
  extensions"; `incremental_models.md` §Known Divergences ("The ledger's warehouse substrate is
  DuckDB-only") cites that record and states the deferral is deliberate, with the recorded
  `MaintenanceStateDowngraded` downgrade — not a Spark builder — as the intended behaviour. A
  recorded user decision outranks an outcome's inference, so building the Spark/BigQuery ledger
  would knowingly contradict it.

  **Additional engineering fact.** Spark/Delta has no cross-statement transaction and no enforced
  `PRIMARY KEY`, so the additive grade's never-fold-twice guarantee (an `INSERT` that violates a
  primary key inside the same transaction as the write) has no faithful Spark realisation at all.
  Criterion 3's literal form is not merely deferred there; it is unachievable without inventing a
  different mechanism — itself a new product decision.

  **Candidate options (for a human):**
  1. **Amend criterion 3** to the already-decided target: on a ledger-less backend the cell takes
     the specified, recorded, explain-visible downgrade, and "transactional" binds only where the
     fold actually happens. Then phase 3 becomes "ledger-structure availability resolution":
     a `merge_ledger` availability bit threaded like the existing `BackendWriteCapabilities`
     precedent (`smelt-logical/src/maintenance/mod.rs`), a downgrade recorded as pure plan data,
     a warning-level `MaintenanceStateDowngraded` diagnostic, and removal of the driver's
     `Grade::Additive` `bail!` and `Grade::Idempotent` silent `tracing::warn` skip
     (`smelt-runtime/src/maintenance_driver.rs`). Note this is a sizeable slice of `state.md`
     §"The degradation contract", which today has no implementation and no owning outcome —
     it may deserve its own outcome rather than a row here.
  2. **Reverse triage item 12** and build the Spark (and BigQuery) ledger builders plus their
     folds, accepting that Spark's fold cannot be transactional and that the additive grade must
     therefore stay refused there — i.e. only the re-run-tolerant grade actually lands.
  3. **Drop criterion 3 from this outcome** and record it under "## Out of scope" pointing at
     triage item 12's future-extensions decision, leaving criteria 1, 2, 4, 5 as the outcome's
     content.

  **Minimum fix regardless of option:** phase 2 left a silent `tracing::warn` skip in the
  `Grade::Idempotent` arm on non-DuckDB backends. Whatever is decided, a silent skip of a
  correctness structure conflicts with `CLAUDE.md` §"Fail-loud discipline" and should become a
  recorded, visible fact.
