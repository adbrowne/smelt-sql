---
feature: multi_backend
status: experimental
last_reviewed: 2026-09-04
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

- **Backends.** A target's `type:` selects a backend (`duckdb` | `spark` | `bigquery` |
  `databricks` | `trino`; see `smelt_yml.md` §"Target shape"). Each backend declares a
  `SqlDialect` (`DuckDB` | `SparkSQL` | `BigQuery` | `Trino`) and a `BackendCapabilities`
  value. A `bigquery` target names a `project`, `dataset`, and `location` in place of DuckDB's
  `database` or Spark's `connect_url`. A `databricks` target declares `SqlDialect::SparkSQL`
  and `BackendCapabilities::databricks()`, and names `host`, `catalog`, and `schema` in place
  of Spark's `connect_url`/`warehouse` — it shares Spark's dialect (both compile to Spark SQL)
  while carrying its own capability profile and connection shape (§"Why Databricks is a
  distinct target type"). A `trino` target declares `SqlDialect::Trino` and
  `BackendCapabilities::trino_iceberg()`, naming `host`/`port`/`user`/`catalog`/`schema` in
  place of Spark's `connect_url` — it is the first backend whose dialect is not shared with
  another target type.
- **Capability matrix.** `BackendCapabilities` is the single declared description of what a
  backend's SQL surface supports. Backends differ **only** in (a) their capability flags and
  (b) the dialect-specific physical SQL the printer emits; they do **not** differ in which
  smelt models a user may write. The flags are:

  | Flag | DuckDB | Spark (Delta) | Spark (Parquet) | BigQuery | Databricks | Trino (Iceberg) |
  |------|:------:|:-------------:|:---------------:|:--------:|:----------:|:---------------:|
  | `supports_qualify` | ✓ | ✗ | ✗ | ✓ | ✗ | ✗ |
  | `supports_create_or_replace_table` | ✓ | ✗ | ✗ | ✓ | ✗ | ✓ |
  | `supports_create_or_replace_view` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
  | `supports_merge` | ✓ | ✓ | ✗ | ✓ | ✓ | ✓ |
  | `supports_column_scoped_merge` | ✓ | ✓ | ✗ | ✓ | ✓ | ✓ |
  | `supports_merge_not_matched_by_source` (spec-only; no struct field yet — §Known Divergences) | ✗ | ✓ | ✗ | ✓ | ✓ | ✗ |
  | `supports_staged_relation_group` (temp-relation-backed statement group, for the merge-less conditional write; spec-only — §Known Divergences) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
  | `staged_relation_residence` (`SessionTemporary` / `TargetSchema`) | SessionTemporary | SessionTemporary | SessionTemporary | SessionTemporary | SessionTemporary | TargetSchema |
  | `staged_relation_group_is_atomic` | ✓ | ✓ | ✓ | ✓ | ✓ | ✗ |
  | `supports_pivot` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
  | `supports_date_literal` | ✓ | ✗ | ✗ | ✓ | ✗ | ✓ |
  | `supports_concat_operator` (`\|\|`) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
  | `supports_array_literal` (`[a,b]`) | ✓ | ✗ | ✗ | ✓ | ✗ | ✓ |
  | `supports_transactional_ddl` | ✓ | ✗ | ✗ | ✓ | ✗ | ✗ |
  | `supports_double_colon_cast` (`x::T`) | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ |
  | `supports_trailing_commas` | ✓ | ✗ | ✗ | ✓ | ✗ | ✗ |
  | `supports_insert_overwrite` | ✗ (emulated) | ✓ | ✓ | ✗ (emulated) | ✓ | ✗ (emulated) |
  | `supports_native_ivm` | ✗ | ✗ | ✗ | ✓ | ✗ | ✗ |
  | `supports_retraction` | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
  | `supports_struct_field_ddl` | ✓ | ✓ | ✗ | ✓ | ✓ | ✓ |
  | `supports_alter_column_using` | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ |
  | `supports_nested_array_ddl` | ✓ | ✓ | ✗ | ✓ | ✓ | ✓ |
  | `supports_merge_schema_write` | ✗ | ✓ | ✓ | ✗ | ✓ | ✗ |
  | `supports_column_mapping` | ✗ | ✓ | ✗ | ✓ | ✓ | ✓ |
  | `supports_pipe_syntax` (`\|>`) | ✗ | ✗ | ✗ | ✓ | ✗ | ✗ |
  | `supports_pipe_set_drop_rename` (star-modifier trio `* REPLACE` / `* EXCLUDE` / `* RENAME`) | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ |
  | `supports_fingerprint_sidecar` (delta-restriction admission over an external `mutable_snapshot` source's synthesized fingerprint diff) | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ |
  | `requires_schema_init` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
  | `null_safe_equality` (synthesised join spelling for a statement-level restructure) | `IS NOT DISTINCT FROM` | `<=>` | `<=>` | `IS NOT DISTINCT FROM` | `<=>` | `IS NOT DISTINCT FROM` |

  The Databricks column equals the Spark (Delta) column in every flag except
  `null_safe_equality`, which follows Spark SQL's own `<=>` spelling rather than DuckDB's
  `IS NOT DISTINCT FROM` — Databricks and Spark share a SQL dialect. `supports_native_ivm` is
  `✗` because the flag states what smelt itself emits, and smelt emits no Enzyme statements;
  Databricks' native incremental-view-maintenance engine is unmodelled (§Known Divergences).
  These cells are inherited from the Spark (Delta) profile and not yet each independently
  executed against a live Databricks workspace (§Known Divergences).

  The Trino column is measured: every cell was established by executing the statement it names
  against a live coordinator (`scripts/trino-up.sh`,
  `crates/smelt-backend-trino/tests/capability_probes.rs`), and `BackendCapabilities::trino_iceberg()`
  was written together with this table in the same commit. The working prior going in was that
  Trino sits near Spark (Delta) — Iceberg and Delta share the same per-table-commit,
  no-cross-table-transaction atomicity shape — and it held on most flags: both refuse `QUALIFY`,
  `::` casts, trailing commas, transactional DDL, native IVM/retraction, `ALTER COLUMN ...
  USING`, pipe syntax and the star-modifier trio, while both accept `CREATE OR REPLACE VIEW`,
  `MERGE`, column-scoped `MERGE`, the staged-relation-group pattern, `PIVOT`, `||`, struct-field
  and nested-array DDL, and column mapping. It broke on seven flags: Trino accepts `CREATE OR
  REPLACE TABLE`, a `DATE` literal, and the `[a,b]` array-literal syntax where Spark(Delta)
  refuses all three; Trino refuses `WHEN NOT MATCHED BY SOURCE`, `INSERT OVERWRITE`, and an
  implicit schema-widening write where Spark(Delta) accepts them; and Trino's null-safe
  equality spelling is `IS NOT DISTINCT FROM` (matching DuckDB/BigQuery) rather than Spark's
  `<=>`. Measured errors for every `✗` are quoted in
  `docs/outcomes/20260913-trino-target-spine/outcome.md` §"Decision log".

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
- **`SMELT_DATABRICKS_HOST` / `SMELT_DATABRICKS_TOKEN`.** Databricks integration tests connect
  to the workspace named by these two variables. When either is unset, Databricks-targeted
  tests **skip** (not fail), exactly as Spark's and BigQuery's do.
- **`SMELT_TRINO_URL`.** Trino integration tests connect to the coordinator this variable
  names. When it is unset, Trino-targeted tests **skip** (not fail), exactly as Spark's,
  BigQuery's and Databricks' do. This skip applies to the backend's own integration and parity
  tests, run by a developer with no Docker tier up; it does not apply to the cross-engine
  emission audit's live Trino legs, which follow the never-skip-green rule stated in §"CI
  tiering" and §"Cross-engine emission audit" instead.

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

Trino is a fourth target for the surface parity legs covers today: full-refresh table and view
materializations and ephemeral (CTE-inlined) models, plus expression- and clause-level emission
correctness (§"Operator lowering", §"Clause-level dialect refusals", §"Cross-engine emission
audit"). Trino realises none of the five correctness structures §"Incremental & schema evolution
per backend" names, for the connector's per-table-commit reason stated there — permanently, not
pending work (`docs/outcomes/20260913-trino-ledger`). Every `batched`/`keyed`/`versioned`
maintenance technique that would otherwise depend on one of those structures downgrades to its
recompute-family equivalent, a full refresh, carrying `MaintenanceStateDowngraded`.
Availability resolution and the maintenance-plan report are independent of the
maintenance-statement dialect and reach this downgrade regardless: `maintenance_dialect` returns
`Err` for `SqlDialect::Trino`, but that `Err` only means Trino's maintenance *statement* text
cannot be rendered yet — it is refused by name at the two statement-rendering leaves
(`smelt explain --show-sql`, the per-cell technique-preview SQL), never surfaced as a refusal of
the plan, the downgrade, or the report themselves. Statement rendering for Trino is
`docs/outcomes/20260913-trino-incremental`'s subject.

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

Trino needs no cloud credential — the coordinator, Iceberg REST catalog and MinIO all run in
Docker — so it gets a real per-PR/nightly tier like Spark's rather than BigQuery's manual-sweep
treatment. The `trino-integration` job (`.github/workflows/compat.yml`) runs per-PR when the PR's
changed paths touch Trino-relevant code (the `smelt-backend-trino` crate, Trino parity tests, the
signature registry, or the dialect printer), and unconditionally on a nightly schedule or the
`run-docker-tests` PR label, exactly as `spark-parity` does. What distinguishes Trino's discipline
from every other gated backend: a live Trino leg that cannot reach the coordinator **fails the
job**, it does not skip green. A skipped audit leg is indistinguishable from a passing one, which
is the precise hole this outcome exists to close, so the job greps its own test output for a
skip and errors out if it finds one rather than letting a green run hide a leg that never executed.
This rule governs the audit's live legs specifically; it does not change the unrelated §Surface
statement that a Trino-targeted *integration* test skips when `SMELT_TRINO_URL` is altogether
unset outside this job (a developer running the suite locally with no Docker tier up) — inside the
`trino-integration` job the tier is always up, so a skip there is a bug, not an expected local
fallback.

The type-property oracle's Trino leg (§"Output-schema type conformance") runs on the same tier as
Trino's other live legs: per-PR on Trino-relevant path changes, else labeled PR + nightly. Its
environment gate is `SMELT_TRINO_URL`: unset, the leg is simply **absent** from the suite (as the
Spark and BigQuery legs are when their own gates are unset) — a distinct state from a reachable
tier whose probes are skipped, which the same never-skip-green rule above forbids.

### Inline row-set construction
Every production path that splices a small literal row set into generated SQL — an ephemeral
seed's CTE, a repair's affected-key list, an append-only baseline probe's recorded partitions, a
`smelt.test` mock dataset — renders it through a single dialect-aware owner
(`smelt_core::build_row_set_table` / `row_set_body`, `crates/smelt-core/src/sql/row_set.rs`)
rather than formatting `VALUES (…)` itself. DuckDB and Spark accept a `VALUES (…),
(…)` table-value constructor directly, unchanged. GoogleSQL has none: `FROM (VALUES (1), (2))`
is a syntax error. The owner renders BigQuery's row set as `SELECT * FROM UNNEST([STRUCT(… AS
col), (…)])` instead — GoogleSQL's own array-of-structs form, whose cost to the query planner is
one operand regardless of how many rows the set carries. Column names come from the first array
element's field aliases, which is also what types the array; later elements are bare tuples. The
chained `SELECT … UNION ALL SELECT …` rewrite is equally valid GoogleSQL and is **not** used,
because it costs one query operand *per row*: a 5,797-row baseline renders as a 692,597-character
statement BigQuery refuses outright ("Not enough resources for query planning - too many
subqueries or query is too complex"). A caller whose first row may hold an untyped `NULL` casts
it, since GoogleSQL types a bare `NULL` as `INT64` and the first element types the array.
Deciding what an *empty* row
set means (an always-false guard row, a `WHERE FALSE` predicate with no row at all, …) stays a
per-caller business decision, not a row-set construction detail — the owner requires at least one
row and callers handle the empty case themselves before reaching it.

### Exact-median lowering
`MEDIAN(x)` is an exact, interpolating median on every backend that executes it. DuckDB and Spark
are emitted unchanged; GoogleSQL has no `MEDIAN` built-in, so the dialect printer
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

`^` is the critical case. In smelt's grammar, and in DuckDB, `^` means power —
a synonym for `**`. But **both GoogleSQL and Spark SQL** define infix `^` as **bitwise XOR**, so
emitting `^` verbatim against either backend silently returns a different number from what smelt's
semantics say. Both GoogleSQL and Spark therefore lower `^` (and `**`) to `POWER(a, b)`. Trino has
no infix `^` operator at all — it is a syntax error, not a differently-meaning operator — making
Trino the third dialect (after GoogleSQL and Spark) that lowers `^`/`**` to `POWER(a, b)` rather
than emitting it verbatim.

`%` (modulo) has no infix form in GoogleSQL at all, so an unlowered `a % b` is a syntax error
there. Its lowering is **operand-conditional** (§"Operand-conditional verdicts"): GoogleSQL's `MOD`
accepts only `INT64` and `NUMERIC`, so `%` lowers to `MOD(a, b)` for integral and decimal operands
and to a truncated-remainder template for floating-point ones (DuckDB's float `%` keeps the sign of
the dividend, so the template is the truncating form, not a floor-based one). An operand whose type
inference cannot resolve takes the `MOD` arm, which is admissible because a misclassified
floating-point operand fails loudly at the warehouse rather than returning a different number.
Trino, unlike GoogleSQL, **does** have an infix `%` operator, so no lowering applies there — it
takes the `Native` verdict.

`//` (floor division) is the sharper case of the same axis. DuckDB's `//` truncates toward zero when
both operands are integral but degrades to plain division the moment either is floating point, so
no single spelling on another engine is correct for both. It is therefore stated per operand class:
integral operands lower to the target's integer-division form (`DIV(a, b)` on GoogleSQL and Spark
SQL), floating-point operands lower to plain `/`, and an operand whose class cannot be resolved is
refused with `UnsupportedOnBackend` — here a wrong guess would be a silently wrong number, so the
unresolved arm must refuse. Each arm is verified by the audit's value leg against DuckDB's own `//`
before it is claimed. Trino has no infix `//` either, but it needs no per-class arms at all: Trino
has no `DIV` function, and its `/` operator is already class-sensitive in the same direction as
DuckDB's `//` — `7/2 = 3` and `-7/2 = -3` (truncation toward zero) over integer operands,
`7.5/2.0 = 3.750000` (plain division) over floating/decimal ones (measured live against the
coordinator). So the whole operand axis collapses to a single unconditional
`Template("{0} / {1}")`, with no `Conditional` arms and no unresolved-operand refusal, because
there is no operand class for which Trino's spelling differs.

