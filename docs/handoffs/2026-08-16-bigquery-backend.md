# Handoff — BigQuery as a first-class backend

**Date:** 2026-08-16
**Updated:** 2026-08-17 — type oracle and divergence registry landed
**Updated:** 2026-08-18 — generative maintenance-conformance leg (`maintenance_conformance_bigquery`),
`scripts/bigquery-conformance.sh`, and the row-set/MEDIAN dialect gaps it found are landed. The
live sweep was run: 10 passed / 11 failed in 886.60s, on two distinct GoogleSQL gaps. See
"Generative maintenance-conformance" below.
**Updated:** 2026-08-18 (later same day) — the oracle-relation seam (`ConformanceBackend::oracle_relation`)
closed both of those gaps. Live re-run: 13 passed / 8 failed / 0 ignored, 1142.10s. Eight cases
still fail, for four distinct causes — one product-side, three harness-side/uncharacterised. See
"Generative maintenance-conformance" below, which now supersedes the paragraph above it.
**Worktree:** `/home/andrew/smelt-sql/.claude/worktrees/bigquery`
**Branch:** `bigquery-backend-research` (clean descendant of `main`; no PR yet)

## Where we are

BigQuery has a leg in every fixed-recipe parity suite, and **all eight pass against the live
warehouse**: `dual_target_harness`, `source_seed`, `seed_parity`, `materialization_parity`,
`lowering_parity`, `merge_parity`, `incremental_parity`, `schema_evolution_parity`.

BigQuery also has a leg on the generative maintenance-conformance gate now
(`crates/smelt-cli/tests/maintenance_conformance_bigquery/`, run via
`scripts/bigquery-conformance.sh`) — see "Generative maintenance-conformance" below for what
passes live today and what still doesn't.

The type oracle now has a BigQuery leg too, backed by dry-run schema queries against the live
warehouse, with a divergence registry in place. A 512-case sweep against 285 live-compared
columns found exactly one unregistered divergence class — the surface is far smaller than
DuckDB's or Spark's. See "The type oracle and BigQuery divergences" below.

```
bash scripts/bigquery-auth.sh       # per session: prompts for the passphrase, 1h token
bash scripts/bigquery-parity.sh     # the whole sweep
```

**Read first:** `docs/research/20260816-bigquery-backend.md` — decisions, rejected
alternatives, phase order, the provisioned environment, and the §"Measured against the live
warehouse" findings. This handoff does not restate it.

## The decisions that constrain everything downstream

1. **Client layer is a PyO3 → Python adapter** (`python/smelt/bigquery_adapter.py`), mirroring
   `spark_adapter.py`, using `google-cloud-bigquery` and Arrow.
2. **Verification is local-gated with no CI tier.** BigQuery tests skip green when
   `SMELT_BQ_PROJECT` is unset. Recorded as a Known Divergence.
3. **Credentials are non-ambient — do not "simplify" this.** No application-default
   credentials at any point. The adapter authenticates from `SMELT_BQ_ACCESS_TOKEN` explicitly
   and refuses to construct without one; `bigquery_smoke.rs` asserts that refusal, because it
   is a security property rather than an implementation detail.
4. **Capability values come from the warehouse, never from documentation.** Every cell in the
   matrix was established by running the statement the flag names (`scripts/bigquery-probe*.sh`).
   The MERGE arm shapes below were established the same way *after* a parity test went red —
   the probe is how a failure gets converted into a fact, not a formality before the work.

## What is implemented

`BackendType::BigQuery`, `Target` `project`/`dataset`/`location`, `SqlDialect::BigQuery`,
`BackendCapabilities::bigquery()` (asserted by `capability_conformance`),
`crates/smelt-backend-bigquery`, the `create_backend` arm behind a `bigquery` feature, and
`MaintenanceDialect::BigQuery`.

Dialect work landed where the warehouse actually refused something:

- **Type names.** GoogleSQL rejects `VARCHAR`, `TEXT`, `DOUBLE`, `REAL`, `FLOAT`. The
  output-boundary cast wrap (`type_conformance.rs`) and the maintenance bootstrap DDL map them
  to `STRING`/`FLOAT64`/`BYTES`; everything else passes through.
