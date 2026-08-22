# BigQuery — what is left

**Spec:** [`docs/specs/multi_backend.md`](../specs/multi_backend.md)
**Spec diff:** none — every item below is already recorded in that spec's
§Known Divergences / Open Questions. This plan does not change the spec; it
collects the open items into one worklist and records what "done" means for
each. Retiring an item means deleting or rewriting its spec entry in the same
commit.
**Docs:** per-item (most items are code + spec only; item 1 also touches
`docs-site/`).

## What this is

A backlog, not a phased plan. The items are independent — no ordering
constraint between them — so each can be picked up alone, and each carries its
own verification. Two are not work items at all but decisions (§Decisions
first).

Everything here is scoped to BigQuery. The backend itself is complete and
measured: the fixed-recipe parity suites and the generative
maintenance-conformance leg both run green against a live warehouse
(`bash scripts/bigquery-conformance.sh`, `--test-threads=1`: 21 passed / 0
failed, 2190.85s, 2026-08-21), and the type oracle
(`crates/smelt-db/tests/prop_helpers/bigquery_oracle.rs`) runs as a third leg
of `prop_type_inference` alongside DuckDB and Spark.

## Progress tracking

| # | Item | Kind | Needs live warehouse | Status |
|---|------|------|----------------------|--------|
| D1 | Cross-engine exchange with BigQuery — GCS or nothing | decision | no | done — accepted as Constraint (filesystem-local by design) |
| D2 | Whether BigQuery gets a CI tier | decision | no | done — accepted as standing decision (no CI tier) |
| 1 | Schema-evolution DDL for BigQuery | feature | yes | done |
| 2 | `supports_native_ivm` — emit the maintained object, or keep the `false` | feature | yes | pending |
| 3 | `ColumnScopedMerge` on an unresolvable projection | decision (+ a missing test) | no | done — accepted as Constraint (ROADMAP #3) |
| 4 | `dags` non-vacuity self-check on the BigQuery leg | coverage | yes (a sweep) | done |
| 5 | `supports_pipe_syntax` has no live coverage | coverage | yes (one case) | done |
| 6 | Conformance sweep vs the one-hour credential window | scaling | yes | pending |
| 7 | Stale spec sentence: the keyed-`MERGE` fix *was* confirmed live | docs | no | done |

## Decisions first

These two gate how much the rest is worth. Neither is implementation work.

### D1 — Cross-engine exchange with BigQuery: GCS or nothing

`cross_engine_parity` and `cross_engine_types_parity`
(`crates/smelt-cli/tests/`) are DuckDB↔Spark only. They hand off through a
Parquet file on a **shared local filesystem**, resolved downstream by the
`read_parquet()` substitution. BigQuery cannot read a host path, and
`multi_backend.md` independently forbids load paths that assume the server
shares the host filesystem — a load path that only works when it does is
specified as a bug, not a deployment constraint.

So a DuckDB↔BigQuery pair is not a third leg of an existing loop; it needs an
object-store exchange boundary (GCS), which the spec currently places
explicitly out of scope until a mirrored test demands one.

**The decision:** either bring remote object stores into scope — which is a
cross-cutting change to the exchange boundary, not a BigQuery feature — or
state in the spec that cross-engine exchange is a two-engine, filesystem-local
capability by design and that BigQuery is out of it. Today the spec implies
the former is merely un-built, which reads as a gap when it is a boundary.

**Done when:** the spec says which, and the §Known Divergences entry either
names a tracking plan or is rewritten as a Constraint.

**Done 2026-08-22 — accepted as a Constraint (filesystem-local by design).**
`multi_backend.md` §Constraints & Invariants now states cross-engine exchange
is a two-engine, filesystem-local capability by design: a third engine that
cannot read a host path needs a new object-store exchange boundary
(S3/GCS/ADLS), which is a cross-cutting change to the exchange design, not a
BigQuery feature, and stays out of scope until a concrete consumer demands
it. The §Known Divergences entry is retired — the type-level half of the gap
is already closed independently by the BigQuery type oracle
(`bigquery_oracle.rs`).

### D2 — Whether BigQuery gets a CI tier

Spark parity runs per-PR on changed paths and nightly in full. BigQuery runs
**only when a developer runs it by hand** (`scripts/bigquery-parity.sh`,
`scripts/bigquery-conformance.sh`), gated on `SMELT_BQ_PROJECT`. A BigQuery
regression therefore surfaces when someone happens to run a sweep, not on any
schedule.

This is deliberate — it keeps cloud credentials, and the one-hour credential
window, out of CI entirely — but it means every other item on this list, once
closed, is unguarded against regression. Note the interaction with item 6: a
CI sweep is ~37 minutes of warehouse time per run, so "nightly" has a real
bill attached.

**The decision needs:** a service account, a GitHub secret, and a billing
call. Not something to drift into.

**Done when:** either a `bigquery-parity` job exists in
`.github/workflows/compat.yml` on the same gated shape as `spark-parity`, or
the spec's "BigQuery has no CI tier" entry is restated as a standing decision
with the reasoning, rather than as an open divergence.

**Done 2026-08-22 — accepted as a standing decision (no CI tier).**
`multi_backend.md` §Constraints & Invariants now carries "BigQuery has no CI
tier, by decision, not by omission" — the reasoning (credentials off CI,
~37min/run billing cost, requires a service account and GitHub secret this
spec cannot provision) and the cost (a claim of Spark-equivalent coverage is
a claim about which gates exist, never when they run) recorded in place. The
§Known Divergences entry is retired; no `bigquery-parity` CI job was added.
Item 6 (case-count vs. credential window) remains open independently, since
hand-run sweeps still need it regardless of CI status.

