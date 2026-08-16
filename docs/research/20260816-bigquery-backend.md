# BigQuery as a first-class backend

**Date:** 2026-08-16
**Question:** What does it take to make BigQuery a first-class smelt execution backend, at
Spark-equivalent gate coverage, and in what order should the work happen? The forcing
constraint is that BigQuery has no usable local engine, so every oracle-driven gate in the
repo — all of which assume a fast in-process or in-container engine — has to be re-tiered
around a network round-trip.

## Findings: the seams already exist

Adding a backend is genuinely additive. Four single-owner extension points carry the whole
surface, and each is already shaped for a third entry:

- `SqlDialect` (`crates/smelt-dialect/src/dialect.rs`) — a closed enum of three variants.
- `BackendCapabilities` — a flat struct of capability flags with a per-backend constructor.
  `capability_conformance.rs::all_fields_destructured` forces every field of a new
  constructor to be named explicitly, so no capability can be silently defaulted.
- `Backend` (`crates/smelt-backend/src/lib.rs`) — ~15 required async methods, with the
  maintenance-technique methods (`merge_into`, `insert_overwrite`, `delete_partitions`,
  `execute_statement_group`, …) supplied as defaults over them.
- `create_backend` (`crates/smelt-backends/src/lib.rs`) — the sole `target_type → Box<dyn
  Backend>` site, feature-gated per backend.

BigQuery is already named in the tree: `supports_pipe_syntax`'s doc comment cites BigQuery's
native pipe support as the reason the flag exists. Every current backend reports `false`, so
BigQuery would be the **first backend to exercise that path** — an untested code path, and a
risk worth naming rather than assuming is free.

The dual-target test harness (`crates/smelt-cli/tests/common/mod.rs`) is likewise ready: a
`TargetKind` enum, `targets_to_run()`, and `targets_yaml()`, with Spark gated on
`SPARK_CONNECT_URL` and skipping *green* when it is absent. A third variant gated on
`SMELT_BQ_PROJECT` is picked up by every parity suite at once.

## Decision: the client layer is a Python adapter, not a Rust crate

`smelt-backend-spark` is a **PyO3 bridge to a thin Python adapter**
(`python/smelt/spark_adapter.py`), with Arrow IPC across the boundary and the invariant that
all SQL generation and orchestration stays in Rust — Python only holds the connection and
returns record batches.

BigQuery should follow this precedent exactly, with a `python/smelt/bigquery_adapter.py`.

*Rejected: a native Rust client.* The available Rust BigQuery crates are thin REST wrappers
that return JSON rows rather than Arrow. The `Backend` trait's currency is
`Vec<RecordBatch>`, and the workspace pins `arrow = 58` to match DuckDB's version — a Rust
client would mean both a manual JSON→Arrow conversion layer and an arrow-version negotiation.
Python's `google-cloud-bigquery` is mature, ADC-native, and has `to_arrow()`. Reusing the
established bridge introduces no new architectural pattern and no new dependency risk.

## Decision: verification is local-gated, with no CI tier

BigQuery tests are feature-gated and skip green when `SMELT_BQ_PROJECT` is unset, exactly as
Spark's were before the `spark-parity` job existed. Credentials are the developer's own ADC
(`gcloud auth application-default login`); no service account, no GitHub secret, no GCP
billing surface in CI.

*Rejected: an emulator-backed fast loop.* `goccy/bigquery-emulator` embeds real ZetaSQL over
SQLite, which makes it high-fidelity for parse acceptance and output schema but divergent in
execution semantics — good for a type oracle, bad for the result and equivalence oracles.
Standing up a second, partially-trusted oracle was judged not worth its cost before the real
one exists. Revisiting it is cheapest immediately after the walking skeleton, when the
measured round-trip latency is known and nothing downstream is sunk.

*Rejected: recorded cassettes.* Fixtures go stale silently and cover generative proptests
poorly — the two properties that matter most for the gates in question.

**This is an accepted gap, not an oversight.** "Spark-equivalent parity" is a claim about
gate *coverage*, not gate *tier*: Spark has a nightly CI job and BigQuery would not, so
BigQuery regressions surface only when the suite is run by hand. This asymmetry belongs in
`multi_backend.md` §Known Divergences so the supported-surface statement stays honest.

## Decision: credentials are least-privilege, encrypted at rest, and short-lived

Development happens on a single-user machine where AI coding sessions run as the developer's own
UID. Any file the developer can read, such a session can read, so "hide the credential" is not
an achievable goal. The design instead layers two real boundaries under two speed bumps.

**Blast radius (real).** A dedicated GCP project, and a service account holding only
`roles/bigquery.jobUser` at project scope plus dataset-scoped `WRITER` on the single test
dataset. Full compromise then means writing to one test dataset and running queries billed to
one capped project.

