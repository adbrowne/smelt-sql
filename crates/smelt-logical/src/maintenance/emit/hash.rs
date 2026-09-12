//! Dialect-dispatched hash-function spelling — the single owner of every
//! `sha256`/`SHA256`/`sha2` occurrence under `src/maintenance/`
//! (`maintenance_dialect_blindness.rs`'s `hash_spelling_has_one_owner` gate).
//! Every other file in this module builds its fingerprint SQL by calling
//! into [`hash_digest_expr`] or [`hash_hex_expr`] rather than spelling a hash
//! function itself.
//!
//! Spark/Databricks has no `sha256` function at all — only `sha2(expr,
//! bits)`, which takes an explicit output bit-length DuckDB's and BigQuery's
//! spellings don't need (`docs/outcomes/20260912-databricks-dogfood-spine/
//! outcome.md` phase 6's finding: the first live Databricks full refresh
//! failed every model whose maintenance statement reached this spelling).

use super::types::MaintenanceDialect;

/// The inner, digest-typed hash expression: DuckDB's and BigQuery's
/// `sha256(expr)` (BigQuery accepts the lowercase spelling; kept as-is
/// rather than cased to `SHA256` so nesting stays byte-identical to the
/// spelling already proven live), and Spark's `sha2(expr, 256)`.
pub(crate) fn hash_digest_expr(expr: &str, dialect: MaintenanceDialect) -> String {
    match dialect {
        MaintenanceDialect::DuckDb => format!("sha256({expr})"),
        MaintenanceDialect::Spark => format!("sha2({expr}, 256)"),
        MaintenanceDialect::BigQuery => format!("sha256({expr})"),
    }
}

/// The outer, STRING-returning hash expression: identical to
/// [`hash_digest_expr`] on DuckDB and Spark (both already return a hex
/// string), but BigQuery's `SHA256` returns `BYTES`, so it is additionally
/// `TO_HEX`-wrapped to stay a STRING like every other dialect's — the same
/// `TO_HEX(SHA256(...))` shape already proven live against BigQuery.
pub(crate) fn hash_hex_expr(expr: &str, dialect: MaintenanceDialect) -> String {
    match dialect {
        MaintenanceDialect::BigQuery => format!("TO_HEX(SHA256({expr}))"),
        _ => hash_digest_expr(expr, dialect),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_expr_dispatches_by_dialect() {
        assert_eq!(
            hash_digest_expr("x", MaintenanceDialect::DuckDb),
            "sha256(x)"
        );
        assert_eq!(
            hash_digest_expr("x", MaintenanceDialect::Spark),
            "sha2(x, 256)"
        );
        assert_eq!(
            hash_digest_expr("x", MaintenanceDialect::BigQuery),
            "sha256(x)"
        );
    }

    #[test]
    fn hex_expr_dispatches_by_dialect() {
        assert_eq!(hash_hex_expr("x", MaintenanceDialect::DuckDb), "sha256(x)");
        assert_eq!(
            hash_hex_expr("x", MaintenanceDialect::Spark),
            "sha2(x, 256)"
        );
        assert_eq!(
            hash_hex_expr("x", MaintenanceDialect::BigQuery),
            "TO_HEX(SHA256(x))"
        );
    }
}
