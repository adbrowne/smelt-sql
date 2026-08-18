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
  | `supports_native_ivm` | ✗ | ✗ | ✗ | ✗ |
  | `supports_retraction` | ✗ | ✗ | ✗ | ✗ |
  | `supports_struct_field_ddl` | ✓ | ✓ | ✗ | ✓ |
  | `supports_alter_column_using` | ✓ | ✗ | ✗ | ✗ |
  | `supports_nested_array_ddl` | ✓ | ✓ | ✗ | ✓ |
  | `supports_merge_schema_write` | ✗ | ✓ | ✓ | ✗ |
  | `supports_column_mapping` | ✗ | ✓ | ✗ | ✓ |
  | `supports_pipe_syntax` (`\|>`) | ✗ | ✗ | ✗ | ✓ |
  | `requires_schema_init` | ✓ | ✓ | ✓ | ✓ |

  This table is the **honest** matrix — `smelt:validate` / the conformance tests assert the code
  constructors (`BackendCapabilities::duckdb()`, `::spark_delta()`, `::spark_parquet()`,
  `::bigquery()`) match it. When a flag changes, this table changes in the same commit. A
  backend's column is established by executing the statement each flag names against a live
  instance of that backend, never by reading its documentation.
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
coverage is generative rather than fixed-recipe-only. Coverage is partial, measured live
2026-08-18 (`bash scripts/bigquery-conformance.sh`, `--test-threads=1`): 13 of 21 cases pass. The
S-restricted oracle relation's `CREATE OR REPLACE TEMPORARY VIEW` gap and the composed family's
hand-rolled row set — both previously the dominant cause of failure here — are closed; no case
fails on either any more. The remaining eight failures span one product-side GoogleSQL dialect gap
(a keyed-fold `MERGE`'s not-matched arm ignores the target dialect), two harness-side gaps (the
oracle's raw-SQL rendering bypasses smelt's `MEDIAN` lowering; a shared default `execute_model`
issues a `DROP VIEW` against an existing `TABLE`, which BigQuery refuses unlike DuckDB and Spark),
a per-case dataset staging collision in the `pinned` family, and two cases whose live failure text
was not captured and needs a fresh run to diagnose. Six of the eight carry a landed fix whose
effect no live sweep has confirmed yet; the pass count above is the last measured one, not a
current one. Full breakdown and evidence in §Known
Divergences. `refresh:
materialized_view` is excluded: no
backend advertises `supports_native_ivm` today (see §"Output-schema type conformance"), so the
mode hard-errors on every backend and there is nothing to verify. Databricks-specific behaviour
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
the equivalent. In aggregate position (`GROUP BY`) `PERCENTILE_CONT` cannot be used at all —
GoogleSQL makes it analytic-only — and `APPROX_QUANTILES`, the one aggregate offered, is
approximate. The lowering there sorts the argument into an array with `ARRAY_AGG(x IGNORE NULLS
ORDER BY x)` and indexes its middle element, averaging the two middle elements at even counts;
the array sub-expression is repeated rather than bound to a name because GoogleSQL rejects an
aggregate inside `UNNEST`. The aggregate form casts to `FLOAT64`, matching the numeric return
type; a temporal argument, which DuckDB's `MEDIAN` accepts, is refused by the backend rather than
silently coerced.

### Operator lowering
An infix operator smelt's grammar accepts but a backend's SQL does not is lowered by the dialect
printer, never emitted verbatim and left to the engine. Two operators need this on GoogleSQL, and
they need it for opposite reasons. `%` (modulo) has no infix form there at all, so an unlowered
`a % b` is a syntax error; it lowers to `MOD(a, b)`. `^` is worse: GoogleSQL *does* define infix
`^`, as bitwise XOR, while smelt's grammar reads it as DuckDB does — a synonym for `**`, power.
An unlowered `^` therefore does not fail on BigQuery, it silently returns a different number, so
`^` and `**` both lower to `POWER(a, b)`. Every other dialect prints all three unchanged. The
lowerings are exact: DuckDB's power operator returns a double for every operand type, negative
base, negative exponent, and `0 ^ 0 = 1` included, and `POWER` agrees on each. They diverge only
at `0 ^ -1`, where DuckDB yields infinity and GoogleSQL raises — a loud failure, not a wrong
answer.

`//` (floor division) is deliberately **not** lowered. DuckDB's `//` truncates toward zero when
both operands are integers, but degrades to plain division the moment either is floating point,
and the printer carries no operand types with which to tell those cases apart. GoogleSQL's `DIV`
matches only the integer case, so substituting it unconditionally would silently floor a result
that should not have been floored. GoogleSQL has no infix `//` either, so leaving it alone makes
it a loud syntax error instead — the correct outcome under fail-loud discipline until the printer
can see operand types.

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
Two flags describe a backend's participation in maintaining a keyed refresh mode's state; both are `false` on every backend today.

