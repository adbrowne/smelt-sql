"""Thin google-cloud-bigquery adapter for smelt's BigQuery backend.

This module provides a minimal wrapper around a BigQuery client, used by the
Rust BigQueryBackend via PyO3 to execute SQL and return Arrow results. All SQL
generation and orchestration stays in Rust; this layer only holds the
connection and returns record batches.

Authentication is from an explicitly-supplied OAuth access token and *never*
from Google application-default credentials. That is a security property rather
than a convenience: ambient credentials on a developer machine carry that
developer's whole cloud identity, so refusing the fallback makes the supplied
token the only route to the warehouse. Constructing the adapter without a token
raises rather than silently reaching for ADC.
"""

import io

import pyarrow as pa


def _field_to_dict(field):
    """Flatten a SchemaField into plain data, recursing through RECORD fields.

    A RECORD's sub-fields are the only place struct member types appear — the
    parent reports only `RECORD` — so the recursion is load-bearing, not
    cosmetic. Decimal precision/scale are deliberately absent: BigQuery reports
    them as None on a query output schema, so there is nothing to carry.
    """
    return {
        "name": field.name,
        "type": field.field_type,
        "mode": field.mode,
        "fields": [_field_to_dict(child) for child in (field.fields or ())],
    }


class BigQueryAdapter:
    """Wraps a BigQuery client for SQL execution with Arrow results."""

    def __init__(self, project, dataset, location=None, access_token=None):
        from google.cloud import bigquery
        from google.oauth2.credentials import Credentials

        if not access_token:
            raise ValueError(
                "BigQuery requires an explicit OAuth access token "
                "(SMELT_BQ_ACCESS_TOKEN). smelt never falls back to Google "
                "application-default credentials: ADC on a developer machine "
                "carries that developer's entire cloud identity, a far larger "
                "blast radius than the dataset-scoped token this backend expects."
            )

        self.project = project
        self.dataset = dataset
        self.location = location

        # A bare-token credential. It carries no refresh token and no client
        # secret, so it cannot mint a replacement when it expires — expiry
        # surfaces as an auth error the developer resolves by re-running
        # scripts/bigquery-auth.sh, which is the intended human-in-the-loop.
        credentials = Credentials(token=access_token)
        self.client = bigquery.Client(
            project=project, credentials=credentials, location=location
        )

    def select_current_schema(self, schema):
        """Select the dataset unqualified references resolve against.

        BigQuery has no session-level `USE`; the equivalent is the job's
        default dataset, so this records the choice and every subsequent job
        carries it.
        """
        self.dataset = schema

    def _job_config(self):
        from google.cloud import bigquery

        config = bigquery.QueryJobConfig()
        if self.dataset:
            config.default_dataset = f"{self.project}.{self.dataset}"
        return config

    def execute_sql(self, sql):
        """Execute SQL and return a pyarrow.Table.

        For DDL/DML statements that return no rows, returns an empty table.
        """
        job = self.client.query(sql, job_config=self._job_config())
        result = job.result()
        # Script and DDL/DML jobs expose no row schema; to_arrow() on those
        # raises rather than returning empty, so report an empty table.
        if result.schema is None:
            return pa.table({})
        return result.to_arrow()

    def dry_run_schema(self, sql):
        """Return the output schema of `sql` without executing it.

        A dry run carries the full output schema, bills zero bytes, reads no
        table, and still rejects invalid SQL with a 400 — so it answers "what
        type does this expression have?" without materialising anything. The
        query cache is disabled because a cache hit would answer from a prior
        job rather than re-planning this exact text.

        The schema is returned as plain JSON-serialisable data (a list of
        `{"name", "type", "mode", "fields"}` dicts) so callers outside Python —
        the Rust type-oracle harness — need no client objects. Note the
        reported names are BigQuery's *legacy* ones (INT64 reports as INTEGER,
        FLOAT64 as FLOAT, STRUCT as RECORD) and array-ness lives in `mode`
        (`REPEATED`), never in the type name; translation is the caller's job.

        A refusal propagates as an exception. Returning an empty schema instead
        would be a lie: a query BigQuery rejects is not a query with no columns,
        and a caller that could not tell the two apart would silently score
        every rejected query as agreeing with anything.
        """
        config = self._job_config()
        config.dry_run = True
        config.use_query_cache = False

        # No .result() call: a dry-run job is complete when client.query()
        # returns, and the schema is already on it.
        job = self.client.query(sql, job_config=config)
        schema = job.schema
        if schema is None:
            raise ValueError(f"BigQuery dry run reported no output schema for: {sql}")
        return [_field_to_dict(field) for field in schema]

    def execute_sql_no_result(self, sql):
        """Execute SQL without collecting results (for DDL/DML)."""
        self.client.query(sql, job_config=self._job_config()).result()

    def ensure_schema(self, schema):
        """Create the dataset if it does not exist.

        BigQuery refuses any write into a dataset that does not exist (probed:
        `Not found: Dataset ... was not found in location ...`), so this must
        run before the first model executes against a fresh warehouse.
        """
        import os

        from google.cloud import bigquery

        dataset_ref = bigquery.Dataset(f"{self.project}.{schema}")
        if self.location:
            dataset_ref.location = self.location

        # A default table expiration makes orphan cleanup structural rather than a
        # matter of teardown running: a run killed by a panic or Ctrl-C leaves
        # tables that delete themselves. Test harnesses set this; production
        # targets leave it unset and tables persist normally.
        expiration_ms = os.environ.get("SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS")
        if expiration_ms:
            dataset_ref.default_table_expiration_ms = int(expiration_ms)

        self.client.create_dataset(dataset_ref, exists_ok=True)

    def table_exists(self, full_name):
        """Check if a table exists by fully-qualified name."""
        from google.cloud.exceptions import NotFound

        try:
            self.client.get_table(full_name)
            return True
        except NotFound:
            return False

    def get_row_count(self, full_name):
        """Get row count for a table."""
        sql = f"SELECT COUNT(*) AS cnt FROM `{full_name}`"
        for row in self.client.query(sql, job_config=self._job_config()).result():
            return row["cnt"]
        return 0

    def load_arrow_table(self, ipc_bytes, full_table_name):
        """Load Arrow IPC stream bytes into a BigQuery table.

        Rows are uploaded through the client API as Parquet — no host path is
        handed to the server to read back, so this carries no assumption that
        the warehouse shares the host filesystem.
        """
        import pyarrow.parquet as pq
        from google.cloud import bigquery

        reader = pa.ipc.open_stream(io.BytesIO(ipc_bytes))
        table = reader.read_all()

        buffer = io.BytesIO()
        pq.write_table(table, buffer)
        buffer.seek(0)

        job_config = bigquery.LoadJobConfig(
            source_format=bigquery.SourceFormat.PARQUET,
            write_disposition=bigquery.WriteDisposition.WRITE_TRUNCATE,
        )
        self.client.load_table_from_file(
            buffer, full_table_name, job_config=job_config
        ).result()

    def close(self):
        """Release the client."""
        self.client.close()
