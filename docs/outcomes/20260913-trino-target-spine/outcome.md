# Outcome: A `type: trino` target exists, stands up from `docker compose`, and materializes a model as an Iceberg table

**Created:** 2026-09-13
**Status:** active
**Driver:** loop. Nothing here needs a cloud account, a credential or a human gate — only
Docker. Phases that need the live server must emit `<<PHASE_BLOCKED>>` when
`scripts/trino-env.sh` cannot reach it, **never skip green**: an unset `SMELT_TRINO_URL` that
silently passes is the same hole an unset `DUCKDB_LIB_DIR` opened.
**Source:** the five-outcome Trino programme agreed 2026-09-13 (this is T1 of T1–T5; siblings:
`20260913-trino-emission`, `-trino-ledger`, `-trino-incremental`, `-trino-dogfood`).
Pattern followed: `docs/outcomes/20260906-bigquery-dogfood-spine` and
`docs/outcomes/20260912-databricks-dogfood-spine` (how a fourth backend was landed), and
`scripts/spark-up.sh` (how a Docker-resident engine tier is scripted).
**Spec anchors:** `docs/specs/multi_backend.md` §Surface (backends, capability matrix),
§"Session initialization", §"Connection security", §"Loading data into a backend",
§"Incremental & schema evolution per backend", §Known Divergences;
`docs/specs/smelt_yml.md` §"Target shape"; `docs/specs/seeds.md` §"Type inference";
`docs/specs/diagnostics.md`

## The outcome

smelt has a fifth backend and a fourth SQL dialect. A target declaring `type: trino` names a
coordinator, a catalog and a schema, authenticates from an explicitly-supplied credential, and
runs a model to completion as an Iceberg table — reached over Trino's own HTTP protocol from
pure Rust, with no Python interpreter, no venv and no third-party client in the path.

The engine it runs against is a committed, pinned `docker compose` tier — a Trino coordinator,
an Iceberg REST catalog and MinIO object storage — brought up by one script in the shape
`scripts/spark-up.sh` already established, so a contributor and CI stand up the same thing.
Iceberg is the connector *because* the connector is what decides Trino's write surface: `MERGE`,
`UPDATE`, `DELETE` and `CREATE OR REPLACE TABLE` exist on Iceberg and do not exist on Hive, so
the whole incremental and ledger story downstream of this outcome depends on that choice.

