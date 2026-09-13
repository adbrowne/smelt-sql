# Phase 11a summary — the Asset Bundle and its tooling, offline

## Shipped

- `mise run setup-databricks` (`scripts/mise-setup-databricks.sh` +
  `scripts/mise-databricks-bin-dir.sh`, mirroring the gcloud pair) pins the Databricks CLI at
  **v1.16.1** into `~/.local/databricks-cli`; `mise.toml`'s `_.path` is now an array combining
  the gcloud and Databricks resolvers.
- `examples/github_activity/databricks.yml` + `resources/github_activity_job.yml`: one job,
  daily cron (`0 0 6 * * ?`), two `spark_python_task`s (`load_next_day` → `smelt_run`, ordered by
  `depends_on`), one serverless environment per task, an `artifacts.smelt_wheel` built by
  `maturin build --release --out dist` from the repo-root `pyproject.toml`
  (`bindings = "bin"`), and `sync.paths` pulling `../../scripts` and `../../python` in alongside
  the bundle root so the job-native wrappers can reach the existing loader and
  `python/smelt/databricks_adapter.py`.
- `examples/github_activity/dbx_job/load_next_day.py` and `run_smelt.py`: thin
  `spark_python_task` wrappers. The loader wrapper shells out to the already-tested
  `scripts/dbx-dogfood-loader.py` (reuse, not a second loader); the run wrapper shells to
  `smelt run --project-dir <Volume path> --target databricks_job`.
- `examples/github_activity/smelt.yml` gained `databricks_job`: the ambient-credential
  deployment form — `host: ${DATABRICKS_HOST}`, no `token:` key at all.
- `scripts/dbx-bundle.sh`, the only caller of `bundle validate`/`deploy`/`run`, plus its two
  `.claude/settings.json` allow-list entries.
- `scripts/dbx_bundle_validate_stub.py`: a minimal stateful HTTP stub of the three endpoints
  `bundle validate` unconditionally calls (SCIM `Me`, `workspace/get-status`,
  `workspace/mkdirs`) — see Decisions.
- `crates/smelt-cli/tests/databricks_bundle.rs` (7 tests, all green, no workspace needed) and an
  extension to `crates/smelt-core/tests/databricks_docs_freshness.rs`.
- Spec delta: `docs/specs/multi_backend.md` §"Connection security" gained the ambient
  in-workspace deployment-form paragraph. Docs-site: `docs-site/docs/guide/targets.md` gained a
  `#### Deployment: Databricks Asset Bundle` subsection. `CLAUDE.md`'s setup block gained the
  `mise run setup-databricks` line.

## Decisions

- **`databricks bundle validate` is not actually workspace-free — measured against CLI
  v1.16.1.** The plan assumed it was a pure schema check. In fact the CLI's
  `PopulateCurrentUser` mutator calls SCIM `Me` on every bundle command regardless of what the
  config references, and root-path bootstrap calls `workspace/get-status` then
  `workspace/mkdirs`. A completely credential-less environment fails before schema is even
  checked ("cannot configure default credentials"). Fix: `scripts/dbx_bundle_validate_stub.py`,
  a stateful loopback HTTP stub answering exactly those three endpoints, which
  `scripts/dbx-bundle.sh validate` launches on an ephemeral port and tears down on exit — genuine
  "no real workspace", not "no auth at all". `bundle_declares_one_daily_scheduled_serverless_job`
  and the CLAUDE.md/outcome-facing prose were adjusted to describe this precisely.
- **`workspace.host` cannot be templated.** The CLI hard-refuses variable interpolation on that
  field (an authentication field) and directs the caller to `DATABRICKS_HOST` instead. Design
  changed from "a bundle variable holds the host" to "the host is `DATABRICKS_HOST`, resolved by
  `scripts/dbx-bundle.sh` from `scripts/dbx-dogfood-env.sh`'s `SMELT_DBX_HOST` for real
  deploy/run, and by the stub for `validate`" — no real hostname is ever committed. The
  structural test (`bundle_targets_name_the_dogfood_workspace_as_a_target_entry`) asserts the
  negative (no literal `cloud.databricks.com` in the file) rather than the plan's literal
  wording ("host comes from a bundle variable").
