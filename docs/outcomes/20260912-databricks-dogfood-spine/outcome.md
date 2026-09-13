# Outcome: The GitHub-activity pipeline runs on Databricks Free Edition and DuckDB, and the numbers agree

**Created:** 2026-09-12
**Status:** active
**Driver:** split. Phases 1–3, 4a and 10 are loop-grindable (no workspace, no credentials) and
this outcome sits in `.claude/outcome-backlog` for them. Phase 4b is **human-gated** — it runs
the provisioning wizard 4a authors, creating the workspace objects and minting the credential.
Phases 5–9 and 11 run live Databricks and need the credential phase 4b produces; a headless loop must emit `<<PHASE_BLOCKED>>` for any of them
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
11. **Unattended, fully on the platform, deployed as a Databricks Asset Bundle.** The job is
    declared in a committed bundle (`examples/github_activity/databricks.yml` plus its
    `resources/`): one job resource with a daily schedule, a serverless environment, the
    Volume path and the two tasks, with the dogfood workspace as a bundle *target* so a
    second workspace is a target entry rather than a fork. `databricks bundle validate`
    runs per-PR with no workspace (it is a pure config check); `databricks bundle deploy`
    and `databricks bundle run` are the only calls the `scripts/dbx-*.sh` wrapper makes.
    The Databricks CLI is pinned and installed through `mise` the way the Google Cloud SDK
    is (`mise run setup-gcloud`), never assumed on `PATH`. The job runs daily on
    serverless compute with two tasks in order: the loader lands the next fixture day, then
    `smelt run` processes it. smelt reaches the task as a wheel declared in the bundle's
    `artifacts:` block — the same `bindings = "bin"` maturin build the PyPI release already
    uses (root `pyproject.toml`) — which `bundle deploy` builds locally and uploads to
    workspace files itself, so no Volume and no hand-written fetch step are needed for the
    binary. This is a placeholder for a PyPI dependency: dev is ahead of the last PyPI
    release, so the environment installs the freshly-built local wheel rather than a
    published version; once a release tracks dev the `artifacts:` block is dropped in favour
    of a pinned `smelt-sql==<version>` in the environment's `dependencies:`. The `databricks`
    target inside the job authenticates with the
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
| 1 | Spec delta: `type: databricks` target shape, `BackendCapabilities::databricks()` profile, connection-security and loading rules, Free Edition constraints; replace the "not yet a distinct backend" divergence | done |
| 2 | Backend, offline: `BackendType::Databricks` dispatch, the `DatabricksSession` builder path in the Python adapter, capability profile, `warehouse`/`format` refusal and token redaction, all asserted with no workspace | done |
| 3 | Tooling, offline: pinned `databricks-connect` venv script, `scripts/dbx-dogfood-env.sh`, and the day loader replaying the Parquet fixture with the redelivery rule, gated by a per-PR slice-identity test against `load_day.sh` | done |
| 4a | Provisioning tooling, offline: the `dbx-provision`/`dbx-key`/`dbx-auth`/`dbx-verify` wrapper set (credential-agnostic over service-principal-OAuth vs PAT), the `.claude/settings.json` deny/allow split, and the Free-Edition facts sheet skeleton, gated with no workspace | done |
| 4b | **[human]** Run the provisioning wizard: `smelt_dogfood` + `smelt_dogfood_oracle` in the `workspace` catalog, the scoped credential minted and encrypted at rest, reachability and out-of-scope-write refusal demonstrated, Free Edition quotas recorded | blocked |
| 4c | **[live]** Close criterion 4 from a reachable session: `dbx-verify.sh` green on both legs, grants confirmed scoped to the two dogfood schemas and the service principal, `free-edition-facts.md` filled with measured/cited quotas | done |
| 5 | **[live]** Load at least two fixture days through the loader; verify counts and the redelivered slice | done |
| 6 | **[live]** First full refresh of the whole model set on Databricks; record every compile refusal and runtime failure rather than fixing in place | done |
| 6b | **[live]** Unblock the Databricks run path so more than one model can complete: make the maintenance fingerprint's hash spelling dialect-dispatched (`sha2(x, 256)` on Spark/Databricks) behind a single owner plus a structural gate, reconcile the bare-host/scheme mismatch between the wizard's `SMELT_DBX_HOST` and the `type: databricks` target contract, then re-run the full refresh to a clean baseline and record what remains | done |
| 6c | **[live]** Recognise Unity Catalog's `[DROP_COMMAND_TYPE_MISMATCH]` as the drop-type-mismatch condition in `SparkBackend::drop_view_if_exists`/`drop_table_if_exists` (one pure, tested predicate per direction), then land the first clean full refresh of the whole model set and record what remains | done |
| 6d | **[live]** Give a *source-written* cast target the same per-dialect spelling the cast-wrap already gets — bare `VARCHAR`/`TEXT` prints as `STRING` on the SparkSQL dialect — through one shared owner outside the printer, then re-run the full refresh to a clean 16/16 and record what remains | done |
| 6e | **[live]** Register `epoch_us` in the `BuiltinRegistry` (the last construct stopping `silver.actor_sessions` and its 3 dependents) with a SparkSQL/Databricks emission spelling, through the Function-registry single-ownership path — not a printer branch — then re-run the full refresh to a clean 16/16 and record what remains | done |
| 6f | **[live]** Elide the window frame on `LAG`/`LEAD` when emitting SparkSQL/Databricks (Spark refuses any frame on an offset function; the SQL standard and DuckDB both ignore it, so elision is semantics-preserving) via a registry `Emission::Rewrite` verdict planned from the source CST outside the printer, then re-run the full refresh to a clean 16/16 and record what remains | done |
| 7 | **[live]** Three or more consecutive incremental windows, run reports captured, frontier and engine-resident state inspected between runs | done |
| 7b | **[live]** Give the Databricks/Delta target a realisable route for `gold.events_enriched`'s key-addressed model-edge cell — either realise the fingerprint sidecar on Delta, or downgrade the cell at plan-derivation time rather than refusing at execution (the shape `20260906-bigquery-correctness` phase 11 took for its three T5 `bail!` sites). Row 7's three windows recorded no further incremental-path refusal beyond this one — it recurs identically (same model, same error) in every window and is otherwise the only gap — so 7b's scope is exactly this one cell; re-run the windows to a clean 16/16 once it lands | done |
| 8 | **[live]** Dual-target parity DuckDB vs Databricks over the same rows, via the generalised comparator; register each difference with a reason or fail | done |
| 9a | Oracle harness, offline: the `databricks_oracle` target and its `databricks_oracle:` source-name entries (anti-vacuity gated), the equivalence sweep over the shared `parity_support` seam with its negative controls, and `scripts/dbx-dogfood-oracle.sh`'s stages — all provable with no workspace | done |
| 9b | **[live]** Trust the numbers: three consecutive incremental windows (`2026-08-13/14/15`), each followed by a full refresh into `smelt_dogfood_oracle` and compared against the incrementally-maintained state; report committed and its shape tests flipped to hard gates | done |
| 9c | Close both blockers offline, no workspace: register `gold_events_enriched`'s understood `UnorderedColumnDivergence` in the oracle suite's own `EQUIVALENCE_DIVERGENCE_REGISTRY` and land 9b's deferred report gates (criterion 8 closed); then root-cause `silver_actor_naming`'s Databricks duplication from the **maintenance-plan and statement differential** `smelt explain` derives with no connection (`dev` vs `databricks`), and land the resolution one of `## Blocked`'s routes calls for, with an offline gate | done |
| 9d | **[live]** Re-measure both Databricks sweeps under 9c's succession fix: reset and replay the dogfood state from scratch (days 1-8 loaded, full refresh, then windows 9/10/11 each with its oracle refresh), re-run `dbx-dogfood-parity.sh`'s snapshot/manifest/live-test sequence AND the equivalence sweep, commit refreshed `08-parity.json` and `09b-equivalence.json`, restore `dbx_registry_entries_are_all_live`; closes criterion 7 and re-closes criterion 8 on post-fix numbers (rows 8 and 9b come off `## Blocked`) | done |
| 9e | Land the ledger-free succession full rebuild 9c's dispatch fix needs: `rebuild_succession_state` stops refusing when the cell is `state_downgraded`, emitting the presented arm alone (no tombstone DDL, no ledger delete/insert, no clock-tie probe) through one new single-owned emitter in `smelt-logical`, gated by a test that *executes* the downgraded path against a real DuckDB backend rather than only asserting the dispatch decision | done |
| 9f | **[live]** Resume 9d from its task 4 under the 9e fix: full refresh over the already-loaded 8 days, windows 9/10/11 each with its oracle refresh, both sweeps re-run, `08-parity.json` and `09b-equivalence.json` committed, `dbx_registry_entries_are_all_live` and the report-driven gates restored; closes criterion 7 and re-closes criterion 8 (rows 8, 9b and 9d come off `## Blocked`) | done |
| 10 | Bank the evidence: the findings handoff, spec Known Divergences updated, docs-site Databricks target page, `ROADMAP.md` item 11 revised, and `.env` (the wizard library's default `ENV_FILE`, currently untracked-but-unignored) added to `.gitignore` | done |
| 11a | Bundle and tooling, offline: the Databricks CLI pinned and installed through `mise` (`mise run setup-databricks`), the committed Asset Bundle (`examples/github_activity/databricks.yml` + `resources/`) declaring one daily-scheduled serverless job with the loader task then the `smelt run` task, smelt installed from the locally-built `bindings = "bin"` wheel in `artifacts:`, the ambient-credential `databricks` job target, the Volume-resident project/state path, `scripts/dbx-bundle.sh` + its `.claude/settings.json` allow-list entry, the deployment-form spec note and docs-site subsection — all gated per-PR with no workspace (structural bundle test + `databricks bundle validate` when the CLI is present) | done |
| 11b | Make the scheduled job self-driving and serverless-safe, offline: the loader gains `--next-day` (earliest fixture day its own ledger has not recorded) so a scheduled run advances the fixture rather than trusting `{{job.trigger.time.iso_date}}`'s real calendar date; its DuckDB access stops requiring a `duckdb` CLI binary a serverless Python environment will not have; the Unity Catalog Volume the `smelt_run` task points `--project-dir` at is declared as a bundle resource and seeded by a `scripts/dbx-bundle.sh seed` stage that cannot clobber `.smelt/` — all gated per-PR with no workspace | done |
| 11c | **[live]** Deploy and prove it: `databricks bundle deploy` to the dogfood target, the Volume seeded, the schedule enabled, **three consecutive scheduled runs** (not manually triggered) completing under a temporarily compressed cadence (the cron becomes a bundle variable; the committed default stays daily and is restored at the end), their run reports pulled from the Volume, the resulting state compared against a full-refresh oracle exactly as criterion 8 checks, the Volume FUSE layer's `.smelt/lock` advisory-locking and rename-atomicity behaviour measured by a committed `volume_probe` job, and the compute the schedule consumed recorded against the Free Edition quotas of criterion 4 | blocked |

## Decision log

- 2026-09-13 (phase 11c plan): **no split — row 11c stays one row, with the schedule cadence
  made a bundle variable so three *scheduled* runs fit one sitting.** The 11a plan entry flagged
  the scheduled-run leg as wall-clock bound: a daily cron needs three calendar days, which no
  headless implement step can wait out, and splitting the row would not have removed the wait —
  it would only have moved it. Instead the committed `quartz_cron_expression` becomes
  `${var.schedule_cron}` with the daily expression as its default, so the proof deploy passes
  `--var schedule_cron='0 0/20 * * * ?'` and the three runs fire inside ~40 minutes, then the row
  redeploys at the committed default. The criterion's substance is preserved exactly — the runs
  are `trigger: PERIODIC`, never manually triggered, and the committed artifact is still a
  daily-scheduled job, which is what the per-PR gate checks. Two further readings: (a) 11a's open
  question about the Volume FUSE layer's advisory locking and rename atomicity is answered by a
  committed `volume_probe` job rather than inferred from whether the runs happened to succeed,
  because "three runs completed" does not distinguish a working `flock` from one that silently
  no-ops; (b) the Volume's `.smelt/` starts empty while `smelt_dogfood` already holds 9f's tables,
  so scheduled run 1 is a full refresh and runs 2 and 3 are the genuine incremental windows the
  criterion asks for — test 6 of the plan asserts the loader advanced a distinct fixture day each
  run (19 of the fixture's 30 days remain unloaded), which is what makes them windows rather than
  repeats. Nothing left the outcome; nothing added to `## Out of scope`.

- 2026-09-13 (phase 11b plan): **row 11b split into 11b (offline) and 11c (live).** `phases/11a-summary.md`
  "For the next planner" named three defects that make criterion 11's "three consecutive
  **scheduled** runs complete" unreachable as deployed, every one of them provable and fixable
  with no workspace: (a) the loader task is fed `{{job.trigger.time.iso_date}}`, i.e. today's
  real calendar date, while the fixture holds a fixed historical range — the first scheduled run
  would ask for a day that does not exist; (b) the loader's DuckDB access shells out to a
  `duckdb` CLI binary that is very unlikely to exist in a Databricks serverless Python
  environment, and `loader_env` does not declare it; (c) nothing provisions or seeds the Unity
  Catalog Volume the `smelt_run` task already points `--project-dir` at, so `.smelt/` has nowhere
  to persist and no run is a genuine incremental window over the previous one. Doing these inside
  the live row would have meant discovering them one failed scheduled run at a time, each costing
  a day of wall-clock on a daily cron. None of it leaves the outcome — all three serve criterion
  11 directly and are now row 11b, gated offline so the headless loop can grind them; the live
  deploy-and-observe work is row 11c unchanged, plus 11a's open question about the Volume FUSE
  layer's advisory locking and rename atomicity, which only a live Volume can answer. Nothing
  added to `## Out of scope`.

- 2026-09-13 (phase 11b implement): **row 11b done — all three defects closed offline.** The
  loader gained `--next-day` (earliest fixture day the ledger hasn't recorded, live `_loader_days`
  table or dry-run store); DuckDB access now prefers the importable `duckdb` Python module over a
  CLI shell-out, falling back to the CLI only when the module is absent; the Unity Catalog Volume
  is now a declared bundle resource (`resources/volume.yml`) seeded by a new `scripts/dbx-bundle.sh
  seed` subcommand that never touches `.smelt/`. `databricks bundle validate` passes locally with
  the new resource present. See `phases/11b-summary.md` for the full "For the next planner" list
  — notably, neither `duckdb` nor `pyarrow` were importable in this session's ambient Python, so
  the module/CLI Arrow-byte parity test skipped rather than ran; 11c should rebuild
  `.smelt-dbx-venv` to pick up the new `duckdb` pin before relying on that gate.

- 2026-09-13 (phase 11a implement): **row 11a done — the Asset Bundle, CLI pinning and offline
  `validate` gate all landed and are green.** Two of the plan's own assumptions turned out wrong
  under real tooling, both recorded in `phases/11a-summary.md` Decisions: (a) `databricks bundle
  validate` is not actually workspace-free — CLI v1.16.1 unconditionally calls SCIM `Me` plus
  `workspace/get-status`/`mkdirs` regardless of what the bundle config references, so the
  per-PR gate needed a genuine (if minimal) local stub
  (`scripts/dbx_bundle_validate_stub.py`), not merely an absent credential; (b)
  `workspace.host` cannot be templated at all (the CLI hard-refuses interpolation on
  authentication fields), so the host comes from `DATABRICKS_HOST` at invocation time rather
  than a bundle variable — no real hostname is ever committed either way, which preserves the
  criterion's actual intent ("a second workspace is an entry, not a fork") even though the
  literal test wording changed. Also found and fixed: `databricks_job`'s `${DATABRICKS_HOST}`
  reference broke `Config::load` in six existing test call sites, because
  `docs/specs/smelt_yml.md`'s interpolation pass is whole-file — every `${VAR}` in the committed
  `smelt.yml` must resolve regardless of which target is selected. All six now stub
  `DATABRICKS_HOST` alongside their existing `SMELT_DBX_HOSTNAME`/`SMELT_DBX_TOKEN` stubs. Found
  by running `cargo test --workspace --no-fail-fast` rather than trusting `verify-phase.sh`'s
  fail-fast report of the first failure alone. Two open questions carried to 11b, not resolved
  here (out of 11a's scope): the Volume this outcome's job targets is never provisioned or
  seeded by anything yet, and the daily schedule's `{{job.trigger.time.iso_date}}` will ask the
  loader for real-calendar dates the fixture does not have until the loader (or the
  schedule/date mapping) changes. `bash .claude/scripts/verify-phase.sh` ALL GREEN; 426/426 test
  binaries pass under `cargo test --workspace --no-fail-fast`. See `phases/11a-summary.md`.
  Nothing left the outcome; nothing added to `## Out of scope`.

- 2026-09-13 (phase 11 plan): **reshaped — row 11 split into 11a (offline) and 11b (live);
  nothing left the outcome.** Row 11 as written bundles four separable deliverables (CLI
  pinning, the committed bundle + per-PR `validate` gate, the live deploy, and three
  *scheduled* runs judged against the oracle) behind a live gate, which would have made the
  whole of criterion 11's checkable configuration un-grindable whenever the workspace is
  unreachable — and the scheduled-run leg is wall-clock bound (a daily schedule needs three
  calendar days), so pinning it to the same row as the config work would idle the loop for
  days. 11a is everything provable with no workspace; 11b is the deploy and the three
  scheduled runs. Two readings recorded while planning 11a: (a) the **ambient-credential spec
  delta criterion 11 names is already landed** — `docs/specs/smelt_yml.md` §"Target shape"
  specifies `token` absent as the in-workspace ambient form and
  `python/smelt/databricks_adapter.py` implements it — so 11a's remaining spec work is only
  the *deployment form* note (ambient token + job-supplied host) in
  `docs/specs/multi_backend.md` §"Connection security"; (b) `.smelt/` is project-root-relative
  with no configurable state directory (`smelt-state/src/file_store.rs`), so "state on a
  Volume" is achieved by the job running the project *from* a Unity Catalog Volume path rather
  than by a new smelt flag — no spec change, and whether the Volume's FUSE layer supports
  `.smelt/lock` advisory locking and rename-atomic writes is a live question for 11b to
  measure and record. Nothing added to `## Out of scope`.

- 2026-09-13 (phase 10 plan): **no reshape — row 10 stands as written; recorded one placement
  reading.** 9f's summary closed criteria 7 and 8 on post-fix live numbers and surfaced nothing
  that needs a new row: the only outstanding difference is the already-registered
  `gold_events_enriched` `UnorderedColumnDivergence`, which row 10 banks rather than fixes (fixes
  are `## Out of scope`, deferred to the follow-on `databricks-correctness` outcome). Criterion
  9's "docs-site gains a Databricks target page" is planned as a `### Databricks` **section of
  `docs-site/docs/guide/targets.md`**, not a new file, because that page is where DuckDB, Spark
  and BigQuery are each documented and it is already registered with the `smelt docs` front door;
  a separate page would need a nav entry and would split the backend comparison across two
  documents for no reader benefit. Nothing left the outcome; nothing added to `## Out of scope`.

- 2026-09-13 (phase 9f implement): **row 9f done — criteria 7 and 8 closed on post-fix live
  numbers; rows 8, 9b and 9d come off `## Blocked`.** Resumed 9d's staged state (8 fixture days,
  26,220 rows, 2026-08-05..08-12) unchanged; built the binary from HEAD so the live legs ran
  under 9e's `emit_succession_full_rebuild_ledgerless` fix. A full refresh over
  `[2026-08-05, 2026-08-13)` landed a clean **16/16**, with `silver.actor_naming` at exactly
  25,700 rows (matching DuckDB) instead of the pre-fix 26,177 with up to 7x duplication. Windows
  9/10/11 (`2026-08-13/14/15`) each ran 16/16 and matched their oracle refresh exactly at every
  relation, including `silver_actor_naming`. The equivalence sweep
  (`09b-equivalence.json`) confirms `silver_actor_naming` exact at all three checkpoints;
  `gold_events_enriched` alone diverges, matching its already-registered
  `UnorderedColumnDivergence`. The parity leg — an 11-day DuckDB replay (after clearing a stale
  local `examples/github_activity/.smelt/targets/dev` + `target/dev.duckdb` from an earlier
  session that tripped `SourceMutationProfileViolated` on a fresh reload) against the Databricks
  end state — measured `silver_actor_naming`'s `dbx_only` at **0** (was 520 in the pre-fix
  `08-parity.json`); only `gold_events_enriched` still diverges, identically bounded. Landed the
  two deferred report gates in `github_activity_dual_target.rs`
  (`the_committed_parity_report_shows_no_unregistered_difference`,
  `dbx_registry_entries_are_all_live`, plus their `dbx_parity_report`/`dbx_divergent_relations`
  helpers) red-green against the fresh `08-parity.json` — 26/26 offline. The three deferral
  comments (module doc, the "live Databricks sweep" section header, and the trailing stub) were
  replaced with what actually landed. `cargo test -p smelt-cli --test github_activity_dbx_oracle`
  stays 18/18 against the refreshed `09b-equivalence.json`.
  `cargo test -p smelt-runtime --test succession_downgraded_rebuild --test statement_parity` and
  `cargo test -p smelt-cli --test maintenance_conformance` all green, untouched by this phase.
  `bash .claude/scripts/verify-phase.sh` green. See `phases/09f-summary.md`. Nothing left the
  outcome; nothing added to `## Out of scope`.

- 2026-09-13 (phase 9e implement): **row 9e done — the ledger-free succession full rebuild
  lands entirely offline.** `presented_arm_statement` extracted as the shared fold both
  `emit_succession_full_rebuild` and the new `emit_succession_full_rebuild_ledgerless` call, so
  the two paths cannot drift; the new emitter is infallible and dialect-blind (no
  `check_succession_dialect` call), matching the plan. `rebuild_succession_state` now branches
  on `cell.state_downgraded` before the `realises_tombstone_ledger` guard, taking the
  ledger-free path with empty ensure/cleanup lists through the same
  `retry_backend_call`/`execute_write_with_bookkeeping` seam. The new
  `tests/succession_downgraded_rebuild.rs` executes the downgraded path against a real DuckDB
  backend (test 3 in the plan's numbering), confirms it reports exactly one statement (test 4),
  confirms the non-downgraded backstop still works (test 5, asserted against DuckDB's own
  `realises_tombstone_ledger() == true` since a genuinely ledger-less backend isn't available
  offline), and confirms idempotent re-runs (test 6). `docs/specs/state.md` §"The degradation
  contract" extended with what the downgraded rebuild writes/skips, per the plan's spec delta.
  All named verification gates green, `bash .claude/scripts/verify-phase.sh` green (fmt and
  clippy needed one round of fixes: `cargo fmt` on the new files, and
  `#[allow(clippy::too_many_arguments)]` on both new `smelt-logical` functions, matching this
  module's existing convention). Task 7's deferral comment in
  `github_activity_dual_target.rs` updated to describe the fix and point restoration of the
  report gates at phase 9f. Nothing left the outcome; nothing added to `## Out of scope`.

- 2026-09-13 (phase 9e plan): **Split 9d's residue into an offline fix row (9e) and a live
  replay row (9f); 9d stays blocked as the record of the finding.** 9d's summary root-caused the
  blocker precisely — `rebuild_succession_state` carries the same unconditional
  `realises_tombstone_ledger` refusal 9c's dispatch fix routes *into* — and that fix is pure Rust
  plus an emitter, provable with no workspace, so it is loop-grindable work serving criteria 6, 7
  and 8 and gets its own row rather than waiting on a live session. The live re-measure is
  everything 9d's tasks 4 onward already specify, unchanged, so 9f inherits that scope verbatim
  instead of being re-planned. Both rows sit before row 10, because row 10 harvests the findings
  the live replay produces. Of the three candidate routes `## Blocked` records, 9e takes route 1's
  *behaviour* (no ledger read or write at all) via route 2's *structure* (a distinct emitter in
  `smelt-logical`, per `CLAUDE.md` §"Maintenance-plan purity"), and adopts route 3's lesson as a
  hard requirement on the test list — the branch is keyed on `cell.state_downgraded`, not on the
  dialect, which makes the downgraded path executable against a real DuckDB backend offline and
  removes the need for a synthetic no-ledger dialect. Nothing left the outcome; nothing added to
  `## Out of scope`.

- 2026-09-13 (phase 9d plan): **9d widened from "re-run the parity sweep" to "reset, replay and
  re-measure BOTH sweeps".** 9c's summary established that the fix changes the execution shape of
  every incremental window for `silver.actor_naming` on Databricks (full rebuild, not window-forward
  patch), so the committed `08-parity.json` and `09b-equivalence.json` both describe a run sequence
  the fixed code would never produce. Patching only the one model's state, or re-running a single
  window on top of the existing state, would leave the committed reports describing a sequence
  nobody ran; a clean replay under the fixed binary is the only evidence that supports restoring the
  liveness ratchet. The replay reloads the source day-by-day after a `TRUNCATE` rather than relying
  on `--event-time-*` bounds to hide the later days, so oracle validity ("the source holds exactly
  the inputs seen so far") is reproduced by construction rather than assumed. This also means 9d
  closes criteria 7 and 8 together in one live sequence instead of paying for two replays; rows 8
  and 9b are flipped by 9d's own evidence. Nothing left the outcome; nothing added to
  `## Out of scope`.

- 2026-09-13 (phase 9d implement): **9d blocked on a second live-only gap in 9c's fix.** The
  offline scaffolding (spec delta to `docs/specs/state.md` §"The degradation contract"; the
  `dbx-dogfood-oracle.sh reset` stage and its test; the parity script's 11-day/checkpoint
  defaults and its lockstep test) landed and is committed. The live replay's reset, DDL and
  8-day reload all succeeded (`github_events` at 26,220 rows, verified against the raw fixture's
  per-day counts), but the full refresh then failed on both succession-grain models:
  `rebuild_succession_state` — the function 9c's dispatch fix routes a `state_downgraded` cell
  to — carries the same unconditional tombstone-ledger gate as the function it was routed away
  from, so the fix never actually reaches Databricks. 9c's own tests only assert the dispatch
  *decision*, never execute either function against a real backend, so no offline gate caught
  this. See `## Blocked`'s phase 9d entry for the root cause, the state left in the workspace,
  and three candidate fix routes for a follow-up phase (tentatively 9e). Rows 8 and 9b stay
  `## Blocked` under their existing entries; 9d's own row is `blocked` rather than `done`.

- 2026-09-13 (phase 9c implement): **row 9c done — criterion 8 closed, and the Databricks
  succession-fold defect root-caused and fixed, entirely offline.** Criterion 8: registered
  `gold_events_enriched`'s `UnorderedColumnDivergence` in `github_activity_dbx_oracle.rs`'s own
  `EQUIVALENCE_DIVERGENCE_REGISTRY` (mirroring the dual-target suite's identical entry) and landed
  the two deferred report-driven gates — 20/20 on that suite. Root cause of `silver_actor_naming`'s
  Databricks-only duplication (row 8/9b): `resolve_live_succession_cell`
  (`crates/smelt-runtime/src/maintenance_driver/succession/mod.rs`) treated any state-downgraded
  cell as "not live", so on Spark/Databricks — where `realisable_state_structures` realises
  nothing, including the `TombstoneLedger` a succession cell's ideal `SuccessionPatch` technique
  needs — the model fell through to the generic `DeleteInsert` driver, which has no `(key, clock)`
  fold. This was found via the plan's own named differential (`maintenance_plan_report` +
  `resolve_availability` at two dialects, no workspace), though `smelt explain` turned out to have
  no `--target` flag to run the differential through the CLI literally as the plan described —
  the library functions were called directly instead, which is a more precise experiment anyway.
  The measured branch was the *opposite* of the plan's own cheapest-first guess (same technique on
  both targets, pointing at the write path): the technique **differs** (downgraded on Databricks),
  which is `## Blocked`'s route 3, not route 1. Fix: `SuccessionCell` gained a `state_downgraded`
  flag; the resolver now stays live for a downgraded cell and the dispatch site
  (`crates/smelt-runtime/src/execute/project/mod.rs`) forces the fold-preserving full-rebuild
  route (`rebuild_succession_state`) whenever a cell is downgraded, never the window-forward patch
  loop. Confirmed via a new resolver-level unit test and a new offline differential test against
  the real `examples/github_activity` project (`explain_maintenance/
  databricks_succession_differential.rs`); `statement_parity`, `walk_coverage`,
  `maintenance_conformance` (104/104) and the full `verify-phase.sh` all green. See
  `phases/09c-summary.md`. **9d must now re-run BOTH the parity and equivalence-oracle sweeps**
  (not parity alone) — the fix changes every incremental run's execution shape for this model, so
  the currently-committed `08-parity.json`/`09b-equivalence.json` are stale for
  `silver_actor_naming`. Nothing left the outcome; nothing added to `## Out of scope`.

- 2026-09-13 (phase 9c plan): **row 9c split into 9c (offline) / 9d (live), and 9b's inference
  corrected.** (a) *The inference.* Row 9's own decision rule was "if the full refresh on
  Databricks duplicates too, the defect is in the model's own SQL under SparkSQL; if it does
  not, it is in the succession-patch write path." 9b measured `silver_actor_naming`'s
  incremental state **equal** to the Databricks full-refresh oracle at all three checkpoints
  while row 8 measured that same incremental state 520 rows **above** DuckDB — so the Databricks
  full refresh carries the duplication as well, and 9b's summary drew the inverted conclusion
  ("confined to the incremental write path"). The defect is shared by both Databricks paths,
  which rules route 1 (a ledger-transaction/tombstone theory specific to the incremental
  MERGE) *less* likely, not more. (b) *What that makes checkable offline.* Both Databricks
  paths fold `(key, clock)` by construction — `emit_succession_patch`'s `__smelt_dedup` CTE and
  `emit_succession_full_rebuild`'s `ROW_NUMBER() … = 1` — and `incremental_shapes.md`
  §`SuccessionClockTie` makes the fold normative ("identical rows are a redelivery and fold
  once"). Databricks' 26,220 rows is exactly the source row count, i.e. *no fold at all*, which
  says the succession emitters are very likely not the statements this model runs on that
  target. The maintenance plan is pure data and `smelt explain` reads target metadata, never a
  connection (`docs/specs/cli.md`), so which technique each target assigns and which statements
  each emits is a **free offline differential** — the decisive experiment needs no workspace and
  no token. (c) *The split.* Root-causing and fixing therefore moves to an offline row 9c and
  only the re-run stays live in 9d, the same shape the 9a/9b split took for the same reason (a
  one-hour OAuth token is the scarce resource; phases 6-9 each showed it). (d) *Criterion 8
  closes in 9c.* `gold_events_enriched` is the only violation in the committed equivalence
  report, and it is understood, bounded and already registered in the dual-target suite, so
  registering it in the oracle suite's registry and landing 9b's deferred report-driven gates
  (tests 5/6 — both read the committed JSON, no snapshot) is offline work that finishes
  criterion 8 rather than waiting on 9d. Nothing left the outcome; nothing added to
  `## Out of scope`.

- 2026-09-13 (phase 9b implement): **row 9b blocked per its own plan contingency — the
  measurement is complete and committed, and it resolves phase 8's open question.** Two
  script bugs were fixed to make the live sequence runnable at all (`dbx-dogfood-loader.py`
  takes `--date`, not the `--day` the oracle script passed; the oracle's repeated
  `--full-refresh` over the same already-populated `smelt_dogfood_oracle` tables needs
  `--allow-full-refresh` per `docs/specs/sources.md` §Semantics 5's retention gate — unlike
  the BigQuery oracle, this target never drops its scratch schema between checkpoints). Also
  cleaned a stale local `examples/github_activity/.smelt/`/`target/` (gitignored, observability
  only per `docs/specs/run_state.md`) that blocked a fresh `--first-full-refresh`. The measured
  sweep: `silver_actor_naming` matches exactly at all three checkpoints — the full-refresh
  oracle does not reproduce row 8's 520-row incremental-only duplication, confining that
  defect to the succession-patch write path (`## Blocked`'s options 1/3, not 2).
  `gold_events_enriched` violates identically to row 8's already-registered
  `UnorderedColumnDivergence` (not new). Per the plan's contingency, tests 5-6
  (`the_committed_equivalence_report_shows_no_violation`,
  `equivalence_registry_entries_are_all_live`) were NOT landed — whether to register
  `gold_events_enriched` in this suite's own registry is left to 9c or later, deliberately not
  decided here. Tests 2-4 landed (15/15 offline); `bash .claude/scripts/verify-phase.sh` green.
  See `phases/09b-summary.md`. Nothing left the outcome; nothing added to `## Out of scope`.

- 2026-09-13 (phase 9b plan): **no reshape; the contingency on a non-clean sweep is fixed in
  advance.** Phase 9a's summary reports nothing left undone and nothing new surfaced, so the
  remaining rows (9b, 9c, 10, 11) stand as written. One planning decision was needed: row 8's
  `silver_actor_naming` divergence makes it likely 9b's sweep is *not* clean, and a hard
  "report shows no violation" gate over a violating report would leave `cargo test` red. The
  plan therefore splits 9b's gates in two — the coverage, anti-vacuity and nothing-exempt
  gates land unconditionally (they pass whatever the numbers say), while the
  no-violation gate and the two-sided liveness ratchet land only if the sweep is clean and
  otherwise move into row 9c, which already owns restoring the parallel parity ratchet. The
  committed report is the deliverable either way, because it is precisely the evidence 9c
  needs to tell an incremental-write-path defect from one shared with the full refresh. This
  mirrors phase 8's shape (evidence committed, ratchet deferred to keep the suite green) and
  adds no work to and removes no work from the outcome.

- 2026-09-13 (phase 9 plan): **row 9 split into 9a/9b, and a new row 9c added to finish
  criterion 7.** (a) *Split.* Row 9 as written bundled a body of offline construction (a new
  `databricks_oracle` target, its source-name entries, a whole equivalence sweep over the
  shared `parity_support` seam, a driver script) with three live windows on a workspace whose
  OAuth token lasts one hour. Phases 6-8 each showed the live budget being the scarce thing;
  building the harness in a phase that needs no workspace at all means the live phase is
  purely "run the sequence and commit the report", and a token expiry costs a re-run rather
  than losing the harness work. Nothing left the outcome — 9a+9b together are exactly the old
  row 9. (b) *Oracle validity is free on this target.* The BigQuery oracle needed
  `UNBOUNDED_REFRESH_RELATIONS` because BigQuery's source tables statically held all thirty
  days, so a whole-source refresh at an intermediate checkpoint saw inputs the incremental leg
  had not. On Databricks the loader lands one day at a time and the source holds **only** the
  inputs seen so far, so a full refresh over the source *is* a full refresh over the inputs
  seen so far at every checkpoint. That exemption machinery is therefore not ported; the
  premise it replaces is asserted per checkpoint instead (the source's day count equals the
  window number), so it is measured rather than assumed. (c) *New row 9c.* Criterion 7 is
  currently blocked on `silver_actor_naming`'s 520-row Databricks-only duplication, and
  criterion 7 is a success criterion, so the work cannot leave the outcome. It gets its own
  row rather than reopening row 8, placed after 9b because 9b's oracle leg is the decisive
  measurement: if the full refresh on Databricks duplicates too, the defect is in the model's
  own SQL under SparkSQL; if it does not, it is in the succession-patch write path, which is
  `## Blocked` option 1 or 3. Planning that resolution before that evidence exists would be
  guesswork. Nothing added to `## Out of scope`.

- 2026-09-13 (phase 8 implement): **row 8 blocked — the generalised comparator, the
  Databricks sweep infrastructure, and one root-caused divergence all landed and are
  committed; a second, larger divergence was found but not resolved.** Backfilled
  `gold.events_enriched`'s coverage gap first (`smelt run --target databricks
  --event-time-start 2026-08-07 --event-time-end 2026-08-10 -s gold.events_enriched+`),
  confirming `intervals.json` contiguous `[2026-08-05, 2026-08-13)` for every
  window-addressed model. Ran `duck`/`dbx-snapshot`/`manifest`/the live test.
  **Divergence 1 (root-caused, registered):** `gold_events_enriched.current_repo_name`
  differed on 7 of 25,786 rows — a repo renamed at `2026-08-12 04:38:01`; `gold.repo_dim`
  and `silver.repo_naming` agree exactly between the two legs (verified live), but
  Databricks' pre-rename rows (written 2026-08-05/06) never healed. Root cause: phase 7b's
  `EnrichmentKeyed` downgrade to window-scoped `DeleteInsert` (Spark/Delta has no
  `MergeLedger`) sacrifices `crates/smelt-runtime/src/execute/enrichment_heal.rs`'s
  unwindowed run-level heal that DuckDB's `ColumnScopedMerge` cell performs every run —
  exactly the trade-off 7b's own decision log named as deferred to `databricks-correctness`.
  Registered with a new `DivergenceBound::UnorderedColumnDivergence` variant (added to
  `parity_support`, reusable by any suite): licenses one named column to differ with no
  enforced direction — unlike `MonotoneDivergence`, `current_repo_name` is a string with no
  natural "ahead"/"behind" — while the row-key set and every other column must still match
  exactly. **Divergence 2 (found, NOT resolved):** `silver_actor_naming` has 520 more rows
  on Databricks than DuckDB (`duckdb_only=0, databricks_only=520`); a `DISTINCT` count shows
  Databricks holds 26,177 distinct rows against DuckDB's 25,700 (both engines see the
  identical 26,220-row/25,786-distinct-id source, confirmed live) — some Databricks rows
  reach up to 7x literal duplication (`SELECT actor_id, created_at, COUNT(*) ... HAVING
  COUNT(*) > 1` returns rows with `c` up to 7). `silver.actor_naming` is a succession-grain
  model (`docs/specs/incremental_shapes.md` §"The succession grain") maintained by the
  succession-patch technique's tombstone mechanism
  (`crates/smelt-runtime/src/maintenance_driver/succession/execute.rs`) — DuckDB's write
  path evidently collapses a redelivered duplicate event onto the same `(actor_id,
  created_at)` succession row (no duplication at all: `total == distinct`), while
  Databricks' does not. This looks like a second, independent gap in the maintenance
  layer's write primitives on Delta — plausibly related to the same "Spark has no
  cross-table transaction for a ledger write" premise this file's own 2026-09-13 research
  note already flags for planner triage, though this is the succession/tombstone
  mechanism rather than `MergeLedger`. **Not root-caused past this point** — designing a
  bound for genuine row-count/duplication divergence (as opposed to a value divergence on
  a matched key) would need either a new comparator primitive (a distinct-set bound) or a
  runtime fix, and reading `succession/execute.rs` to confirm the write-path root cause is
  more than this phase's remaining budget. Left for the next planner alongside the
  MergeLedger/Catalog-Commits triage above, since both bear on the same question: does the
  maintenance layer's Delta-targeting write path need work that outsizes this outcome's
  "only fix what's needed for a run to complete" licence. **To keep `cargo test` green**,
  the committed Databricks parity report and its liveness ratchet
  (`dbx_registry_entries_are_all_live`, mirroring `registry_entries_are_all_live`) were
  deliberately NOT added this phase — landing them over a report showing an unregistered
  divergence would make the standing offline gate permanently red. The six offline tests,
  the shared `parity_support` generalisation (tasks 1-5), the two scripts (tasks 6-7), and
  the `gold.events_enriched` backfill (task 8) are all committed and green
  (`bash .claude/scripts/verify-phase.sh`). See `phases/08-summary.md`. Nothing left the
  outcome; nothing added to `## Out of scope`.
- 2026-09-13 (research note, not a phase decision — **planner triage needed**): **the "Spark
  has no cross-table transaction" premise behind `MergeLedger`/`ReconciliationLedger`/
  `TombstoneLedger`'s permanent Spark absence (`docs/specs/state.md` §"Which dialects realise
  which structure", `crates/smelt-logical/src/maintenance/availability/state_structure.rs`'s
  `realisable_state_structures`) may no longer hold on Databricks specifically.** Databricks
  shipped **Catalog Commits** (GA, 2026) — Unity Catalog becomes the commit coordinator for
  Delta (built on the Coordinated Commits protocol) and explicitly supports running multiple
  SQL statements across multiple UC-managed Delta tables as one atomic commit
  ([docs](https://docs.databricks.com/aws/en/tables/features/catalog-commits),
  [GA announcement](https://www.databricks.com/blog/convergence-open-table-formats-and-open-catalogs-catalog-commits-generally-available)).
  That is precisely the primitive the doc comment says Delta lacks ("per-table atomicity and
  no cross-table transaction, so a ledger write and its data write cannot be made atomic").
  This is **Databricks/Unity-Catalog-specific** (requires Catalog Commits enabled on UC-managed
  tables), not a property of generic Spark-on-Delta-Lake-OSS — the `SqlDialect::SparkSQL` arm
  covers both today, so unlocking this needs either a Databricks-specific capability flag or a
  confirmed UC-only distinction, not a blanket flip of the `SparkSQL` row. Also unconfirmed:
  whether Databricks Free Edition (this outcome's target) has Catalog Commits available/enabled
  at all, and whether smelt's own transaction model (single Spark Connect session, no explicit
  multi-statement SQL transaction API surfaced yet) can actually drive a coordinated commit
  from the client side. Raised by the user mid-loop questioning the "permanent absence" framing
  after phase 7b's bug (a `MergeLedger`-requiring `ColumnScopedMerge` cell downgrading on
  Spark). **Not investigated or acted on here** — this is a design question bigger than one
  phase (would touch `realisable_state_structures`, `docs/specs/state.md`'s dialect table, and
  potentially years of "no" rows). Flagging for the next spec/plan pass to triage: confirm
  Free-Edition support empirically, decide whether it's in scope for this outcome or a
  follow-up outcome, and if in scope, spec the capability split before touching the `Technique`
  downgrade logic again.
- 2026-09-13 (phase 8 plan): **no reshape; phase 8 absorbs the `gold.events_enriched`
  backfill.** `07-summary.md` flagged that this one model's coverage has a hole
  (`[2026-08-07, 2026-08-10)`) from the windows that failed before 7b landed, and warned that
  rows 8 and 9 must not assume contiguity. Parity over a hole would measure the hole, so the
  backfill run is task 8 of phase 8 rather than a row of its own — it is one `smelt run`
  invocation, not a body of work, and row 9 inherits contiguous coverage from it. The other
  discovery (`completed_at`/`duration_ms` now populated, so the row-7 gap is not reproducible
  on a clean run) is punch-list material row 10 already owns. Generalisation route chosen for
  criterion 7's "generalised over the target rather than duplicated": rename
  `bq_parity_support` → `parity_support`, make the landing seam's export encoding an explicit
  documented contract instead of a BigQuery-shaped branch, share the manifest struct, and split
  the model-exclusion constant per sweep (Databricks excludes nothing — 6b–6f closed every
  construct). Nothing left the outcome; nothing added to `## Out of scope`.
- 2026-09-12 (phase 7b implement): **row 7b done — 16/16 clean on all three new windows; the
  real live bug was one level deeper than the plan's own root-cause analysis.** The plan assumed
  `gold.events_enriched`'s failing cell was ideally-derived as `PerGroupRecompute` directly; in
  fact its only key-addressed candidate is the `gold.repo_dim` edge's `ColumnScopedMerge` cell
  (`KeyDiscovery::EnrichmentKeyed`, a value-enrichment join), which ideal derivation never gives
  a `PerGroupRecompute` technique. The actual failure path: Spark has no `MergeLedger`, so
  availability resolution downgrades this `ColumnScopedMerge` cell via `recompute_equivalent`'s
  **generic** `key_scope.is_some() → PerGroupRecompute` rule — which, before this phase, did not
  distinguish discovery routes and handed the key-addressed driver a `PerGroupRecompute` cell it
  can never execute for `EnrichmentKeyed` (the variant's own doc comment says the driver never
  dispatches it). Fixed by making `recompute_equivalent` route on `KeyDiscovery` explicitly:
  `UpstreamKeyed`/`DownstreamGrainOverUpstream` downgrade to `PerGroupRecompute` (the driver
  dispatches these) and `EnrichmentKeyed` downgrades straight to `DeleteInsert`. This is the
  same `recompute_equivalent`/`required_state_structure` pair the plan already scoped, so no
  file beyond what the plan's tasks touched was needed. W4 (`2026-08-10`), W5 (`2026-08-11`),
  W6 (`2026-08-12`) each landed 16/16 success/0 failed/0 skipped; `intervals.json` confirms
  every model ends at `2026-08-13` (`gold.events_enriched` carries a coverage gap
  `[2026-08-07, 2026-08-10)` from windows that failed before this fix — noted for row 8/9's
  planner, not fixed here). See `phases/07b-summary.md`. Nothing left the outcome; nothing added
  to `## Out of scope`.
- 2026-09-12 (phase 7b plan): **route chosen — downgrade at plan derivation, not sidecar
  realisation on Delta.** Root cause verified: `availability::required_state_structure` is keyed
  on `Technique` alone and returns `None` for `PerGroupRecompute`
  (`crates/smelt-logical/src/maintenance/availability/state_structure.rs:119`), so a
  key-addressed model-edge cell's sidecar need is never seen by `resolve_availability` and is
  instead re-discovered at run time by `maintenance_driver/key_addressed/mod.rs:124` — a second
  source of truth for a plan fact, which maintenance-plan purity forbids. Fix: make the
  requirement cell-shaped (a `PerGroupRecompute` cell carrying an `UpstreamKeyed` /
  `DownstreamGrainOverUpstream` `key_scope` requires the `FingerprintSidecar`) and downgrade it
  to `DeleteInsert`. Realising the sidecar on Delta (Spark emitters in `ddl_spark.rs`, a
  capability flip, new backend seams) is a correctness feature for the follow-on
  `databricks-correctness` outcome, beyond this outcome's "only way a run completes at all"
  licence.
- 2026-09-12 (phase 7b plan): **no reshape.** Phase 7's one new discovery — run reports'
  `completed_at`/`duration_ms` are never populated — is a defect the live run surfaced, which
  this outcome's Out-of-scope section says is recorded rather than fixed; row 10 already
  commits to listing every such defect in the findings handoff, so it needs no row of its own.
  Row 7b's scope is unchanged from what row 7 recorded: one cell, one model.

- 2026-09-12 (phase 7): **three incremental windows landed and inspected, all matching the plan's
  predictions exactly.** W1/W2/W3 (`2026-08-07`/`08`/`09`) each ran 14 success / 1 failed
  (`gold.events_enriched`) / 1 skipped (`marts.star_growth`); `intervals.json` advanced
  `2026-08-07→08→09→10` for every successful model while `gold.events_enriched` stayed pinned at
  `2026-08-07`. Row-count deltas on the raw tables matched the fixture's own DuckDB oracle
  exactly (2,388 new rows per table per day). Engine-resident state is unchanged at 19 tables
  across all three windows — no ledger/sidecar/tombstone table exists for Spark/Delta, per
  `docs/specs/state.md`, so all bookkeeping lives in `.smelt/targets/databricks/*.json`. No new
  incremental-path refusal surfaced beyond the known `gold.events_enriched` gap row 7b already
  owns. See `phases/07-summary.md` for full detail, including a discovered (but unfixed) gap:
  run reports' `completed_at`/`duration_ms` fields are never populated.
- 2026-09-12 (phase 7 plan): **reshape — row 7b inserted between rows 7 and 8; row 7's window
  schedule carries real rows rather than the BigQuery spine's empty ones.** Two findings drove
  this. (a) The `gold.events_enriched` refusal 6f recorded is gated on
  `BackendCapabilities::supports_fingerprint_sidecar`, `false` for Spark/Delta
  (`crates/smelt-dialect/src/dialect.rs:295`), and fires at *execution* time
  (`maintenance_driver/key_addressed/mod.rs:124`) rather than being downgraded at plan
  derivation. It is on the incremental path as much as the full-refresh one, so it will recur in
  every window; criteria 7 and 8 are stated over the whole model set, so clearing it is work
  serving the success criteria and gets its own row rather than a decision-log mention. Row 7
  still runs first, so one fix row can take the whole inventory of incremental-path refusals
  instead of guessing at them. (b) Read back live, `smelt_dogfood` holds event-time days 08-05
  (3,264) and 08-06 (2,714) only — but this loader replays a **local** Parquet fixture holding
  08-05 through 08-20, not a `githubarchive` scan, so landing a further day is nearly free.
  Row 7's three windows therefore each land a new fixture day (08-07, 08-08, 08-09: 2,334 /
  4,086 / 2,597 new event rows) instead of repeating the BigQuery spine's two no-new-row
  windows, and each also exercises the loader's ~2% late-arrival reach-back. No new full refresh
  is run — `20260912-124833-91da82` is the baseline and nothing under `crates/` has changed
  since. Nothing left the outcome; nothing added to `## Out of scope`.

- 2026-09-12 (phase 6f implement): **row 6f done — `LAG`/`LEAD` frame elision confirmed live;
  a new, unrelated blocker now occupies the same failure point, and the model set is closer to
  whole than at any prior phase.** The elision is decided **live**, at the point the printer
  visits a `WINDOW_FRAME` node (`crates/smelt-dialect/src/frame_elision.rs`), not planned ahead
  against the model's own `syntax` tree as the plan's literal design proposed — that design would
  not have reached `silver.actor_sessions`'s actual `LAG` calls at all, since they live inside a
  `smelt.define` function body inlined by **textual re-parse at print time**
  (`printer::reexpand_call_body`), invisible to any pre-pass walking the top-level model tree
  (confirmed via `emission_settle.rs::settled_verdict_for`'s existing range-lookup-miss fallback,
  which exists for exactly this reason). No new `PrintContext` field was added. The live re-run
  (`20260912-124833-91da82`) moved from **11 success / 1 failed / 4 skipped** to **14 success / 1
  failed / 1 skipped**: `silver.actor_sessions` and its 3 former dependents all now succeed. The
  one remaining failure is new and unrelated to window frames — `gold.events_enriched` fails with
  `Feature not supported by Spark SQL: key-addressed model-edge affected-key discovery over a
  KeyedUpsert upstream (group-grain fingerprint-sidecar diff)`, skipping `marts.star_growth` as
  its dependent. 14/16 models still complete, so the outcome's own "only fix what's needed to
  complete at all" exception does not apply — recorded in `06f-summary.md`, left for row 7's
  planner with the same "one construct clears, the next is exposed" shape as 6c → 6d → 6e → 6f.
  Also recorded, not resolved: the dialect-audit's derived `Position::Window` probes carry no
  window frame at all, so `ElideWindowFrame` is unverified by the standing cross-engine audit
  end-to-end (only by the phase's own targeted tests and this live run). Nothing left the outcome;
  nothing added to `## Out of scope`.

- 2026-09-12 (phase 6f plan): **reshape — row 6f inserted before row 7.** Phase 6e's summary
  leaves the live full refresh at 11 success / 1 failed / 4 skipped: `silver.actor_sessions`'s
  `LAG(...)` calls carry an explicit `RANGE BETWEEN INTERVAL '2 days' PRECEDING` frame, and Spark
  refuses any frame on `lag`/`lead`. Criteria 6, 7 and 8 are each stated over the whole model set,
  so four absent models are work that serves the Success criteria and cannot be deferred out; and
  running three incremental windows (row 7) over a model set that cannot full-refresh would make
  row 7's evidence untrustworthy, so 6f strictly precedes it. Elision rather than refusal or
  model-set narrowing: `LAG`/`LEAD` are offset functions the SQL standard defines to ignore the
  frame, and DuckDB agrees — measured 2026-09-12, a framed `LAG` returns a value nine days back
  through a two-day frame, identical to the unframed call — so dropping the frame on Spark cannot
  move criterion 7's parity leg. The source frames stay untouched (they are `sessionize.sql`'s
  load-bearing `max_lookback` declaration). Fourth phase in the 6c → 6d → 6e → 6f chain, each
  clearing one construct at the same failure point. Nothing left the outcome; nothing added to
  `## Out of scope`.

- 2026-09-12 (phase 6e implement): **row 6e done — `epoch_us` registered and confirmed live; a
  new, unrelated blocker now occupies the same failure point.** `EPOCH_US` landed as a normal
  `Signature` addition (`(Timestamp) -> BigInt`, registry-first inference, `unix_micros`/
  `UNIX_MICROS` templates on SparkSQL/BigQuery) — no printer branch, per the Function-registry
  single-ownership invariant. The live full refresh (run `20260912-121546-c86d9f`) moved from
  11/1/4 to 11/1/4 again but the failing model's error changed: `silver.actor_sessions`'s `LAG(...)`
  call carries an explicit window frame, which Spark refuses (`Cannot specify window frame for lag
  function`) — DuckDB's frame is legitimate there, so this is a Spark-only emission gap, not an
  `epoch_us`-adjacent issue. Recorded, not fixed, per the outcome's own boundary — same "one
  construct clears, the next construct at the same failure point is exposed" shape as 6c → 6d →
  6e. Nothing left the outcome; nothing added to `## Out of scope`.

- 2026-09-12 (phase 6e plan): **reshape — row 6e inserted before row 7.** Phase 6d's summary shows the full refresh still at 11 success / 1 failed / 4 skipped: `silver.actor_sessions` calls `epoch_us`, a DuckDB-only builtin with *no* `BuiltinRegistry` entry at all, so it reaches Databricks printed verbatim and fails `[UNRESOLVED_ROUTINE]`, skipping three dependents. Criteria 6, 7 and 8 are each stated over "the same model set" / "each model's output", so four models silently absent is work that serves the Success criteria and cannot be deferred out; the alternative 6c/6d posed (excluding `actor_sessions` and its dependents from criteria 6/7/8) would narrow the outcome and is rejected. The fix shape is a normal `Signature` addition (`EPOCH_US`, `Timestamp -> BigInt`, `Emission::Template("unix_micros({0})")` on SparkSQL), not a structural one, so it is one small phase rather than a programme. Running three incremental windows (row 7) over a model set that cannot full-refresh would also make row 7's evidence untrustworthy, so 6e strictly precedes it. Nothing left the outcome; nothing added to `## Out of scope`.

- 2026-09-12 (phase 6d implement): **row 6d done — the source-cast spelling fix confirmed live;
  a new, unrelated blocker now occupies the same failure point.** `type_conformance.rs::
  source_cast_type_sql` is the single owner shared with the cast-wrap's `type_cast_sql`; `TYPE_SPEC`
  now dispatches to it unconditionally in the printer, with trivia preserved exactly and no
  `SqlDialect`/function-name branch added to `printer/` (`emission_ownership` stays green). The
  live re-run (`20260912-110921-5ecbf6`) confirms the fix — `bronze.events` compiles
  `CAST(actor_id AS STRING)`, not `VARCHAR` — but lands on the same **11 success / 1 failed / 4
  skipped** shape as 6c, because `silver.actor_sessions` now fails on a *different* statement in
  the same model: `functions/sessionize.sql`'s `epoch_us(...)` call has no `BuiltinRegistry` entry
  at all (not merely unemitted for Spark), so it reaches Unity Catalog verbatim and fails
  `[UNRESOLVED_ROUTINE]`. A run still completes (11/16), so the "only fix what's needed to
  complete at all" exception does not apply — recorded in `06d-summary.md` rather than fixed, left
  for row 7's planner with the same two-option shape 6c's summary posed (register `epoch_us` for
  Spark, or exclude `actor_sessions` and its 3 dependents from criteria 6/7/8). Nothing left the
  outcome; nothing added to `## Out of scope`.

- 2026-09-12 (phase 6d plan): **reshape — inserted row 6d between 6c and the incremental
  windows, taking the first of the two options 6c's summary left to this planner.** 6c's live
  re-run completed 11/16 with `silver.actor_sessions` failing on `[DATATYPE_MISSING_SIZE]`
  (Spark/Databricks reject a bare `CAST(x AS VARCHAR)` cast target) and 3 dependents skipping.
  The alternative — carrying `actor_sessions` and its dependents as an exclusion through rows
  7, 8 and 9 — would make criterion 6's "the same model set", criterion 7's "each model's
  output" and criterion 8's per-model oracle each quietly 12/16, and criterion 7's wording
  ("or the difference is a registered divergence") admits no such blanket carve-out. The fix
  is also small and already half-owned: `type_conformance::type_cast_sql` *already* spells a
  bare string type as `STRING` on SparkSQL for the cast-wrap; only source-written cast targets
  bypass it, because the printer prints `TYPE_SPEC` verbatim. Routing `TYPE_SPEC` through that
  same owner closes the gap for Spark and, for free, the same class on BigQuery. Nothing left
  the outcome; nothing added to `## Out of scope`.

- 2026-09-12 (phase 6c implement): **row 6c done — drop-type-mismatch fix confirmed live; the
  three previously-failing bootstrap models now succeed, and one new, unrelated blocker
  surfaced.** `is_table_not_view_error`/`is_view_not_table_error` (`crates/smelt-backend-spark/
  src/lib.rs`) now recognise Unity Catalog's `[DROP_COMMAND_TYPE_MISMATCH]` text alongside the
  two vanilla-Spark shapes, gated by 5 new tests with no regression on the existing 31. The
  re-run full refresh landed 11 success / 1 failed / 4 skipped (16 total): `bronze.events`,
  `silver.actor_naming`, `silver.repo_naming` all now succeed. The one remaining failure is a
  new, unrelated dialect gap — `silver.actor_sessions` casts to a bare `CAST(x AS VARCHAR)`,
  which Databricks' parser rejects with `[DATATYPE_MISSING_SIZE]` since Spark requires a
  length on `VARCHAR` as a cast target (fix candidate: emit `STRING` instead of `VARCHAR` for
  an unqualified string cast on Spark/Databricks dialects) — 3 more models skip as its
  dependents. Since 11/16 models still complete, the outcome's own "only fix what's needed for
  a run to complete at all" exception does not apply, so this is recorded rather than fixed
  (`06c-summary.md`), left for row 7's planner to schedule (either as a small fix ahead of the
  incremental windows, or an explicit exclusion). Nothing left the outcome; nothing added to
  `## Out of scope`.

- 2026-09-12 (phase 6c plan): **reshape — inserted row 6c between 6b and the incremental
  windows.** Criterion 6 has two halves and only the second is rowed: the full refresh of the
  *whole* model set has still never completed. Phase 6b's re-run was 1 success / 3 failed / 12
  skipped because `SparkBackend::drop_view_if_exists` swallows only the two vanilla-OSS-Spark
  error shapes for "DROP VIEW on a TABLE", while Unity Catalog returns a third,
  `[DROP_COMMAND_TYPE_MISMATCH]`, for the identical condition — so every self-referential
  bootstrap model (`bronze.events`, `silver.actor_naming`, `silver.repo_naming`) fails on any run
  against an already-populated schema, which is every run from here on. That is dialect-invariant
  and would make criteria 6, 7 and 8 vacuous over one model, so it is the outcome's own named
  exception ("unless the fix is the only way a run completes at all") and gets a row rather than
  leaving the outcome. It is kept separate from row 7 rather than folded in because the clean
  full refresh *is* the baseline row 7's three windows are measured against, and row 7 is already
  a large live phase. `drop_table_if_exists` is fixed symmetrically in the same row since the
  same gap exists in the opposite direction. Reachability was probed before planning:
  `scripts/dbx-query.sh "SELECT current_user()"` returned the credential's own principal, so the
  live gate is open. Rows 7-11 are unchanged; nothing left the outcome; nothing added to
  `## Out of scope`.

- 2026-09-12 (phase 6b implement): **row 6b done — the hash-dialect fix and host
  reconciliation both confirmed live; one new, unrelated finding recorded rather than fixed.**
  `emit/hash.rs` now single-owns every `sha256`/`SHA256`/`sha2` spelling under
  `src/maintenance/`, gated by a new structural test
  (`hash_spelling_has_one_owner`); Spark/Databricks now emits `sha2(x, 256)`, DuckDB and
  BigQuery are byte-identical to before (pinned by
  `duckdb_and_bigquery_hash_spellings_are_unchanged`). `SMELT_DBX_HOSTNAME` (bare) is now
  exported alongside the scheme-bearing `SMELT_DBX_HOST`, and `smelt.yml`'s `host:` key reads
  the bare variable — no more shell workaround needed. The re-run full refresh confirmed the
  phase-6 root cause is gone (no failure mentions `sha256` or a hash function at all) but
  landed on the identical **1 success / 3 failed / 12 skipped** shape for a *different* reason:
  Databricks' `[DROP_COMMAND_TYPE_MISMATCH]` error text for "DROP VIEW on a table" isn't one of
  the two strings `SparkBackend::drop_view_if_exists` recognizes as "safe to swallow" (both are
  vanilla-Spark-shaped), so every self-referential bootstrap model
  (`bronze.events`/`silver.actor_naming`/`silver.repo_naming`) fails the moment its target table
  already exists from a prior run — confirmed live via `DESCRIBE EXTENDED`, which shows a real,
  valid Delta table, not a stray view. This is squarely a new finding outside 6b's own
  boundary (hash spelling + host contract), and a run still completes (1 model succeeds), so
  the "only fix what's needed to complete at all" exception does not apply — it is recorded in
  `06b-summary.md` with its exact fix candidate for the next live phase, which now needs it
  before three consecutive incremental windows are possible (every window past the first will
  re-hit this on the same three models). Nothing left the outcome; nothing added to `## Out of
  scope`.

- 2026-09-12 (phase 7 plan): **reshape — inserted row 6b between the full refresh and the
  incremental windows.** Phase 6's live full refresh completed 1 of 16 models: the maintenance
  layer's append-only baseline-snapshot fingerprint emits `sha256(...)` as literal,
  dialect-unaware text (`crates/smelt-logical/src/maintenance/emit/fingerprint.rs`), and
  Spark/Databricks has no `sha256` function — only `sha2(expr, 256)`. The cause is
  dialect-invariant, so every subsequent window fails identically and criteria 6, 7 and 8
  (three windows, dual-target parity per model, the oracle equivalence check per window) would
  all be vacuous over a single model. This is therefore not a `databricks-correctness` deferral
  but the outcome's own named exception — "unless the fix is the only way a run completes at
  all" — and it gets a phase row rather than leaving the outcome. Phase 6b also folds in the
  `SMELT_DBX_HOST` scheme mismatch phase 6 recorded and worked around in the shell, because
  three scripted consecutive windows (row 7) and the Asset Bundle (row 11) both need a
  committed, workaround-free target config. Rows 7-11 are unchanged. Deliberately NOT in 6b:
  migrating the maintenance layer's hash spelling into `BuiltinRegistry`'s
  `Signature::emission` table, which is the architectural question phase 6 raised against the
  Function-registry single-ownership invariant — that is a follow-on outcome's, recorded in
  row 10's findings handoff.

- 2026-09-12 (phase 5 implement): **row 5 done — two fixture days live in Unity Catalog, and a
  load-blocking Arrow/pandas bug was fixed rather than merely recorded.** Reachability was
  still open (`current_user()` succeeded), so the append-mode fix, `--apply-ddl`, and the 5 new
  offline tests landed first and went green before any live write. The first live
  `--date 2026-08-05` call then failed outright with `CANNOT_INFER_TYPE_FOR_FIELD` —
  `DatabricksAdapter.load_arrow_table` passed a raw `pyarrow.Table` to Databricks Connect's
  `createDataFrame`, which (pyspark 3.5.0, `pyspark.sql.connect.session`) has no
  `pyarrow.Table` overload and tried to infer a row schema instead. Since the load could not
  complete at all, the plan's own exception to "record, don't fix" applied: converted to
  `table.to_pandas()` before `createDataFrame`, added a regression test asserting the argument
  type. A second local-environment gap surfaced offline first: `duckdb_query_arrow`'s
  `COPY ... FORMAT arrow` failed because `arrow` is a community extension, not autoloaded —
  fixed with an explicit `LOAD arrow;` prefix. Both fixture days landed with exact counts
  (events/arrival 5,978 each; 3,201 / 2,777 per day; redelivered slice 63, all matching a
  DuckDB-computed oracle exactly), and the day-2 re-run was a genuine idempotent no-op read
  back from the live ledger. Also found (and fixed, not deferred): `.claude/settings.json` and
  this file's own `## Blocked` item (b) were already edited on disk, uncommitted, resolving the
  token-refresh question (`dbx-auth.sh` deny→allow, since it prints only an expiry) — a
  `dbx_dogfood_provision.rs` test still asserted the old deny state, so it was updated to match
  rather than left red or the settings reverted. Item (b)'s own remaining open question (raising
  `gpg-agent`'s cache TTL) is untouched — a headless `dbx-auth.sh` run here still failed for
  want of a TTY passphrase prompt, but this phase never needed a fresh mint. Nothing left the
  outcome; nothing added to `## Out of scope`.

- 2026-09-12 (plan 5): **no reshape; the workspace is reachable, so row 5 plans rather than
  blocks.** `scripts/dbx-query.sh "SELECT current_user()"` returned the credential's own
  principal from this worktree, so the live gate is open for now — but the one-hour OAuth
  lifetime of `## Blocked` item (b) is unresolved, so the plan makes reachability task 5's
  explicit first step with `<<PHASE_BLOCKED>>` as the only alternative to a green load. Phase
  4c's summary surfaced nothing needing a new row (its session-per-invocation latency note
  bites phases 6-9, not the loader, which already holds one session across a day's load).
  Two planning calls, both inside row 5's stated boundary: the append fix is a `mode=`
  keyword on `DatabricksAdapter.load_arrow_table` only — `spark_adapter.py` is untouched so
  the Spark parity tier stays out of the blast radius, and the Rust call site's two positional
  arguments keep their present replace semantics; and the loader gains an `--apply-ddl` mode
  because `--emit-ddl` prints DDL nobody executes, while `cmd_execute`'s ledger `INSERT`
  requires the ledger table to exist. Nothing left the outcome; nothing was added to
  `## Out of scope`.

- 2026-09-12 (phase 4c implement): **criterion 4's demonstrable half is closed.**
  `dbx-verify.sh` passed both legs on the first run against the live workspace;
  `SHOW GRANTS ON SCHEMA` for both `smelt_dogfood` and `smelt_dogfood_oracle`
  returned exactly the 4 expected privileges (`CREATE TABLE`, `MODIFY`, `SELECT`,
  `USE SCHEMA`) scoped to a single principal, confirmed via `current_user()` to be
  the credential's own identity — so the grantee bug flagged in the 4c planning
  decision was already fixed and needed no re-grant. `free-edition-facts.md` now
  has zero `TBD`s: concurrency and storage are cited from Databricks' own Free
  Edition limitations doc, cold-start latency is measured (a session-per-`dbx-query.sh`-invocation
  design means there is no cold/warm distinction to report), and session idle
  timeout is a measured partial (an `INVALID_HANDLE.SESSION_CLOSED` reproduced twice
  during `dbx-verify.sh`, suggesting single-digit-second serverless teardown) plus a
  cited fact that Databricks itself publishes no number. Two pre-existing offline
  test regressions were found and fixed along the way — a stale `CREATE TABLE`
  refusal-probe assertion that predated 4a's own switch to `CREATE SCHEMA`, and a
  loader-env test that fell through to phase 4b's real on-disk config instead of
  isolating "no credential" state — both confirmed pre-existing by reproducing
  against committed HEAD before touching either. Row 4b stays `blocked`; its
  residue (the human's own `04b-summary.md`, and Blocked item (b)'s token-refresh
  question) is untouched.

- 2026-09-12 (plan 4c): **row 4c added — the demonstrable half of criterion 4 is no longer
  human-gated.** Probing from this worktree found the workspace reachable via the allow-listed
  `scripts/dbx-query.sh` and `SHOW SCHEMAS IN workspace` returning **both** `smelt_dogfood` and
  `smelt_dogfood_oracle` — so the human's 4b run created the schemas after the 4b planning probe
  saw only `default`/`information_schema`. The grantee bug that entry flagged is also already
  fixed: `dbx-provision.sh` now grants to `$SMELT_DBX_CLIENT_ID`, not the literal
  `` `account users` ``. What criterion 4 still lacks is *demonstration* — `dbx-verify.sh` has
  never been run green, the grants have never been read back, and `free-edition-facts.md` is
  five `TBD`s — and none of that needs a browser or the account console, only a reachable
  session and the allow-listed wrappers. Leaving it inside blocked row 4b would strand work a
  success criterion names, which this process forbids, so it becomes row 4c ahead of phase 5;
  reading the credential's real scope is also a prerequisite to trusting the first live write.
  Row 4b stays `blocked` for what genuinely remains human: its own `04b-summary.md` and the
  token-refresh decision in `## Blocked` item (b), which still gates phases 5-9 and 11. No work
  left the outcome; nothing was added to `## Out of scope`.

- 2026-09-12 (plan 4b): **row 4b blocked — a human is running it live in this worktree right
  now, and a structural token-refresh question has no answer yet.** Probing the environment
  found the credential leg already done by hand: `SMELT_DBX_HOST` is set, a valid OAuth M2M
  bearer token (a JWT, so the credential kind is **service-principal OAuth M2M**, not a PAT) is
  present with ~58 minutes of life, and `scripts/dbx-query.sh` reaches the workspace
  successfully. `SHOW SCHEMAS IN workspace` returns only `default` and `information_schema`, so
  the two dogfood schemas are **not yet created** and the facts sheet is still all `TBD`. The
  worktree also carries uncommitted edits to `scripts/dbx-dogfood-env.sh` and an untracked
  `.env`; the edits fix two real bugs this planner independently found (the token file is
  `token\nexpiry`, so `$(cat)` produced a malformed two-line `SMELT_DBX_TOKEN` — now `head -n1`;
  and `${VAR:+SET}${VAR:-UNSET…}` printed the whole token when set, violating criterion 1's
  connection-security rule — now a plain `SET`/`UNSET` branch). A headless implement step must
  not race a live human session against the same workspace, so no plan was written. Reshape:
  phase 10 additionally picks up `.gitignore`-ing `.env`, which phase 4a's summary flagged and
  which is a live leak risk while the wizard's default `ENV_FILE` is `.env`.

- 2026-09-12 (plan 2, addendum): **smelt reaches the job as a locally-built wheel via the
  bundle's `artifacts:` block, not a PyPI dependency.** smelt already ships a `bindings =
  "bin"` maturin build published to PyPI as `smelt-sql` (root `pyproject.toml`,
  `.github/workflows/release.yml`), so no new packaging mechanism is needed — but dev is
  ahead of the last PyPI release, so pinning a PyPI version would run stale code. DAB's
  `artifacts:` build-and-upload step (target: workspace files, no Volume) covers exactly
  this gap: build the same wheel locally, let `bundle deploy` stage it, install it via the
  job environment's `dependencies:`. This is explicitly a placeholder — once a release
  tracks dev, drop `artifacts:` and pin `smelt-sql==<version>` from PyPI instead.

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
- 2026-09-12 (scaffold, addendum 2): **the job ships as a Databricks Asset Bundle, not a
  hand-posted job spec.** A bundle's `databricks.yml` holds the job, schedule, environment
  and Volume path in one reviewable file; `bundle validate` gives a per-PR gate that needs
  no workspace; and targets make a second workspace a config entry. The cost is a pinned
  Databricks CLI, installed through mise alongside gcloud.

- 2026-09-12 (plan 1): **the credential-free form of the target is specified in phase 1, not
  deferred to phase 11.** Phase 11 needs `token` to be optional so a task running inside the
  workspace can use the ambient session; specifying `token` as required now and relaxing it
  later would publish a rule the outcome already knows is wrong. Phase 1 therefore states
  `token` optional with ambient authentication as the alternative form; phase 11 keeps its own
  spec delta for the job asset and the Volume-resident run state. No other reshape: every
  success criterion still maps to a phase row.

- 2026-09-12 (plan 2): **no reshape.** Phase 1 shipped spec text only and surfaced nothing out of
  scope; its six named tests map one-to-one onto phase 2's test list (plus a `host` requiredness
  test and a pure session-plan test, both inside the row's stated boundary). Two implementation
  calls recorded here because they constrain later phases: the literal-`token` refusal runs as a
  **pre-interpolation** pass over the raw YAML (after `interpolate_env_vars` the origin of a
  value is unrecoverable), and the Databricks backend reuses `smelt-backend-spark` behind a
  flavor discriminator rather than getting its own crate — the SQL surface is identical and only
  the session builder and capability profile differ.

- 2026-09-12 (phase 2 implement): **`BackendCapabilities::databricks()` is `spark_delta()`
  verbatim, no field overrides.** Every flag phase 1's spec named for the Databricks column
  already equals Spark(Delta)'s own value, so the constructor is a direct delegation rather
  than a field-by-field copy. Shipped: `BackendType::Databricks`, `Target.host`/`.token` with
  redacting `Debug`/serde, `Config::validate_targets`/`check_literal_secrets`,
  `smelt-backend-spark`'s `SparkFlavor` discriminator + `plan_session` (pure, no interpreter),
  `SparkBackend::new_databricks`, `python/smelt/databricks_adapter.py`, the `databricks` cargo
  feature on `smelt-backends::create_backend`, and a cross-backend-edge refusal in
  `smelt-runtime` naming both targets. All eight named tests pass with no live workspace; see
  `phases/02-summary.md`.

- 2026-09-12 (plan 3): **no reshape.** Phase 2 surfaced nothing out of scope; its one carried
  note — that `databricks-connect`'s real Python API has never been checked against
  `python/smelt/databricks_adapter.py` — lands inside phase 3's venv script as an import
  verification step (the shape `scripts/bigquery-venv.sh` already uses), not as a new row. Two
  planning calls: the loader's per-day slice is emitted as DuckDB-executable SQL so the per-PR
  identity gate can compare it to `load_day.sh`'s own rows with no workspace and no Python
  client, and per-day idempotence is proved offline via a `--dry-run-store` ledger that runs the
  real guard with the Unity Catalog sink swapped out.

- 2026-09-12 (phase 3 implement): **the pinned `databricks-connect==15.4.5` client's Python
  API matched `python/smelt/databricks_adapter.py` (phase 2) with no changes needed** —
  `bash scripts/dbx-dogfood-venv.sh`'s import-verification step passed on the first run.
  Shipped: `scripts/dbx-dogfood-requirements.txt`, `dbx-dogfood-venv.sh`, `dbx-dogfood-env.sh`,
  `dbx-dogfood-loader.py`/`.sh`, and `crates/smelt-cli/tests/dbx_dogfood_loader.rs` (8 tests,
  all offline). One real gap surfaced and is *not* fixed here (out of this phase's boundary):
  `DatabricksAdapter.load_arrow_table` drops and recreates its target table rather than
  appending, so the loader's `--date D` execute path would overwrite rather than accumulate
  days — phase 5 (first live load) must fix this before trusting a multi-day load. See
  `phases/03-summary.md`.

- 2026-09-12 (plan 4a): **row 4 split into `4a` (offline tooling) and `4b` (the human run).**
  Phase 3's summary left phase 4 needing four scripts that do not exist yet (`dbx-key.sh`,
  `dbx-auth.sh`, `dbx-provision.sh`, `dbx-verify.sh`) plus the settings deny/allow split — all
  of which are authorable and gateable with no workspace, exactly as `bigquery-key.sh` /
  `bigquery-auth.sh` / `bq-dogfood-provision.sh` were. Only *running* them needs a human with
  a browser and an account. Splitting keeps the loop productive and shrinks the human gate to
  "run one wizard and paste two values". No work leaves the outcome: every clause of success
  criterion 4 now lands in `4a` (the mechanism) or `4b` (the demonstration). The credential
  choice (service principal + OAuth M2M vs PAT) is deliberately *not* decided in `4a` — the
  scripts are written credential-agnostic and `4b` records which Free Edition actually permits,
  because that is an empirical fact about the account, not a design call.

- 2026-09-12 (phase 4a implement): **`dbx-verify.sh` shells out through `dbx-query.sh`
  rather than reimplementing the Databricks Connect call**, so the verification path and the
  one allow-listed query path are the same code. Two real bugs surfaced and were fixed by the
  offline test suite before any live workspace existed: `dbx-key.sh --self-test`'s original
  `trap ... EXIT` referenced a function-local variable that was already out of scope by the
  time the trap fired (an "unbound variable" under `set -u`), and the wizard's own comment
  text tripped its own "never grant ALL PRIVILEGES" test. See `phases/04a-summary.md` for the
  full list, including a `.env` file that appeared mid-session with a real-looking Databricks
  host — left untouched, likely a concurrent human run of phase 4b in this shared worktree.

- 2026-09-12 (phase 6 plan): **No reshape.** Phase 5's summary surfaced nothing that serves
  the success criteria and lacks a home: the gpg-agent cache-TTL follow-up is already tracked
  under `## Blocked` item (b) as human-only, the `INVALID_HANDLE.SESSION_CLOSED` warning is a
  Free-Edition fact for `free-edition-facts.md` (phase 10's harvest), and the two
  `pyarrow`-gated loader tests' CI-hardness question is a note for the same handoff. Phase 6
  keeps its shape; the oracle target and its source-name entry stay with phase 9, which is
  the first phase that reads them.

- 2026-09-12 (phase 6 implement): **row 6 done — first full refresh run, one root-cause
  finding, two environment fixes made under the "no run completes at all" exception.** Result:
  1 success (`silver.events_deduped`, 5,915 rows), 3 failed, 12 skipped — every model whose
  driving source needs an append-only baseline snapshot over `raw.github_events`/
  `raw.github_events_arrival` fails identically, because `crates/smelt-logical/src/
  maintenance/emit/fingerprint.rs` hand-spells `sha256(...)` as literal SQL text rather than
  through the Function-Registry emission path, and Spark/Databricks has no `sha256` routine
  (only `sha2(expr, bits)`) — recorded, not fixed, as `06-summary.md` finding 1. Two fixes
  were made because nothing lived without them: `smelt-cli` had no `databricks` Cargo feature
  at all (added, mirroring `spark`'s shape), and the PyO3-embedded interpreter never runs the
  Databricks venv's `distutils-precedence.pth` shim since `PYTHONPATH`-appended directories
  skip `site` module `.pth` processing (fixed with a 6-line `_distutils_hack.add_shim()` in
  `python/smelt/databricks_adapter.py`, verified against a standalone repro first). A third
  issue — `scripts/dbx-key.sh` stores the host WITH its `https://` scheme, but the target's
  `host:` field requires bare — was worked around only in the shell for this phase's live
  calls, with no committed script or config touched; left as an open reconciliation for
  phase 10 or later. Declaring the target also reopened `MaintenanceStateDowngraded` (the
  same four cells BigQuery itself triggered before its own ledger support landed) and broke
  three offline test files because env-var interpolation is target-blind by spec — both fixed
  as test-only changes following the BigQuery phase 11 precedent exactly (an expected-messages
  list restored to use, and dummy `SMELT_DBX_HOST`/`SMELT_DBX_TOKEN` values added at four
  `smelt`-spawning call sites). Nothing left the outcome; nothing added to `## Out of scope`.

## Blocked

- **2026-09-13 — phase 9d (replay under 9c's fix, live). RESOLVED by phases 9e/9f — 9e's
  ledgerless rebuild closed the gap offline, and 9f's replay confirms it live (clean 16/16
  full refresh, `silver_actor_naming` exact at all three windows, `dbx_only` 520 → 0). Original
  text kept for context.** A second live-only gap in 9c's fix, never reachable by 9c's own
  offline tests. The reset stage, the loader/DDL sequence and the
  8-day fixture reload (days 2026-08-05 through 2026-08-12, `workspace.smelt_dogfood.
  github_events` at 26,220 rows) all completed cleanly. `smelt run --target databricks
  --full-refresh --allow-full-refresh --event-time-start 2026-08-05 --event-time-end
  2026-08-13` then failed on both succession-grain models (`silver.actor_naming`,
  `silver.repo_naming`) with `Feature not supported by Spark SQL: succession-patch technique
  (window-forward driver)`.

  **Root cause.** 9c's dispatch fix (`crates/smelt-runtime/src/execute/project/mod.rs:2557-2561`)
  routes a `cell.state_downgraded` succession cell to `rebuild_succession_state` instead of the
  window-forward loop — but `rebuild_succession_state` itself
  (`crates/smelt-runtime/src/maintenance_driver/succession/execute.rs:339`) carries the exact
  same unconditional `if !realises_tombstone_ledger(backend.dialect()) { bail!(...) }` guard as
  `execute_succession_maintenance`. So the routing change never actually reaches Databricks: it
  just moves the same refusal from one function to the other. 9c's own verification — the
  differential test (`explain_maintenance/databricks_succession_differential.rs`) and the
  resolver-level regression test (`state_downgraded_cell_still_dispatches_marked_for_full_rebuild`)
  — only assert the *dispatch decision* (a downgraded cell resolves live and is marked for full
  rebuild); neither calls `rebuild_succession_state` or `execute_succession_maintenance` against
  a real backend, so this gap was invisible offline. 9c's own "For the next planner" note flagged
  this class of risk generically ("re-run ... to get fresh, correct numbers under the fix") but
  did not anticipate the fix itself would still refuse.

  **Why this is not this phase's own fix.** `rebuild_succession_state` builds and reads a
  tombstone table (`tombstone_table_ddl`, the clock-tie probe, `emit_succession_full_rebuild`'s
  own signature) as part of its normal operation — it was written for the case where a ledger
  IS realisable (DuckDB's own full-refresh-of-an-existing-ledger path) and never adapted for a
  backend where `StateStructure::TombstoneLedger` has no realisation at all. Making it work for
  that case needs a real design answer (does a downgraded full rebuild skip the tombstone table
  and clock-tie probe entirely, given `docs/specs/state.md`'s new §"The degradation contract"
  sentence that the recompute region is the whole presented table and there is no ledger to
  probe against? or does it need a ledger-free variant of `emit_succession_full_rebuild`?) —
  not a boolean-check fix like 9c's, and not something to improvise mid-replay.

  **State left behind.** The Databricks workspace (`workspace.smelt_dogfood`) holds 8 days of
  loaded source data (2026-08-05 through 2026-08-12, `github_events` at 26,220 rows,
  `github_events_arrival` populated identically) and NO model tables — the full refresh failed
  before writing any model, including the 14 non-succession models (the run aborts the whole
  batch on any model failure). `workspace.smelt_dogfood_oracle` is empty (post-reset, untouched).
  Left in this state deliberately: the next fix attempt can re-run the full refresh directly
  from task 4 of `phases/09d-plan.md` without repeating the reset/reload. No `.smelt/targets/`
  local state exists for `databricks`/`databricks_oracle` (cleared by the reset stage, never
  recreated since no run succeeded).

  **Candidate fix routes for the next planner:**
  1. Give `rebuild_succession_state` a `cell.state_downgraded`-gated branch that skips the
     tombstone table and clock-tie probe entirely and emits a bare `CREATE OR REPLACE TABLE …
     AS SELECT` over `emit_succession_full_rebuild`'s `ROW_NUMBER() … = 1` shape restricted to
     non-delete-flagged rows — no ledger read or write at all, matching "the recompute region is
     the whole presented table" literally.
  2. Same target shape, but reached by a distinct new function
     (`rebuild_succession_state_ledgerless` or similar) rather than a branch inside the existing
     one, if the ledger-bearing and ledger-free bodies diverge enough that a shared function
     becomes an `if` maze — matching the single-owned-emitter discipline `CLAUDE.md` §
     "Maintenance-plan purity" asks for.
  3. Add a live execution leg to the offline differential test (or a new one) that exercises
     `rebuild_succession_state`/`execute_succession_maintenance` against SOME dialect gate
     (even a synthetic no-ledger dialect on DuckDB, if constructible) so this class of gap is
     caught next time without needing a live Databricks run at all.
  Whichever route is chosen: re-run task 4 onward of `phases/09d-plan.md` (the full refresh, then
  windows 9-11, then both sweeps) once the fix lands — the offline scaffolding (the `reset`
  stage, the checkpoint-lockstep tests, the `state.md` spec delta) is already committed and does
  not need repeating.

- **2026-09-13 — phase 9b (equivalence oracle, live). RESOLVED by phase 9f — the re-measured
  sweep under 9e's fix is clean (`silver_actor_naming` exact at all three checkpoints, as it
  already was here; only `gold_events_enriched`'s already-registered divergence remains) and
  the two deferred report gates now land. Original text kept for context.** Three checkpoints (w09/w10/w11,
  `2026-08-13/14/15`) measured; the sweep is not clean, per the plan's own contingency
  (commit evidence, land tests 2-4 only, block, hand to 9c). Two relations violate:
  1. **`gold_events_enriched`** — `9/9`, `10/10`, `16/16` (`incr_only`/`oracle_only`) at
     w09/w10/w11. This is the SAME divergence phase 8 root-caused and registered
     (`DivergenceBound::UnorderedColumnDivergence` on `current_repo_name`) in the
     dual-target suite — not new, not investigated further. Left for 9c or a later phase to
     decide whether to also register it in `github_activity_dbx_oracle.rs`'s own
     `EQUIVALENCE_DIVERGENCE_REGISTRY` (it is understood and bounded, so the high bar that
     registry's doc comment sets is arguably met — but the plan's contingency scoped this
     phase to evidence-gathering only, so the call was left open rather than made here).
  2. **`silver_actor_naming` matches exactly at all three checkpoints** (`incr_only=0,
     oracle_only=0`) — this is NOT a violation, and it is the decisive evidence phase 8's own
     entry (below) asked for: Databricks' full-refresh oracle does not reproduce the 520-row
     duplication the incremental leg shows against DuckDB, so the defect is confined to the
     incremental write path (succession-patch tombstone mechanism in
     `crates/smelt-runtime/src/maintenance_driver/succession/execute.rs`), not shared with a
     full refresh on the same engine. This resolves phase 8's open question in favour of
     `## Blocked`'s options 1 or 3 below (a write-path bug), ruling out option 2 (an
     accepted, both-legs-see-it duplication).
  See `phases/09b-summary.md` for the full measured table and two script bugs fixed along
  the way (`scripts/dbx-dogfood-oracle.sh`'s loader flag name and a missing
  `--allow-full-refresh` license, both needed for the sequence to run at all).

- **2026-09-13 — phase 8 (dual-target parity). RESOLVED by phase 9c — see the 9c decision-log
  entry below for the root cause and fix; route 1 (a re-read of `succession/execute.rs`'s
  MERGE/patch statement) turned out to name the wrong mechanism, route 3 (fix the write path)
  is what landed, at a smaller scope than either candidate anticipated. Live-re-measured and
  closed by phase 9f: the re-run `08-parity.json` shows `silver_actor_naming` at `dbx_only=0`
  (was 520), and `the_committed_parity_report_shows_no_unregistered_difference` /
  `dbx_registry_entries_are_all_live` are now hard gates.** Original text kept
  for context: `silver_actor_naming` has 520 more rows
  on Databricks than DuckDB after the live sweep (`duckdb_only=0, databricks_only=520`;
  Databricks holds 26,177 `DISTINCT` rows against DuckDB's 25,700 over the identical
  source, with individual `(actor_id, created_at)` pairs duplicated up to 7x on
  Databricks). `silver.actor_naming` is a succession-grain model
  (`models/silver/actor_naming.sql`) maintained by the succession-patch technique's
  tombstone mechanism (`crates/smelt-runtime/src/maintenance_driver/succession/
  execute.rs`); DuckDB's write path collapses a redelivered duplicate event onto the same
  succession row, Databricks' apparently does not. Root cause not yet confirmed past this
  point (see the phase 8 implement decision-log entry above for the full measurement).
  Candidate options for the next planner:
  1. **Read `succession/execute.rs`'s MERGE/patch statement** to confirm whether it relies
     on a ledger-backed idempotence check this repo's own 2026-09-13 research note
     (Catalog Commits / cross-table transactions on Delta) already flags as possibly wrong
     for Databricks specifically — if so, this may be the SAME root cause as that note
     rather than a second one, and the two should be triaged together.
  2. **Add a distinct-set comparator bound** to `parity_support` (e.g. `DivergenceBound::
     DuplicateTolerant` — the two sides' `SELECT DISTINCT` row sets must match exactly,
     only cardinality may differ, and only in the direction of the licensed side never
     having FEWER distinct rows) if the duplication turns out to be a genuinely accepted,
     if ugly, behaviour rather than a bug worth fixing.
  3. **Fix the write path** if root-caused to a bounded, real defect (matching this
     outcome's exception for "the only way a run completes at all" would not apply here,
     since the run does complete — this would need to be justified as small enough to not
     belong to the deferred `databricks-correctness` outcome, or explicitly folded into
     that outcome instead).
  Whichever route is chosen, `crates/smelt-cli/tests/github_activity_dual_target.rs`'s
  Databricks sweep, `scripts/dbx-dogfood-parity.sh`, and `scripts/dbx_dogfood_export.py`
  are already built, tested and committed (phase 8) — the remaining work is re-running the
  `dbx-snapshot`/`manifest`/live-test sequence once `silver_actor_naming` is resolved, then
  restoring `08-parity.json` and its liveness-ratchet test (removed this phase to keep
  `cargo test` green — see `parity_support`'s and `github_activity_dual_target.rs`'s
  module doc comments for exactly what was deferred).

- **2026-09-12 — phase 4b (provisioning run).** Two things a human must settle.

  **(a) Finish the wizard.** The credential leg is done (OAuth M2M service principal,
  reachable). What remains of criterion 4: create `workspace.smelt_dogfood` and
  `workspace.smelt_dogfood_oracle`, apply the schema-scoped grants (`dbx-provision.sh`'s
  grantee is currently the untested literal `` `account users` `` — adjust to the service
  principal's actual application ID), run `bash scripts/dbx-verify.sh` so both the
  reachability and the inverted out-of-scope-write-refusal legs pass, and fill in
  `free-edition-facts.md` (four quota rows plus the credential-kind line). Then commit the
  in-flight `scripts/dbx-dogfood-env.sh` fix, write `phases/04b-summary.md`, and flip this row
  to `done`.

  **(b) RESOLVED 2026-09-12 — how a headless step gets a live token.** OAuth M2M tokens
  last one hour, and `scripts/dbx-auth.sh` (the only refresher) was in `permissions.deny` and
  needs a gpg passphrase, so a loop iteration couldn't mint one and any live phase longer than
  the human's last manual refresh died mid-run. Of the three candidates weighed (allow-list
  `dbx-auth.sh` directly and rely on the `gpg-agent` cache; move the refresh into the
  already-allowed `dbx-query.sh`/`dbx-dogfood-loader.sh` wrappers; keep every live phase
  human-attended), the human chose **option (i)**: `dbx-auth.sh` moved from `permissions.deny`
  to `permissions.allow` in `.claude/settings.json` (it only ever prints an expiry, never the
  token). This depends on `gpg-agent`'s passphrase cache outliving the ~1-hour gap between
  refreshes, which the stock default (`default-cache-ttl` 600s / `max-cache-ttl` 7200s) does
  not — raising both (e.g. to 43200s in `~/.gnupg/gpg-agent.conf`) is a machine-level,
  security-relevant change the auto-mode classifier correctly refused to make unattended, so
  it is still open and tracked as a follow-up on the human, not on this outcome's loop. Once
  set, the human primes the cache once per gap (or once per `max-cache-ttl` window) and
  headless phases mint their own refresh via the now-allowed `dbx-auth.sh`. This unblocks
  phases 5–9 and 11 for the loop, provided the cache TTL is actually raised before a live run
  longer than ~1 hour is attempted unattended.

- **2026-09-13 — phase 11c (deploy and prove three scheduled runs, live). Human-gated: needs a
  Unity Catalog grant.** Tasks 1–4 of the plan landed and are green: the `schedule_cron`
  bundle variable plus explicit `pause_status: UNPAUSED` (tests
  `bundle_declares_one_daily_scheduled_serverless_job` amended,
  `bundle_schedule_is_explicitly_unpaused` new), the `volume_probe` job resource and
  `dbx_job/volume_probe.py` (flock advisory, `os.replace()` rename atomicity, `fsync` probes;
  test `bundle_volume_probe_targets_the_declared_volume` new), `.smelt-dbx-venv` rebuilt so
  `duckdb` is present, and `scripts/dbx-verify.sh` green against a freshly reachable workspace
  (the encrypted token had expired since phase 9f; `dbx-auth.sh` reminted it). All 12
  `databricks_bundle` tests plus `databricks bundle validate` are green.

  **What stopped task 5.** `scripts/dbx-bundle.sh deploy` failed two ways, the second of which
  is the real blocker:
  1. A genuine bug, fixed in this phase: the wrapper's `deploy`/`run`/`seed` branches exported
     only `DATABRICKS_HOST`, never `DATABRICKS_TOKEN`, so every live subcommand failed
     unified-auth resolution outright (there is no `~/.databrickscfg` in this environment —
     credentials are ambient env vars only). Fixed by also exporting
     `DATABRICKS_TOKEN="${SMELT_DBX_TOKEN}"` on every `deploy`/`run`/`seed` invocation, plus an
     explicit `SMELT_DBX_TOKEN` presence check alongside the existing `SMELT_DBX_HOST` one.
  2. A separate local-only fix, also landed: `pyproject.toml`'s `data = "smelt_sql.data"`
     requires that directory to exist for `maturin build` (it's gitignored — CI and
     `tests/agent-loop/harness/build_local_wheel.sh` both `mkdir -p` it locally); this worktree
     never had it. Created `smelt_sql.data/scripts/` (untracked, matches `.gitignore`).
  3. **The actual blocker.** With both of the above fixed, `deploy` built the wheel, uploaded
     bundle files, created both job resources (`github_activity_daily`,
     `github_activity_volume_probe` — confirmed present via `databricks jobs list`), then failed
     on the Volume resource: `cannot create resources.volumes.smelt_dogfood_project: User does
     not have CREATE VOLUME on Schema 'workspace.smelt_dogfood'. (403 PERMISSION_DENIED)`.
     Phase 4b/4c's grant (`scripts/dbx-provision.sh`) predates 11b's Volume resource and only
     ever granted `USE SCHEMA, CREATE TABLE, SELECT, MODIFY` — `CREATE VOLUME` was never in
     scope because nothing needed it yet.

  **Fix authored, not applied.** `scripts/dbx-provision.sh`'s oauth-m2m grant statement now
  also grants `CREATE VOLUME` on `${CATALOG}.${SCHEMA}` (i.e. `workspace.smelt_dogfood` only —
  the oracle schema has no Volume and keeps its narrower grant). This is a schema-owner-only
  operation the scoped service-principal credential cannot self-grant (by design — the same
  reason `dbx-query.sh` is deliberately read-only and never used to attempt this). Unblocking
  needs a human, via one of:
  1. Re-run `bash scripts/dbx-provision.sh` (idempotent — re-issues `CREATE SCHEMA IF NOT
     EXISTS` plus the now-widened `GRANT`) as whichever workspace identity owns/administers
     `workspace.smelt_dogfood` (this is what phase 4b already did once; it just predates the
     Volume requirement).
  2. Or run the single missing grant by hand: `GRANT CREATE VOLUME ON SCHEMA
     workspace.smelt_dogfood TO `<the oauth-m2m client id in
     ~/.config/databricks-smelt-dogfood/config.env>`;`.

  **State left in the workspace.** Two job resources exist
  (`github_activity_daily` id `459361340805633`, `github_activity_volume_probe` id
  `42210763587936`) with the daily job's schedule at the committed default
  (`0 0 6 * * ?`, `UNPAUSED`) — harmless (it will simply fail its `smelt_run` task once
  triggered, since the Volume it needs doesn't exist yet) and will be reconciled by the next
  successful `deploy` once the grant lands. No Volume, no seed, no probe run, no scheduled-run
  evidence — none of tasks 5–13 beyond the deploy attempt were reachable.

  **Candidate options for resuming 11c**, in the order they should be tried: (a) human runs one
  of the two grant routes above, then the loop resumes 11c from task 5 (`deploy`); (b) if Volume
  creation turns out to need a broader Unity Catalog admin capability than a single `GRANT`
  (unconfirmed — untested against this workspace), the human instead pre-creates the Volume by
  hand and the bundle deploy step may need `databricks bundle deployment bind` to adopt it
  rather than create it, which would be new work for whichever phase resumes this row.
