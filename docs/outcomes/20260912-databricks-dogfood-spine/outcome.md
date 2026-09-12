# Outcome: The GitHub-activity pipeline runs on Databricks Free Edition and DuckDB, and the numbers agree

**Created:** 2026-09-12
**Status:** queued
**Driver:** split. Phases 1–3 and 10 are loop-grindable (no workspace, no credentials) and this
outcome sits in `.claude/outcome-backlog` for them. Phase 4 is **human-gated** — it provisions
the workspace objects and mints the credential. Phases 5–9 and 11 run live Databricks and need
the credential phase 4 produces; a headless loop must emit `<<PHASE_BLOCKED>>` for any of them
when `scripts/dbx-dogfood-env.sh` cannot reach the workspace, never skip green. Phase 10
harvests the *committed* summaries of phases 5–9 and needs no credential of its own.
**Source:** `docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md` (the pattern this
repeats on a third target); `docs/research/20260906-bigquery-dogfood.md` §"The programme" (D0,
D1), §"Sequencing: models first, punch-list second"
**Spec anchors:** `docs/specs/multi_backend.md` §Surface (backends, capability table),
§"Connection security", §"Loading data into a backend", §Known Divergences ("Databricks is not
yet a distinct backend"); `docs/specs/smelt_yml.md` §"Target shape"; `docs/specs/sources.md`;
`docs/specs/incremental_models.md` §"The equivalence invariant"; `docs/specs/run_state.md`

## The outcome

`examples/github_activity/` — the bronze→silver→gold→mart pipeline already trusted on DuckDB
and BigQuery — runs to completion on a **third target**: a Databricks Free Edition workspace,
reached through a first-class `type: databricks` target rather than a hand-assembled Spark
Connect URL. Free Edition is serverless-only and Unity-Catalog-only, so the target models
those facts (a Databricks Connect session, catalog-qualified Delta tables, no host-visible
warehouse path) instead of inheriting the local-Spark assumptions that would silently break.
The population is the **same** stable 0.1% GitHub Archive sample the other two targets see: a
loader replays the committed Parquet fixture day by day into a Unity Catalog table, including
the deliberate 2% previous-day redelivery, so the three targets are comparable row for row.
Against that table the pipeline runs a full refresh, then at least three consecutive
incremental windows; every model's output agrees with DuckDB over the same rows, every
incremental state matches a full-refresh oracle, and every divergence is registered rather
than tolerated. The defects the live run surfaces are written down as a punch-list rather than
fixed here. Finally the pipeline stops depending on this machine at all: a Databricks Job
with a daily schedule runs the loader task and then `smelt run` **on the platform's own
compute**, with smelt's run state living in a Unity Catalog Volume, so a day lands and is
processed with no laptop, no token in flight, and nothing outside the workspace.

Everything that can be built without a workspace is built and tested **first**: the target
type, the session builder, the pinned client environment and the loader are green offline
before a credential exists — so the live run is a test of the *backend on Databricks*, not
of the models or the tooling.

## Success criteria (checkable)

1. **A `type: databricks` target exists and is specified.** `docs/specs/multi_backend.md` and
   `docs/specs/smelt_yml.md` describe the target shape (`host`, `token` via `${ENV}` only,
   `catalog`, `schema`, serverless compute), its `BackendCapabilities::databricks()` profile
   (Delta semantics; `warehouse` and Parquet format are not keys of this target and are
   refused with a diagnostic, not ignored), and the connection-security rule that the token
   never appears in a log line or error. The spec's Known Divergence "Databricks is not yet a
   distinct backend" is replaced by a statement of what *is* modelled.
2. **The connection path is Databricks Connect, offline-tested.** The Python adapter builds a
   `DatabricksSession` (serverless) for a `databricks` target and a plain `SparkSession.remote`
   for a `spark` target; which builder a target selects, token redaction, and refusal of a
   `warehouse` key are asserted with no live workspace. A pinned client environment
   (`databricks-connect`, separate from the local-Spark `pyspark` venv, which it conflicts
   with) is reproducible from one script, with `scripts/dbx-dogfood-env.sh` exporting what the
   PyO3-embedded interpreter needs — mirroring `scripts/spark-env.sh`.
3. **Loader, no cloud needed to prove it.** `scripts/dbx-dogfood-loader.py` (or `.sh`) replays
   `examples/github_activity/seeds/github_events_sample.parquet` one day at a time into
   `workspace.smelt_dogfood.github_events` through the backend's own Arrow load path (never a
   host-path file the server cannot see), applying exactly the redelivery rule of
   `examples/github_activity/load_day.sh` and stamping `ingested_date`. A per-PR test proves
   the loader's per-day slice is row-identical to the DuckDB loader's for every day of the
   fixture, and that it is idempotent per day. The loader is documented as **external to
   smelt**; smelt's source declaration is the contract.
