# Phase 11d plan — the ambient form means no explicit host either (offline)

## Objective

Make a `databricks` target with **both** `host` and `token` absent build its session from the
workload's own ambient workspace context, calling no `.host(...)` on the Databricks Connect
builder at all — the shape a serverless Job task actually presents (11c measured: no
`DATABRICKS_HOST`, but a live `SPARK_REMOTE` Connect channel). This unblocks criterion 11's
scheduled runs, whose `load_next_day` task currently dies on a hard `SMELT_DBX_HOST` requirement
before any smelt code runs, and corrects criterion 1's spec claim to a measured fact. Offline
only — no workspace, no credential.

## Spec delta (first)

- `docs/specs/multi_backend.md` §"Connection security" — rewrite the "second, credential-free
  form" paragraph. Replace the claim that "a Databricks job automatically exports
  `DATABRICKS_HOST` into the task's runtime environment" (measured false on Free Edition
  serverless job tasks, 11c) with: the ambient form omits `host` as well as `token`, and the
  session is built with no explicit host or token, honouring whatever workspace context the
  runtime already established. Keep the redaction rule's vacuity note.
- `docs/specs/smelt_yml.md` §"Target shape" — `host` becomes *required unless the target is in
  ambient form* (`token` also absent); `token` present with `host` absent is a hard
  configuration error naming both keys, since a token carries no workspace address.
- `docs-site/docs/guide/targets.md` — the Databricks target's ambient/deployment subsection
  gains the two-keys-absent form (one short example: a job target with only `type`, `catalog`,
  `schema`).

## Tests (red first)

1. `smelt-core` `config::tests::databricks_ambient_target_omits_host_and_token` — a
   `type: databricks` target with neither key passes `validate_targets`.
2. `smelt-core` `config::tests::databricks_token_without_host_is_refused` — `token` present,
   `host` absent → error naming both keys.
3. `smelt-core` `config::tests::databricks_target_requires_host` (existing) — retargeted: a
   target with `token` present and `host` absent still errors; the bare-hostname rule is
   unchanged when `host` *is* present.
4. `smelt-backend-spark` `session::tests::plan_session_databricks_ambient_carries_no_host` —
   `plan_session(Databricks, None, None, None)` yields `SessionArgs::Databricks { host: None,
   token: None }`, and the host-present case is unchanged.
5. `smelt-cli` `tests/dbx_dogfood_loader.rs::adapter_omits_host_builder_call_when_ambient` —
   drive `DatabricksAdapter` against a stub `databricks.connect` module (the existing
   `run_python` harness) with `host=None`: the recorded builder call list contains `serverless`
   and no `host`; with a host it contains `host`.
6. `smelt-cli` `tests/dbx_dogfood_loader.rs::loader_runs_ambiently_with_no_host_env` — via
   `run_loader_with_fake_adapter` with `SMELT_DBX_HOST`/`SMELT_DBX_TOKEN` unset: the loader
   completes and its `init` log records `host=None`, rather than exiting with
   "SMELT_DBX_HOST is not set".
7. `smelt-cli` `tests/databricks_bundle.rs::job_target_is_ambient` — the `databricks_job` target
   in `examples/github_activity/smelt.yml` declares neither `host` nor `token`, and the bundle's
   job environments export no `SMELT_DBX_HOST`/`DATABRICKS_HOST`.

## Tasks

1. Land the spec delta above (multi_backend.md, smelt_yml.md, docs-site targets page).
2. `crates/smelt-core/src/config.rs` `validate_targets`: `host` absent is an error only when
   `token` is present (message names both keys); both absent is the ambient form and passes.
   Keep the bare-hostname check on the `Some(host)` arm, and the foreign-key refusals as is.
3. `crates/smelt-backend-spark/src/session.rs`: `SessionArgs::Databricks.host` becomes
   `Option<String>`; `plan_session` stops defaulting a missing host to `""`.
4. `crates/smelt-backend-spark/src/lib.rs` `new_databricks(host: Option<&str>, ...)`: pass the
   optional host through to the adapter call and log `host=<ambient>` when absent (never a
   token).
5. `crates/smelt-backends/src/lib.rs`: drop the `Databricks target requires 'host' field`
   bail — the config validator now owns that rule — and pass `target_config.host.as_deref()`
   through; the connect-failure message tolerates an absent host.
6. `python/smelt/databricks_adapter.py`: `__init__(self, host=None, catalog=None, token=None)`;
   apply `.host(host)` only when host is truthy, `.token(token)` only when token is truthy;
   document that the host-less form relies on the runtime's own workspace context.
7. `scripts/dbx-dogfood-loader.py`: replace the three duplicated
   `os.environ["SMELT_DBX_HOST"]`-or-exit blocks with one `_connect()` helper that treats both
   `SMELT_DBX_HOST` and `SMELT_DBX_TOKEN` as optional and constructs the adapter once.
8. `examples/github_activity/smelt.yml`: the `databricks_job` target drops `host:
   ${DATABRICKS_HOST}` (comment updated to the measured ambient fact). Drop any
   `DATABRICKS_HOST`/`SMELT_DBX_HOST` plumbing the bundle's job environments carry for it
   (`examples/github_activity/resources/github_activity_job.yml`).

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, shellcheck, full
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test dbx_dogfood_loader --test databricks_bundle`.
- `cargo test -p smelt-core --lib config`.
- `bash scripts/dbx-bundle.sh validate` (config-only; no workspace).

## Commit message

`feat(databricks): ambient target form builds its session with no explicit host`
