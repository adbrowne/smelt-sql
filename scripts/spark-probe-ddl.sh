#!/usr/bin/env bash
# spark-probe-ddl.sh — which schema-evolution DDL forms does Spark accept?
#
#     bash scripts/spark-up.sh             # start the Delta-enabled server
#     source scripts/spark-env.sh
#     bash scripts/spark-probe-ddl.sh
#
# smelt's Spark generator (`crates/smelt-state/src/ddl_spark.rs`) turns
# backend-agnostic `SchemaOperation`s into Spark SQL. This probe establishes,
# against a live server, which forms Delta and Parquet tables really accept —
# the measured facts the generator's rules are written from. Both formats are
# probed for every form, because Delta and Parquet diverge on most of them.
#
# Every case gets its own fresh table: a form that fails can leave the table in
# a state that makes the next form's answer meaningless.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."
[[ -n "${SPARK_CONNECT_URL:-}" ]] || { echo "no SPARK_CONNECT_URL — run: source scripts/spark-env.sh" >&2; exit 1; }

exec "${PYSPARK_PYTHON:-.smelt-spark-venv/bin/python}" - "$@" <<'PY'
import os, sys, uuid
from pyspark.sql import SparkSession

spark = SparkSession.builder.remote(os.environ["SPARK_CONNECT_URL"]).getOrCreate()
run = uuid.uuid4().hex[:8]
n = 0

def probe(label, defs, stmt, fmt, seed=None):
    """CREATE a fresh <fmt> table with <defs>, run <stmt>, report.

    `@T` in <stmt> is the catalog-qualified name; `@LEAF` is the bare table
    name, for probing what a statement missing the catalog resolves to.
    """
    global n
    n += 1
    t = f"spark_catalog.default.probe_{run}_{n}"
    try:
        spark.sql(f"CREATE TABLE {t} ({defs}) USING {fmt}")
        if seed:
            spark.sql(seed.replace("@T", t))
    except Exception as e:
        print(f"  SETUP-FAIL  {fmt:<7} {label:<44} -- {str(e).splitlines()[0][:90]}")
        return
    try:
        spark.sql(stmt.replace("@T", t).replace("@LEAF", t.rsplit(".", 1)[1]))
        print(f"  ACCEPTED    {fmt:<7} {label}")
    except Exception as e:
        print(f"  REFUSED     {fmt:<7} {label:<44} -- {str(e).splitlines()[0][:110]}")
    finally:
        try:
            spark.sql(f"DROP TABLE IF EXISTS {t}")
        except Exception:
            pass

def both(label, defs, stmt, seed=None):
    for fmt in ("DELTA", "PARQUET"):
        probe(label, defs, stmt, fmt, seed)

ROW = "INSERT INTO @T VALUES (1, 'x')"
BASE = "id BIGINT, label STRING"

print("── ADD COLUMN ─────────────────────────────────────────────────────")
both("ADD COLUMNS (c STRING)",           BASE, "ALTER TABLE @T ADD COLUMNS (note STRING)", ROW)
both("ADD COLUMN (singular)",            BASE, "ALTER TABLE @T ADD COLUMN note STRING", ROW)
both("ADD COLUMNS (c VARCHAR) bare",     BASE, "ALTER TABLE @T ADD COLUMNS (note VARCHAR)", ROW)
both("ADD COLUMNS (c TEXT)",             BASE, "ALTER TABLE @T ADD COLUMNS (note TEXT)", ROW)
both("ADD COLUMNS (c DOUBLE)",           BASE, "ALTER TABLE @T ADD COLUMNS (ratio DOUBLE)", ROW)
both("ADD COLUMNS (c INTEGER)",          BASE, "ALTER TABLE @T ADD COLUMNS (n INTEGER)", ROW)
both("ADD COLUMNS (c STRING NOT NULL)",  BASE, "ALTER TABLE @T ADD COLUMNS (note STRING NOT NULL)", ROW)
both("ADD COLUMNS (c STRING DEFAULT 'a')", BASE, "ALTER TABLE @T ADD COLUMNS (note STRING DEFAULT 'a')", ROW)
both("ADD COLUMNS NOT NULL DEFAULT",     BASE, "ALTER TABLE @T ADD COLUMNS (note STRING NOT NULL DEFAULT 'a')", ROW)
for fmt in ("DELTA", "PARQUET"):
    probe("two-part name (catalog omitted)", BASE,
          "ALTER TABLE default.@LEAF ADD COLUMNS (note STRING)", fmt, ROW)

print("── DROP COLUMN ────────────────────────────────────────────────────")
both("DROP COLUMN",                      BASE, "ALTER TABLE @T DROP COLUMN label", ROW)

print("── WIDEN ──────────────────────────────────────────────────────────")
both("ALTER COLUMN c TYPE BIGINT",       "id INT, label STRING", "ALTER TABLE @T ALTER COLUMN id TYPE BIGINT", "INSERT INTO @T VALUES (1, 'x')")
both("ALTER COLUMN c SET DATA TYPE",     "id INT, label STRING", "ALTER TABLE @T ALTER COLUMN id SET DATA TYPE BIGINT", "INSERT INTO @T VALUES (1, 'x')")
both("widen INT -> DOUBLE",              "id INT, label STRING", "ALTER TABLE @T ALTER COLUMN id TYPE DOUBLE", "INSERT INTO @T VALUES (1, 'x')")
both("widen DECIMAL(5,2)->DECIMAL(10,4)","amount DECIMAL(5,2), label STRING", "ALTER TABLE @T ALTER COLUMN amount TYPE DECIMAL(10,4)", "INSERT INTO @T VALUES (1.25, 'x')")
both("widen with DuckDB type VARCHAR",   BASE, "ALTER TABLE @T ALTER COLUMN label TYPE VARCHAR", ROW)

print("── NULLABILITY ────────────────────────────────────────────────────")
both("DROP NOT NULL",                    "id BIGINT NOT NULL, label STRING", "ALTER TABLE @T ALTER COLUMN id DROP NOT NULL", "INSERT INTO @T VALUES (1, 'x')")
both("SET NOT NULL (no NULLs present)",  BASE, "ALTER TABLE @T ALTER COLUMN label SET NOT NULL", ROW)
both("SET NOT NULL (NULLs present)",     BASE, "ALTER TABLE @T ALTER COLUMN label SET NOT NULL", "INSERT INTO @T VALUES (1, NULL)")

print("── DML ────────────────────────────────────────────────────────────")
both("UPDATE (no WHERE)",                BASE, "UPDATE @T SET label = 'y'", ROW)
both("UPDATE ... WHERE col IS NULL",     BASE, "UPDATE @T SET label = 'y' WHERE label IS NULL", "INSERT INTO @T VALUES (1, NULL)")
PY