## Functional gaps

### 1 — Schema-evolution DDL for BigQuery

**What** (as this item was written). Not implemented. GoogleSQL rejects the
type names the DuckDB generator emits (`VARCHAR`, `TEXT`, `DOUBLE` are each `Type not found`) and
has no `ALTER COLUMN … USING`, so no generator is shared. A schema change on a
BigQuery model resolves to a full refresh instead of a migration.

**Why it matters.** This is the largest missing *capability* — it changes what
a user can do, not just what is verified. It already fails safe: the refusal
names its reason and no rejected DDL reaches the warehouse.

**Where.** The DuckDB generator and its Spark counterpart; `schema_evolution_parity`
already carries a BigQuery leg, so there is a place to assert into.

**Done when.** A GoogleSQL generator emits the add-column / widen-type cases the
other backends support, `schema_evolution_parity`'s BigQuery leg asserts them
against a live warehouse, and the full-refresh fallback remains for the cases
GoogleSQL genuinely cannot express (named, not silent). User docs updated —
`docs-site/docs/guide/targets.md` currently says nothing about the limitation.

**Done 2026-08-21.** `crates/smelt-state/src/ddl_bigquery.rs` emits the flat
cases (add column, drop column, scalar widening, `DROP NOT NULL`, backfill
`UPDATE`); everything GoogleSQL cannot express resolves to
`FullRefreshBlocked` naming the column and the limitation.

Two findings shaped it. First, every rule is *measured*, not read from docs:
`scripts/bigquery-probe-ddl.sh` (committed) runs 55 forms against the live
warehouse on one fresh table each, and three of its answers contradict the
obvious guess — a `DEFAULT` cannot ride on an `ADD COLUMN` (it needs a
following `SET DEFAULT`), `BIGNUMERIC → FLOAT64` is *refused* despite being a
documented widening, and `SET DATA TYPE` on a `REQUIRED` column is refused
outright, which is why the generator consults the deployed schema and plans
that case as a full refresh rather than letting the statement fail mid-run.

Second, the dispatch was wronger than the tracking entry said. Only the
*complex* change kinds ever reached a backend generator; `ADD COLUMN`,
`DROP COLUMN`, `ALTER COLUMN … TYPE` and `SET NOT NULL` were emitted inline in
DuckDB's dialect for every backend. The old BigQuery arm — the refusal this
item was written against — sat in the branch those changes never took, so a
flat schema change on BigQuery did not resolve to a full refresh at all; it
emitted DuckDB SQL the warehouse rejects. BigQuery now routes the whole diff
through its own generator before that loop.

Verification: `schema_evolution_parity` grew two legs that execute the
statements `plan_migration_for_backend` *emits* (rather than hand-written DDL,
which measures the warehouse and not the generator) — green on DuckDB and
against the live warehouse. Shown load-bearing: swapping `SET DATA TYPE` back
to DuckDB's `ALTER COLUMN … TYPE` makes the BigQuery leg fail live.

