---
feature: multi_backend
status: experimental
last_reviewed: 2026-06-30
owners: [andrew]
---

# Multi-backend execution & backend parity

> **What this is.** A normative spec for how smelt runs the same logical model across more
> than one execution backend (today DuckDB and Spark), and the parity contract that binds
> them: the `BackendCapabilities` matrix, how the dialect printer lowers logical SQL to each
> backend's valid physical SQL, and the cross-engine data-exchange rules. Out of scope: the
> `Backend` trait method surface itself (see `architecture.md` §"Backend trait surface"); how
> the batched-refresh strategy is *chosen* (see `incremental_models.md`); how schema changes are
> *classified* (see `schema_evolution.md`); target YAML shape (see `smelt_yml.md`). This spec
> owns the **parity contract** that ties those together.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is
> the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No
> plan-phase headings or status callouts in §Surface/§Semantics/§Design/§Constraints;
> implementation status goes in §Known Divergences with a plan link.

## Surface

- **Backends.** A target's `type:` selects a backend (`duckdb` | `spark` | `bigquery`; see
  `smelt_yml.md` §"Target shape"). Each backend declares a `SqlDialect` (`DuckDB` | `SparkSQL` |
  `PostgreSQL` | `BigQuery`) and a `BackendCapabilities` value. A `bigquery` target names a
  `project`, `dataset`, and `location` in place of DuckDB's `database` or Spark's `connect_url`.
- **Capability matrix.** `BackendCapabilities` is the single declared description of what a
  backend's SQL surface supports. Backends differ **only** in (a) their capability flags and
  (b) the dialect-specific physical SQL the printer emits; they do **not** differ in which
  smelt models a user may write. The flags are:

  | Flag | DuckDB | Spark (Delta) | Spark (Parquet) | BigQuery |
  |------|:------:|:-------------:|:---------------:|:--------:|
  | `supports_qualify` | ✓ | ✗ | ✗ | ✓ |
  | `supports_create_or_replace_table` | ✓ | ✗ | ✗ | ✓ |
  | `supports_create_or_replace_view` | ✓ | ✓ | ✓ | ✓ |
  | `supports_merge` | ✓ | ✓ | ✗ | ✓ |
  | `supports_column_scoped_merge` | ✓ | ✓ | ✗ | ✓ |
  | `supports_merge_not_matched_by_source` | ✗ | ✓ | ✗ | ✓ |
  | `supports_staged_relation_group` (temp-relation-backed statement group, for the merge-less conditional write) | ✓ | ✓ | ✓ | ✓ |
  | `supports_pivot` | ✓ | ✓ | ✓ | ✓ |
  | `supports_date_literal` | ✓ | ✗ | ✗ | ✓ |
  | `supports_concat_operator` (`\|\|`) | ✓ | ✓ | ✓ | ✓ |
  | `supports_array_literal` (`[a,b]`) | ✓ | ✗ | ✗ | ✓ |
  | `supports_transactional_ddl` | ✓ | ✗ | ✗ | ✓ |
  | `supports_double_colon_cast` (`x::T`) | ✓ | ✗ | ✗ | ✗ |
  | `supports_trailing_commas` | ✓ | ✗ | ✗ | ✓ |
  | `supports_insert_overwrite` | ✗ (emulated) | ✓ | ✓ | ✗ (emulated) |
  | `supports_native_ivm` | ✗ | ✗ | ✗ | ✓ |
  | `supports_retraction` | ✗ | ✗ | ✗ | ✗ |
  | `supports_struct_field_ddl` | ✓ | ✓ | ✗ | ✓ |
  | `supports_alter_column_using` | ✓ | ✗ | ✗ | ✗ |
  | `supports_nested_array_ddl` | ✓ | ✓ | ✗ | ✓ |
  | `supports_merge_schema_write` | ✗ | ✓ | ✓ | ✗ |
  | `supports_column_mapping` | ✗ | ✓ | ✗ | ✓ |
  | `supports_pipe_syntax` (`\|>`) | ✗ | ✗ | ✗ | ✓ |
  | `supports_pipe_set_drop_rename` (star-modifier trio `* REPLACE` / `* EXCLUDE` / `* RENAME`) | ✓ | ✗ | ✗ | ✗ |
  | `requires_schema_init` | ✓ | ✓ | ✓ | ✓ |
  | `null_safe_equality` (synthesised join spelling for a statement-level restructure) | `IS NOT DISTINCT FROM` | `<=>` | `<=>` | `IS NOT DISTINCT FROM` |

  This table is the **honest** matrix — `smelt:validate` / the conformance tests assert the code
  constructors (`BackendCapabilities::duckdb()`, `::spark_delta()`, `::spark_parquet()`,
  `::bigquery()`) match it. When a flag changes, this table changes in the same commit. A
  backend's column is established by executing the statement each flag names against a live
  instance of that backend, never by reading its documentation.

  Two flags in the table describe pipe-query emission. `supports_pipe_syntax` decides whether a
  `|>` query is emitted natively or lowered to standard SQL. `supports_pipe_set_drop_rename`
  qualifies the lowering: the lowered forms of `|> SET`, `|> DROP` and `|> RENAME` are built from
  the star-modifier trio `SELECT * REPLACE (…)` / `* EXCLUDE (…)` / `* RENAME (…)`, which only
  DuckDB accepts. A backend that lowers pipes but does not accept that trio (both Spark profiles)
  leaves those three stages unlowered rather than emitting SQL it would reject. BigQuery reads
  `✗` because GoogleSQL has `* EXCEPT` and `* REPLACE` but neither `* EXCLUDE` nor `* RENAME`;
  the value is not reached in practice, since BigQuery emits pipes natively.
- **`SPARK_CONNECT_URL`.** Spark integration tests connect to a Spark Connect server at this
  URL. When it is unset, Spark-targeted tests **skip** (not fail). The runtime backend reads
  `connect_url` from the target config (see `smelt_yml.md`).
- **`SMELT_BQ_PROJECT`.** BigQuery integration tests run against this GCP project. When it is
  unset, BigQuery-targeted tests **skip** (not fail), exactly as Spark's do. `SMELT_BQ_DATASET`
  and `SMELT_BQ_LOCATION` name the base dataset and its location; each test run creates and drops
  a uniquely-suffixed dataset beneath them, and that dataset carries a default table expiration so
  tables orphaned by an interrupted run are reclaimed without depending on teardown.
- **BigQuery authenticates from an explicit token.** The backend reads a short-lived OAuth access
  token from `SMELT_BQ_ACCESS_TOKEN` and **never** falls back to Google application-default
  credentials. This is a security property, not a convenience: ambient credentials on a developer
  machine carry that developer's whole cloud identity, so refusing the fallback makes the
  explicitly-supplied token the only route to the warehouse.

## Semantics

### Parity contract
The same smelt model, materialized on any backend, must produce the **same logical result**
(same rows, same column types up to the documented type-conformance rules in
`crates/smelt-dialect/src/type_conformance.rs`). A backend lacking a SQL feature does **not**
reject the model — the dialect printer **lowers** the logical construct to an equivalent
physical form the backend accepts. A capability flag set to `false` is an instruction to the
printer to lower, never a reason to emit invalid SQL or to surface a user-facing error.

**Supported-surface statement.** Multi-target parity covers: full-refresh table and view
materializations, ephemeral (CTE-inlined) models, and the `batched`/`keyed`/`versioned`
incremental maintenance legs — each exercised on both DuckDB and Spark by the same parametrized
CLI integration tests, plus the DuckDB-anchored `maintenance_conformance` suite (`smelt-cli`
crate) for the maintenance legs specifically. BigQuery joins the fixed-recipe suites
(materialization, seed, lowering, merge, incremental DELETE+INSERT, schema evolution) and, like
Spark, has its own leg of the generative dual-execution harness
(`maintenance_conformance_bigquery`, see §"Generative equivalence coverage"), so its incremental
coverage is generative rather than fixed-recipe-only. Every one of that leg's 21 cases passes
against the live warehouse, measured in a single uninterrupted sweep on 2026-08-21
(`bash scripts/bigquery-conformance.sh`: 22 passed / 0 failed / 0 ignored, 621.61s, measured
2026-08-22 at the default 4-way concurrency). `refresh: materialized_view` is covered separately and differently, because
its correctness is not smelt's to verify: BigQuery is the only backend advertising
`supports_native_ivm`, and for that mode smelt runs no combiner and keeps no ledger, so the
generative equivalence oracle has nothing to drive. What is verified instead is the *emission* —
`materialized_view_parity` asserts against a live warehouse that the object smelt creates is an
engine-owned `MATERIALIZED VIEW` and not a substituted table, and that an ineligible query is
refused with the engine's own reason. On the three backends without native IVM the mode
hard-errors, which is asserted offline. Databricks-specific behaviour
beyond what the generic Spark Connect adapter exercises is excluded (see §Known Divergences).

