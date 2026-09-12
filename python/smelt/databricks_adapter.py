"""Thin Databricks Connect adapter for smelt's Databricks backend.

Mirrors `smelt.spark_adapter.SparkAdapter`'s method surface exactly, so the
Rust `SparkBackend` (behind its `SparkFlavor::Databricks` discriminator) can
call either adapter identically. Only session construction differs: this
adapter builds a serverless `DatabricksSession` via Databricks Connect
instead of a `SparkSession.remote(...)` Spark Connect session.

Requires the `databricks-connect` package, which conflicts with plain
`pyspark` — pin it in its own venv, separate from the local-Spark
environment (`scripts/spark-env.sh`).
"""

import io

import pyarrow as pa


class DatabricksAdapter:
    """Wraps a Databricks Connect `DatabricksSession` for SQL execution with Arrow results."""

    def __init__(self, host, catalog=None, token=None):
        from databricks.connect import DatabricksSession

        # Stored verbatim so tests can assert the value reaching the builder
        # unmodified — no smelt-side parsing or rewriting of the host.
        self.host = host

        builder = DatabricksSession.builder.host(host).serverless(True)
        if token:
            builder = builder.token(token)
        # else: ambient credentials — the form a workload running inside the
        # workspace itself takes (`docs/specs/smelt_yml.md` §"Target shape").
        self.spark = builder.getOrCreate()

        if catalog:
            self.spark.catalog.setCurrentCatalog(catalog)

    def select_current_schema(self, schema):
        """Select the current database/schema.

        Called after ensure_schema() has created the schema, so the schema
        is guaranteed to exist before setCurrentDatabase is called.
        """
        self.spark.catalog.setCurrentDatabase(schema)

    def execute_sql(self, sql):
        """Execute SQL and return a pyarrow.Table.

        For DDL statements that return no data, returns an empty table.
        """
        df = self.spark.sql(sql)
        if hasattr(df, "toArrow"):
            return df.toArrow()
        elif hasattr(df, "toPandas"):
            pandas_df = df.toPandas()
            return pa.Table.from_pandas(pandas_df)
        else:
            raise RuntimeError("Databricks Connect version does not support Arrow conversion")

    def execute_sql_no_result(self, sql):
        """Execute SQL without collecting results (for DDL/DML)."""
        self.spark.sql(sql)

    def table_exists(self, full_name):
        """Check if a table exists by fully-qualified name."""
        return self.spark.catalog.tableExists(full_name)

    def get_row_count(self, full_name):
        """Get row count for a table."""
        row = self.spark.sql(f"SELECT COUNT(*) AS cnt FROM {full_name}").collect()
        return row[0]["cnt"]

    def load_arrow_table(self, ipc_bytes, full_table_name):
        """Load Arrow IPC stream bytes into a Unity Catalog table via createDataFrame.

        Rows are sent through the session client — serverless compute shares
        no filesystem with the client at all, so there is no host-path
        fallback to reach for even by mistake
        (`docs/specs/multi_backend.md` §"Loading data into a backend").

        Args:
            ipc_bytes: Arrow IPC stream bytes (bytes object) containing the
                       table data.
            full_table_name: Fully-qualified table name, e.g. "catalog.schema.table".
        """
        reader = pa.ipc.open_stream(io.BytesIO(ipc_bytes))
        table = reader.read_all()

        if self.spark.catalog.tableExists(full_table_name):
            self.spark.sql(f"DROP TABLE IF EXISTS {full_table_name}")

        df = self.spark.createDataFrame(table)
        df.write.saveAsTable(full_table_name)

    def close(self):
        """Stop the Databricks session."""
        self.spark.stop()
