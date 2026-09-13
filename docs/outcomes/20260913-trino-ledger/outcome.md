# Outcome: Trino takes Spark's state-residency posture — correctness structures unrealisable, every dependent cell recorded as downgraded

**Created:** 2026-09-13
**Status:** queued
**Driver:** loop. Docker only, no credential, no human gate. Live-tier phases must emit
`<<PHASE_BLOCKED>>` when the coordinator is unreachable, never skip green.
**Depends on:** `20260913-trino-target-spine` (T1) for the backend and the tier.
Independent of `20260913-trino-emission` (T2), which owns expression spelling; this outcome owns
*bookkeeping* — the half the maintenance-plan invariant explicitly excludes from
`smelt-logical`'s single ownership ("ledger DDL/DML in `smelt-state` excluded as bookkeeping").
**Source:** T3 of the five-outcome Trino programme agreed 2026-09-13; rescoped the same day on
the ruling that **Trino's transaction support is equivalent to Spark's**, and that some
incremental features being unsupported on Trino for now is acceptable. Split from T4 at the seam
the maintenance-plan invariant draws. Pattern followed:
`crates/smelt-state/src/ddl_spark.rs`, whose header records every rule as *measured against a
live server rather than read from documentation* (`scripts/spark-probe-ddl.sh`).
**Spec anchors:** `docs/specs/state.md` §"Which dialects realise which structure" (the table this
outcome extends), §"The residency rule", §"The optionality rule", §"The degradation contract",
§"Declarations stay fail-loud", §Diagnostics; `docs/specs/schema_evolution.md`;
`docs/specs/run_state.md`; `docs/specs/multi_backend.md` §"Incremental & schema evolution per
backend"

## The outcome

Trino's column in `state.md` §"Which dialects realise which structure" reads like Spark (Delta)'s:
**no** for the transactional merge ledger, the reconciliation ledger, observed output deltas, the
fingerprint sidecar and the tombstone ledger. Not "not yet" — a permanent, reasoned absence, for
exactly Spark's reason. Iceberg gives per-table atomic commits and no cross-table transaction, so
a ledger write and its data write cannot be made atomic, and the additive fold's never-fold-twice
refusal has no sound realisation there. The residency rule is therefore satisfied by *declining to
claim* these structures rather than by realising them.

One thing has to be checked rather than inherited. Unlike Spark, Trino has explicit
`START TRANSACTION` / `COMMIT` syntax, so the absence of cross-table atomicity is a property of
the Iceberg connector rather than of the SQL surface, and a connector that did give it would make
"no" the wrong answer. A probe confirms the expected behaviour against the live coordinator and
records the server's own words. The expected answer is Spark's; the probe exists so the claim is
measured, as `ddl_spark.rs` measured every one of its rules.

Everything downstream then follows the degradation contract already specified, and the two
invariants that govern it: **a claim implies a builder** (so Trino claims none of these and no run
reaches an execution path expecting one), and **an absence implies a downgrade, never a refusal**
(so every dependent cell falls to the cheapest recompute-family technique preserving the
equivalence invariant, carries `MaintenanceStateDowngraded`, and is visible in `smelt explain`).
The one exception is a declaration whose semantics *are* a statement about state:
`contract.deferral` promises a ledger-measured lag, so on Trino it fails loudly with
`DeclaredContractRequiresState` rather than silently skipping.

What Trino does get is the other half of `smelt-state`: schema-evolution DDL. Every
`SchemaOperation` maps to a Trino/Iceberg statement or to an honest full refresh, and — exactly as
`ddl_spark.rs` did — **every rule in the table is established by running the form against a live
Iceberg table and recording what the server answered**, because a statement the deployed table
refuses is worse than a migration smelt declines to express. Iceberg is the more capable format
here, so this is where Trino is expected to diverge *upward* from Spark's Parquet column and to
land near or above Delta's.

## Success criteria (checkable)

1. **The posture is confirmed by measurement, then stated.** `scripts/trino-probe-state.sh` (in
   the shape of `scripts/spark-probe-ddl.sh`) runs the atomicity candidates against the live tier
   — a multi-statement `START TRANSACTION` … `COMMIT` spanning two Iceberg tables, the same with
   the second statement failing, and a single-statement write — and prints what Trino answered
   verbatim. The decision log records the verdict and the server's error text. If the probe finds
   genuine cross-table atomicity, that is a **finding that changes this outcome** and is escalated
   in the decision log rather than absorbed; the phases below assume Spark's answer.
