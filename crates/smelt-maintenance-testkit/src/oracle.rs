//! Cell `P0-3` (`docs/research/20260705-property-discovery-loop.md` §2.3, §3b;
//! `docs/plans/20260705-property-discovery-loop.md` phase C, acceptance (ii)).
//!
//! The Link-C oracle: two relations (the maintained output and a full-refresh
//! baseline computed over the source's state at step `k`, design N3) are
//! equal iff their `EXCEPT ALL` (multiset) difference is empty in BOTH
//! directions. Plain `EXCEPT` is set-semantics and is blind to multiplicity —
//! it would silently miss an additive combiner (`SUM`/`COUNT`) double-counting
//! a re-delivered delta (cell `G-02`), which is exactly the divergence class
//! this oracle exists to catch (design F2). This module proves the two modes
//! actually differ on a duplicated-row fixture, then exposes the multiset
//! equality check every later Link-C cell diffs against.
//!
//! Column scope (design N2): callers pass `left_sql`/`right_sql` already
//! projected to the diff scope — all columns by default, minus any column
//! that is provably payload AND declared non-deterministic. This module does
//! not decide that scope; it is the mechanical multiset-equality primitive
//! cells build the scoped query on top of.

#![allow(dead_code)]

use duckdb::Connection;
use smelt_backend::Backend;

/// Row count of `(left EXCEPT ALL right)` — multiset difference: a row
/// present in `left` more times than in `right` contributes
/// `count_left - count_right` to this number (when positive). Zero in both
/// directions ⇔ `left` and `right` are equal multisets.
///
/// Kept as a raw-`Connection` convenience alongside
/// [`except_all_row_count_via_backend`] (`docs/plans/20260720-prod-w9-spark-conformance-twin.md`
/// Phase 2) for callers that only ever hold a `duckdb::Connection` — this
/// module's own in-memory unit tests below, and
/// `crates/smelt-cli/tests/property_discovery/*.rs` (a separate,
/// DuckDB-only test suite outside this plan's Spark-twin scope). The
/// maintenance-conformance harness's own real call sites (`gate.rs`,
/// `pinned.rs`) route through the `_via_backend` counterpart instead.
pub fn except_all_row_count(conn: &Connection, left_sql: &str, right_sql: &str) -> i64 {
    let sql = format!("SELECT count(*) FROM (({left_sql}) EXCEPT ALL ({right_sql})) AS d");
    conn.query_row(&sql, [], |row| row.get(0))
        .expect("except all count query")
}

/// Row count of `(left EXCEPT right)` — plain set difference; blind to
/// multiplicity (a row duplicated N times on one side and once on the other
/// contributes 0 here, unlike `except_all_row_count`).
pub fn except_row_count(conn: &Connection, left_sql: &str, right_sql: &str) -> i64 {
    let sql = format!("SELECT count(*) FROM (({left_sql}) EXCEPT ({right_sql})) AS d");
    conn.query_row(&sql, [], |row| row.get(0))
        .expect("except count query")
}

/// The Link-C oracle (design §2.3): `left` and `right` are the same multiset
/// iff `EXCEPT ALL` is empty in both directions. Callers project `left_sql`/
/// `right_sql` to the diff column scope (N2) before calling this.
///
/// Raw-`Connection` convenience wrapper, kept for the same reason
/// [`except_all_row_count`] is — see its doc comment. The harness's own
/// real call sites use [`multiset_equal_via_backend`].
pub fn multiset_equal(conn: &Connection, left_sql: &str, right_sql: &str) -> bool {
    except_all_row_count(conn, left_sql, right_sql) == 0
        && except_all_row_count(conn, right_sql, left_sql) == 0
}

/// [`except_all_row_count`]'s counterpart routed through
/// [`smelt_backend::Backend::execute_sql`] instead of a raw `duckdb::Connection`
/// (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 2) — the
/// same `EXCEPT ALL` SQL text, only the execution channel moved, so this is
/// portable to any `Backend` impl (the DuckDB impl today; a Spark impl once
/// a later phase wires it up). Mirrors
/// `crates/smelt-runtime/tests/statement_parity.rs::except_all_count`'s
/// existing count-extraction pattern. This is the maintenance-conformance
/// harness's real execution channel — `crates/smelt-cli/tests/maintenance_conformance/{gate,pinned}.rs`
/// call it (via [`multiset_equal_via_backend`]) rather than the raw
/// `duckdb::Connection` sibling above.
pub async fn except_all_row_count_via_backend(
    backend: &dyn Backend,
    left_sql: &str,
    right_sql: &str,
) -> anyhow::Result<i64> {
    let sql = format!("SELECT count(*) FROM (({left_sql}) EXCEPT ALL ({right_sql})) AS d");
    let batches = backend
        .execute_sql(&sql)
        .await
        .map_err(|e| anyhow::anyhow!("except all count query: {e}"))?;
    let batch = batches
        .first()
        .ok_or_else(|| anyhow::anyhow!("except all count query returned no batches"))?;
    let counts = batch
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .ok_or_else(|| anyhow::anyhow!("except all count query's count(*) column was not Int64"))?;
    Ok(counts.value(0))
}

