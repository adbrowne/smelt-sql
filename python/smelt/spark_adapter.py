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

    def __init__(self, connect_url, catalog=None):
        from pyspark.sql import SparkSession

        builder = SparkSession.builder
        if connect_url:
            builder = builder.remote(connect_url)
        self.spark = builder.getOrCreate()

        if catalog:
            self.spark.catalog.setCurrentCatalog(catalog)

    def select_current_schema(self, schema):
        """Select the current database/schema.

        Called by SparkBackend::new() after ensure_schema() has created the schema,
        so the schema is guaranteed to exist before setCurrentDatabase is called.
        """
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

    def load_arrow_table(self, parquet_path, full_table_name):
        """Load a Parquet file written from Arrow batches into a Spark table.

        Drops any existing table with the same name first, then reads the
        Parquet file and saves as a managed table.

        Args:
            parquet_path: Local filesystem path to the Parquet file.
            full_table_name: Fully-qualified table name, e.g. "catalog.schema.table".
        """
        # Drop the existing table if present.
        if self.spark.catalog.tableExists(full_table_name):
            self.spark.sql(f"DROP TABLE IF EXISTS {full_table_name}")

        # Read the Parquet file and persist as a managed Spark table.
        df = self.spark.read.parquet(parquet_path)
        df.write.saveAsTable(full_table_name)

    def close(self):
        """Stop the Spark session."""
        self.spark.stop()