- **`SHA256` returns `BYTES`**, so fingerprint emitters need a `TO_HEX` wrap no other dialect
  requires.
- **No `INSERT OVERWRITE`** — `insert_overwrite` lowers to the scoped `DELETE` + `INSERT`.
- **No whole-row `MERGE` star forms** — see below; this was the one thing the parity legs broke.
- **Schema-evolution DDL is unimplemented** — resolves to a full refresh naming the reason
  rather than emitting DDL BigQuery would reject. (The warehouse's own `ADD COLUMN` works and is
  covered by `schema_evolution_parity`; that is a separate question from smelt emitting the DDL.)

## The parity harness

`TargetKind::BigQuery { dataset }` in `crates/smelt-cli/tests/common/mod.rs`. A suite enumerates
its targets through `targets_to_run(label)`, so a backend joins every suite in one edit and the
compiler names each suite that has not yet handled it.

- **The dataset is derived, not minted.** `bq_dataset(label)` = `<base>_<label>_<pid>`, so
  staging (which writes the `bq:` block via `bq_target_block`) and the assertion loop compute the
  same name independently — no state threaded through suites that hand-write their `smelt.yml`.
  Pid separates concurrent runs; label separates suites, because **the per-table modification
  quota binds** and suites must not share target tables.
- Each suite drops its dataset on the way out; `bigquery-env.sh` exports a default table
  expiration as the backstop for an interrupted run.
- `bigquery-parity.sh` passes `--no-fail-fast` deliberately: the sweep is slow and rate-limited,
  so one red leg must not hide the state of the others.

## The whole-row MERGE lowering

The emitters had always produced `WHEN MATCHED THEN UPDATE SET *` / `WHEN NOT MATCHED THEN
INSERT *`, in a function whose own doc comment called itself dialect-invariant. GoogleSQL accepts
neither. Probed (`scripts/bigquery-probe-merge.sh`): `SET *` → `Expected "(" but got "*"`;
`INSERT *` → `Expected keyword ROW or keyword VALUES but got "*"`; `SET c = source.c` and
`INSERT ROW` are taken.

- The two **column-scoped** emitters take a `columns` list and spell the matched arm out under
  BigQuery. The two **keyed folds** already emit an explicit `SET` of fold expressions, so only
  their not-matched arm varies — no column list needed there.
- The list is the model's output projection, carried on `CompiledModel::output_columns` and
  derived from the compiled SQL's select list via the same `SelectItem::column_name` the
  analyzer's `model_schema` reads, so the build path and the editor agree on a model's columns.
- **It is inert wherever a star form exists**, and that is asserted rather than assumed — roughly
  twenty existing call sites now pass `&[]`, so a test pins that DuckDB's and Spark's emitted
  text stays byte-identical whatever is passed. Do not weaken that test.
- **Empty means unknown, never "no columns."** A surviving wildcard leaves the list empty, and an
  empty `SET` list is a syntactically valid `MERGE` that silently stops updating matched rows.
  `smelt_backend::require_merge_columns` refuses instead, and both paths that emit a whole-row
  MERGE (the `Backend` default and the maintenance driver) route through that one guard.
- **New limitation, now in the spec:** a model whose output projection is not statically
  resolvable cannot use `Technique::ColumnScopedMerge` on BigQuery. DuckDB and Spark are
  unaffected.

## The type oracle and BigQuery divergences

The oracle asks BigQuery for a query's output schema via a dry-run job — free, reads no table,
still rejects invalid SQL — over a persistent Python subprocess, gated on
`SMELT_BQ_ACCESS_TOKEN`; absent, the leg is not present and the suite is green. Full findings
and the registered-divergence detail are in `docs/research/20260816-bigquery-backend.md`
§"Measured against the live warehouse"; the summary:

- A 512-case proptest sweep compared 285 columns against the live warehouse (the default
  256-case sweep compares ~139) and produced exactly one unregistered divergence class, at 18
  occurrences: smelt infers `Integer` where BigQuery reports `BigInt`.