- **`supports_native_ivm`** — the backend can maintain a declared query as a **native incremental view** (Databricks Enzyme, Snowflake Dynamic Tables). It gates the `refresh: materialized_view` mode: `true` → smelt emits the native maintained object and the engine owns freshness; `false` → the hard error above. It is *not* consulted for the smelt-driven keyed modes (`keyed`, `versioned`), which maintain their own state with `merge_into` + views on any backend.
- **`supports_retraction`** — whether the backend's native IVM can **invert** a contribution (delete / reprocess a prior input). Meaningful only alongside `supports_native_ivm`; native IVM sets it `true` generally. It does **not** describe smelt-driven retraction: whether a `keyed` model can retract is a *per-model* property of its column families' algebra (the group rung, `incremental_shapes.md` §"The maintenance boundary"), derived from the SQL, not a blanket backend flag.

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
the compiled SQL's select list using the same notion of an output column name the analyzer's
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
- **Cross-engine exchange is filesystem-local today.** Remote object stores (S3/GCS/ADLS) are
  explicitly out of scope until a mirrored test demands one.
- **Data loading carries no host-filesystem assumption.** A backend's load path (seeds, test
  fixtures, Arrow batches) must transfer rows through the backend client API, never via a host
  path the server is asked to read. A load path that only works when the server shares the host
  filesystem is a bug, not a deployment constraint (see §"Loading data into a backend").
- **No new logical surface per backend.** Backends may differ in physical SQL and capability
  flags only; the set of writable smelt models is backend-independent.

## Known Divergences / Open Questions

- **`supports_pipe_syntax` is unexercised by any parity test.** BigQuery is the only backend
  reporting `true`, and no parity fixture writes a pipe query, so the printer's
  emit-pipes-natively path has no live coverage on the one backend that would take it. Every
  other BigQuery-relevant printer path does: `materialization_parity`, `seed_parity`,
  `lowering_parity`, `merge_parity`, `incremental_parity` and `schema_evolution_parity` each
  carry a BigQuery leg. `NOT MATCHED BY SOURCE` is likewise uncovered, but for a different
  reason — no emitter produces the clause on any backend yet, so there is nothing to run.
  Tracked in `docs/research/20260816-bigquery-backend.md`.
- **A model whose output columns are not statically resolvable cannot use
  `Technique::ColumnScopedMerge` on BigQuery.** The whole-row `MERGE` needs an explicit column
  list there (see §"Whole-row MERGE"), and a surviving wildcard projection leaves that list
  empty, so the run fails with an error naming the model rather than emitting a matched arm
  that assigns nothing. DuckDB and Spark are unaffected — their `UPDATE SET *` needs no list.
- **`cross_engine_parity` and `cross_engine_types_parity` are DuckDB↔Spark only.** They assert
  handoff between two live engines rather than looping over `targets_to_run`, so extending them
  to BigQuery means a new engine *pair*, not a third leg of an existing loop. The type-level
  half of that gap is what a BigQuery type oracle would close.
- **BigQuery advertises `supports_native_ivm: false` despite supporting materialized views.**
  The warehouse accepts `CREATE MATERIALIZED VIEW` with incremental refresh, so unlike DuckDB
  and Spark this backend's `false` describes smelt, not the engine: `true` obliges smelt to emit
  the native maintained object and cede freshness to the engine, and that emission path does not
  exist. Until it does, `refresh: materialized_view` hard-errors on BigQuery exactly as it does
  everywhere else. This is the first case where a flag's value is an implementation statement
  rather than a warehouse one, and it is the reason the matrix cell alone is not a sufficient
  description. Tracked in `docs/research/20260816-bigquery-backend.md`.
- **Schema-evolution DDL is not implemented for BigQuery.** GoogleSQL rejects the type names the
  DuckDB generator emits (`VARCHAR`, `TEXT`, `DOUBLE` are each `Type not found`) and has no
  `ALTER COLUMN … USING`, so no generator is shared. A schema change on a BigQuery model
  therefore resolves to a full refresh rather than a migration — surfaced as a refusal naming
  the reason, never as emitted DDL the warehouse would reject. Tracked in
  `docs/research/20260816-bigquery-backend.md`.