*Rejected: user application-default credentials.* `gcloud auth application-default login` is the
conventional path and the worst option available here — the resulting token carries the
developer's entire Google Cloud identity across every project they can reach. A leak escalates
from "one test dataset" to "everything".

**Human-in-the-loop (real).** The service-account key is encrypted at rest with a passphrase
(gpg symmetric AES256, openssl `-pbkdf2` fallback). `scripts/bigquery-auth.sh` decrypts to a
temporary file that is shredded on exit, mints a one-hour OAuth access token, and writes it
mode-`600` with an expiry stamp that `scripts/bigquery-env.sh` checks. Outside that window no
usable credential exists, and minting one requires a human to type the passphrase — which an
agent cannot do. The plaintext key never persists.

**Non-default path (speed bump).** The gcloud configuration lives in
`~/.config/gcloud-smelt-bq` via `CLOUDSDK_CONFIG`, never `~/.config/gcloud`. The path every
Google client library probes stays empty, so nothing discovers these credentials ambiently.

**Harness deny rules (speed bump).** `.claude/settings.json` denies `gcloud`, `bq`, the two
BigQuery scripts, and reads of the credential directory.

The adapter authenticates from `SMELT_BQ_ACCESS_TOKEN` explicitly and never falls back to
application-default credentials. That is a security property rather than an implementation
detail: it makes ambient credentials unusable by construction, so the exported short-lived
token is the only route to GCP.

Two accepted consequences. Within the one-hour window an agent session *can* run the BigQuery
suite — chosen deliberately over shell-only export, which would isolate more strictly but
prevent agents from running the tests at all. And a GCP budget **alerts rather than hard-stops**,
so the budget cap bounds surprise, not spend.

## Decision: ordering is walking-skeleton-first

The recommended order gets one trivial model materializing in a real BigQuery dataset before
any breadth work, then widens.

*Rejected: mirroring the Spark programme bottom-up* (dialect → backend → parity → incremental
→ conformance). It follows a path already walked, but the first live query lands several
phases in, and every later phase is blocked behind an auth/latency/cost loop nothing in the
repo has exercised.

*Rejected: oracle-first*, on the theory that GoogleSQL's divergence from DuckDB is the largest
surface. It is a large surface, but a well-documented one, and the printer is already
capability-driven — filling the struct and letting `capability_conformance` report breakage is
most of the work. The oracle also needs a live connection regardless, so it does not actually
front-load the unknown.

The unknown being front-loaded is the **loop**, not the dialect: provisioning, ADC, dataset
lifecycle, per-query latency, and quota behaviour. Those determine whether the remaining work
takes days or weeks, and none of them can be settled on paper. Walking-skeleton-first also
fails cheap — if round-trip latency makes generative conformance untenable, that is known
while reopening the emulator question is still inexpensive.

### Phase shape

0. **Provisioning (human-only).** `gcloud` install, ADC login, project + test dataset,
   billing. Output is `scripts/bigquery-env.sh` mirroring `scripts/spark-env.sh`
   (`SMELT_BQ_PROJECT`, `SMELT_BQ_DATASET`, `SMELT_BQ_LOCATION`). Best delivered as an
   interactive wizard rather than a doc.
1. **Walking skeleton.** `BackendType::BigQuery`, `Target` fields (`project`, `dataset`,
   `location`), `SqlDialect::BigQuery`, a best-effort capability constructor, the Python
   adapter, `crates/smelt-backend-bigquery` with only the required `Backend` methods, and the
   `create_backend` arm. Gate: `bigquery_smoke.rs`. **Done when one model materializes and
   per-query latency and quota behaviour are measured.**
2. **Dialect and printer.** Fill the capability struct empirically, one flag at a time against
   live BigQuery, and add the lowerings. Ends with the `capability_conformance` matrix row.
3. **Type oracle and divergences.** A BigQuery oracle beside `duckdb_oracle.rs`, plus a
   BigQuery column in `divergences.rs` and a differential/gaps ratchet.
4. **Full-refresh parity.** Third `TargetKind` in `materialization_parity`,
   `cross_engine_parity`, `cross_engine_types_parity`, `seed_parity`, `lowering_parity`.
5. **Incremental legs.** `merge_parity`, `incremental_parity`.
6. **Schema evolution.** `schema_evolution_parity`.
7. **Conformance.** `maintenance_conformance_bigquery` at reduced case count, plus
   `statement_parity` and `execute_parity`.
8. **Spec and user docs reconciliation.**

Per the spec-first rule, the `multi_backend.md` capability-matrix and supported-surface edits
land *ahead of* the phases implementing them; phase 8 is the user-facing docs plus final
reconciliation.