- Three divergences are registered in `divergences.rs`, all `BackendSpecific`:
  `bigquery_single_integer_width` (BigQuery has one integer type, so width is unobservable on
  this leg), `bigquery_decimal_width_unreported` (BigQuery's dry-run schema omits
  precision/scale for `NUMERIC`/`BIGNUMERIC` entirely, surfaced as the sentinel `Decimal{0,0}`),
  and `bigquery_timestamp_keyword_is_zone_aware` — **the one genuine smelt gap found**: BigQuery
  spells its zone-aware type `TIMESTAMP` and its naive type `DATETIME`, the inverse of
  SQL-standard/DuckDB/PostgreSQL, and type inference has no notion of target dialect so `CAST(x
  AS TIMESTAMP)` reads the standard meaning regardless of target. Registered rather than fixed —
  fixing it means threading a target dialect into type inference's inputs.
- The registry's "matches any Decimal" wildcard is now a distinct `ANY_DECIMAL` constant, not
  `Decimal{0,0}` — that value is BigQuery's unreported-width sentinel and the two meanings
  collided before the split.
- `check_types_against_oracle` previously skipped on any oracle error; it now only skips on an
  allow-listed query-refusal shape, and fails on everything else, including an unrecognised 4xx.
  This matters because an invalid/expired BigQuery token does **not** surface as 401/403 — it's a
  client-side google-auth message with no HTTP status at all. A coverage floor
  (`BIGQUERY_COLUMN_COVERAGE_FLOOR = 50`) additionally fails the leg if too few columns were
  compared. Both guards were verified live: a deliberately invalid token fails in 8 seconds
  rather than passing green.
- Token window math: a full default sweep takes ~85s, a 512-case sweep ~162s — both fit inside
  the one-hour token. Proptest shrinking dominates wall-clock on a failing run (150s+ on a
  512-case failure), so surveying a new backend's type surface in one wide pass is cheaper than
  iterating fail-fast.

## Two findings that change downstream sizing

- **The per-table modification quota binds.** Eight rapid `CREATE OR REPLACE TABLE` statements
  against *one* table name are refused (`Your table exceeded quota for table update
  operations`); the same rate across distinct tables is not. A generative conformance suite
  must allocate a fresh target table per case. An earlier reading that the limit did not bind
  came from a loop whose own round-trip latency kept it under the burst threshold — do not
  trust a rate measurement whose spacing is set by its own latency.
- **BigQuery supports native materialized views.** `supports_native_ivm` is nonetheless `false`,
  because `true` obliges smelt to emit a native maintained object and that path does not exist.
  It is the only flag whose value describes smelt rather than the engine.

## Working constraints that cost time to rediscover

- `.claude/settings.json` **denies** `gcloud`, `bq`, and the credential scripts, so an agent
  cannot reach GCP directly. It *can* run `scripts/bigquery-probe*.sh`, `bigquery-verify.sh`,
  `bigquery-test.sh` and `bigquery-parity.sh`, which read the minted token off disk.
- Interactive flows — gcloud OAuth, the gpg passphrase — need a **real terminal**.
- **`bq` is unusable on this host** (bundled-Python pyOpenSSL mismatch:
  `module 'lib' has no attribute 'GEN_EMAIL'`). `gcloud` is fine. Everything talks to BigQuery
  over the REST API with `curl` + a token; do not reintroduce `bq`.
- **`bigquery-auth.sh` activates the service account**, which sets it as the gcloud config's
  active account. It now restores the previous account on exit — if that regresses, every
  human-identity operation silently runs as the service account and fails with
  `PERMISSION_DENIED`.
- The worktree guard rejects compound shell commands (pipes, `source`, `$(…)` plus redirects).
  Put anything non-trivial in a script and run the script.

## Session workflow

```
bash scripts/bigquery-venv.sh        # once: uv-based client venv (system python has no pip)
bash scripts/bigquery-auth.sh        # per session: prompts for the passphrase, 1h token
bash scripts/bigquery-parity.sh      # the fixed-recipe parity sweep (8 suites)
bash scripts/bigquery-conformance.sh # the generative maintenance-conformance leg (21 cases)
bash scripts/bigquery-test.sh        # defaults to the smoke suite; takes cargo args
```

