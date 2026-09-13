# Phase 3 plan — Tooling, offline: pinned client venv, `dbx-dogfood-env.sh`, and the day loader

**Objective.** Build everything the live phases need to talk to a Databricks workspace, and
prove it without one: a pinned `databricks-connect` client environment separate from the
local-Spark `pyspark` venv, a `scripts/dbx-dogfood-env.sh` that mirrors `scripts/spark-env.sh`,
and a day loader that replays the committed Parquet fixture into Unity Catalog with exactly
`examples/github_activity/load_day.sh`'s redelivery rule. Advances success criteria 2 (pinned
client environment, reproducible from one script) and 3 (loader + per-PR slice-identity gate),
and keeps criterion 10's "Spark parity tier is unaffected" true by construction.

**Spec delta.** None. Phase 1 already specified the target shape, the connection path and the
loading rule (`docs/specs/multi_backend.md` §"Connection security", §"Loading data into a
backend", §"Why Databricks is a distinct target type"); this phase ships tooling *external to
smelt*, whose contract is smelt's source declaration. User-facing documentation of the loader
goes in `examples/github_activity/README.md` alongside the existing DuckDB and BigQuery loader
sections, not in a spec.

## Tests (red first)

`crates/smelt-cli/tests/dbx_dogfood_loader.rs` (new, `#![cfg(feature = "duckdb")]`, reusing
`github_activity_support::{FIXTURE_DAYS, duckdb_exec, duckdb_scalar_i64, load_day}`):

1. `slice_matches_load_day_rows_for_every_fixture_day` — for every day in `FIXTURE_DAYS`, the
   rows `--emit-slice-sql --date D` selects from the fixture are a multiset match for the rows
   `load_day.sh` inserts into `main.sources_raw_github_events` for D (symmetric `EXCEPT ALL`
   both directions, plus equal counts).
2. `arrival_slice_stamps_ingested_date_like_load_day` — same comparison for the arrival twin,
   `ingested_date = D` on both the real-day and the redelivered rows.
3. `first_fixture_day_has_an_empty_redelivery_arm` — on the earliest fixture day the D-1 arm
   matches zero rows and the loader needs no special case (count equals the plain day count).
4. `redelivery_modulus_is_parsed_from_load_day_not_restated` — the loader's modulus is read out
   of `load_day.sh`; a drift between the two fails here, mirroring `github_activity_loader.rs`.
5. `loader_is_idempotent_per_day` — `--date D --dry-run-store <dir>` twice: the first run
   records D in the store's ledger and reports it loaded, the second exits 0 reporting "already
   loaded" and writes nothing further (the real guard runs before any frame is built).
6. `emitted_load_statements_reference_no_host_path` — the Spark-side statements the loader
   emits (`--emit-sql`, `--emit-ddl`) name only catalog-qualified tables; the fixture path
   appears solely in the local DuckDB read, proving rows cross by the Arrow load path.
7. `client_env_is_pinned_and_disjoint_from_the_spark_venv` — the pin file names an exact
   `databricks-connect==` version and no bare `pyspark`, and the venv script's target directory
   is not `.smelt-spark-venv`.
8. `dbx_dogfood_env_exports_the_dbx_venv_and_no_secret` — sourcing `scripts/dbx-dogfood-env.sh`
   with no credential config on disk puts the dbx venv and repo `python/` on `PYTHONPATH`, never
   `.smelt-spark-venv`, prints no token-shaped value, and leaves the live gate variable unset so
   Databricks-targeted tests skip green.

## Tasks

1. Red: add `crates/smelt-cli/tests/dbx_dogfood_loader.rs` with the eight tests above.
2. `scripts/dbx-dogfood-requirements.txt` — exact pins for `databricks-connect` and `pyarrow`,
   with a comment stating the resolved version and why `pyspark` must not be co-installed.
3. `scripts/dbx-dogfood-venv.sh` — `uv venv .smelt-dbx-venv` (Python 3.12, `--allow-existing`)
   + `uv pip install -r` the pin file, then verify `from smelt.databricks_adapter import
   DatabricksAdapter` imports against the real package (the check phase 2's summary asked for).
   Refuses to run if `.smelt-spark-venv` is the target; leaves that venv untouched.
4. `scripts/dbx-dogfood-env.sh` — sourced, no shebang, `# shellcheck shell=bash`; exports
   `PYTHONPATH` (dbx venv site-packages + repo `python/`), `PYSPARK_PYTHON`, catalog/schema
   defaults (`workspace`, `smelt_dogfood`), and reads host/token from the gpg-backed config dir
   phase 4 will create if it exists — echoing the target, never the token.
5. `scripts/dbx-dogfood-loader.py` — stdlib-only argument handling with modes
   `--emit-ddl`, `--emit-sql --date D`, `--emit-slice-sql --date D`, `--date D` (execute), and
   `--date D --dry-run-store <dir>`. Execute path: read the day's slice from the fixture with
   DuckDB → Arrow → `spark.createDataFrame` → append to `workspace.smelt_dogfood.github_events`
   and `…_arrival`; per-day idempotence via a `_loader_days` Delta table checked first.
6. `scripts/dbx-dogfood-loader.sh` — thin wrapper sourcing `dbx-dogfood-env.sh` and running the
   loader under the dbx venv's interpreter, so criterion 4's `scripts/dbx-*.sh` allow-list is
   the only entry point a session needs.
7. Green the tests; document the loader in `examples/github_activity/README.md` (external to
   smelt, its contract is the source declaration) next to the DuckDB/BigQuery loader sections.
8. Write `phases/03-summary.md`: what shipped, the resolved `databricks-connect` version and
   whether its API surface matched `python/smelt/databricks_adapter.py`, and anything phase 4's
   human step must know (schemas, grants, config-dir layout the env script expects).

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, shellcheck, full
  `cargo test`, example_diagnostics) — shellcheck is zero-findings and covers the new scripts.
- `cargo test -p smelt-cli --test dbx_dogfood_loader --quiet`.
- `cargo test -p smelt-cli --test github_activity_loader --quiet` (the BigQuery loader's drift
  gate must stay green while `load_day.sh` gains a second consumer).
- `bash scripts/dbx-dogfood-venv.sh` run once locally; record the resolved version. If the
  install cannot reach the index, say so in the summary rather than claiming the check ran.
- `ls .smelt-spark-venv` unchanged and `cargo test -p smelt-backend-spark --quiet` green — the
  Spark parity tier is untouched.

## Commit message

`outcome(databricks-dogfood-spine): phase 3 pins the databricks-connect client and the day loader`
