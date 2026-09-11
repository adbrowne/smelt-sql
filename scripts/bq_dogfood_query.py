#!/usr/bin/env python3
"""Run one GoogleSQL statement against the dogfood project and print its rows.

Sibling of `bq_dogfood_export.py`, for the small statements the dogfood scripts
issue between the big exports: catalogue reads, census counts, and DDL. Rows go
to stdout as newline-delimited JSON (BigQuery's REST encoding decoded back to
the declared type); one line of job statistics — rows, `totalBytesBilled`,
`numDmlAffectedRows`, job id — goes to stderr, so a caller can pipe stdout
without losing the cost figure.

    python3 scripts/bq_dogfood_query.py <sql-file>|- [--dry]

Reads SMELT_BQ_ACCESS_TOKEN and SMELT_BQ_PROJECT from the environment; the
token is never printed.
"""

import json
import os
import sys
import urllib.error
import urllib.request

API = "https://bigquery.googleapis.com/bigquery/v2"


def _request(url, token, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method="POST" if data else "GET")
    req.add_header("Authorization", f"Bearer {token}")
    if data:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req) as resp:
            return json.load(resp)
    except urllib.error.HTTPError as e:
        sys.exit(f"HTTP {e.code}: {e.read().decode()[:3000]}")


def _decode(field, cell):
    """BigQuery REST encodes every value as a string; recover the declared type."""
    value = cell["v"]
    if value is None:
        return None
    kind = field["type"]
    if kind in ("INTEGER", "INT64"):
        return int(value)
    if kind in ("FLOAT", "FLOAT64", "NUMERIC", "BIGNUMERIC"):
        return float(value)
    if kind in ("BOOLEAN", "BOOL"):
        return value == "true"
    return value


def main():
    source = sys.argv[1]
    sql = sys.stdin.read() if source == "-" else open(source).read()
    dry = "--dry" in sys.argv[2:]
    token = os.environ["SMELT_BQ_ACCESS_TOKEN"]
    project = os.environ["SMELT_BQ_PROJECT"]

    result = _request(
        f"{API}/projects/{project}/queries",
        token,
        {
            "query": sql,
            "useLegacySql": False,
            "maxResults": 20000,
            "timeoutMs": 300000,
            "dryRun": dry,
        },
    )
    if "error" in result:
        sys.exit(json.dumps(result["error"], indent=2))
    if dry:
        print(json.dumps({"dryRunBytes": result["statistics"]["totalBytesProcessed"]}))
        return

    schema = result.get("schema", {}).get("fields", [])
    job = result["jobReference"]
    page, total = result, 0
    while True:
        for row in page.get("rows", []):
            print(
                json.dumps(
                    {f["name"]: _decode(f, c) for f, c in zip(schema, row["f"])}
                )
            )
            total += 1
        token_next = page.get("pageToken")
        if not token_next:
            break
        page = _request(
            f"{API}/projects/{job['projectId']}/queries/{job['jobId']}"
            f"?location={job.get('location', 'US')}"
            f"&pageToken={token_next}&maxResults=20000",
            token,
        )

    sys.stderr.write(
        json.dumps(
            {
                "rows": total,
                "billed": result.get("totalBytesBilled"),
                "dml": result.get("numDmlAffectedRows"),
                "job": job["jobId"],
            }
        )
        + "\n"
    )


if __name__ == "__main__":
    main()
