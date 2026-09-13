# Outcome: The incremental families execute on Trino and the equivalence invariant holds there

**Created:** 2026-09-13
**Status:** queued
**Driver:** loop. Docker only, no credential, no human gate. Live-tier phases must emit
`<<PHASE_BLOCKED>>` when the coordinator is unreachable, never skip green — a conformance gate
that skips looks exactly like a conformance gate that passes.
**Depends on:** `20260913-trino-target-spine` (T1) for the backend and tier;
`20260913-trino-ledger` (T3) for state residency and the degradation verdict, which decides
which techniques are even reachable on Trino; `20260913-trino-emission` (T2) for the expression
spelling the statements are built over.
**Source:** T4 of the five-outcome Trino programme agreed 2026-09-13, split from T3 at the seam
the maintenance-plan invariant draws: `smelt-logical` single-owns every maintenance statement a
run executes, with ledger bookkeeping in `smelt-state` explicitly excluded. T3 took the excluded
half; this outcome takes the owned half.
**Spec anchors:** `docs/specs/incremental_models.md` §"The equivalence invariant",
§"Statement emission (single owner)", §"The contract lattice", §"Upstream model edges";
`docs/specs/incremental_shapes.md`; `docs/specs/model_transforms.md`;
`docs/specs/architecture.md` §"Constraints & Invariants" item 12 (maintenance-plan purity);
`docs/specs/multi_backend.md` §"Whole-row MERGE", §"Column-scoped merge and conditional-write
capabilities", §"Incremental & schema evolution per backend"

## The outcome

An incremental model maintained on Trino computes the right answer, and that is proved
generatively rather than asserted. The equivalence invariant —
`incremental_state(S) == full_refresh(inputs ∈ S)` for every maintained model under **any** valid
run sequence — is checked on Trino by the same gate that checks it on DuckDB, Spark and
BigQuery: a deterministic-seeded sample of typed model recipes, staged and driven through the
real `execute_project` pipeline and compared to a full-refresh oracle after every run step.

Every statement those runs execute comes from a pure emitter in `smelt-logical`'s maintenance
layer. Trino executes; it never authors. The `statement_parity` gate proves both halves on a
fourth engine — per-family *executed-equals-emitted* parity, and the structural no-authoring leg
— so a Trino-shaped `MERGE` cannot be assembled in the backend crate where no one is looking.