Crucially, the capability profile is **measured, not read**. `BackendCapabilities::trino_iceberg()`
is populated by executing the statement each flag names against the live coordinator and
recording what happened — the discipline `multi_backend.md` §Surface already requires ("A
backend's column is established by executing the statement each flag names against a live
instance of that backend, never by reading its documentation"). Trino is expected to be the
first backend in the matrix with **no `PIVOT`**, no `QUALIFY`, no `::` cast, no `[a,b]` array
literal, no `INSERT OVERWRITE`, no transactional DDL and **no temporary tables**; each of those
is confirmed by execution here and its consequence handed to the sibling outcome that owns it.

The useful prior is that **Trino sits near Spark (Delta)**, not near DuckDB. Iceberg and Delta
have the same atomicity shape — per-table commits, no cross-table transaction — which is why
`20260913-trino-ledger` starts from Spark's state-residency column rather than treating residency
as an open question (ruling of 2026-09-13). That prior tells this outcome where to look hardest:
the cells where Iceberg is expected to be *more* capable than Delta, and the handful where
Trino's SQL surface is poorer than Spark's despite the shared storage semantics. It is a prior,
not an answer — every cell is still established by execution.

## Success criteria (checkable)

1. **The target is specified before it is built.** `docs/specs/multi_backend.md` §Surface and
   `docs/specs/smelt_yml.md` §"Target shape" describe the `trino` target: its keys
   (`host`, `port`, `user`, `catalog`, `schema`, TLS on/off, password via `${ENV}` **only**),
   its `SqlDialect::Trino` dialect, and its `BackendCapabilities::trino_iceberg()` profile.
   Keys belonging to another target (`connect_url`, `warehouse`, `database`, `project`,
   `dataset`) are **refused with a diagnostic, not ignored** — a named `DiagnosticCode` with a
   fixture under `examples/broken/`. The capability matrix gains a Trino column, and the
   §"Connection security" rule is stated for it: the credential never appears in a log line,
   an error message, or a run report.
2. **`DialectId::Trino` and `SqlDialect::Trino` exist and are exhaustive.** `DialectId::ALL`
   carries four variants and `all_is_exhaustive` proves it; the slug is `trino` and round-trips;
   every `match` over `SqlDialect` compiles without a wildcard arm that would have silently
   absorbed the new variant. **This criterion deliberately does not claim emission coverage** —
   `Signature::emission_at` returns `Native` for any `(dialect, position)` pair with no entry,
   so the new variant enters asserting every built-in is natively spelled on Trino. That hole is
   `20260913-trino-emission`'s subject; this outcome must record it in the decision log and in
   `multi_backend.md` §Known Divergences rather than leave it unremarked.
3. **The tier stands up from one script, pinned.** `scripts/trino-up.sh`, `trino-down.sh` and
   `trino-env.sh` bring up and tear down a `docker compose` tier of pinned images — a Trino
   coordinator, an Iceberg REST catalog, and MinIO — with the Iceberg catalog properties
   committed under `scripts/` (not typed by hand), and export `SMELT_TRINO_URL` plus whatever
   else the backend reads. `scripts/README-trino.md` records the version pins and why. The
   script is idempotent and survives container-owned leftovers from a previous run — the
   failure `scripts/spark-up.sh` actually hit, where a `chmod` on a root-owned leftover aborted
   under `set -e` *before* `docker run` and every test then failed with no hint the server had
   never started.
4. **`smelt-backend-trino` implements the whole `Backend` trait over pure Rust HTTP.** Every
   required method (`execute_sql`, `create_table_as`, `create_view_as`, `drop_*`,
   `get_row_count`, `get_preview`, `table_exists`, `ensure_schema`, `dialect`, `capabilities`,
   `load_table`) works against the live tier, driven by `POST /v1/statement` and `nextUri`
   paging with Trino's own result pages decoded to Arrow. Trino error responses map to typed
   `BackendError` variants — `QueryError`, `UnsupportedFeature`, a connection failure — never a
   stringly-typed catch-all, and never a silent empty result. `create_materialized_view_as`
   inherits the erroring default (`supports_native_ivm` is `false`).
5. **The capability profile is established by execution and recorded.** For every flag in the
   §Surface matrix there is a probe that executes the statement the flag names against the live
   coordinator; the resulting column is written into both `BackendCapabilities::trino_iceberg()`
   and the spec table in the same commit, and a conformance test asserts the constructor matches
   the table. Where a flag is `✗`, the measured error is quoted in the decision log. The
   expected-`✗` set above is a hypothesis this criterion tests, not a conclusion it assumes.
6. **Data gets in.** `load_table` lands Arrow `RecordBatch`es at `catalog.schema.name` over the
   seed type set of `seeds.md` §"Type inference", rejecting NULLs in a non-nullable Arrow field
   as every other backend does, and round-tripping each supported type through Trino's own
   type system (`TIMESTAMP(6)`, unbounded `VARCHAR`, `DECIMAL(p,s)`) back to the same smelt
   `DataType`. The chosen bulk path (batched `INSERT … VALUES` versus staging Parquet into MinIO)
   is a decision-log entry with the measured reason, and `seed_parity` covers Trino.
7. **A model actually runs.** An example workspace compiles and materializes on the Trino
   target end-to-end via `execute_project` — a table and a view — with zero diagnostics, the
   rows readable back, and the run report written. Wired into `crates/smelt-cli`'s target-parity
   suite the way Spark's and BigQuery's are.
8. **CI runs it, gated the way Spark's is.** A `compat.yml` job stands up the compose tier and
   runs the Trino integration tests, gated on `schedule`, the `run-docker-tests` label, or a
   `changes` filter for Trino paths — mirroring `spark-integration`. When `SMELT_TRINO_URL` is
   unset the tests **skip** (not fail), and a test proves the skip is a skip rather than a
   vacuous pass.
9. **Gates green.** `bash .claude/scripts/verify-phase.sh` passes and no ratchet is lowered:
   in particular `.claude/hardening-baseline.txt` gains a `smelt-backend-trino` entry rather
   than absorbing new `unwrap`/`expect` elsewhere, and `no_println_in_libraries` stays at zero.
   The existing Spark, BigQuery and Databricks tiers are untouched — the new compose tier binds
   no port and no container name they use.

## Out of scope

- **Every other Trino connector**: Hive, Delta Lake, Memory, JDBC/PostgreSQL, and Trino's
  federation story (querying two catalogs in one statement). Iceberg is the one profile. A
  second connector profile — `spark_delta()`/`spark_parquet()`-style — is a later outcome if a
  real need appears, not a hedge taken now.
- **Emission verdicts.** Which spelling each built-in takes on Trino, the `dialect_audit` Trino
  legs, `dialect-coverage.md`'s Trino column and the implicit-`Native` gate all belong to
  `20260913-trino-emission`. This outcome only makes them *executable* by providing a client.
- **smelt's own state on Trino** (`20260913-trino-ledger`) and **the incremental/maintenance
  families** (`20260913-trino-incremental`).
- **Trino as a parser source dialect.** smelt SQL is the source dialect and Trino is a target
  only, so no `smelt-parser-compat` corpus, differential or gaps-baseline work is implied.
- **Trino materialized views and `REFRESH MATERIALIZED VIEW`** — `supports_native_ivm` stays
  `false`; Trino's MV refresh is externally scheduled, not incremental maintenance.
- **Operating Trino**: fault-tolerant execution, resource groups, spill, coordinator HA,
  performance tuning, a multi-worker cluster. A single-coordinator tier is enough to prove a
  backend.
- **Unattended/scheduled execution** on the tier (the Databricks outcome's criterion 11 shape).

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec delta: the `trino` target shape and capability column in `multi_backend.md` + `smelt_yml.md`, the foreign-key refusal diagnostics, the connection-security rule, and the Known Divergence naming the implicit-`Native` emission hole this outcome does not close | done |
| 2 | `DialectId::Trino` + `SqlDialect::Trino` land with no wildcard match arm anywhere absorbing them; `ALL` exhaustiveness and slug round-trip green; every resulting compile error across the workspace resolved deliberately rather than defaulted | done |
| 3 | `BackendType::Trino` and the `trino` target shape in `smelt-core::config`: the keys parse, a literal password is refused pre-interpolation, every foreign key is named (not the first only), and a committed `examples/` fixture proves the refusal — the implementation half of criterion 1 | done |
| 4 | The Docker tier: pinned `docker compose` (Trino + Iceberg REST catalog + MinIO), committed catalog properties, `scripts/trino-{up,down,env}.sh` idempotent over container-owned leftovers, `README-trino.md` version pins | done |
| 5 | `smelt-backend-trino`: the HTTP statement client (`/v1/statement` + `nextUri` paging, result pages → Arrow, typed `BackendError` mapping, credential redaction) proved by unit tests with no live server | done |
| 6 | The `Backend` trait impl over the live tier: DDL, existence, row count, preview, `ensure_schema`, a table and a view materialized as Iceberg objects and read back through `execute_model`, and the `smelt-backends` factory constructing it by name | done |
| 7 | `load_table`: the Arrow path over the seed type set with the bulk-strategy decision measured and recorded, NULL-in-non-nullable rejection, type round-trip, `seed_parity` Trino leg | done |
| 8 | Establish the capability profile **by execution**: one probe per matrix flag against the live coordinator, plus the two `SqlDialect` *language* properties (`supports_aggregate_filter_clause`, `supports_interval_range_frame`) phase 2 landed conservatively `false`; `BackendCapabilities::trino_iceberg()` replaces phase 6's provisional all-`false` profile and the spec table is written in the same commit, constructor-matches-table conformance test, measured errors quoted for every `✗` | done |
| 9 | End-to-end on the real pipeline: `dialect_and_capabilities` stops refusing Trino, an example workspace compiles and materializes a table and a view on the Trino target via `execute_project` with zero diagnostics and a run report written, wired into `smelt-cli`'s target-parity suite the way Spark's and BigQuery's are — criterion 7 | pending |
| 10 | CI: the `compat.yml` Trino job gated like `spark-integration`, the unset-`SMELT_TRINO_URL` skip proved to be a skip, `changes` filter for Trino paths | pending |
| 11 | Close: `docs-site/` Trino target page, `hardening-baseline` entry for the new crate, `verify-phase.sh` green, divergences updated, and the measured `✗` consequences (no `PIVOT`, no temp tables, no transactional DDL) handed forward to the sibling outcomes that own them | pending |

## Decision log

- **2026-09-14 — phase 8: the capability profile, measured against a live coordinator.**
  27 probes ran in `crates/smelt-backend-trino/tests/capability_probes.rs`, zero skipped
  (`SMELT_TRINO_URL` set). The prior — Trino sits near Spark (Delta) — held on 19 of 26 flags
  and broke on 7. Measured errors for every `✗`, quoted verbatim:
  - `supports_qualify`: `mismatched input 'QUALIFY'. Expecting: ',', 'CROSS', 'EXCEPT', ...`
  - `supports_merge_not_matched_by_source` (spec-only): `mismatched input 'BY'. Expecting: 'AND', 'THEN'`
  - `supports_transactional_ddl`: `Client does not support transactions` — a property of
    smelt's stateless `/v1/statement` HTTP client (no session-token continuity across
    `START TRANSACTION` / DDL / `ROLLBACK`), not a per-statement Trino grammar rejection like
    the rest of this list.
  - `supports_double_colon_cast`: `mismatched input '::'. Expecting: '%', '*', '+', ',', '-', ...`
  - `supports_trailing_commas`: `mismatched input '<EOF>'. Expecting: '*', <expression>`
  - `supports_insert_overwrite`: `mismatched input 'OVERWRITE'. Expecting: 'INTO'`
  - `supports_native_ivm`: `createMaterializedView is not supported for Iceberg REST catalog`
  - `supports_alter_column_using`: `mismatched input 'USING'. Expecting: '(', 'ARRAY', <EOF>`
  - `supports_merge_schema_write`: `Insert query has mismatched column types: Table:
    [integer], Query: [integer, varchar(1)]`
  - `supports_pipe_syntax`: `mismatched input 'FROM'. Expecting: 'ALTER', 'ANALYZE', 'CALL', ...`
    (measured against the pinned `trinodb/trino:483` coordinator)
  - `supports_pipe_set_drop_rename`: `mismatched input 'EXCLUDE'. Expecting: ',', 'EXCEPT', ...`
  - `null_safe_equality` spelling: `SELECT 1 <=> NULL` → `mismatched input '>'. Expecting:
    'ALL', 'ANY', 'SOME', <expression>`; `IS NOT DISTINCT FROM` executes — Trino's spelling
    matches DuckDB/BigQuery, not Spark.

  The prior broke (Trino more capable than Spark(Delta)) on three: `supports_create_or_replace_table`,
  `supports_date_literal`, `supports_array_literal` (`[1,2,3]` bracket syntax) all execute
  cleanly on Trino where Spark(Delta) refuses them. `supports_array_literal` was probed via
  `cardinality([1,2,3])` rather than selecting the array value — decoding a Trino `array(...)`
  result to Arrow is a separate, unrelated gap this flag does not gate.

  `supports_struct_field_ddl`, `supports_nested_array_ddl` and `supports_column_mapping` all
  measured `true`: `ALTER TABLE ... ADD COLUMN s.b INTEGER` / `items.element.b INTEGER` both
  succeed, and `ALTER TABLE ... RENAME COLUMN` followed by a `SELECT` on the new name reads the
  prior data back — Iceberg's field-ID column tracking survives the rename with no table rewrite.

  Both `SqlDialect` language properties phase 2 landed conservatively `false` measured `true`:
  `FILTER (WHERE ...)` and `RANGE BETWEEN INTERVAL ... PRECEDING` both execute cleanly, matching
  the doc-comment hints already left in `dialect.rs` ahead of this phase.

  `supports_retraction` and `supports_fingerprint_sidecar` were not independently probed:
  the former is meaningful only alongside `supports_native_ivm` (measured false, so retraction
  is trivially false), the latter is an implementation-scope fact about smelt's own code (no
  Trino/Iceberg fingerprint-sidecar implementation exists), not an engine capability a live
  probe can measure — both are named in `every_capability_field_has_a_probe`'s
  `not_independently_probed` list so the coverage gate stays honest about the carve-out.

  For phase 9: `dialect_and_capabilities`'s refusal will now see `supports_qualify`,
  `supports_double_colon_cast`, `supports_trailing_commas`, `supports_alter_column_using`,
  `supports_pipe_syntax`, `supports_pipe_set_drop_rename`, `supports_transactional_ddl`,
  `supports_insert_overwrite`, `supports_native_ivm` and `supports_merge_schema_write` as
  `false` (refuse-worthy) and everything else in the matrix as `true`.

