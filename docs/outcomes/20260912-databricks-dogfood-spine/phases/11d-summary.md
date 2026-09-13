# Phase 11d summary — ambient form means no explicit host either (offline)

**Shipped:**
- `Config::validate_targets` (`crates/smelt-core/src/config.rs`): `host` absent is only an
  error when `token` is present (names both keys); both absent is the ambient form and loads
  cleanly.
- `SessionArgs::Databricks.host` is now `Option<String>`; `plan_session` and
  `SparkBackend::new_databricks` (`smelt-backend-spark`) pass an absent host straight through
  with no `""` default and log `<ambient>` in its place.
- `smelt-backends::create_backend` drops the `"Databricks target requires 'host' field"` bail
  — the config validator owns that rule now — and passes `Option<&str>` through.
- `python/smelt/databricks_adapter.py`: `__init__(self, host=None, catalog=None, token=None)`;
  `.host(...)` is called on the builder only when `host` is truthy.
- `scripts/dbx-dogfood-loader.py`: the three duplicated `SMELT_DBX_HOST`-or-exit blocks
  collapsed into one `_connect()` helper; both `SMELT_DBX_HOST` and `SMELT_DBX_TOKEN` are
  optional.
- `examples/github_activity/smelt.yml`'s `databricks_job` target drops `host:
  ${DATABRICKS_HOST}` entirely — the comment now states the measured fact (no host-bearing env
  var on a Free Edition serverless job task, only a live `SPARK_REMOTE` Connect channel).
- Spec delta landed: `docs/specs/multi_backend.md` §"Connection security",
  `docs/specs/smelt_yml.md` §"Target shape", `docs-site/docs/guide/targets.md` (host table row
  + a two-keys-absent example in the deployment subsection).
- Tests: `smelt-core` `databricks_ambient_target_omits_host_and_token`,
  `databricks_token_without_host_is_refused`, retargeted `databricks_target_requires_host`;
  `smelt-backend-spark` `plan_session_databricks_ambient_carries_no_host`; `smelt-cli`
  `dbx_dogfood_loader.rs::adapter_omits_host_builder_call_when_ambient` (drives the real
  adapter `__init__` against a stubbed `databricks.connect` builder),
  `loader_runs_ambiently_with_no_host_env`; `databricks_bundle.rs::job_target_is_ambient`.

**Decisions:**
- The host-var check in `job_target_is_ambient` excludes `databricks.yml` — that file
  legitimately documents `DATABRICKS_HOST` as the Databricks *CLI's* own workspace-auth
  mechanism for `bundle validate`/`deploy`/`run`, which is the deployer's credential, not the
  deployed job task's runtime environment. Only the job resource and its task scripts are
  checked.
- `adapter_omits_host_builder_call_when_ambient` reuses the existing `dbx_venv_python()` gate
  (skips with a printed reason when `.smelt-dbx-venv` isn't built) since the real
  `databricks_adapter.py` imports `pyarrow` at module scope; it ran for real in this session.

**For the next planner:** nothing left undone in this phase's scope. Phase 11e is next: resume
11c's live legs (compressed-cadence redeploy, three consecutive scheduled runs, run reports,
oracle comparison, quota recording, `volume_probe` write-up, cadence restore) under this fix.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full `cargo test`, `example_diagnostics`). One round of fixes needed: `cargo fmt --all`, and
  `.claude/scripts/large-file-check.sh --update` for `crates/smelt-core/src/config.rs`'s
  legitimate growth (validation logic + new tests).
- `cargo test -p smelt-cli --test dbx_dogfood_loader --test databricks_bundle --features duckdb`
  — 34/34 passed.
- `cargo test -p smelt-core --lib config` / `--lib databricks` — all passed.
- `bash scripts/dbx-bundle.sh validate` (with the Databricks CLI on `PATH`) — `Validation OK!`.
