#!/usr/bin/env python3
"""dbx-dogfood-loader.py — the Databricks leg's day loader for
examples/github_activity/ (docs/outcomes/20260912-databricks-dogfood-spine/
phases/03-plan.md).

External to smelt, exactly as scripts/bq-dogfood-loader.sh is for BigQuery —
smelt orders the run, it does not author the load. It never restates
examples/github_activity/load_day.sh's redelivery rule: the modulus is
*parsed* out of that script at runtime, so a re-pin of load_day.sh cannot
silently drift out of step with what lands in Unity Catalog.

Modes:
    --emit-ddl                          Delta DDL for the two raw tables and
                                         the per-day ledger table. No network.
    --apply-ddl                         Executes the same statements
                                         --emit-ddl prints, against the live
                                         workspace (needs a credential).
    --emit-sql --date D                 Describes day D's append in
                                         catalog-qualified terms. No network.
    --emit-slice-sql --date D           The DuckDB-executable SELECT queries
                                         (event-time and arrival) that produce
                                         day D's rows straight from the
                                         fixture — the per-PR identity gate
                                         compares these to load_day.sh's own
                                         output with no workspace. No network.
    --date D                            Execute: read day D's slice from the
                                         fixture via DuckDB, hand it to
                                         Databricks Connect as Arrow, and
                                         append it into Unity Catalog. Rows
                                         cross via the Arrow load path
                                         (DatabricksAdapter.load_arrow_table)
                                         — Databricks Connect never reads a
                                         host-visible file, since serverless
                                         compute shares no filesystem with the
                                         client
                                         (docs/specs/multi_backend.md
                                         §"Loading data into a backend").
    --date D --dry-run-store <dir>      Same idempotence guard and fixture
                                         read as --date D, but records the day
                                         in a local ledger file under <dir>
                                         instead of touching a real
                                         workspace — proves the guard and the
                                         read path with no credential.

The idempotence guard always runs BEFORE any frame is built (matching
load_day.sh's own `_loader_days` check): an already-loaded day is a no-op
that touches neither the fixture nor Unity Catalog / the dry-run store.
"""

import argparse
import os
import re
import subprocess
import sys

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
LOAD_DAY_SH = os.path.join(REPO_ROOT, "examples/github_activity/load_day.sh")
SAMPLE_PARQUET = os.path.join(
    REPO_ROOT, "examples/github_activity/seeds/github_events_sample.parquet"
)

CATALOG = os.environ.get("SMELT_DBX_CATALOG", "workspace")
SCHEMA = os.environ.get("SMELT_DBX_SCHEMA", "smelt_dogfood")
EVENTS_TABLE = f"{CATALOG}.{SCHEMA}.github_events"
ARRIVAL_TABLE = f"{CATALOG}.{SCHEMA}.github_events_arrival"
LEDGER_TABLE = f"{CATALOG}.{SCHEMA}._loader_days"


def parsed_modulus():
    """The redelivery modulus, parsed out of load_day.sh rather than restated."""
    with open(LOAD_DAY_SH, encoding="utf-8") as f:
        text = f.read()
    m = re.search(r"MOD\(CAST\(id AS BIGINT\), (\d+)\)", text)
    if not m:
        raise RuntimeError(f"could not parse the redelivery modulus out of {LOAD_DAY_SH}")
    return int(m.group(1))


def events_select_sql(date, modulus):
    return (
        f"SELECT * FROM read_parquet('{SAMPLE_PARQUET}')\n"
        f"WHERE CAST(created_at AS DATE) = DATE '{date}'\n"
        "UNION ALL\n"
        f"SELECT * FROM read_parquet('{SAMPLE_PARQUET}')\n"
        f"WHERE CAST(created_at AS DATE) = DATE '{date}' - INTERVAL 1 DAY\n"
        f"  AND MOD(CAST(id AS BIGINT), {modulus}) = 0"
    )


def arrival_select_sql(date, modulus):
    return (
        f"SELECT *, DATE '{date}' AS ingested_date FROM read_parquet('{SAMPLE_PARQUET}')\n"
        f"WHERE CAST(created_at AS DATE) = DATE '{date}'\n"
        "UNION ALL\n"
        f"SELECT *, DATE '{date}' AS ingested_date FROM read_parquet('{SAMPLE_PARQUET}')\n"
        f"WHERE CAST(created_at AS DATE) = DATE '{date}' - INTERVAL 1 DAY\n"
        f"  AND MOD(CAST(id AS BIGINT), {modulus}) = 0"
    )