- **2026-09-13 — the spec's Trino capability column enters as `?`, not as a documentation-read
  guess.** Criterion 1 wants the column in the matrix now; criterion 5 (and the rule under the
  table) forbids a cell not established by execution, and `capability_conformance.rs` is a
  hand-written spec↔constructor gate that would silently drift for eight phases if the column
  carried values before `trino_iceberg()` existed. So phase 1 writes the column with every cell
  `?` plus a Known Divergence, and phase 8 replaces `?` with measured values in the same commit
  as the constructor. A test asserts the cells stay `?` until then, so the placeholder cannot rot
  into an unmeasured claim.

- **2026-09-13 — reshape: the target-config surface gets its own row (new phase 3).** Criterion 1
  has two halves: the *spec* of the `trino` target shape (phase 1) and the *code* that parses and
  refuses it. No row owned the second half — phase 2 is the dialect enum, phases 4-6 are the tier
  and the client — so `BackendType::Trino`, the `host`/`port`/`user`/`catalog`/`schema`/TLS keys,
  the `${ENV}`-only password rule and the foreign-key refusal fixture would have arrived as
  unplanned drift inside whichever phase first needed to load a `trino` target. Split out rather
  than deferred: criterion 1 is a success criterion. Former phases 3-9 renumbered 4-10; no
  summaries existed, so nothing is orphaned.
