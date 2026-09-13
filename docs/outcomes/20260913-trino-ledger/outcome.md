# Outcome: smelt's own state on Trino is either atomically resident or explicitly degraded — never silently pretended

**Created:** 2026-09-13
**Status:** queued
**Driver:** loop. Docker only, no credential, no human gate. Live-tier phases must emit
`<<PHASE_BLOCKED>>` when the coordinator is unreachable, never skip green.
**Depends on:** `20260913-trino-target-spine` (T1) for the backend and the tier.
Independent of `20260913-trino-emission` (T2), which owns expression spelling; this outcome owns
*bookkeeping* statements — the half the maintenance-plan invariant explicitly excludes from
`smelt-logical`'s single ownership ("ledger DDL/DML in `smelt-state` excluded as bookkeeping").
**Source:** T3 of the five-outcome Trino programme agreed 2026-09-13. Split from T4 at the
seam the invariant itself draws. Pattern followed: `crates/smelt-state/src/ddl_spark.rs` (whose
header records every rule as *measured against a live server rather than read from
documentation*, via `scripts/spark-probe-ddl.sh`) and `ddl_bigquery/`'s submodule split.
**Spec anchors:** `docs/specs/state.md` §"The state-structure inventory", §"The residency rule",
§"The optionality rule", §"The degradation contract", §"Declarations stay fail-loud",
§Diagnostics; `docs/specs/schema_evolution.md`; `docs/specs/run_state.md`;
`docs/specs/multi_backend.md` §"Incremental & schema evolution per backend"

## The outcome

Trino answers the question `state.md` §"The residency rule" asks of every backend: *can a
correctness structure be written in the same backend transaction as the write it records, so
that data-without-bookkeeping is impossible by construction?* Trino has no transactional DDL
and no temporary tables, so the answer cannot be inherited from DuckDB or Spark — it has to be
**measured against the live coordinator**, and then honoured.

Whichever way the measurement lands, the result is a stated, printed, tested position:

- If Iceberg on Trino can commit smelt's bookkeeping atomically with the data write, then
  `ddl_trino` realises the correctness structures — the merge ledger, the interval store, the
  observed-delta and tombstone relations — as engine-resident Iceberg tables, and the residency
  rule holds on a fifth backend.
- If it cannot, the backend declares it **has no builder** for the structures it cannot write
  atomically, availability resolution downgrades every dependent cell to the cheapest
  recompute-family technique that preserves the equivalence invariant, each downgrade is
  recorded as `MaintenanceStateDowngraded` and printed by `smelt explain`, and a declaration
  whose semantics *are* a statement about state (`contract.deferral`) fails loudly with
  `DeclaredContractRequiresState`.

What must not happen — and what a standing test proves cannot happen — is the third option: a
ledger written non-atomically that smelt then *trusts*, violating the trust-boundary corollary
("smelt trusts a correctness structure's content because smelt is its only writer and it commits
atomically with the data"). A partially-committed ledger on a backend that cannot be atomic is
worse than no ledger, because the degradation path is correct and the false-trust path is not.

Alongside that, Trino gets the other half of `smelt-state`: schema-evolution DDL. Every
`SchemaOperation` maps to a Trino/Iceberg statement or to a full refresh, and — exactly as
`ddl_spark.rs` did — **every rule in the table is established by running the form against a live
Iceberg table and recording what the server answered**, because a statement the deployed table
refuses is worse than a migration smelt declines to express.

## Success criteria (checkable)

1. **The atomicity question is answered by measurement and written down.** A probe script
   (`scripts/trino-probe-state.sh`, in the shape of `scripts/spark-probe-ddl.sh`) runs the
   candidate forms against the live tier — multi-statement `START TRANSACTION`/`COMMIT` over
   Iceberg, a single-statement `MERGE` carrying both the data and the bookkeeping effect, and
   the failure/rollback behaviour when the second statement of a pair fails — and prints what
   Trino answered. The decision log records the verdict, the exact statements, and the server's
   own error text. `docs/specs/state.md` and `multi_backend.md` §"Incremental & schema evolution
   per backend" state Trino's residency position in the same commit.
2. **The verdict is implemented as the spec's own binary, not a third way.** Either every
   correctness structure the measurement admits is realised in `ddl_trino` and written
   atomically with its data write, **or** the backend reports no builder for it and the
   degradation contract runs. A standing test proves there is no path on which smelt writes a
   correctness structure non-atomically on Trino and then reads it as trusted.
3. **`ddl_trino` exists as a sibling, split before it sprawls.** Trino's state layer lands as a
   module directory (`crates/smelt-state/src/ddl_trino/…`) the way `ddl_bigquery/` is, not as a
   single file — its two peers are 1,883 and 1,708 lines, and
   `.claude/large-file-baseline.txt` is a ratchet. It is reachable only through the same
   abstract entry points its peers are; no consumer branches on Trino outside the state layer.
4. **Schema-evolution DDL, measured not read.** Every `SchemaOperation` — add nullable column,
   add column with a `default:`, add `NOT NULL` column, drop column, widen a type, `DROP`/`SET
   NOT NULL`, add/drop a struct field, backfill — maps to a Trino/Iceberg statement or to an
   honest full refresh, with the mapping table in the module header and each row established by
   executing the form against a live Iceberg table. Trino's type spellings (`VARCHAR` unbounded,
   `TIMESTAMP(6)`, `DECIMAL(p,s)`, `ROW`/`ARRAY`/`MAP`) are derived from what the server
   accepted. `supports_struct_field_ddl`, `supports_nested_array_ddl`,
   `supports_alter_column_using`, `supports_column_mapping` and `supports_merge_schema_write`
   in the T1 matrix are confirmed or corrected here, with the correction pushed back into the
   spec table.