`::` (the cast operator) has no Trino spelling at all, joining GoogleSQL and Spark SQL as dialects
where `supports_double_colon_cast = false`: `CAST(x AS t)` is the only form Trino's grammar
accepts, per the existing `supports_double_colon_cast = false` lowering rule (§"Output-schema type
conformance").

The `POWER` lowerings are exact: DuckDB's power operator returns a double for every operand type,
negative base, negative exponent, and `0 ^ 0 = 1` included, and `POWER` agrees on each. They
diverge only at `0 ^ -1`, where DuckDB yields infinity and GoogleSQL raises — a loud failure,
not a wrong answer.

### Clause-level dialect refusals
Not every dialect difference is a built-in's spelling. Some SQL *clauses* smelt's grammar
accepts are absent from a target dialect entirely, and none has a registry entry to carry a
verdict, because none belongs to any one function:

- **The aggregate `FILTER (WHERE …)` clause.** DuckDB and Spark SQL have it; GoogleSQL has no such
  clause and answers `Syntax error: Expected ")" but got "("`.
- **An `INTERVAL`-offset `RANGE` window frame** (`RANGE BETWEEN INTERVAL '2 days' PRECEDING`).
  DuckDB and Spark SQL have it; GoogleSQL's `RANGE` frames take a numeric offset over a numeric
  `ORDER BY` only, and answer `Syntax error: Unexpected keyword PRECEDING`.
- **The `UNPIVOT` clause.** Trino's grammar has no `UNPIVOT` keyword: a live coordinator answers
  `mismatched input 'UNPIVOT'` (`docs/outcomes/20260913-trino-emission` phase 4). This is
  narrower than it first looks — `PIVOT` and `UNPIVOT` are not symmetric on Trino, unlike every
  other supported backend: `PIVOT (COUNT(id) FOR cat IN ('a'))` executes cleanly on the same
  coordinator, so `supports_pivot` stays `Native` (`true`) for Trino and only `UNPIVOT` needs a
  refusal. The diagnostic layer (`DiagnosticCode::UnsupportedConstruct`,
  `check_unsupported_constructs`) already refuses both `PIVOT` and `UNPIVOT` for **every**
  backend, because a pivot's output columns depend on data values a compile-time analysis
  cannot see — this clause-level refusal is the backstop for a compile entry point
  (`compile_with_sql`) that runs no diagnostics query and would otherwise print `UNPIVOT`
  verbatim to a coordinator that cannot parse it.

Each is declared as a dialect fact (`SqlDialect::supports_aggregate_filter_clause`,
`SqlDialect::supports_interval_range_frame`, `SqlDialect::supports_unpivot`) and refused at
**compile time** with `UnsupportedOnBackend`, naming the construct, the backend, and — where one
exists — the portable rewrite; never emitted verbatim and left to the engine. The refusal is
checked on the call or clause that carries the construct, not on an enclosing or nested one.

Neither is lowered automatically, and in both cases the reason is that the automatic lowering
would be unsound or unreachable rather than merely unwritten:

- `agg(x) FILTER (WHERE p)` → `agg(CASE WHEN p THEN x END)` is exactly equivalent only for an
  aggregate that **ignores NULL inputs** — true of `MIN`/`MAX`/`SUM`/`AVG`/`COUNT`/`STRING_AGG`,
  false of `ARRAY_AGG`, which would gain one NULL element per excluded row. That property is not
  yet registry data, so the author is told the rewrite rather than handed a silently different
  answer for some aggregates.
- The interval frame's GoogleSQL equivalent exists — wrap the sole `ORDER BY` key in
  `UNIX_MICROS(…)` and state the offset in microseconds, which is exact for a GoogleSQL
  TIMESTAMP — but applying it means rewriting the `OVER` clause's `ORDER BY` as well as the frame,
  and window specs print through `smelt-parser`'s dialect-agnostic `Display` with no dialect seam
  to do that in. The author's own numeric rewrite is accepted meanwhile, and because bound
  derivation reads the **source** CST before any lowering, a later print-time lowering would not
  disturb `max_lookback` derivation.

Trino's own absences, measured directly against a live coordinator
(`docs/outcomes/20260913-trino-target-spine`): no `QUALIFY` clause, no trailing commas in a
select list, and no `UNPIVOT` clause. `QUALIFY` and trailing commas are refused at compile time
the same way as the two constructs above — `SqlDialect::supports_qualify` and
`SqlDialect::supports_trailing_commas` already cover any dialect with the flag `false`, Trino
included, since neither requires a Trino-specific rewrite: `QUALIFY` lowers to the existing
wrap-in-subquery rewrite and a trailing comma is simply never emitted. `UNPIVOT` is refused the
same way, via `SqlDialect::supports_unpivot` (§"Clause-level dialect refusals") — there is no
lowering to admit: smelt already refuses both `PIVOT` and `UNPIVOT` for every target at the
diagnostic layer, because a pivot's output columns depend on data values no compile-time
analysis can see, and a Trino-specific lowering would have to enumerate the `IN`-list values to
name its own output columns — precisely the projection smelt declines to derive
(§"Source-derived projection" in `architecture.md`). `PIVOT` itself is not in this list: measured
against the same coordinator, `PIVOT (COUNT(id) FOR cat IN ('a'))` executes cleanly, so
`supports_pivot` stays `Native` (`true`) for Trino, matching every other supported backend — the
two clauses are not symmetric here, unlike everywhere else they are both supported or both
refused. Measured positively, not negatively, in the same session: `[a, b]` array literal syntax
**does** work on Trino — unlike Spark, which needs `ARRAY(a, b, c)` — so Trino's
`supports_array_literal` verdict is an explicit `Native`, stated rather than inherited by
omission.

### Refusal covers function bodies
A `smelt.define` function call is opaque in the calling model's own CST — the body is inlined at
print time — so a walk over the model tree alone cannot see a refused construct declared *inside*
a function. The compile-path refusal therefore walks **both** the model's tree and the
expanded-source tree, the latter being the model with its function calls inlined. That expansion
is still smelt SQL, before any dialect lowering, so this is not a re-parse of printed output
(`architecture.md` §"Source-derived projection") — it is the same expanded-source pass the
lookback-bound deriver already makes over function bodies. Refusals are deduplicated by
(construct, reason), and the model-tree occurrence is preferred because its span points at the
user's own file rather than into expanded text. This applies to every `Emission::Unsupported`
verdict and every clause-level refusal alike: a body is not an exemption. Trino has two concrete
instances proven this way: a one-argument `LOG(x)` (the `Conditional` verdict's arity-1 arm) and
an `UNPIVOT` clause (`supports_unpivot = false`) are each refused at compile time when written
inside a `smelt.define` function body, not only in a model's own tree.

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

### Template emission
Most of a dialect's differences from smelt SQL are neither a bare rename nor a structural rewrite:
the target has the operation, but spells it around the same arguments differently — `DAY(d)` is
`EXTRACT(DAY FROM d)` on GoogleSQL, `POSITION(needle, hay)` is `STRPOS(hay, needle)`,
`DATE_TRUNC(part, d)` takes its arguments in the other order, `LOG2(x)` is `LOG(x, 2)`,
`GROUP_CONCAT(x, sep)` on Spark SQL is `CONCAT_WS(sep, COLLECT_LIST(x))`, and DuckDB's missing
`DATE_SUB(d, i)` is the infix `d - i`. Such a difference is stated as a **template verdict**: registry
data holding the target spelling with positional placeholders, interpreted by exactly one generic
printer routine. The registry owns *what* the target spells; the printer owns only the mechanics of
substitution, and knows no function names.

**Template form.** A template is a string of target-dialect text in which `{n}` names the call's
`n`-th positional argument (zero-based). It may reference any argument any number of times and may
carry arbitrary literal text around them. Every argument the call supplies must be referenced at
least once; a template that would silently drop an argument is malformed. Templates are static
registry data and are validated when the registry is built — every placeholder index within the
entry's declared arity, balanced parentheses, and no argument unreferenced — so a malformed
template is a build-time failure, never a runtime one. A template used as an operand-conditional
arm's verdict (§"Operand-conditional verdicts") is validated by these same rules, at the same
registry-construction time, against the entry's own signature.

**Substitution preserves precedence.** A placeholder is replaced by the argument's *printed* text —
lowered for the same dialect, recursively, so nested calls and operators inside an argument receive
their own verdicts. An argument that is a compound expression (any operator, `CASE`, `CAST`,
comparison, …) is parenthesised on substitution; an atom (a literal, an identifier, a call) is
not. A template whose outermost form is not itself a single call — `DAYOFWEEK({0}) - 1`,
`{0} - {1}` — is emitted wrapped in parentheses, so its result composes correctly wherever the
original call stood. Both rules are decided from structure, never from the template's text, so a
template author need not reason about the precedence of the context a call appears in.

**A template applies to a plain positional call, and refuses everything else.** A call carrying a
modifier the template cannot carry — `DISTINCT`, `FILTER (WHERE …)`, `WITHIN GROUP (ORDER BY …)`,
an `ORDER BY` inside the argument list, `IGNORE NULLS`/`RESPECT NULLS`, a named (`=>`) argument,
or a `*` argument — is refused at compile time with `UnsupportedOnBackend`, naming the modifier,
before the printer runs. Dropping a modifier would change the answer (a dropped `DISTINCT` counts
duplicates; a dropped `FILTER` aggregates rows the author excluded), so refusal is the only safe
outcome. A call's own `OVER` clause is not a modifier of the call: it prints unchanged after the
template's output, exactly as it does for an in-place rewrite. Because `(expr) OVER (…)` is not
valid SQL, a template stated at a window position (or at `Any` for an entry that can occupy one)
must be a single call form; the registry validation enforces that too.

**Templates replace bespoke rewrites, not the other way round.** A rewrite whose output is a fixed
shape over the call's own positional arguments is stated as a template. `RewriteId` is reserved for
lowerings that must read call structure a placeholder cannot name — a `WITHIN GROUP` sort key, a
position-dependent shape, an argument that must be inspected rather than copied. Adding a
`RewriteId` variant where a template suffices is a review error: it moves per-function knowledge
back into the printer, which is what single ownership forbids.

A template is subject to the same position scoping as every other verdict (§"Emission is scoped to
call position"), is probed by the audit like any rewrite (§"Cross-engine emission audit"), and is
rendered as `template` in the coverage table. On DuckDB, print-level identity holds because a
template is never the verdict for a `Native` entry.

### Operand-conditional verdicts
Some verdicts depend on how a call is made, not only on where it sits. GoogleSQL's `MOD` takes
`INT64` or `NUMERIC` and refuses `FLOAT64`. `LOG(x)` is base-10 on DuckDB and natural on Spark SQL
and GoogleSQL, so its one-argument form lowers to `LOG10(x)` — but the two-argument
`LOG(base, x)` is native on Spark and argument-reversed on GoogleSQL. Spark's `TRUNC` is temporal
only, so a numeric operand needs a different answer from a date. Spark's `TO_JSON` takes a struct or
array, not a scalar. `//` is correct on no other engine without knowing whether its operands are
integral. None of these is expressible as one verdict per `(dialect, position)`.

**An entry may state its verdict as an ordered list of arms.** Each arm is guarded by a call
**arity** and/or a per-argument **operand class**, and carries an ordinary verdict — `Native`,
`Rename`, `Template`, `Rewrite`, `Restructure`, or `Unsupported`. The first arm whose guard the call
satisfies is the verdict. A conditional entry must end in an explicit `otherwise` arm; a conditional
with no `otherwise` is malformed and fails registry validation. The registry's own signature must
admit every arity a guard names, so an arity arm cannot claim a call shape the entry does not
otherwise recognise.

**Operand classes.** An argument's class is a pure, total function of its inferred `DataType`,
single-owned beside the registry: `Integral` (the integer widths), `Decimal`, `Floating` (`Float`,
`Double`), `String`, `Boolean`, `Temporal` (`Date`, `Time`, and the timestamps), `Interval`,
`Composite` (`Array`, `Struct`, `Map`), `Binary` (`Blob`), and `Unresolved` for an argument whose
type inference reports `Unknown`. `Null` also classifies as `Unresolved` — a NULL literal
discriminates nothing, and sending it to `otherwise` is the fail-safe direction this axis's cost
rule already demands. A guard names classes, never concrete types, because the engine behaviours
this axis exists for split along exactly these lines and a finer key would multiply arms without
buying a different answer.

**Resolution happens on the compile path, never in the printer.** Arity is read from the source
CST. Operand class is read from the same type inference that derives the model's projection
(§"Output-schema type conformance"), over the same source CST. The compile path resolves every
conditional entry at every call site to one settled verdict *before* printing, and hands the
settled verdicts to the printer alongside the restructure plans. The printer holds no type context
and cannot ask for one: a conditional verdict that reaches the printer unresolved is a bug in the
compile path, not something the printer works around. This is the same discipline as position —
decided once, from the source, handed to the registry lookup — extended to the two further inputs
the lookup now takes.

**The `otherwise` arm is where an unresolved operand lands, and it is constrained by what a wrong
guess costs.** If the arm's lowering, applied to an operand of the wrong class, fails loudly on the
engine (a type error at prepare or execution), the `otherwise` arm may be that lowering — `%` on
GoogleSQL takes `MOD`, because `MOD` on a `FLOAT64` is refused by the warehouse. If a wrong guess
would instead return a different number and succeed, the `otherwise` arm **must** be `Unsupported`
with a reason that names the argument whose type could not be resolved — `//` on any non-DuckDB
target is the standing example, because integer division applied to a floating-point operand
computes silently. This rule is what keeps the fail-loud discipline intact across the axis: an
unresolved type can cost a diagnostic or a loud engine error, never a quiet wrong answer.

**The audit probes every arm.** The fixture carries one typed column per class, so an arm guarded
on a class is probed with that class's column, an arm guarded on an arity with a call of that
arity, and the `otherwise` arm with whichever class no earlier arm names. No typed fixture column
can classify as `Unresolved` — a NULL-bearing column still has a declared type — so an arm guarded
on that class is probed with a bare `NULL` literal instead, which is what the class means at a call
site. Coverage totality counts arms, not entries: an arm no probe reaches is named by the gate,
never skipped. The ledger keys a row on the arm it describes, and the coverage table renders a
conditional cell as the set of its arms' verdicts, as it already does for a cell whose positions
differ.

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
`PERCENTILE_CONT`/`PERCENTILE_DISC` with no window form. Trino has neither shape: measured live,
`MAX_BY`/`MIN_BY`/`APPROX_DISTINCT` are window functions in every position on Trino, whole-partition
and running-frame both, so no `Emission::Restructure` verdict exists for Trino today — its
`ARG_MAX`/`ARG_MIN`/`APPROX_COUNT_DISTINCT` entries carry a single `Position::Any` `Rename`, the same
shape as their Spark verdicts. `AnalyticToCte`'s dialect list is not to be read as exhaustive by
omission because of this: a future built-in may still need it on Trino, and the standing coverage
gate over every `(entry, dialect)` pair (`docs/outcomes/20260913-trino-emission`) is what enforces
that a new one cannot fall through to an unverified implicit `Native`, not this enumeration.
`APPROX_COUNT_DISTINCT` is the sharp case on GoogleSQL:
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
  `IS NOT DISTINCT FROM` on DuckDB, GoogleSQL and Trino and `<=>` on Spark SQL; the difference
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

### Frame elision on offset functions

An offset built-in the SQL standard defines to ignore its window frame — `LAG`/`LEAD` are the
declared instance — may be registered, per dialect and per position, to emit with its `OVER`
clause's frame removed rather than printed. Spark refuses any frame on `lag`/`lead` outright
("Cannot specify window frame for lag function"); DuckDB and the standard both compute the same
result with or without one, so dropping the frame on Spark's SparkSQL dialect changes nothing a
caller can observe. `LAG` and `LEAD` carry this verdict at `(DialectId::SparkSql,
Position::Window)`; every other dialect and position stays `Native`.

The elision is planned from the source CST before printing, exactly like a `Restructure` verdict,
and is never recovered from printed SQL. It touches only the call's `WINDOW_FRAME` sub-node — the
call's own text, and everything else in its `OVER` clause (`PARTITION BY`, `ORDER BY`), is
untouched. The source frame stays in the model's own text; a downstream reader (a derived lookback
bound, a human reading the model) still sees it. A call reached through a *named* window reference
(`OVER w`, with `w`'s frame declared on a shared `WINDOW w AS (...)` clause) is not eligible for
elision — the frame is not the call's own sibling node — so the frame reaches the target dialect
unchanged and the backend's own refusal, if any, fires there rather than being silently dropped.

### Cross-engine emission audit

Two complementary legs verify what the registry declares:

- **Schema leg** — for each `(entry, dialect)` pair, the probe is compiled and sent to the
  dialect's oracle (DuckDB prepare, Spark `DESCRIBE QUERY`, BigQuery dry run, Trino `/v1/statement`
  execution). The oracle returns
  the output schema; the leg asserts acceptance and compares smelt's inferred type against the
  oracle's report using the existing `compare_types`/`divergences` machinery. Acceptance alone
  catches every missing lowering and every `Unsupported` entry. Trino has no dry-run mode, so its
  schema leg is a real execution against the live coordinator rather than a prepare/describe call
  — the same fixture, just run instead of only planned.
- **Value leg** — the same probe is executed on the target dialect and on DuckDB (the reference);
  rows are compared using a typed comparator (exact for integers, strings, booleans; relative
  tolerance for floats; scale-normalised for decimals; NULL equals NULL; deterministic `ORDER BY`).
  This is the leg that catches the `^` class of silent semantic divergence. Trino's value leg
  compares against DuckDB as reference, exactly like every other dialect.

**The implicit-`Native` hole is closed by rule, not by diligence.** `Signature::emission_at`
returns `Native` for any `(dialect, position)` pair carrying no registered entry, which means a
new dialect can silently claim every built-in is natively spelled the moment its `DialectId`
exists — a claim no probe has tested. The coverage-totality gate distinguishes three outcomes
per `(entry, dialect)`, not two: **passing** (an explicit verdict exists, or a `Native` claim has
been executed by the audit and observed correct), **gap** (a registered `Gap { issue }` row, which
does not fail but ratchets down only), and **unverified** (no explicit verdict and no audit
observation backing an implicit `Native` claim) — and `unverified` is a **failure**, named by
entry, distinct from both `passing` and `gap`. This is a general rule over every dialect the
registry supports, not a Trino special case; Trino is simply the dialect whose addition forced it
to be written down, because it is the first dialect added *after* the registry's default already
existed.

A dialect introduced after the registry default *may* record its outstanding `unverified` pairs
in a **shrink-only census file** while its verdicts and audit legs are still being built. The
census is two-sided exactly like the gap ratchet: a pair absent from it fails immediately, so a
newly-added built-in can never silently acquire a claim on the introduced dialect, and a recorded
pair that has since gained an explicit verdict or an audit-leg observation is a stale-census
failure telling you to tighten the file. The census is deleted, not grandfathered, once its count
reaches zero, at which point `unverified` reverts to a plain failure for that dialect with no
census to fall back on.

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
entry × dialect → native / rename / template / rewrite / restructure / unsupported / divergent /
gap. A cell holds one verdict per position where an entry's positions differ — and one per arm
where an entry's verdict is operand-conditional — rendered as the set rather than collapsed to a
single value, because collapsing would hide exactly the aggregate/window (or integral/floating)
asymmetry the position and operand axes exist to record. The table is derived
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
| Schema + value legs, Trino | live Trino/Iceberg (Docker, no credential) | per-PR on Trino-relevant path changes, else labeled PR + nightly (`trino-integration` job, §"CI tiering") |
| Schema + value legs, BigQuery | live BigQuery | manual sweep, `scripts/bigquery-dialect-audit.sh`, gated on `SMELT_BQ_PROJECT` |

BigQuery remains manual, consistent with §"BigQuery has no CI tier, by decision, not by omission".
Trino does not: unlike BigQuery, it needs no cloud credential, so its audit legs run in CI on the
same discipline as Spark's, subject to the never-skip-green rule stated in §"CI tiering".

### Output-schema type conformance
Where a backend's native return type for an expression differs from smelt's inferred type, a
model's **output columns** are reconciled to the inferred type: the compiled SQL is wrapped in an
outer `SELECT CAST(col AS <inferred>) AS col, …` over the model body
(`type_conformance.rs::wrap_with_type_casts`, applied to every backend at compile time —
`smelt-runtime/src/compile.rs`). A model therefore writes the **same schema to every warehouse**,
regardless of engine. This is the multi-backend instance of the canonical-return-type rule in
`functions.md` §"Canonical return types are CAST-enforced"; the backend namespace
(`spark.ceil(...)`, `bigquery.sum(...)`) is the explicit per-call opt-out that inherits the
engine-native type and marks the model non-portable.

The per-dialect cast-target spelling applies to **every** cast smelt prints, not only the cast
wrap's own synthesized casts: a cast target written in a model's own SQL (`CAST(x AS VARCHAR)`,
`x::VARCHAR`) is spelled the same way. On the SparkSQL dialect an unqualified string cast target
(`VARCHAR` with no length, `TEXT`) prints as `STRING`, because Spark treats bare `VARCHAR` as the
read-compatible char/varchar family and refuses it as a cast target
(`[DATATYPE_MISSING_SIZE]`); a length-qualified `VARCHAR(n)` is unchanged. One function
(`type_conformance.rs`) owns this spelling for both the cast wrap and a source-written cast
target, so the two can never diverge.

The column names and inferred types the cast wrap uses are derived from the model's **source**
select list — the CST as written, before dialect lowering. The dialect printer's rendered output
is never re-read to recover a projection: a backend-lowered expression (a BigQuery `MEDIAN`
rewritten to an `ARRAY_AGG`-indexing form, `%` rewritten to `MOD()`, and so on) does not parse
back as the SQL smelt's own grammar accepts, so reconstructing names or types from it is not a
source of truth smelt can rely on. The projection is derived once, from the pre-print CST, and
every consumer — the cast wrap and the output column list alike — reads that single derivation.
A standing gate (`cargo test -p smelt-runtime --test projection_dialect_invariance`) proves this
mechanically: one model exercising every construct the printer lowers compiles for DuckDB, Spark
SQL, BigQuery and Trino, and its `output_columns` and cast-wrap column names are byte-identical
across all four.

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

The type-property oracle (`cargo test -p smelt-db --test type_property_tests`) is the mechanism
that keeps this section's claims honest against live engines: it compares smelt's inferred type
for each generated expression against **four** live oracles — DuckDB (always), Spark, BigQuery,
and Trino (via `/v1/statement` schema metadata) — whichever of the latter three have their
environment gate set. A tolerated difference is a registered `divergences.rs` entry keyed per
backend (`duckdb_type`/`spark_type`/`bigquery_type`/`trino_type`), and an inferred `Unknown` is a
registered `known_unknowns.rs` entry; the string-family `Text`/`Varchar` leniency is the only
blanket rule, applied uniformly across all four backends. A Trino column type smelt's Arrow
mapping (`trino_type_to_arrow`) cannot decode is **fatal** to the leg, never a skipped case: the
coordinator accepted and executed the query, so an undecodable result is smelt's own mapping gap,
not the engine's rejection of the SQL.

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
  transform's licence names). Trino is now the second backend measuring `false` here (`WHEN NOT
  MATCHED BY SOURCE THEN DELETE` refuses with `mismatched input 'BY'. Expecting: 'AND', 'THEN'`,
  `docs/outcomes/20260913-trino-incremental/phases/01-summary.md`), and the same consequence
  applies — but over the **non-atomic, `TargetSchema`-resident** staged group Trino builds
  (`staged_relation_residence = TargetSchema`, `staged_relation_group_is_atomic = false`, below),
  never the session-temporary, atomic group every other `false` case so far has had: the three
  recovery obligations that non-atomic shape already carries (every stage statement precedes any
  target mutation; the relation's name is derived, deterministic, and collision-safe; the group
  reclaims its own relation with a leading `DROP ... IF EXISTS`) cover the departed-row delete
  too, since it executes inside the same non-atomic group as every other statement there. The
  clause forms phase 1 measured **accepted** are the vocabulary the merge-less conditional write
  is built from: `WHEN MATCHED THEN DELETE` (the delete arm, used in place of the absent `BY
  SOURCE` clause), a `WHEN MATCHED AND <pred>` guard (confirmed to restrict which matched rows
  update), two ordered `WHEN MATCHED` arms resolving **first-match-wins** (confirmed: the first
  matching arm's action applies, never the second), and a subquery `USING (SELECT … FROM
  <staged>) s` source (matching the shape T3's staged relation presents, as opposed to a `VALUES`
  list).
- **`supports_staged_relation_group`** — the backend can execute a statement group built around a
  named staged relation (`CREATE` it, populate it, run dependent statements against it, `DROP`
  it). Gates the staged-candidate conditional DELETE+INSERT — the merge-less realisation of
  change-suppressed writes, and the only conditional write path available to a backend with
  `supports_merge = false` (Spark-over-Parquet). This flag conflated two distinct facts until
  `20260913-trino-ledger` split them, once a backend existed (Trino/Iceberg) whose answer to each
  differed from every prior backend's:
  - **`staged_relation_residence`** (`StagedRelationResidence::SessionTemporary` |
    `TargetSchema`) — where the staged relation lives. `SessionTemporary` on every backend with a
    session temp namespace (DuckDB, both Spark profiles, BigQuery, Databricks): `CREATE TEMP
    TABLE`, implicitly dropped with the session. `TargetSchema` on Trino, which has none: a real,
    explicitly-named, explicitly-dropped table in the target's own schema, carrying the same
    `__smelt_staged_`/`__smelt_diff_patch_`/`__smelt_repair_` name prefixes the session-temporary
    form uses, derived once by `StagedRelation::derive` (`smelt-logical`) rather than re-spelled
    per emitter.
  - **`staged_relation_group_is_atomic`** — whether the group (`CREATE`, populate, use, `DROP`)
    can run as one atomic backend transaction. `true` everywhere the relation is session-temporary;
    `false` on Trino, which has no transactional write capability at all, not even single-statement
    DDL inside an explicit transaction (`docs/outcomes/20260913-trino-ledger/phases/01-summary.md`
    — every write form Trino's Iceberg connector was probed with refused with "Catalog only
    supports writes using autocommit"). A group built for a non-atomic capability is emitted with
    `StatementGroup::transactional = false`; `Backend::execute_statement_group`'s default
    (sequential, uncoordinated) implementation is the correct execution shape for it, not a silent
    downgrade.

  A non-atomic group carries three recovery obligations no session-temporary group needs, since
  the one-transaction guarantee that would otherwise make them moot is unavailable
  (`model_transforms.md` §"The staged-candidate conditional DELETE+INSERT" states the full
  contract): every stage statement precedes any target mutation; the relation's name is derived,
  deterministic per (purpose, target table), and prefixed so it can never collide with a user
  model; and the group reclaims its own relation with a leading `DROP ... IF EXISTS` before its
  own `CREATE`, so an orphan left by a run interrupted between the stage and the apply is never
  adopted as live data by a later run. Concurrent runs of the same model are excluded by the state
  lock (`run_state.md`'s state locking), never by the relation's name.

These flags live in `BackendCapabilities` itself, queried by admission exactly like every other
capability flag above — never re-derived by a consumer. `supports_column_scoped_merge`,
`staged_relation_residence` and `staged_relation_group_is_atomic` are struct fields;
`supports_merge_not_matched_by_source` and `supports_staged_relation_group` are specified ahead of
their own struct fields (see §Known Divergences).

### The fingerprint sidecar capability

- **`supports_fingerprint_sidecar`** — the backend can build and diff the fingerprint sidecar
  (`sources.md` §"The fingerprint sidecar") that synthesizes an exact changed-key delta for an
  external `mutable_snapshot` source with no native change feed. `true` for DuckDB alone today.
  The flag is queried by admission exactly like every other capability flag — never re-derived by
  a consumer from the target's dialect. A flagless target's consequence differs by leg:
  - **External delta restriction** (a mutation-sensitive cell driven by such a source): keeps the
    widened-scan recompute — the gap is declared here, never a silent narrowing a consumer has to
    discover.
  - **Repair-family / model-edge group-grain recompute**: refuses with `UnsupportedOnBackend`
    rather than falling back to a widened scan, because a clamped current-source scan over a
    `mutable_snapshot` is unsound on this leg, not merely wider.
  The sidecar's own DDL (`smelt-state`'s `ddl_duckdb::generate_fingerprint_sidecar_table_ddl`) is
  DuckDB-shaped; a second backend declaring the flag needs its own DDL before the flag can be set
  for it. `capability_conformance` pins the current declared matrix (DuckDB alone `true`).

### Whole-row MERGE

The column-scoped merge and keyed-fold emitters upsert a whole row, and the SQL families spell
that in **three** ways, not two. DuckDB and Spark accept `WHEN MATCHED THEN UPDATE SET *` and
`WHEN NOT MATCHED THEN INSERT *`, which name no columns. GoogleSQL accepts neither: `SET *` is a
syntax error, and the whole-row insert is spelled `INSERT ROW`. So BigQuery's matched arm is
rendered column by column, `c = source.c`, over the model's output projection. Trino is a third,
distinct family, not a member of either: measured by executing each form against a live
Iceberg-backed coordinator (`docs/outcomes/20260913-trino-incremental/phases/01-summary.md`),
every star/`ROW` shorthand is a grammar-level parse error on **both** arms —
`WHEN MATCHED THEN UPDATE SET *` (`mismatched input '*'. Expecting: <identifier>`),
`WHEN NOT MATCHED THEN INSERT *` (`mismatched input '*'. Expecting: '(', 'VALUES'`), and
`WHEN NOT MATCHED THEN INSERT ROW` (`mismatched input 'ROW'. Expecting: '(', 'VALUES'`) each
refuse. So Trino, like BigQuery, renders both arms column by column — `UPDATE SET c = source.c`
and the explicit column-list `INSERT (c, …) VALUES (source.c, …)` — over the model's output
projection; unlike BigQuery, Trino has no `INSERT ROW` fallback at all, so its not-matched arm is
never spelled any other way.

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
asserts each constructor sets it and that a first-run model against a fresh schema succeeds. A
`databricks` target issues `CREATE SCHEMA IF NOT EXISTS <catalog>.<schema>` against Unity
Catalog — the catalog-qualified form, since a Unity Catalog schema is always addressed within
a catalog rather than standing alone. A `trino` target issues the same catalog-qualified
`CREATE SCHEMA IF NOT EXISTS <catalog>.<schema>` against the Iceberg catalog, for the same
reason: a Trino schema has no standalone address outside its catalog.

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

A `databricks` target carries the secret differently: the token is not embedded in a URL at
all, but read from its own `token` key (`smelt_yml.md` §"Target shape") and handed to the
Databricks Connect session builder (`DatabricksSession.builder...token(...)`) as a distinct
argument. Because the key *is* the secret rather than a substring of a larger URL value, a
literal (non-`${VAR}`) `token` is not a smell but a **hard configuration error** — there is no
adjacent context in which a literal could be an intentional non-secret value. Whether the
token came from `${ENV}` interpolation or the ambient-credential fallback (`token` absent), it
is never written to a log line, a run report, a diagnostic, or an error message: every
rendering path for a `databricks` target config redacts the field.

A `databricks` target has a second, credential-free form: **both** `host` and `token` absent,
for a target running from inside the workspace it targets, such as a Databricks Job task.
Measured against a real Free Edition serverless job task (`docs/outcomes/
20260912-databricks-dogfood-spine/phases/11c-summary.md`): no host-bearing
`DATABRICKS_*`/`DBX_*` environment variable is exported into the task's runtime at all — the
runtime's own ambient channel is a pre-established Spark Connect session (`SPARK_REMOTE`), not
an env-var host. The ambient form therefore omits `host` as well as `token`: the session is
built with no explicit host or token call on the Databricks Connect builder at all, honouring
whatever workspace context the runtime already established. `token` present with `host` absent
is a hard configuration error naming both keys — a token carries no workspace address, so that
combination cannot be the ambient form and cannot be a working `host`-present form either. This
form carries no secret for a log line to leak in the first place, so the redaction rule above is
vacuous here rather than relied upon — the same code path applies unconditionally regardless of
which form produced the config.

A `trino` target carries its credential the same way `databricks` carries `token`: the
password is not a substring of a connection URL but a distinct `password` key
(`smelt_yml.md` §"Target shape"), sent as HTTP Basic auth alongside `user`. Because the key
*is* the secret, a literal (non-`${VAR}`) `password` is a **hard configuration error**, not a
smell, for the same reason as `databricks`' `token`. `password` absent is Trino's
unauthenticated form — the local Docker tier's default — and carries no secret for the
redaction rule to protect; the resolved password, when present, never reaches a log line, a
run report, a diagnostic or an error message.

### Loading data into a backend
Loading external rows into a backend (seeds, test fixtures, an Arrow batch) must not assume the
backend's process shares the host filesystem. The transfer is performed through the backend's
own client API — for Spark Connect, the rows are sent as an in-memory frame
(`createDataFrame` from Arrow), **not** by writing a host-path file and asking the server to
read it back (`spark.read.parquet('/tmp/…')`), which fails with `PATH_NOT_FOUND` against any
containerized or remote Connect server whose JVM cannot see the host path. This is distinct from
cross-engine *exchange* below, where the shared `warehouse` filesystem is an explicit
requirement; data **loading** carries no such assumption.

On a `databricks` target this in-memory path is not merely preferred but the only one that can
work: serverless compute shares no filesystem with the client at all, so there is no host-path
fallback to reach for even by mistake. Rows load through the session's own `createDataFrame`
from Arrow, exactly as Spark Connect's does; smelt assumes neither a DBFS root nor a Volume
mount is available to write through.

A `trino` target's client protocol is HTTP (`/v1/statement`), with no host-filesystem
assumption of its own. The measured bulk path is `INSERT INTO … SELECT CAST(…) FROM (VALUES
…)`: a staged-Parquet-into-object-storage path was considered and rejected, because a `trino`
target carries no object-store credential of its own (`host`/`port`/`user`/`catalog`/`schema`/
`tls`/`password` only) — writing a Parquet file into the Iceberg connector's backing MinIO
bucket would need a target-shape change wider than loading. Each `INSERT` casts every value
column from an untyped `(VALUES …)` row so a NULL cell and a narrow integer literal both carry
the same type the surrounding `CREATE TABLE` gave the column, rather than an ambiguous type
Trino would otherwise infer from the literal alone. Rows are chunked at 1,000 per statement,
a bound picked from measurement against the live tier (a 12,000-row batch loaded across 12
statements at roughly 6,000 rows/second) rather than from taste — it keeps each statement's
HTTP body and Trino's own parse time bounded regardless of how large the seed batch is.

Reading a value back through this client requires one more header than the write path: without
`X-Trino-Client-Capabilities: PARAMETRIC_DATETIME` on every request, the coordinator drops
`timestamp(p)` (and every other parametric date/time type) to its unparameterized base type for
legacy client compatibility — measured against a live tier, this is not merely a type-name
cosmetic: a `timestamp(6)` value written as `...123456` reads back as `...123`, silently
truncated to millisecond precision, even though the underlying stored value retains full
microsecond precision (confirmed via `to_unixtime`). The client sends this header on every
request, so every backend method that reads a *value* — not only `load_table`'s round trip —
gets full parametric precision.

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

A cross-backend edge into or out of a `databricks` target is **refused with a diagnostic**
rather than compiled to a `read_parquet()` substitution. The substitution's precondition — a
`warehouse` path both processes can read — cannot hold for Databricks: a `databricks` target
has no `warehouse` key at all (it is one of the keys §"Target shape" hard-errors on), and
serverless compute exposes no host-visible file layout for a DuckDB process to read back.

A cross-backend edge into or out of a `trino` target is likewise **refused with a diagnostic**.
The same precondition fails the same way: a `trino` target has no `warehouse` key at all (it is
one of the keys §"Target shape" hard-errors on), and the object storage behind the Iceberg
catalog is not a host filesystem path a DuckDB process could read back.

### Incremental & schema evolution per backend
Strategy *resolution* (`incremental_models.md`) and change *classification*
(`schema_evolution.md`) consult the capability matrix but are specified in those documents.
This spec only requires that the resolved strategy and migration plan are expressible in the
target backend's physical SQL via the lowering rules above — e.g. a backend without native
`INSERT OVERWRITE` resolves to `DeleteInsert`; a backend without `ALTER COLUMN … USING`
resolves nested widening to a table rewrite.

The `trino` target realises none of the five correctness structures `state.md`'s table
(§"Which dialects realise which structure") tracks — the Iceberg connector accepts writes only
in autocommit, so a ledger write and its data write can never commit together. Every cell that
would otherwise depend on one of those structures takes the degradation contract's
recompute-family downgrade instead, carrying `MaintenanceStateDowngraded`; it is never refused.
Which landing state that is, per `Technique`, is derived once by
`smelt_logical::maintenance::availability` rather than asserted here:

| `Technique` | On `trino` | Diagnostic |
|---|---|---|
| `DeleteInsert` | reachable; the write window is emulated (`DELETE` + `INSERT`), since `supports_insert_overwrite` is `✗`. The pair executes sequentially rather than in one transaction, since Iceberg writes are autocommit-only (`incremental_shapes.md` §"First-run and backfill"); an `INSERT` failure leaves the chunk's window empty rather than rolling back, which re-running the same window restores because the `DELETE` covers exactly the window the `INSERT` writes | — |
| `PerGroupRecompute`, no `key_scope` | reachable | — |
| `PerGroupRecompute`, key-addressed (`UpstreamKeyed` / `DownstreamGrainOverUpstream`) | refused — needs the fingerprint sidecar, and a clamped current-source scan is unsound here, not merely wider | `UnsupportedOnBackend` |
| `KeyedFold` | downgraded to its recompute-family equivalent — no reconciliation ledger, so no never-fold-twice refusal | `MaintenanceStateDowngraded` |
| `ColumnScopedMerge`, `InPlaceUpdate` | downgraded — no transactional merge ledger, exactly as on Spark (Delta) | `MaintenanceStateDowngraded` |
| `SuccessionPatch` | downgraded to `DeleteInsert` (full rebuild), never a ledger-less patch | `MaintenanceStateDowngraded` |

`supports_column_scoped_merge = ✓` and the `ColumnScopedMerge` row above are **not** in conflict:
the flag describes a statement shape Trino can execute (§"Whole-row MERGE"'s column-by-column
form), while the plan cell's technique demands a correctness structure — the transactional merge
ledger — the dialect does not realise, and admission asks the second question after the first.
Trino introduces **no new diagnostic code**: the three codes named above —
`MaintenanceStateDowngraded`, `UnsupportedOnBackend`, and `DeclaredContractRequiresState` (used by
`contract.deferral`'s refusal, below) — cover every route in the table, so no later phase may mint
a Trino-specific one.

Schema evolution is a separate axis: Iceberg supports it directly, and its measured
`SchemaOperation` mapping lands in `ddl_trino`
(`docs/outcomes/20260913-trino-ledger/outcome.md`). The contract lattice degrades the same way:
`contract.deferral` is a statement about state (the reconciliation ledger's frontier) and
refuses with `DeclaredContractRequiresState` on Trino exactly as on Spark, while
`contract.frozen_horizon` and `contract.retain_departed` are statements about the model's own
SQL and stay admitted — `frozen_horizon`'s late-arrival verification probe renders on Trino
like it does on any other dialect with a `MaintenanceDialect` (the skip-with-run-time-warning
route in `contract_probes.rs` remains for a dialect that still has none). The staged-candidate
conditional
DELETE+INSERT's staged relation is realised too, over a non-temp, non-atomic residence
(`staged_relation_residence = TargetSchema`, `staged_relation_group_is_atomic = false` — §"Column-
scoped merge and conditional-write capabilities" above): a real, explicitly-named,
explicitly-dropped table in the target's own schema, reclaimed with a leading `DROP ... IF EXISTS`
before every run so an orphan left by an interruption between the stage and the apply is never
adopted as live data by a later run.

## Design

- **Why Databricks is a distinct target type.** A Databricks Free Edition workspace is
  serverless-only, Unity-Catalog-mandatory, and exposes no host-visible warehouse directory —
  none of which the existing `spark` target models. `type: spark`'s `connect_url` hands a raw
  `sc://` URL straight to PySpark's `builder.remote()`, which cannot address serverless
  compute at all; reaching it requires the `databricks-connect` client's `DatabricksSession`
  builder, a distinct package that conflicts with the plain `pyspark` the local-Spark path
  pins. Rejected: a `serverless: true` flag on `type: spark`. That would have kept
  `warehouse:` and `format:` as legal-but-broken keys on a Databricks target — configuration
  that parses, looks plausible, and silently does nothing (or fails deep in a session builder)
  because serverless compute has no warehouse path to write and no format choice to make. A
  distinct `databricks` type turns that gap into a set of hard errors named at config-load time
  (§"Target shape" refusals) instead of a runtime surprise, and lets the capability profile and
  connection shape diverge from Spark's without contorting one type to cover two backends.
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
- **Template interpretation is generic; per-function spelling is registry data.** The printer's
  template routine substitutes printed argument text into placeholders and applies the structural
  parenthesisation rules of §"Template emission"; it matches no function name and reads no template
  text to decide behaviour. Every `RewriteId` variant's doc comment states which call structure a
  placeholder could not name — the reason it is not a template. Malformed templates (an index
  beyond the arity, an unreferenced argument, a non-call form at a window position) fail registry
  construction, so the printer never sees one. Gate: `cargo test -p smelt-dialect --test
  emission_ownership` (no name-matched arm in the interpreter; every `RewriteId` justified) plus a
  registry-validation test that builds the full registry.
- **Operand-conditional verdicts are settled on the compile path; the printer receives no
  conditional and holds no type context.** Arity and operand class are resolved from the source CST
  and the projection's own type inference before printing, and the printer is handed one verdict
  per call site. Every conditional entry ends in an `otherwise` arm, and that arm is `Unsupported`
  wherever a misclassified operand would compute a different number rather than fail loudly on the
  engine (§"Operand-conditional verdicts"). Gates: `cargo test -p smelt-runtime --test dialect_seam`
  pins that an unresolved operand on a wrong-number entry is refused at compile time and that no
  compile entry point reaches the printer with a conditional unsettled; the audit's coverage
  totality counts arms, so an arm no probe reaches fails per-PR.
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

- **The BigQuery conformance leg's live evidence has a date.** The last all-green live sweep of
  `crates/smelt-cli/tests/maintenance_conformance_bigquery/` against a real warehouse is
  2026-08-22 (22 cases, 621.61s, 4-way concurrent); every commit since is verified offline
  only. A re-sweep is owed whenever maintenance emission or the shared
  `smelt-maintenance-testkit` render surface changes again. Between sweeps, the offline gates
  standing in for a live re-run are `cargo test -p smelt-maintenance-testkit --test
  googlesql_render` (every DAG-body and composed-pool rendered body prints clean GoogleSQL),
  `cargo test -p smelt-dialect --test modulo_lowering --test power_lowering` (the `%`/`^`
  lowerings those bodies depend on), `cargo test -p smelt-backend --test
  merge_columns_guard` (`require_merge_columns`), and
  `no_family_hardcodes_a_backend_dialect` (`crates/smelt-maintenance-testkit/src/families/mod.rs`,
  `dags.rs`) — none of them substitutes for the live leg itself, only for the specific defect
  classes a prior live sweep found and fixed.

- **`%` on BigQuery still lowers to `MOD` for every operand (#173).** GoogleSQL's `MOD` accepts only
  `INT64`/`NUMERIC` and fails at the warehouse on a floating-point operand; `Emission::Conditional`
  exists and is populated for `//`, `LOG`, `TRUNC` and `TO_JSON` on Spark, but `%` on BigQuery has
  not yet been given the same operand-conditional treatment. `//`'s own per-class arms are stated
  and verified live (`docs/outcomes/20260904-dialect-emission-vocabulary` phase 7); BigQuery's
  remains open, tracked in the same outcome.

- **A clause GoogleSQL lacks is refused rather than lowered (#200, #201).** Both
  §"Clause-level dialect refusals" constructs stop at compile time on BigQuery, so a model valid
  on DuckDB and Spark stays unrunnable there until its author rewrites it. The aggregate
  `FILTER (WHERE …)` clause waits on a null-input disposition in `BuiltinRegistry` — the
  `CASE WHEN` rewrite changes `ARRAY_AGG`'s answer, so it cannot be applied blindly (#200). The
  `INTERVAL`-offset `RANGE` frame waits on a window-spec dialect seam — the exact GoogleSQL form
  needs the `OVER` clause's `ORDER BY` rewritten too, and window specs print through
  `smelt-parser`'s dialect-agnostic `Display` (#201). The concrete cost is two models of
  `examples/github_activity` (`silver.actor_sessions` and its downstream) that build on DuckDB
  and not on BigQuery.

- **A Trino `array(...)` result column does not decode to Arrow yet.** `trino_type_to_arrow`
  maps the `array(...)` type *signature* to `DataType::List` correctly, so a query's declared
  schema is right; the gap is narrower than that — the result-page *cell* decoder
  (`build_column` in `crates/smelt-backend-trino/src/arrow_convert.rs`) has no `DataType::List`
  builder arm, so a model projecting an array-typed column reads back with an error rather than
  an Arrow list value. No plan is yet tracking the close.

- **`supports_transactional_ddl = false` measures smelt's client, not Trino's grammar.** The
  measured `Client does not support transactions` error comes from smelt's stateless
  `/v1/statement` HTTP client having no session continuity across `START TRANSACTION`/DDL/
  `ROLLBACK`, not from Trino rejecting the statements themselves. Stated so a later outcome does
  not spend effort "fixing" Trino for a limitation that is smelt's client design. Owner:
  `docs/outcomes/20260913-trino-ledger/`.

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
- **The Databricks capability matrix column is measured against a live workspace for the
  flags the `github_activity` dogfood pipeline exercises; the rest remain inherited.** A full
  refresh, eleven consecutive incremental windows, a dual-target parity sweep against DuckDB,
  and three full-refresh-oracle equivalence checks all ran live against a Databricks Free
  Edition workspace. That live run measured Delta/Unity-Catalog semantics, `merge_into`,
  column-scoped merge, `IS NOT DISTINCT FROM` null-safe equality, and Unity Catalog's
  schema-evolution DDL — every flag `examples/github_activity`'s 16 models reach. Flags no
  model in that pipeline exercises (e.g. the merge-not-matched-by-source and staged-relation-
  group rows) remain inherited from the Spark (Delta) column, unverified live. Evidence:
  `docs/outcomes/20260912-databricks-dogfood-spine/phases/08-parity.json`,
  `phases/09b-equivalence.json` (dated 2026-09-13),
  `docs/handoffs/2026-09-13-databricks-findings.md`.
- **No cross-engine exchange for Databricks.** §"Cross-engine data exchange" refuses a
  cross-backend edge into or out of a `databricks` target outright; a Volumes-based exchange
  path is a later design, not attempted here.
- **A succession-grain cell with no realisable `TombstoneLedger` (every Spark/Databricks
  target today) rebuilds the whole presented table on every incremental window, never a
  window-forward patch.** `docs/specs/state.md` §"The degradation contract" states the
  mechanism; the cost is O(source) per window rather than O(window), invisible at the
  `github_activity` fixture's scale but a real trade-off at production scale. Measured live on
  `silver.actor_naming`: this fixed a live 7x row-duplication defect on the incremental write
  path (`docs/handoffs/2026-09-13-databricks-findings.md`, "Defects fixed in place" item 6)
  rather than merely documenting a pre-existing cost. Whether Databricks' Catalog Commits
  feature can unlock a
  real `MergeLedger`/`TombstoneLedger` on this backend specifically — closing the gap at the
  root rather than accepting the cost — is open triage, not investigated here; see the handoff.
- **`gold.events_enriched`'s key-addressed model-edge cell downgrades to `DeleteInsert` at
  plan-derivation time on Delta/Databricks, rather than realising a fingerprint sidecar.**
  §"The fingerprint sidecar capability" states the flagless-target consequence for a
  `mutable_snapshot`-driven repair-family/model-edge cell as an execution-time
  `UnsupportedOnBackend` refusal; for this cell's shape (`KeyDiscovery::EnrichmentKeyed`, whose
  ideal technique is `ColumnScopedMerge`, not a plain `PerGroupRecompute`) availability
  resolution instead downgrades it to `DeleteInsert` before execution, per
  `docs/specs/state.md` §"The degradation contract". The trade: a `DeleteInsert` cell of this
  shape forgoes the unwindowed, run-level heal a `ColumnScopedMerge` cell performs on every
  run, producing a small, bounded, registered `UnorderedColumnDivergence` against a
  ledger-backed target — measured and bounded, not a correctness gap. See
  `docs/handoffs/2026-09-13-databricks-findings.md` finding 1.
- **Databricks has no native-IVM emission.** `supports_native_ivm` is `false` for
  `databricks()` — smelt emits no Enzyme statements, so `refresh: materialized_view` hard-errors
  on this backend exactly as it does on DuckDB and both Spark profiles.
- **The Statement Execution API is not a Databricks connection path.** Databricks Connect
  (`DatabricksSession`) is the only modelled way to reach a `databricks` target; a SQL-warehouse
  connection via the Statement Execution API is a separate, unstarted design.
- **Databricks paid-tier compute shapes are unmodelled.** The `databricks` target is specified
  against Free Edition's serverless-only, Unity-Catalog-mandatory shape. Classic clusters,
  instance profiles, and private networking on a paid workspace are out of scope until a
  concrete need arises.
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
  `crates/smelt-state/src/ddl_spark.rs`, `crates/smelt-backend-trino/`.
- **Tests**: `crates/smelt-cli/tests/multi_engine_test.rs`,
  `crates/smelt-backend-spark/tests/load_table.rs`, `crates/smelt-backend-spark/src/tests.rs`,
  `crates/smelt-db/tests/prop_helpers/spark_oracle.rs`,
  `crates/smelt-oracle-testkit/src/trino_oracle.rs`,
  `crates/smelt-db/tests/type_property_tests.rs` (four oracles: DuckDB, Spark, BigQuery, Trino).
- **User docs**: `docs-site/docs/` backend / targets pages.
- **Plans (history)**: `docs/plans/20260328-multi-engine-example.md`,
  `docs/plans/20260628-spark-parity.md`,
  `docs/outcomes/20260913-trino-target-spine/outcome.md`,
  `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **Related specs**: `architecture.md` (§"Backend trait surface"), `smelt_yml.md`
  (§"Target shape"), `incremental_models.md`, `schema_evolution.md`, `testing.md`,
  `types.md`.