def cmd_emit_slice_sql(date):
    modulus = parsed_modulus()
    print("-- events")
    print(events_select_sql(date, modulus) + ";")
    print()
    print("-- arrival")
    print(arrival_select_sql(date, modulus) + ";")


def ddl_statements():
    """The Delta DDL for the two raw tables and the per-day ledger table, as a
    list of statements — the single source both --emit-ddl and --apply-ddl
    consume, so the two modes cannot drift apart."""
    return [
        f"CREATE TABLE IF NOT EXISTS {EVENTS_TABLE} (\n"
        f"  id STRING,\n"
        f"  type STRING,\n"
        f"  created_at TIMESTAMP,\n"
        f"  actor_id BIGINT,\n"
        f"  actor_login STRING,\n"
        f"  repo_id BIGINT,\n"
        f"  repo_name STRING,\n"
        f"  org_id BIGINT,\n"
        f"  public BOOLEAN,\n"
        f"  payload STRING\n"
        ")\n"
        "USING DELTA",
        f"CREATE TABLE IF NOT EXISTS {ARRIVAL_TABLE} (\n"
        f"  id STRING,\n"
        f"  type STRING,\n"
        f"  created_at TIMESTAMP,\n"
        f"  actor_id BIGINT,\n"
        f"  actor_login STRING,\n"
        f"  repo_id BIGINT,\n"
        f"  repo_name STRING,\n"
        f"  org_id BIGINT,\n"
        f"  public BOOLEAN,\n"
        f"  payload STRING,\n"
        f"  ingested_date DATE\n"
        ")\n"
        "USING DELTA",
        f"CREATE TABLE IF NOT EXISTS {LEDGER_TABLE} (\n"
        "  day DATE\n"
        ")\n"
        "USING DELTA",
    ]


def cmd_emit_ddl():
    print(
        f"-- Delta DDL for the Databricks dogfood leg "
        f"(docs/outcomes/20260912-databricks-dogfood-spine/phases/03-plan.md).\n"
        f"-- Catalog-qualified only — the fixture never appears here; rows\n"
        f"-- cross via the Arrow load path (see --date D to execute)."
    )
    for stmt in ddl_statements():
        print(stmt + ";\n")


def cmd_apply_ddl():
    from smelt.databricks_adapter import DatabricksAdapter

    host = os.environ.get("SMELT_DBX_HOST")
    if not host:
        print("SMELT_DBX_HOST is not set — source scripts/dbx-dogfood-env.sh first", file=sys.stderr)
        sys.exit(1)
    token = os.environ.get("SMELT_DBX_TOKEN")

    adapter = DatabricksAdapter(host, catalog=CATALOG, token=token)
    try:
        for stmt in ddl_statements():
            adapter.execute_sql_no_result(stmt)
        print("applied DDL")
    finally:
        adapter.close()


def cmd_emit_sql(date):
    modulus = parsed_modulus()
    print(
        f"-- Databricks load for day {date}: rows cross via the Arrow load path\n"
        "-- (DatabricksAdapter.load_arrow_table), never a host-visible file —\n"
        "-- serverless compute shares no filesystem with the client\n"
        '-- (docs/specs/multi_backend.md §"Loading data into a backend").\n'
        "--\n"
        f"-- Real day-{date} rows plus a deterministic 1/{modulus} slice of the\n"
        "-- previous day's rows (the loader's declared at-least-once redelivery\n"
        "-- behaviour, MOD(CAST(id AS BIGINT), "
        f"{modulus}) = 0) are appended, via\n"
        "-- createDataFrame(...).write.saveAsTable(...), to:\n"
        f"--   {EVENTS_TABLE}\n"
        f"--   {ARRIVAL_TABLE} (stamped ingested_date = DATE '{date}')\n"
        "--\n"
        f"-- Use --date {date} to execute, or --emit-slice-sql --date {date} to\n"
        "-- see the DuckDB-side SELECT that produces the rows."
    )


def duckdb_query_arrow(sql):
    """Runs `sql` against the fixture via the `duckdb` CLI and returns Arrow IPC
    stream bytes (stdlib subprocess only — no Python duckdb/pyarrow dependency
    is needed for this step, matching load_day.sh's own use of the CLI).

    `arrow` moved out of DuckDB's core/autoload-known extension set into the
    community repository, so unlike `parquet` it is never autoloaded — an
    explicit `LOAD` is required even once `INSTALL ... FROM community` has
    cached it on disk (`duckdb :memory: -c "INSTALL arrow FROM community"`,
    a one-time step this function does not perform itself)."""
    proc = subprocess.run(
        [
            "duckdb",
            ":memory:",
            "-c",
            f"LOAD arrow; COPY ({sql}) TO '/dev/stdout' (FORMAT arrow)",
        ],
        capture_output=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"duckdb query failed: {proc.stderr.decode(errors='replace')}")
    return proc.stdout