**Phase 3 is a re-scoping checkpoint, not a sized phase.** BigQuery has no integer type but
`INT64`, no `FLOAT32`, and one `STRING` type, so a large share of DuckDB-canonical inferences
will diverge. Most of that should be mechanical, absorbed by the existing output-boundary cast
wrap (`type_conformance.rs::wrap_with_type_casts`) — which is the mechanism working as
designed — but the mechanical/semantic split is unknown until phase 2 reports. Phase 3 should
be re-scoped then rather than estimated now.

## Dialect surface to expect

Empirical verification in phase 2 governs; this is the anticipated shape, not a finding.

- Backtick identifier quoting, and project/dataset/table three-part names.
- `SELECT * EXCEPT(...)` — collides syntactically with the set operator `EXCEPT`.
- `STRUCT`/`ARRAY` literal syntax; `NUMERIC`/`BIGNUMERIC` decimal families.
- `QUALIFY` is supported.
- Native pipe syntax — the first `supports_pipe_syntax: true` backend.
- `MERGE` including `WHEN NOT MATCHED BY SOURCE`, so BigQuery sets a flag Spark-over-Parquet
  cannot.
- **No `INSERT OVERWRITE`** — partition replacement lowers to partition-decorator truncation
  or a scoped `DELETE` + `INSERT`. Partitioning and clustering become physical config.
- Restrictive schema evolution: no `ALTER COLUMN ... TYPE ... USING`, narrow type-relaxation
  rules only.

## Dataset lifecycle and test isolation

A BigQuery dataset is the analogue of a schema, and `Target.schema` maps onto it directly, so
per-run isolation is a unique dataset (`smelt_test_<pid>_<nanos>`) created at harness setup
and dropped with `delete_contents` at teardown. This mirrors the Spark warehouse-dir isolation
and makes concurrent runs — two worktrees, or a developer alongside an autonomy loop — safe by
construction.

Teardown does not run on a hard panic or Ctrl-C. Rather than depend on it, the harness sets
**`defaultTableExpirationMs` on the test dataset at creation**, so every table inherits an
expiry and self-deletes. Orphan cleanup becomes structural rather than a matter of discipline.
This belongs in phase 1, which is when crashes are most frequent.

**Cost is not the constraint; quota may be.** Test tables are kilobytes against a 10 MB
per-table-per-query billing minimum and a 1 TiB/month free tier, with DDL free and the first
10 GB of storage free — even a heavy generative suite is effectively free. The real risk is
**per-table modification rate limits**, and `maintenance_conformance` is precisely a workload
that applies repeated DML to one table. The current limit is not stated here because it has
changed over time and varies by operation class; **phase 1 measures it** with a tight DML loop.
That number and the per-query latency together set phase 7's case count. If the limit binds,
the mitigation is a fresh target table per generative case rather than fewer cases.

## Gate matrix

All entries are local-gated, skipping green when `SMELT_BQ_PROJECT` is unset.

| Existing gate | BigQuery equivalent | Phase |
|---|---|---|
| `spark_smoke` | `bigquery_smoke` | 1 |
| `capability_conformance` | matrix row, all fields destructured | 2 |
| `type_property_tests` | BigQuery oracle + `divergences.rs` column | 3 |
| `duckdb_differential` | BigQuery differential + gaps ratchet | 3 |
| `materialization_parity`, `cross_engine_parity`, `cross_engine_types_parity`, `seed_parity`, `lowering_parity` | third `TargetKind` | 4 |
| `merge_parity`, `incremental_parity` | third `TargetKind` | 5 |
| `schema_evolution_parity` | third `TargetKind` | 6 |
| `maintenance_conformance_spark` | `maintenance_conformance_bigquery`, reduced cases | 7 |
| `statement_parity`, `execute_parity` | backend-parametrized | 7 |

## Provisioned environment

Phase 0 is complete. The following exists in GCP and does not need re-deriving:

| Resource | Value |
|---|---|
| Project | `smelt-bq-test-20260816` (dedicated; billing linked) |
| APIs | `bigquery.googleapis.com`, `iam.googleapis.com` |
| Dataset | `smelt_test`, location `US`, 24h default table expiration |
| Service account | `smelt-bq-test@smelt-bq-test-20260816.iam.gserviceaccount.com` |
| Grants | `roles/bigquery.jobUser` (project scope); `WRITER` on `smelt_test` only |
| Credential store | `~/.config/gcloud-smelt-bq` (isolated `CLOUDSDK_CONFIG`) |

