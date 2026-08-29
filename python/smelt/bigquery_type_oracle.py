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

    in   {"exec": "SELECT 2 + 3 AS s"}
    out  {"rows": [[{"t": "int", "v": "5"}]]}
    out  {"error": "<message>"}     (the warehouse refused this SQL)

The `sql` verb dry-runs and bills nothing; the `exec` verb really executes and
therefore really costs money. Every cell of an `exec` reply is a
`{"t": <tag>, "v": <string>}` pair — never a bare JSON number, because JSON's
double would silently round an INT64 or a NUMERIC. Tags are `null`, `int`,
`float`, `bool`, `text`, `decimal`, `date`, `timestamp`; anything else the
warehouse returns is tagged `text` with its canonical rendering, so the Rust
side never has to guess.

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


def _tag_cell(value):
    """Render one Python/pyarrow value as a `{"t", "v"}` tagged cell.

    Every value is carried as a *string*. A JSON number would lose precision on
    exactly the two types this audit cares most about — INT64 beyond 2^53 and
    NUMERIC — and a silently rounded value read as agreement is the failure
    mode the value leg exists to catch.
    """
    import datetime
    import decimal

    if value is None:
        return {"t": "null"}
    # bool before int: bool is a subclass of int in Python.
    if isinstance(value, bool):
        return {"t": "bool", "v": "true" if value else "false"}
    if isinstance(value, int):
        return {"t": "int", "v": str(value)}
    if isinstance(value, decimal.Decimal):
        return {"t": "decimal", "v": format(value, "f")}
    if isinstance(value, float):
        return {"t": "float", "v": repr(value)}
    if isinstance(value, datetime.datetime):
        return {"t": "timestamp", "v": value.isoformat()}
    if isinstance(value, datetime.date):
        return {"t": "date", "v": value.isoformat()}
    if isinstance(value, str):
        return {"t": "text", "v": value}
    return {"t": "text", "v": str(value)}


def _tagged_rows(table):
    """Convert a pyarrow.Table into tagged rows, in column order."""
    columns = table.column_names
    return [
        [_tag_cell(row[name]) for name in columns]
        for row in table.to_pylist()
    ]


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
            if "exec" in request:
                verb, sql = "exec", request["exec"]
            else:
                verb, sql = "sql", request["sql"]
        except Exception as exc:  # noqa: BLE001
            _emit({"error": f"malformed request: {exc}"})
            continue

        try:
            if verb == "exec":
                _emit({"rows": _tagged_rows(adapter.execute_sql(sql))})
            else:
                _emit({"columns": adapter.dry_run_schema(sql)})
        except Exception as exc:  # noqa: BLE001 — a refusal is an expected reply
            _emit({"error": str(exc)})

    return 0


if __name__ == "__main__":
    sys.exit(main())
