//! Reusable DuckDB oracle harness for backbuild conformance tests (research
//! `docs/research/20260802-backbuild-synthesis.md` §6 "Conformance
//! harness"). Shared, per-file, via `#[path = "backbuild_conformance/
//! harness.rs"] mod harness;` — each `tests/*.rs` file compiles as its own
//! independent crate, so this cannot be a regular library module.
//!
//! The harness only *executes* statements a [`smelt_logical::backbuild`]
//! option already authored; it never authors SQL of its own beyond plain
//! test-fixture staging and the equivalence-check queries themselves
//! (statement single-ownership, `docs/specs/architecture.md`
//! §"Constraints & Invariants" item 12).

#![allow(dead_code)]

use duckdb::Connection;

use smelt_logical::backbuild::BackbuildOption;

/// Execute arbitrary DDL/DML to stage a test's source-table fixtures (e.g.
/// `CREATE TABLE orders (...); INSERT INTO orders VALUES (...);`). This SQL
/// is test-authored fixture setup, not a backbuild-emitted statement.
pub fn stage_inputs(conn: &Connection, ddl_and_data: &str) {
    conn.execute_batch(ddl_and_data)
        .expect("stage backbuild conformance test inputs");
}

/// `(Re)build` the deployed table fresh from the before-definition:
/// `DROP TABLE IF EXISTS <table>; CREATE TABLE <table> AS <before_sql>;`.
/// Idempotent over stable staged source data — calling this again gives
/// [`verify_option`] a genuinely fresh copy of the before-table to apply
/// each option's script against, per research §6 ("each option's script
/// applies to a fresh copy of the staged before-table").
pub fn build_before(conn: &Connection, table: &str, before_sql: &str) {
    conn.execute_batch(&format!(
        "DROP TABLE IF EXISTS {table}; CREATE TABLE {table} AS {before_sql}"
    ))
    .expect("build before-definition table");
}

/// Rebuild `table` fresh from `before_sql`, apply `option`'s statements in
/// order, then assert the result matches a full rebuild from `after_sql`
/// (research §2's guarantee: `T == eval(after, I)`, multiset-equal with
/// columns matched by name and type). Every option is verified
/// independently against its own fresh copy of the before-table — a case
/// admitting several options proves each one (research §6).
pub fn verify_option(
    conn: &Connection,
    table: &str,
    before_sql: &str,
    after_sql: &str,
    option: &BackbuildOption,
) {
    build_before(conn, table, before_sql);
    for stmt in &option.statements {
        conn.execute_batch(stmt)
            .unwrap_or_else(|e| panic!("apply backbuild option statement `{stmt}`: {e}"));
    }
    assert_matches_full_rebuild(conn, table, after_sql);
}

/// Rebuild `table` fresh from `before_sql`, apply an already-assembled
/// multi-statement script (`classify::assemble`'s output — a composite
/// spanning several atoms' chosen options, H-ordered) in order, then assert
/// the result matches a full rebuild from `after_sql`. The composite
/// (research §4 "H. Composites") sibling of [`verify_option`], which only
/// ever verifies one atom's own option in isolation — this executes a
/// caller-assembled script and authors nothing of its own (statement
/// single-ownership, `docs/specs/architecture.md` §"Constraints &
/// Invariants" item 12).
pub fn verify_script(
    conn: &Connection,
    table: &str,
    before_sql: &str,
    after_sql: &str,
    script: &[String],
) {
    build_before(conn, table, before_sql);
    for stmt in script {
        conn.execute_batch(stmt)
            .unwrap_or_else(|e| panic!("apply backbuild script statement `{stmt}`: {e}"));
    }
    assert_matches_full_rebuild(conn, table, after_sql);
}

/// Assert `table`'s current contents equal a fresh full rebuild from
/// `after_sql`: multiset row equality (two-way `EXCEPT ALL` — plain
/// `EXCEPT` is set-based and would miss duplicate-row-count drift) plus
/// column name/type equality, per research §6 ("column name/type
/// comparison").
///
/// Both checks are column-**name**-driven, not raw physical-position-driven:
/// `table`'s physical column order can legitimately differ from
/// `after_sql`'s declared order — an `ALTER TABLE … ADD COLUMN` always
/// appends physically at the end, regardless of where the after-definition
/// declares the column (research §4 "H. Composites": "after an `ALTER ADD`,
/// the table's physical column order differs from the after-definition's
/// declared order"). A raw `SELECT * ... EXCEPT ALL SELECT * ...` compares
/// columns positionally, which would misreport a same-valued-but-reordered
/// table as non-equivalent (or worse, silently compare two differently-named
/// columns against each other) — so both sides are explicitly projected
/// through the same after-definition-declared column-name list before the
/// value comparison, and the schema check compares name/type *sets* rather
/// than name/type *sequences*.
pub fn assert_matches_full_rebuild(conn: &Connection, table: &str, after_sql: &str) {
    conn.execute_batch(&format!(
        "CREATE OR REPLACE TEMP VIEW __backbuild_after AS {after_sql}"
    ))
    .expect("create full-rebuild comparison view");

    let table_schema = describe(conn, table);
    let after_schema = describe(conn, "__backbuild_after");
    let mut table_schema_sorted = table_schema.clone();
    table_schema_sorted.sort();
    let mut after_schema_sorted = after_schema.clone();
    after_schema_sorted.sort();
    assert_eq!(
        table_schema_sorted, after_schema_sorted,
        "{table}'s column name/type set does not match the full rebuild of after_sql"
    );

    // Project both sides through the same explicit, after-declared column
    // list so positional `EXCEPT ALL` compares each name-matched column
    // pair against itself, never against a differently-named column that
    // merely happens to share the same physical ordinal position.
    let col_list = after_schema
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let table_select = format!("SELECT {col_list} FROM {table}");
    let after_select = format!("SELECT {col_list} FROM __backbuild_after");

    let forward = except_all_count(conn, &table_select, &after_select);
    let backward = except_all_count(conn, &after_select, &table_select);
    assert_eq!(
        forward, 0,
        "{table} has rows not present in the full rebuild of after_sql (EXCEPT ALL forward)"
    );
    assert_eq!(
        backward, 0,
        "the full rebuild of after_sql has rows not present in {table} (EXCEPT ALL backward)"
    );

    conn.execute_batch("DROP VIEW __backbuild_after")
        .expect("drop full-rebuild comparison view");
}

fn except_all_count(conn: &Connection, left_sql: &str, right_sql: &str) -> i64 {
    conn.query_row(
        &format!("SELECT count(*) FROM (({left_sql}) EXCEPT ALL ({right_sql})) AS d"),
        [],
        |row| row.get(0),
    )
    .expect("except all count query")
}

/// Collect the first column of `sql`'s result rows as strings, in result
/// order. Used to snapshot column values directly (as text) before and after
/// applying an option's script — the D1 sibling-untouched assertion needs a
/// direct value comparison, not just the multiset oracle (which would not
/// catch a sibling column silently rewritten to a value that happens to
/// already be present elsewhere in the table).
pub fn text_column(conn: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(sql).expect("prepare text_column query");
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("text_column query rows");
    rows.collect::<Result<Vec<_>, _>>()
        .expect("collect text_column rows")
}

fn describe(conn: &Connection, relation: &str) -> Vec<(String, String)> {
    let mut stmt = conn
        .prepare(&format!("DESCRIBE {relation}"))
        .expect("describe relation");
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .expect("describe relation rows");
    rows.collect::<Result<Vec<_>, _>>()
        .expect("collect describe relation rows")
}