/// [`multiset_equal`]'s counterpart routed through
/// [`smelt_backend::Backend::execute_sql`] (Phase 2) — same multiset-equality
/// contract (`EXCEPT ALL` empty in both directions), different execution
/// channel. See [`except_all_row_count_via_backend`]'s doc comment.
pub async fn multiset_equal_via_backend(
    backend: &dyn Backend,
    left_sql: &str,
    right_sql: &str,
) -> anyhow::Result<bool> {
    Ok(
        except_all_row_count_via_backend(backend, left_sql, right_sql).await? == 0
            && except_all_row_count_via_backend(backend, right_sql, left_sql).await? == 0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed_two_tables(conn: &Connection, left_rows: &str, right_rows: &str) {
        conn.execute_batch(&format!(
            r#"
        CREATE TABLE left_t(id BIGINT, val DOUBLE);
        INSERT INTO left_t VALUES {left_rows};
        CREATE TABLE right_t(id BIGINT, val DOUBLE);
        INSERT INTO right_t VALUES {right_rows};
        "#
        ))
        .expect("seed left_t/right_t");
    }

    #[test]
    fn duplicated_identical_row_is_visible_to_except_all_but_not_except() {
        let conn = Connection::open_in_memory().expect("open duckdb");
        // left has row (1, 10.0) TWICE (e.g. a re-delivered delta folded into an
        // additive combiner without a dedup ledger, cell G-02); right has it once.
        seed_two_tables(&conn, "(1, 10.0), (1, 10.0)", "(1, 10.0)");

        let left_sql = "SELECT * FROM left_t";
        let right_sql = "SELECT * FROM right_t";

        assert_eq!(
            except_row_count(&conn, left_sql, right_sql),
            0,
            "plain EXCEPT is set-semantics: a duplicated identical row is invisible \
             to it — this is why the oracle must not use plain EXCEPT (design F2)"
        );
        assert_eq!(
            except_all_row_count(&conn, left_sql, right_sql),
            1,
            "EXCEPT ALL is multiset-semantics: the one extra duplicate surfaces \
             as a single divergent row"
        );
        assert!(
            !multiset_equal(&conn, left_sql, right_sql),
            "the oracle must report divergence for a duplicated row — the exact \
             double-counting shape cell G-02 hunts"
        );
    }

    #[test]
    fn equal_multisets_compare_equal_regardless_of_row_order() {
        let conn = Connection::open_in_memory().expect("open duckdb");
        seed_two_tables(&conn, "(1, 10.0), (2, 20.0)", "(2, 20.0), (1, 10.0)");

        assert!(
            multiset_equal(&conn, "SELECT * FROM left_t", "SELECT * FROM right_t"),
            "same rows in different physical order must compare equal"
        );
    }

    #[test]
    fn unequal_multisets_of_distinct_rows_diverge_in_both_except_forms() {
        let conn = Connection::open_in_memory().expect("open duckdb");
        seed_two_tables(&conn, "(1, 10.0)", "(1, 10.0), (2, 20.0)");

        let left_sql = "SELECT * FROM left_t";
        let right_sql = "SELECT * FROM right_t";

        assert_eq!(except_row_count(&conn, left_sql, right_sql), 0);
        assert_eq!(
            except_row_count(&conn, right_sql, left_sql),
            1,
            "the extra (2, 20.0) row is a genuine set difference, visible to plain EXCEPT too"
        );
        assert!(!multiset_equal(&conn, left_sql, right_sql));
    }

    /// `multiset_equal_routes_through_backend_trait` (`docs/plans/20260720-prod-w9-spark-conformance-twin.md`
    /// Phase 2 TDD list): proves [`multiset_equal_via_backend`] executes via
    /// [`smelt_backend::Backend::execute_sql`] (the real DuckDB impl), not a raw
    /// `duckdb::Connection`, and still catches a seeded divergence — the
    /// trait-level mirror of
    /// `crates/smelt-cli/tests/maintenance_conformance/harness_self_check.rs::oracle_flags_a_seeded_divergence`.
    #[tokio::test]
    async fn multiset_equal_routes_through_backend_trait() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let db_path = tmp.path().join("db.duckdb");
        let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open DuckDbBackend");

        // `Backend::execute_sql` prepares one statement at a time (unlike
        // `duckdb::Connection::execute_batch`), so each DDL/DML statement is its
        // own call.
        for stmt in [
            "CREATE TABLE main.left_t(id BIGINT, val DOUBLE)",
            "INSERT INTO main.left_t VALUES (1, 10.0), (2, 20.0)",
            "CREATE TABLE main.right_t(id BIGINT, val DOUBLE)",
            "INSERT INTO main.right_t VALUES (2, 20.0), (1, 10.0)",
        ] {
            backend
                .execute_sql(stmt)
                .await
                .unwrap_or_else(|e| panic!("seed equal multisets ({stmt}): {e}"));
        }

        assert!(
            multiset_equal_via_backend(
                &backend,
                "SELECT * FROM main.left_t",
                "SELECT * FROM main.right_t"
            )
            .await
            .expect("equal-multiset comparison must not error"),
            "equal multisets in different physical order must compare equal via the Backend trait"
        );

        // Seed a divergence: a re-delivered delta double-counted into
        // `left_t` — the exact multiplicity bug this oracle exists to catch
        // (design F2, mirrored from `duplicated_identical_row_is_visible_to_except_all_but_not_except`).
        backend
            .execute_sql("INSERT INTO main.left_t VALUES (1, 10.0);")
            .await
            .expect("seed a divergence");

        assert!(
            !multiset_equal_via_backend(
                &backend,
                "SELECT * FROM main.left_t",
                "SELECT * FROM main.right_t"
            )
            .await
            .expect("divergent-multiset comparison must not error"),
            "the Backend-routed oracle must flag the seeded divergence, exactly like the raw-Connection oracle"
        );
    }
}