- **2026-09-13 — refusal channel: a hard configuration error, not a `DiagnosticCode`.** Criterion 1
  asks for the refusal to be "a named `DiagnosticCode`". Verified against the code: `smelt.yml`
  target-shape violations do not flow through `DiagnosticCode` at all — `diagnostics.md` has no
  code for them, and the `databricks` precedent (`Config::validate_targets`,
  `check_literal_secrets`) raises a `ConfigError::LoadError` naming every offending key and its
  backend. Trino follows that precedent rather than inventing a second channel; the criterion's
  substance (refused, named, never silently ignored, fixture-proved) is met. Reopening this would
  mean specifying config-load diagnostics for all five backends, which is out of scope here.

- **2026-09-13 — reshape: phase 8 also measures the two `SqlDialect` language properties.**
  `supports_aggregate_filter_clause` and `supports_interval_range_frame` are dialect-language
  facts declared on `SqlDialect`, not `BackendCapabilities` flags, so they sat outside every
  row's intent while still being claims about Trino's SQL surface — exactly what criterion 5
  forbids going unmeasured. Phase 2 lands both `false` (conservative: a `false` makes smelt
  refuse the construct rather than emit SQL Trino may reject) and phase 8's row now owns
  turning them into measured values. Not deferred out; no new row needed.