Scripts, in the order they run: `bigquery-login.sh` (browser OAuth, human),
`bigquery-provision.sh` (APIs/dataset/service account/IAM, scriptable),
`bigquery-key.sh` (mint + passphrase-encrypt, human), then per session
`bigquery-auth.sh` + `bigquery-env.sh`. `bigquery-verify.sh` proves the chain
including a negative test that writing outside the granted dataset is refused.
`bigquery-setup.sh` remains the single-command path for a fresh machine.

**Measured round-trip cost** (2026-08-16, `US` multi-region, from Melbourne):
`SELECT 1` **745 ms**; `CREATE OR REPLACE TABLE … AS SELECT` **1,945 ms**. So a
statement costs roughly 0.7–2 s against BigQuery, versus sub-millisecond on in-process
DuckDB — three orders of magnitude. A generative conformance case that drives, say, four
run steps of three statements each lands near 20 s of wall-clock, putting a hundred-case
suite at roughly half an hour. That is the number behind the reduced case count phase 7
budgets for, and it argues for concurrency across cases rather than fewer cases.

**The budget alert is not provisioned.** `gcloud billing budgets` authenticates
through Application Default Credentials — the full-identity credential this design
deliberately refuses to create — so automating it would have undone the credential
posture to save a minute of clicking. It stays a manual console step, tracked in
Open Questions below.

## Measured against the live warehouse

The capability column, the type-name surface, and the rate limit were established by executing
statements against the provisioned project (`scripts/bigquery-probe.sh`, `-probe2`, `-probe3`,
`-probe4`), not from documentation.

**The per-table modification rate limit binds, and latency is not the governing constraint.**
Twenty-five sequential `UPDATE`s against one table, spaced by their own ~3 s round trip, all
succeeded — but eight rapid `CREATE OR REPLACE TABLE` statements against a *single* table name
were refused with `Job exceeded rate limits: Your table exceeded quota for table update
operations`. The limit is per table, not per project, and it is reached by burst rate rather
than by total volume. A generative suite must therefore allocate a fresh target table per case;
that mitigation is required, not optional, and it also means concurrency across cases is the
right way to absorb latency rather than cutting cases.

**Type names.** GoogleSQL rejects exactly `VARCHAR`, `TEXT`, `DOUBLE`, `REAL` and `FLOAT`
(`Type not found`). It accepts the integer aliases (`INTEGER`, `BIGINT`, `SMALLINT`, `TINYINT`),
`DECIMAL`/`NUMERIC`/`BIGNUMERIC`, `BOOLEAN`/`BOOL`, `STRING`, `BYTES`, `DATE`, `TIME`,
`DATETIME`, `TIMESTAMP`, `JSON` and `INTERVAL` verbatim. Only the rejected families need
rewriting at the output-boundary cast wrap and in the maintenance emitters' bootstrap DDL, which
is a far smaller surface than "a large share of DuckDB-canonical inferences will diverge"
anticipated — the divergence is concentrated in *type spelling*, not type semantics.

**Two surprises worth naming.** BigQuery accepts `CREATE MATERIALIZED VIEW` with incremental
refresh, making it the first backend whose `supports_native_ivm: false` describes smelt rather
than the engine. And `SHA256` returns `BYTES` rather than a hex string, so every fingerprint
expression needs a `TO_HEX` wrap that neither other dialect requires.

**The least-privilege grant excludes dataset creation.** The service account holds `jobUser`
plus `WRITER` on one dataset, which does not include `bigquery.datasets.create` — so the
dataset-per-run isolation described above is unavailable to it, and the suites fall back to
table-level isolation inside the granted dataset. Widening the grant is a real option (the
project is dedicated and capped), but it was not taken unilaterally.

## Open questions

- **Phase 3's true size** — mechanical versus semantic split of the type-divergence surface.
  Answered by phase 2.
- **Whether to widen the service-account grant to allow dataset creation**, restoring
  dataset-per-run isolation, or to keep table-level isolation inside the one granted dataset.
  The project is dedicated and capped, so the widening is small; the current fallback works.
- **The budget alert is unprovisioned** (see §Provisioned environment). Either accept the
  manual console step permanently, or find a budgets path that does not require
  application-default credentials.
- **Whether the emulator earns a place as a fast inner loop** for parse and type work only,
  once the real round-trip cost is measured. Deliberately deferred, not dismissed.
- **Whether the CI-tier gap should stay permanent.** Recorded as a Known Divergence; a nightly
  job needs a service account and a billing decision that has not been made.

## References

- `docs/specs/multi_backend.md` — parity contract, capability matrix, CI tiering.
- `docs/plans/20260328-spark-backend.md` — the precedent backend programme.
- `crates/smelt-dialect/src/dialect.rs`, `crates/smelt-backend/src/lib.rs`,
  `crates/smelt-backends/src/lib.rs`, `crates/smelt-cli/tests/common/mod.rs`.
