"""Probe what the google-cloud-bigquery client reports for a dry-run query.

The type oracle reaches BigQuery through `smelt.bigquery_adapter`, so the fact
that the REST API carries an output schema on a dry run is only useful if the
Python client surfaces the same thing. This asks the client directly, and dumps
each SchemaField's attributes verbatim so an attribute the caller did not think
to ask for is still visible.

Run via `bash scripts/bigquery-probe-schema.sh`, which supplies PYTHONPATH and
the token.
"""

import os
import time

CASES = [
    ("int64", "CAST(1 AS INT64) AS c"),
    ("float64", "CAST(1.5 AS FLOAT64) AS c"),
    ("array of int64", "[1,2,3] AS c"),
    ("struct", "STRUCT(1 AS a, 'x' AS b) AS c"),
    ("array of struct", "[STRUCT(1 AS a)] AS c"),
    ("numeric", "CAST(99.99 AS NUMERIC) AS c"),
    ("bignumeric", "CAST(99.99 AS BIGNUMERIC) AS c"),
    ("interval", "INTERVAL 1 DAY AS c"),
    ("json", "JSON '{\"a\":1}' AS c"),
    ("multi-column", "CAST(1 AS INT64) AS a, CAST('x' AS STRING) AS b"),
]


def main():
    from google.cloud import bigquery
    from google.oauth2.credentials import Credentials

    token = os.environ["SMELT_BQ_ACCESS_TOKEN"]
    project = os.environ["SMELT_BQ_PROJECT"]
    location = os.environ.get("SMELT_BQ_LOCATION")

    client = bigquery.Client(
        project=project,
        credentials=Credentials(token=token),
        location=location,
    )

    config = bigquery.QueryJobConfig(dry_run=True, use_query_cache=False)

    for label, select_list in CASES:
        sql = f"SELECT {select_list}"
        start = time.monotonic()
        try:
            job = client.query(sql, job_config=config)
        except Exception as exc:  # noqa: BLE001 — the refusal is the finding
            print(f"  {label}: REJECTED — {str(exc)[:110]}")
            continue
        elapsed_ms = int((time.monotonic() - start) * 1000)

        schema = job.schema
        if schema is None:
            print(f"  {label}: dry run carried NO schema ({elapsed_ms}ms)")
            continue

        rendered = [
            {
                "name": f.name,
                "field_type": f.field_type,
                "mode": f.mode,
                "precision": f.precision,
                "scale": f.scale,
                "fields": [(sf.name, sf.field_type, sf.mode) for sf in (f.fields or ())],
            }
            for f in schema
        ]
        print(f"  {label} ({elapsed_ms}ms): {rendered}")

    # A dry run must reject invalid SQL, otherwise the oracle would report a
    # schema for a query the warehouse would never accept.
    try:
        client.query("SELECT nosuchfunction(1) AS c", job_config=config)
        print("  invalid SQL: ACCEPTED — dry run does not validate!")
    except Exception as exc:  # noqa: BLE001
        print(f"  invalid SQL: rejected — {str(exc)[:90]}")

    # Latency of a repeated dry run, which is what the property test pays per
    # generated case.
    sql = "SELECT CAST(1 AS INT64) AS c"
    start = time.monotonic()
    runs = 10
    for _ in range(runs):
        client.query(sql, job_config=config)
    per_call = int((time.monotonic() - start) * 1000 / runs)
    print(f"  {runs} sequential dry runs: {per_call}ms each")


if __name__ == "__main__":
    main()