5. **The staged relation group works without temporary tables.** Trino has none, so
   `supports_staged_relation_group` — the temp-relation-backed statement group behind the
   merge-less conditional write — is realised over a real relation in a scratch schema, with its
   name derived (never colliding across concurrent runs), its lifecycle owned, and its cleanup
   proved: a run interrupted between the stage and the apply leaves **no** partially-applied
   data and no orphan relation that a later run mistakes for its own. If instead the flag is
   measured `false`, the consuming path refuses by name rather than degrading silently.
6. **Locking and versioning are realised or refused by name.** `run_state.md`'s state locking
   and version checks either work on Trino — demonstrated by two concurrent runs where exactly
   one proceeds — or are refused with a diagnostic that names the backend and the missing
   capability. A lock that silently never locks is the failure this criterion exists to exclude.
7. **`.smelt/` stays non-correctness-bearing.** A test deletes `.smelt/` between runs on the
   Trino target and asserts every maintained table equals what it equalled before — the
   residency rule's own falsifiable form. Under `state.mode: stateless`, nothing is written
   under `.smelt/` and no maintained table's value changes.
8. **The degradation is visible wherever it applies.** Every cell downgraded for want of a Trino
   structure carries `MaintenanceStateDowngraded`, is rendered by `smelt explain` (text and
   `--json`), and is derived **once** by the pure availability resolver — no consumer re-derives
   it at run time. The ideal plan still exists as a derived object even where it cannot run, per
   §"The degradation contract"'s resolve-late requirement.
9. **Gates green.** `verify-phase.sh` passes; `.claude/hardening-baseline.txt` and
   `.claude/large-file-baseline.txt` are respected rather than bumped; `state_docs_freshness`
   and the diagnostics catalogue gate stay green; every new `DiagnosticCode` has a fixture under
   `examples/broken/` and an entry in `docs/specs/diagnostics.md`.

## Out of scope

- **The maintenance statements themselves** — the per-family emitters in `smelt-logical`, the
  `statement_parity` executed-vs-emitted legs, and `maintenance_conformance` on Trino are
  `20260913-trino-incremental`'s. The seam is the invariant's own: bookkeeping DDL/DML here,
  maintenance statements there.
- **Expression and clause emission** (`20260913-trino-emission`).
- **Changing the residency rule.** If Trino cannot be atomic, this outcome takes the
  degradation the spec already defines; it does not relax the rule, invent a two-phase commit,
  or add a compensating-transaction mechanism. A weaker residency rule is a spec change needing
  its own research, not a workaround discovered mid-phase.
- **Reworking another backend's state layer.** A defect this work reveals in `ddl_duckdb`,
  `ddl_spark` or `ddl_bigquery` is recorded and handed on.
- **Ledger compaction, retention or vacuuming** of Iceberg snapshots, and Iceberg table
  maintenance generally (`expire_snapshots`, `rewrite_data_files`) — operating concerns, not
  correctness.
- **State in a second catalog** or any store outside the target backend: the trust boundary
  forbids it being correctness-bearing, so there is nothing to build.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Measure first: `scripts/trino-probe-state.sh` runs the atomicity candidates (multi-statement transaction over Iceberg, single-statement combined write, mid-pair failure behaviour) against the live tier and prints the server's answers verbatim | pending |
| 2 | Spec delta from the measurement: Trino's residency position in `state.md` and `multi_backend.md` §"Incremental & schema evolution per backend" — which structures are realisable, which degrade, and the diagnostics either path needs | pending |
| 3 | Schema-evolution DDL: `ddl_trino/` as a module directory with the measured `SchemaOperation` → Trino/Iceberg mapping table in its header, Trino type spellings derived from what the server accepted, and the T1 capability cells confirmed or corrected back into the spec table | pending |
| 4 | The correctness structures, per phase 1's verdict: realise the merge ledger, interval store, observed-delta and tombstone relations as Iceberg tables written atomically with their data write — or implement "no builder" and let availability resolution downgrade, with `MaintenanceStateDowngraded` recorded | pending |
| 5 | The standing no-false-trust test: prove there is no path on which smelt writes a correctness structure non-atomically on Trino and then reads it as trusted | pending |
| 6 | The staged relation group without temp tables: a scratch-schema relation with a derived non-colliding name, owned lifecycle, and proved cleanup after an interruption between stage and apply — or a by-name refusal if the flag measured `false` | pending |
| 7 | Locking and versioning: two concurrent runs where exactly one proceeds, or a refusal naming the backend and the missing capability — never a lock that never locks | pending |
| 8 | `.smelt/` is not correctness-bearing on Trino: delete-between-runs equality, and `state.mode: stateless` writing nothing while changing no maintained table's value | pending |
| 9 | Surface and close: `smelt explain` rendering Trino's downgrades (text + `--json`), diagnostics catalogue and `examples/broken/` fixtures, `docs-site/` state page updated, `verify-phase.sh` green with no baseline bumped | pending |

## Decision log

## Blocked