### 2 — `supports_native_ivm`: emit the maintained object, or keep the `false`

**What.** BigQuery advertises `supports_native_ivm: false` while the warehouse
accepts `CREATE MATERIALIZED VIEW` with incremental refresh. Unlike DuckDB and
Spark, this flag's value describes smelt, not the engine.

**Why it matters.** `true` obliges smelt to emit the native maintained object
and cede freshness to the engine; that emission path does not exist, so
`refresh: materialized_view` hard-errors on BigQuery exactly as everywhere
else. This is the first flag in the matrix whose value is an implementation
statement — worth resolving in one direction rather than leaving the matrix
cell ambiguous.

**Done when.** Either the emission path exists and the flag flips (with a
parity leg proving the engine maintains it), or the spec states that smelt
does not delegate maintenance to engine-native IVM as a design position, and
the flag is documented as meaning "smelt does not emit this" everywhere.

### 3 — `ColumnScopedMerge` on an unresolvable projection

**What.** A model whose output columns are not statically resolvable cannot use
`Technique::ColumnScopedMerge` on BigQuery: the whole-row `MERGE` needs an
explicit column list there, and a surviving wildcard projection leaves it
empty, so the run fails with an error naming the model. DuckDB and Spark are
unaffected — their `UPDATE SET *` needs no list.

**Why it matters.** It is a live refusal on a real technique, BigQuery-only.

**Checked 2026-08-21 — still reachable; the spec entry is accurate.** The
source-derived projection work did not empty the unresolvable set. Alias
synthesis names unnamed *expressions*; a wildcard is not enumerable at all, so
`Projection::columns` stays `None` and `output_columns` stays empty. Two
committed tests pin exactly that —
`projection_source_derived::bare_wildcard_projection_still_yields_empty_output_columns`
and `::struct_spread_select_item_yields_empty_output_columns`.

Nothing between there and the write narrows it: no admission-time or
choice-time guard reads `output_columns`, so a `SELECT *` model carrying a
`unique_key` resolves to `ColumnScopedMerge` normally and is refused only at
execution, by `smelt_backend::require_merge_columns`
(`crates/smelt-backend/src/lib.rs:76`). The refusal itself is sound and
well-aimed — it names the model, explains that GoogleSQL has no
`UPDATE SET *`, and gives the remedy — and it is the right shape, since the
alternative is a syntactically valid `MERGE` whose matched arm assigns nothing
and silently stops updating rows.

**The refusal now has a test** (`crates/smelt-backend/tests/merge_columns_guard.rs`,
2026-08-21). It was previously called from `maintenance_driver.rs:2580` and
`lib.rs:430` and asserted nowhere — a documented limitation whose guard is
unproven is one refactor away from becoming the silent-no-op it exists to
prevent. Three cases: BigQuery refuses an empty list and names the model,
BigQuery admits a resolved one, and the star dialects (DuckDB, Spark,
PostgreSQL) accept an empty list — the last keeping the guard narrow, since
widening it would refuse `SELECT *` models the other backends handle
correctly. Verified load-bearing: neutering the condition makes the refusal
case fail.

**So the item is a decision, not a bug.** Either:

- **Accept it.** Rewrite the spec entry as a Constraint: a BigQuery model using
  `ColumnScopedMerge` must have a statically enumerable projection. Cheap,
  honest, and the refusal already implements it.
- **Close it.** Expand the wildcard at compile time. smelt already knows the
  upstream schemas — the `TypeContext` the projection is derived against is
  assembled from them — so `SELECT *` over resolvable upstreams could yield a
  real column list rather than `None`. That is a change to the projection
  owner, benefiting every consumer of `output_columns`, not a BigQuery patch.
  Note it cannot cover every case: a wildcard over an unresolvable upstream
  stays unknown, so the refusal survives either way.

**Done when.** The spec entry is either restated as a Constraint or retired by
the compile-time expansion, and `require_merge_columns` has a test.

