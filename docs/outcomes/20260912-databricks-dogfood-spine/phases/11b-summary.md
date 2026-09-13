# Phase 11b summary — scheduled loader made self-driving and serverless-safe

## Shipped

- `scripts/dbx-dogfood-loader.py`: new `--next-day` mode. `fixture_days()` enumerates the
  fixture's distinct days via one DuckDB query; `next_unloaded_day()` subtracts whichever ledger
  applies (`_loader_days` table live, the dry-run store's ledger file offline) and returns the
  earliest remainder, or `None` once exhausted — the exhausted case prints a message and exits 0
  rather than erroring.
- DuckDB access no longer hard-requires the `duckdb` CLI binary: `duckdb_rows`/`duckdb_query_arrow`
  now prefer the importable `duckdb` Python module and fall back to the CLI only when the module
  is absent — one SQL definition, two access paths.
- `examples/github_activity/resources/volume.yml`: new bundle resource declaring the Unity
  Catalog Volume, keyed on the same `${var.catalog}`/`${var.schema}`/`${var.volume_name}`
  references `resources/github_activity_job.yml`'s `smelt_run` task already composes its
  `--project-dir` from.
- `scripts/dbx-bundle.sh seed`: copies `smelt.yml` and `models/` onto the Volume via
  `databricks fs cp --recursive --overwrite`; the copy list deliberately excludes `.smelt/`.
  Already covered by the existing `.claude/settings.json` wildcard
  (`Bash(bash scripts/dbx-bundle.sh*)`) — no settings change needed.
- `resources/github_activity_job.yml`: `load_next_day` task now passes `--next-day` instead of
  `{{job.trigger.time.iso_date}}`; `loader_env` gains `duckdb` in its `dependencies:`.
- `examples/github_activity/dbx_job/load_next_day.py` updated to match (no more required date
  argument).
- `docs-site/docs/guide/targets.md` §"Deployment: Databricks Asset Bundle" documents `seed`, the
  Volume resource, and the self-driving loader.
- 8 new tests: 5 in `crates/smelt-cli/tests/dbx_dogfood_loader.rs` (next-day resolution,
  advancing across invocations, exhaustion is a clean no-op, module/CLI Arrow-byte parity, no
  `duckdb` CLI on `PATH`), 3 in `crates/smelt-cli/tests/databricks_bundle.rs` (Volume/task
  variable parity, `loader_env` dependency coverage, seed copy-list safety).

## Decisions

- `--next-day`'s live path opens two Databricks Connect sessions (one to read the ledger, one via
  the existing `cmd_execute`) rather than threading a shared adapter through — simpler, and the
  extra session cost is a once-daily scheduled job, not a hot path.
- The two duckdb-module-dependent tests (Arrow-byte parity, no-CLI-on-PATH) skip with a printed
  reason when `duckdb` isn't importable, matching this repo's existing `SPARK_CONNECT_URL`/
  `dbx_venv_python()` gating convention for optional local environments — neither the system
  `python3` nor `.smelt-dbx-venv` had it installed in this session, so both skipped here.

## For the next planner

- Phase 11c (live) can now proceed: `databricks bundle validate` passes locally against the stub
  with the new Volume resource present. Still untested against a real workspace: whether
  `databricks fs cp` targets a Volume path correctly pre-first-deploy (the Volume must exist
  before `seed` can write to it — 11c's task order should be deploy, then seed, then enable the
  schedule).
- Neither `duckdb` nor `pyarrow` were importable in the ambient system `python3` this session;
  `.smelt-dbx-venv` has `pyarrow` (from `databricks-connect`'s own dependency) but not `duckdb`.
  11c or a future phase should rebuild `.smelt-dbx-venv` via `scripts/dbx-dogfood-venv.sh` to pick
  up the new `duckdb` pin in `dbx-dogfood-requirements.txt`, so the module/CLI parity test
  actually runs rather than skipping.
- Out of scope, not done: threading `--next-day`'s live ledger read through the same
  `DatabricksAdapter` session `cmd_execute` opens (see Decisions above) — worth doing if session
  setup cost ever matters for this job.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test dbx_dogfood_loader --test databricks_bundle` — 28 passed, 0
  failed.
- `bash scripts/dbx-bundle.sh validate` (stub path, CLI v1.16.1 on `PATH`) — `Validation OK!`,
  new Volume resource present.
- `bash .claude/scripts/shellcheck-gate.sh` — zero findings.
- `cargo test --workspace --no-fail-fast --quiet` — exit 0, no failures anywhere in the suite.