- **2026-09-13 — phase 2 does not add a `MaintenanceDialect::Trino` variant; it makes
  `maintenance_dialect` fallible instead.** Surveyed the blast radius: adding the variant breaks
  ~150 match arms across ten emitters in `smelt-logical/src/maintenance/emit/`, each needing a
  Trino SQL spelling that `20260913-trino-incremental` owns. The fail-loud alternative already
  has a precedent in this codebase — `smelt_state::UnsupportedLedgerDialect` — so
  `maintenance_dialect` returns `Result` and its ~20 callers (all in `Result` contexts) refuse
  Trino by name. T4 narrows the error away by adding the variant; it never silently aliases
  Trino onto Spark's spellings, which share Iceberg's atomicity shape but not its SQL.

- **2026-09-13 — the compiler does not find every arm; `type_cast_sql` proves it.**
  `smelt-dialect/src/type_conformance.rs` ends its `(dt, dialect)` match with `_ =>
  dt.to_backend_sql()`, so `SqlDialect::Trino` would have compiled clean while silently emitting
  `FLOAT` and `BLOB` in the output cast wrap — neither of which is a Trino type (`REAL`,
  `VARBINARY`). Criterion 2's "no wildcard match arm anywhere absorbing them" is therefore an
  explicit audit task in phase 2, not a by-product of `cargo check`. The one wildcard left
  standing is `Signature::engine_native`'s implicit `Native`, already recorded as a Known
  Divergence owned by `20260913-trino-emission`.

- **2026-09-14 — `dialect_audit/main.rs` gets a test-local `AUDITED_DIALECTS` const excluding
  Trino, distinct from `DialectId::ALL`.** Phase 2's wildcard audit correctly named Trino arms
  in `fixture.rs`/`probe.rs` as `unreachable!()` (Trino has no fixture/probe/baseline entry —
  that's `20260913-trino-emission`'s job), but `dialect_audit/main.rs` has 4 tests that loop
  directly over `DialectId::ALL`, which now includes Trino and hits those `unreachable!()` arms.
  Building Trino audit coverage now would be out of scope; leaving `DialectId::ALL`
  non-exhaustive would violate criterion 2. Added a 3-member `AUDITED_DIALECTS` const scoped to
  this test file only; `DialectId::ALL` itself is untouched. See phase 2 summary.

- **2026-09-14 — phase 3 makes `dialect_and_capabilities` fallible rather than landing a
  placeholder `BackendCapabilities::trino_iceberg()`.** `BackendType::Trino` forces
  `smelt-runtime/src/compile.rs`'s `(dialect, capabilities)` match to answer for Trino, but the
  2026-09-13 ruling above reserves the capability profile for phase 8, where the constructor and
  the spec table are written in one commit. A placeholder constructor would be exactly the
  unmeasured claim `capability_conformance.rs` and the `?`-cells gate exist to prevent, and
  aliasing Trino onto `BackendCapabilities::spark()` would bake in the Delta-shaped prior the
  outcome insists is a prior and not an answer. So the function returns `Result` and refuses
  Trino by name — the same fail-loud shape phase 2 gave `maintenance_dialect` and
  `ddl_backend_for_dialect`. Phase 8 narrows the error away.

