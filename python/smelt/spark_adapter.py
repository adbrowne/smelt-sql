"""Thin PySpark adapter for smelt's Spark backend.

This module provides a minimal wrapper around PySpark's SparkSession,
used by the Rust SparkBackend via PyO3 to execute SQL and return Arrow results.

Works with:
- Local Spark (via spark-connect)
- Databricks Connect (pip install databricks-connect)
- Any PySpark-compatible environment
"""

import pyarrow as pa


class SparkAdapter:
    """Wraps a PySpark SparkSession for SQL execution with Arrow results."""

    def __init__(self, connect_url, catalog=None, schema=None):
        from pyspark.sql import SparkSession

        builder = SparkSession.builder
        if connect_url:
            builder = builder.remote(connect_url)
        self.spark = builder.getOrCreate()

        if catalog:
            self.spark.catalog.setCurrentCatalog(catalog)
        if schema:
            self.spark.catalog.setCurrentDatabase(schema)

    def execute_sql(self, sql):
        """Execute SQL and return a pyarrow.Table.

        For DDL statements that return no data, returns an empty table.
        """
        df = self.spark.sql(sql)
        # toArrow() is PySpark 4.0+; fall back for older versions
        if hasattr(df, "toArrow"):
            return df.toArrow()
        elif hasattr(df, "toPandas"):
            # PySpark 3.x: toPandas() uses Arrow internally when enabled
            pandas_df = df.toPandas()
            return pa.Table.from_pandas(pandas_df)
        else:
            raise RuntimeError("PySpark version does not support Arrow conversion")

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

    def close(self):
        """Stop the Spark session."""
        self.spark.stop()
