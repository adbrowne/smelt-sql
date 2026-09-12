# Phase 3 summary — Tooling, offline: pinned client, day loader

**Shipped:**
- `scripts/dbx-dogfood-requirements.txt` — exact `databricks-connect==15.4.5` + `pyarrow`
  pins, no bare `pyspark`.
- `scripts/dbx-dogfood-venv.sh` — creates `.smelt-dbx-venv` (disjoint from
  `.smelt-spark-venv`), installs the pins, verifies `smelt.databricks_adapter` imports
  against the real package.
- `scripts/dbx-dogfood-env.sh` — sourced; puts the dbx venv + repo `python/` on
  `PYTHONPATH`, exports `SMELT_DBX_CATALOG`/`SMELT_DBX_SCHEMA` (`workspace`/
  `smelt_dogfood`), reads `SMELT_DBX_HOST`/`SMELT_DBX_TOKEN` from a gpg-backed config
  dir (`~/.config/databricks-smelt-dogfood` by default) that phase 4 will populate —
  echoes the target, never the token.
- `scripts/dbx-dogfood-loader.py` + `.sh` — modes `--emit-ddl`, `--emit-sql --date D`,
  `--emit-slice-sql --date D`, `--date D` (execute), `--date D --dry-run-store <dir>`.
  The redelivery modulus is parsed out of `load_day.sh` at runtime, never restated.
  `--emit-slice-sql` prints the two DuckDB-executable SELECTs (event-time, arrival) that
  produce a day's rows straight from the fixture.
- `crates/smelt-cli/tests/dbx_dogfood_loader.rs` — 8 tests, all green, all offline.
- `examples/github_activity/README.md` — new "The Databricks loader" section next to
  the DuckDB/BigQuery ones.

**Decisions:**
- The resolved `databricks-connect` version is **15.4.5**, and
  `python/smelt/databricks_adapter.py` (phase 2) imports cleanly against it —
  `bash scripts/dbx-dogfood-venv.sh` ran once locally and its import-verification step
  passed with no changes needed to the adapter.
- `--emit-sql`/`--emit-ddl` describe the append in catalog-qualified terms only (no
  fixture path); the actual rows cross via `--emit-slice-sql`'s DuckDB SELECTs → Arrow →
  `DatabricksAdapter.load_arrow_table`, matching the "no host-visible file" rule.
- The idempotence ledger for `--dry-run-store` is a flat `loader_days.txt` under the
  given directory; the real `--date D` execute path instead checks a
  `{catalog}.{schema}._loader_days` Delta table before building any Arrow frame, mirroring
  `load_day.sh`'s own `main._loader_days` ordering.

**For the next planner:**
- **Correctness gap in the shared adapter, not this phase's scope to fix**:
  `DatabricksAdapter.load_arrow_table` (`python/smelt/databricks_adapter.py`, phase 2)
  drops and recreates the target table rather than appending. Calling it once per day —
  which is what `cmd_execute` in `dbx-dogfood-loader.py` currently does — would overwrite
  the previous day's rows instead of accumulating history. This has never been exercised
  against a live workspace. **Phase 5 (first live load) must fix this — either give the
  adapter an append mode or route the loader's execute path through
  `execute_sql_no_result` with an explicit `INSERT INTO ... SELECT` against a temp
  view — before trusting any multi-day load.** Flagged in-line at the call site too.
- Phase 4 (human-gated) needs, from this phase: the config dir path
  (`~/.config/databricks-smelt-dogfood`, overridable via `SMELT_DBX_CONFIG_DIR`), the
  expected `config.env` key (`SMELT_DBX_HOST`) and `token` file (plaintext, gpg-decrypted
  by an auth script phase 4 still needs to write — no `dbx-auth.sh` exists yet, unlike
  `bigquery-auth.sh`), and the two schema names (`smelt_dogfood`, `smelt_dogfood_oracle`
  per the outcome — only the first is wired into the loader's defaults so far).
- Nothing out of the stated phase boundary surfaced otherwise; `load_day.sh` needed no
  changes, and the BigQuery loader's own drift gate (`github_activity_loader.rs`) stayed
  green untouched.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets,
  shellcheck, full `cargo test`, example_diagnostics).
- `cargo test -p smelt-cli --test dbx_dogfood_loader --features duckdb --quiet` — 8 passed.
- `cargo test -p smelt-cli --test github_activity_loader --features duckdb --quiet` — 11
  passed (BigQuery loader's drift gate unaffected).
- `bash scripts/dbx-dogfood-venv.sh` — ran once locally; resolved
  `databricks-connect==15.4.5`, `smelt.databricks_adapter` import verified OK.
- `cargo test -p smelt-backend-spark --quiet` — green; `.smelt-spark-venv` untouched
  (did not exist before or after this phase's work).