**Done 2026-08-22 — accepted as a Constraint.** `multi_backend.md` §Constraints & Invariants now
carries "A BigQuery `ColumnScopedMerge` model must have a statically enumerable projection"; the
§Known-Divergences entry is retired. `require_merge_columns` already has its test
(`crates/smelt-backend/tests/merge_columns_guard.rs`, above). The compile-time expansion was *not*
taken here — but the broader position it points at, that smelt should always know a model's output
schema, is recorded as ROADMAP #3 "Total Output-Schema Resolution". The information exists for the
common case (`derive_projection` returns `None` on any wildcard without consulting the upstream
schema it already holds), and the residual unresolvable cases — external tables, CSV seeds,
unresolved function-star, `ON`-joins — are each independently closable there.

## Coverage gaps

### 4 — `dags` non-vacuity self-check on the BigQuery leg

**What.** `dags`'s seeded-divergence self-check — the one that proves a case's
per-node comparison is *capable* of failing — has a wrapper in
`maintenance_conformance_spark` but not in `maintenance_conformance_bigquery`.
(`harness_self_check`, the gate-oracle self-check, does run on all three legs.)

**Why it matters, more than it looks.** On 2026-08-21 the same check proved the
Spark `dags` leg had been passing **vacuously** — the full-refresh twin shared
the incremental project's schema, so the equality assertion could read one
already-overwritten table for both sides, and every earlier green run carried
no evidence about the incremental engine at all. BigQuery's twin resolves to
its own per-case dataset, so its comparison is distinct *by construction* — but
what is missing is the assertion that says so, and construction is exactly what
a future change would alter.

**Cost.** A live sweep to confirm, which is why it was tracked rather than
landed.

**Done when.** `dags_bigquery` carries the seeded-divergence wrapper, and it is
demonstrated load-bearing the way the Spark one was: reverting the twin
derivation makes it fail.

**Done 2026-08-22.** `dags_bigquery.rs` grew
`dags_oracle_flags_a_seeded_divergence_on_bigquery`, a thin wrapper over
`smelt_maintenance_testkit::families::dags::run_oracle_flags_a_seeded_divergence`
— the same function the Spark leg's self-check already calls. All 6 tests
in `dags_bigquery` passed live (`smelt-bq-test-20260816`, 2026-08-22,
`--test-threads=1`, 816.92s), including the new wrapper.

Shown load-bearing, but via a different failure mode than Spark's shared-
schema vacuity: `BigQueryConformanceBackend::twin_target`/`twin_schema`
were temporarily collapsed onto `target`/`schema` — reproducing the exact
pre-fix state `main.rs`'s point-3 finding describes — and the self-check
failed immediately, not by accepting a corrupted multiset but by
reproducing the `409 Already Exists` collision that motivated
`twin_target` in the first place (both projects then race to create the
same table on the same dataset). Still the right evidence: the wrapper's
pass depends on the distinct-dataset property it exists to assert, and
removing that property breaks it. Reverted immediately after; re-run
confirmed green (63.87s).

### 5 — `supports_pipe_syntax` has no live coverage

**What.** BigQuery is the only backend reporting `true`, and no parity fixture
writes a pipe query, so the printer's emit-pipes-natively path has no live
coverage on the one backend that would take it. Every other BigQuery-relevant
printer path does have a leg: `materialization_parity`, `seed_parity`,
`lowering_parity`, `merge_parity`, `incremental_parity`,
`schema_evolution_parity`.

**Why it matters.** A capability flag claiming `true` with nothing exercising it
is a claim, not a fact — and the capability-conformance suite asserts the flag's
*value*, not that the path it enables works.

**Cost.** The cheapest item here: one fixture in an existing parity leg, one
warehouse round trip.

**Note.** `NOT MATCHED BY SOURCE` is likewise uncovered, but for an unrelated
reason — no emitter produces the clause on any backend yet, so there is nothing
to run. Do not bundle it with this.

**Done when.** A pipe query runs through a BigQuery parity leg against a live
warehouse.

**Done 2026-08-22.** `crates/smelt-cli/tests/pipe_parity.rs` runs one pipe
query — pre-aggregation `\|> WHERE`, `\|> EXTEND`, `\|> AGGREGATE … GROUP BY`,
post-aggregation `\|> WHERE`, `\|> ORDER BY`, `\|> LIMIT`, reading its upstream
through a `smelt.<path>` reference — on both DuckDB (lowered) and a live
BigQuery warehouse (native), asserting identical rows. Green live, 16.3s.
Registered in `scripts/bigquery-parity.sh`.

