---
feature: multi_backend
status: experimental
last_reviewed: 2026-06-28
owners: [andrew]
---

# Multi-backend execution & backend parity

> **What this is.** A normative spec for how smelt runs the same logical model across more
> than one execution backend (today DuckDB and Spark), and the parity contract that binds
> them: the `BackendCapabilities` matrix, how the dialect printer lowers logical SQL to each
> backend's valid physical SQL, and the cross-engine data-exchange rules. Out of scope: the
> `Backend` trait method surface itself (see `architecture.md` §"Backend trait surface"); how
> incremental strategies are *chosen* (see `incremental_models.md`); how schema changes are
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
  | `supports_pivot` | ✓ | ✓ | ✓ |
  | `supports_date_literal` | ✓ | ✗ | ✗ |
  | `supports_concat_operator` (`\|\|`) | ✓ | ✓ | ✓ |
  | `supports_array_literal` (`[a,b]`) | ✓ | ✗ | ✗ |
  | `supports_transactional_ddl` | ✓ | ✗ | ✗ |
  | `supports_double_colon_cast` (`x::T`) | ✓ | ✗ | ✗ |
  | `supports_trailing_commas` | ✓ | ✗ | ✗ |
  | `supports_insert_overwrite` | ✗ (emulated) | ✓ | ✓ |
  | `supports_materialized_views` | ✗ (table fallback) | ✓ | ✓ |
  | `supports_struct_field_ddl` | ✓ | ✓ | ✗ |
  | `supports_alter_column_using` | ✓ | ✗ | ✗ |
  | `supports_nested_array_ddl` | ✓ | ✓ | ✗ |
  | `supports_merge_schema_write` | ✗ | ✓ | ✓ |
  | `supports_column_mapping` | ✗ | ✓ | ✗ |
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
- `supports_materialized_views = false` → fall back to `Table` materialization (DuckDB).

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
reference to `read_parquet('{warehouse}/{schema}/{model}/**/*.parquet', hive_partitioning =
true)`. This requires the referenced Spark model to be `materialization: table` and the Spark
target to declare a `warehouse` path that is on a filesystem the DuckDB process can also read.
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

- **Parity is not yet verified end to end.** At `last_reviewed`, Spark backend code exists but
  is largely unexercised: integration tests are gated on `SPARK_CONNECT_URL` and no Spark runs
  in CI. The dual-target matrix, the conformance suite, and the gated CI job are being built;
  tracked in `docs/plans/20260628-spark-parity.md`. Until that lands, the matrix above is the
  *intended* contract, and specific lowerings (QUALIFY, date literals, `::` cast, array
  literals) may emit invalid Spark SQL where the printer does not yet honor the flag.
- **Session init and Arrow loading are not yet honored by the Spark backend.** The Spark adapter
  selects the current schema before creating it (so a first run against a fresh schema fails
  `[SCHEMA_NOT_FOUND]`), and `load_table` stages a host-path Parquet the remote JVM cannot read
  (`[PATH_NOT_FOUND]`). The §Semantics "Session initialization" and "Loading data into a backend"
  contracts above describe the intended behavior; the fixes are tracked in
  `docs/plans/20260628-spark-parity.md`.
- **Cross-engine type conformance at the Parquet boundary is unvalidated.** Decimal precision
  and timestamp-timezone round-tripping across Spark→DuckDB are not yet asserted by a test.
  Tracked in `docs/plans/20260628-spark-parity.md`.
- **Partition-pruned cross-engine reads.** The `read_parquet()` substitution reads the full
  Parquet glob on every downstream run; partition pruning at the exchange boundary is a
  performance gap, not a correctness one. Deferred.
- **Databricks** is not yet a distinct backend; the Spark adapter can attach to Databricks
  Connect but Databricks-specific capability differences are not modelled.

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
  `docs/plans/20260628-spark-parity.md`.
- **Related specs**: `architecture.md` (§"Backend trait surface"), `smelt_yml.md`
  (§"Target shape"), `incremental_models.md`, `schema_evolution.md`, `testing.md`,
  `types.md`.
