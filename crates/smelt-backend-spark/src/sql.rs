//! Pure SQL generation functions for the Spark backend.
//!
//! All SQL strings sent to Spark are built here, making them
//! independently testable without a Python runtime or Spark cluster.

use smelt_backend::{PartitionRange, PartitionSpec};

/// Build a fully qualified table name: catalog.schema.table
pub fn qualified_name(catalog: &str, schema: &str, name: &str) -> String {
    format!("{}.{}.{}", catalog, schema, name)
}

/// DROP TABLE IF EXISTS
pub fn drop_table(table_name: &str) -> String {
    format!("DROP TABLE IF EXISTS {}", table_name)
}

/// DROP VIEW IF EXISTS
pub fn drop_view(view_name: &str) -> String {
    format!("DROP VIEW IF EXISTS {}", view_name)
}

/// CREATE TABLE [USING format] ... AS SELECT
///
/// When `using` is `Some("DELTA")`, emits `CREATE TABLE ... USING DELTA AS SELECT`
/// which enables DELETE / MERGE / schema-evolution on Spark.
/// When `None`, omits the USING clause (plain managed Parquet for cross-engine path).
pub fn create_table_as(table_name: &str, query: &str, using: Option<&str>) -> String {
    match using {
        Some(fmt) => format!("CREATE TABLE {} USING {} AS {}", table_name, fmt, query),
        None => format!("CREATE TABLE {} AS {}", table_name, query),
    }
}

/// CREATE OR REPLACE VIEW ... AS SELECT
pub fn create_view_as(view_name: &str, query: &str) -> String {
    format!("CREATE OR REPLACE VIEW {} AS {}", view_name, query)
}

/// CREATE DATABASE IF NOT EXISTS catalog.schema
pub fn create_database(catalog: &str, schema: &str) -> String {
    format!("CREATE DATABASE IF NOT EXISTS {}.{}", catalog, schema)
}

/// SELECT * FROM table LIMIT n
pub fn select_preview(table_name: &str, limit: usize) -> String {
    format!("SELECT * FROM {} LIMIT {}", table_name, limit)
}

/// DELETE FROM table WHERE partition_col IN (values)
///
/// Kept for internal use (e.g., tests). The primary runtime path now uses
/// `delete_partitions_range` which emits a range predicate.
#[allow(dead_code)]
pub fn delete_partitions(table_name: &str, partition: &PartitionSpec) -> String {
    let values_list = partition
        .values
        .iter()
        .map(|v| format!("'{}'", v.replace("'", "''")))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "DELETE FROM {} WHERE {} IN ({})",
        table_name, partition.column, values_list
    )
}

/// DELETE FROM table WHERE partition_col >= start AND partition_col < end
pub fn delete_partitions_range(table_name: &str, partition: &PartitionRange) -> String {
    format!(
        "DELETE FROM {} WHERE {} >= '{}' AND {} < '{}'",
        table_name,
        partition.column,
        partition.start.replace('\'', "''"),
        partition.column,
        partition.end.replace('\'', "''"),
    )
}

/// INSERT INTO table (query)
pub fn insert_into(table_name: &str, query: &str) -> String {
    format!("INSERT INTO {} {}", table_name, query)
}

/// MERGE INTO ... USING ... ON ... WHEN MATCHED/NOT MATCHED
pub fn merge_into(table_name: &str, source_sql: &str, unique_key: &[String]) -> String {
    let on_clause = unique_key
        .iter()
        .map(|k| format!("target.{} = source.{}", k, k))
        .collect::<Vec<_>>()
        .join(" AND ");

    format!(
        "MERGE INTO {} AS target USING ({}) AS source ON {} \
         WHEN MATCHED THEN UPDATE SET * \
         WHEN NOT MATCHED THEN INSERT *",
        table_name, source_sql, on_clause
    )
}

/// INSERT OVERWRITE TABLE ... PARTITION (col) (query)
pub fn insert_overwrite(table_name: &str, query: &str, partition: &PartitionRange) -> String {
    format!(
        "INSERT OVERWRITE TABLE {} PARTITION ({}) {}",
        table_name, partition.column, query
    )
}

/// Truncate SQL for logging (first 200 chars).
pub fn truncate_sql(sql: &str) -> String {
    if sql.len() > 200 {
        format!("{}...", &sql[..200])
    } else {
        sql.to_string()
    }
}