- **2026-09-14 — `SqlCompiler::new`/`CompilerRegistry::new` become fallible
  to thread `dialect_and_capabilities`'s Trino refusal.** Both constructors
  were infallible before this phase. Making the dialect/capabilities lookup
  fail for Trino (per the 2026-09-14 ruling above) forced `SqlCompiler::new`
  and `CompilerRegistry::new` to return `anyhow::Result<Self>`; every
  production call site (`execute_project`, `smelt-cli`'s `check`/`explain`,
  `smelt-ui`'s `build.rs`, `smelt-runtime`'s `profile.rs`) now propagates
  with `?`, and `profile.rs` gained a new `ProfileWorkspaceError::
  CompilerRegistryFailed` arm. ~35 test call sites across the workspace
  needed a mechanical `.unwrap()` added; none changed behavior since every
  one constructs a non-Trino target.
- **2026-09-14 — `smelt-maintenance-testkit::print_body_for_dialect` refuses
  Trino with `unimplemented!` rather than a placeholder capability profile.**
  This testkit crate is dev-dependency-only everywhere (no crate depends on
  it normally, it produces no binary), so it is exempt from the
  `unwrap`/`expect` hardening ratchet; no S-restricted-oracle recipe
  exercises Trino today, so a loud panic is preferable to fabricating an
  unmeasured `BackendCapabilities`.
- **2026-09-14 — `check_literal_secrets` and the databricks
  `token`/foreign-key checks were generalised to a `(backend, key)` table
  (`LITERAL_SECRET_KEYS`) rather than duplicated for `trino`.** `trino`'s
  `password` follows exactly the same pre-interpolation literal-value rule
  as `databricks`' `token`; a second hand-copied function would have let the
  two drift silently.
- **2026-09-14 — `.claude/large-file-baseline.txt` updated for `config.rs`,
  `compile.rs`, `graph.rs`, `s_tracker.rs`, `execute/project/mod.rs`, and
  `smelt-cli/tests/resume.rs`.** Each grew from real Trino-shaped content
  (new `Target` fields threaded through every literal, new tests, the
  `Result`-returning constructor plumbing) rather than incidental bloat; no
  file crossed a cohesion boundary that would justify a split as part of this
  phase.

- **2026-09-14 — phase 4's tier uses named volumes, not host bind mounts, for writable state.**
  Criterion 3 names the exact failure `scripts/spark-up.sh` hit: a `chmod` on a root-owned
  leftover aborting under `set -e` *before* `docker run`, after which every test failed with no
  hint the server never started. That hazard exists only because Spark's warehouse and Ivy cache
  are host-owned bind mounts. A compose tier has a structural way out — Docker-managed named
  volumes removed by `down -v`, with only the read-only `scripts/trino-catalog/` bind-mounted —
  so phase 4 takes it rather than re-implementing `ensure_container_writable`. No reshape of the
  remaining rows: the phase 3 summary surfaced nothing out of scope, and phases 4 and 5 are
  independently unblocked.

- **2026-09-14 — phase 4's pins: `trinodb/trino:483` (equal to `:latest` at pin time, pinned by
  digit so a future `:latest` move can't silently change CI behavior), `apache/iceberg-rest-fixture:1.10.1`
  (the Apache project's own REST-catalog fixture, JDBC/SQLite-backed so `down -v` always yields a
  clean catalog), `quay.io/minio/minio:RELEASE.2025-09-07T16-13-09Z` and
  `quay.io/minio/mc:RELEASE.2025-08-13T08-35-41Z`.** MinIO no longer publishes to Docker Hub, so
  both MinIO images come from `quay.io/minio/*`; picked the newest plain `RELEASE.*` tag with no
  `-cpuv1`/`hotfix` suffix. Only the Trino coordinator publishes a host port (`18080` by default,
  `SMELT_TRINO_PORT` to override); MinIO and the Iceberg REST catalog are reachable only inside
  the compose network, so there is nothing else to collide with `spark-up.sh`'s `15002`/
  `smelt-spark`. Live legs run: `trino-up.sh` reached ready, a `POST /v1/statement` round-trip
  (`CREATE SCHEMA` → `CREATE TABLE` → `INSERT` → `SELECT`) returned the inserted row
  `[1, "hello"]` confirming the write went through to MinIO and back, and a second `trino-up.sh`
  run with no intervening `down` reached ready again — the idempotency requirement, satisfied
  structurally by named volumes rather than by leftover-detection logic.