**Generative equivalence coverage.** The equivalence invariant
(`incremental_models.md` §"The equivalence invariant") is verified generatively — not just by
fixed-recipe parity tests — on every supported backend, via a single dual-execution harness that
owns the recipe pool, run schedules, and multiset-comparison oracle; the backend under test is a
parameter, not a duplicated implementation. The parameter is a `ConformanceTarget` naming the
backend a staged case runs against — DuckDB, Spark/Delta, or BigQuery (the last carrying the
dataset the case isolates in, derived rather than threaded so staging and read-back agree
without shared state) — which every staging/render/run entry point in the harness accepts, so
adding a backend widens the harness's target seam rather than duplicating it. The test families
themselves have a single owner: each is written once, target-generically, and a backend supplies
only what genuinely differs about it — the corruption statement its dialect accepts, the pacing
its rate limits require, how a case's target and schema are named — through declared hooks. A
family never branches on which backend it is running against; a family that did would be a
duplicated implementation wearing a parameter. On DuckDB this
runs per-PR as `cargo test -p smelt-cli --test maintenance_conformance`. On Spark this runs in
the gated tier (see "CI tiering" below) as `cargo test -p smelt-cli --features smelt-cli/spark
--test maintenance_conformance_spark`, with a reduced deterministic case count; rollout across
the recipe pool is tracked incrementally, with any leg still DuckDB-only recorded in §Known
Divergences until it lands.

