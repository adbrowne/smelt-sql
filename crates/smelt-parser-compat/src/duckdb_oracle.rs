//! DuckDB execution oracle for differential parsing.
//!
//! This module answers one question: *does DuckDB accept this SQL?* It is the
//! ground truth for the two-directional dialect-conformance gates (architecture
//! spec §Constraints #13):
//!
//! - **Accept direction** — DuckDB accepts ⇒ smelt must parse cleanly or the
//!   statement must be a registered gap.
//! - **Fidelity direction** — smelt parses cleanly ⇒ the SQL smelt prints back
//!   must still be accepted (and, where possible, executed) by DuckDB. This
//!   closes the silent-mis-parse class: a query that "parses" into the wrong
//!   tree prints to SQL DuckDB rejects or that means something else.
//!
//! The oracle runs against an in-memory DuckDB with a canned schema prelude so
//! that column/table references in the corpus resolve.

use duckdb::Connection;

/// A canned schema prelude covering the tables and columns referenced by the
/// seed corpus. Every corpus statement is written against these objects so the
/// oracle can bind names during `PREPARE`.
pub const SCHEMA_PRELUDE: &str = "\
CREATE TABLE t (
    a INTEGER,
    b VARCHAR,
    c DOUBLE,
    d DATE,
    ts TIMESTAMP
);
INSERT INTO t VALUES (1, 'hello', 1.5, DATE '2020-01-01', TIMESTAMP '2020-01-01 00:00:00');
";

/// Open a fresh in-memory DuckDB connection with the schema prelude applied.
fn connect_with_prelude(schema_prelude: &str) -> Result<Connection, String> {
    let conn = Connection::open_in_memory().map_err(|e| format!("open in-memory DuckDB: {e}"))?;
    if !schema_prelude.trim().is_empty() {
        conn.execute_batch(schema_prelude)
            .map_err(|e| format!("schema prelude failed: {e}"))?;
    }
    Ok(conn)
}

/// Return `Ok(())` if DuckDB accepts `sql` (prepares without error), else an
/// error string describing the rejection.
///
/// This uses `PREPARE`, which fully parses, binds, and type-checks the
/// statement without executing it — strong enough to reject unknown syntax,
/// unknown columns, and type errors, but cheap.
pub fn duckdb_accepts(sql: &str, schema_prelude: &str) -> Result<(), String> {
    let conn = connect_with_prelude(schema_prelude)?;
    conn.prepare(sql)
        .map(|_| ())
        .map_err(|e| format!("DuckDB rejected: {e}"))
}

/// Return `Ok(())` if DuckDB accepts *and successfully executes* `sql`.
///
/// Stronger than [`duckdb_accepts`]: it runs the statement to completion,
/// catching binder/runtime errors a bare `PREPARE` might defer. Used by the
/// fidelity direction so printed SQL is proven evaluable, not merely parseable.
pub fn duckdb_executes(sql: &str, schema_prelude: &str) -> Result<(), String> {
    let conn = connect_with_prelude(schema_prelude)?;
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| format!("DuckDB rejected (prepare): {e}"))?;
    // Drain the result set so binder/runtime errors surface.
    let mut rows = stmt
        .query([])
        .map_err(|e| format!("DuckDB rejected (execute): {e}"))?;
    while rows
        .next()
        .map_err(|e| format!("DuckDB rejected (row): {e}"))?
        .is_some()
    {}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_select() {
        assert!(duckdb_accepts("SELECT a FROM t", SCHEMA_PRELUDE).is_ok());
    }

    #[test]
    fn rejects_unknown_column() {
        assert!(duckdb_accepts("SELECT nope FROM t", SCHEMA_PRELUDE).is_err());
    }

    #[test]
    fn rejects_garbage_syntax() {
        assert!(duckdb_accepts("SELECT a FROM t GARBAGE MORE", SCHEMA_PRELUDE).is_err());
    }

    #[test]
    fn executes_plain_select() {
        assert!(duckdb_executes("SELECT a FROM t", SCHEMA_PRELUDE).is_ok());
    }

    #[test]
    fn accepts_duckdb_idioms() {
        // Sanity: DuckDB really does accept the idioms the corpus asserts it accepts.
        assert!(duckdb_accepts("SELECT TRY_CAST(b AS INTEGER) FROM t", SCHEMA_PRELUDE).is_ok());
        assert!(duckdb_accepts("SELECT a, count(*) FROM t GROUP BY ALL", SCHEMA_PRELUDE).is_ok());
        assert!(duckdb_accepts("SELECT a FROM t ORDER BY ALL", SCHEMA_PRELUDE).is_ok());
    }
}