**The cheapest item on the list found the biggest bug.** The first run failed
on **DuckDB**, before BigQuery was reached: `run failed at model
'pipe_showcase': SQL could not be parsed for presentation projection`. Every
compiled model passes through `hide_state_columns` →
`presentation_projection`, which parsed the SQL and demanded a `SELECT_STMT`
before checking whether it had any work to do. A pipe query has no top-level
`SELECT_STMT`, so **every pipe-query model failed to run, on every backend** —
a whole documented language feature was unreachable end-to-end, and the error
blamed the user's SQL for being unparseable. Nothing caught it because the
`examples/test_workspace` pipe models are only ever *diagnosed*, never run, and
`pipe_equivalence` drives the printer directly rather than `smelt run`.

The fix is two guards in `presentation_projection`: an empty `state_bearing`
map returns the SQL untouched without parsing it (nothing to hide ⇒ no
standing to judge the form), and a pipe query that *does* read a state-bearing
model refuses with a new `PipeQueryOverStateBearingModel` variant naming the
form, rather than reporting unparseable SQL. That second case stays fail-closed
on purpose: this emitter hides state columns behind a select list, and a pipe
query has none to edit, so passing it through would leak `__part` columns into
a consumer's schema.

**Non-vacuity — the item-4 lesson applied up front.** A live BigQuery leg alone
would *not* have proven anything about native emission: GoogleSQL accepts the
lowered SQL too, so flipping `supports_pipe_syntax` to `false` would leave the
parity leg green. Two assertions close that. `crates/smelt-dialect/tests/pipe_native.rs`
pins that BigQuery emits `\|>` verbatim (and that the lowering backends emit
none) for the same query — shown load-bearing: flipping the flag to `false`
fails it with the lowered SQL in the message. And `pipe_parity` asserts its
BigQuery leg *ran* whenever `bigquery_enabled()`, so a silent DuckDB-only pass
is a failure rather than a skip.

### 6 — Conformance sweep vs the one-hour credential window

**What.** An all-green 21-case sweep takes 2190.85s (~37 min) — roughly three
fifths of the one-hour token window, and a sweep cannot be refreshed without a
human re-entering the passphrase. The pool is not free to grow much before
concurrency or a case-count reduction becomes necessary.

**The constraint is a quota, not latency.** Repeated modification of *one*
table is refused with `Your table exceeded quota for table update operations`
after roughly eight rapid statements; the same rate spread across distinct
tables is not. So concurrency is available — it just requires a fresh target
table per case. Concurrency is preferred to cutting cases, because it preserves
coverage.

**Also worth remembering.** A failing sweep costs a fraction of a passing one
(1142.10s with eight failures), so headroom measured on a red suite is an
illusion. Start a sweep on a freshly minted token.

**Done when.** Either the pool runs concurrently within one window with a fresh
target table per case, or the case count is a decided number with the reasoning
recorded — the current state is "undecided", which is the actual defect.

## Documentation

### 7 — Stale spec sentence: the keyed-`MERGE` fix *was* confirmed live

`multi_backend.md`'s keyed-fold `MERGE` entry ends "asserted offline; no live
sweep has confirmed the case passes yet." But
`gate_keyed_bigquery::keyed_pool_upholds_end_state_equivalence_on_bigquery`
(`crates/smelt-cli/tests/maintenance_conformance_bigquery/gate_keyed_bigquery.rs`)
is in the suite that measured 21 passed / 0 failed on 2026-08-21, so the sweep
did confirm it.

Free to fix, no warehouse needed. Worth doing early — a spec sentence that
understates its own evidence invites someone to spend a token window
re-confirming it.

## References

- [`docs/specs/multi_backend.md`](../specs/multi_backend.md) — the parity contract, capability matrix, CI tiering, and the §Known Divergences entries this plan tracks.
- [`docs/research/20260816-bigquery-backend.md`](../research/20260816-bigquery-backend.md) — the backend's design decisions, provisioned environment, and measured findings.
- [`docs/plans/20260817-bigquery-generative-conformance.md`](20260817-bigquery-generative-conformance.md) — the generative leg; its §Deferred entries are both now closed.
- [`docs/plans/20260819-source-derived-projection.md`](20260819-source-derived-projection.md) — relevant to item 3.
