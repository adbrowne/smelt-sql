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

/// CREATE OR REPLACE MATERIALIZED VIEW ... AS SELECT — the `refresh:
/// materialized_view` delegation target (`docs/specs/materialized_view.md`).
///
/// **Measured**, not guessed — `scripts/bigquery-probe-mv.sh`
/// (`docs/research/20260816-bigquery-backend.md` §"Materialized views"):
///
/// - `OR REPLACE` genuinely *replaces*: re-running the identical definition
///   is accepted, and swapping the aggregation itself (`SUM` → `COUNT(*)`)
///   changes the value the view serves. So one idempotent statement covers
///   both a plain re-run and a definition change — no drop-then-create
///   emulation, unlike `create_table_as`'s reason for using `CREATE OR
///   REPLACE TABLE` on other grounds. A plain `CREATE MATERIALIZED VIEW`
///   over an existing view fails `Already Exists`, which is why `OR
///   REPLACE` is the form emitted here.
/// - No `OPTIONS(...)` clause is emitted. `enable_refresh`,
///   `refresh_interval_minutes` and `max_staleness` are all accepted, but
///   refresh is **on by default**, so omitting the clause already gets the
///   engine-owned freshness this mode exists for. Emitting a knob here
///   would pre-empt the per-engine physical-strategy modifier that
///   `docs/specs/materialized_view.md` §Known Divergences deliberately
///   defers to a later mode.
pub fn create_materialized_view_as(view_name: &str, query: &str) -> String {
    format!(
        "CREATE OR REPLACE MATERIALIZED VIEW {} AS {}",
        view_name, query
    )
}