4. **Provisioned, scoped, and reachable — Free Edition facts recorded.** The workspace holds
   `smelt_dogfood` and `smelt_dogfood_oracle` schemas in the `workspace` catalog. The
   credential (a service principal with an OAuth machine-to-machine secret if Free Edition
   permits one, else a personal access token — the decision log records which and why) is
   granted on those two schemas and nothing else, stored gpg-encrypted under its own config
   dir exactly as `scripts/bigquery-key.sh` does, and reached from a Claude session only
   through `scripts/dbx-*.sh` wrappers allow-listed in `.claude/settings.json`. Reachability is
   demonstrated, not assumed: a query against the two schemas succeeds and a write outside
   them is refused. Free Edition carries no bill, so in place of a budget cap the outcome
   records the quotas that bound it (serverless concurrency, cold-start latency, storage) as
   measured facts.
5. **Loaded.** The loader has landed at least two fixture days, `ingested_date` stamped and
   the redelivered slice present, verified by a row count against the Parquet fixture's own
   count for those days plus the expected 2%.
6. **Databricks leg, live.** The same model set compiles and runs against the dogfood schema
   — a full refresh, then **at least three consecutive incremental windows** — with the run
   report captured for each. Every compile refusal and runtime failure is *recorded*, not
   fixed in place, unless the fix is the only way a run completes at all.
7. **The two targets agree.** The dual-target comparator already used for BigQuery
   (`scripts/bq-dogfood-parity.sh` and `crates/smelt-cli/tests/github_activity_dual_target.rs`)
   gains a Databricks leg, generalised over the target rather than duplicated. Each model's
   output over the same rows is equal between DuckDB and Databricks, or the difference is a
   registered divergence with a reason. An unregistered difference fails.
8. **The numbers are trustworthy.** After each incremental window, each model's state on
   Databricks equals a full refresh over the inputs seen so far, written to
   `smelt_dogfood_oracle` — the equivalence invariant on a real pipeline, on a third engine.
9. **Evidence banked.** `docs/handoffs/2026-XX-XX-databricks-findings.md` lists every defect,
   divergence, missing emission verdict and unsupported construct the live runs surfaced,
   each with the model and statement that provoked it, plus the Free Edition constraints that
   shaped the design. The spec's Known Divergences are updated from it, and
   `docs-site/` gains a Databricks target page. This document is the input to a follow-on
   `databricks-correctness` outcome, which this outcome does **not** create.
10. **Gates green.** `bash .claude/scripts/verify-phase.sh` passes; no ratchet lowered. The
    Spark parity tier is unaffected: `scripts/spark-up.sh` and its `pyspark` venv keep
    working alongside the new `databricks-connect` environment.
11. **Unattended, fully on the platform.** A Databricks Job, defined as a committed asset
    (`examples/github_activity/databricks/` — a Databricks Asset Bundle or an equivalent
    checked-in job spec, deployed by one `scripts/dbx-*.sh` wrapper), runs daily on
    serverless compute with two tasks in order: the loader lands the next fixture day, then
    `smelt run` processes it. The smelt binary is a released Linux build fetched onto the
    task (not compiled there); the `databricks` target inside the job authenticates with the
    **ambient** session — no token, no `${ENV}` secret — which the spec records as a second,
    credential-free form of the same target; and the project's `.smelt/` run state (ledger,
    manifests, run reports) persists across runs on a Unity Catalog Volume, so each daily run
    is a genuine incremental window over the previous one. Demonstrated, not assumed: at
    least three consecutive **scheduled** runs (not manually triggered) complete, their run
    reports are captured from the Volume, and the state they leave matches a full-refresh
    oracle exactly as criterion 8 checks. The compute budget the schedule consumes is
    recorded against the Free Edition quotas of criterion 4.

## Out of scope

- **Fixing** anything the live run surfaces beyond what is needed to make a run complete at
  all. Fixes belong to a follow-on `databricks-correctness` outcome, scaffolded from
  criterion 9's handoff.
- **Databricks-native capabilities**: native IVM via Enzyme (`supports_native_ivm` stays
  `false`), metrics views, Photon, liquid clustering, `CREATE OR REPLACE TABLE` tuning. The
  capability profile records what the live run proves, nothing speculative.
- **SQL-warehouse execution** (the Statement Execution API) as an alternative to Databricks
  Connect. One connection path is enough to prove the backend; a second is a separate
  decision.