2. **`state.md`'s realisability table gains a Trino column reading `no` five times**, with the
   reason stated as Spark's is (per-table atomicity, no cross-table transaction) and marked
   permanent rather than pending. `multi_backend.md` §"Incremental & schema evolution per backend"
   states the same for the Trino target.
3. **A claim implies a builder — vacuously, and provably.** A standing test asserts Trino claims
   no correctness structure and that no execution path on a Trino target can reach a builder
   expecting one. The failure this excludes is the one §"Constraints & Invariants" names: a claimed
   structure with no builder means resolution skips the downgrade, the run reaches the execution
   path, finds nothing, and *refuses* — losing exactly the graceful degradation the contract
   promises.
4. **An absence implies a downgrade, never a refusal.** Every cell that would have used a Trino
   correctness structure resolves to its recompute-family equivalent, carries
   `MaintenanceStateDowngraded`, and is rendered by `smelt explain` (text and `--json`). The
   downgrade is derived **once** by the pure availability resolver in `smelt-logical` — no
   consumer re-derives it at run time — and the *ideal* plan still exists as a derived object even
   though it cannot run, per the resolve-late requirement.
5. **The state-bearing declaration refuses by name.** `contract.deferral` on a Trino target fails
   with `DeclaredContractRequiresState`, because the bounded lag it promises is ledger-measured and
   Trino has no frontier to measure it against. `frozen_horizon` and `retain_departed` follow
   whatever the existing grain and posture rules already say — no new lattice point is defined.
6. **Schema-evolution DDL, measured not read.** Every `SchemaOperation` — add nullable column, add
   column with a `default:`, add `NOT NULL` column, drop column, widen a type, `DROP`/`SET NOT
   NULL`, add/drop a struct field, backfill — maps to a Trino/Iceberg statement or to an honest
   full refresh, with the mapping table in the module header and **each row established by
   executing the form against a live Iceberg table**. Trino's type spellings (`VARCHAR` unbounded,
   `TIMESTAMP(6)`, `DECIMAL(p,s)`, `ROW`/`ARRAY`/`MAP`) are derived from what the server accepted.
   `supports_struct_field_ddl`, `supports_nested_array_ddl`, `supports_alter_column_using`,
   `supports_column_mapping` and `supports_merge_schema_write` from T1's matrix are confirmed or
   corrected here, with the correction pushed back into the spec table in the same commit.
7. **`ddl_trino` is a sibling, split before it sprawls.** Trino's state layer lands as a module
   directory (`crates/smelt-state/src/ddl_trino/…`) the way `ddl_bigquery/` is, not as a single
   file — its peers are 1,883 and 1,708 lines and `.claude/large-file-baseline.txt` is a ratchet.
   Reachable only through the same abstract entry points its peers use; no consumer branches on
   Trino outside the state layer.
8. **The staged relation group works without temporary tables.** Trino has none, so
   `supports_staged_relation_group` — the temp-relation-backed statement group behind the
   merge-less conditional write — is realised over a real relation in a scratch schema, with a
   derived non-colliding name, an owned lifecycle, and proved cleanup: a run interrupted between
   the stage and the apply leaves **no** partially-applied data and no orphan relation a later run
   mistakes for its own. If T1 measured the flag `false`, the consuming path refuses by name
   instead.
9. **Locking and versioning are realised or refused by name.** `run_state.md`'s state locking and
   version checks either work on Trino — demonstrated by two concurrent runs where exactly one
   proceeds — or are refused with a diagnostic naming the backend and the missing capability. A
   lock that silently never locks is the failure this criterion excludes.
10. **`.smelt/` stays non-correctness-bearing.** A test deletes `.smelt/` between runs on the Trino
    target and asserts every maintained table equals what it equalled before — the residency rule's
    own falsifiable form, and the one that matters most on a backend whose correctness structures
    are all absent. Under `state.mode: stateless`, nothing is written under `.smelt/` and no
    maintained table's value changes.
11. **Gates green.** `verify-phase.sh` passes; `.claude/hardening-baseline.txt` and
    `.claude/large-file-baseline.txt` are respected rather than bumped; `state_docs_freshness` and
    the diagnostics catalogue gate stay green; every new `DiagnosticCode` has a fixture under
    `examples/broken/` and an entry in `docs/specs/diagnostics.md`.

## Out of scope

- **Building any correctness-structure realisation for Trino.** The merge ledger, reconciliation
  ledger, observed output deltas, fingerprint sidecar and tombstone ledger are declined, for
  Spark's reason. Revisiting that needs a connector that offers cross-table atomicity, which is a
  research question, not a phase.
