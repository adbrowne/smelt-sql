#!/usr/bin/env python3
"""Run one SQL statement against the Databricks dogfood workspace and print
its rows as newline-delimited JSON.

Sibling of `bq_dogfood_query.py`, over `smelt.databricks_adapter.DatabricksAdapter`
instead of BigQuery's REST API — Databricks Connect is the one connection path
this outcome proves (docs/outcomes/20260912-databricks-dogfood-spine/outcome.md
§"Out of scope"). The only entry points a Claude session may reach this
through are scripts/dbx-query.sh (read-only, allow-listed) and
scripts/dbx-verify.sh (also allow-listed, asserts a write outside the two
dogfood schemas is refused).

    python3 scripts/dbx_dogfood_query.py <sql>

Reads SMELT_DBX_HOST, SMELT_DBX_TOKEN (optional — absent means ambient
credentials) and SMELT_DBX_CATALOG from the environment. Never prints the
token.
"""

import json
import os
import sys


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: dbx_dogfood_query.py <sql>")
    sql = sys.argv[1]

    host = os.environ.get("SMELT_DBX_HOST")
    if not host:
        sys.exit("SMELT_DBX_HOST is not set — source scripts/dbx-dogfood-env.sh first")
    token = os.environ.get("SMELT_DBX_TOKEN")
    catalog = os.environ.get("SMELT_DBX_CATALOG", "workspace")

    from smelt.databricks_adapter import DatabricksAdapter

    adapter = DatabricksAdapter(host, catalog=catalog, token=token)
    try:
        table = adapter.execute_sql(sql)
        rows = table.to_pylist()
        for row in rows:
            print(json.dumps(row, default=str))
        sys.stderr.write(json.dumps({"rows": len(rows)}) + "\n")
    finally:
        adapter.close()


if __name__ == "__main__":
    main()
