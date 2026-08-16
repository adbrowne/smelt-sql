//! Pure SQL generation functions for the BigQuery backend.
//!
//! All SQL strings sent to BigQuery are built here, making them independently
//! testable without a Python runtime or a live warehouse.

use smelt_backend::PartitionRange;

/// Build a fully qualified, backtick-quoted table name: `` `project.dataset.table` ``.
///
/// GoogleSQL quotes identifiers with backticks, and a project id routinely
/// contains hyphens (`smelt-bq-test-20260816`), which are otherwise parsed as
/// subtraction — so the quoting is required, not cosmetic.
pub fn qualified_name(project: &str, dataset: &str, name: &str) -> String {
    format!("`{}.{}.{}`", project, dataset, name)
}

/// DROP TABLE IF EXISTS
pub fn drop_table(table_name: &str) -> String {
    format!("DROP TABLE IF EXISTS {}", table_name)
}

/// DROP VIEW IF EXISTS
pub fn drop_view(view_name: &str) -> String {
    format!("DROP VIEW IF EXISTS {}", view_name)
}

/// CREATE OR REPLACE TABLE ... AS SELECT
///
/// BigQuery supports `CREATE OR REPLACE TABLE` natively, so no
/// DROP-then-CREATE emulation is needed (unlike Spark).
pub fn create_table_as(table_name: &str, query: &str) -> String {
    format!("CREATE OR REPLACE TABLE {} AS {}", table_name, query)
}

/// CREATE OR REPLACE VIEW ... AS SELECT
pub fn create_view_as(view_name: &str, query: &str) -> String {
    format!("CREATE OR REPLACE VIEW {} AS {}", view_name, query)
}

/// SELECT * FROM table LIMIT n
pub fn select_preview(table_name: &str, limit: usize) -> String {
    format!("SELECT * FROM {} LIMIT {}", table_name, limit)
}

/// INSERT INTO table SELECT ...
pub fn insert_into(table_name: &str, query: &str) -> String {
    format!("INSERT INTO {} {}", table_name, query)
}

/// DELETE over a half-open partition range `[start, end)`.
pub fn delete_partitions_range(table_name: &str, partition: &PartitionRange) -> String {
    format!(
        "DELETE FROM {} WHERE {} >= '{}' AND {} < '{}'",
        table_name,
        partition.column,
        partition.start.replace('\'', "''"),
        partition.column,
        partition.end.replace('\'', "''")
    )
}

/// Truncate SQL for log output.
pub fn truncate_sql(sql: &str) -> String {
    const MAX: usize = 200;
    if sql.len() <= MAX {
        sql.to_string()
    } else {
        format!("{}...", &sql[..MAX])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hyphenated project id must survive quoting — unquoted it parses as
    /// subtraction and the statement fails.
    #[test]
    fn qualified_name_backticks_hyphenated_project() {
        assert_eq!(
            qualified_name("smelt-bq-test-20260816", "smelt_test", "orders"),
            "`smelt-bq-test-20260816.smelt_test.orders`"
        );
    }

    #[test]
    fn create_table_as_uses_native_or_replace() {
        assert_eq!(
            create_table_as("`p.d.t`", "SELECT 1"),
            "CREATE OR REPLACE TABLE `p.d.t` AS SELECT 1"
        );
    }

    #[test]
    fn create_view_as_uses_or_replace() {
        assert_eq!(
            create_view_as("`p.d.v`", "SELECT 1"),
            "CREATE OR REPLACE VIEW `p.d.v` AS SELECT 1"
        );
    }
}
