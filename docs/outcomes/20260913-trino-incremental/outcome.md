# Outcome: The incremental families reachable on Trino execute correctly, and the equivalence invariant holds there under full degradation

**Created:** 2026-09-13
**Status:** active
**Driver:** loop. Docker only, no credential, no human gate. Live-tier phases must emit
`<<PHASE_BLOCKED>>` when the coordinator is unreachable, never skip green — a conformance gate
that skips looks exactly like a conformance gate that passes.
**Depends on:** `20260913-trino-target-spine` (T1) for the backend and tier;
`20260913-trino-ledger` (T3), which declines every correctness structure on Trino for Spark's
reason and so fixes which techniques are reachable at all; `20260913-trino-emission` (T2) for the
expression spelling the statements are built over.
**Source:** T4 of the five-outcome Trino programme agreed 2026-09-13; rescoped the same day on the
ruling that **Trino's transaction support is equivalent to Spark's** and that some incremental
features being unsupported on Trino for now is acceptable. Split from T3 at the seam the
maintenance-plan invariant draws: `smelt-logical` single-owns every maintenance statement a run
executes, with ledger bookkeeping in `smelt-state` explicitly excluded. T3 took the excluded half;
this outcome takes the owned half. Pattern followed:
`crates/smelt-cli/tests/maintenance_conformance_spark/` — the existing precedent for proving the
equivalence invariant on a backend where every correctness structure is absent.
**Spec anchors:** `docs/specs/incremental_models.md` §"The equivalence invariant",
§"Statement emission (single owner)", §"The contract lattice", §"Upstream model edges";
`docs/specs/incremental_shapes.md`; `docs/specs/state.md` §"The degradation contract";
`docs/specs/architecture.md` §"Constraints & Invariants" item 12 (maintenance-plan purity);
`docs/specs/multi_backend.md` §"Whole-row MERGE", §"Column-scoped merge and conditional-write
capabilities", §"Incremental & schema evolution per backend"

## The outcome

An incremental model maintained on Trino computes the right answer, and that is proved
generatively rather than asserted. The equivalence invariant —
`incremental_state(S) == full_refresh(inputs ∈ S)` for every maintained model under **any** valid
run sequence — is checked on Trino by the same gate that checks it elsewhere: a
deterministic-seeded sample of typed model recipes, staged and driven through the real
`execute_project` pipeline and compared to a full-refresh oracle after every run step.

The interesting part is *which* techniques it is checked over. T3 declines every correctness
structure on Trino for Spark's reason — Iceberg gives per-table atomicity and no cross-table
transaction — so Trino arrives here **fully degraded**, exactly as Spark (Delta) does. The
families that execute are the ledger-free ones: insert-only append, the whole-row `MERGE` upsert
(idempotent by key, needing no never-fold-twice refusal), the column-scoped merge, the merge-less
conditional write over T3's staged relation, the delete-and-insert window, and per-group recompute.
The ledger-dependent ones take their specified downgrade or by-name refusal, and that is an
accepted landing state rather than a gap to close here.

This is where the degradation contract's central claim gets tested instead of restated:
availability resolution changes a cell's **cost, never its result**, because every recompute-family
technique satisfies the same equivalence invariant. So the sample must pass under Trino's *actual*
availability — not under a hypothetically fully-stateful one — and a downgraded cell is asserted
oracle-equal just as a ledger-backed cell is on DuckDB. `maintenance_conformance_spark` already
does this for a structure-less backend, so Trino's leg follows that precedent rather than DuckDB's.

Every statement those runs execute comes from a pure emitter in `smelt-logical`'s maintenance
layer. Trino executes; it never authors. `statement_parity` proves both halves on a fourth engine
— per-family executed-equals-emitted parity, and the structural no-authoring leg — so a
Trino-shaped `MERGE` cannot be assembled inside the backend crate where no one is looking.

