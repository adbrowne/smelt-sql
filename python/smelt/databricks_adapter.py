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

    def __init__(self, host=None, catalog=None, token=None):
        # PyO3's embedded interpreter reaches this venv's site-packages via a
        # bare PYTHONPATH entry (`scripts/dbx-dogfood-env.sh`), not real venv
        # activation, so the venv's own `distutils-precedence.pth` (which
        # shims `import distutils` onto `setuptools._distutils` on Python
        # 3.12, where stdlib `distutils` no longer exists) is never processed
        # — `.pth` files only run for directories the `site` module itself
        # registers, not ones appended via `sys.path`/`PYTHONPATH`.
        # `databricks-connect` (via `pyspark`) imports `distutils` internally,
        # so without this the session builder fails outright with
        # `ModuleNotFoundError: No module named 'distutils'`. A no-op wherever
        # the shim already ran (real venv activation, an older Python with a
        # native `distutils`, or no `setuptools` present at all).
        try:
            import _distutils_hack

            _distutils_hack.add_shim()
        except ImportError:
            pass

        from databricks.connect import DatabricksSession

        # Stored verbatim so tests can assert the value reaching the builder
        # unmodified — no smelt-side parsing or rewriting of the host.
        self.host = host

        builder = DatabricksSession.builder
        if host:
            # `databricks.connect.session`'s builder routes to an entirely
            # different branch the instant *any* of `.serverless(...)`,
            # `.host(...)` or `.token(...)` is called — that branch always
            # builds its own `Config(host=..., token=..., profile=...)` and
            # never consults `SPARK_REMOTE` or a notebook's ambient session.
            # So `.serverless(True)` is only safe to call alongside an
            # explicit host: it never reaches the ambient ladder anyway, and
            # Free Edition is serverless-only.
            builder = builder.serverless(True).host(host)
        # else: no explicit host at all — the ambient form. Calling *no*
        # builder method here is load-bearing, not merely simpler: a
        # `.serverless(True)` call still present when `host` is absent (as
        # this adapter did through phase 11d) forces the same
        # explicit-`Config()` branch and skips the ambient ladder
        # (`_try_get_notebook_session()` then `SPARK_REMOTE`) — which is
        # exactly the channel a serverless job task's own Connect session
        # already established (`docs/specs/multi_backend.md`
        # §"Connection security"; measured live, phase 11e).
        if token:
            builder = builder.token(token)
        # else: ambient credentials — the form a workload running inside the
        # workspace itself takes (`docs/specs/smelt_yml.md` §"Target shape").
        self.spark = builder.getOrCreate()

        # No `setCurrentCatalog` call: every statement this adapter's callers
        # issue is already catalog-and-schema-qualified
        # (`crates/smelt-backend-spark/src/sql.rs::qualified_name`, called at
        # every DDL/DML call site), so nothing depends on an implicit current
        # catalog — which matters because `spark.catalog.*` is not safe to
        # call on the ambient (notebook-borrowed) session at all: measured
        # live (phase 11e), that session runs an ordinary `.sql(...)` query
        # fine but raises `[NO_ACTIVE_SESSION] No active Spark session
        # found` on every `Catalog` RPC (`setCurrentCatalog`,
        # `setCurrentDatabase`, `tableExists` alike). `table_exists` below
        # uses a plain `information_schema` query instead, for the same
        # reason, on both the ambient and explicit-host paths.

    def select_current_schema(self, schema):
        """No-op — see `__init__`'s comment on why this adapter never calls
        `spark.catalog.setCurrentDatabase`. Kept so `SparkBackend` can call
        it identically on either flavor
        (`crates/smelt-backend-spark/src/lib.rs`)."""

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
        """Check if a table exists by fully-qualified name.

        A plain `information_schema` query rather than
        `spark.catalog.tableExists` — see `__init__`'s comment: every
        `Catalog` RPC raises `NO_ACTIVE_SESSION` on the ambient
        (notebook-borrowed) session, while an ordinary `.sql(...)` query
        does not.
        """
        catalog, schema, table = full_name.split(".")
        row = self.spark.sql(
            f"SELECT count(*) AS c FROM {catalog}.information_schema.tables "
            f"WHERE table_schema = '{schema}' AND table_name = '{table}'"
        ).collect()
        return row[0]["c"] > 0

    def get_row_count(self, full_name):
        """Get row count for a table."""
        row = self.spark.sql(f"SELECT COUNT(*) AS cnt FROM {full_name}").collect()
        return row[0]["cnt"]

    def load_arrow_table(self, ipc_bytes, full_table_name, mode="overwrite"):
        """Load Arrow IPC stream bytes into a Unity Catalog table via createDataFrame.

        Rows are sent through the session client — serverless compute shares
        no filesystem with the client at all, so there is no host-path
        fallback to reach for even by mistake
        (`docs/specs/multi_backend.md` §"Loading data into a backend").

        Args:
            ipc_bytes: Arrow IPC stream bytes (bytes object) containing the
                       table data.
            full_table_name: Fully-qualified table name, e.g. "catalog.schema.table".
            mode: "overwrite" (default) drops and recreates the table, matching
                  the existing two-positional-argument call site from
                  `SparkBackend`. "append" writes without dropping, for a
                  caller (the dogfood loader) that must accumulate rows across
                  multiple calls rather than losing history on the second one.
        """
        reader = pa.ipc.open_stream(io.BytesIO(ipc_bytes))
        table = reader.read_all()
        # Databricks Connect's createDataFrame (pyspark.sql.connect) has no
        # pyarrow.Table overload — passing one directly makes it iterate the
        # table as row data and fail inferring a schema from field "_1". A
        # pandas DataFrame is accepted directly, so convert here rather than
        # asking every caller to know this quirk.
        df = self.spark.createDataFrame(table.to_pandas())

        if mode == "append":
            df.write.mode("append").saveAsTable(full_table_name)
        elif mode == "overwrite":
            if self.table_exists(full_table_name):
                self.spark.sql(f"DROP TABLE IF EXISTS {full_table_name}")
            df.write.saveAsTable(full_table_name)
        else:
            raise ValueError(f"unsupported load_arrow_table mode: {mode!r}")

    def close(self):
        """Stop the Databricks session — but only one this adapter itself
        built. The ambient form's session is the notebook/job runtime's own
        shared session (`self.spark is` the object `_try_get_notebook_
        session()` returned), not something this adapter owns: measured live
        (phase 11e), calling `.stop()` on it tears the session down for the
        rest of the process, so a second `_connect()` later in the same
        script — exactly what `cmd_next_day()` then `cmd_execute()` do,
        back to back — raises `[NO_ACTIVE_SESSION]` on its very first query.
        """
        if self.host:
            self.spark.stop()
