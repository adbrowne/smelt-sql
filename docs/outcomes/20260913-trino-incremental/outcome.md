# Outcome: The incremental families reachable on Trino execute correctly, and the equivalence invariant holds there under full degradation

**Created:** 2026-09-13
**Status:** queued
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
- **Widening the recipe pool for the other three engines**, even if Trino work reveals a pool gap —
  recorded and handed on.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Characterise Iceberg `MERGE` by execution: each clause form run against the live tier, the three merge capability flags confirmed or corrected in the spec matrix, measured errors quoted | pending |
| 2 | Spec delta: `multi_backend.md` §"Whole-row MERGE" / §"Column-scoped merge and conditional-write capabilities" / §"Incremental & schema evolution per backend" stated for Trino, including which families are reachable and which take T3's downgrade, plus the refusal diagnostics any absent clause needs | pending |
| 3 | The append and whole-row-`MERGE` upsert families executing end-to-end through `execute_project`, with their `statement_parity` executed-vs-emitted legs | pending |
| 4 | The emulated delete-and-insert window: `DELETE` range exactly covering the insert's write window, asserted directly and under out-of-order and repeated application | pending |
| 5 | Column-scoped merge and the merge-less conditional write over T3's staged relation — or by-name refusals where phase 1 measured the clause absent | pending |
| 6 | The degraded routes: per-group recompute, the succession grain's full rebuild in place of the patch route (presented table row- and column-identical to the ledger-bearing rebuild's presented arm), and the sidecar-less key-addressed downgrade — each recorded on the cell and explain-visible | pending |
| 7 | `statement_parity`'s structural no-authoring leg for `smelt-backend-trino`, plus a check that adding Trino introduced no second plan-derivation site and no consumer-side dialect branch | pending |
| 8 | The generative gate: `maintenance_conformance` Trino leg modelled on the Spark leg, over the live tier, oracle-equal after **every** run step with cells resolved under Trino's actual availability, recipe pool no narrower than the admitted families, gated in `compat.yml` | pending |
| 9 | Contract lattice on Trino: `frozen_horizon` and `retain_departed` through the single oracle transform and probe emitter or refused by existing rules, `deferral` refusing per T3, no ad hoc point | pending |
| 10 | Mid-stream schema evolution under maintenance, still oracle-equal; then close: divergences rewritten, `docs-site/` page stating plainly which incremental features Trino does and does not support and why, `verify-phase.sh` green | pending |

## Decision log

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

## Blocked
