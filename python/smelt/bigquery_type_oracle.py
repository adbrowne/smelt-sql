"""Line-protocol BigQuery type oracle for smelt's type property tests.

The property-test harness asks the warehouse "what type does this expression
have?" thousands of times per run. Spawning an interpreter and a BigQuery
client per question would dominate the cost of the run, so this module is a
tiny persistent server: one interpreter, one client, one dry-run job per
question.

Protocol — one JSON object per line in, exactly one JSON object per line out:

    in   {"sql": "SELECT CAST(1 AS INT64) AS c"}
    out  {"columns": [{"name": "c", "type": "INTEGER", "mode": "NULLABLE",
                       "fields": []}]}
    out  {"error": "<message>"}     (the warehouse refused this SQL)

A refusal is an ordinary reply, not a crash: the harness generates SQL that
BigQuery is entitled to reject, and those cases are skipped rather than scored.

Nothing but protocol lines is ever written to stdout — diagnostics go to
stderr. Startup failures are reported as one `{"error": ...}` line on stdout
followed by a non-zero exit, because a silent death would look to the Rust
side like *every* query being rejected: a whole warehouse's disagreement
quietly recoloured as "nothing to check". Making that indistinguishable-looking
failure impossible is the point of the shape.

Credentials come from the same environment `scripts/bigquery-env.sh` exports;
the adapter refuses application-default credentials, and this module opens no
other route to GCP.
"""

import json
import os
import sys

from smelt.bigquery_adapter import BigQueryAdapter


def _emit(obj):
    """Write one protocol line and flush.

    The flush is required, not tidiness: the Rust side blocks reading a single
    line, so a reply sitting in a pipe buffer is a deadlock.
    """
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def _build_adapter():
    project = os.environ.get("SMELT_BQ_PROJECT")
    if not project:
        raise ValueError("SMELT_BQ_PROJECT is not set")
    return BigQueryAdapter(
        project=project,
        dataset=os.environ.get("SMELT_BQ_DATASET"),
        location=os.environ.get("SMELT_BQ_LOCATION"),
        access_token=os.environ.get("SMELT_BQ_ACCESS_TOKEN"),
    )


def main():
    try:
        adapter = _build_adapter()
    except Exception as exc:  # noqa: BLE001 — any startup failure must be visible
        _emit({"error": f"bigquery type oracle startup failed: {exc}"})
        return 1

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
            sql = request["sql"]
        except Exception as exc:  # noqa: BLE001
            _emit({"error": f"malformed request: {exc}"})
            continue

        try:
            _emit({"columns": adapter.dry_run_schema(sql)})
        except Exception as exc:  # noqa: BLE001 — a refusal is an expected reply
            _emit({"error": str(exc)})

    return 0


if __name__ == "__main__":
    sys.exit(main())