**A comparison against a full-refresh oracle must reach two distinct stores.** Where a family
stages a case twice — an incremental project and a full-refresh oracle twin — the two must resolve
to different physical storage, and the harness expresses that through a declared seam
(`ConformanceBackend`'s `twin_target`/`twin_schema`) rather than a naming convention a caller is
trusted to honour. The requirement is not cosmetic: two projects sharing one store make the
comparison read a single table twice, so the assertion passes regardless of what the incremental
engine computed, and the twin's own source seeding lands on top of the incremental project's
rather than beside it. Backends satisfy this differently — DuckDB incidentally, since each staged
project owns a private database file; BigQuery and Spark explicitly, because both address one
shared store where only the dataset or schema separates two projects' tables. Because a
comparison that has gone vacuous is indistinguishable from a passing one, the property is asserted
rather than assumed: the paired family carries a self-check that seeds a divergence into the
incremental side after both builds and requires the comparison to refuse it. Runs on all three
legs (DuckDB, Spark, BigQuery).

**CI tiering.** Two tiers enforce the supported surface. A **per-PR tier** — gated on the PR's
changed paths touching Spark-relevant code (the Spark backend crate, Spark/parity integration
tests, the function-signature registry, type inference, the parser's dialect surface, or the
Python adapter) — runs `spark-parity` and `type-property-spark`. A **nightly tier** runs the
full Spark job set (including the corpus-driven `spark-integration` parser-compat job)
unconditionally, and is also reachable on demand via the `run-docker-tests` PR label. A Spark
regression outside the per-PR path filter still surfaces within one nightly cycle rather than
sitting unnoticed on `main` indefinitely.

BigQuery's fixed-recipe suites and its generative-conformance leg
(`maintenance_conformance_bigquery`) have neither tier: both run only when a developer executes
them by hand, via `scripts/bigquery-parity.sh` and `scripts/bigquery-conformance.sh`
respectively, against their own GCP project and a freshly minted token
(`scripts/bigquery-auth.sh`). This is not an oversight — it keeps cloud credentials, and the
short-lived credential window a BigQuery session runs under, out of CI entirely — but it means a
BigQuery regression does not surface on `main` on any schedule the way a Spark one does. See
§Known Divergences for the credential-window constraint this bounds.

### Inline row-set construction
Every production path that splices a small literal row set into generated SQL — an ephemeral
seed's CTE, a repair's affected-key list, an append-only baseline probe's recorded partitions, a
`smelt.test` mock dataset — renders it through a single dialect-aware owner
(`smelt_core::build_row_set_table` / `row_set_body`, `crates/smelt-core/src/sql/row_set.rs`)
rather than formatting `VALUES (…)` itself. DuckDB, Spark, and PostgreSQL accept a `VALUES (…),
(…)` table-value constructor directly, unchanged. GoogleSQL has none: `FROM (VALUES (1), (2))`
is a syntax error. The owner renders BigQuery's row set as the portable chained `SELECT … UNION
ALL SELECT …` instead — column names come from the first branch's aliased projections, since
that branch is the only one carrying them on a `UNION ALL` chain. Deciding what an *empty* row
set means (an always-false guard row, a `WHERE FALSE` predicate with no row at all, …) stays a
per-caller business decision, not a row-set construction detail — the owner requires at least one
row and callers handle the empty case themselves before reaching it.

### Exact-median lowering
`MEDIAN(x)` is an exact, interpolating median on every backend that executes it. DuckDB, Spark,
and PostgreSQL are emitted unchanged; GoogleSQL has no `MEDIAN` built-in, so the dialect printer
lowers it, and both lowerings are exact because an approximate substitute would make the
equivalence oracle report divergences that are artefacts of the substitution — or hide real ones.
In window position (`MEDIAN(x) OVER w`) the lowering is `PERCENTILE_CONT(x, 0.5) OVER w`, which
interpolates as DuckDB does; `PERCENTILE_DISC` picks a stored value instead and is therefore not
the equivalent. That lowering holds only where `w` is a whole-partition window, because GoogleSQL
forbids a window `ORDER BY` on `PERCENTILE_CONT`: a running `MEDIAN(x) OVER (PARTITION BY g ORDER
BY t)` has no exact GoogleSQL form and is refused (§"Statement-level lowering"). In aggregate position (`GROUP BY`) `PERCENTILE_CONT` cannot be used at all —
GoogleSQL makes it analytic-only — and `APPROX_QUANTILES`, the one aggregate offered, is
approximate. The lowering there sorts the argument into an array with `ARRAY_AGG(x IGNORE NULLS
ORDER BY x)` and indexes its middle element, averaging the two middle elements at even counts;
the array sub-expression is repeated rather than bound to a name because GoogleSQL rejects an
aggregate inside `UNNEST`. The aggregate form casts to `FLOAT64`, matching the numeric return
type; a temporal argument, which DuckDB's `MEDIAN` accepts, is refused by the backend rather than
silently coerced.

The *decision* that `MEDIAN` needs rewriting on BigQuery is registry data, stated once per
position: `MEDIAN`'s `BuiltinRegistry` entry carries `Emission::Rewrite(RewriteId::BigQueryMedian)`
at both `Position::Aggregate` and `Position::WholePartitionWindow`, and
`Emission::Unsupported` at `Position::Window`. The *shape* each rewrite emits stays printer code;
the registry names the rewrite and the position it applies to, the printer holds its logic. The
printer never infers position from the CST itself — position is the question the compile path asks
the registry, not an answer the printer derives (§"Statement-level lowering").

### Operator lowering
An infix operator smelt's grammar accepts but a backend's SQL does not is lowered by the dialect
printer, never emitted verbatim and left to the engine. Emission ownership for every operator is
data in `BuiltinRegistry` — the printer reads the `Emission` verdict for the active dialect and
dispatches on it; no name-matched dialect arm lives in `printer.rs`.

`^` is the critical case. In smelt's grammar, and in DuckDB and PostgreSQL, `^` means power —
a synonym for `**`. But **both GoogleSQL and Spark SQL** define infix `^` as **bitwise XOR**, so
emitting `^` verbatim against either backend silently returns a different number from what smelt's
semantics say. Both GoogleSQL and Spark therefore lower `^` (and `**`) to `POWER(a, b)`.

`%` (modulo) has no infix form in GoogleSQL at all, so an unlowered `a % b` is a syntax error
there; it lowers to `MOD(a, b)`.

`//` (floor division) is not lowered anywhere. DuckDB's `//` truncates toward zero when both
operands are integers but degrades to plain division the moment either is floating point, and the
printer carries no operand types with which to tell those cases apart. GoogleSQL has no infix `//`
and no safe universal substitute, so `//` is declared `Unsupported` on Spark, PostgreSQL, and
BigQuery — the compiler refuses it rather than emitting SQL the engine will reject or
misinterpret at runtime.

The `POWER` lowerings are exact: DuckDB's power operator returns a double for every operand type,
negative base, negative exponent, and `0 ^ 0 = 1` included, and `POWER` agrees on each. They
diverge only at `0 ^ -1`, where DuckDB yields infinity and GoogleSQL raises — a loud failure,
not a wrong answer.

### Emission is scoped to call position
A built-in's emission verdict is stated per `(dialect, position)`, not per dialect alone, because a
backend's support for a built-in routinely differs between the positions it can appear in. GoogleSQL
is the sharp case in both directions: `PERCENTILE_CONT` is refused under a `GROUP BY`
(`percentile_cont aggregate function is not supported`) but accepted with an `OVER` clause, while
`MAX_BY` is the exact reverse (`Aggregate function MAX_BY does not support an OVER clause`).

Four positions are probed, and a fifth key, `Any`, is a wildcard used only for lookup:

| Position | The call's context | Probe shape |
|---|---|---|
| `Scalar` | a row-wise expression | `SELECT <expr> AS a FROM fixture` |
| `Aggregate` | the call itself is an aggregate, with no `OVER` | `SELECT g, <expr> AS a FROM fixture GROUP BY g` |
| `WholePartitionWindow` | `OVER w` where `w` covers its whole partition | `SELECT <expr> OVER (PARTITION BY g) AS a FROM fixture` |
| `Window` | `OVER w` with any narrower frame | `SELECT <expr> OVER (PARTITION BY g ORDER BY rid) AS a FROM fixture` |

**Lookup consults the call's own position, then `Any`, and stops.** There is deliberately no
fallback *between* positions, and in particular none between the two window keys, because such a
fallback is wrong in both directions. Falling from `WholePartitionWindow` to `Window` would refuse
`MEDIAN(x) OVER (PARTITION BY g)` on Spark — the very call the restructure exists to serve — on the
strength of a verdict about running windows. Falling from `Window` to `Any` would let a running
`MAX_BY(x, t) OVER (PARTITION BY g ORDER BY t)` reach BigQuery as `Native` and fail at the
warehouse, which is what §"Compile-path refusal" promises can never happen.

Because there is no fallback, an entry that declares a verdict at one window position **must**
declare one at the other. That obligation is checked, not assumed: the coverage-totality gate fails
an entry carrying a `WholePartitionWindow` verdict and no `Window` verdict, naming the entry and
the dialect. An entry listing no verdict at all for a dialect is `Native` everywhere, so the
majority of the registry states one row and means it in every position.

Position is decided once, by the compile path, from the source CST, and handed to the registry. The
printer never re-derives it: a printer that inspected sibling nodes to tell aggregate position from
window position would hold emission knowledge the registry owns.

**Deciding whether a window is whole-partition.** A window is whole-partition when, *after resolving
any named-window reference*, it has no window `ORDER BY` and no frame clause, or carries an explicit
`BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING` frame **with no `EXCLUDE` clause**. Every other
window is running, including the common `ORDER BY` with no explicit frame, whose SQL default frame is
`RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW`.

Two spellings make this decision impossible to take at the call site, and both are real:

- **`EXCLUDE` changes the answer per row.** `ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING
  EXCLUDE CURRENT ROW` matches the unbounded-frame wording but is not whole-partition: on DuckDB,
  `SUM(x)` over rows `1,2,3` returns `5, 4, 3` — a distinct value per row. A classifier that ignored
  `EXCLUDE` would collapse three answers into one, silently.
- **A named window hides the frame.** `AGG(x) OVER w` carries no `ORDER BY` and no frame at its own
  site, so a purely local rule classifies every such call as whole-partition. With
  `WINDOW w AS (PARTITION BY g ORDER BY t)` DuckDB returns `1, 3` — running. Classification therefore
  resolves the `WINDOW` clause first, and a reference that cannot be resolved (or a window-spec
  inheritance such as `OVER (w ORDER BY t)` whose base is unresolved) is treated as running, never as
  whole-partition. Refusing is the safe direction: it costs a diagnostic, where guessing costs a
  wrong number.

### Statement-level lowering
Some built-ins cannot be lowered by substituting one expression for another, because the backend
offers the operation only in the *other* position from the one the author wrote. Two shapes recur,
and both are lowered by restructuring the statement around a synthesised CTE.

**Admissible shapes are enumerated, and everything else is refused.** A statement-level lowering
rewrites a whole query block, so — unlike an expression rewrite — it can be defeated by parts of the
block it does not touch. The restructure therefore applies only to a query block where all of the
following hold, and refuses with `UnsupportedOnBackend` otherwise:

1. The grouping is a plain `GROUP BY` over column references or expressions — not `ROLLUP`, `CUBE`,
   or `GROUPING SETS`. Those compute super-aggregate rows that no `PARTITION BY` produces: for
   `t = {(g=1,x=1),(g=2,x=100)}`, `GROUP BY ROLLUP(g)` owes a total row of `50.5`, where the
   partitioned form yields whichever single group's value `ANY_VALUE` reaches.
2. Every occurrence of the affected built-in is in the select list. An occurrence in `HAVING`,
   the query's `ORDER BY`, or `QUALIFY` is refused, because leaving it in place would ship a
   statement still containing the construct the lowering exists to remove.
3. The call carries no `DISTINCT` and no `FILTER (WHERE …)`. Neither has an analytic form on any
   supported backend, so neither survives a move into window position.
4. The select list has no unexpanded wildcard. `SELECT *` would otherwise expand against the
   *restructured* `FROM` and pick up the synthesised columns.

**An analytic-only built-in in aggregate position.** GoogleSQL's `PERCENTILE_CONT` and
`PERCENTILE_DISC` require an `OVER` clause and cannot appear under a `GROUP BY` at all. smelt spells
these as ordered-set aggregates, so the lowering is a change of *call shape* as well as of position —
GoogleSQL rejects `WITHIN GROUP` outright. The query's `FROM` and `WHERE` move into a CTE that adds
the value as an analytic column over the grouping keys, and the outer query reads it back:

```sql
-- smelt
SELECT g, COUNT(*) AS n, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS med
FROM t WHERE ok GROUP BY g

-- GoogleSQL
WITH __smelt_r0 AS (
  SELECT g, PERCENTILE_CONT(x, 0.5) OVER (PARTITION BY g) AS v FROM t WHERE ok
)
SELECT g, COUNT(*) AS n, ANY_VALUE(v) AS med FROM __smelt_r0 GROUP BY g
```

The `WITHIN GROUP (ORDER BY …)` sort key becomes the analytic form's first argument. A `DESC` sort
key inverts the fraction — `PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x DESC)` is
`PERCENTILE_CONT(x, 1 - 0.5)` — and a `NULLS FIRST`/`NULLS LAST` modifier that the target's analytic
form cannot express is refused rather than dropped.

`v` is constant within each `g`, so `ANY_VALUE` reads that constant exactly rather than sampling.
Sibling aggregates such as `COUNT(*)` are untouched.

**The same built-in over a whole-partition window is not a restructure at all.** GoogleSQL accepts
`PERCENTILE_CONT`/`PERCENTILE_DISC` with a partition-only `OVER` clause natively, in their
two-argument analytic spelling — the call is already in the right position, and only its *shape*
needs to change, from the ordered-set spelling to the analytic one:

```sql
-- smelt
SELECT g, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) AS med FROM t

-- GoogleSQL
SELECT g, PERCENTILE_CONT(x, 0.5) OVER (PARTITION BY g) AS med FROM t
```

Because the window is already in place, this is an in-place expression rewrite planned from the
source CST like any other `Emission::Rewrite`, not a statement-level `Emission::Restructure`: no CTE
is synthesised, and the call's own `OVER` clause prints unchanged. The same `DESC`-inverts-the-
fraction and `NULLS FIRST`/`NULLS LAST`-is-refused rules apply, and are shared with the
aggregate-position lowering above rather than restated.

**An aggregate-only built-in in window position.** GoogleSQL has `MAX_BY`/`MIN_BY` and
`APPROX_COUNT_DISTINCT` as aggregates with no analytic form at all — refused with an `OVER`
clause even when the window is partition-only — and DuckDB and Spark have the ordered-set
`PERCENTILE_CONT`/`PERCENTILE_DISC` with no window form. `APPROX_COUNT_DISTINCT` is the sharp case:
GoogleSQL's own dry run accepts the analytic spelling over a partition-only window, and only
execution refuses it, so a schema/dry-run leg alone cannot see this gap. The lowering binds the
source once, groups it by the partition keys, and joins the result back:

```sql
-- smelt
SELECT id, g, ARG_MAX(x, t) OVER (PARTITION BY g) AS best FROM tbl WHERE ok

-- GoogleSQL
WITH __smelt_base AS (SELECT id, g, x, t FROM tbl WHERE ok),
     __smelt_w0   AS (SELECT g, MAX_BY(x, t) AS v FROM __smelt_base GROUP BY g)
SELECT b.id, b.g, w.v AS best
FROM __smelt_base b JOIN __smelt_w0 w ON b.g IS NOT DISTINCT FROM w.g
```

Four details are load-bearing:

- **`WHERE` goes inside the bound source, never on the join.** Window functions are evaluated after
  `WHERE`, so a predicate left outside would let `__smelt_w0` aggregate rows the original query had
  already discarded — returning the `x` of a filtered-out row that happened to hold the maximum `t`.
- **The source is bound once.** Repeating the `FROM` in both branches would evaluate it twice, which
  is wrong for any non-deterministic source and wasteful for every other one. A `PARTITION BY` over
  an *expression* is grouped and joined on that expression, so a non-deterministic partition key is
  refused rather than evaluated twice.
- **The join is null-safe.** `GROUP BY g` places NULL keys in their own group, but `ON b.g = w.g`
  never matches NULL, so a plain equi-join silently drops every row whose partition key is NULL —
  measured on BigQuery as 3 rows kept out of 5. The null-safe comparison is spelled
  `IS NOT DISTINCT FROM` on DuckDB, PostgreSQL and GoogleSQL and `<=>` on Spark SQL; the difference
  is a `BackendCapabilities` spelling, never a dialect arm in the printer.
- **The join is total, so it is an inner join.** `__smelt_w0` is `__smelt_base` grouped on the same
  keys, so every base row has exactly one match by construction. Floating-point keys do not break
  this: GoogleSQL groups NaN keys together *and* treats two NaNs as not-distinct, so the two halves
  agree. A window with no `PARTITION BY` degenerates to a one-row CTE and a `CROSS JOIN`.

This lowering computes one value per partition, so it is admissible **only** at
`Position::WholePartitionWindow`. A running window over a built-in with no analytic form on the
target has no correct CTE form — a per-row correlated subquery would be a different construct with
different cost — and is refused at compile time with `UnsupportedOnBackend`, naming the built-in, the
backend, and the requirement that the window be whole-partition. The registry states that refusal as
an ordinary `Position::Window` verdict; nothing about it is special-cased.

**Mechanics that the shapes above depend on.** A synthesised CTE is *appended to the author's `WITH`
list* rather than prefixed to the statement, so a model that already begins `WITH a AS (…)` stays
valid and the synthesised body may reference the author's bindings. Synthesised names carry the
`__smelt_` prefix that is reserved from author identifiers. Base-table references in the outer select
are qualified to the bound source's alias. Several decorrelated windows with different `PARTITION BY`
keys yield one grouped CTE and one join each, over the same single bound source. The restructure
applies to one query block: an affected call inside an author-written CTE or a `FROM` subquery
restructures *that* block, and a correlated subquery whose block would need a hoisted CTE is refused.

Restructuring happens on the source CST, before printing, and rewrites only the expression behind a
select item and the query's `FROM` — never a select item's name. A model's output column names and
types are therefore unchanged by it, as §"Output-schema type conformance" requires; admissibility
rule 4 is what makes that claim hold in the presence of `SELECT *`.

### Cross-engine emission audit

Two complementary legs verify what the registry declares:

- **Schema leg** — for each `(entry, dialect)` pair, the probe is compiled and sent to the
  dialect's oracle (DuckDB prepare, Spark `DESCRIBE QUERY`, BigQuery dry run). The oracle returns
  the output schema; the leg asserts acceptance and compares smelt's inferred type against the
  oracle's report using the existing `compare_types`/`divergences` machinery. Acceptance alone
  catches every missing lowering and every `Unsupported` entry.
- **Value leg** — the same probe is executed on the target dialect and on DuckDB (the reference);
  rows are compared using a typed comparator (exact for integers, strings, booleans; relative
  tolerance for floats; scale-normalised for decimals; NULL equals NULL; deterministic `ORDER BY`).
  This is the leg that catches the `^` class of silent semantic divergence.

**Probes are derived from registry data, not authored by hand.** `SyntaxForm` determines the
spelling (`a % b` versus `MOD(a, b)`); `kind` determines which positions apply. A small override
table covers the minority where a type-correct argument is not a meaningful one — regex patterns,
date-part strings, JSON paths. Aggregates are probed in every position they can occupy — including
both window positions separately — because the emission verdict is scoped to position
(§"Emission is scoped to call position") and a suite that probed only one of them would leave the
other's claim untested. The probe positions are the registry's four call positions exactly
(`Any` is a lookup wildcard, never a position a call occupies), so the audit maintains no axis of
its own.

**The fixture** is a single inline `VALUES` CTE — approximately eight rows, one typed column per
`TypeConstraint` family — with NULL-bearing rows. No DDL, no cleanup, no materialised objects.
The same fixture serves a BigQuery dry run and a real execution.

**Ledger verdicts** — `dialect_divergences.rs` records one row per `(entry, dialect)` when a pair
does not pass both legs cleanly:

| Verdict | Meaning | Fails? |
|---|---|---|
| `Divergent { reason }` | Accepted and permanent (e.g. Spark integer-division semantics) | No — reported as a semantic difference users must know about |
| `Gap { issue }` | A lowering we owe, with a tracking issue | No — but the count ratchets down only |
| `SchemaOnly { reason }` | Nondeterministic entry (`RANDOM`, `NOW`, `CURRENT_DATE`, `UUID`) | No — value leg skipped, reason recorded |
| absent | Must pass both legs | Yes |

The ledger is two-sided: an unregistered mismatch fails loudly, and so does an unreachable row —
an entry naming a pair that no longer diverges is an error telling you to delete it.

**A position split is a joint change.** Adding a probe position and the lowering that serves it land
together. Introducing `WholePartitionWindow` alone would newly probe pairs that today carry a
`Position::Window` ledger row describing an engine with *no* window form — DuckDB's and Spark's
ordered-set percentiles, BigQuery's `MAX_BY`/`MIN_BY` and `APPROX_COUNT_DISTINCT` — and every one
would fail the new position as an unregistered mismatch. With the restructure (or, for BigQuery's
ordered-set percentiles, the in-place analytic rewrite) in place they pass it instead, and their
existing rows narrow to the running-window case rather than being deleted.

**Coverage table** — the suite emits a standing table to `docs/reference/dialect-coverage.md`:
entry × dialect → native / rename / rewrite / restructure / unsupported / divergent / gap. A cell
holds one verdict per position where an entry's positions differ, rendered as the set rather than
collapsed to a single value, because collapsing would hide exactly the aggregate/window asymmetry
the position axis exists to record. The table is derived
from registry data and ledger verdicts alone, so it is deterministic and gateable per-PR. The legs
*test the claims the table makes* rather than producing it — a mismatch between a registry verdict
and what the oracle observes fails the suite. A doc-sync gate fails when the generated table
diverges from the checked-in file.

**Gates, by tier:**

| Gate | Needs a warehouse? | Tier |
|---|---|---|
| Coverage totality — every entry × dialect has a verdict; every probe derivable or overridden | no | per-PR |
| Printer/registry consistency — no name-matched dialect arms remain in `printer.rs` | no | per-PR |
| Schema + value legs, DuckDB | no (in-memory) | per-PR |
| Schema + value legs, Spark | Spark Connect | labeled PR + nightly |
| Schema + value legs, BigQuery | live BigQuery | manual sweep, `scripts/bigquery-dialect-audit.sh`, gated on `SMELT_BQ_PROJECT` |

BigQuery remains manual, consistent with §"BigQuery has no CI tier, by decision, not by omission".

### Output-schema type conformance
Where a backend's native return type for an expression differs from smelt's inferred type, a
model's **output columns** are reconciled to the inferred type: the compiled SQL is wrapped in an
outer `SELECT CAST(col AS <inferred>) AS col, …` over the model body
(`type_conformance.rs::wrap_with_type_casts`, applied to every backend at compile time —
`smelt-runtime/src/compile.rs`). A model therefore writes the **same schema to every warehouse**,
regardless of engine. This is the multi-backend instance of the canonical-return-type rule in
`functions.md` §"Canonical return types are CAST-enforced"; the backend namespace
(`spark.ceil(...)`, `postgres.sum(...)`) is the explicit per-call opt-out that inherits the
engine-native type and marks the model non-portable.

The column names and inferred types the cast wrap uses are derived from the model's **source**
select list — the CST as written, before dialect lowering. The dialect printer's rendered output
is never re-read to recover a projection: a backend-lowered expression (a BigQuery `MEDIAN`
rewritten to an `ARRAY_AGG`-indexing form, `%` rewritten to `MOD()`, and so on) does not parse
back as the SQL smelt's own grammar accepts, so reconstructing names or types from it is not a
source of truth smelt can rely on. The projection is derived once, from the pre-print CST, and
every consumer — the cast wrap and the output column list alike — reads that single derivation.

Each top-level select item resolves to an output name by one rule, applied in order:

1. An explicit alias is used unchanged.
2. Absent an alias, if the item's inferred name is a valid bare identifier — a bare or qualified
   column reference, or a `CAST` of one — that name is used. Every dialect agrees on this name,
   so nothing is synthesized.
3. Otherwise (a function call, an arithmetic expression, a literal, a `CASE`, …), the name
   `_smelt_col{n}` is synthesized, where `n` is the item's 1-based position in the select list,
   and bound to the item as a real alias rather than merely inferred at reference time.

The `_smelt_` prefix is reserved for smelt's own generated identifiers. A user-written projection
alias beginning with `_smelt_` is a diagnostic, which is what makes a synthesized `_smelt_col{n}`
name collision-free.

Expressions whose smelt-inferred (DuckDB-canonical) type diverges from Spark's native type, all
reconciled at the output boundary by the cast wrap: `CEIL`/`FLOOR(Double)` (Spark native BigInt),
`AVG(Decimal)` (Spark native Decimal), `SIGN(x)` (Spark native Double/Integer/BigInt/Decimal). The
full registry is `crates/smelt-db/tests/prop_helpers/divergences.rs`; output conformance is
asserted by `crates/smelt-db/tests/proptests/type_conformance_tests.rs`.

Required lowerings when the corresponding flag is `false` (non-exhaustive; the conformance
suite is the executable list):

- `supports_qualify = false` → wrap the windowed query in a subquery and move the QUALIFY
  predicate to an outer `WHERE`.
- `supports_date_literal = false` → emit `to_date('YYYY-MM-DD')` (or `CAST('…' AS DATE)`)
  instead of `DATE 'YYYY-MM-DD'`.
- `supports_double_colon_cast = false` → emit `CAST(x AS T)` instead of `x::T`.
- `supports_array_literal = false` → emit `ARRAY(a, b, c)` instead of `[a, b, c]`.
- `supports_trailing_commas = false` → never emit a trailing comma in a select/grouping list.
- `supports_create_or_replace_table = false` → emulate via `DROP TABLE IF EXISTS` + `CREATE
  TABLE` (the Spark backend already does this).
- `supports_insert_overwrite = false` → emulate via range `DELETE` + `INSERT` (DuckDB).
- `supports_native_ivm = false` → `refresh: materialized_view` is a **hard error**, *not* a lowering.
  This is the one carve-out from lower-don't-reject: `refresh: materialized_view` is a declared
  commitment to engine-owned freshness (`materialized_view.md`), so substituting a smelt-driven or
  full-refresh table would swap the declared contract. Every other refresh mode (`batched`,
  `keyed`, `versioned`) is smelt-driven and needs no backend IVM. No backend
  today advertises native IVM — DuckDB and both Spark profiles set the flag `false`, so
  `refresh: materialized_view` currently always errors; native IVM would be a Databricks-only
  capability (Enzyme).

### Incremental-view-maintenance capabilities
Two flags describe a backend's participation in maintaining a keyed refresh mode's state.

- **`supports_native_ivm`** — the backend can maintain a declared query as a **native incremental view** (BigQuery materialized views, Databricks Enzyme, Snowflake Dynamic Tables). It gates the `refresh: materialized_view` mode: `true` → smelt emits the native maintained object and the engine owns freshness; `false` → the hard error above. `true` on BigQuery, `false` on DuckDB and both Spark profiles, where it is a warehouse fact: no native IVM runtime exists to delegate to. Because the flag states what *smelt emits* for a backend, not merely what the engine could support, a backend whose engine has IVM still reads `false` until the emission exists — the flag is never a claim about the warehouse alone. It is *not* consulted for the smelt-driven keyed modes (`keyed`, `versioned`), which maintain their own state with `merge_into` + views on any backend.
- **`supports_retraction`** — whether the backend's native IVM can **invert** a contribution (delete / reprocess a prior input). Meaningful only alongside `supports_native_ivm`, and `false` even on BigQuery: its materialized views do not invert a prior contribution, and a retraction-shaped query is refused at creation rather than maintained. It does **not** describe smelt-driven retraction: whether a `keyed` model can retract is a *per-model* property of its column families' algebra (the group rung, `incremental_shapes.md` §"The maintenance boundary"), derived from the SQL, not a blanket backend flag.

### Column-scoped merge and conditional-write capabilities

Four flags describe a backend's participation in the targeted-write and conditional-write
transforms (`model_transforms.md` §"Generic column-scoped merge", §"Change-suppressed MERGE and
the staged-candidate conditional DELETE+INSERT"). Like every capability flag, admission consults
the struct directly — a plan cell whose chosen technique needs a flag the target backend does not
set is never offered that technique, at plan time, not surfaced as a runtime error.

- **`supports_column_scoped_merge`** — the backend can execute a `MERGE`/`UPDATE ... FROM`
  restricted to one mutation-sensitivity column-group's columns against a source projection that
  carries the full target row (recomputing only the group's columns, passing every other column
  through unchanged from existing state). Gates the generic column-scoped merge transform and, by
  extension, the dimension-driven horizon-bounded MERGE and the keyed column-scoped-`MERGE` half
  of definition-change field-backfill.
- **`supports_merge_not_matched_by_source`** — the backend's `MERGE` dialect exposes a `WHEN NOT
  MATCHED BY SOURCE` clause, so a region-scoped change-suppressed MERGE can delete departed rows
  in the same statement. `false` does not refuse the change-suppressed MERGE transform; it
  changes its lowering — the departed-row delete is emitted as a separate scoped `DELETE`
  statement inside the same statement group instead of a `MERGE` clause (the dialect split the
  transform's licence names).
- **`supports_staged_relation_group`** — the backend can execute a statement group built around a
  named temporary relation (`CREATE` the staged relation, populate it, run dependent statements
  against it, `DROP` it), transactional as a unit. Gates the staged-candidate conditional
  DELETE+INSERT — the merge-less realisation of change-suppressed writes, and the only conditional
  write path available to a backend with `supports_merge = false` (Spark-over-Parquet).

These flags live in `BackendCapabilities` itself, queried by admission exactly like every other
capability flag above — never re-derived by a consumer. `supports_column_scoped_merge` is a
struct field; `supports_merge_not_matched_by_source` and `supports_staged_relation_group` are
specified ahead of their own struct fields (see §Known Divergences).

### Whole-row MERGE

The column-scoped merge and keyed-fold emitters upsert a whole row, and the two SQL families
spell that differently. DuckDB and Spark accept `WHEN MATCHED THEN UPDATE SET *` and `WHEN NOT
MATCHED THEN INSERT *`, which name no columns. GoogleSQL accepts neither: `SET *` is a syntax
error, and the whole-row insert is spelled `INSERT ROW`. So BigQuery's matched arm is rendered
column by column, `c = source.c`, over the model's output projection.

That projection is carried on the compiled model (`CompiledModel::output_columns`), derived from
the model's source select list using the same notion of an output column name the analyzer's
`model_schema` query uses — so the build path and the editor agree on what a model's columns are.
It is **inert** wherever a star form exists: passing it never perturbs DuckDB's or Spark's
emitted text.

Where the projection is not statically resolvable — a surviving wildcard, an unnamed select item
— the list is empty, and empty means *unknown*, never *no columns*. A backend needing the list
refuses at that point rather than emitting a `MERGE` whose matched arm assigns nothing, which
would silently stop updating matched rows. The keyed folds are exempt: their matched arm is
already an explicit `SET` list of fold expressions, so only their not-matched arm varies by
dialect and no column list is needed.

### Session initialization
Before any model executes, a backend's session must be usable against a target schema that may
not exist yet (first run against a fresh warehouse). When `requires_schema_init = true`, the
backend **creates the target schema** (`CREATE SCHEMA IF NOT EXISTS` / `CREATE DATABASE IF NOT
EXISTS <catalog>.<schema>`) during session init, **before** it selects the current
schema/database and before the first model runs. Selecting a non-existent schema must never be
the first statement a fresh session issues — on backends whose `setCurrentDatabase`/`USE` hard-
fails for a missing schema (Spark Connect raises `[SCHEMA_NOT_FOUND]`), that ordering bug blocks
every model on first run. The flag is `true` for every backend today; the conformance suite
asserts each constructor sets it and that a first-run model against a fresh schema succeeds.

### Connection security
A backend target's connection string may need secrets (an auth token) or TLS parameters that
must not live in the checked-in `smelt.yml`. These are carried as `${ENV_VAR}` references inside
the `connect_url` string, resolved by the config-load interpolation pass (`smelt_yml.md`
§"Environment interpolation") — this spec adds no second interpolation mechanism. Token and TLS
settings are passed as Spark Connect URL parameters, never as new YAML keys:

```yaml
targets:
  databricks:
    type: spark
    connect_url: "sc://host:443/;token=${DATABRICKS_TOKEN};use_ssl=true"
```

The interpolated URL — token and all — passes to the Spark Connect Python client
(`builder.remote(connect_url)`) unmodified; smelt never parses out or stores the token
separately, and never logs the resolved URL. A `connect_url` holding a literal (non-`${VAR}`)
token is a lint-worthy smell: the secret sits in the committed YAML in plaintext, exactly what
the interpolation mechanism exists to prevent.

### Loading data into a backend
Loading external rows into a backend (seeds, test fixtures, an Arrow batch) must not assume the
backend's process shares the host filesystem. The transfer is performed through the backend's
own client API — for Spark Connect, the rows are sent as an in-memory frame
(`createDataFrame` from Arrow), **not** by writing a host-path file and asking the server to
read it back (`spark.read.parquet('/tmp/…')`), which fails with `PATH_NOT_FOUND` against any
containerized or remote Connect server whose JVM cannot see the host path. This is distinct from
cross-engine *exchange* below, where the shared `warehouse` filesystem is an explicit
requirement; data **loading** carries no such assumption.

### Cross-engine data exchange
When a model on backend A references a model pinned to backend B (a cross-backend edge, found
by `DependencyGraph::find_cross_backend_edges()`), smelt resolves the reference to a
file-format read against B's materialized output rather than a three-part table name. Today
the only path is **Spark → DuckDB**: a DuckDB model referencing a Spark model compiles the
reference to `read_parquet('{warehouse}/{schema}.db/{model}/**/*.parquet', hive_partitioning =
true)`. The `.db` suffix reflects the Hive metastore directory convention that Spark SQL uses
when `spark.sql.warehouse.dir` is set (empirically verified on Spark 4.1.x). This requires the
referenced Spark model to be `materialization: table` and the Spark target to declare a
`warehouse` path that is on a filesystem the DuckDB process can also read.
No explicit copy step exists; Spark writes Parquet, DuckDB reads it natively.

### Incremental & schema evolution per backend
Strategy *resolution* (`incremental_models.md`) and change *classification*
(`schema_evolution.md`) consult the capability matrix but are specified in those documents.
This spec only requires that the resolved strategy and migration plan are expressible in the
target backend's physical SQL via the lowering rules above — e.g. a backend without native
`INSERT OVERWRITE` resolves to `DeleteInsert`; a backend without `ALTER COLUMN … USING`
resolves nested widening to a table rewrite.

## Design

- **Capabilities are data, not branches.** Centralizing backend differences in one
  `BackendCapabilities` value (rather than scattering `if dialect == Spark` across the printer)
  keeps the parity contract auditable: the conformance suite can enumerate every flag, and the
  matrix table above is the single source of truth a reviewer checks. Rejected: per-call-site
  dialect checks — they make "what does Spark support?" unanswerable without reading the whole
  printer.
- **Lower, don't reject.** Treating a missing capability as a printer-lowering obligation (not
  a diagnostic) is what makes "the same model runs everywhere" true. A user writing `QUALIFY`
  should not need to know their target lacks it. Rejected: surfacing a "Spark does not support
  QUALIFY" diagnostic — it would push backend physics into the user's logical model, violating
  the logical/physical separation that is smelt's reason to exist.
- **Verification-first parity.** Parity is asserted by a **multi-target test matrix** (the same
  CLI integration tests parametrized over `{DuckDb, Spark, BigQuery}`) plus a
  capability-conformance suite, run against a real Spark Connect server and a real BigQuery
  project. A capability the code claims but no test exercises against a live backend is treated
  as unverified. Rejected: trusting the capability constructors without live execution — the
  whole motivation here is that unverified Spark code had drifted from reality.
- **One target list, not per-suite target lists.** A suite enumerates its targets through the
  shared `targets_to_run(label)` harness rather than hard-coding a pair, so adding a backend
  reaches every suite in one edit and the compiler names each suite that has not yet handled it.
  The label scopes BigQuery's dataset, which is *derived* from `(base, label, pid)` rather than
  minted and threaded through the test: staging and assertion compute the same name
  independently. Rejected: minting a unique dataset per run and passing it around — it forces
  every suite that hand-writes its `smelt.yml` to also plumb state into its assertion loop.
  Rejected too: one shared dataset for all suites — BigQuery's per-table modification quota
  binds on repeated writes to a single table name, so suites must not share target tables.
- **Delta as the parity baseline.** Delta is the Spark default because MERGE, column mapping,
  and rich schema evolution — the features that bring Spark to DuckDB parity — require it.
  Parquet format is a documented, reduced-capability profile, not the parity target.
- **Spark Connect, not embedded JVM.** The Connect client is pure-gRPC Python, so the host
  needs no JVM and the server version is isolated in a container. This matches the existing
  type-oracle container and the `SPARK_CONNECT_URL` test gating. Rejected: an embedded local
  JVM, which couples parity testing to the host's Java version.

## Constraints & Invariants

- **The capability matrix table in §Surface and the `BackendCapabilities` constructors agree.**
  A conformance test asserts each flag of `::duckdb()`, `::spark_delta()`, `::spark_parquet()`
  equals the table. Changing one without the other is a spec-vs-code drift the conformance test
  must fail on.
- **A `false` capability never reaches the user as a diagnostic.** Every `false` flag has a
  corresponding printer lowering; emitting invalid physical SQL for a `false` flag is a bug.
- **Default `cargo test` is backend-agnostic.** With `SPARK_CONNECT_URL` unset, every
  Spark-targeted test skips; the suite stays green without Spark installed. Spark coverage runs
  only in the gated job that provides the server.
- **BigQuery has no CI tier, by decision, not by omission.** Spark parity runs per-PR on changed
  paths and nightly in full; BigQuery's fixed-recipe suites and its generative-conformance leg
  run only when a developer executes them by hand (`scripts/bigquery-parity.sh`,
  `scripts/bigquery-conformance.sh`), gated on `SMELT_BQ_PROJECT`. This is deliberate: it keeps
  cloud credentials, and the short-lived credential window a BigQuery session runs under, out of
  CI entirely. The cost is real — a BigQuery regression surfaces only when someone runs a sweep,
  never on a schedule — and is accepted rather than resolved: adding a tier needs a service
  account, a GitHub secret, and a recurring billing commitment (a green conformance sweep alone
  runs ~37 minutes of warehouse time), none of which this spec can decide unilaterally. A claim
  of Spark-equivalent BigQuery coverage is therefore a claim about which gates exist, never about
  when they run.
- **Cross-engine exchange is a two-engine, filesystem-local capability by design.**
  `cross_engine_parity`/`cross_engine_types_parity` hand off through a shared local Parquet
  file. A third engine that cannot read a host path (BigQuery, or any object-store-only engine)
  needs a new exchange boundary — remote object stores (S3/GCS/ADLS) — not a mirrored leg of the
  existing loop, and that boundary is a cross-cutting change to the exchange design, never a
  per-backend patch. It stays out of scope until a concrete consumer demands cross-engine
  exchange with such an engine; it is not part of BigQuery backend completion.
- **Data loading carries no host-filesystem assumption.** A backend's load path (seeds, test
  fixtures, Arrow batches) must transfer rows through the backend client API, never via a host
  path the server is asked to read. A load path that only works when the server shares the host
  filesystem is a bug, not a deployment constraint (see §"Loading data into a backend").
- **No new logical surface per backend.** Backends may differ in physical SQL and capability
  flags only; the set of writable smelt models is backend-independent.
- **A capability flag advertising a *path* carries live coverage of that path.** Asserting a
  flag's value proves only that the matrix is accurate; it says nothing about the emission the
  flag selects. `supports_pipe_syntax` is the case in point: BigQuery is the only backend
  reporting `true`, so it is the only backend whose printer emits `|>` rather than lowering, and
  a pipe query runs through `pipe_parity` on a live warehouse and must produce the same rows the
  lowered form produces on DuckDB. The offline half of that pair (`smelt-dialect`'s
  `pipe_native`) pins that BigQuery is sent pipes at all — without it the live leg would keep
  passing on lowered SQL, which GoogleSQL also accepts, and prove nothing about the native path.
- **Delegated maintenance is emitted, never simulated.** Where `supports_native_ivm` is `true`,
  `refresh: materialized_view` resolves to the engine's own maintained object —
  `CREATE OR REPLACE MATERIALIZED VIEW` on BigQuery, carrying no `OPTIONS` clause, so the engine's
  default refresh behaviour is what owns freshness. smelt runs no combiner and writes no
  reconciliation ledger for these models, and the equivalence invariant is discharged by the
  engine rather than by smelt's generative oracle (`materialized_view.md` §Constraints item 4).
  Two consequences are load-bearing. First, substituting an ordinary table would serve *identical
  rows*, so the live leg asserts the created object's **type**, not its contents — row equality
  alone would go green against exactly the silent fallback §"No silent fallback" forbids. Second,
  eligibility is the engine's verdict alone: an unsupported query shape is refused with
  BigQuery's own message relayed verbatim, never pre-empted by a smelt-side check and never
  quietly downgraded to a table.
- **A BigQuery `ColumnScopedMerge` model must have a statically enumerable projection.**
  GoogleSQL has no `UPDATE SET *`, so the whole-row `MERGE` renders its matched arm column by
  column over the model's output projection (§"Whole-row MERGE"). Where that projection is not
  statically enumerable — a surviving wildcard, an unnamed select item — the column list is
  empty and the run is refused with an error naming the model, rather than emitting a matched
  arm that assigns nothing and silently stops updating rows. DuckDB and Spark are unaffected:
  their `UPDATE SET *` needs no list. Making every model's output schema knowable (ROADMAP
  "Total Output-Schema Resolution") would narrow this to genuinely unresolvable upstreams, not
  retire it.
- **A built-in's per-dialect spelling derives from `BuiltinRegistry`; `printer.rs` holds no
  name-matched dialect arm.** Recognition, lowering decision, rewrite dispatch and restructure
  dispatch all flow from `BuiltinRegistry::emission_at(dialect, position)`. A dialect arm keyed on
  a function name is a violation of single ownership (§"Function-registry single ownership" in
  `architecture.md` §Constraints #14), and so is a printer that derives a call's position for
  itself. There is deliberately no position-blind lookup: a caller that could ask for a dialect's
  verdict without naming a position could silently get the wrong one for the position it is in.
  Gate: `cargo test -p smelt-dialect --test emission_ownership`.
- **A statement-level lowering is planned before printing and never re-parses printed SQL.**
  The restructure plan is a pure function of the source CST and the registry; the printer consumes
  it. Recovering the plan — or a model's projection — from the dialect-printed string is forbidden,
  because a backend's own lowering does not parse back as smelt SQL
  (§"Output-schema type conformance"). Gates: `cargo test -p smelt-runtime --test
  projection_dialect_invariance` pins that a decorrelated model's output columns are byte-identical
  across every dialect; `cargo test -p smelt-runtime --test dialect_seam` pins that a running
  window over a built-in with no analytic form on the target is refused at compile time rather than
  emitted.
- **Every `RestructureId` is dispatched, and every restructure preserves row multiplicity.**
  A synthesised CTE join must not add or drop rows: the grouped branch is derived from the bound
  source on the same keys, and the comparison is null-safe, so the join is total and one-to-one.
  An equi-join on a nullable partition key is the failure this rules out — it type-checks, runs,
  and silently drops rows. Gates: `cargo test -p smelt-dialect --test emission_ownership` for the
  dispatch half; `cargo test -p smelt-runtime --test restructure_multiplicity` — a row-count
  assertion over a NULL-bearing partition key, against a real DuckDB — for the multiplicity half.
  The audit's value leg does **not** discharge this on its own: `ANY_VALUE` is a registered
  nondeterministic entry, probed on the schema leg only, so a lowering that routes through it is
  never value-compared by the audit. The multiplicity gate is what covers it.
- **Each admissibility rule in §"Statement-level lowering" has a refusal test.** The rules exist
  because each corresponds to a query the lowering would otherwise mis-answer silently — a `ROLLUP`
  super-aggregate row, an occurrence in `HAVING`, a `FILTER (WHERE …)`, a `SELECT *`, an `EXCLUDE`
  frame, an unresolved named window. A rule with no test asserting the refusal is a rule that will
  regress into a silent wrong answer, so the suite carries one case per rule.

## Known Divergences / Open Questions

- **`NOT MATCHED BY SOURCE` is unexercised.** No emitter produces the clause on any backend, so
  there is nothing to run against a warehouse; the capability row records what GoogleSQL accepts,
  not a path smelt takes. Tracked in `docs/research/20260816-bigquery-backend.md`.
- **Spark's schema-evolution DDL covers the additive changes only.** Spark has its own generator
  (no generator is shared: bare `VARCHAR` is `DATATYPE_MISSING_SIZE` and `TEXT` is not a type,
  the add is spelled `ADD COLUMNS (…)`, and the name is three-part), which emits the nullable
  column add, the struct-field add, the `NOT NULL` relaxation and the backfill `UPDATE`. The
  rules are stated for the table smelt creates — `USING DELTA` with no table properties — and
  three of them are properties of *that table* rather than of Delta: a `DEFAULT` clause on the
  add needs `allowColumnDefaults`, a drop needs `delta.columnMapping.mode`, and a widening needs
  `delta.enableTypeWidening`. smelt does not enable any of them, because each irreversibly raises
  the table's protocol version, so those changes resolve to a table rewrite (Delta) or a full
  refresh (Parquet) whose reason names the column and the limitation, never to DDL the server
  would reject. The per-operation detail is `schema_evolution.md` §"Backend capability matrix".
- **BigQuery's schema-evolution DDL covers the flat changes only.** GoogleSQL has its own
  generator (no generator is shared: it rejects `VARCHAR`, `TEXT` and `DOUBLE` as
  `Type not found`, spells widening `SET DATA TYPE`, and has no `ALTER COLUMN … USING`), which
  emits the column add, column drop, scalar widening and NOT NULL *relaxation* cases. What
  GoogleSQL cannot express — adding a `NOT NULL` column, tightening to `NOT NULL`, any struct
  field add/remove, any nested or array-element widening — resolves to a full refresh whose
  reason names the column and the limitation, never to DDL the warehouse would reject. The
  per-operation detail is `schema_evolution.md` §"Backend capability matrix". Tracked in
  `docs/research/20260816-bigquery-backend.md`.
- **Per-run dataset isolation depends on a grant the runner may not hold.** Creating a dataset
  per run needs `bigquery.datasets.create`; a principal granted only `WRITER` on one dataset
  cannot, and the suites then isolate by table name inside the granted dataset instead. Both
  paths are safe for concurrent runs and only teardown differs (a dataset drop versus a table
  drop), so the fallback is a supported mode rather than a degraded one — but the two modes leave
  different residue behind a crash, which is why created datasets carry a default table
  expiration. Tracked in `docs/research/20260816-bigquery-backend.md`.
- **The generative conformance case count on BigQuery is undecided.** Every statement costs a
  network round trip — measured at roughly 0.7 s for a trivial query and 2 s for a
  `CREATE TABLE` — against sub-millisecond in-process DuckDB. Concurrency across cases is
  preferred to cutting cases, because it preserves coverage, but it is bounded by a per-table
  limit rather than by latency: repeated modification of *one* table is refused with
  `Your table exceeded quota for table update operations` after roughly eight rapid statements,
  while the same rate spread across distinct tables is not. A generative suite must therefore
  allocate a fresh target table per case rather than reusing one. Tracked in
  `docs/research/20260816-bigquery-backend.md`.
- **The exact median was silently rounded on BigQuery, by the output-schema cast wrap rather than
  by the lowering.** `apply_type_casts` re-parses SQL that the dialect printer has *already*
  lowered, so a BigQuery median arrives as `(CAST(x AS FLOAT64) + CAST(y AS FLOAT64)) / 2`.
  `FLOAT64` is a GoogleSQL spelling smelt's type parser does not recognise, leaving both operands
  unresolved — and division's promotion rule then adopted the one type it could see, the literal
  `2`'s. The wrap emitted `CAST(med_val AS SMALLINT)` and an exact median left the warehouse
  rounded (`-284.5` measured as `-285`). Division with exactly one unresolved operand now yields
  no type, so no cast is emitted and the backend's own arithmetic stands.
- **The keyed-fold `MERGE`'s not-matched arm ignores the target dialect on BigQuery, emitting
  `INSERT *` where GoogleSQL needs `INSERT ROW`.** §"Whole-row MERGE" documents the `INSERT ROW`
  spelling, and the emitter that spells it
  (`smelt_logical::maintenance::emit::whole_row_insert_arm`,
  `crates/smelt-logical/src/maintenance/emit.rs:293`) already dispatches on `MaintenanceDialect`
  correctly. The bug is in its caller: `build_cumulative_merge_sql`
  (`crates/smelt-runtime/src/cumulative.rs:621`) takes no dialect parameter at all and calls both
  `emit_keyed_fold` and `emit_keyed_fold_suppressed` with `MaintenanceDialect::DuckDb` hardcoded
  (`crates/smelt-runtime/src/cumulative.rs:644`, `:652`), so a keyed model's cumulative-aggregate
  `MERGE` always emits `INSERT *` regardless of the target backend. Measured live 2026-08-18: `400
  Syntax error: Expected keyword ROW or keyword VALUES but got "*"` on
  `gate_keyed_bigquery::keyed_pool_upholds_end_state_equivalence_on_bigquery`. This is a genuine
  **product-side** dialect gap, not merely a test-harness issue: it affects any real user model
  using `refresh: keyed` with a cumulative aggregate on BigQuery. The dialect now threads from the
  maintenance driver through `WindowedKeyedRule::merge_sql` and into
  `build_cumulative_merge_sql`, resolved once via `smelt_backend::maintenance_dialect`, so the
  not-matched arm spells `INSERT ROW` on BigQuery and stays byte-identical on DuckDB. Confirmed
  live: `gate_keyed_bigquery::keyed_pool_upholds_end_state_equivalence_on_bigquery` is in the
  all-green 21-case sweep measured 2026-08-21, so the case that produced the syntax error now
  passes against the warehouse. Tracked in
  `docs/plans/20260817-bigquery-generative-conformance.md`.
- **The BigQuery generative-conformance leg is bounded by a one-hour credential window.** The
  service account's OAuth access token (`scripts/bigquery-auth.sh`) is short-lived and cannot be
  refreshed without a human re-entering the passphrase, so one session can drive at most one
  token's worth of wall-clock against the live warehouse — a sweep that outlives the window stops
  mid-case rather than degrading gracefully. `scripts/bigquery-conformance.sh` refuses to start a
  sweep it cannot see through: it fails loud, naming the missing thing and the fix, when
  `SMELT_BQ_PROJECT` is unset (an unset project would otherwise skip green, proving nothing) or
  when no valid token is on disk (`bash scripts/bigquery-auth.sh` mints one). The sweep runs its
  cases **concurrently**, which is what keeps it inside one window: every case derives its own
  dataset, and BigQuery's table-update burst quota binds per table, so nothing is shared to
  contend on. Measured all-green: 621.61s at the default 4-way concurrency (2026-08-22, 22 cases),
  against 2190.85s for the same suite run sequentially (2026-08-21, 21 cases). Wall-clock is
  dominated by the measured 3s per-statement pacing floor, so concurrency across cases is what
  absorbs it; the thread count is bounded rather than unbounded to stay clear of project-level
  concurrent-query limits, a different constraint from the per-table quota.
  Headroom must never be read off a *failing* sweep: a failing case costs a fraction of a passing
  one (the same suite measured 1142.10s when eight cases failed fast), so a red run's timing
  understates the real budget. The token budget is checked **once per process** against the whole
  sweep's estimated cost rather than per test — a per-test check cannot express a concurrent
  sweep's true cost, since each test would pass its own budget while the sweep collectively
  overran the window. That estimate is deliberately a sequential-cost ceiling, so it stays a safe
  bound whatever concurrency the runner chooses, which means a sweep is started against a freshly
  minted token rather than the remainder of a window a session has already spent. Tracked in
  `docs/plans/20260817-bigquery-generative-conformance.md`.
- **`supports_merge_not_matched_by_source` / `supports_staged_relation_group` are specified
  ahead of their own `BackendCapabilities` fields.** `supports_column_scoped_merge` migrated
  into the capability struct (`crates/smelt-dialect/src/dialect.rs`), matrixed above and asserted
  by the capability-conformance test alongside every other flag; the `Backend` trait no longer
  carries its own `supports_column_scoped_merge` method. The other two flags in this section
  still have no struct field or conformance assertion — the change-suppressed MERGE's
  `NOT MATCHED BY SOURCE` lowering and the staged-candidate mechanism's temp-relation grouping
  are not yet gated by a declared capability. Adding those fields is later work; the matrix above
  records the intended end state so admission has one place to specify against. Tracked in
  `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **Parity is verified by a gated CI job.** The full dual-target matrix (DuckDB + Spark),
  conformance suite, and the W1–W7 parity initiative are complete. The `spark-parity` CI job in
  `.github/workflows/compat.yml` provisions a Delta-enabled Spark Connect server, runs
  `cargo test --features smelt-cli/spark` (including MERGE, schema evolution, nested-array DDL,
  decimal precision, and timezone-aware timestamp round-trips), and tears it down. Cross-engine
  type conformance (decimal, `TIMESTAMP_NTZ`, and timezone-aware timestamps) is asserted end to
  end. The matrix above is the verified contract, not just the intended one. Tracked in
  `docs/plans/20260628-spark-parity.md`.
- **Intermediate-expression types are not individually cast.** Output-schema conformance (see
  §"Output-schema type conformance") guarantees a model's *written* schema matches the inferred
  schema on every backend. It does not rewrite *nested* occurrences of a divergent expression: a
  `CEIL(d)`, `SIGN(x)`, or `AVG(dec)` used inside a larger expression is evaluated with the
  engine's native type mid-query on Spark (e.g. `CEIL(d)` as BigInt) before the outer column cast
  applies. For the registered numeric divergences the *values* are preserved, but the intermediate
  *type* can affect engine-native semantics (e.g. integer vs floating division on the
  intermediate). Closing this would require backend-aware inference or a per-call emit-time cast on
  the divergent built-ins; both are deferred. Registry:
  `crates/smelt-db/tests/prop_helpers/divergences.rs` (`ceil_floor_double`, `avg_decimal`,
  `sign_*`).
- **Partition-pruned cross-engine reads.** The `read_parquet()` substitution reads the full
  Parquet glob on every downstream run; partition pruning at the exchange boundary is a
  performance gap, not a correctness one. Deferred.
- **Databricks** is not yet a distinct backend; the Spark adapter can attach to Databricks
  Connect but Databricks-specific capability differences are not modelled.
- **The `spark_type` divergence ledger.** The ledger in
  `crates/smelt-db/tests/prop_helpers/divergences.rs` (23 entries) has been re-verified entry by
  entry against a live Spark Connect server: every recorded `spark_type` (both `Some` claims and
  `None` "matches smelt" claims) was checked against `DESCRIBE QUERY` output for the entry's
  representative expression, corrected where stale (e.g. `SIGN`'s Spark return type is always
  `Double` regardless of argument type, not the argument's own type as previously recorded), and
  confirmed by a 1000-case property soak with zero new unregistered divergences. Per-PR gating on
  Spark-relevant paths (§"CI tiering" above) is in place as of `.github/workflows/compat.yml`'s
  `changes` job.
- **The generative maintenance-conformance oracle is dual-backend.** The dual-execution harness
  (see §"Generative equivalence coverage") runs the same recipe pool, run schedules, and
  multiset-equivalence oracle against a live Spark Connect server in the gated CI tier
  (`cargo test -p smelt-cli --features smelt-cli/spark --test maintenance_conformance_spark`),
  covering the append-only, keyed, mutable, redelivery, interleave, boundary, schema-evolution,
  composed-pool, DAG-propagation, pinned-hazard, and change-feed-admission legs. A small number of
  legs remain DuckDB-only for reasons independent of this rollout, not because the Spark twin
  hasn't landed: `Additive`-combiner keyed/composed folds have no Spark ledger dialect yet for the
  never-fold-twice reconciliation ledger (the runtime fails loud rather than mishandling it); the
  probe harness (`probes.rs`) and the feed-declared-source execution-driven leg (as opposed to its
  admission check, which is covered) still stage through a raw DuckDB connection rather than the
  backend trait. Full per-leg disposition is tracked in the gap table in
  `docs/plans/20260719-prod-w4-spark.md`; the remaining DuckDB-only legs are follow-up work, not
  blockers to the supported-vs-beta label decision.

## References

- **Code**: `crates/smelt-dialect/src/dialect.rs` (`SqlDialect`, `BackendCapabilities`),
  `crates/smelt-dialect/src/printer.rs`, `crates/smelt-dialect/src/type_conformance.rs`,
  `crates/smelt-backend/src/lib.rs` (`Backend` trait), `crates/smelt-backend-duckdb/`,
  `crates/smelt-backend-spark/`, `python/smelt/spark_adapter.py`,
  `crates/smelt-state/src/ddl_spark.rs`.
- **Tests**: `crates/smelt-cli/tests/multi_engine_test.rs`,
  `crates/smelt-backend-spark/tests/load_table.rs`, `crates/smelt-backend-spark/src/tests.rs`,
  `crates/smelt-db/tests/prop_helpers/spark_oracle.rs`,
  `crates/smelt-db/tests/type_property_tests.rs` (Spark oracle).
- **User docs**: `docs-site/docs/` backend / targets pages.
- **Plans (history)**: `docs/plans/20260328-multi-engine-example.md`,
  `docs/plans/20260628-spark-parity.md`,
  `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **Related specs**: `architecture.md` (§"Backend trait surface"), `smelt_yml.md`
  (§"Target shape"), `incremental_models.md`, `schema_evolution.md`, `testing.md`,
  `types.md`.