- **`root_path` is left at its per-user default**, not pinned to a literal. A literal under
  `/Shared` (the first attempt) draws a CLI warning that the path is writable by every workspace
  user; since identity resolution happens unconditionally anyway (previous point), pinning
  bought nothing and traded a scoped default for a broader one.
- **`databricks_job`'s `${DATABRICKS_HOST}` reference breaks `Config::load` everywhere the var
  isn't set**, because `smelt_yml.md`'s interpolation pass is whole-file, not per-target (every
  `${VAR}` in the committed `smelt.yml` must resolve, whether or not that target is selected).
  Six existing test call sites across `github_activity_databricks.rs`, `github_activity_replay.rs`,
  `list_external_step.rs`, `github_activity_support/mod.rs`, and
  `explain_maintenance/databricks_succession_differential.rs` already stub `SMELT_DBX_HOSTNAME`/
  `SMELT_DBX_TOKEN` for exactly this reason (the existing `databricks`/`databricks_oracle`
  targets); each gained a `DATABRICKS_HOST` stub alongside. Found by running the full
  `cargo test --workspace --no-fail-fast` rather than trusting the first failure `verify-phase.sh`
  reported (it fails fast, so later binaries' failures were invisible until re-run).

## For the next planner (11b)

- **Exact deploy command**: `scripts/dbx-bundle.sh deploy` (wraps `databricks bundle deploy
  --target dogfood`, `DATABRICKS_HOST` from `scripts/dbx-dogfood-env.sh`'s `SMELT_DBX_HOST`).
  Run: `scripts/dbx-bundle.sh run github_activity_daily`.
- **Volume path**: the `smelt_run` task's first parameter is
  `/Volumes/${var.catalog}/${var.schema}/${var.volume_name}/project` (defaults resolve to
  `/Volumes/workspace/smelt_dogfood/smelt_project/project`). **Nothing provisions this Volume or
  seeds it with the project files yet** — 11b must create the Volume, copy
  `examples/github_activity`'s `smelt.yml`/`models/` onto it once (or as part of first deploy),
  and confirm `.smelt/` writes there persist across runs.
- **Open live question carried from the plan**: whether the Volume's FUSE layer supports
  `.smelt/lock` advisory locking and rename-atomic writes — unmeasured, 11b's job.
- **New open question this phase surfaced**: the daily schedule's `{{job.trigger.time.iso_date}}`
  dynamic value reference feeds `load_next_day.py --date <today's real calendar date>`, but the
  fixture's data lives on a fixed historical date range (`2026-08-05` onward per prior phases'
  live runs). The first several/many scheduled runs will ask the loader for a day the fixture
  does not have. This wasn't in 11a's scope to resolve (no task named it), but 11b's "three
  consecutive scheduled runs complete" criterion cannot be met until either the loader computes
  "next unloaded day" from its own ledger table instead of trusting the trigger date, or the
  schedule/date mapping is otherwise reconciled. Flagging now so 11b's planner sizes it in.
- **`loader_env`'s dependency list is a best-effort port of
  `scripts/dbx-dogfood-requirements.txt`** (`databricks-connect==15.4.5`, `pyarrow`) — not
  measured against a live serverless Python environment. The loader's `duckdb_query_arrow` also
  shells out to the `duckdb` CLI binary via `subprocess`, which is very unlikely to be preinstalled
  in a Databricks serverless Python environment; 11b will need to add it as a dependency (or
  reimplement that one function against a Python duckdb/pyarrow path) before the loader task can
  actually run.

## Gates

- `bash .claude/scripts/verify-phase.sh` — **ALL GREEN** (fmt, clippy both feature sets,
  shellcheck, full `cargo test --workspace`, `example_diagnostics`).
- `cargo test -p smelt-cli --test databricks_bundle` — 7/7.
- `cargo test -p smelt-core --test databricks_docs_freshness` — 3/3.
- `bash scripts/dbx-bundle.sh validate` from a clean shell (`env -i`, no credentials) — exit 0.
- `cargo test -p smelt-cli --test github_activity_dual_target` — 26/26, unchanged.
- `cargo test --workspace --no-fail-fast` — 426/426 test binaries green.