/// DROP MATERIALIZED VIEW IF EXISTS
///
/// **Measured** (`scripts/bigquery-probe-mv.sh`,
/// `docs/research/20260816-bigquery-backend.md` §"Materialized views"):
/// dropping the base *table* out from under a live materialized view is
/// accepted, so teardown needs no ordering — but `DROP TABLE IF EXISTS` and
/// `DROP VIEW IF EXISTS` both *fail* against a materialized view (`Cannot
/// drop ... which has type MATERIALIZED_VIEW. A table was expected.`);
/// `IF EXISTS` does not rescue a wrong-type object, because the object does
/// exist. That failure is new with this feature — no materialized view
/// could exist before it — so cleaning one up now genuinely needs this
/// statement: `BigQueryBackend::execute_model` issues it before delegating
/// to the ordinary table/view drop-and-create path, and
/// `create_materialized_view_as` implicitly needs no such call itself
/// (`CREATE OR REPLACE MATERIALIZED VIEW` handles that case per the "OR
/// REPLACE" note above).
pub fn drop_materialized_view(view_name: &str) -> String {
    format!("DROP MATERIALIZED VIEW IF EXISTS {}", view_name)
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

/// Whether a BigQuery DDL failure means "the name exists, but as the wrong
/// kind of object" — the shape that a defensive drop-or-replace must treat
/// as *absent for its target kind*, not as a real failure.
///
/// Three call sites rely on this, all catalog-type collisions that only
/// exist because `refresh: materialized_view` lets a table, a view, and a
/// materialized view now share one logical model name across runs:
///
/// - `drop_view_if_exists` / `drop_table_if_exists`: the default
///   `Backend::execute_model` (`crates/smelt-backend/src/lib.rs`)
///   unconditionally issues both "in case the materialization type
///   changed". On DuckDB and Spark, `DROP VIEW IF EXISTS` against an
///   existing TABLE (or the reverse) is a no-op, honouring the `IF EXISTS`
///   contract; BigQuery instead raises a hard `400`. Measured live
///   2026-08-18 (`docs/specs/multi_backend.md` §Known Divergences):
///   `400 Cannot drop <project>:<dataset>.recipe_additive_agg which has
///   type TABLE. A view was expected.`
/// - `BigQueryBackend::create_materialized_view_as`'s defensive
///   drop-table/drop-view-first step (forward flip *into*
///   `materialized_view`): `CREATE OR REPLACE MATERIALIZED VIEW` is refused
///   outright when a TABLE already holds the name (measured via
///   `scripts/bigquery-probe-mv.sh`,
///   `docs/research/20260816-bigquery-backend.md` §"Materialized views":
///   `... is not allowed for this operation because it is currently a
///   TABLE.`), so the emitter drops first — but if the *existing* object is
///   itself already a materialized view, that defensive drop hits the next
///   bullet's failure instead, which must also be tolerated here (`CREATE
///   OR REPLACE MATERIALIZED VIEW` handles the actual replacement).
/// - `BigQueryBackend::execute_model`'s defensive
///   drop-materialized-view-first step (reverse flip *out of*
///   `materialized_view`): `DROP TABLE IF EXISTS` / `DROP VIEW IF EXISTS`
///   against an existing materialized view both fail — measured:
///   `Cannot drop ... which has type MATERIALIZED_VIEW. A table was
///   expected.` `IF EXISTS` does not rescue a wrong-type object, because
///   the object does exist.
///
/// This is an allow-list of verified (or, where noted, directly symmetric
/// but not yet independently observed) shapes, not a deny-list — the same
/// discipline `classify_bq_error` in `smelt-maintenance-testkit` uses for
/// the quota-refusal shape (`docs/specs/multi_backend.md` §"Measured
/// against the live warehouse"), and for the identical reason: a classifier
/// built as "not a 400 I recognise ⇒ treat as absent" would swallow
/// unrelated failures (bad SQL, missing dataset, permission errors) instead
/// of failing loud on them.
pub fn is_wrong_type_drop_failure(error_message: &str) -> bool {
    const WRONG_KIND_SHAPES: &[&str] = &[
        // Measured live 2026-08-18 — DROP VIEW IF EXISTS against a TABLE.
        "which has type TABLE. A view was expected",
        // Symmetric reverse (DROP TABLE IF EXISTS against a VIEW); not
        // independently observed, included defensively.
        "which has type VIEW. A table was expected",
        // Measured via scripts/bigquery-probe-mv.sh — DROP TABLE IF EXISTS
        // (and, symmetrically, DROP VIEW IF EXISTS) against a materialized
        // view.
        "which has type MATERIALIZED_VIEW. A table was expected",
        "which has type MATERIALIZED_VIEW. A view was expected",
        // Symmetric reverse (DROP MATERIALIZED VIEW IF EXISTS against an
        // ordinary table/view); not independently observed, included
        // defensively for `execute_model`'s unconditional defensive drop.
        "which has type TABLE. A materialized view was expected",
        "which has type VIEW. A materialized view was expected",
        // Measured via scripts/bigquery-probe-mv.sh — CREATE OR REPLACE
        // MATERIALIZED VIEW refused because the name is currently a TABLE.
        "is not allowed for this operation because it is currently a TABLE",
        // Symmetric reverse (currently a VIEW); not independently observed,
        // included defensively.
        "is not allowed for this operation because it is currently a VIEW",
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

    /// Pins the measured `CREATE OR REPLACE MATERIALIZED VIEW` form
    /// (`docs/specs/materialized_view.md`, `scripts/bigquery-probe-mv.sh`).
    #[test]
    fn create_materialized_view_as_uses_or_replace_no_options() {
        assert_eq!(
            create_materialized_view_as("`p.d.mv`", "SELECT 1"),
            "CREATE OR REPLACE MATERIALIZED VIEW `p.d.mv` AS SELECT 1"
        );
    }

    /// Pins the `DROP MATERIALIZED VIEW IF EXISTS` form used by
    /// `BigQueryBackend::execute_model`'s reverse-flip cleanup.
    #[test]
    fn drop_materialized_view_emits_measured_form() {
        assert_eq!(
            drop_materialized_view("`p.d.mv`"),
            "DROP MATERIALIZED VIEW IF EXISTS `p.d.mv`"
        );
    }

    /// The exact error text observed live 2026-08-18 against a real
    /// warehouse (`docs/specs/multi_backend.md` §Known Divergences) must
    /// classify as "wrong object type" — this is the case that was
    /// crashing `column_add_between_runs_recovers_equivalence_on_bigquery`
    /// and `full_refresh_interleave_resets_state_correctly_on_bigquery`.
    #[test]
    fn wrong_type_matches_live_drop_view_against_table_error() {
        let msg = "400 Cannot drop project:dataset.recipe_additive_agg which has type TABLE. \
                    A view was expected.";
        assert!(is_wrong_type_drop_failure(msg));
    }

    /// The symmetric case: `DROP TABLE IF EXISTS` against an existing VIEW.
    /// Not yet observed live, but the same BigQuery error family with the
    /// object kinds swapped — included defensively.
    #[test]
    fn wrong_type_matches_drop_table_against_view_error() {
        let msg = "400 Cannot drop project:dataset.some_view which has type VIEW. \
                    A table was expected.";
        assert!(is_wrong_type_drop_failure(msg));
    }

    /// Measured via `scripts/bigquery-probe-mv.sh`
    /// (`docs/research/20260816-bigquery-backend.md` §"Materialized
    /// views"): `DROP TABLE IF EXISTS` against an existing materialized
    /// view.
    #[test]
    fn wrong_type_matches_drop_table_against_materialized_view_error() {
        let msg = "400 Cannot drop project:dataset.mv_model which has type \
                    MATERIALIZED_VIEW. A table was expected.";
        assert!(is_wrong_type_drop_failure(msg));
    }

    /// Symmetric to the above: `DROP VIEW IF EXISTS` against an existing
    /// materialized view. Not independently observed, included
    /// defensively.
    #[test]
    fn wrong_type_matches_drop_view_against_materialized_view_error() {
        let msg = "400 Cannot drop project:dataset.mv_model which has type \
                    MATERIALIZED_VIEW. A view was expected.";
        assert!(is_wrong_type_drop_failure(msg));
    }

    /// Measured via `scripts/bigquery-probe-mv.sh`: `CREATE OR REPLACE
    /// MATERIALIZED VIEW` refused because the name currently holds a TABLE.
    #[test]
    fn wrong_type_matches_create_materialized_view_over_table_error() {
        let msg = "400 CREATE OR REPLACE MATERIALIZED VIEW is not allowed for this operation \
                    because it is currently a TABLE.";
        assert!(is_wrong_type_drop_failure(msg));
    }

    /// An unrelated 400 (bad SQL) must NOT be classified as "wrong object
    /// type" — swallowing this would violate fail-loud discipline
    /// (CLAUDE.md §"Fail-loud discipline").
    #[test]
    fn wrong_type_does_not_match_unrelated_bad_request() {
        let msg = "400 Syntax error: Unexpected keyword FORM at [1:15]";
        assert!(!is_wrong_type_drop_failure(msg));
    }

    /// A 403 permission-denied error must NOT be classified as "wrong
    /// object type" either — a different failure family entirely.
    #[test]
    fn wrong_type_does_not_match_permission_denied() {
        let msg = "403 Access Denied: Dataset project:dataset: User does not have \
                    bigquery.tables.delete permission";
        assert!(!is_wrong_type_drop_failure(msg));
    }

    /// A quota-refusal error (a different classified shape, owned by
    /// `smelt-maintenance-testkit::classify_bq_error`) must NOT match here
    /// either — the two classifiers are disjoint by design.
    #[test]
    fn wrong_type_does_not_match_quota_refusal() {
        let msg = "400 Exceeded rate limits: too many table update operations for this \
                    table. exceeded quota for table update operations";
        assert!(!is_wrong_type_drop_failure(msg));
    }

    /// A generic "not found" error must NOT match either — the classifier
    /// is specifically about a *wrong-type* collision, not any DDL failure.
    #[test]
    fn wrong_type_does_not_match_not_found() {
        let msg = "404 Not found: Table project:dataset.does_not_exist";
        assert!(!is_wrong_type_drop_failure(msg));
    }
}