Trino's shape makes two families interesting rather than routine. It has **no `INSERT
OVERWRITE`**, so the delete-and-insert window family runs over an emulated form whose write
window must exactly cover what it deletes — the `filter_range` correctness point recorded when
the old CLI incremental path was removed. And its `MERGE` support is Iceberg's, so which clauses
exist — `WHEN NOT MATCHED BY SOURCE` above all, which the whole-row and column-scoped merge
paths lean on — is measured against the live coordinator, and a family whose clause is absent is
refused by name or routed to an emulation that is proved equivalent, never approximated.

Where T3's verdict denies Trino a correctness structure, the affected cells arrive here already
downgraded by availability resolution. This outcome proves the **downgraded** route is equally
correct: a recompute-family technique satisfies the same equivalence invariant, so the sample
must pass under Trino's actual availability, not under a hypothetically fully-stateful one.

## Success criteria (checkable)

1. **`MERGE` on Iceberg is characterised by execution.** Which clauses Trino's Iceberg `MERGE`
   accepts — `WHEN MATCHED`, `WHEN NOT MATCHED`, `WHEN NOT MATCHED BY SOURCE`, a column-scoped
   `UPDATE SET`, a conditional `AND` guard — is established by running each form against the
   live tier, and `supports_merge`, `supports_column_scoped_merge` and
   `supports_merge_not_matched_by_source` in the §Surface matrix are confirmed or corrected in
   the same commit. The measured error text for every `✗` is quoted in the decision log.
2. **Every family either executes or refuses by name.** For each maintenance family the plan can
   assign — insert-only append, whole-row `MERGE` upsert, column-scoped merge, the merge-less
   conditional write over a staged relation, delete-and-insert window, per-group recompute, and
   the succession patch — the Trino route either executes correctly or is refused with a
   diagnostic naming the backend and the missing capability. No family silently emits SQL Trino
   rejects, and none silently produces a different answer.
3. **The emulated overwrite covers its write window exactly.** With no `INSERT OVERWRITE`, the
   delete-and-insert window emulation's `DELETE` covers precisely the range the subsequent
   insert writes — no wider (data loss) and no narrower (duplicates) — asserted directly, and
   re-asserted under an out-of-order and a repeated application of the same window.
4. **Statements are emitted, never authored.** `cargo test -p smelt-runtime --test
   statement_parity` gains its Trino leg in both halves: per-family executed-vs-emitted parity
   over a real `execute_project` run, and the structural leg proving `smelt-backend-trino`
   authors no maintenance statement of its own.
5. **The plan is not re-derived.** The maintenance plan for a Trino target is derived once by the
   pure functions in `smelt-logical` and consumed unchanged by diagnostics, rule application,
   runtime lowering and the graph layer. Adding Trino introduces no second derivation site, and
   no `SqlDialect::Trino` branch appears in a consumer that should be reading the plan.
6. **The equivalence invariant is proved generatively on Trino.** A Trino leg of
   `cargo test -p smelt-cli --test maintenance_conformance` runs the deterministic-seeded sample
   of typed recipes through the real pipeline against the live tier, asserting equality with the
   full-refresh oracle **after every run step** — not only at the end. The leg is gated in
   `compat.yml` like `maintenance-conformance-spark`, and its recipe pool is no narrower than the
   families criterion 2 admits (a gate is only as wide as its pool).
7. **The downgraded routes are proved, not assumed.** Where T3's verdict denies a structure, the
   sample runs under Trino's *actual* availability and passes, with each downgrade recorded on
   the cell. A cell that downgrades is still asserted equal to the oracle — the contract's claim
   that availability changes cost and never result, tested rather than restated.
8. **The contract lattice is honoured as a triple.** `frozen_horizon`, `deferral` and
   `retain_departed` on a Trino target either work — the conformance gate consuming the single
   pure oracle transform, runtime probes emitting from the same definition — or are refused by
   the existing rules. No lattice point is defined ad hoc for Trino, and
   `DeclaredContractRequiresState` fires where T3 left the frontier unrealisable.
9. **Schema evolution mid-stream.** A maintained model whose schema evolves between runs
   (nullable column added, type widened, column dropped) continues to satisfy the equivalence
   invariant on Trino, taking T3's measured migration route or an honest full refresh.
10. **Gates green.** `verify-phase.sh` passes; `execute_parity` still holds (CLI and UI consume
    one pipeline); no ratchet lowered; every new diagnostic has a fixture and a catalogue entry.

## Out of scope

- **Bookkeeping DDL/DML and the residency verdict** — `20260913-trino-ledger`'s. This outcome
  consumes that verdict and does not revisit it.
- **Expression and clause spelling** — `20260913-trino-emission`'s.
- **Real-pipeline, real-data parity against DuckDB** — `20260913-trino-dogfood`'s. The gate here
  is generative over synthetic recipes; the dogfood outcome is the one that runs
  `examples/github_activity/`.
- **New maintenance families, new techniques, or new contract-lattice points.** Trino gets the
  families that exist. If Iceberg offers something smelt has no family for (an Iceberg
  `MERGE`-on-read tuning, a branch/tag write, a snapshot-based delta read), it is recorded as a
  future extension, not built.
- **Native incremental-view maintenance.** `supports_native_ivm` stays `false`; Trino's
  materialized views refresh on an external schedule and are not maintenance smelt emits.
- **Performance of the maintenance statements.** Correctness is the subject; cost measurement
  belongs to the dogfood outcome, which runs real volumes.
- **Widening the recipe pool for the other three engines**, even if Trino work reveals a pool gap
  — recorded and handed on.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Characterise Iceberg `MERGE` by execution: each clause form run against the live tier, the three merge capability flags confirmed or corrected in the spec matrix, measured errors quoted | pending |
| 2 | Spec delta: `multi_backend.md` §"Whole-row MERGE" / §"Column-scoped merge and conditional-write capabilities" / §"Incremental & schema evolution per backend" stated for Trino, plus the refusal diagnostics any absent clause needs | pending |
| 3 | The append and whole-row-`MERGE` upsert families executing on Trino end-to-end through `execute_project`, with their `statement_parity` executed-vs-emitted legs | pending |
| 4 | The emulated delete-and-insert window: `DELETE` range exactly covering the insert's write window, asserted directly and under out-of-order and repeated application | pending |
| 5 | Column-scoped merge and the merge-less conditional write over T3's staged relation — or by-name refusals where phase 1 measured the clause absent | pending |
| 6 | Per-group recompute and the succession patch on Trino, including the routes T3's verdict downgrades, each downgrade recorded on the cell | pending |
| 7 | `statement_parity`'s structural no-authoring leg for `smelt-backend-trino`, and a check that adding Trino introduced no second plan-derivation site and no consumer-side dialect branch | pending |
| 8 | The generative gate: `maintenance_conformance` Trino leg over the live tier, equality with the full-refresh oracle after **every** run step, recipe pool no narrower than the admitted families, gated in `compat.yml` like the Spark leg | pending |
| 9 | Contract lattice on Trino: the three declared points working through the single oracle transform and probe emitter, or refused by existing rules, with `DeclaredContractRequiresState` where T3 left the frontier unrealisable | pending |
| 10 | Mid-stream schema evolution under maintenance, still oracle-equal; then close: divergences rewritten, `docs-site/` incremental-on-Trino notes, `verify-phase.sh` green | pending |

## Decision log

## Blocked