- **Cross-engine exchange** to or from Databricks. The Spark `read_parquet()` substitution
  assumes a shared warehouse filesystem that Free Edition does not have; Volumes-based
  exchange is a later design.
- **Orchestration beyond one job.** Criterion 11's job is the whole platform-side surface:
  no alerting, no log routing to an external sink, no multi-workspace deployment, no
  Workflows fan-out. Failure notification is the job's own built-in email/UI setting.
- **Paid-tier features** (classic clusters, instance profiles, private networking). Free
  Edition is the target; a paid workspace is not assumed anywhere.
- Everything in `docs/research/20260906-bigquery-dogfood.md` §"Out of scope".

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec delta: `type: databricks` target shape, `BackendCapabilities::databricks()` profile, connection-security and loading rules, Free Edition constraints; replace the "not yet a distinct backend" divergence | pending |
| 2 | Backend, offline: `BackendType::Databricks` dispatch, the `DatabricksSession` builder path in the Python adapter, capability profile, `warehouse`/`format` refusal and token redaction, all asserted with no workspace | pending |
| 3 | Tooling, offline: pinned `databricks-connect` venv script, `scripts/dbx-dogfood-env.sh`, and the day loader replaying the Parquet fixture with the redelivery rule, gated by a per-PR slice-identity test against `load_day.sh` | pending |
| 4 | **[human]** Provision: `smelt_dogfood` + `smelt_dogfood_oracle` in the `workspace` catalog, the scoped credential encrypted at rest, `scripts/dbx-*.sh` wrappers and settings allow-list, reachability and refusal demonstrated, Free Edition quotas recorded | pending |
| 5 | **[live]** Load at least two fixture days through the loader; verify counts and the redelivered slice | pending |
| 6 | **[live]** First full refresh of the whole model set on Databricks; record every compile refusal and runtime failure rather than fixing in place | pending |
| 7 | **[live]** Three or more consecutive incremental windows, run reports captured, frontier and engine-resident state inspected between runs | pending |
| 8 | **[live]** Dual-target parity DuckDB vs Databricks over the same rows, via the generalised comparator; register each difference with a reason or fail | pending |
| 9 | **[live]** Trust the numbers: full-refresh oracle in `smelt_dogfood_oracle` vs incremental state after each window | pending |
| 10 | Bank the evidence: the findings handoff, spec Known Divergences updated, docs-site Databricks target page, `ROADMAP.md` item 11 revised | pending |
| 11 | **[live]** Package the pipeline as a daily Databricks Job on serverless compute — committed job asset, released smelt binary fetched onto the task, ambient-session `databricks` target (spec delta), loader task then `smelt run` task, `.smelt/` state on a Unity Catalog Volume — and prove three consecutive scheduled runs against the oracle | pending |

## Decision log

- 2026-09-12 (scaffold): **Free Edition, Databricks Connect, `type: databricks`.** The account
  is Databricks Free Edition (serverless-only, Unity Catalog mandatory, no host-visible
  warehouse directory, no bill). The existing `type: spark` path hands a raw `sc://` URL to
  PySpark's `builder.remote()`, which cannot reach serverless compute; that needs the
  `databricks-connect` client's `DatabricksSession`, whose package conflicts with the plain
  `pyspark` the local-Spark scripts pin. A distinct target type was chosen over a
  `serverless:` flag on `type: spark` so that Free Edition's facts are modelled as a
  capability profile rather than left as legal-but-broken keys, matching the spec's own
  "Databricks is not yet a distinct backend" gap. The population is the committed Parquet
  fixture replayed by a loader — GitHub Archive is not reachable from Databricks — which is
  what makes a three-way parity possible.
- 2026-09-12 (scaffold, addendum): **the pipeline must also run with nothing outside the
  workspace.** Phase 11 packages `smelt run` as a daily Databricks Job on the platform's own
  compute. Two consequences shape it. First, the `databricks` target needs a second,
  credential-free form — inside a Databricks task the session is ambient, and shipping a
  token into the job would be strictly worse than using it. Second, smelt's run state has to
  outlive the task's ephemeral filesystem, so the project directory (or at least `.smelt/`)
  lives on a Unity Catalog Volume; whether a Volume path is fast and consistent enough for
  the interval ledger is a fact the live phase measures, not assumes. The BigQuery
  equivalent (`20260906-bigquery-unattended`) stayed human-gated and unlisted; here it is a
  listed phase because Free Edition has no per-run bill to guard and the job spec is a
  committed asset the loop can author offline before the human deploys it.

## Blocked