def duckdb_scalar(sql):
    proc = subprocess.run(
        ["duckdb", ":memory:", "-json", "-c", sql],
        capture_output=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"duckdb query failed: {proc.stderr.decode(errors='replace')}")
    import json

    rows = json.loads(proc.stdout.decode())
    return list(rows[0].values())[0]


def ledger_path(store_dir):
    return os.path.join(store_dir, "loader_days.txt")


def dry_run_already_loaded(store_dir, date):
    path = ledger_path(store_dir)
    if not os.path.exists(path):
        return False
    with open(path, encoding="utf-8") as f:
        return date in {line.strip() for line in f if line.strip()}


def dry_run_record(store_dir, date):
    with open(ledger_path(store_dir), "a", encoding="utf-8") as f:
        f.write(date + "\n")


def cmd_dry_run_store(date, store_dir):
    os.makedirs(store_dir, exist_ok=True)
    # The guard runs BEFORE any frame is built — an already-loaded day never
    # touches the fixture, matching load_day.sh's own ordering.
    if dry_run_already_loaded(store_dir, date):
        print(f"day {date} already loaded, skipping", file=sys.stderr)
        return

    modulus = parsed_modulus()
    events_count = duckdb_scalar(f"SELECT count(*) AS c FROM ({events_select_sql(date, modulus)}) t")
    arrival_count = duckdb_scalar(
        f"SELECT count(*) AS c FROM ({arrival_select_sql(date, modulus)}) t"
    )
    dry_run_record(store_dir, date)
    print(f"loaded day {date} (dry run): {events_count} events, {arrival_count} arrival rows")


def cmd_execute(date):
    from smelt.databricks_adapter import DatabricksAdapter

    host = os.environ.get("SMELT_DBX_HOST")
    if not host:
        print("SMELT_DBX_HOST is not set — source scripts/dbx-dogfood-env.sh first", file=sys.stderr)
        sys.exit(1)
    token = os.environ.get("SMELT_DBX_TOKEN")

    adapter = DatabricksAdapter(host, catalog=CATALOG, token=token)
    try:
        if adapter.table_exists(LEDGER_TABLE):
            already = adapter.execute_sql(
                f"SELECT count(*) AS c FROM {LEDGER_TABLE} WHERE day = DATE '{date}'"
            )
            if already.column("c")[0].as_py() > 0:
                print(f"day {date} already loaded, skipping", file=sys.stderr)
                return

        modulus = parsed_modulus()
        events_bytes = duckdb_query_arrow(events_select_sql(date, modulus))
        arrival_bytes = duckdb_query_arrow(arrival_select_sql(date, modulus))

        adapter.load_arrow_table(events_bytes, EVENTS_TABLE, mode="append")
        adapter.load_arrow_table(arrival_bytes, ARRIVAL_TABLE, mode="append")
        adapter.execute_sql_no_result(f"INSERT INTO {LEDGER_TABLE} VALUES (DATE '{date}')")
        print(f"loaded day {date}")
    finally:
        adapter.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--emit-ddl", action="store_true")
    parser.add_argument("--apply-ddl", action="store_true")
    parser.add_argument("--emit-sql", action="store_true")
    parser.add_argument("--emit-slice-sql", action="store_true")
    parser.add_argument("--date")
    parser.add_argument("--dry-run-store")
    args = parser.parse_args()

    if args.emit_ddl:
        cmd_emit_ddl()
        return
    if args.apply_ddl:
        cmd_apply_ddl()
        return
    if args.emit_sql:
        if not args.date:
            parser.error("--emit-sql requires --date")
        cmd_emit_sql(args.date)
        return
    if args.emit_slice_sql:
        if not args.date:
            parser.error("--emit-slice-sql requires --date")
        cmd_emit_slice_sql(args.date)
        return
    if not args.date:
        parser.error("--date is required")
    if args.dry_run_store:
        cmd_dry_run_store(args.date, args.dry_run_store)
        return
    cmd_execute(args.date)


if __name__ == "__main__":
    main()
