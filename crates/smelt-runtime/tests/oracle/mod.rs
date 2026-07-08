//! `EXPERIMENTAL(property-discovery): disposable`
//!
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
//!
//! Duplicated from `crates/smelt-cli/tests/property_discovery/oracle.rs`
//! (that copy stays there, still shared by the `g_05`/`g_06`/`g_08`/`g_09`
//! cells) because each file under `smelt-runtime/tests/` compiles as its own
//! independent test binary and cannot `use crate::oracle` across binaries.

#![allow(dead_code)]

use duckdb::Connection;

/// Row count of `(left EXCEPT ALL right)` — multiset difference: a row
/// present in `left` more times than in `right` contributes
/// `count_left - count_right` to this number (when positive). Zero in both
/// directions ⇔ `left` and `right` are equal multisets.
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
pub fn multiset_equal(conn: &Connection, left_sql: &str, right_sql: &str) -> bool {
    except_all_row_count(conn, left_sql, right_sql) == 0
        && except_all_row_count(conn, right_sql, left_sql) == 0
}

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