- **2026-09-14 — phase 5's client is `reqwest` over pure Rust, and its tests run against a
  local `axum` stub coordinator rather than the Docker tier.** `reqwest` 0.12 is already in
  `Cargo.lock` (pulled by `libduckdb-sys`), so a direct `default-features = false` +
  `rustls-tls` dependency adds no new compilation graph — cheaper than hand-rolling HTTP over
  `hyper`, and it keeps criterion 4's "no Python interpreter, no venv, no third-party client"
  promise. The tests bind an `axum` router (already a workspace dependency via `smelt-ui`) on an
  ephemeral port so paging, error mapping and header assertions are provable with
  `SMELT_TRINO_URL` unset — the live tier is phase 6's oracle, not phase 5's.

- **2026-09-14 — reshape: criterion 7's `execute_project` leg gets its own row (new phase 9).**
  Phase 6's row said "a model materialized as an Iceberg table and read back", which reads as
  either a `Backend`-trait-level materialization or a full `execute_project` run. Only the first
  is reachable now: `dialect_and_capabilities` refuses Trino by name until phase 8 lands
  `BackendCapabilities::trino_iceberg()` (ruling of 2026-09-14), so `SqlCompiler::new` — and
  therefore `execute_project` — cannot construct a Trino compiler at all. Criterion 7 also asks
  for a `smelt-cli` target-parity leg, which no row owned. Rather than let that arrive as
  unplanned drift inside the CI or close phase, phase 6 is narrowed to the trait level (through
  `execute_model`, which is where a table and a view are actually created) and a new phase 9
  owns the end-to-end leg, placed after phase 8 because it depends on it. Former phases 9–10
  renumbered 10–11; no summaries exist for them, so nothing is orphaned. Nothing left the
  outcome.

- **2026-09-14 — phase 6's `Backend::capabilities()` returns a provisional all-`false` profile,
  pinned by a test.** `capabilities()` is a required trait method, so phase 6 must answer it, but
  phase 8 owns the measured profile. Every flag `false` is the fail-safe direction — audited the
  consumers (`maintenance_driver/driver.rs`, `schema_evolution.rs`,
  `maintenance_driver/sidecar.rs`): a `false` makes smelt refuse the construct or take the
  non-transactional path, never emit SQL Trino might reject. A test asserts *every* flag is
  `false`, so the placeholder cannot rot into an unmeasured positive claim, and phase 8 deletes
  both the constructor and the test in the same commit as the measured profile. Same shape as
  phase 1's `?`-cells gate.

- **2026-09-14 — `smelt-backend-trino` is a non-optional dependency of `smelt-backends`, unlike
  `spark`/`bigquery`/`databricks`.** Those are feature-gated because their dependency graphs are
  heavy and environment-bound (PyO3 + a Python interpreter, the Google SDK). Trino's is
  `reqwest` with `rustls-tls`, already resolved in `Cargo.lock` via `libduckdb-sys`, so gating it
  would buy no build time while hiding the `Backend` impl from the default `cargo test` and from
  `clippy-gate.sh`'s two feature sets — exactly the "an audit leg that skips is
  indistinguishable from one that passes" hole this outcome's driver note warns about. The live
  legs self-gate on `SMELT_TRINO_URL` instead; compilation is never gated.

- **2026-09-14 — phase 6 refuses `delete_partitions`/`insert_into_from_query`/`insert_overwrite`
  by name rather than porting DuckDB's DELETE+INSERT emulation.** These three `Backend` trait
  methods are required (no provided default), but the plan's task list never named them — an
  omission this phase's `cargo check` surfaced. All three belong to the incremental/maintenance
  family this outcome's Out of scope section reserves for `20260913-trino-incremental`; DuckDB's
  own emulation for them assumes transactional semantics and a partition-literal axis rendering
  neither measured against Iceberg, so copying it would be a guess T4 would have to re-decide
  anyway. Each refuses with `BackendError::UnsupportedFeature` naming the method and the outcome
  that owns it — the same shape as `load_table`'s phase-7 refusal.
- **2026-09-14 — live legs measured a real divergence: `VALUES (1, 'x')` types its integer
  column as Trino `integer` (Arrow `Int32`), not `bigint`.** Only `count(*)` is reliably
  `bigint`. `create_view_as_then_read_back_and_drop` initially asserted `Int64Array` and failed
  against the live tier; fixed to `Int32Array` with a comment recording the distinction, since a
  wrong assumption here would have silently under-tested `arrow_convert`'s type mapping.

