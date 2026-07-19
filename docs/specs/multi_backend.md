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

- **Backends.** A target's `type:` selects a backend (`duckdb` | `spark`; see `smelt_yml.md`
  §"Target shape"). Each backend declares a `SqlDialect` (`DuckDB` | `SparkSQL` | `PostgreSQL`)
  and a `BackendCapabilities` value.
- **Capability matrix.** `BackendCapabilities` is the single declared description of what a
  backend's SQL surface supports. Backends differ **only** in (a) their capability flags and
  (b) the dialect-specific physical SQL the printer emits; they do **not** differ in which
  smelt models a user may write. The flags are:

  | Flag | DuckDB | Spark (Delta) | Spark (Parquet) |
  |------|:------:|:-------------:|:---------------:|
  | `supports_qualify` | ✓ | ✗ | ✗ |
  | `supports_create_or_replace_table` | ✓ | ✗ | ✗ |
  | `supports_create_or_replace_view` | ✓ | ✓ | ✓ |
  | `supports_merge` | ✓ | ✓ | ✗ |
  | `supports_column_scoped_merge` | ✓ | ✓ | ✗ |
  | `supports_merge_not_matched_by_source` | ✗ | ✓ | ✗ |
  | `supports_staged_relation_group` (temp-relation-backed statement group, for the merge-less conditional write) | ✓ | ✓ | ✓ |
  | `supports_pivot` | ✓ | ✓ | ✓ |
  | `supports_date_literal` | ✓ | ✗ | ✗ |
  | `supports_concat_operator` (`\|\|`) | ✓ | ✓ | ✓ |
  | `supports_array_literal` (`[a,b]`) | ✓ | ✗ | ✗ |
  | `supports_transactional_ddl` | ✓ | ✗ | ✗ |
  | `supports_double_colon_cast` (`x::T`) | ✓ | ✗ | ✗ |
  | `supports_trailing_commas` | ✓ | ✗ | ✗ |
  | `supports_insert_overwrite` | ✗ (emulated) | ✓ | ✓ |
  | `supports_native_ivm` | ✗ | ✗ | ✗ |
  | `supports_retraction` | ✗ | ✗ | ✗ |
  | `supports_struct_field_ddl` | ✓ | ✓ | ✗ |
  | `supports_alter_column_using` | ✓ | ✗ | ✗ |
  | `supports_nested_array_ddl` | ✓ | ✓ | ✗ |
  | `supports_merge_schema_write` | ✗ | ✓ | ✓ |
  | `supports_column_mapping` | ✗ | ✓ | ✗ |
  | `supports_pipe_syntax` (`\|>`) | ✗ | ✗ | ✗ |
  | `requires_schema_init` | ✓ | ✓ | ✓ |

  This table is the **honest** matrix — `smelt:validate` / the conformance tests assert the code
  constructors (`BackendCapabilities::duckdb()`, `::spark_delta()`, `::spark_parquet()`) match
  it. When a flag changes, this table changes in the same commit.
- **`SPARK_CONNECT_URL`.** Spark integration tests connect to a Spark Connect server at this
  URL. When it is unset, Spark-targeted tests **skip** (not fail). The runtime backend reads
  `connect_url` from the target config (see `smelt_yml.md`).

## Semantics

### Parity contract
The same smelt model, materialized on any backend, must produce the **same logical result**
(same rows, same column types up to the documented type-conformance rules in
`crates/smelt-dialect/src/type_conformance.rs`). A backend lacking a SQL feature does **not**
reject the model — the dialect printer **lowers** the logical construct to an equivalent
physical form the backend accepts. A capability flag set to `false` is an instruction to the
printer to lower, never a reason to emit invalid SQL or to surface a user-facing error.

**Supported-surface statement.** Dual-target parity covers: full-refresh table and view
materializations, ephemeral (CTE-inlined) models, and the `batched`/`keyed`/`versioned`
incremental maintenance legs — each exercised on both DuckDB and Spark by the same parametrized
CLI integration tests, plus the DuckDB-anchored `maintenance_conformance` suite (`smelt-cli`
crate) for the maintenance legs specifically. `refresh: materialized_view` is excluded: no
backend advertises `supports_native_ivm` today (see §"Output-schema type conformance"), so the
mode hard-errors on every backend and there is nothing to verify. Databricks-specific behaviour
beyond what the generic Spark Connect adapter exercises is excluded (see §Known Divergences).

**CI tiering.** Two tiers enforce the supported surface. A **per-PR tier** — gated on the PR's
changed paths touching Spark-relevant code (the Spark backend crate, Spark/parity integration
tests, the function-signature registry, type inference, the parser's dialect surface, or the
Python adapter) — runs `spark-parity` and `type-property-spark`. A **nightly tier** runs the
full Spark job set (including the corpus-driven `spark-integration` parser-compat job)
unconditionally, and is also reachable on demand via the `run-docker-tests` PR label. A Spark
regression outside the per-PR path filter still surfaces within one nightly cycle rather than
sitting unnoticed on `main` indefinitely.

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
- **`supports_retraction`** — whether the backend's native IVM can **invert** a contribution (delete / reprocess a prior input). Meaningful only alongside `supports_native_ivm`; native IVM sets it `true` generally. It does **not** describe smelt-driven retraction: whether a `keyed` model can retract is a *per-model* property of its column families' algebra (the group rung, `incremental_models.md` §"The maintenance boundary"), derived from the SQL, not a blanket backend flag.

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
- **Verification-first parity.** Parity is asserted by a **dual-target test matrix** (the same
  CLI integration tests parametrized over `{DuckDb, Spark}`) plus a capability-conformance
  suite, run against a real Spark Connect server. A capability the code claims but no test
  exercises against a live backend is treated as unverified. Rejected: trusting the capability
  constructors without live execution — the whole motivation here is that unverified Spark code
  had drifted from reality.
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
  `crates/smelt-db/tests/prop_helpers/divergences.rs` (22 entries) has been re-verified entry by
  entry against a live Spark Connect server: every recorded `spark_type` (both `Some` claims and
  `None` "matches smelt" claims) was checked against `DESCRIBE QUERY` output for the entry's
  representative expression, corrected where stale (e.g. `SIGN`'s Spark return type is always
  `Double` regardless of argument type, not the argument's own type as previously recorded), and
  confirmed by a 1000-case property soak with zero new unregistered divergences. Per-PR gating on
  Spark-relevant paths (§"CI tiering" above) is in place as of `.github/workflows/compat.yml`'s
  `changes` job.
- **The generative maintenance-conformance oracle has no Spark twin.** The
  deterministic-seeded `ModelRecipe` pool and its S-restricted multiset-equivalence oracle
  (`incremental_models.md` §"The equivalence invariant") run only against the DuckDB backend.
  Spark's incremental techniques (region-overwrite, keyed fold, column-scoped merge, in-place
  update) each have hand-authored fixed-recipe dual-target parity coverage, but not the generative
  sweep, its admission-rate statistics, or DAG-propagation/boundary/redelivery/schema-evolution
  probes. Building a Spark-native twin (or a dual-execution mode of the existing harness) is
  tracked as post-v0.5 backlog seeded by the gap table in
  `docs/plans/20260719-prod-w4-spark.md`.

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