- **Per-run dataset isolation depends on a grant the runner may not hold.** Creating a dataset
  per run needs `bigquery.datasets.create`; a principal granted only `WRITER` on one dataset
  cannot, and the suites then isolate by table name inside the granted dataset instead. Both
  paths are safe for concurrent runs and only teardown differs (a dataset drop versus a table
  drop), so the fallback is a supported mode rather than a degraded one — but the two modes leave
  different residue behind a crash, which is why created datasets carry a default table
  expiration. Tracked in `docs/research/20260816-bigquery-backend.md`.
- **BigQuery has no CI tier.** Spark parity runs per-PR on changed paths and nightly in full;
  BigQuery runs **only when a developer runs it by hand** against their own GCP project, gated on
  `SMELT_BQ_PROJECT`. A BigQuery regression therefore does not surface on `main` on any schedule.
  This is a deliberate consequence of keeping cloud credentials off CI, not an oversight, and it
  means a claim of Spark-equivalent BigQuery coverage is a claim about which gates exist, never
  about when they run. Revisiting it requires a service account, a GitHub secret, and a billing
  decision. Tracked in `docs/research/20260816-bigquery-backend.md`.
- **The generative conformance case count on BigQuery is undecided.** Every statement costs a
  network round trip — measured at roughly 0.7 s for a trivial query and 2 s for a
  `CREATE TABLE` — against sub-millisecond in-process DuckDB. Concurrency across cases is
  preferred to cutting cases, because it preserves coverage, but it is bounded by a per-table
  limit rather than by latency: repeated modification of *one* table is refused with
  `Your table exceeded quota for table update operations` after roughly eight rapid statements,
  while the same rate spread across distinct tables is not. A generative suite must therefore
  allocate a fresh target table per case rather than reusing one. Tracked in
  `docs/research/20260816-bigquery-backend.md`.