`bigquery-conformance.sh` is deliberately not invoked by `bigquery-parity.sh` — the parity sweep
is a bounded routine check, while the conformance leg is slow (every statement is a network round
trip) and may want the whole one-hour token window to itself. It fails loud, before starting any
sweep, if `SMELT_BQ_PROJECT` is unset or no valid token is on disk — see
`docs/specs/multi_backend.md` §Known Divergences for why that matters (a sweep with a project set
but no token would otherwise die mid-case having burned real quota).

## Generative maintenance-conformance

The `maintenance_conformance_bigquery` binary drives the same recipe pools, schedule driver, and
S-restricted multiset oracle the DuckDB and Spark legs use
(`docs/plans/20260817-bigquery-generative-conformance.md`), against a live BigQuery warehouse.
Three dialect gaps the first live run found are closed: the shared families no longer hardcode a
Spark dialect assumption (`EXCEPT ALL`, `USING DELTA`, a fixed Spark schema name), inline row sets
compile through one dialect-aware owner that emits a portable `UNION ALL` chain instead of a
`FROM (VALUES …)` constructor GoogleSQL rejects, and `MEDIAN` lowers to an exact GoogleSQL form
rather than tripping `Function not found`.

**First measured live, 2026-08-18** (`bash scripts/bigquery-conformance.sh`, `--test-threads=1`):
10 passed, 11 failed, 0 ignored, in 886.60s. Ten of the eleven failures were the S-restricted
oracle relation's `CREATE OR REPLACE TEMPORARY VIEW` (BigQuery refuses it: `400 CREATE TEMP VIEW
is unsupported`); the eleventh was the composed family's hand-rolled `(VALUES …) AS t(id, d,
val)` row set bypassing the dialect-aware owner.

**Both gaps closed** by a `ConformanceBackend::oracle_relation` seam: DuckDB/Spark's default
still materializes the temp view byte-for-byte; BigQuery's override issues no DDL and returns an
inline derived table instead. The composed family's row set now routes through
`smelt_core::build_row_set_table` like every other caller.

**Re-measured live, 2026-08-18 (later same day)** (`bash scripts/bigquery-conformance.sh`,
`--test-threads=1`): **13 passed, 8 failed, 0 ignored**, in **1142.10s** (~19 minutes)
wall-clock — about a third of the one-hour credential window, with headroom to spare.
`harness_self_check_bigquery::oracle_flags_a_seeded_divergence_on_bigquery` now **passes** — the
first live proof that BigQuery's leg has a non-vacuous oracle. A targeted re-run of just the
`gate_bigquery` module independently measured 3 passed / 3 failed in 200.75s.

**The remaining eight failures, by cause:**

1. **Product-side — the keyed-fold `MERGE` ignores the target dialect.**
   `gate_keyed_bigquery::keyed_pool_upholds_end_state_equivalence_on_bigquery` fails with `400
   Syntax error: Expected keyword ROW or keyword VALUES but got "*"` on a compiled `MERGE … WHEN
   NOT MATCHED THEN INSERT *` statement. The emitter that spells `INSERT ROW` for BigQuery
   (`whole_row_insert_arm`, `crates/smelt-logical/src/maintenance/emit.rs:293`) is correct; its
   caller, `build_cumulative_merge_sql` (`crates/smelt-runtime/src/cumulative.rs:621`), takes no
   dialect parameter and hardcodes `MaintenanceDialect::DuckDb` at both call sites
   (`:644`, `:652`), so a keyed model's cumulative-aggregate `MERGE` always emits `INSERT *`
   regardless of backend. This affects real `refresh: keyed` models with a cumulative aggregate
   on BigQuery, not just the harness — highest-priority fix.
2. **Harness-side — the oracle bypasses smelt's `MEDIAN` lowering.**
   `gate_bigquery::append_only_partition_pool_upholds_equivalence_on_bigquery` fails with `400
   Function not found: MEDIAN`, raised from `STracker`'s raw-SQL oracle query rather than a
   compiled model — the oracle renders its own `HolisticAgg` body without going through smelt's
   printer, so the exact-`MEDIAN` GoogleSQL lowering never applies there.
3. **Product-side — a shared default `DROP VIEW`s an existing `TABLE`.**
   `gate_bigquery::column_add_between_runs_recovers_equivalence_on_bigquery` and
   `gate_bigquery::full_refresh_interleave_resets_state_correctly_on_bigquery` fail with `400
   Cannot drop <project>:<dataset>.recipe_additive_agg which has type TABLE. A view was expected.`
   The default `Backend::execute_model` (`crates/smelt-backend/src/lib.rs:216`, `:222`-`223`)
   unconditionally drops both kinds "in case the materialization type changed" — harmless on
   DuckDB/Spark, a hard error on BigQuery when the existing object is the other type.
4. **Harness-side — a staging collision in the `pinned` family.**
   `pinned_bigquery::hazard_schedules_are_pinned_on_bigquery` and
   `pinned_bigquery::pinned_recipes_reproduce_catalogue_coverage_on_bigquery` fail with `409
   Already Exists: Table …sources_events` — a case stages its source table twice into one
   per-case dataset, the same twin-target collision shape the `dags` family already had fixed.
5. **Uncharacterised.** `dags_bigquery::diamond_propagation_suffices_on_bigquery` and
   `gate_composed_bigquery::composed_keyed_pool_upholds_equivalence_on_bigquery` failed but their
   failure text scrolled off the tailed sweep log before it was captured. Needs a fresh live run
   to diagnose — no cause should be assumed.

All eight are recorded in `docs/specs/multi_backend.md` §Known Divergences. None is fixed yet.

## Next steps, in order

1. **Fix the product-side `INSERT ROW` dialect-threading gap** (cause 1 above) — thread the
   target dialect into `build_cumulative_merge_sql` instead of hardcoding
   `MaintenanceDialect::DuckDb`. Highest value: it's the only remaining gap that affects real
   user models, not just the harness.
2. **Diagnose the two uncharacterised failures** (`dags_bigquery::diamond_propagation_suffices`,
   `gate_composed_bigquery::composed_keyed_pool_upholds_equivalence`) with a fresh live run that
   captures full failure text.
3. Fix the `pinned` family's twin-target staging collision (cause 4), applying the same fix the
   `dags` family already got.
4. Fix or accept the harness-side gaps: route the S-restricted oracle's `MEDIAN` aggregate
   through smelt's printer (cause 2), and make the shared `execute_model` default tolerant of a
   type-mismatched `DROP VIEW`/`DROP TABLE` on BigQuery (cause 3) — the latter may want a
   BigQuery-specific override rather than a change to the shared default.
5. **`supports_pipe_syntax` has no live coverage.** BigQuery is the only backend reporting
   `true` and no parity fixture writes a pipe query, so the printer's emit-pipes-natively path
   is the one BigQuery-relevant path still unexercised. A fixture with a `|>` query closes it.
6. **Cross-engine pairs.** `cross_engine_parity` / `cross_engine_types_parity` assert handoff
   between two live engines rather than looping over `targets_to_run`, so extending them means a
   new engine *pair*, not a third leg. Sized separately from the work above.
7. **`/smelt:plan`** if the remaining work is taken as one programme rather than piecemeal.

## Loose ends

- **The budget alert is unprovisioned.** `gcloud billing budgets` requires ADC, which this
  design refuses. Manual console step: US$5/month on `smelt-bq-test-20260816`. A GCP budget
  alerts rather than hard-stops.
- **One pre-existing test failure, unrelated to this work:**
  `cargo test -p smelt-logical --test contract_lattice_spec` wants a
  `### The contract, plan, and graph layer` heading in `docs/specs/incremental_models.md` that
  is absent at `HEAD`. It predates the BigQuery branch; `verify-phase.sh` is otherwise green.
- **Three capability-matrix rows are not `BackendCapabilities` struct fields**
  (`supports_column_scoped_merge` aside, the `..._not_matched_by_source` and
  `..._staged_relation_group` rows). The spec says so deliberately — they are "specified ahead of
  their own struct fields" — but it means a reader grepping the codebase for a matrix row will
  come up empty. Not a defect; just a trap worth knowing before chasing it.