- **2026-09-14 — phase 7 measures the `INSERT`-over-HTTP path; Parquet staging into MinIO is not
  reachable from the target shape this outcome specified.** Criterion 6 wants the bulk path
  chosen by measurement. Only one candidate is actually available to measure: the `trino` target
  carries `host`/`port`/`user`/`catalog`/`schema`/TLS/`password` and **no object-store
  credentials**, so smelt cannot write a Parquet file into MinIO for the Iceberg connector to
  read without new target keys — a `smelt_yml.md` §"Target shape" change wider than this outcome.
  Phase 7 therefore measures the `INSERT INTO … SELECT CAST(…) FROM (VALUES …)` path and records
  the numbers. If measurement shows that path inadequate at seed scale, that is an escalation
  (a spec change to the target shape) to be recorded and raised, **not** absorbed into the phase.

- **2026-09-14 — the `seed_parity` Trino leg is a separate test, not a `targets_to_run()` member.**
  `targets_to_run()` is consumed by every W1+ suite in `smelt-cli`, most of which run
  `smelt build`; `dialect_and_capabilities` refuses Trino by name until phase 8, so adding Trino
  to that helper would turn currently-green suites red for a reason unrelated to seeding.
  `smelt seed` itself never constructs a `SqlCompiler` (verified: `commands/seed.rs` reaches only
  `create_backend`), so the CLI seed leg is genuinely reachable now and is not deferred.

- **2026-09-14 — phase 7 measured a genuine Trino protocol divergence: without
  `X-Trino-Client-Capabilities: PARAMETRIC_DATETIME`, a written `timestamp(6)` value silently
  truncates to millisecond precision on read-back.** The round-trip test
  (`load_table_round_trips_the_whole_seed_type_set`) initially failed with `1705318861123000 !=
  1705318861123456` even though the write path (a `CAST(... AS timestamp(6))`) was correct.
  Probed directly against the live coordinator: `to_unixtime(t)` on the stored value returned
  the full `1705318861.123456`, proving the *storage* is not the problem — only the REST
  protocol's default value formatting is, a legacy-client-compatibility behavior Trino's own
  client-capabilities negotiation exists to opt out of. Fixed by adding the capability header to
  every request in `TrinoClient::send`, not by loosening the test's precision assertion — the
  header is a client-protocol correctness fix affecting every read through this client, not a
  `load_table`-specific workaround. Recorded in `multi_backend.md` §"Loading data into a
  backend" so any future reader of the read path (`execute_sql`, `get_preview`, …) knows why the
  header exists.
- **2026-09-14 — the bulk path is a chunked `INSERT INTO … SELECT CAST(…) FROM (VALUES …)`,
  measured at ~6,000 rows/second, 1,000 rows per statement.** No Parquet-staging alternative was
  reachable to measure against: the `trino` target shape phase 3 landed has no object-store
  credential, so smelt cannot write a file into the Iceberg connector's backing MinIO bucket
  without a `smelt_yml.md` target-shape change wider than this outcome (recorded as an
  escalation, not absorbed here). A live 12,000-row load
  (`load_table_loads_a_multi_chunk_batch`) completed in ~1.8–2.1s across 12 chunked `INSERT`
  statements — adequate at seed scale; a future need for faster bulk loads is the trigger to
  revisit Parquet staging, not a default assumed now.
- **2026-09-14 — `build_load_plan` is a pure, table-driving function separate from
  `TrinoBackend::load_table`.** Nullability validation, DDL-type mapping, and chunked-`INSERT`
  construction are all testable without a live server or an HTTP stub — `load_table` itself
  becomes a thin wrapper that executes the plan's statements over `TrinoClient`. This let phase
  7's unit tests (nullability rejection "before any statement is built", exact chunk-boundary
  row counts) assert directly against the plan's structure rather than needing a stub coordinator
  to capture request bodies.
- **2026-09-14 — `smelt-backend-trino` is a dev-dependency of `smelt-cli`, unconditional (no
  feature flag), matching the outcome's 2026-09-14 ruling that the crate itself is non-optional.**
  The `seed_loads_into_trino` CLI-parity test constructs `TrinoBackend` directly for read-back
  (`common::trino_backend`/`fetch_trino_rows`), the same way the Spark and BigQuery legs
  construct their own backend types directly rather than going through `smelt-backends`' factory.

- **2026-09-14 (phase 8 planning) — the two spec-only matrix rows are measured but not
  conformance-gated.** `supports_merge_not_matched_by_source` and `supports_staged_relation_group`
  appear in the §Surface matrix but name no field on `BackendCapabilities` for *any* backend, so
  `capability_conformance.rs` cannot assert them. Phase 8 still probes both against the live
  coordinator and writes the measured Trino cells — a `?` left behind would rot exactly the way
  the 2026-09-13 entry above guards against — and marks the two rows in the table as spec-only.
  Closing that pre-existing spec↔struct drift for all six columns is not this outcome's work.
  No phase-table reshape: phase 7's summary surfaced nothing that changes the remaining rows.

## Blocked