Two of Trino's own properties make families interesting rather than routine. It has **no `INSERT
OVERWRITE`**, so the delete-and-insert window runs over an emulated form whose write window must
exactly cover what it deletes — the `filter_range` correctness point recorded when the old CLI
incremental path was removed. And its `MERGE` support is Iceberg's, so which clauses exist —
`WHEN NOT MATCHED BY SOURCE` above all — is measured against the live coordinator, and a family
whose clause is absent is refused by name or routed to an emulation proved equivalent, never
approximated.

## Success criteria (checkable)

1. **`MERGE` on Iceberg is characterised by execution.** Which clauses Trino's Iceberg `MERGE`
   accepts — `WHEN MATCHED`, `WHEN NOT MATCHED`, `WHEN NOT MATCHED BY SOURCE`, a column-scoped
   `UPDATE SET`, a conditional `AND` guard — is established by running each form against the live
   tier, and `supports_merge`, `supports_column_scoped_merge` and
   `supports_merge_not_matched_by_source` in the §Surface matrix are confirmed or corrected in the
   same commit. The measured error text for every `✗` is quoted in the decision log.
2. **The reachable families execute.** Insert-only append, the whole-row `MERGE` upsert, the
   column-scoped merge, the merge-less conditional write over T3's staged relation, the
   delete-and-insert window, and per-group recompute each run correctly on Trino end-to-end through
   `execute_project`.
3. **The unreachable families degrade or refuse, by name, and that is the landing state.** The
   additive keyed fold's never-fold-twice route, the succession patch's window-forward route, and
   the sidecar-dependent key-addressed per-group route take the downgrade T3's absence specifies —
   each recorded on the cell and explain-visible — or refuse with a diagnostic naming the backend
   and the missing capability. **No family silently emits SQL Trino rejects, and none silently
   produces a different answer.** Accepting these as unsupported for now is the 2026-09-13 ruling,
   not a discovered shortfall.
4. **The emulated overwrite covers its write window exactly.** With no `INSERT OVERWRITE`, the
   delete-and-insert window emulation's `DELETE` covers precisely the range the subsequent insert
   writes — no wider (data loss) and no narrower (duplicates) — asserted directly, and re-asserted
   under an out-of-order and a repeated application of the same window.
5. **Statements are emitted, never authored.** `cargo test -p smelt-runtime --test
   statement_parity` gains its Trino leg in both halves: per-family executed-vs-emitted parity over
   a real `execute_project` run, and the structural leg proving `smelt-backend-trino` authors no
   maintenance statement of its own.
6. **The plan is not re-derived.** The maintenance plan for a Trino target is derived once by the
   pure functions in `smelt-logical` and consumed unchanged by diagnostics, rule application,
   runtime lowering and the graph layer. Adding Trino introduces no second derivation site, and no
   `SqlDialect::Trino` branch appears in a consumer that should be reading the plan.
7. **The equivalence invariant is proved generatively on Trino, under full degradation.** A Trino
   leg of `cargo test -p smelt-cli --test maintenance_conformance` runs the deterministic-seeded
   sample of typed recipes through the real pipeline against the live tier, asserting equality with
   the full-refresh oracle **after every run step** — not only at the end — with every cell resolved
   under Trino's actual (structure-less) availability. Modelled on
   `maintenance_conformance_spark`, gated in `compat.yml` like `maintenance-conformance-spark`, and
   with a recipe pool no narrower than the families criterion 2 admits: a generative gate is only
   as wide as its pool.
8. **A downgraded cell is asserted oracle-equal, not exempted.** The contract's claim that
   availability changes cost and never result is tested: for every cell Trino downgrades, the
   sample compares it to the oracle rather than skipping it. Where the succession grain's full
   rebuild replaces the patch route, what it writes is checked to be row- and column-identical to
   the ledger-bearing rebuild's own presented arm, per §"The degradation contract".
9. **The contract lattice is honoured as a triple.** `frozen_horizon` and `retain_departed` on a
   Trino target work through the single pure oracle transform and probe emitter, or are refused by
   the existing rules; `contract.deferral` refuses with `DeclaredContractRequiresState` per T3. No
   lattice point is defined ad hoc for Trino, and the conformance gate consumes the oracle
   transform rather than encoding its own comparator.
10. **Schema evolution mid-stream.** A maintained model whose schema evolves between runs (nullable
    column added, type widened, column dropped) continues to satisfy the equivalence invariant on
    Trino, taking T3's measured migration route or an honest full refresh.
11. **Gates green.** `verify-phase.sh` passes; `execute_parity` still holds (CLI and UI consume one
    pipeline); no ratchet lowered; every new diagnostic has a fixture and a catalogue entry.

## Out of scope

- **Rebuilding a ledger-dependent technique on some other mechanism so Trino can have it.** The
  additive fold's never-fold-twice refusal, the window-forward succession patch, `deferral`'s
  measured lag and the fingerprint sidecar are unsupported on Trino, accepted per the 2026-09-13
  ruling. Each gets its downgrade or refusal; none gets a substitute implementation.
- **Bookkeeping DDL/DML and the residency posture** — `20260913-trino-ledger`'s. This outcome
  consumes that posture and does not revisit it.
- **Expression and clause spelling** — `20260913-trino-emission`'s.
- **Real-pipeline, real-data parity against DuckDB** — `20260913-trino-dogfood`'s. The gate here is
  generative over synthetic recipes; the dogfood outcome runs `examples/github_activity/`.
- **New maintenance families, new techniques, or new contract-lattice points.** Trino gets the
  families that exist. If Iceberg offers something smelt has no family for — a `MERGE`-on-read
  tuning, a branch/tag write, a snapshot-based delta read — it is recorded as a future extension,
  not built. (An Iceberg snapshot-based delta read is the most tempting of these, since it looks
  like a route back to the structures T3 declined; it is explicitly a research question, not a
  phase.)
- **Native incremental-view maintenance.** `supports_native_ivm` stays `false`; Trino's materialized
  views refresh on an external schedule and are not maintenance smelt emits.
- **Performance of the maintenance statements.** Correctness is the subject; cost measurement
  belongs to the dogfood outcome, which runs real volumes.
- **The same repair-family sidecar gap on BigQuery.** `realisable_state_structures(BigQuery)` omits
  `FingerprintSidecar` too, so phase 6c's fix makes every repair-admitted cell downgrade on an
  already-shipped backend. Correcting BigQuery's own reachability (building the sidecar there, or
  ratifying the downgrade as its landing state) belongs to `20260906-bigquery-correctness`, not to a
  Trino outcome; 6c records the behaviour change and escalates it.

- **Widening the recipe pool for the other three engines**, even if Trino work reveals a pool gap —
  recorded and handed on.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Characterise Iceberg `MERGE` by execution: each clause form run against the live tier, the three merge capability flags confirmed or corrected in the spec matrix, measured errors quoted | done |
| 2 | Spec delta: `multi_backend.md` §"Whole-row MERGE" / §"Column-scoped merge and conditional-write capabilities" / §"Incremental & schema evolution per backend" stated for Trino, including which families are reachable and which take T3's downgrade, plus the refusal diagnostics any absent clause needs | done |
| 3 | The append and whole-row-`MERGE` upsert families executing end-to-end through `execute_project` — including landing `maintenance_dialect` for `SqlDialect::Trino`, which returns `Err` today and blocks every family — with their `statement_parity` executed-vs-emitted legs | blocked |
| 3a | Real (non-dry-run) execution resolves each model's run window and every batch `TimeRange` in that model's OWN partition axis (gap 2), so an integer-axis model's injected predicates render bare rather than quoted | done |
| 3b | Typed ANSI partition literals (`DATE '…'` / `TIMESTAMP '…'`) from the single `partition_literal` owner, so a calendar-axis predicate type-checks on a strict engine (gap 1) | blocked |
| 3c | A `ColumnScopedMerge` cell downgraded to `PerGroupRecompute` for an `UpstreamMutation`-triggered (unclocked) cell resolves `key_scope: None` — the full-scan recompute the reachable row already promises — instead of demanding a `ScanClamp` that cannot exist (gap 3) | done |
| 3b2 | Gap 1, re-attempted under the 2026-09-15 column-type ruling: the referenced partition column's declared SQL type reaches the single literal renderer, so a calendar predicate renders typed against a DATE/TIMESTAMP column and bare-quoted against a declared-VARCHAR one; plus the `render_time_literal` symbolic-placeholder fix 3b found | done |
| 3d | Phase 3's deferred live legs, now unblocked: the append and whole-row-`MERGE` upsert families end-to-end through `execute_project` on Trino, plus `statement_parity`'s Trino executed-vs-emitted leg | blocked |
| 3e | Live-Trino test isolation: every live-tier test gets a guaranteed-unique schema/namespace (or one process-wide guard), and `trino_state_residency.rs`'s `TRINO_ENV_GUARD` lock scope is widened to cover `stage_residency_project`'s own `SMELT_TRINO_URL` read — so the conformance and family gates fail for real reasons only | done |
| 3f | Gap 4: every partition literal the windowed-keyed maintenance driver emits goes through 3b2's single literal-renderer owner, typed against the referenced column — the driving-source pushdown filter (`smelt-runtime/src/maintenance_driver/{driver.rs,cumulative.rs}`) and the target-scan slice bound (`TargetSlicePredicate::Range` in both keyed-fold emitters, plus `emit_recurrence_bound_probe`'s reuse of it) — the third and fourth emission sites of the class 3a/3b2 fixed | done |
| 3g | Gap 5: `Technique::KeyedFold` gets a **plan-time** availability resolution mirroring the repair family's `resolve_availability` — the idempotent grade downgrades to a reachable technique on a structure-less backend, the additive grade takes a named, explain-visible downgrade or refuses with a diagnostic naming the backend and the missing structure (criterion 3's never-fold-twice route). An execution-time `BackendError::unsupported` is not sufficient: criterion 3 requires the verdict on the cell and explain-visible | done |
| 3h | Phase 3d's deferred legs, re-attempted on 3f+3g: the whole-row `MERGE` upsert (keyed-fold) family end-to-end through `execute_project` on Trino, plus `statement_parity`'s Trino executed-vs-emitted leg (3d's reverted `RecordingBackend`/`emit_keyed_fold` byte-identity design redone) | done |
| 4 | The emulated delete-and-insert window: `DELETE` range exactly covering the insert's write window, asserted directly and under out-of-order and repeated application | done |
| 5 | The merge-less conditional write over T3's staged relation (the departed-row delete as a separate scoped `DELETE`, since `WHEN NOT MATCHED BY SOURCE` is absent), and the column-scoped merge — executing where its cell needs no merge ledger, taking T3's `MaintenanceStateDowngraded` route where it does, per Spark's precedent | done |
| 6 | The degraded routes proved live on Trino: per-group recompute, the succession grain's full rebuild in place of the patch route (presented table row- and column-identical to the ledger-bearing rebuild's presented arm), and the sidecar-less key-addressed downgrade — each recorded on the cell and explain-visible | blocked |
| 6b | The degraded families' emission residue: `statement_parity`'s Trino byte-identity leg for the additive keyed fold's downgrade route (its whole-target rebuild statements, proved in 3h only by result-equality — criterion 5's per-family parity covers the downgraded family too), plus succession's own partition-literal sites (its `driving_steps` call site in `execute/project/mod.rs` still passes `Undeclared`, 3f's untouched residue): measure whether the degraded succession route emits a literal against a typed column on Trino at all, then route it through 3b2's single renderer or land a census test recording it unreachable and why (note: `succession_window_predicate` deliberately emits UNTYPED literals for a measured GoogleSQL reason — `maintenance_sql_dialect_purity.rs` pins it) | done |
| 6c | The repair family's own sidecar requirement, the gap phase 6 measured: a repair-admitted `PerGroupRecompute` cell is ALWAYS over a `mutable_snapshot` source, whose affected-key discovery `repair::discovery_posture` routes unconditionally to `RepairDiscovery::SidecarDiff` — so the cell needs `StateStructure::FingerprintSidecar` independent of `key_scope`, but `required_state_structure` only asks for it when `key_scope: Some(...)`, and a clamp-bounded cell hard-refuses at execution instead of taking a recorded, explain-visible downgrade. Fix in `smelt-logical`'s single-owner availability module (require the sidecar whenever the cell is repair-admitted; `resolve_availability`'s replacement must also clear `scans`, or `has_repair_family_lowering` still dispatches the same resolver), then re-enable phase 6's parked `per_group_recompute_matches_full_refresh_on_trino` — criterion 2's per-group-recompute family and criterion 3's named downgrade both depend on it. Escalate the BigQuery reachability finding separately; do not absorb it | blocked |
| 6d | The live-Trino conditional-write parity failure 6b measured: `statement_parity::trino::staged_candidate_conditional_parity_on_trino` fails consistently (2 recorded statement groups instead of 1 — a first-run bootstrap `CREATE TABLE … AS` group appears on the second `execute_project` call, as if run 1's target table were invisible to run 2's factory). Untouched by 6b's diff; suspected Iceberg REST-catalog visibility/consistency between two separate `TrinoBackend` instances. Diagnose against the live tier and fix, or — if the cause is the test's own two-backend staging rather than a product defect — restructure the test and record the measured reason. Criterion 5 covers the merge-less conditional write's per-family parity, so this is not deferrable | blocked |
| 6e | Form B's interval-literal spelling gap, the second blocker 6c measured on `per_group_recompute_matches_full_refresh_on_trino`: no `INTERVAL` spelling works end to end on live Trino — `INTERVAL '3 days'` parses but Trino rejects (`Unknown type: interval`), `INTERVAL '3' DAY` fails to parse in `smelt-parser`, and `INTERVAL 3 DAY` parses but `smelt_logical::analysis::source_bounds::parse_quoted_interval` (the Form B bound classifier) recognises only the quoted-string spelling, so obligation 4 fails closed with `RepairSliceUnbounded` before the sidecar question is reached. Widen the classifier to the bare-numeric spelling `smelt-parser` already accepts and/or extend the `BETWEEN`-clause parse to `INTERVAL '<n>' <UNIT>`, then re-enable 6c's parked live test. Criterion 2's per-group-recompute family depends on it, so it stays in the outcome | blocked |
| 6f | Phase 6d's residue, so criterion 5's merge-less-conditional-write parity does not leave the outcome with 6d's blocked row: 6d proved `staged_candidate_conditional_parity_on_trino`'s fixture can NEVER exercise the staged-candidate write live on Trino (its `NewData` cell is necessarily also `KeyedFold`-eligible, and Trino permanently downgrades that to a whole-target rebuild that subsumes it). Pick one of 6d's three recorded options — prefer (c) retarget the test's assertions to the whole-target-rebuild route and move the staged-candidate emitter's Trino byte-identity proof to a differently-shaped model or a unit-level harness, unless (a) threading `technique_overrides` through `resolve_keyed_fold_state_downgrade`/`resolve_repair_state_downgrade` proves cheap — and land the measured reason. Also covers 6d's second finding: the `whole_target_rebuild_downgrade` vs `column_scoped_cell` dispatch (`execute/project/mod.rs` ~line 2222) has the same shape as the double-dispatch bug 6d fixed and is currently unverified either way — prove or fix it | pending |
| 7 | `statement_parity`'s structural no-authoring leg for `smelt-backend-trino`, plus a check that adding Trino introduced no second plan-derivation site and no consumer-side dialect branch | pending |
| 8 | The generative gate: `maintenance_conformance` Trino leg modelled on the Spark leg, over the live tier, oracle-equal after **every** run step with cells resolved under Trino's actual availability, recipe pool no narrower than the admitted families, gated in `compat.yml` | pending |
| 9 | Contract lattice on Trino: `frozen_horizon` and `retain_departed` through the single oracle transform and probe emitter or refused by existing rules, `deferral` refusing per T3, no ad hoc point | pending |
| 10 | Mid-stream schema evolution under maintenance, still oracle-equal; then close: divergences rewritten, `docs-site/` page stating plainly which incremental features Trino does and does not support and why, `verify-phase.sh` green | pending |

## Decision log
- **2026-09-15 — phase 6d implementation: the masked-`table_exists`-error hypothesis was measured and falsified; the real cause is a genuine double-dispatch defect, now fixed, but it exposed a second, structural blocker the fixture cannot get past.** A tight in-process probe (fresh `TrinoBackend` instances, 0–1000ms delays, `crates/smelt-runtime/tests/tmp_visibility_probe.rs`, not committed) showed `information_schema.tables` visibility on this tier is immediate and consistent every time — no REST-catalog visibility window exists. Instrumenting the real dispatch instead found: `whole_target_rebuild_downgrade` (`keyed_fold_state_downgrade.or(repair_state_downgrade)`) is `Some` on *every* run of this fixture's model on Trino (its `NewData`-trigger cell is `Technique::KeyedFold`, permanently downgraded because Trino has no reconciliation ledger) — that downgrade's drop+create fires unconditionally, regardless of `table_exists_before_run`, alongside the membership-recompute dispatch (gated only on `table_exists_before_run`). Run 1 only hits the downgrade; run 2 hits both, producing the observed 2 groups. Fixed generally: membership recompute now also requires `whole_target_rebuild_downgrade.is_none()` (a whole-target rebuild already recomputes everything the narrower membership recompute exists to patch). The masked-error class was still worth fixing independently (fail-loud discipline) — all 9 `table_exists(...).unwrap_or(false)` sites in `smelt-runtime` now propagate the backend's own error, with a unit test and a source-scan census. Verified no regression via the full `smelt-runtime` suite (live and offline) plus the DuckDB `technique_lowering` e2e suite. But the double-dispatch fix exposes a *further* structural fact: with it applied, run 2 executes exactly one group — the whole-target rebuild, not the staged-candidate conditional recompute the test asserts against — because this fixture's aggregate is, by the classifier, necessarily also `KeyedFold`-eligible, and Trino permanently downgrades that. This exact fixture cannot exercise the staged-candidate write live on Trino; see `phases/06d-summary.md` for the three options recorded for the next planner (thread `technique_overrides` through the downgrade resolvers, reshape the fixture, or retarget the test's assertions to the whole-target-rebuild route). Row stays `blocked`.

- **2026-09-15 — reshape at phase 6d planning: the Form B interval-literal gap gets its own row (6e).** Phase 6c fixed the sidecar requirement it was given and proved it five independent ways, but measured a *second*, distinct blocker on the same live test: obligation 4's bound classifier (`source_bounds::parse_quoted_interval`) only recognises the DuckDB-native quoted-string `INTERVAL` spelling, while the only spellings Trino executes either fail to parse or fail the classifier — so the repair-admitted per-group-recompute cell can never discharge its bounded-read obligation on Trino, and criterion 2's per-group-recompute family cannot be proved live. That is Success-criteria work, so it gets a row rather than leaving the outcome. Placed after 6d and before 7: it touches the parser/bound-derivation layer, which 6d's existence-check work does not, so the two are independent and 6d keeps its order. Row 6c stays `blocked` — its own scope is done; 6e carries what remains.

- **2026-09-15 — reshape at phase 6c planning: 6b's measured live failure gets a row (6d), and the BigQuery half of 6c's finding is escalated rather than absorbed.** `statement_parity::trino::staged_candidate_conditional_parity_on_trino` fails live and consistently against a tree 6b's diff never touched. It is the merge-less conditional write's own per-family parity leg, which criterion 5 names, so it cannot leave the outcome — it gets phase 6d rather than a hand-forward note. Separately, 6c's fix (`required_state_structure` requiring the fingerprint sidecar for every repair-admitted cell) changes BigQuery's landing state as well, since `realisable_state_structures(BigQuery)` omits the sidecar: every repair-admitted cell there stops hard-refusing at execution and starts taking a recorded downgrade. That is strictly more honest than the status quo, but *whether BigQuery should realise the sidecar at all* is that backend's question; 6c records the change and §Out of scope escalates it to `20260906-bigquery-correctness`.

- **2026-09-15 — phase 6c design call: the repair cell's sidecar requirement is derived from the cell's own shape, not from a new `PlanCell` field.** A repair-admitted cell is exactly `technique: PerGroupRecompute`, `key_scope: None`, a non-empty `scans`, no recorded downgrade — `derive_repair_cell` is its only producer, and `derive/model_edge.rs` (the only other production `PerGroupRecompute` producer) always carries a `key_scope`. This is the same structural discriminator `has_repair_family_lowering` already uses, so it adds no second vocabulary; a `PlanCell` field would instead touch 51 construction sites for a fact the existing shape already determines. The requirement's *unconditionality* is what needs proving, and it is provable purely: `faithful_fold`'s `partitioned_input` condition holds for `AppendOnly`, so the repair-narrowing branch can only fire for a non-append-only posture, and `ChangeFeed` is refused by name upstream — leaving `MutableSnapshot`, whose `discovery_posture` is unconditionally `SidecarDiff`.

- **2026-09-15 — phase 6b implementation: `succession_window_predicate` never reads `column_type`, so the `driving_steps` call site's `Undeclared` argument is inert residue by construction, not a live gap.** Measured via a source-scan census (no `partition_literal(`/`PartitionColumnType` in `src/maintenance_driver/succession/`) plus an equivalence test (`driving_steps(..., Date)` and `driving_steps(..., Undeclared)` yield identical steps and identical rendered predicates), and pinned offline/permanently with `succession_patch_always_downgrades_on_trino` in `smelt-logical`. The additive keyed fold's downgrade rebuild now routes through `emit_create_table_as`/`execute_statement_group` instead of `Backend::create_table_as` directly, closing criterion 5's last per-family parity gap (proved on both DuckDB and live Trino). Discovered in passing: `statement_parity::trino::staged_candidate_conditional_parity_on_trino` fails live and consistently (not flaky, confirmed in isolation) — unrelated to this phase's diff, escalated in `phases/06b-summary.md` rather than absorbed or silently left broken.

- **2026-09-15 — reshape at phase 6b planning: the repair family's sidecar requirement gets its own row (6c).** Phase 6 proved four of its five degraded-route legs live but measured a fifth failing for a reason outside its own plan: every repair-admitted `PerGroupRecompute` cell needs `StateStructure::FingerprintSidecar` regardless of `key_scope` (because repair narrowing only ever fires for a `mutable_snapshot` source, and that posture's discovery is unconditionally `SidecarDiff`), while `required_state_structure` only asks for it on the key-addressed route — so the cell is never downgraded and execution hard-refuses by name. That is criterion 2's per-group-recompute family and criterion 3's "downgrade or refuse, recorded and explain-visible" clause, so the work stays in the outcome and gets a row rather than a deferral. Placed after 6b, not before: 6b touches the keyed-fold and succession emission paths, which the repair-family fix does not, so the two are independent and the existing pending row keeps its order. The BigQuery half of the finding (`realisable_state_structures(BigQuery)` also omits the sidecar, so the identical refusal is very likely live in production on an already-shipped backend) is NOT adopted here — it is escalated as a standing gap outside this outcome, per §Out of scope's rule against rebuilding a ledger-dependent technique elsewhere.

- **2026-09-15 — reshape at phase 6 planning: phase 6's row split into 6 (live proofs) and 6b (emission residue).** The row bundled three live degraded-route proofs with two emission-side items of a different kind — a `statement_parity` byte-identity leg and succession's untyped partition-literal residue. Nothing leaves the outcome; 6b carries both, and gains the measured constraint this planning found: `succession_window_predicate` emits untyped string bounds *deliberately* (a typed `DATE '…'` broke `examples/github_activity`'s `silver.repo_naming` on BigQuery, pinned by `crates/smelt-runtime/tests/maintenance_sql_dialect_purity.rs::the_succession_window_predicate_uses_untyped_date_literals`), so "route it through 3b2's renderer" cannot be assumed — 6b must measure whether a Trino succession run reaches that predicate at all before changing it. The degraded route (`rebuild_succession_state`'s `state_downgraded` arm → `emit_succession_full_rebuild_ledgerless`) rebuilds from the whole source and emits no window predicate, so the site may well be unreachable on Trino; that is a census test, not an emitter change.

- **2026-09-15 — phase 5 implementation: both measured facts matched the plan's prediction exactly, and the fix generalised to one shared helper.** Measured against the live coordinator: `DELETE FROM t USING s WHERE …` refuses with `mismatched input 'USING'. Expecting: '.', '@', 'WHERE', <EOF>`; the correlated `WHERE EXISTS` form is accepted and computes correctly (`crates/smelt-backend-trino/tests/staged_group_live.rs`). A single `changed_row_delete` helper renders both forms and covers all four production `USING` sites uniformly — including `emit_diff_patch`'s update leg, whose predicate carries a `slice_predicate` clause independent of the staged relation, which stays correct wrapped inside the `EXISTS` subquery. `StagedRelation::derive_for_capabilities` replaced six hardcoded `SessionTemporary`/`atomic: true` call sites; `WindowedKeyedRule::write_group` grew a `capabilities` parameter on the trait itself; a new `SqlCompiler::capabilities()` accessor and `execute::targets::capabilities_for_target` cover the no-live-backend preview/dry-run paths. Both the membership-sensitive recompute (test 8) and the column-scoped-merge downgrade (test 9) matched a `--full-refresh` oracle on the first live run, with no follow-up gap discovered. `crates/smelt-cli/tests/trino_incremental_families.rs` crossed the 1500-line large-file cap once tests 8-9 landed — split into a `trino_incremental_families/` directory target (`main.rs` + three family submodules) rather than registering an exception; all 12 tests still pass unchanged.

- **2026-09-15 — phase 5 planning: the merge-less conditional write is blocked on two pre-existing defects, not on anything Trino-specific about the transform.** (a) Every production staged-relation derivation site — `maintenance_driver/membership/execute.rs` (×3), `delta_restriction/mod.rs`, `repair/execute.rs` (×2), `cumulative.rs` — hardcodes `StagedRelationResidence::SessionTemporary, atomic = true`, so T3's capability data (`staged_relation_residence = TargetSchema`, `staged_relation_group_is_atomic = false`) never reaches an emitter and Trino would be handed `CREATE TEMP TABLE` inside a transactional group. (b) Four emitters spell their delete leg `DELETE FROM t USING s`, which is not Trino grammar (`staged.rs` ×2, `recompute.rs`'s `emit_per_group_recompute` and `emit_diff_patch` update leg). Phase 5 fixes both as one class — the shared delete-leg helper covers phase 6's `emit_per_group_recompute` site too, so phase 6 keeps only its own live proof, not an emitter change. `required_state_structure` makes `ColumnScopedMerge` require the merge ledger **unconditionally**, so on Trino every plan-cell column-scoped merge downgrades; the "executing where its cell needs no merge ledger" half of the row is the ledger-free `Backend::merge_into` route already proved in 3h, and phase 5's column-scoped half is therefore the recorded, explain-visible downgrade plus oracle equality.

- **2026-09-15 — phase 4 implementation: `Backend::insert_overwrite`/`delete_and_insert_transactional` have no production caller either — the real `DeleteInsert` dispatch path is `execute_model_incremental_with_bookkeeping`'s own `IncrementalStrategy::DeleteInsert` arm (`crates/smelt-backend/src/lib.rs`), which builds the group via the dialect-agnostic `build_delete_insert_group` with a bare, unquoted `schema.table` name and never routes through `delete_and_insert_transactional` at all.** That method's only caller today is `smelt-maintenance-testkit`'s Link-C harness (a dev-dependency, not production) plus each backend's own live tests — the same "unreached by `execute_project`" situation `delete_partitions` is already in, just not by-name-refused. The implementation still follows the plan's BigQuery-precedent shape (`self.qualified_name`, catalog-qualified) since the method is public `Backend` API a future caller could reach, and it is consistent with BigQuery's/Spark's own overrides; but `statement_parity/trino.rs`'s new `delete_insert_parity_on_trino` test asserts the bare-name shape actually executed by the real `refresh: incremental`/`grain: partition` dispatch, not the qualified name this override would produce. Discovered because the test first asserted the qualified form and failed — the executed SQL used the bare form instead.
- **2026-09-15 — phase 4 test design: a fixture needing a between-run source mutation must use a declared external source, not a first-class inline model.** `stage_int_partition_project`'s `seed_events` (`materialization: table`, a static `VALUES` body) is rebuilt from scratch by smelt on every run — a raw `DELETE`/`INSERT` mutation applied directly against its live table between two `smelt run` invocations is silently undone by the next run's own `CREATE TABLE … AS`. Measured live: the mutation test initially failed with the mart still showing pre-mutation data despite the raw mutation SQL succeeding, traced to `seed_events` being rebuilt as part of the very run meant to observe the mutation. Fixed by staging `delete_insert_window_replaces_only_its_own_rows_on_trino` over a declared source (`models/sources/events.yml`, `mutation_profile: append_only`, seeded directly on the live tier) instead — smelt only ever reads a declared source, never rewrites it, so the mutation survives to the next run.
- **2026-09-15 — reshape at phase 4 planning: the additive keyed fold's *downgrade* statements join phase 6's row.** 3h's summary recorded that `statement_parity`'s Trino leg proves byte-identity only for the idempotent (`MIN`) `MERGE` shape; the additive (`SUM`) downgrade's whole-target rebuild (`emit_create_table_as`/`emit_full_refresh`) is proved live only by result-equality. Criterion 5 asks for per-family executed-vs-emitted parity, and a downgraded family is still a family, so this is work the Success criteria require — it gets a home rather than leaving the outcome. Phase 6 already owns the degraded routes, so the row is widened rather than a new phase added.
- **2026-09-15 — phase 4 scope call: `TrinoBackend::delete_partitions` stays refusing.** No runtime path calls `Backend::delete_partitions` (the only non-impl reference is a test double), so implementing it on Trino would add an unreached statement-authoring site in the backend crate for phase 7's structural leg to police. Its refusal message is rewritten to say it is unreached rather than to point at this outcome. `insert_overwrite`, which the capability matrix advertises as emulated, *is* implemented — by delegation to `delete_and_insert_transactional`, BigQuery's precedent.
- **2026-09-15 — 3h implementation: kept `RecordingBackendFactory`'s DuckDB construction path
  unchanged rather than routing it through `smelt_backends::create_backend`.** Generalizing
  `RecordingBackend`'s `inner` to `Box<dyn Backend>` only needs the type widened; the shared
  `create_backend` helper requires a `database` field on the `Target` even when a
  `database_override` is supplied, which several pre-existing `statement_parity` fixtures'
  `smelt.yml` omit (they rely entirely on the test's own `db_path`). Using it broke 6 DuckDB
  tests; reverted to the original direct `DuckDbBackend::new(&path, &schema)` construction, and
  gave Trino its own `TrinoRecordingBackendFactory` in `trino.rs` instead.
- **2026-09-15 — 3h implementation: `device_agg`'s fixture needs `maintenance.scan_bounds.
  per_source.events.allow_full_scan: true` even for the idempotent (`MIN`) leg.** Measured live:
  `MaintenanceScanUnbounded` refuses the build when the driving relation is a declared external
  source (not a plain first-class model with inline `timeseries:` frontmatter), regardless of
  fold grade. Matches the existing DuckDB fixture in `keyed_fold_state_downgrade_execution.rs`.
- **2026-09-15 — 3h implementation: `smelt-runtime`'s own `tests/common/mod.rs`, not a reuse of
  `smelt-cli`'s.** `trino_ci_wiring.rs::every_live_trino_test_schema_name_comes_from_the_shared_
  helper` scans each live-gated binary's own source for a private `fn trino_schema(`/`fn
  unique_schema(` definition; cargo's per-crate `tests/` compilation has no cross-crate module
  sharing, so the fix is a same-shaped sibling file (`crates/smelt-runtime/tests/common/mod.rs`),
  loaded into the `statement_parity` binary via `#[path = "../common/mod.rs"] mod common;` in
  `main.rs` (one directory below where the file lives) rather than an inline helper in `trino.rs`
  itself.

- **2026-09-15 — 3h planning: the `MERGE` family fixture must be an *idempotent* fold, and the
  additive grade's downgrade is proved live here rather than in row 6.** 3d's deferred test design
  named a `SUM` combiner; 3g then graded `SUM` `Additive`, which downgrades to the whole-target
  rebuild on a structure-less backend. A `SUM` fixture would therefore have proved the rebuild
  route while claiming to prove criterion 2's whole-row `MERGE` upsert — passing vacuously. So 3h
  reaches the family with a `MIN` combiner (the `events_deduped` shape) and adds an explicit
  not-downgraded assertion, and keeps the `SUM` fixture as the criterion-3/8 proof that a
  downgraded cell is asserted oracle-equal on the live tier — 3g proved that route on DuckDB only.
  Folded into 3h rather than given a row or pushed to row 6 (which names per-group recompute,
  succession and the key-addressed downgrade, not the keyed fold) because it reuses 3h's own
  fixture and staging helper. No row added, split or reordered.

- **2026-09-15 — 3g's design fixed at planning time: the keyed fold's requirement is
  grade-dependent, and the additive cell's degraded route is the whole-target rebuild.**
  The row offered "a named, explain-visible downgrade **or** a refusal". Planning read the
  driver and found the two grades already behave differently: the `Grade::Idempotent` arm
  *skips* its merge-ledger record where unrealisable (a repeat merge of the same window
  converges), while only `Grade::Additive` needs the never-fold-twice ledger. So the fix is
  not one verdict for `Technique::KeyedFold` but two — idempotent requires nothing and is not
  downgraded (which is what makes criterion 2's whole-row `MERGE` upsert reachable on Trino at
  all), additive downgrades to the recompute family and is executed by the run shape's own
  whole-target route, precisely the paragraph `state.md` §"The degradation contract" already
  writes for a downgrade-reached `PerGroupRecompute` cell with no `key_scope` and no
  `ScanClamp`. The downgrade route is preferred over the refusal because it reuses the
  existing rebuild path (`succession`'s `state_downgraded` flag is the precedent), keeps the
  cell's recorded verdict and the run's actual behaviour coherent, and leaves criterion 8's
  "a downgraded cell is asserted oracle-equal, not exempted" satisfiable. No substitute
  implementation is built: nothing new executes that did not exist before.
- **2026-09-15 — reshape at 3g planning: row 6 widened to name succession's partition
  literals.** 3f's summary left succession's own `driving_steps` call site at
  `PartitionColumnType::Undeclared` as "worth a follow-up census", out of 3f's scope. On Trino
  the succession cell is downgraded to the full-rebuild route, which row 6 owns, and a bare
  string bound against a typed column is the same failure 3a/3b2/3f fixed four times over — so
  it serves criteria 3 and 8 and is not deferred out. Folded into row 6 rather than given its
  own row, because whether that route emits a partition literal at all is unverified; row 6
  runs it live and will settle it. No row added, split or reordered.
- **2026-09-15 — 3g implementation: `AVG`/`STDDEV_*`/`VAR_*` graded `Additive`, not
  `Idempotent`.** The planned predicate (fold combiner → `is_additive_combiner`, `Sum`/`BitXor`
  only) graded a decomposed `AVG` fold idempotent, since `combiner_for_function(Avg)` returns
  `None` (it is not a direct monoid). That is wrong: `AVG`'s decomposed state is a
  Welford-style `(sum, count)` pair, which double-counts on re-merge exactly like a bare `SUM`
  (`analysis::discriminants::combiner_discriminants`'s `decomposable` flag names this family).
  Caught by two pre-existing `smelt-db` diagnostic fixtures (`device_avg`, an `AVG` model)
  failing after the naive predicate landed — added `is_additive_fold_function` (additive
  directly, or via `decomposable`) and re-graded. `examples/github_activity`'s `events_deduped`
  (`MIN`-only) legitimately lost its downgrade under the correct grading — updated that
  fixture's expected-diagnostics list rather than treating it as a regression.

- **2026-09-15 — phase 3f done.** All five identified sites of the class now render through
  `partition_literal`: the two `TargetSlicePredicate::Range` emitters, `emit_recurrence_bound_probe`,
  the driving-source pushdown filter (`cumulative.rs`), and `driving_steps`'s `TimeRange`. Kept the
  emitters `Result`-free (panic-on-unreachable-`Err` instead of cascading `Result` through
  `WindowedKeyedRule`) since the only inputs reaching them are provably day-aligned strings. Two
  existing byte-parity tests over a genuinely `DATE`-typed model column changed their expected
  spelling from bare-quoted to `DATE '…'` — an intended behavior change, not a regression.
- **2026-09-15 — reshape at phase 3f planning: row 3f widened from gap 4's driving-source
  pushdown alone to *every* partition literal the windowed-keyed driver emits.** Reading the
  driver showed the target-scan slice bound is a fourth site of the identical class:
  `TargetSlicePredicate::Range` raw-quotes `lower`/`upper` in `emit_keyed_fold`,
  `emit_keyed_fold_suppressed` and `emit_recurrence_bound_probe`, so a locality-admitted keyed
  model on a `DATE` column would fail on Trino the moment gap 4 stopped failing first. It serves
  criteria 2 and 3 and sits in the same driver behind the same single-owner renderer, so it is
  folded into 3f rather than deferred out or given its own row. No row added, split or reordered.
- **2026-09-15 — 3f's proofs are offline by construction.** Gap 5 (3g) refuses every
  `Technique::KeyedFold` cell at execution time on a structure-less backend, so no live Trino
  `MERGE` is reachable until 3g lands. 3f proves the literal spelling at the statement level and
  re-runs the existing live tier for no-regression only; the live keyed-fold leg stays 3h's.

- **2026-09-15 — phase 3e done.** Counter-based naming rule landed for both `trino_schema` and
  `unique_schema`; the anti-regression gate (`every_live_trino_test_schema_name_comes_from_the_shared_helper`)
  immediately caught and forced the fix of two pre-existing private-duplicate helpers
  (`backend_live.rs`, `staged_relation_lifecycle.rs`) that predated this phase. Making
  `trino_schema` non-deterministic per call surfaced a real latent bug in
  `materialization_parity.rs`, which assumed two independent same-label calls returned the same
  string — fixed by resolving the schema once and threading it through. Three consecutive green
  live-tier runs at default parallelism, 0 flakes.
- **2026-09-15 — reshape after 3d.** 3d's summary reports the whole-row `MERGE` upsert family
  blocked on two newly-measured gaps (gap 4, the windowed-keyed driver's driving-source pushdown
  literal typing; gap 5, `Technique::KeyedFold`'s missing plan-time downgrade). Both serve success
  criteria 2 and 3, so neither is deferred out: added rows **3f** (gap 4), **3g** (gap 5) and **3h**
  (re-attempt 3d's two deferred live legs on top of them), sequenced before row 4 because families
  4-6 and the generative gate 8 all build on the same driver and technique-resolution path. Row 3d
  stays `blocked` as a record of what it measured; 3h is its successor, not its reopening.
- **2026-09-15 — 3e's mechanism fixed at planning time.** The row offered "a guaranteed-unique
  schema/namespace **or** one process-wide guard". The racing tests are separate test *binaries*,
  i.e. separate processes, so an in-process mutex cannot serialise them: the phase takes the
  unique-naming route, and the guard work is narrowed to its real defect — `stage_residency_project`
  reading `SMELT_TRINO_URL` outside `TRINO_ENV_GUARD`.

- **2026-09-15 — phase 3d: landed the append family + the derived CI-wiring census; deferred the
  whole-row `MERGE` upsert family (gaps 4/5, see Blocked log below).** Full writeup in the Blocked
  entry; the one-line version: the append family's live leg
  (`append_family_matches_full_refresh_on_trino`) and `trino_ci_wiring.rs`'s directory-scan census
  are done, but the keyed-fold `MERGE` family and `statement_parity`'s Trino leg are unreachable
  today for reasons outside this phase's task list, so row 3d stays `blocked` rather than `done`.

- **2026-09-15 — reshape at phase 3d planning: no row added, split or reordered; 3d absorbs phase 3's
  unlanded Test 9 (CI wiring) and generalizes the live-gated census from a hardcoded list to a
  directory scan.** Phase 3's Tests 7-8 are 3d's stated scope already; its Test 9 (the
  `trino-integration` job runs the new binaries) was never landed, and the survey done while planning
  found the gap is wider than the two new binaries: `trino_incremental_families`,
  `trino_state_residency`, `trino_ddl_live`, `trino_lock_versioning`, `trino_posture_plan_invariance`
  and `trino_broken_foreign_keys` are all live-gated and none is run by the job, so each passes
  vacuously in CI by skipping. A hardcoded census in `trino_ci_wiring.rs` is what let that drift
  happen, so the fix is to derive the census rather than extend the list. This serves criteria 2, 5
  and 11 — a gate nothing runs proves nothing — and so does not leave the outcome. The live legs 3d
  adds also run serially (`--test-threads=1`) per 3b2's interim workaround, pending phase 3e.

- **2026-09-15 — phase 3b2 landed: `column_type` must be resolved through the SAME projection
  `apply_type_casts` uses, never through `resolved_model_schema` (the axis's own resolution
  path) — the two can genuinely disagree.** Live discovery during this phase: for
  `examples/web_analytics`'s `marts.daily_active_users_by_method` (a `GROUP BY` passthrough of
  an upstream model's `event_date`), `resolved_model_schema` infers `Date` while
  `SqlCompiler::apply_type_casts`'s own projection inference (over the *wrapped* source SQL
  `inject_source_filters` produces) infers `Varchar` for the identical column — a pre-existing
  divergence between two independent type-inference call paths, invisible before this phase
  because both renderings quoted identically. Resolving `column_type` via the axis's own
  `resolved_model_schema` path (as first attempted) reproduced this exact divergence as a live
  `Cannot compare values of type VARCHAR and type DATE` failure in
  `per_partition_equivalence::web_analytics_session_attribution_matches_full_rebuild`, because the
  physical column `apply_type_casts` actually creates is `VARCHAR` while the injected DELETE
  literal rendered `DATE '…'`-typed. Fixed by adding `SqlCompiler::resolve_partition_column_type`
  (`compile.rs`), which reuses `derive_projection_for` + `build_projection_type_context` — the
  exact machinery `apply_type_casts` itself calls — so `column_type` and the physical CAST always
  agree, regardless of which inference is "correct" in the abstract. The call site additionally
  runs the model's SQL through `inject_source_filters` with a placeholder range first (a *shape
  probe*): the wrapping `inject_source_filters` performs (a bounded source ref becomes a derived-
  table subquery) itself changes how the outer projection resolves a passed-through column's
  type, so probing on the model's bare, un-wrapped SQL was insufficient and had to be dropped —
  only the wrapped shape matches what `apply_type_casts` will actually see. The underlying
  `derive_projection`/`infer_expression_type` divergence itself (why a subquery-wrapped FROM loses
  the upstream column's real type) is not fixed here — it is a pre-existing `smelt-db` type-
  inference gap, out of this phase's scope, and worth its own investigation.
- **2026-09-15 — `IncrementalPlan::column_type` (populated by `resolve_partition_axes`) was
  removed rather than kept as a second, unused source of truth.** Once every consumer moved to
  `SqlCompiler::resolve_partition_column_type`, the field became dead code (flagged by
  `cargo build`'s own `dead_code` lint) — keeping an unread, divergence-prone field around would
  have invited a future regression back to the bug this phase just fixed. `ResolvedAxis` (axis
  resolution) is unchanged; only the column-type half moved.


- **2026-09-15 — reshape at phase 3b2 planning: a new row `3e` owns live-Trino test isolation.**
  Phase 3c's summary records that the live-tier suite is flaky under default parallelism — three
  runs, three *different* unrelated live-Trino files failing with schema-creation/namespace races,
  each passing in isolation — plus a real lock-scope bug in `trino_state_residency.rs` where
  `stage_residency_project` reads `SMELT_TRINO_URL` without holding `TRINO_ENV_GUARD`. This is not
  out of scope: criteria 7, 8 and 11 all rest on a live gate whose failures must be trustworthy,
  and a gate that fails randomly is indistinguishable from one that fails for cause. It therefore
  gets a row rather than a hand-off, placed after 3d (the first phase whose own live legs it
  protects) and before the generative gate of phase 8, which is the phase it matters most for.
  Phase 3b2's own live verification runs serially (`--test-threads=1`) as an interim workaround.

- **2026-09-15 — phase 3c landed: `has_repair_family_lowering` declines a downgrade-derived
  clamp-less `PerGroupRecompute` cell instead of refusing `MaintenanceRepairSliceMissing`.**
  Reachable only via a single unclocked `mutable_snapshot` source with `ANY_VALUE(...)` columns
  (no JOIN, no aggregate fold/repair-eligible combiner) — `SUM`/`MAX` trip a pre-execution
  diagnostic gate under snapshot-reconcile, and any JOIN-based enrichment's `GROUP BY` defeats
  skeleton-source-closure pruning. Full-workspace `cargo test` against the live Trino tier proved
  flaky under default parallelism on THREE unrelated live-Trino test files across two separate
  runs (schema-creation races) — none touched by this phase's diff, each passing cleanly in
  isolation; see `phases/03c-summary.md` "For the next planner" for the root cause and a
  candidate fix.
- **2026-09-15 — reshape at phase 3c planning: gap 1 gets a successor row (`3b2`) and a ruling —
  option (a), thread the referenced column's declared type to the literal renderer.** Phase 3b's
  block entry escalates three options and recommends a human ruling; this loop has no human gate,
  and gap 1 blocks criteria 2 and 7, so it may not sit blocked indefinitely and may not leave the
  outcome. Ruling: **(a)**. It is the only one of the three the block entry itself calls correct in
  both directions, and the type is already declared data (`columns:`/`type: VARCHAR`), not
  something new to infer. (b) — casting the column — was rejected because wrapping the partition
  column in a `CAST` defeats partition pruning on every engine (a silent whole-scan regression in
  exactly the predicate whose job is to bound the scan) and quietly changes malformed-string
  behaviour; (c) — Trino-only typed literals — was rejected because it contradicts the
  2026-09-14 single-spelling rationale *and* is still wrong for a declared-VARCHAR partition column
  on Trino itself, so it buys nothing the other two do not. Consequence for the 2026-09-14 ruling:
  its direction (typed ANSI literals, one spelling per dialect) stands for a DATE/TIMESTAMP-typed
  column and is *narrowed* — the renderer stays dialect-blind but becomes column-type-aware. Row
  3b stays `blocked` with its trace intact; `3b2` carries the work, placed after 3c (they are
  independent; 3c is ready now, 3b2 needs the plumbing) and before 3d, which needs both. 3b2 also
  carries 3b's independently-discovered `render_time_literal` symbolic-placeholder fix
  (`{{window_start}}`/`{{window_end}}` reaching `partition_literal` via
  `diagnostics::preview::placeholder_range`). A controller who disagrees may overrule before 3b2
  is implemented; nothing is built on the ruling yet.

- **2026-09-15 — phase 3c planning: a downgrade-derived `PerGroupRecompute` cell has no
  repair-family lowering, rather than a clamp-less one.** Of the two shapes for gap 3's fix —
  resolve the cell with `slice: None` and let the repair driver run an unclamped affected-key scan,
  or decline to resolve it as a repair cell at all — the second. A cell that reached
  `PerGroupRecompute` by availability downgrade never passed
  `repair::admit_per_group_recompute`'s obligations, so the repair family's lowering has no
  admission behind it: routing it there would next demand the fingerprint sidecar (a
  `mutable_snapshot` source's `RepairDiscoveryPosture::SidecarDiff`) that
  `required_state_structure` explicitly did *not* require of this cell — a second, run-time
  derivation of the requirement the maintenance-plan-purity rule forbids. Declining instead leaves
  the run shape's own whole-target route (the snapshot-reconcile whole-source keyed `MERGE`) to
  perform the full-scan recompute the `key_scope: None` reachable row already promises. The
  `MaintenanceRepairSliceMissing` bail survives unchanged for a genuinely repair-admitted cell
  missing its clamp, which remains an internal inconsistency. No row added, split or reordered for
  this decision.

- **2026-09-15 — phase 3b planning: the third calendar shape fails loud rather than falling back
  to an untyped string.** The 2026-09-14 ruling fixed the direction (typed ANSI literals on every
  dialect, one spelling, no dialect threading) but not what happens to a calendar-axis value that
  is neither date- nor timestamp-shaped. `partition_literal` returns `Err` for it, matching the
  integer arm's existing fail-closed discipline and CLAUDE.md's fail-loud rule, rather than
  reverting to `'…'` — a silent untyped fallback is exactly the shape that let gap 1 reach a live
  Trino run undetected. Cost: `emit_statements.rs`'s `"it's"` → `'it''s'` escaping assertion
  becomes an error-case assertion. No row added, split or reordered; no reshape was needed, since
  3a's summary reported the phase landing cleanly with 3b already scoped as the next blocker.

- **2026-09-14 — reshape at phase 4 planning: phase 3's three blocking gaps become four rows
  (3a/3b/3c/3d) ahead of phase 4, option (a) of the block report.** Phase 3's summary recommends
  fixing gaps 1-3 before any further live `execute_project` proof, because phases 4-6 (delete-and-
  insert window, merge-less conditional write, degraded routes) all drive calendar-partitioned or
  snapshot-reconcile-shaped models through exactly the paths that carry them. Planning phase 4
  first would re-discover all three. None of this work leaves the outcome: every one of the three
  gaps blocks criterion 2 (the reachable families execute end-to-end through `execute_project`) and
  criterion 7 (the generative gate runs through the real pipeline), so each gets a row rather than
  a hand-off. Row 3 stays `blocked` with its trace intact; 3d carries its two unachieved tests
  (the CLI family proof and `statement_parity`'s Trino leg) rather than re-opening row 3.
  Phases 4-10 are unchanged in content and order.

- **2026-09-14 — ruling for phase 3b (gap 1): typed ANSI literals everywhere, one spelling, no
  dialect threading.** Of the two options the phase-3 summary poses, `partition_literal`'s calendar
  axis renders `DATE '2026-01-01'` / `TIMESTAMP '…'` on *every* dialect rather than per-dialect
  rendering threaded through `Region`/`inject_time_filter`/`inject_source_filters`. Rationale: the
  ANSI typed literal is accepted by DuckDB, Spark, BigQuery and Trino alike, so one spelling
  type-checks everywhere and no consumer acquires a dialect branch (criterion 6's shape); the
  competing option would put dialect knowledge into four call sites that today are dialect-blind.
  The cost is the ~19 files pinning exact literal text — verified to be test-fixture and
  spec-freshness assertions over rendered SQL, not stored state: every production caller of
  `partition_literal` (`smelt-backend-{duckdb,spark,bigquery}/sql.rs`, `transformer.rs`,
  `emit/types.rs::Region`) renders it into a SQL predicate, and none persists it into the ledger,
  so no migration of recorded state is implied. If 3b's implementer finds a persisted-literal site
  this survey missed, that is an escalation, not an absorb.

- **2026-09-14 — reshape at phase 3 planning: no row changed; phase 3 absorbs the three
  consequences of landing `maintenance_dialect`.** Turning
  `maintenance_dialect(SqlDialect::Trino)` from `Err` into `Ok` is not local: it deletes
  `multi_backend.md` §Known Divergences' "No maintenance dialect on Trino" entry, it un-skips
  `contract_probes.rs`'s `frozen_horizon` late-arrival probe on Trino (the skip-with-warning route
  survives for any dialect that still has no maintenance dialect), and it re-points the tests that
  pin today's `Err` (`staged_relation_atomicity.rs`, `trino_contract_points.rs`,
  `trino_explain_downgrade.rs`, `trino_posture_plan_invariance.rs`). All of that is inside phase
  3's existing scope — landing the gate means landing what depends on it — so no row was added,
  split or reordered. *Proving* the `frozen_horizon` probe on Trino remains criterion 9 / phase 9;
  phase 3 only stops the spec from claiming it is skipped. Also settled while planning: the
  new-variant match arms across the emit layer are filled under a stated three-way discipline
  (executed-and-measured here / spelled with the later phase that proves it named / refused where
  the site returns a `Result`), so a family phase 3 does not execute cannot acquire an unproven
  Trino spelling without a written owner.

- **2026-09-14 — reshape at phase 2 planning: criterion 2's "column-scoped merge" split in two.**
  `smelt_logical::maintenance::availability::required_state_structure` maps `Technique::
  ColumnScopedMerge` (and `InPlaceUpdate`) to `StateStructure::MergeLedger` unconditionally, and
  `realisable_state_structures(SqlDialect::Trino)` is already `vec![]` — so a plan cell whose
  technique is `ColumnScopedMerge` **downgrades** on Trino exactly as it does on Spark (Delta),
  even though `supports_column_scoped_merge` measured `true`. The capability describes a statement
  shape Trino can execute; the cell's technique demands a correctness structure the dialect does
  not realise, and the two questions are asked in that order. Criterion 2's flat listing of "the
  column-scoped merge" as an executing family therefore reads as the capability-gated statement
  shape, not as a promise that every `ColumnScopedMerge` cell runs incrementally on Trino. No work
  leaves the outcome: phase 5's row now names both routes explicitly, and criterion 8 (a downgraded
  cell is asserted oracle-equal, never exempted) is what keeps the downgraded route honest. No row
  added, removed or reordered.

- **2026-09-14 — phase 1: Iceberg `MERGE` characterised by execution** (measured against
  `trinodb/trino:483` + `apache/iceberg-rest-fixture:1.10.1`, `crates/smelt-backend-trino/tests/merge_clause_forms.rs`):
  - `WHEN MATCHED THEN UPDATE SET *` (the whole-row shorthand) — **refused**:
    `mismatched input '*'. Expecting: <identifier>`. Every `SET` target must be a named column.
  - `WHEN NOT MATCHED THEN INSERT *` — **refused**: `mismatched input '*'. Expecting: '(', 'VALUES'`.
  - `WHEN NOT MATCHED THEN INSERT ROW` — **refused**: `mismatched input 'ROW'. Expecting: '(', 'VALUES'`.
    The explicit column-list form (`INSERT (n, lbl) VALUES (s.n, s.lbl)`) is accepted (already exercised
    by `probe_supports_merge`) and is the emitter's only route for the insert arm — phase 3 and phase 5's
    emitters must render `UPDATE SET` and `INSERT` column-by-column, never `SET *` / `INSERT *` / `INSERT ROW`.
  - `WHEN MATCHED AND <pred> THEN UPDATE ...` — accepted, and the guard correctly restricts which
    matched rows update (value leg confirmed: only the guarded row changed).
  - `WHEN MATCHED THEN DELETE` — accepted; the delete arm exists for the merge-less conditional-write
    and delete-and-insert routes.
  - Two ordered `WHEN MATCHED` arms — accepted, and confirmed **first-match-wins** (the first matching
    arm's `UPDATE` applied, not the second).
  - `USING (SELECT ... FROM <staged>) s` (subquery source over a staged relation, not a `VALUES` list)
    — accepted, matching the shape T3's staged relation presents.
  - `WHEN NOT MATCHED BY SOURCE THEN DELETE` — **refused**: `mismatched input 'BY'. Expecting: 'AND', 'THEN'`.
    Confirms the existing `supports_merge_not_matched_by_source: false` measurement
    (`crates/smelt-backend-trino/tests/capability_probes.rs::probe_supports_merge_not_matched_by_source`);
    no change to the spec matrix or `BackendCapabilities::trino_iceberg()` was needed — `supports_merge`
    and `supports_column_scoped_merge` (both `true`) also confirmed unchanged, since every accepted form
    above is a shape those flags already cover.

- **2026-09-14 — reshape at phase 1 planning.** Phase 3's row now names landing
  `maintenance_dialect` for `SqlDialect::Trino` explicitly. The T1 hand-forward measured it
  returning `Err` (`crates/smelt-backend/src/lib.rs`, guarded by
  `maintenance_dialect_is_ok_for_the_three_implemented_dialects_and_err_for_trino`), so *no*
  maintenance family runs on a `trino` target until it lands — it is the gate in front of
  criterion 2, not an incidental detail of the first family. No row added, split or removed; the
  work was already inside phase 3's scope and is now visible in its title.

- **2026-09-14 — hand-forward from `20260913-trino-target-spine` phase 11.** Measured, for this
  outcome to act on: `maintenance_dialect` returns `Err` for `SqlDialect::Trino` today, so no
  incremental/maintenance family runs on a `trino` target until this outcome lands one
  (`docs/specs/multi_backend.md` §Known Divergences) — a full refresh is the only route in the
  meantime. Capability cells measured `false` that bound the reachable family set:
  `supports_native_ivm`, `supports_merge_not_matched_by_source`, `supports_merge_schema_write`,
  `supports_insert_overwrite` (emulated, like DuckDB's and BigQuery's). Measured `true`, available
  to build on: `supports_create_or_replace_table`, `supports_merge`, `supports_column_scoped_merge`,
  `supports_staged_relation_group`.

- 2026-09-13 (scaffold, before phase 1): **some incremental features are accepted as unsupported on
  Trino for now.** Ruling by the programme owner, paired with the ruling that Trino's transaction
  support equals Spark's (recorded in `20260913-trino-ledger`'s decision log). Consequence for this
  outcome: Trino arrives **fully degraded** — T3 declines all five correctness structures — so the
  reachable family set is the ledger-free one, and the ledger-dependent routes take their specified
  downgrade or by-name refusal as a landing state rather than as work to finish here. This makes
  `maintenance_conformance_spark` the precedent to follow rather than the DuckDB leg, since Spark is
  already a structure-less backend proving the same invariant. It also raises the stakes on
  criterion 8: with most cells downgraded, a sample that *exempted* downgraded cells from the oracle
  comparison would be testing almost nothing.

- **2026-09-15 — phase 6e planning reshape: added row 6f so criterion 5's residue does not leave the outcome with 6d's blocked row.** 6d is blocked on a design choice its summary could not make unilaterally, but the per-family parity of the merge-less conditional write is a success criterion, and the rule is that work serving the criteria is never deferred out. 6f carries 6d's three recorded options forward with a preference stated (retarget the assertions to the route Trino actually takes), plus 6d's unverified `whole_target_rebuild_downgrade` vs `column_scoped_cell` dispatch check. No row removed, nothing moved to Out of scope.

## Blocked

- **2026-09-15 — phase 6e (row 6e): the phase's own work is DONE and fully verified live — blocked only by an unrelated, pre-existing gate failure in `verify-phase.sh`'s full workspace `cargo test`.** All of 6e's own acceptance targets pass: the parser now accepts `INTERVAL '<n>' <UNIT>` (new tests `interval_quoted_number_bare_unit_parses/_roundtrips`, `interval_quoted_string_alias_is_not_absorbed` in `smelt-parser/src/parser/tests.rs`); `source_bounds`/`monotonicity` share one owner (`parse_interval_literal_after_keyword` / `parse_interval_value_and_unit`) across all three spellings, fixing a real latent bug where `INTERVAL '1' HOUR` folded to 1 second instead of 3600 (new tests in `source_bounds.rs` and `monotonicity.rs`); `per_group_recompute_matches_full_refresh_on_trino` is un-parked (`#[test]`, no more `#[allow(dead_code)]`) and passes against a live `scripts/trino-up.sh` tier, along with all 17 other tests in `trino_incremental_families`. `cargo fmt --check`, the clippy gate (both feature sets), shellcheck, `example_diagnostics`, `smelt-parser`, `smelt-logical --lib` (1018 tests), `smelt-parser-compat --test duckdb_differential` and `--test external_corpus` (ledger shrunk by 2 now-passing entries the parser fix closed), and `smelt-db --test type_property_tests` are all green.

  The one red gate is `cargo test --quiet` at the full-workspace level: `smelt-runtime --test staged_relation_atomicity::every_production_derivation_site_reads_the_capability` fails, naming `crates/smelt-runtime/src/cumulative/tests.rs` as hardcoding a session-temporary staged-relation shape instead of deriving it from `BackendCapabilities`. This file is untouched by 6e (confirmed via `git status`/`git diff` — zero lines changed) and the failure predates this phase: `git log` shows the file was last touched by `0e1ab3e12 refactor(smelt-runtime): split cumulative.rs into mod.rs + tests.rs`, already on this branch's `HEAD` before phase 6e started, and unrelated to interval literals or Trino incremental families. Isolating the rest of the workspace test suite around it (every other test target) is green; this is the single failure. Per the implement-step contract ("pre-flight verification is red on something that is NOT this phase's own target: do not halt and do not force it"), 6e's own sound work is committed (`20f0f24f0`) and the row is marked `blocked` rather than `done`, so the required gate stays honest. Candidate options for whoever picks this up: (a) fix `cumulative/tests.rs` to derive its staged-relation shape from `BackendCapabilities` like the gate expects (likely a small, mechanical fix — the file was just split out of `cumulative.rs` and probably carried the hardcoded shape forward unexamined); (b) if the hardcoded shape is deliberate test-only scaffolding, get `staged_relation_atomicity`'s site-scan to exclude `tests.rs`/`tests/` modules the way the hardening-budget gate already excludes test-support crates, and land that scope narrowing with a reviewer sign-off note. Either fix (not phase 6e's own scope) unblocks re-verifying and flipping this row to `done` with no further 6e-specific work needed.

- **2026-09-15 — phase 6d (row 6d): fixed the double-dispatch defect the live failure actually had, but the fixture cannot pass as written.** `statement_parity::trino::staged_candidate_conditional_parity_on_trino` was measured to fail not for the planned masked-error reason (a probe ruled out any REST-catalog visibility lag on this tier) but because `whole_target_rebuild_downgrade` — Trino's permanent downgrade of this fixture's `KeyedFold` cell to an unconditional whole-target rebuild, since Trino has no reconciliation ledger — fired on every run alongside the membership-recompute dispatch, which is gated only on `table_exists_before_run`. Fixed: membership recompute now also requires `whole_target_rebuild_downgrade.is_none()` in `execute/project/mod.rs`, verified against the full `smelt-runtime` suite (live and offline) with no regression. But with the double-dispatch gone, run 2 now executes exactly one group — the whole-target rebuild, not the staged-candidate conditional recompute this test asserts against — because the fixture's aggregate is unavoidably `KeyedFold`-eligible under today's classifier, and that is permanently downgraded on Trino. Options recorded in `phases/06d-summary.md`: (a) thread `technique_overrides` through `resolve_keyed_fold_state_downgrade`/`resolve_repair_state_downgrade` (they currently ignore overrides entirely, unlike `resolve_live_membership_recompute_cell`) so a pin can suppress the competing `KeyedFold` cell; (b) reshape the fixture to a model whose `NewData` trigger isn't `KeyedFold`-eligible (may not be possible under today's classifier); (c) retarget this test's assertions to the whole-target-rebuild route and prove the staged-candidate emitter's Trino byte-identity elsewhere. Criterion 5 still needs this resolved — not deferrable.

- **2026-09-15 — phase 6c fixed the sidecar-requirement gap phase 6 measured, and re-verified it
  five independent ways, but test 2 (`per_group_recompute_matches_full_refresh_on_trino`) is STILL
  not achieved live — for a second, unrelated, newly-discovered reason.** The fix itself: `smelt-
  logical::maintenance::repair::is_repair_admitted` names the discriminator (`PerGroupRecompute`,
  `key_scope: None`, non-empty `scans`, no recorded downgrade); `required_state_structure`'s
  `PerGroupRecompute` arm now requires `FingerprintSidecar` for it; `resolve_availability` clears
  `scans` on a `PerGroupRecompute`-original downgrade so `has_repair_family_lowering` stays exact.
  Proved: (1) pure unit tests in `crates/smelt-logical/tests/maintenance_availability/{repair.rs,
  trino_invariants.rs}`; (2) the runtime seam
  (`availability_seam::repair_downgrade_routes_to_whole_target_rebuild` — the downgrade is reported
  AND `resolve_live_per_group_recompute_cell` returns `None` for the same cell, no double
  dispatch); (3) the equivalence invariant with **no live tier at all**
  (`repair_lowering.rs::repair_downgrade_matches_full_refresh_offline`, DuckDB +
  `warehouse_tables: none`); (4) `smelt explain --json` on a real project
  (`explain_maintenance/repair.rs::explain_shows_the_repair_sidecar_downgrade`); (5) `smelt explain
  --json` on a live-tier-targeted-but-offline-invoked `trino` project
  (`degraded_routes::per_group_recompute_cell_is_explain_visible_on_trino`, now asserting the
  downgrade rather than its absence).

  What remains blocked is narrower than phase 6's original finding: driving the SAME fixture
  through a live `smelt run --target trino` end to end. Measured against the live tier (Docker
  Trino 483 + Iceberg REST + MinIO, brought up via `scripts/trino-up.sh`): the fixture's Form B
  band (`WHERE order_date BETWEEN TIMESTAMP '2025-01-14' - INTERVAL '3 days' AND TIMESTAMP
  '2025-01-14'`) — needed to discharge the repair family's obligation 4 (bounded per-group read
  footprint) at all — parses fine in `smelt-parser` and is exactly what `smelt explain` proves
  offline, but Trino's live engine cannot execute it:
  `io.trino.spi.type.TypeNotFoundException: Unknown type: interval`. Three spellings were tried
  live, none works end to end: `INTERVAL '3 days'` (quoted, DuckDB-native) parses but does not
  execute on Trino; `INTERVAL '3' DAY` (quoted number, bare unit — ANSI/Trino-native) does not even
  parse in `smelt-parser` (`Expected AND_KW, found IDENT` inside the `BETWEEN` clause); `INTERVAL 3
  DAY` (bare number, bare unit — `smelt-parser` does support this form per `parser/expr.rs`'s own
  comment) parses, but `smelt_logical::analysis::source_bounds::parse_quoted_interval` — the Form B
  bound-derivation text-scan classifier obligation 4 depends on — recognises only the quoted-string
  spelling, so this spelling fails obligation 4 closed (`RepairSliceUnbounded`) before the sidecar
  question is ever reached. This is a pre-existing gap in Form B's own interval-literal handling
  (a DuckDB-native SQL spelling baked into a cross-backend derivation), not something phase 6c's
  sidecar fix could or should absorb — it only surfaced now because test 2 was dead code (never
  actually run against a live Trino tier) until this phase re-enabled it.

  `per_group_recompute_matches_full_refresh_on_trino` is kept in `degraded_routes.rs` as an
  `#[allow(dead_code)]` function again (phase 6's own precedent, itself following phase 3's), with
  an updated doc comment recording this second, distinct root cause. Candidate fix for a future
  phase: either widen `parse_quoted_interval`/the Form B classifier to also recognise the
  `INTERVAL <n> <UNIT>` bare-numeric spelling `smelt-parser` already accepts (letting Trino-portable
  SQL discharge obligation 4), or extend `smelt-parser`'s `BETWEEN`-clause handling to accept
  `INTERVAL '<n>' <UNIT>` so a Trino-executable literal at least parses — either unblocks this test
  without touching the sidecar fix this phase actually shipped.

- **2026-09-15 — phase 6 blocked on test 2 (`per_group_recompute_matches_full_refresh_on_trino`),
  after landing tests 1, 3, 4, 5, 6 live.** `degraded_routes.rs` proves the succession
  full-rebuild-in-place-of-patch route and the sidecar-less key-addressed downgrade live end to end,
  and proves the repair-family cell's *shape* (`PerGroupRecompute`, no downgrade, a bounded scan
  clamp) via `smelt explain --json` — but its live *execution* refuses.

  Measured against the live tier: `smelt: run failed at model 'repair_customer_max': Feature not
  supported by Trino: group-grain fingerprint-sidecar affected-key discovery for a mutable_snapshot
  repair source (P9)`. Root cause, read from `derive/new_data.rs` and `repair.rs`: a repair-admitted
  `PerGroupRecompute` cell is **always** over a `MutationProfile::MutableSnapshot` source (repair
  narrowing only ever fires for that posture — an append-only source's fold always succeeds, so no
  repair cell is ever admitted for it), and `repair::discovery_posture` routes that posture's
  affected-key discovery to `RepairDiscovery::SidecarDiff` **unconditionally**, never the plain
  clamped scan (`repair_lowering.rs::snapshot_source_discovery_uses_the_sidecar_diff` proves this at
  the unit level, offline, where it looks correct because DuckDB realises the sidecar). So a
  repair-family cell needs `StateStructure::FingerprintSidecar` **independent of `key_scope`** — but
  `required_state_structure`'s `PerGroupRecompute` arm only asks for the sidecar when `key_scope` is
  `Some(...)` (the key-addressed route); a clamp-bounded repair cell (`key_scope: None`) is treated
  as needing nothing, so `resolve_availability` never downgrades it and execution hard-refuses
  instead of taking a recorded, explain-visible downgrade.

  This single source of truth is very likely wrong for BigQuery too, in production, right now —
  `realisable_state_structures(SqlDialect::BigQuery)` does not list `FingerprintSidecar` either
  (`state_structure.rs`'s own doc comment names the sidecar as BigQuery's pending work), so any
  `mutable_snapshot`-sourced repair-family model on a live BigQuery target should hit the identical
  refusal today. Not verified against a live BigQuery tier here — that is itself worth a fast,
  independent check outside this outcome's scope, since it would be a standing production gap on an
  already-shipped backend, not a new-backend-onboarding finding.

  Candidate fix (not attempted — it edits the `smelt-logical` single-owner availability module several
  gates depend on, warranting its own reviewed phase): (1) `required_state_structure`'s
  `PerGroupRecompute` arm must require the sidecar whenever the cell is repair-admitted (`scans`
  non-empty, `key_scope: None`), not only when `key_scope: Some(...)`. (2) `resolve_availability`'s
  replacement for `PerGroupRecompute` must also clear `scans` when downgrading — `recompute_equivalent`
  maps `Corner::ColumnMerge` back to `PerGroupRecompute` itself, so a bare downgrade record with
  `scans` still populated leaves `has_repair_family_lowering` returning `true`, still dispatching the
  same sidecar-needing resolver. The target shape is exactly the "declined" cell
  `has_repair_family_lowering`'s own doc comment describes: `state_downgrade: Some`, `key_scope:
  None`, `scans: []`, routed to the whole-target rebuild the `key_scope: None ⇒ full-scan recompute`
  promise already covers (the same route `20260913-trino-incremental` phase 3c's gap-3 fix already
  proved live for the *declined-cell* shape — `snapshot_reconcile_keyed_model_runs_on_trino`).

  `per_group_recompute_matches_full_refresh_on_trino` is kept in
  `crates/smelt-cli/tests/trino_incremental_families/degraded_routes.rs` as an `#[allow(dead_code)]`
  function (phase 3's own precedent for a discovered, unowned gap) rather than a permanently-red
  `#[test]`; its fixture (`stage_repair_project`/`seed_trino_repair_orders`) is ready for whichever
  phase lands the fix above. Tests 1, 3, 4, 5, 6 are committed and green.

- **2026-09-14 — phase 3 blocked at its live `execute_project` proof (Tests 7-8), after landing
  `MaintenanceDialect::Trino` and every emitter it forced.** `MaintenanceDialect::Trino` exists,
  resolves for all four dialects, and every arm it forced is filled per the three-way discipline
  (`crates/smelt-logical/src/maintenance/emit/{merge,hash,bootstrap,fingerprint,partition_bucket,
  probes}.rs`, `succession/mod.rs`, `smelt-runtime`'s `sidecar.rs`). `TrinoBackend::insert_into_
  from_query` is implemented. The append and whole-row-MERGE-upsert families are proved live at the
  `Backend`-trait level (`crates/smelt-backend-trino/tests/backend_live.rs`:
  `insert_into_from_query_appends_and_leaves_prior_rows_intact`,
  `delete_and_insert_transactional_covers_two_disjoint_windows`,
  `merge_into_upserts_matched_and_unmatched_rows_across_two_runs` — all three pass against the live
  tier). A real bug in the Trino `sha256` spelling was found and fixed live along the way
  (`hash_digest_expr`'s Trino arm needed hex-encoding to keep the digest-of-digests composition
  well-typed).

  What blocks Tests 7 and 8 (a real `execute_project` CLI run, and `statement_parity`'s Trino leg)
  is **three separate, pre-existing, cross-cutting bugs, none owned by this phase and none
  Trino-specific in mechanism** — Trino's stricter typing is simply what surfaced them; every other
  backend's looser implicit coercion has been masking them:
  1. **Calendar-literal type coercion** — `partition_literal`'s calendar axis and `transformer.rs`'s
     injected scan-window predicate both render a bare quoted string compared against a `DATE`/
     `TIMESTAMP` column; DuckDB/Spark/BigQuery coerce it implicitly, Trino refuses
     (`Cannot apply operator: date <= varchar(10)`, measured live).
  2. **A bare-integer run window's axis is only re-resolved on the `--dry-run` path.** The real
     execution path's `(None, None)` dispatch arm in `execute/project/mod.rs` hardcodes a calendar
     axis rather than calling `parse_run_window_in_axis` the way `--dry-run`/`build_model_plans`
     does, so an integer-axis model's real run injects a quoted string against an `INTEGER` column
     (`Cannot apply operator: integer <= varchar(1)`, measured live). DuckDB tolerates it; Trino
     does not.
  3. **A snapshot-reconcile-shaped keyed model's `ColumnScopedMerge`-downgrade route assumes a
     `ScanClamp` that cannot exist for an `UpstreamMutation`-triggered (unclocked) cell**, refusing
     with `MaintenanceRepairSliceMissing` instead of falling back to the full-scan recompute the
     `key_scope: None` "reachable" row already promises. `20260913-trino-ledger`'s Spark twin
     realises the identical fully-degraded posture, so this is very likely reachable on Spark too.

- **2026-09-15 — phase 3d blocked on the whole-row `MERGE` upsert (keyed-fold) family and
  `statement_parity`'s Trino leg; the append family and the CI-wiring census landed.** Gaps 1-3
  above were closed by 3a/3b2/3c, so this phase attempted the two live legs phase 3 deferred. The
  **append family** (insert-only `grain: partition`, over two disjoint windowed runs, oracle-compared
  to a `--full-refresh` rebuild) now runs end-to-end
  (`append_family_matches_full_refresh_on_trino`, `crates/smelt-cli/tests/
  trino_incremental_families.rs`) — landed. The **whole-row `MERGE` upsert (keyed-fold) family**
  does not run, live-measured against a real coordinator (`scripts/trino-up.sh`), for two further
  gaps neither closed by 3a/3b2/3c nor owned by this phase's task list (pure test-infrastructure
  scope — no fix task for either):
  - **Gap 4** — the windowed-keyed-maintenance driver's own per-step driving-source pushdown filter
    (`crates/smelt-runtime/src/maintenance_driver/{driver.rs,cumulative.rs}`, stepping over the
    driving source's own timeseries axis) renders the run window's bound as a bare string against
    the driving source's own partition column regardless of its real type — the same class of bug
    as gaps 1/2, in a third emission site neither of those phases' fixes touched (they fixed
    `transformer.rs`'s injected filters and the mart's own declared `partition_column` type
    resolution, not the driving-source stepping loop inside the windowed-keyed driver itself).
    Measured live: an `INTEGER`-axis driving source fails `Cannot apply operator: bigint <=
    varchar(10)`; a `DATE`-axis one fails `Cannot apply operator: date <= varchar(10)`. Blocks the
    idempotent (`MIN`/`MAX`-style) keyed-fold shape before a write mechanism is ever chosen.
  - **Gap 5** — `Technique::KeyedFold`'s `Grade::Additive` branch (a fold-eligible combiner such as
    `SUM`) has no plan-time downgrade, unlike the repair family's `resolve_availability`: the
    windowed-keyed driver checks `realises_reconciliation_ledger` at EXECUTION time and hard-refuses
    (`BackendError::unsupported("additive-fold windowed-keyed maintenance ledger
    (never-fold-twice)")`). Since `realisable_state_structures` is `vec![]` for both
    `SqlDialect::SparkSQL` and `SqlDialect::Trino` (the fully-degraded ruling recorded in the
    outcome-backlog's queued-next header), this refusal is unconditional on Trino today — measured
    live, independent of gap 4 (reached even on a `DATE` axis with no literal-typing issue at all).
    Worse: `required_state_structure` maps **every** `Technique::KeyedFold` cell (both grades) to
    `StateStructure::ReconciliationLedger`, so even the idempotent shape — if gap 4 is fixed first —
    would still need its OWN downgrade path added to the windowed-keyed driver (mirroring
    `resolve_availability`) before a real `MERGE` could ever be reached on a fully-degraded backend.
  Given neither gap has a fix task in this phase's scope, and the row's own acceptance criteria name
  the whole-row `MERGE` family explicitly, this phase does not close row 3d. What it verified is
  useful independent of the fix: the append family's live leg, and the CI-wiring gates (tests 4/5 in
  `trino_ci_wiring.rs`) now derive their live-gated census from a directory scan
  (`crates/smelt-{cli,backend-trino}/tests`, matching on real call-site markers —
  `trino_env(`/`targets_to_run_with_trino(`/`live_env_or_skip(`/a raw `SMELT_TRINO_URL` lookup, not
  a filename convention) rather than a hardcoded list, and the `trino-integration` job's smelt-cli
  step now runs `trino_ddl_live`, `trino_state_residency`, `trino_lock_versioning`,
  `trino_incremental_families` and `materialization_parity` alongside the two it already ran — all
  five confirmed passing live. Note for the next planner: the 2026-09-15 decision log entry above
  this one names `trino_posture_plan_invariance` and `trino_broken_foreign_keys` as additional
  live-gated files the phase-planning survey found missing from CI — the derived, marker-based
  census in this phase found BOTH to be fully offline (`smelt explain`/`Config::load` against a
  placeholder target, no live connection at all, already running in the ordinary `cargo test -p
  smelt-cli` suite with no Docker tier needed) — that survey's filename-based method over-counted;
  no CI change was needed for them.

  **Candidate next steps for gap 4 + gap 5** (not decided here — a planner call): (a) fix gap 4 by
  routing the windowed-keyed driver's driving-source pushdown through the same
  `resolve_partition_column_type`/typed-literal path 3b2 landed, then separately decide gap 5 —
  either add a plan-time downgrade for `Technique::KeyedFold` mirroring `resolve_availability` (so a
  fully-degraded backend falls back to `PerGroupRecompute` the way the repair family already does),
  or accept the keyed-fold MERGE family as a **declared, permanent** Trino gap (parallel to T3/T4's
  fully-degraded ruling) and rescope row 3d/the outcome's remaining criteria accordingly. (b) If (a)
  is taken, re-attempt this phase's deferred tests 2/3 verbatim — `trino.rs`'s `RecordingBackend`
  generalization and `smelt-backend-trino` dev-dependency were drafted and verified compiling during
  this phase's investigation, then reverted (unused code, since the family the test needs is
  unreachable today) — both are cheap to redo once gap 4/5 land.

  Each is documented with the exact live error text and root-cause trace in
  `crates/smelt-cli/tests/trino_incremental_families.rs`'s doc comments (now a documentation-only
  file naming the three gaps rather than a live CLI test, since the CLI dispatch path is what hits
  all three). None is a small fix folded into this phase: gap 1 needs a design decision (global
  ANSI-literal reformat vs. threading dialect-awareness through `partition_literal`/`Region`/
  `inject_time_filter`/`inject_source_filters`, weighed against ~19 files pinning exact literal
  text); gap 2 is mechanical but touches the shared run-window dispatch every family's real
  execution goes through; gap 3 is a `smelt_logical::maintenance::choice` cell-derivation fix.
  Candidate options for the next planner: (a) spin up one or more dedicated fix phases for gaps 1-3
  before resuming phase 3's remaining CLI-level tests (recommended — phases 4-6 will very likely
  hit the same gaps for any calendar-partitioned or snapshot-reconcile-shaped model), or (b) accept
  the `backend_live.rs` statement-level proofs as sufficient for phase 3's own scope and rescope
  Tests 7-8 into whichever fix phase lands gap 1/2/3. Full trace: `phases/03-summary.md`.

- **2026-09-15 — phase 3b blocked before its live-tier tests: the 2026-09-14 global-ANSI-literal
  ruling is unsound for a declared-VARCHAR calendar partition column, discovered via
  `cargo test --workspace` on the implementation, not via design review.** The plan's Task 5
  ("run `cargo test --workspace` and repair every fixture pinning the old spelling") was followed
  literally, and it surfaced more than fixture text: `crates/smelt-runtime/tests/statement_parity/
  staged_candidate_conditional.rs::events_deduped_composed_suppression_storm_rerun_writes_zero_rows`
  fails against a REAL DuckDB connection — not a pinned-text assertion — with `Binder Error: Cannot
  compare values of type VARCHAR and type DATE - an explicit cast is required`. The fixture is
  copied verbatim from `examples/web_analytics/models/sources/raw/events.yml`, which declares its
  `partition_column` (`event_date`) as `type: VARCHAR` on purpose — comment: "hive partition value;
  emitted as Utf8 by the partitioned writer (DuckDB casts to DATE on use)". This is the flagship
  web-analytics example's real raw/bronze layer, not a test artifact: a Hive-style partitioned
  writer commonly emits the partition value as a string column, with casting left to consumers.

  The 2026-09-14 ruling's premise — "the ANSI typed literal … is accepted unchanged by DuckDB,
  Spark, BigQuery and Trino alike, so one spelling serves every dialect" — is true only when the
  column being compared is *itself* DATE/TIMESTAMP-typed. Gap 1's original symptom (`date <=
  varchar(10)`, Trino refusing a bare string against a typed column) and this new symptom (`VARCHAR
  <> DATE`, DuckDB refusing a typed literal against a string column) are the SAME defect — a
  literal/column type mismatch — surfacing in opposite directions. Forcing the literal to always be
  typed trades one direction of the bug for the other; it does not close it. No spelling of the
  literal alone (independent of the actual column's SQL type) can satisfy both a strict DATE column
  and a legitimately-VARCHAR one.

  This is a design question the plan does not answer and phase 3b's own scope discipline ("this
  phase changes literal rendering only… does not touch axis resolution") rules out resolving
  unilaterally: `partition_literal`/`Region`/`inject_source_filters`/`inject_time_filter` are
  deliberately schema-blind (single-owner rendering, no dialect or column-type threading per the
  architecture invariants), and today have no channel to learn a referenced column's actual SQL
  type. Closing this soundly needs one of:
  (a) thread the source's declared column type (already present in `columns:` — see `type: VARCHAR`
  above) through to the literal renderer so it can choose bare-vs-typed per predicate, the largest
  change but the only one that is correct for both directions;
  (b) cast the *column* instead of typing the *literal* — e.g. `CAST(event_date AS DATE) >= DATE
  '…'` — which fixes both directions without threading type info to the renderer, but changes the
  predicate shape everywhere and needs its own soundness check (a malformed string in an untyped
  column would now fail the cast rather than compare lexicographically, which may be desired
  fail-loud behavior or may be a behavior change worth flagging separately);
  (c) narrow gap 1's fix to only the Trino target (a dialect-keyed emission choice after all,
  contradicting the 2026-09-14 ruling's "no dialect threading" rationale, but avoiding any change to
  DuckDB/Spark/BigQuery's already-working bare-quoted rendering for non-Trino targets).
  No code from this attempt survives — `partition_literal`, its callers, and all touched fixtures
  were reverted to a clean HEAD before this entry was written; the tree is clean. Recommended next
  step: escalate options (a)-(c) for a ruling before re-attempting 3b; option (b) is the least
  invasive if a controller confirms the cast's fail-loud behavior on malformed strings is
  acceptable. Full trace: `phases/03b-summary.md`.
