#!/usr/bin/env python3
"""Run one BigQuery SQL file and write every result row as newline-delimited JSON.

Reads SMELT_BQ_ACCESS_TOKEN / SMELT_BQ_PROJECT from the environment (export them
with `. scripts/bigquery-env.sh`); the token is never printed. Pages through
jobs.getQueryResults so the row count is bounded only by the query, and decodes
BigQuery's REST row encoding (every scalar arrives as a string) back to typed
JSON using the result schema.

    python3 scripts/bq_dogfood_export.py <sql-file> <out.ndjson>
"""

import json
import os
import sys
import urllib.request

API = "https://bigquery.googleapis.com/bigquery/v2"


def _request(url, token, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method="POST" if data else "GET")
    req.add_header("Authorization", f"Bearer {token}")
    if data:
        req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req) as resp:
        return json.load(resp)


def _decode(field, cell):
    """BigQuery REST encodes every value as a string; recover the declared type."""
    if field.get("mode") == "REPEATED":
        return [_decode({**field, "mode": "NULLABLE"}, item) for item in cell["v"]]
    value = cell["v"]
    if value is None:
        return None
    kind = field["type"]
    if kind == "RECORD":
        return _decode_row(field["fields"], value["f"])
    if kind in ("INTEGER", "INT64"):
        return int(value)
    if kind in ("FLOAT", "FLOAT64", "NUMERIC", "BIGNUMERIC"):
        return float(value)
    if kind in ("BOOLEAN", "BOOL"):
        return value == "true"
    if kind == "TIMESTAMP":
        # Epoch seconds with microsecond fraction; keep full precision as a float.
        return float(value)
    return value


def _decode_row(fields, cells):
    return {f["name"]: _decode(f, c) for f, c in zip(fields, cells)}


def main():
    sql_path, out_path = sys.argv[1], sys.argv[2]
    token = os.environ["SMELT_BQ_ACCESS_TOKEN"]
    project = os.environ["SMELT_BQ_PROJECT"]
    sql = open(sql_path).read()

    result = _request(
        f"{API}/projects/{project}/queries",
        token,
        {"query": sql, "useLegacySql": False, "maxResults": 10000, "timeoutMs": 180000},
    )
    if "error" in result:
        sys.exit(json.dumps(result["error"], indent=2))

    job = result["jobReference"]
    schema = result["schema"]["fields"]
    total = 0
    with open(out_path, "w") as out:
        page = result
        while True:
            for row in page.get("rows", []):
                out.write(json.dumps(_decode_row(schema, row["f"])) + "\n")
                total += 1
            token_next = page.get("pageToken")
            if not token_next:
                break
            page = _request(
                f"{API}/projects/{job['projectId']}/queries/{job['jobId']}"
                f"?location={job.get('location', 'US')}"
                f"&pageToken={token_next}&maxResults=10000",
                token,
            )

    print(f"{total} rows -> {out_path}")
    print(f"bytes billed: {result.get('totalBytesProcessed')}")


if __name__ == "__main__":
    main()