- **Eight of the BigQuery generative-conformance leg's 21 cases still fail live, for four
  distinct causes.** Measured 2026-08-18 (`bash scripts/bigquery-conformance.sh`,
  `--test-threads=1`: 13 passed / 8 failed / 0 ignored, 1142.10s wall-clock). The two gaps
  previously recorded in this section — the S-restricted oracle relation's `CREATE OR REPLACE
  TEMPORARY VIEW` (`STracker::materialize_s_as_view`,
  `crates/smelt-maintenance-testkit/src/s_tracker.rs:296`) and the composed family's hand-rolled
  `(VALUES …)` row set (`composed_delta_values_sql`,
  `crates/smelt-maintenance-testkit/src/families/gate_composed.rs:201`) — are both **closed**: no
  case fails on either gap any more. Critically,
  `harness_self_check_bigquery::oracle_flags_a_seeded_divergence_on_bigquery` now **passes**,
  the first live demonstration that BigQuery's leg has a non-vacuous oracle (it catches a
  deliberately seeded divergence). The remaining eight failures:
  - `gate_bigquery::append_only_partition_pool_upholds_equivalence_on_bigquery` — `400 Function
    not found: MEDIAN`. The harness executes its S-restricted oracle SQL as raw SQL
    (`STracker`), bypassing smelt's printer, so the exact-`MEDIAN` GoogleSQL lowering
    (§"Exact-median lowering") that applies to compiled models does not apply to the oracle's own
    rendering of a `HolisticAgg` body. **Harness-side.** *Fixed, unconfirmed live:* the oracle body
    now round-trips through the dialect-aware printer, so the lowering applies to it exactly as it
    does to a compiled model.
  - `gate_bigquery::column_add_between_runs_recovers_equivalence_on_bigquery` and
    `gate_bigquery::full_refresh_interleave_resets_state_correctly_on_bigquery` — `400 Cannot
    drop <project>:<dataset>.recipe_additive_agg which has type TABLE. A view was expected.` The
    default `Backend::execute_model` (`crates/smelt-backend/src/lib.rs:216`,`:222`-`223`)
    unconditionally issues `drop_view_if_exists` before `drop_table_if_exists` (or the reverse)
    "in case the materialization type changed" — a no-op on DuckDB and Spark when the object is
    the other kind, but BigQuery's `DROP VIEW IF EXISTS` errors on a type mismatch instead of
    treating it as absent. **Product-side** (the default `execute_model` implementation shared by
    every backend). *Fixed, unconfirmed live:* the BigQuery backend now classifies that one error
    shape as "already absent" and honours the `IF EXISTS` contract every other backend keeps;
    every other error still propagates.
  - `gate_keyed_bigquery::keyed_pool_upholds_end_state_equivalence_on_bigquery` — `400 Syntax
    error: Expected keyword ROW or keyword VALUES but got "*"` on a compiled `MERGE … WHEN NOT
    MATCHED THEN INSERT *` statement. **Product-side**; recorded as its own divergence below
    (`build_cumulative_merge_sql` hardcodes the DuckDB dialect). *Fixed, unconfirmed live:* the
    dialect now threads from the driver through `WindowedKeyedRule::merge_sql`.
  - `pinned_bigquery::hazard_schedules_are_pinned_on_bigquery` and
    `pinned_bigquery::pinned_recipes_reproduce_catalogue_coverage_on_bigquery` — `409 Already
    Exists: Table …sources_events`. A case stages its source table twice into one per-case
    dataset — structurally the same twin-target collision shape the `dags` family already fixed,
    in a family (`pinned`) that did not get that fix. **Harness-side.** *Fixed, unconfirmed live:*
    each independent staging in the family now carries its own case through the existing
    target/schema seam.
  - `dags_bigquery::diamond_propagation_suffices_on_bigquery` and
    `gate_composed_bigquery::composed_keyed_pool_upholds_equivalence_on_bigquery` — both
    characterised by the 2026-08-19 sweep. `composed_keyed_pool` **passes**; it was collateral
    from the gaps already closed. `diamond_propagation` failed with `400 Syntax error: Expected
    ")" but got "%"` — a model body's `id % 2` reaching GoogleSQL unlowered, now fixed
    (§"Operator lowering") but not yet confirmed live.
  A targeted re-run of just the `gate_bigquery` module independently measured 3 passed / 3 failed
  in 200.75s, consistent with the full-sweep numbers above. Tracked in
  `docs/plans/20260817-bigquery-generative-conformance.md`.
- **Re-measured live 2026-08-19: 14 passed / 7 failed / 0 ignored, 1265.89s** — but five of the
  seven failures are the credential window expiring mid-sweep (474s remaining against a 600s
  estimated need), not defects: `gate_mixed`, both `pinned` cases, and two `harness_self_check`
  cases were refused by the token preflight and never ran, so the `pinned` staging-collision fix
  remains unconfirmed. The `INSERT ROW` dialect gap and both `DROP`-type-mismatch cases now pass
  live, as does `gate_composed_bigquery::composed_keyed_pool_upholds_equivalence_on_bigquery`.
  Two genuine failures remain: `dags_bigquery::diamond_propagation_suffices_on_bigquery` (the
  unlowered `%`, since fixed) and
  `gate_bigquery::append_only_partition_pool_upholds_equivalence_on_bigquery`, which now clears
  the `MEDIAN` error and fails instead on a real S-restricted equivalence violation at the first
  run whose `ColumnScopedMerge` takes the `MATCHED` arm rather than inserting a new partition.
  BigQuery is the only backend whose matched arm is an explicit `SET c = source.c, …` column list
  rather than `SET *` (GoogleSQL has no star form — §"Whole-row MERGE"), and the identical
  recipe and schedule pass on DuckDB. Cause unconfirmed: distinguishing a stale row from a
  partial column update from a duplicated row needs a live re-run, which now reports the
  differing rows rather than only the two queries. Tracked in
  `docs/plans/20260817-bigquery-generative-conformance.md`.
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
  not-matched arm spells `INSERT ROW` on BigQuery and stays byte-identical on DuckDB — asserted
  offline; no live sweep has confirmed the case passes yet. Tracked in
  `docs/plans/20260817-bigquery-generative-conformance.md`.
- **The BigQuery generative-conformance leg is bounded by a one-hour credential window.** The
  service account's OAuth access token (`scripts/bigquery-auth.sh`) is short-lived and cannot be
  refreshed without a human re-entering the passphrase, so one session can drive at most one
  token's worth of wall-clock against the live warehouse — a sweep that outlives the window stops
  mid-case rather than degrading gracefully. `scripts/bigquery-conformance.sh` refuses to start a
  sweep it cannot see through: it fails loud, naming the missing thing and the fix, when
  `SMELT_BQ_PROJECT` is unset (an unset project would otherwise skip green, proving nothing) or
  when no valid token is on disk (`bash scripts/bigquery-auth.sh` mints one). Measured 2026-08-18:
  the full 21-case shared-family sweep (`bash scripts/bigquery-conformance.sh`,
  `--test-threads=1`) takes 1142.10s (~19 minutes) wall-clock — about a third of the one-hour
  token window, with large headroom. The credential-window constraint itself remains real (the
  window still bounds a session to one token's worth of wall-clock, and a larger case pool or a
  slower network day could still exhaust it), but no concurrency or case-count reduction is needed
  to fit today's sweep inside one window. Tracked in
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
