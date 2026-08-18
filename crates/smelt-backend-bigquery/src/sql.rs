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

/// Whether a BigQuery `DROP ... IF EXISTS` failure means "the name exists,
/// but as the other kind of object" — the shape that must be treated as
/// *absent* for the drop's target kind, not as a real failure.
///
/// The default `Backend::execute_model`
/// (`crates/smelt-backend/src/lib.rs`) unconditionally issues both
/// `drop_view_if_exists` and `drop_table_if_exists` for every model "in case
/// the materialization type changed". On DuckDB and Spark a
/// `DROP VIEW IF EXISTS` against an existing TABLE (or the reverse) is a
/// no-op, honouring the `IF EXISTS` contract. BigQuery instead raises a hard
/// `400`, so the BigQuery backend classifies this exact shape and swallows
/// it at the drop call sites — matching the `IF EXISTS` semantics every
/// other backend already gives for free, without silently absorbing any
/// other 400.
///
/// This is an allow-list of verified shapes, not a deny-list — the same
/// discipline `classify_bq_error` in `smelt-maintenance-testkit` uses for
/// the quota-refusal shape (`docs/specs/multi_backend.md` §"Measured
/// against the live warehouse"), and for the identical reason: a classifier
/// built as "not a 400 I recognise ⇒ treat as absent" would swallow
/// unrelated failures (bad SQL, missing dataset, permission errors) instead
/// of failing loud on them.
///
/// Verified error text (`docs/specs/multi_backend.md` §Known Divergences,
/// observed live 2026-08-18):
/// `400 Cannot drop <project>:<dataset>.recipe_additive_agg which has type
/// TABLE. A view was expected.`
///
/// The reverse direction (`DROP TABLE IF EXISTS` against an existing VIEW)
/// is not yet observed live but is the same BigQuery error family with the
/// object kinds swapped, so it is included defensively.
pub fn is_wrong_object_type_for_drop(error_message: &str) -> bool {
    const WRONG_KIND_SHAPES: &[&str] = &[
        "which has type TABLE. A view was expected",
        "which has type VIEW. A table was expected",
    ];
    WRONG_KIND_SHAPES
        .iter()
        .any(|shape| error_message.contains(shape))
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

    /// The exact error text observed live 2026-08-18 against a real
    /// warehouse (`docs/specs/multi_backend.md` §Known Divergences) must
    /// classify as "wrong object type" — this is the case that was
    /// crashing `column_add_between_runs_recovers_equivalence_on_bigquery`
    /// and `full_refresh_interleave_resets_state_correctly_on_bigquery`.
    #[test]
    fn wrong_object_type_matches_live_drop_view_against_table_error() {
        let msg = "400 Cannot drop project:dataset.recipe_additive_agg which has type TABLE. \
                    A view was expected.";
        assert!(is_wrong_object_type_for_drop(msg));
    }

    /// The symmetric case: `DROP TABLE IF EXISTS` against an existing VIEW.
    /// Not yet observed live, but the same BigQuery error family with the
    /// object kinds swapped — included defensively.
    #[test]
    fn wrong_object_type_matches_drop_table_against_view_error() {
        let msg = "400 Cannot drop project:dataset.some_view which has type VIEW. \
                    A table was expected.";
        assert!(is_wrong_object_type_for_drop(msg));
    }

    /// An unrelated 400 (bad SQL) must NOT be classified as "wrong object
    /// type" — swallowing this would violate fail-loud discipline
    /// (CLAUDE.md §"Fail-loud discipline").
    #[test]
    fn wrong_object_type_does_not_match_unrelated_bad_request() {
        let msg = "400 Syntax error: Unexpected keyword FORM at [1:15]";
        assert!(!is_wrong_object_type_for_drop(msg));
    }

    /// A 403 permission-denied error must NOT be classified as "wrong
    /// object type" either — a different failure family entirely.
    #[test]
    fn wrong_object_type_does_not_match_permission_denied() {
        let msg = "403 Access Denied: Dataset project:dataset: User does not have \
                    bigquery.tables.delete permission";
        assert!(!is_wrong_object_type_for_drop(msg));
    }

    /// A quota-refusal error (a different classified shape, owned by
    /// `smelt-maintenance-testkit::classify_bq_error`) must NOT match here
    /// either — the two classifiers are disjoint by design.
    #[test]
    fn wrong_object_type_does_not_match_quota_refusal() {
        let msg = "400 Exceeded rate limits: too many table update operations for this \
                    table. exceeded quota for table update operations";
        assert!(!is_wrong_object_type_for_drop(msg));
    }
}
