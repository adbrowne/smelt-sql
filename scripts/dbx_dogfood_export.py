#!/usr/bin/env python3
"""Export every compared relation of a Databricks schema to typed NDJSON.

One `DatabricksAdapter` session for the whole run (not one per relation, and
not the CLI's own compiled `smelt.yml` connection). Discovers relations from
`${SMELT_DBX_CATALOG}.information_schema.tables` for `${SMELT_DBX_SCHEMA}`,
applies the same exclusion rules `crates/smelt-cli/tests/parity_support/mod.rs`
applies (`sources_`/`_smelt_` prefixes, `__tombstones` suffix, the exact
source-table names), and writes one `<relation>.ndjson` per relation into
`<out_dir>`.

Read-only: every statement issued is a `SELECT`. This is the Databricks half
of the landing seam's **export encoding contract**
(`crates/smelt-cli/tests/parity_support/mod.rs` module doc): a TIMESTAMP
column is written as epoch seconds (a float, so sub-second precision
survives), every other column as text `json.dumps(..., default=str)`
produces — the same encoding `bq_dogfood_export.py` produces from BigQuery's
own REST rows, so `load_exported_snapshot` needs no per-target branch.

    python3 scripts/dbx_dogfood_export.py <out_dir>

Reads SMELT_DBX_HOST, SMELT_DBX_TOKEN (optional — absent means ambient
credentials), SMELT_DBX_CATALOG (default "workspace") and SMELT_DBX_SCHEMA
(default "smelt_dogfood") from the environment. Never prints the token.
"""

import json
import os
import sys

import pyarrow as pa
import pyarrow.compute as pc

# Kept in lockstep with crates/smelt-cli/tests/parity_support/mod.rs's
# EXCLUDED_PREFIXES / EXCLUDED_SUFFIXES / EXCLUDED_EXACT.
EXCLUDED_PREFIXES = ("sources_", "_smelt_")
EXCLUDED_SUFFIXES = ("__tombstones",)
EXCLUDED_EXACT = {"github_events", "github_events_arrival", "_loader_days"}


def is_compared(name):
    if any(name.startswith(p) for p in EXCLUDED_PREFIXES):
        return False
    if any(name.endswith(s) for s in EXCLUDED_SUFFIXES):
        return False
    if name in EXCLUDED_EXACT:
        return False
    return True


def _is_timestamp(field):
    return pa.types.is_timestamp(field.type)


def _encode_table(table):
    """One JSON-ready dict per row: TIMESTAMP columns as epoch-second floats,
    everything else left to `json.dumps(..., default=str)`."""
    columns = {}
    for field in table.schema:
        col = table.column(field.name)
        if _is_timestamp(field):
            micros = pc.cast(col, pa.int64())
            columns[field.name] = [
                None if v.as_py() is None else v.as_py() / 1_000_000 for v in micros
            ]
        else:
            columns[field.name] = col.to_pylist()
    n = table.num_rows
    for i in range(n):
        yield {name: columns[name][i] for name in columns}


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: dbx_dogfood_export.py <out_dir>")
    out_dir = sys.argv[1]

    host = os.environ.get("SMELT_DBX_HOST")
    if not host:
        sys.exit("SMELT_DBX_HOST is not set — source scripts/dbx-dogfood-env.sh first")
    token = os.environ.get("SMELT_DBX_TOKEN")
    catalog = os.environ.get("SMELT_DBX_CATALOG", "workspace")
    schema = os.environ.get("SMELT_DBX_SCHEMA", "smelt_dogfood")

    os.makedirs(out_dir, exist_ok=True)

    from smelt.databricks_adapter import DatabricksAdapter

    adapter = DatabricksAdapter(host, catalog=catalog, token=token)
    try:
        relations_table = adapter.execute_sql(
            f"SELECT table_name FROM {catalog}.information_schema.tables "
            f"WHERE table_schema = '{schema}' AND table_type = 'MANAGED' "
            "ORDER BY table_name"
        )
        relations = [
            row["table_name"]
            for row in relations_table.to_pylist()
            if is_compared(row["table_name"])
        ]
        for relation in relations:
            table = adapter.execute_sql(f"SELECT * FROM {catalog}.{schema}.{relation}")
            out_path = os.path.join(out_dir, f"{relation}.ndjson")
            with open(out_path, "w") as out:
                total = 0
                for row in _encode_table(table):
                    out.write(json.dumps(row, default=str) + "\n")
                    total += 1
            print(f"{relation}: {total} rows -> {out_path}")
        print(f"exported {len(relations)} relation(s) to {out_dir}")
    finally:
        adapter.close()


if __name__ == "__main__":
    main()