- **The incremental features the absence costs.** The additive keyed fold's never-fold-twice
  refusal, the succession patch's window-forward route, `contract.deferral`, and the
  sidecar-dependent key-addressed per-group route are **accepted as unsupported on Trino for now**
  (ruling of 2026-09-13). Each takes its specified downgrade or by-name refusal; none is rebuilt
  on a different mechanism here.
- **A two-phase commit, a compensating transaction, or any mechanism that makes a non-atomic
  ledger look atomic.** Worse than no ledger: the degradation path is correct and false trust is
  not.
- **Weakening the residency rule.** A weaker rule is a spec change needing its own research, never
  a workaround discovered mid-phase.
- **The maintenance statements themselves** — the per-family emitters, `statement_parity`'s legs
  and `maintenance_conformance` on Trino are `20260913-trino-incremental`'s. The seam is the
  invariant's own: bookkeeping here, maintenance statements there.
- **Expression and clause emission** (`20260913-trino-emission`).
- **Reworking another backend's state layer.** A defect this reveals in `ddl_duckdb`, `ddl_spark`
  or `ddl_bigquery` is recorded and handed on. In particular, tightening Spark's column is not
  this outcome's business even though Trino now shares it.
- **Iceberg table maintenance** (`expire_snapshots`, `rewrite_data_files`, snapshot retention) —
  operating concerns, not correctness.
- **State in a second catalog** or any store outside the target backend: the trust boundary forbids
  it being correctness-bearing, so there is nothing to build.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Confirm the posture: `scripts/trino-probe-state.sh` runs the cross-table `START TRANSACTION` candidates (including a failing second statement) against the live tier and prints Trino's answers verbatim; escalate in the decision log if genuine cross-table atomicity is found, since that would change this outcome | pending |
| 2 | Spec delta: `state.md`'s realisability table gains a Trino column reading `no` five times with Spark's reason stated as permanent, `multi_backend.md` §"Incremental & schema evolution per backend" states it for the target, and the diagnostics the degradation needs are named | pending |
| 3 | Schema-evolution DDL: `ddl_trino/` as a module directory with the measured `SchemaOperation` → Trino/Iceberg mapping table in its header, type spellings derived from what the server accepted, and T1's five schema-related capability cells confirmed or corrected back into the spec table | pending |
| 4 | Wire the absence: Trino claims no correctness structure, availability resolution downgrades every dependent cell to its recompute equivalent with `MaintenanceStateDowngraded`, derived once by the pure resolver with the ideal plan still materialised | pending |
| 5 | The two invariants as standing tests: no execution path on Trino reaches a builder for an unclaimed structure (claim ⇒ builder), and no absence produces a refusal where the contract specifies a downgrade (absence ⇒ downgrade) | pending |
| 6 | `contract.deferral` refuses on Trino with `DeclaredContractRequiresState`; `frozen_horizon` and `retain_departed` follow the existing grain and posture rules with no new lattice point | pending |
| 7 | The staged relation group without temp tables: scratch-schema relation with a derived non-colliding name, owned lifecycle, proved cleanup after an interruption between stage and apply — or a by-name refusal if T1 measured the flag `false` | pending |
| 8 | Locking and versioning: two concurrent runs where exactly one proceeds, or a refusal naming the backend and the missing capability — never a lock that never locks | pending |
| 9 | `.smelt/` is not correctness-bearing on Trino: delete-between-runs equality, and `state.mode: stateless` writing nothing while changing no maintained table's value | pending |
| 10 | Surface and close: `smelt explain` rendering Trino's downgrades (text + `--json`), diagnostics catalogue and `examples/broken/` fixtures, `docs-site/` state page updated with what Trino costs and why, `verify-phase.sh` green with no baseline bumped | pending |

## Decision log

- 2026-09-13 (scaffold, before phase 1): **Trino's transaction support is taken as equivalent to
  Spark's, and some incremental features are accepted as unsupported on Trino for now.** Ruling by
  the programme owner. `state.md` §"Which dialects realise which structure" already records
  Spark's absence as permanent and reasoned — "Delta provides per-table atomicity and no
  cross-table transaction, so a ledger write and its data write cannot be made atomic, and the
  additive fold's never-fold-twice refusal has no sound realisation there" — and Iceberg has the
  same shape. So this outcome starts from Spark's column rather than treating residency as an open
  binary, which is why phase 1 is a confirmation probe rather than an investigation. The one fact
  not inheritable is that Trino, unlike Spark, has `START TRANSACTION`/`COMMIT` syntax: the
  absence of cross-table atomicity is a property of the Iceberg connector, not of the SQL surface,
  so it is measured rather than assumed, and a probe finding otherwise escalates instead of being
  absorbed.

## Blocked
