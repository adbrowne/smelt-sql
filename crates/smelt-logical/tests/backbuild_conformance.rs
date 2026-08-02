//! DuckDB conformance tests for `smelt_logical::backbuild::{derive_backbuild_options,
//! assemble}` — Phase 2 (option enumeration, refusals, and the FullRefresh
//! baseline). See `docs/research/20260802-backbuild-synthesis.md` §2, §4
//! (G-class "Honest refusals", the CTE posture note), and §6
//! ("Conformance harness"), and
//! `.superpowers/sdd/20260802-backbuild-synthesis/task-2-brief.md`.
//!
//! This phase's classifier never admits a targeted technique (that starts
//! later), so every case here proves two things: the targeted script is
//! correctly empty (either because the diff is a true no-op, or because a
//! refusal makes `FullRefresh` the atom's only option), and the
//! `FullRefresh` baseline itself is oracle-verified against a real DuckDB.
//!
//! Skips loudly (never silently) when `DUCKDB_LIB_DIR`/the system DuckDB
//! library is unavailable: this crate's dev-profile already requires it to
//! *compile* (the `duckdb` dev-dependency links against it), same posture
//! as `tests/maintenance_plan_conformance.rs`.

#[path = "backbuild_conformance/harness.rs"]
mod harness;

use std::collections::{BTreeMap, BTreeSet};

use duckdb::Connection;

use smelt_logical::backbuild::{
    assemble, definition_diff, derive_backbuild_options, BackbuildInputs, DefinitionDiff,
    SelectListDiff, Selection, SourceRef, Technique,
};

fn parse(sql: &str) -> smelt_parser::File {
    let parse = smelt_parser::parse(sql);
    smelt_parser::File::cast(parse.syntax()).expect("file")
}

fn inputs(table: &str, after_sql: &str) -> BackbuildInputs {
    BackbuildInputs {
        table: table.to_string(),
        after_sql: after_sql.to_string(),
        row_identity: None,
        not_null_columns: BTreeSet::new(),
        added_column_types: BTreeMap::new(),
        sources: BTreeMap::new(),
    }
}

fn targeted_of_len(len: usize) -> Selection {
    Selection::Targeted {
        atom_choices: vec![0; len],
    }
}

#[test]
fn harness_smoke_a0_noop() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (id INT, amount INT);
         INSERT INTO orders VALUES (1, 10), (2, -5), (3, 20);",
    );

    let before_sql = "SELECT id, amount FROM orders WHERE amount > 0 -- keep positive orders";
    let after_sql =
        "SELECT\n  id,\n  amount\nFROM orders\nWHERE amount > 0  -- keep positive orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(diff.is_noop(), "expected an A0 no-op diff: {diff:?}");

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert!(
        options.atoms.is_empty(),
        "an empty diff must yield no atoms, got {:?}",
        options.atoms
    );

    let targeted = assemble(&options, &targeted_of_len(0));
    assert!(
        targeted.is_empty(),
        "A0's targeted script must be empty, got {targeted:?}"
    );

    harness::verify_option(&conn, "t", before_sql, after_sql, &options.full_refresh);
}

#[test]
fn g1_group_by_change_refuses() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (id INT, amount INT, status TEXT);
         INSERT INTO orders VALUES
           (1, 10, 'open'), (2, 20, 'open'), (3, 5, 'closed'), (4, 7, 'closed');",
    );

    let before_sql = "SELECT status, count(*) AS n FROM orders GROUP BY status";
    let after_sql = "SELECT status, amount, count(*) AS n FROM orders GROUP BY status, amount";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());
    let comparable = match &diff {
        DefinitionDiff::Comparable(c) => c,
        other => panic!("expected Comparable diff, got {other:?}"),
    };
    assert!(
        matches!(
            comparable.skeleton,
            smelt_logical::backbuild::SkeletonDiff::Changed { .. }
        ),
        "expected a skeleton Changed verdict for an added GROUP BY key, got {:?}",
        comparable.skeleton
    );

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert!(
        atom.options.is_empty(),
        "a grain change must admit no targeted option, got {:?}",
        atom.options
    );
    assert_eq!(
        atom.inadmissible.len(),
        1,
        "inadmissible: {:?}",
        atom.inadmissible
    );
    let refusal = &atom.inadmissible[0];
    assert!(
        refusal.reason.contains("G1"),
        "expected a G1-labelled refusal for a GROUP BY key change, got: {}",
        refusal.reason
    );

    let targeted = assemble(&options, &targeted_of_len(1));
    assert!(
        targeted.is_empty(),
        "a grain change must yield no composed targeted script, got {targeted:?}"
    );

    harness::verify_option(&conn, "t", before_sql, after_sql, &options.full_refresh);
}

#[test]
fn g1_distinct_toggle_refuses() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (id INT, status TEXT);
         INSERT INTO orders VALUES (1, 'open'), (2, 'open'), (3, 'closed');",
    );

    let before_sql = "SELECT status FROM orders";
    let after_sql = "SELECT DISTINCT status FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert!(atom.options.is_empty());
    assert_eq!(
        atom.inadmissible.len(),
        1,
        "inadmissible: {:?}",
        atom.inadmissible
    );
    let refusal = &atom.inadmissible[0];
    assert!(
        refusal.reason.contains("G1"),
        "expected a G1-labelled refusal for a DISTINCT toggle, got: {}",
        refusal.reason
    );

    let targeted = assemble(&options, &targeted_of_len(1));
    assert!(targeted.is_empty());

    harness::verify_option(&conn, "t", before_sql, after_sql, &options.full_refresh);
}

#[test]
fn g2_join_condition_change_refuses() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (id INT, region_id INT, alt_region_id INT);
         CREATE TABLE regions (region_id INT, region_name TEXT);
         INSERT INTO orders VALUES (1, 100, 200), (2, 101, 201);
         INSERT INTO regions VALUES (100, 'north'), (101, 'south'), (200, 'east'), (201, 'west');",
    );

    let before_sql =
        "SELECT o.id, r.region_name FROM orders o LEFT JOIN regions r ON o.region_id = r.region_id";
    let after_sql =
        "SELECT o.id, r.region_name FROM orders o LEFT JOIN regions r ON o.alt_region_id = r.region_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());
    let comparable = match &diff {
        DefinitionDiff::Comparable(c) => c,
        other => panic!("expected Comparable diff, got {other:?}"),
    };
    assert!(
        matches!(
            comparable.skeleton,
            smelt_logical::backbuild::SkeletonDiff::Changed { .. }
        ),
        "expected a skeleton Changed verdict for an edited join condition, got {:?}",
        comparable.skeleton
    );

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert!(atom.options.is_empty());
    assert_eq!(
        atom.inadmissible.len(),
        1,
        "inadmissible: {:?}",
        atom.inadmissible
    );
    let refusal = &atom.inadmissible[0];
    assert!(
        refusal.reason.contains("G2"),
        "expected a G2-labelled refusal for an edited join condition, got: {}",
        refusal.reason
    );

    let targeted = assemble(&options, &targeted_of_len(1));
    assert!(targeted.is_empty());

    harness::verify_option(&conn, "t", before_sql, after_sql, &options.full_refresh);
}

// ===== B1/B2 (task-3-brief.md) =====

#[test]
fn b1_constant_column() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (id INTEGER, amount INTEGER);
         INSERT INTO orders VALUES (1, 10), (2, 20), (3, -5);",
    );

    let before_sql = "SELECT id, amount FROM orders";
    let after_sql = "SELECT id, amount, 'active' AS status FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let mut added_column_types = BTreeMap::new();
    added_column_types.insert("status".to_string(), "TEXT".to_string());
    let inputs = BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.to_string(),
        row_identity: None,
        not_null_columns: BTreeSet::new(),
        added_column_types,
        sources: BTreeMap::new(),
    };

    let options = derive_backbuild_options(&diff, &inputs);
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(atom.options.len(), 1, "options: {:?}", atom.options);
    // B3 is attempted independently (research §2 "Options, not choices")
    // and refuses — a literal constant has no column dependency to bind an
    // upstream alias to — but that refusal coexists with the admitted B1
    // option rather than being suppressed by it
    // (`b_dual_derivable_refusals_still_recorded`).
    assert_eq!(atom.inadmissible.len(), 1, "{:?}", atom.inadmissible);
    assert!(atom.inadmissible[0].reason.contains("B3"));
    let option = &atom.options[0];
    assert_eq!(option.statements.len(), 2, "{option:?}");
    assert!(option.statements[0].starts_with("ALTER TABLE"));
    assert!(option.statements[1].starts_with("UPDATE"));

    harness::verify_option(&conn, "t", before_sql, after_sql, option);
}

#[test]
fn b1_arithmetic_over_stored_columns() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (id INTEGER, price INTEGER, qty INTEGER);
         INSERT INTO orders VALUES (1, 10, 2), (2, 5, 3), (3, 7, 0);",
    );

    let before_sql = "SELECT id, price, qty FROM orders";
    let after_sql = "SELECT id, price, qty, price * qty AS total FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let mut added_column_types = BTreeMap::new();
    added_column_types.insert("total".to_string(), "INTEGER".to_string());
    let inputs = BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.to_string(),
        row_identity: None,
        not_null_columns: BTreeSet::new(),
        added_column_types,
        sources: BTreeMap::new(),
    };

    let options = derive_backbuild_options(&diff, &inputs);
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(atom.options.len(), 1, "options: {:?}", atom.options);
    // B3 is attempted independently and refuses — `price`/`qty` are
    // unqualified, so the grain-link proof cannot bind a FROM-tree alias —
    // but the refusal coexists with the admitted B1 option
    // (`b_dual_derivable_refusals_still_recorded`).
    assert_eq!(atom.inadmissible.len(), 1, "{:?}", atom.inadmissible);
    assert!(atom.inadmissible[0].reason.contains("B3"));

    harness::verify_option(&conn, "t", before_sql, after_sql, &atom.options[0]);
}

#[test]
fn b2_rename_touches_no_rows() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (id INTEGER, amount INTEGER);
         INSERT INTO orders VALUES (1, 10), (2, 20);",
    );

    let before_sql = "SELECT id, amount FROM orders";
    let after_sql = "SELECT id, amount AS total FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(atom.options.len(), 1, "options: {:?}", atom.options);
    let option = &atom.options[0];
    assert_eq!(
        option.statements,
        vec!["ALTER TABLE t RENAME COLUMN amount TO total"]
    );

    harness::verify_option(&conn, "t", before_sql, after_sql, option);
}

// ===== D1 (task-4-brief.md) =====

#[test]
fn d1_changed_expression_updates_one_column() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (id INTEGER, amount INTEGER, rate INTEGER);
         INSERT INTO orders VALUES (1, 100, 2), (2, 50, 3), (3, 10, 5);",
    );

    // The "before" formula forgot to apply the conversion rate; "after"
    // fixes it. `amount` and `rate` are unchanged, bare pull-throughs, so
    // both are stored 1:1 representatives `amount_usd`'s fixed formula can
    // be derived from.
    let before_sql = "SELECT id, amount, rate, amount AS amount_usd FROM orders";
    let after_sql = "SELECT id, amount, rate, amount * rate AS amount_usd FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(atom.options.len(), 1, "options: {:?}", atom.options);
    assert!(atom.inadmissible.is_empty());
    let option = &atom.options[0];
    assert_eq!(option.statements.len(), 1, "{option:?}");
    assert_eq!(
        option.statements[0],
        "UPDATE t SET amount_usd = amount * rate"
    );

    harness::build_before(&conn, "t", before_sql);

    let sibling_sql =
        "SELECT id::VARCHAR || '|' || amount::VARCHAR || '|' || rate::VARCHAR FROM t ORDER BY id";
    let siblings_before = harness::text_column(&conn, sibling_sql);

    for stmt in &option.statements {
        conn.execute_batch(stmt)
            .unwrap_or_else(|e| panic!("apply D1 update `{stmt}`: {e}"));
    }

    let siblings_after = harness::text_column(&conn, sibling_sql);
    assert_eq!(
        siblings_before, siblings_after,
        "sibling columns (id, amount, rate) must be byte-identical to their pre-script values — \
         D1 must touch only amount_usd"
    );

    harness::assert_matches_full_rebuild(&conn, "t", after_sql);
}

#[test]
fn d1_formatting_only_change_is_noop() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (id INTEGER, price INTEGER, qty INTEGER);
         INSERT INTO orders VALUES (1, 10, 2), (2, 5, 3);",
    );

    let before_sql = "SELECT id, price*qty AS total FROM orders";
    let after_sql = "SELECT id, price  *  qty AS total FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(
        diff.is_noop(),
        "a whitespace-only reformat of a column expression must be a no-op diff: {diff:?}"
    );
    match &diff {
        DefinitionDiff::Comparable(c) => match &c.select_list {
            SelectListDiff::Diffed { changed, .. } => assert!(
                changed.is_empty(),
                "a formatting-only expression change must not appear in `changed`, got \
                 {changed:?}"
            ),
            other => panic!("expected a Diffed select-list, got {other:?}"),
        },
        other => panic!("expected a Comparable diff, got {other:?}"),
    }

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert!(
        options.atoms.is_empty(),
        "a formatting-only reformat must yield no atoms, got {:?}",
        options.atoms
    );

    let targeted = assemble(&options, &targeted_of_len(0));
    assert!(targeted.is_empty());

    harness::verify_option(&conn, "t", before_sql, after_sql, &options.full_refresh);
}

#[test]
fn changed_cte_refuses() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (id INT, amount INT);
         INSERT INTO orders VALUES (1, 10), (2, 20);",
    );

    let before_sql = "WITH base AS (SELECT id, amount FROM orders) SELECT id, amount FROM base";
    let after_sql =
        "WITH base AS (SELECT id, amount * 2 AS amount FROM orders) SELECT id, amount FROM base";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());
    assert!(
        matches!(diff, DefinitionDiff::Opaque { .. }),
        "expected an Opaque diff for a changed CTE body, got {diff:?}"
    );

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert!(atom.options.is_empty());
    assert_eq!(
        atom.inadmissible.len(),
        1,
        "inadmissible: {:?}",
        atom.inadmissible
    );
    let refusal = &atom.inadmissible[0];
    assert!(
        refusal.reason.contains("CTE"),
        "expected a named CTE-change refusal, got: {}",
        refusal.reason
    );

    let targeted = assemble(&options, &targeted_of_len(1));
    assert!(targeted.is_empty());

    harness::verify_option(&conn, "t", before_sql, after_sql, &options.full_refresh);
}

// ===== B3 / D2 (task-5-brief.md) =====

fn source_ref(physical_name: &str, unique_key: &[&str], not_null_columns: &[&str]) -> SourceRef {
    SourceRef {
        physical_name: physical_name.to_string(),
        unique_key: Some(unique_key.iter().map(|s| s.to_string()).collect()),
        not_null_columns: not_null_columns.iter().map(|s| s.to_string()).collect(),
    }
}

fn orders_pullthrough_inputs(
    after_sql: &str,
    added_column_types: &[(&str, &str)],
) -> BackbuildInputs {
    let mut sources = BTreeMap::new();
    sources.insert(
        "o".to_string(),
        source_ref("orders", &["order_id"], &["order_id"]),
    );
    BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.to_string(),
        row_identity: None,
        not_null_columns: BTreeSet::new(),
        added_column_types: added_column_types
            .iter()
            .map(|(name, ty)| (name.to_string(), ty.to_string()))
            .collect(),
        sources,
    }
}

/// `BackbuildInputs` for a `GROUP BY` model's B5 tests: no declared
/// `sources` (B5's re-aggregation reads the model's own FROM/WHERE text
/// directly, never `BackbuildInputs::sources` — and leaving it empty also
/// keeps B3 from spuriously attempting the same aggregate expression, since
/// `admit_upstream_pullthrough` always refuses without a declared source
/// for the aggregate's FROM-tree alias). `not_null_group_keys` declares
/// which `GROUP BY` key output columns are provably `NOT NULL` (research §4
/// intro "Key addressability").
fn group_by_inputs(
    after_sql: &str,
    added_column_types: &[(&str, &str)],
    not_null_group_keys: &[&str],
) -> BackbuildInputs {
    BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.to_string(),
        row_identity: None,
        not_null_columns: not_null_group_keys.iter().map(|s| s.to_string()).collect(),
        added_column_types: added_column_types
            .iter()
            .map(|(name, ty)| (name.to_string(), ty.to_string()))
            .collect(),
        sources: BTreeMap::new(),
    }
}

#[test]
fn b5_aggregate_column_backfill() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (customer_id INTEGER, qty INTEGER);
         INSERT INTO orders VALUES (1, 5), (1, 3), (2, 10), (3, 7), (3, 1);",
    );

    let before_sql = "SELECT o.customer_id AS customer_id, count(*) AS n FROM orders o GROUP BY \
         o.customer_id";
    let after_sql = "SELECT o.customer_id AS customer_id, count(*) AS n, SUM(o.qty) AS \
                      total_qty FROM orders o GROUP BY o.customer_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = group_by_inputs(after_sql, &[("total_qty", "HUGEINT")], &["customer_id"]);
    let options = derive_backbuild_options(&diff, &inputs);
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(atom.options.len(), 1, "options: {:?}", atom.options);

    let option = &atom.options[0];
    assert_eq!(option.technique, Technique::AggregateColumnBackfill);
    assert_eq!(option.statements.len(), 2, "{option:?}");
    assert!(option.statements[0].starts_with("ALTER TABLE t ADD COLUMN total_qty"));
    assert!(
        option.statements[1].starts_with("UPDATE t SET total_qty = s.total_qty FROM (SELECT"),
        "{}",
        option.statements[1]
    );
    assert!(option.statements[1].contains("GROUP BY o.customer_id"));
    assert!(option.statements[1].contains("WHERE t.customer_id = s.customer_id"));

    // Sibling column `n` must be byte-identical before/after — B5 only
    // ever touches its own freshly-added column.
    harness::build_before(&conn, "t", before_sql);
    let sibling_sql = "SELECT n::VARCHAR FROM t ORDER BY customer_id";
    let siblings_before = harness::text_column(&conn, sibling_sql);
    for stmt in &option.statements {
        conn.execute_batch(stmt)
            .unwrap_or_else(|e| panic!("apply B5 statement `{stmt}`: {e}"));
    }
    let siblings_after = harness::text_column(&conn, sibling_sql);
    assert_eq!(
        siblings_after, siblings_before,
        "sibling column `n` must be byte-identical to its pre-script values — B5 must touch \
         only total_qty"
    );

    harness::assert_matches_full_rebuild(&conn, "t", after_sql);
}

#[test]
fn b5_where_clause_is_carried() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (customer_id INTEGER, qty INTEGER);
         INSERT INTO orders VALUES
           (1, 5), (1, -100),
           (2, 10), (2, 3),
           (3, -1);",
    );

    // Every customer has at least one row the model's own WHERE drops
    // (customer 1's -100, customer 3's only row). A bare re-aggregation
    // over `<upstream> GROUP BY <keys>` (skipping the WHERE) would
    // over-count customer 1's total and wrongly materialize a group for
    // customer 3 — the research §4 B5 trap made executable.
    let before_sql = "SELECT o.customer_id AS customer_id, count(*) AS n FROM orders o WHERE \
                       o.qty > 0 GROUP BY o.customer_id";
    let after_sql = "SELECT o.customer_id AS customer_id, count(*) AS n, SUM(o.qty) AS \
                      total_qty FROM orders o WHERE o.qty > 0 GROUP BY o.customer_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = group_by_inputs(after_sql, &[("total_qty", "HUGEINT")], &["customer_id"]);
    let options = derive_backbuild_options(&diff, &inputs);
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(atom.options.len(), 1, "options: {:?}", atom.options);
    let option = &atom.options[0];
    assert!(
        option.statements[1].contains("WHERE o.qty > 0"),
        "the re-aggregation subquery must carry the model's own WHERE verbatim: {}",
        option.statements[1]
    );

    harness::verify_option(&conn, "t", before_sql, after_sql, option);
}

#[test]
fn b5_update_is_matched_only() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (customer_id INTEGER, status TEXT, qty INTEGER);
         INSERT INTO orders VALUES
           (1, 'active', 5), (1, 'active', 3),
           (2, 'inactive', 10),
           (3, 'active', 7);",
    );

    // Customer 2 is entirely `inactive` — the model's own WHERE has always
    // filtered it out, so `t` never has a row for it, before or after.
    let before_sql = "SELECT o.customer_id AS customer_id, count(*) AS n FROM orders o WHERE \
                       o.status = 'active' GROUP BY o.customer_id";
    let after_sql = "SELECT o.customer_id AS customer_id, count(*) AS n, SUM(o.qty) AS \
                      total_qty FROM orders o WHERE o.status = 'active' GROUP BY o.customer_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = group_by_inputs(after_sql, &[("total_qty", "HUGEINT")], &["customer_id"]);
    let options = derive_backbuild_options(&diff, &inputs);
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(atom.options.len(), 1, "options: {:?}", atom.options);
    let option = &atom.options[0];

    harness::build_before(&conn, "t", before_sql);
    let count_before: i64 = conn
        .query_row("SELECT count(*) FROM t", [], |row| row.get(0))
        .expect("count before");
    for stmt in &option.statements {
        conn.execute_batch(stmt)
            .unwrap_or_else(|e| panic!("apply B5 statement `{stmt}`: {e}"));
    }
    let count_after: i64 = conn
        .query_row("SELECT count(*) FROM t", [], |row| row.get(0))
        .expect("count after");
    assert_eq!(
        count_before, count_after,
        "B5 must be matched-only — customer 2's group (filtered out by the model's own WHERE) \
         must never be inserted"
    );

    harness::assert_matches_full_rebuild(&conn, "t", after_sql);
}

/// `BackbuildInputs` for a B6 window-column test: a declared `row_identity`
/// (proven NOT NULL) is the only fact B6's self-read proof needs beyond the
/// window's own dependency resolution — no declared `sources` (B6 never
/// reads upstream).
fn row_identity_inputs(
    after_sql: &str,
    added_column_types: &[(&str, &str)],
    identity: &[&str],
) -> BackbuildInputs {
    BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.to_string(),
        row_identity: Some(identity.iter().map(|s| s.to_string()).collect()),
        not_null_columns: identity.iter().map(|s| s.to_string()).collect(),
        added_column_types: added_column_types
            .iter()
            .map(|(name, ty)| (name.to_string(), ty.to_string()))
            .collect(),
        sources: BTreeMap::new(),
    }
}

#[test]
fn b6_row_number_over_stored_columns() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (order_id INTEGER, status TEXT, amount INTEGER);
         INSERT INTO orders VALUES
           (1, 'open', 10), (2, 'open', 20), (3, 'closed', 5), (4, 'closed', 7),
           (5, 'open', 1);",
    );

    let before_sql =
        "SELECT o.order_id AS order_id, o.status AS status, o.amount AS amount FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.status AS status, o.amount AS amount, \
                      ROW_NUMBER() OVER (PARTITION BY status ORDER BY order_id) AS rn FROM \
                      orders o";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = row_identity_inputs(after_sql, &[("rn", "BIGINT")], &["order_id"]);
    let options = derive_backbuild_options(&diff, &inputs);
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(atom.options.len(), 1, "options: {:?}", atom.options);

    let option = &atom.options[0];
    assert_eq!(option.technique, Technique::WindowColumnBackfill);
    assert_eq!(option.statements.len(), 2, "{option:?}");
    assert!(option.statements[0].starts_with("ALTER TABLE t ADD COLUMN rn"));
    assert!(
        option.statements[1].starts_with("UPDATE t SET rn = s.rn FROM (SELECT"),
        "{}",
        option.statements[1]
    );
    assert!(
        option.statements[1].contains("ROW_NUMBER() OVER (PARTITION BY status ORDER BY order_id)"),
        "{}",
        option.statements[1]
    );
    assert!(
        option.statements[1].contains("FROM t)"),
        "{}",
        option.statements[1]
    );
    assert!(
        option.statements[1].contains("WHERE t.order_id = s.order_id"),
        "{}",
        option.statements[1]
    );

    // Sibling columns must be byte-identical before/after — B6 only ever
    // touches its own freshly-added column.
    harness::build_before(&conn, "t", before_sql);
    let sibling_sql = "SELECT status || ':' || amount::VARCHAR FROM t ORDER BY order_id";
    let siblings_before = harness::text_column(&conn, sibling_sql);
    for stmt in &option.statements {
        conn.execute_batch(stmt)
            .unwrap_or_else(|e| panic!("apply B6 statement `{stmt}`: {e}"));
    }
    let siblings_after = harness::text_column(&conn, sibling_sql);
    assert_eq!(
        siblings_after, siblings_before,
        "sibling columns must be byte-identical to their pre-script values — B6 must touch only \
         rn"
    );

    harness::assert_matches_full_rebuild(&conn, "t", after_sql);
}

#[test]
fn b6_window_respects_stored_row_set() {
    // The model's own WHERE filters some rows out of `t` entirely (order 3
    // never lands in the stored table). B6's self-read subquery computes
    // the window over `t` itself, so it can only ever see the rows the
    // model's own WHERE admits — matching the rebuild by construction.
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (order_id INTEGER, status TEXT, amount INTEGER);
         INSERT INTO orders VALUES
           (1, 'open', 10), (2, 'open', 20), (3, 'open', -5), (4, 'closed', 7),
           (5, 'open', 1);",
    );

    let before_sql = "SELECT o.order_id AS order_id, o.status AS status, o.amount AS amount \
                       FROM orders o WHERE o.amount > 0";
    let after_sql = "SELECT o.order_id AS order_id, o.status AS status, o.amount AS amount, \
                      ROW_NUMBER() OVER (PARTITION BY status ORDER BY order_id) AS rn FROM \
                      orders o WHERE o.amount > 0";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = row_identity_inputs(after_sql, &[("rn", "BIGINT")], &["order_id"]);
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = &options.atoms[0];
    assert_eq!(atom.options.len(), 1, "options: {:?}", atom.options);
    let option = &atom.options[0];

    // If the self-read subquery somehow read the raw `orders` source
    // instead of the stored table `t`, order 3 (filtered by amount > 0)
    // would still be visible to the window and would shift the rank of
    // order 5 within the 'open' partition — the oracle catches that
    // divergence directly.
    harness::verify_option(&conn, "t", before_sql, after_sql, option);
}

#[test]
fn b3_upstream_pullthrough() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (order_id INTEGER, customer TEXT, discount INTEGER);
         INSERT INTO orders VALUES (1, 'alice', 10), (2, 'bob', 20), (3, 'carol', 30);",
    );

    let before_sql = "SELECT o.order_id AS order_id, o.customer AS customer FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.customer AS customer, o.discount AS \
                      discount FROM orders o";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = orders_pullthrough_inputs(after_sql, &[("discount", "INTEGER")]);
    let options = derive_backbuild_options(&diff, &inputs);
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(atom.options.len(), 1, "options: {:?}", atom.options);
    // B1 is attempted independently and refuses — `discount` is not
    // (yet) a stored representative of this model, only an upstream-only
    // dependency — but the refusal coexists with the admitted B3 option
    // (`b_dual_derivable_refusals_still_recorded`).
    assert_eq!(atom.inadmissible.len(), 1, "{:?}", atom.inadmissible);
    assert!(atom.inadmissible[0].reason.contains("B1"));
    let option = &atom.options[0];
    assert_eq!(option.statements.len(), 2, "{option:?}");
    assert!(option.statements[0].starts_with("ALTER TABLE t ADD COLUMN discount"));
    assert!(option.statements[1].starts_with("UPDATE t SET discount ="));
    assert!(option.statements[1].contains("FROM orders u"));
    assert!(option.statements[1].contains("WHERE t.order_id = u.order_id"));

    harness::verify_option(&conn, "t", before_sql, after_sql, option);
}

#[test]
fn b3_respects_model_filter() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (order_id INTEGER, customer TEXT, discount INTEGER);
         INSERT INTO orders VALUES (1, 'alice', 10), (2, 'bob', 20), (3, 'carol', 30);",
    );

    let before_sql = "SELECT o.order_id AS order_id, o.customer AS customer FROM orders o WHERE \
                       o.customer <> 'bob'";
    let after_sql = "SELECT o.order_id AS order_id, o.customer AS customer, o.discount AS \
                      discount FROM orders o WHERE o.customer <> 'bob'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = orders_pullthrough_inputs(after_sql, &[("discount", "INTEGER")]);
    let options = derive_backbuild_options(&diff, &inputs);
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(atom.options.len(), 1, "options: {:?}", atom.options);

    // Only the surviving (non-`bob`) rows are ever in `t` to begin with —
    // the join simply never matches the filtered-out row, no extra
    // predicate needed (research §4 B3: "Rows filtered out of t by the
    // model's WHERE are simply never matched — the join touches only
    // existing rows").
    harness::verify_option(&conn, "t", before_sql, after_sql, &atom.options[0]);
}

#[test]
fn b_dual_derivable_added_column_yields_both_options() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (order_id INTEGER, amount INTEGER);
         INSERT INTO orders VALUES (1, 100), (2, 50), (3, 10);",
    );

    // `amount_copy` is a bare copy of `amount`, which is *already* a
    // stored, unchanged bare pull-through of alias `o` — the same
    // dual-derivability shape as `d_dual_derivable_yields_both_options`
    // (research §2 "Options, not choices"), but for an *added* column
    // (B1/B3) rather than a changed one (D1/D2): readable both from the
    // model's own stored `amount` column (B1, self-read) and by rereading
    // upstream `orders` directly through its declared, NOT-NULL,
    // stored-bare-pull-through `order_id` key (B3, upstream read).
    let before_sql = "SELECT o.order_id AS order_id, o.amount AS amount FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.amount AS amount, o.amount AS amount_copy \
                      FROM orders o";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = orders_pullthrough_inputs(after_sql, &[("amount_copy", "INTEGER")]);
    let options = derive_backbuild_options(&diff, &inputs);
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(
        atom.options.len(),
        2,
        "expected both B1 and B3 options, got {:?}",
        atom.options
    );
    assert!(atom.inadmissible.is_empty());

    let self_read = atom
        .options
        .iter()
        .find(|o| !o.reads_upstream)
        .expect("a self-read (B1) option");
    assert_eq!(self_read.technique, Technique::SelfDerivedColumnAdd);

    let upstream_read = atom
        .options
        .iter()
        .find(|o| o.reads_upstream)
        .expect("an upstream-read (B3) option");
    assert_eq!(upstream_read.technique, Technique::UpstreamPullthrough);

    // Each option independently reaches the same, correct end state from a
    // fresh copy of the before-table (research §6: "each option's script
    // applies to a fresh copy of the staged before-table").
    harness::verify_option(&conn, "t", before_sql, after_sql, self_read);
    harness::verify_option(&conn, "t", before_sql, after_sql, upstream_read);
}

#[test]
fn d2_changed_expression_from_upstream() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (order_id INTEGER, base_price INTEGER, discount INTEGER);
         INSERT INTO orders VALUES (1, 100, 10), (2, 200, 20), (3, 300, 0);",
    );

    // "price" used to pull `base_price` straight through; it now reads
    // `discount` instead — an upstream column the model never stored.
    let before_sql = "SELECT o.order_id AS order_id, o.base_price AS price FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.discount AS price FROM orders o";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = orders_pullthrough_inputs(after_sql, &[]);
    let options = derive_backbuild_options(&diff, &inputs);
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(atom.options.len(), 1, "options: {:?}", atom.options);
    assert!(atom.inadmissible.is_empty());
    let option = &atom.options[0];
    assert_eq!(option.statements.len(), 1, "{option:?}");
    assert_eq!(
        option.statements[0],
        "UPDATE t SET price = u.discount FROM orders u WHERE t.order_id = u.order_id"
    );

    harness::build_before(&conn, "t", before_sql);
    let sibling_sql = "SELECT order_id::VARCHAR FROM t ORDER BY order_id";
    let siblings_before = harness::text_column(&conn, sibling_sql);

    for stmt in &option.statements {
        conn.execute_batch(stmt)
            .unwrap_or_else(|e| panic!("apply D2 update `{stmt}`: {e}"));
    }

    let siblings_after = harness::text_column(&conn, sibling_sql);
    assert_eq!(
        siblings_after, siblings_before,
        "sibling columns (order_id) must be byte-identical to their pre-script values — D2 \
         must touch only price"
    );

    harness::assert_matches_full_rebuild(&conn, "t", after_sql);
}

#[test]
fn d_dual_derivable_yields_both_options() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (order_id INTEGER, amount INTEGER, rate INTEGER);
         INSERT INTO orders VALUES (1, 100, 2), (2, 50, 3), (3, 10, 5);",
    );

    // `amount_usd`'s new formula reads only `o.amount`/`o.rate` — both are
    // *also* bare, unchanged, stored pull-throughs of the same alias `o`
    // whose unique_key (`order_id`) is itself pulled through and declared
    // NOT NULL, so the expression is derivable both from stored columns
    // (D1, self-read) and by re-reading the upstream directly (D2,
    // `UPDATE ... FROM`).
    let before_sql = "SELECT o.order_id AS order_id, o.amount AS amount, o.rate AS rate, \
                       o.amount AS amount_usd FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.amount AS amount, o.rate AS rate, \
                      o.amount * o.rate AS amount_usd FROM orders o";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = orders_pullthrough_inputs(after_sql, &[]);
    let options = derive_backbuild_options(&diff, &inputs);
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(
        atom.options.len(),
        2,
        "expected both D1 and D2 options, got {:?}",
        atom.options
    );
    assert!(atom.inadmissible.is_empty());

    let self_read = atom
        .options
        .iter()
        .find(|o| !o.reads_upstream)
        .expect("a self-read (D1) option");
    assert_eq!(
        self_read.statements,
        vec!["UPDATE t SET amount_usd = amount * rate"]
    );

    let upstream_read = atom
        .options
        .iter()
        .find(|o| o.reads_upstream)
        .expect("an upstream-read (D2) option");
    assert_eq!(
        upstream_read.statements,
        vec![
            "UPDATE t SET amount_usd = u.amount * u.rate FROM orders u WHERE t.order_id = \
             u.order_id"
        ]
    );

    // Each option independently reaches the same, correct end state from a
    // fresh copy of the before-table (research §6: "each option's script
    // applies to a fresh copy of the staged before-table").
    harness::verify_option(&conn, "t", before_sql, after_sql, self_read);
    harness::verify_option(&conn, "t", before_sql, after_sql, upstream_read);
}

#[test]
fn b3_stale_upstream_documents_precondition() {
    // Research §2 "Why the precondition is load-bearing": an upstream-read
    // script bakes in *current* upstream state, so if `T_old` is not
    // actually `eval(before, I)` at apply time — here, because `orders`
    // was mutated *after* `t` was built from `before_sql` — the backfilled
    // column reflects the fresh upstream while untouched sibling columns
    // still reflect the stale build. This is the documented edge of the
    // contract, not a bug: the test demonstrates the divergence from a
    // full rebuild against the *current* inputs, rather than asserting
    // (falsely) that the script still matches one.
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (order_id INTEGER, customer TEXT, discount INTEGER);
         INSERT INTO orders VALUES (1, 'alice', 10), (2, 'bob', 20);",
    );

    let before_sql = "SELECT o.order_id AS order_id, o.customer AS customer FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.customer AS customer, o.discount AS \
                      discount FROM orders o";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = orders_pullthrough_inputs(after_sql, &[("discount", "INTEGER")]);
    let options = derive_backbuild_options(&diff, &inputs);
    let option = &options.atoms[0].options[0];

    // `T_old` is built from `before` while the precondition (`T_old ==
    // eval(before, I)`) still holds.
    harness::build_before(&conn, "t", before_sql);

    // The precondition is now violated: `orders` changes *after* `t` was
    // built.
    conn.execute_batch("UPDATE orders SET customer = 'zeta', discount = 999 WHERE order_id = 1")
        .expect("mutate upstream after build_before");

    for stmt in &option.statements {
        conn.execute_batch(stmt)
            .unwrap_or_else(|e| panic!("apply B3 backfill `{stmt}`: {e}"));
    }

    // The backfilled column is an upstream-read script — it bakes in the
    // *current* (post-mutation) upstream state.
    let discount_now =
        harness::text_column(&conn, "SELECT discount::VARCHAR FROM t WHERE order_id = 1");
    assert_eq!(discount_now, vec!["999".to_string()]);

    // The untouched sibling column still reflects the *stale* build —
    // `customer` was never part of this script's write set.
    let customer_now =
        harness::text_column(&conn, "SELECT customer::VARCHAR FROM t WHERE order_id = 1");
    assert_eq!(customer_now, vec!["alice".to_string()]);

    // Demonstrate the actual divergence: a full rebuild against the
    // *current* inputs disagrees with `t` on the sibling column.
    let rebuilt_customer = harness::text_column(
        &conn,
        &format!("SELECT customer::VARCHAR FROM ({after_sql}) AS rebuilt WHERE order_id = 1"),
    );
    assert_eq!(rebuilt_customer, vec!["zeta".to_string()]);
    assert_ne!(
        customer_now, rebuilt_customer,
        "t must diverge from a full rebuild against current inputs — this is the documented \
         edge of the §2 precondition, not a correctness bug in the script"
    );
}

// ===== B4 (task-6-brief.md) =====

fn customers_join_inputs(after_sql: &str, added_column_types: &[(&str, &str)]) -> BackbuildInputs {
    let mut sources = BTreeMap::new();
    sources.insert(
        "c".to_string(),
        source_ref("customers", &["customer_id"], &["customer_id"]),
    );
    BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.to_string(),
        row_identity: None,
        not_null_columns: BTreeSet::new(),
        added_column_types: added_column_types
            .iter()
            .map(|(name, ty)| (name.to_string(), ty.to_string()))
            .collect(),
        sources,
    }
}

#[test]
fn b4_left_join_enrichment_fanout() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (order_id INTEGER, customer_id INTEGER);
         CREATE TABLE customers (customer_id INTEGER, customer_name TEXT);
         INSERT INTO orders VALUES (1, 100), (2, 100), (3, 200);
         INSERT INTO customers VALUES (100, 'alice'), (200, 'bob');",
    );

    // Genuine fan-out: two fact rows (1, 2) share dimension row 100 — the
    // join must enrich both without dropping or duplicating either.
    let before_sql = "SELECT o.order_id AS order_id, o.customer_id AS customer_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.customer_id AS customer_id, \
                      c.customer_name AS customer_name FROM orders o LEFT JOIN customers c ON \
                      o.customer_id = c.customer_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = customers_join_inputs(after_sql, &[("customer_name", "TEXT")]);
    let options = derive_backbuild_options(&diff, &inputs);
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(
        atom.options.len(),
        2,
        "a bare column pull must admit both B4 shapes, got {:?}",
        atom.options
    );
    assert!(atom.inadmissible.is_empty());

    let update_from = atom
        .options
        .iter()
        .find(|o| o.technique == Technique::JoinEnrichmentUpdateFrom)
        .expect("the UPDATE ... FROM shape");
    assert!(update_from.statements[1].contains("FROM customers c"));
    assert!(update_from.statements[1].contains("WHERE t.customer_id = c.customer_id"));

    let scalar_subquery = atom
        .options
        .iter()
        .find(|o| o.technique == Technique::JoinEnrichmentScalarSubquery)
        .expect("the scalar-subquery shape");
    assert!(scalar_subquery.statements[1].contains("(SELECT c.customer_name FROM customers c"));

    // Each option is independently oracle-verified against its own fresh
    // copy of the before-table (research §6).
    for option in &atom.options {
        harness::verify_option(&conn, "t", before_sql, after_sql, option);
    }
}

#[test]
fn b4_unmatched_rows_null_extend() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (order_id INTEGER, customer_id INTEGER);
         CREATE TABLE customers (customer_id INTEGER, customer_name TEXT);
         INSERT INTO orders VALUES (1, 100), (2, 300);
         INSERT INTO customers VALUES (100, 'alice');",
    );

    let before_sql = "SELECT o.order_id AS order_id, o.customer_id AS customer_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.customer_id AS customer_id, \
                      c.customer_name AS customer_name FROM orders o LEFT JOIN customers c ON \
                      o.customer_id = c.customer_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = customers_join_inputs(after_sql, &[("customer_name", "TEXT")]);
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = &options.atoms[0];
    assert_eq!(atom.options.len(), 2, "{:?}", atom.options);

    // Each option matches a full rebuild exactly — the multiset oracle
    // catches a wrongly-skipped or wrongly-defaulted unmatched row on its
    // own, but pin the unmatched row's value directly too (NULL, not
    // skipped or defaulted).
    for option in &atom.options {
        harness::verify_option(&conn, "t", before_sql, after_sql, option);
    }

    harness::build_before(&conn, "t", before_sql);
    for stmt in &atom.options[0].statements {
        conn.execute_batch(stmt)
            .unwrap_or_else(|e| panic!("apply B4 backfill `{stmt}`: {e}"));
    }
    let unmatched = harness::text_column(
        &conn,
        "SELECT coalesce(customer_name, '<NULL>') FROM t WHERE order_id = 2",
    );
    assert_eq!(unmatched, vec!["<NULL>".to_string()]);
}

#[test]
fn b4_general_expression_null_extension() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (order_id INTEGER, customer_id INTEGER);
         CREATE TABLE customers (customer_id INTEGER, customer_name TEXT);
         INSERT INTO orders VALUES (1, 100), (2, 300);
         INSERT INTO customers VALUES (100, 'alice');",
    );

    // COALESCE(c.customer_name, 'none') — NULL-extension must be
    // *evaluated*, not skipped: an unmatched row must end 'none', not NULL.
    let before_sql = "SELECT o.order_id AS order_id, o.customer_id AS customer_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.customer_id AS customer_id, \
                      COALESCE(c.customer_name, 'none') AS customer_label FROM orders o LEFT \
                      JOIN customers c ON o.customer_id = c.customer_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = customers_join_inputs(after_sql, &[("customer_label", "TEXT")]);
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = &options.atoms[0];
    // The two naive shapes are the traps this test pins: a bare
    // `UPDATE ... FROM` isn't even offered for a non-bare expression, and
    // the whole-expression scalar subquery is not what
    // `requalify_scalar_subquery` produces — only the per-reference
    // substituted form is ever built.
    assert_eq!(
        atom.options.len(),
        1,
        "a general expression must offer only the per-reference substituted scalar-subquery \
         option, got {:?}",
        atom.options
    );
    assert_eq!(
        atom.options[0].technique,
        Technique::JoinEnrichmentScalarSubquery
    );
    assert!(
        atom.options[0].statements[1].starts_with("UPDATE t SET customer_label = COALESCE("),
        "{:?}",
        atom.options[0].statements
    );
    assert!(atom.inadmissible.is_empty());

    harness::verify_option(&conn, "t", before_sql, after_sql, &atom.options[0]);

    harness::build_before(&conn, "t", before_sql);
    for stmt in &atom.options[0].statements {
        conn.execute_batch(stmt)
            .unwrap_or_else(|e| panic!("apply B4 backfill `{stmt}`: {e}"));
    }
    let unmatched = harness::text_column(&conn, "SELECT customer_label FROM t WHERE order_id = 2");
    assert_eq!(unmatched, vec!["none".to_string()]);
}

// ===== B7 (task-9-brief.md) =====

#[test]
fn b7_two_joins_sequential_backfill() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (order_id INTEGER, dim1_id INTEGER);
         CREATE TABLE dim1 (dim1_id INTEGER, region_id INTEGER);
         CREATE TABLE dim2 (region_id INTEGER, region_name TEXT);
         INSERT INTO orders VALUES (1, 10), (2, 20), (3, 10);
         INSERT INTO dim1 VALUES (10, 100), (20, 200);
         INSERT INTO dim2 VALUES (100, 'north'), (200, 'south');",
    );

    // dim2's ON keys on `d1.region_id` — a column dim1 provides *and* the
    // model stores (the added, bare `region_id` pull-through) — so dim2's
    // backfill must run strictly after dim1's.
    let before_sql = "SELECT o.order_id AS order_id, o.dim1_id AS dim1_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.dim1_id AS dim1_id, d1.region_id AS \
                      region_id, d2.region_name AS region_name FROM orders o LEFT JOIN dim1 d1 \
                      ON o.dim1_id = d1.dim1_id LEFT JOIN dim2 d2 ON d1.region_id = \
                      d2.region_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let mut sources = BTreeMap::new();
    sources.insert(
        "d1".to_string(),
        source_ref("dim1", &["dim1_id"], &["dim1_id"]),
    );
    sources.insert(
        "d2".to_string(),
        source_ref("dim2", &["region_id"], &["region_id"]),
    );
    let inputs = BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.to_string(),
        row_identity: None,
        not_null_columns: BTreeSet::new(),
        added_column_types: [
            ("region_id".to_string(), "INTEGER".to_string()),
            ("region_name".to_string(), "TEXT".to_string()),
        ]
        .into_iter()
        .collect(),
        sources,
    };

    let options = derive_backbuild_options(&diff, &inputs);
    assert_eq!(options.atoms.len(), 2, "atoms: {:?}", options.atoms);
    assert!(
        matches!(
            &options.atoms[0].change,
            smelt_logical::backbuild::AtomicChange::AddedColumn { name } if name == "region_id"
        ),
        "expected dim1's backfill atom (region_id) first, got {:?}",
        options.atoms[0].change
    );
    assert!(
        matches!(
            &options.atoms[1].change,
            smelt_logical::backbuild::AtomicChange::AddedColumn { name } if name == "region_name"
        ),
        "expected dim2's backfill atom (region_name) second, got {:?}",
        options.atoms[1].change
    );
    for atom in &options.atoms {
        assert_eq!(atom.options.len(), 2, "atom: {atom:?}");
        assert!(atom.inadmissible.is_empty());
    }

    // Both admitted shapes (bare pull-through `UPDATE ... FROM` and the
    // scalar-subquery form) compose into an oracle-equal script, with
    // dim1's ALTER+UPDATE always ordered before dim2's.
    for shape in [0usize, 1] {
        let selection = Selection::Targeted {
            atom_choices: vec![shape, shape],
        };
        let script = assemble(&options, &selection);
        assert_eq!(script.len(), 4, "shape {shape}: {script:?}");
        assert!(
            script[0].contains("region_id") && script[0].starts_with("ALTER"),
            "shape {shape}: {script:?}"
        );
        assert!(
            script[2].contains("region_name") && script[2].starts_with("ALTER"),
            "shape {shape}: {script:?}"
        );
        harness::verify_script(&conn, "t", before_sql, after_sql, &script);
    }
}

#[test]
fn b7_independent_joins_either_order() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (order_id INTEGER, customer_id INTEGER, product_id INTEGER);
         CREATE TABLE customers (customer_id INTEGER, customer_name TEXT);
         CREATE TABLE products (product_id INTEGER, product_name TEXT);
         INSERT INTO orders VALUES (1, 100, 900), (2, 200, 900), (3, 100, 800);
         INSERT INTO customers VALUES (100, 'alice'), (200, 'bob');
         INSERT INTO products VALUES (900, 'widget'), (800, 'gadget');",
    );

    // Two added joins, each keyed on an already-stored (pre-existing)
    // representative column — no reference relationship between them, so
    // either processing order is correct.
    let before_sql =
        "SELECT o.order_id AS order_id, o.customer_id AS customer_id, o.product_id AS product_id \
         FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.customer_id AS customer_id, o.product_id \
                      AS product_id, c.customer_name AS customer_name, p.product_name AS \
                      product_name FROM orders o LEFT JOIN customers c ON o.customer_id = \
                      c.customer_id LEFT JOIN products p ON o.product_id = p.product_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let mut sources = BTreeMap::new();
    sources.insert(
        "c".to_string(),
        source_ref("customers", &["customer_id"], &["customer_id"]),
    );
    sources.insert(
        "p".to_string(),
        source_ref("products", &["product_id"], &["product_id"]),
    );
    let inputs = BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.to_string(),
        row_identity: None,
        not_null_columns: BTreeSet::new(),
        added_column_types: [
            ("customer_name".to_string(), "TEXT".to_string()),
            ("product_name".to_string(), "TEXT".to_string()),
        ]
        .into_iter()
        .collect(),
        sources,
    };

    let options = derive_backbuild_options(&diff, &inputs);
    assert_eq!(options.atoms.len(), 2, "atoms: {:?}", options.atoms);
    for atom in &options.atoms {
        assert_eq!(atom.options.len(), 2, "atom: {atom:?}");
        assert!(atom.inadmissible.is_empty());
    }
    // Both backfills are emitted regardless of which alias happens to sort
    // first — the independence case names no required relative order.
    let names: BTreeSet<String> = options
        .atoms
        .iter()
        .map(|a| match &a.change {
            smelt_logical::backbuild::AtomicChange::AddedColumn { name } => name.clone(),
            other => panic!("expected AddedColumn, got {other:?}"),
        })
        .collect();
    assert_eq!(
        names,
        BTreeSet::from(["customer_name".to_string(), "product_name".to_string()])
    );

    let script = assemble(&options, &targeted_of_len(2));
    assert_eq!(script.len(), 4, "{script:?}");
    harness::verify_script(&conn, "t", before_sql, after_sql, &script);
}

// ===== E1/E4 (task-7-brief.md) =====

#[test]
fn e1_tighten_deletes_only() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (id INTEGER, status TEXT);
         INSERT INTO orders VALUES (1, 'active'), (2, 'cancelled'), (3, 'active');",
    );

    let before_sql = "SELECT id, status FROM orders";
    let after_sql = "SELECT id, status FROM orders WHERE status = 'active'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(atom.options.len(), 1, "{atom:?}");
    let option = &atom.options[0];
    assert_eq!(option.technique, Technique::PredicateTightenDelete);
    assert_eq!(option.statements.len(), 1, "{:?}", option.statements);
    assert!(
        option.statements[0].starts_with("DELETE FROM t WHERE"),
        "{:?}",
        option.statements
    );
    assert!(option.statements[0].contains("IS NOT TRUE"));

    harness::verify_option(&conn, "t", before_sql, after_sql, option);
}

#[test]
fn e1_null_semantics() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (id INTEGER, amount INTEGER);
         INSERT INTO orders VALUES (1, 10), (2, -5), (3, NULL);",
    );

    let before_sql = "SELECT id, amount FROM orders";
    let after_sql = "SELECT id, amount FROM orders WHERE amount > 0";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    let atom = &options.atoms[0];
    let option = &atom.options[0];

    harness::build_before(&conn, "t", before_sql);
    for stmt in &option.statements {
        conn.execute_batch(stmt)
            .unwrap_or_else(|e| panic!("apply E1 delete `{stmt}`: {e}"));
    }
    // The regression trap (research §4 E1): a bare `NOT` would wrongly
    // *keep* the NULL-amount row (id=3), since `NOT NULL` is itself NULL,
    // and `WHERE NULL` drops the row from a DELETE's own predicate — the
    // row would survive. `IS NOT TRUE` deletes it, matching the rebuild's
    // three-valued `WHERE amount > 0` semantics.
    let remaining_ids = harness::text_column(&conn, "SELECT id::TEXT FROM t ORDER BY id");
    assert_eq!(remaining_ids, vec!["1".to_string()]);

    harness::verify_option(&conn, "t", before_sql, after_sql, option);
}

#[test]
fn e4_horizon_extension_inserts_region() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE events (ts DATE, amount INTEGER);
         INSERT INTO events VALUES
           ('2023-06-01', 1),
           ('2024-06-01', 2),
           ('2025-06-01', 3);",
    );

    let before_sql = "SELECT ts, amount FROM events WHERE ts >= '2025-01-01'";
    let after_sql = "SELECT ts, amount FROM events WHERE ts >= '2024-01-01'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(atom.options.len(), 1, "{atom:?}");
    let option = &atom.options[0];
    assert_eq!(option.technique, Technique::HorizonExtensionInsert);
    assert_eq!(option.statements.len(), 1, "{:?}", option.statements);
    assert!(option.statements[0].starts_with("INSERT INTO t"));

    harness::build_before(&conn, "t", before_sql);
    for stmt in &option.statements {
        conn.execute_batch(stmt)
            .unwrap_or_else(|e| panic!("apply E4 insert `{stmt}`: {e}"));
    }
    // Exactly the [2024-01-01, 2025-01-01) region is backfilled alongside
    // the pre-existing 2025 row — the 2023 row stays excluded.
    let inserted_ts = harness::text_column(&conn, "SELECT ts::TEXT FROM t ORDER BY ts");
    assert_eq!(
        inserted_ts,
        vec!["2024-06-01".to_string(), "2025-06-01".to_string()]
    );

    harness::verify_option(&conn, "t", before_sql, after_sql, option);
}

#[test]
fn e4_idempotent_with_identity() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE events (id INTEGER, ts DATE, amount INTEGER);
         INSERT INTO events VALUES
           (1, '2023-06-01', 10),
           (2, '2024-06-01', 20),
           (3, '2025-06-01', 30);",
    );

    let before_sql = "SELECT id, ts, amount FROM events WHERE ts >= '2025-01-01'";
    let after_sql = "SELECT id, ts, amount FROM events WHERE ts >= '2024-01-01'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let mut backbuild_inputs = inputs("t", after_sql);
    backbuild_inputs.row_identity = Some(vec!["id".to_string()]);
    backbuild_inputs.not_null_columns = BTreeSet::from(["id".to_string()]);

    let options = derive_backbuild_options(&diff, &backbuild_inputs);
    let atom = &options.atoms[0];
    let option = &atom.options[0];
    assert!(
        option.rerun_safe,
        "a declared, NOT-NULL-proven row identity must make E4's INSERT rerun-safe"
    );
    assert!(
        option.statements[0].to_uppercase().contains("NOT EXISTS"),
        "expected the identity anti-join guard, got: {:?}",
        option.statements
    );

    harness::build_before(&conn, "t", before_sql);
    for stmt in &option.statements {
        conn.execute_batch(stmt)
            .unwrap_or_else(|e| panic!("apply E4 insert `{stmt}`: {e}"));
    }
    // Re-run: the anti-join guard must make this a no-op, not a duplicate
    // insert.
    for stmt in &option.statements {
        conn.execute_batch(stmt)
            .unwrap_or_else(|e| panic!("re-apply E4 insert `{stmt}`: {e}"));
    }

    harness::assert_matches_full_rebuild(&conn, "t", after_sql);
}

#[test]
fn e4_nullable_identity_is_one_shot() {
    // A declared row identity that is NOT proven NOT NULL must not get the
    // anti-join guard: `t.id = __backbuild_diff.id` never matches a NULL
    // `id` on either side, so a rerun would silently re-insert an
    // already-inserted NULL-identity row — the guard is only sound once the
    // identity is provably NOT NULL (research §4 intro "Key
    // addressability"). Undeclared-NOT-NULL is treated exactly like no
    // declared identity: no guard, `rerun_safe: false`, but the script
    // itself is still correct on a single application.
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE events (id INTEGER, ts DATE, amount INTEGER);
         INSERT INTO events VALUES
           (1, '2023-06-01', 10),
           (2, '2024-06-01', 20),
           (3, '2025-06-01', 30);",
    );

    let before_sql = "SELECT id, ts, amount FROM events WHERE ts >= '2025-01-01'";
    let after_sql = "SELECT id, ts, amount FROM events WHERE ts >= '2024-01-01'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let mut backbuild_inputs = inputs("t", after_sql);
    backbuild_inputs.row_identity = Some(vec!["id".to_string()]);
    // Deliberately no `not_null_columns` declaration for `id`.

    let options = derive_backbuild_options(&diff, &backbuild_inputs);
    let atom = &options.atoms[0];
    let option = &atom.options[0];
    assert!(
        !option.rerun_safe,
        "an identity not proven NOT NULL must not be treated as rerun-safe"
    );
    assert!(
        !option.statements[0].to_uppercase().contains("NOT EXISTS"),
        "expected no identity anti-join guard, got: {:?}",
        option.statements
    );

    // The script is still correct on a single application.
    harness::verify_option(&conn, "t", before_sql, after_sql, option);
}

#[test]
fn e4_group_key_range_admits() {
    // The carve-out: E4 where the range column is itself a GROUP BY key —
    // every group lies wholly inside or outside the difference region, so
    // extending history on a date-keyed aggregate is sound (research §4
    // E-class grain precondition).
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (order_date DATE, amount INTEGER);
         INSERT INTO orders VALUES
           ('2023-06-01', 5),
           ('2024-06-01', 10),
           ('2024-06-01', 15),
           ('2025-06-01', 20);",
    );

    let before_sql = "SELECT order_date, SUM(amount) AS total FROM orders \
                       WHERE order_date >= '2025-01-01' GROUP BY order_date";
    let after_sql = "SELECT order_date, SUM(amount) AS total FROM orders \
                      WHERE order_date >= '2024-01-01' GROUP BY order_date";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(atom.options.len(), 1, "{atom:?}");
    assert_eq!(atom.options[0].technique, Technique::HorizonExtensionInsert);

    harness::verify_option(&conn, "t", before_sql, after_sql, &atom.options[0]);
}

// ===== C1 regression: E4 upper-bound (`<`/`<=`) widening (final-review
// finding C1) — `try_e4_pair`'s `"<" | "<="` arm called
// `is_widened_lower_bound` (already the correct upper-bound widening
// condition) and then wrongly negated it, so a genuine upper-bound widening
// was refused while a narrowing was silently admitted (emitting a no-op
// `INSERT` and leaving rows a rebuild would drop). Covered for both
// comparison operators (`<` and `<=`) and both literal kinds (Number and
// the ISO-8601 Text shape) per the reviewer's fixture list. =====

#[test]
fn e4_upper_bound_strict_lt_widening_admits_number() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE readings (amount INTEGER);
         INSERT INTO readings VALUES (50), (150), (250);",
    );

    let before_sql = "SELECT amount FROM readings WHERE amount < 100";
    let after_sql = "SELECT amount FROM readings WHERE amount < 200";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(
        atom.options.len(),
        1,
        "a genuine upper-bound widening must admit E4, got {atom:?}"
    );
    assert_eq!(atom.options[0].technique, Technique::HorizonExtensionInsert);

    harness::verify_option(&conn, "t", before_sql, after_sql, &atom.options[0]);
}

#[test]
fn e4_upper_bound_strict_lt_narrowing_refuses_number() {
    // The exact C1 counter-example, adapted to `<`: narrowing the upper
    // bound must never be admitted — it would emit a no-op INSERT (nothing
    // satisfies the always-complement predicate) and silently leave rows a
    // rebuild would drop.
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE readings (amount INTEGER);
         INSERT INTO readings VALUES (50), (150), (250);",
    );

    let before_sql = "SELECT amount FROM readings WHERE amount < 200";
    let after_sql = "SELECT amount FROM readings WHERE amount < 100";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert!(
        atom.options.is_empty(),
        "an upper-bound narrowing must refuse E4, got {:?}",
        atom.options
    );
    assert_eq!(atom.inadmissible.len(), 1, "{atom:?}");
    assert!(
        atom.inadmissible[0]
            .reason
            .contains("does not provably widen"),
        "expected the range-widening refusal reason, got: {}",
        atom.inadmissible[0].reason
    );

    harness::verify_option(&conn, "t", before_sql, after_sql, &options.full_refresh);
}

#[test]
fn e4_upper_bound_lte_widening_admits_number() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE readings (amount INTEGER);
         INSERT INTO readings VALUES (50), (150), (250);",
    );

    let before_sql = "SELECT amount FROM readings WHERE amount <= 100";
    let after_sql = "SELECT amount FROM readings WHERE amount <= 200";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    let atom = &options.atoms[0];
    assert_eq!(
        atom.options.len(),
        1,
        "a genuine upper-bound widening must admit E4, got {atom:?}"
    );
    assert_eq!(atom.options[0].technique, Technique::HorizonExtensionInsert);

    harness::verify_option(&conn, "t", before_sql, after_sql, &atom.options[0]);
}

#[test]
fn e4_upper_bound_lte_narrowing_refuses_number() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE readings (amount INTEGER);
         INSERT INTO readings VALUES (50), (150), (250);",
    );

    let before_sql = "SELECT amount FROM readings WHERE amount <= 200";
    let after_sql = "SELECT amount FROM readings WHERE amount <= 100";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    let atom = &options.atoms[0];
    assert!(
        atom.options.is_empty(),
        "an upper-bound narrowing must refuse E4, got {:?}",
        atom.options
    );

    harness::verify_option(&conn, "t", before_sql, after_sql, &options.full_refresh);
}

#[test]
fn e4_upper_bound_strict_lt_widening_admits_text() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE events (ts DATE, amount INTEGER);
         INSERT INTO events VALUES
           ('2022-06-01', 1),
           ('2023-06-01', 2),
           ('2025-06-01', 3);",
    );

    let before_sql = "SELECT ts, amount FROM events WHERE ts < '2023-01-01'";
    let after_sql = "SELECT ts, amount FROM events WHERE ts < '2024-01-01'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    let atom = &options.atoms[0];
    assert_eq!(
        atom.options.len(),
        1,
        "a genuine upper-bound widening must admit E4, got {atom:?}"
    );
    assert_eq!(atom.options[0].technique, Technique::HorizonExtensionInsert);

    harness::verify_option(&conn, "t", before_sql, after_sql, &atom.options[0]);
}

#[test]
fn e4_upper_bound_strict_lt_narrowing_refuses_text() {
    // The literal C1 counter-example from the final review: `ts < X` -> `ts
    // < Y` with `Y < X` (a narrowing) must refuse, not admit a no-op INSERT
    // that silently leaves rows a rebuild would drop.
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE events (ts DATE, amount INTEGER);
         INSERT INTO events VALUES
           ('2022-06-01', 1),
           ('2023-06-01', 2),
           ('2025-06-01', 3);",
    );

    let before_sql = "SELECT ts, amount FROM events WHERE ts < '2025-01-01'";
    let after_sql = "SELECT ts, amount FROM events WHERE ts < '2024-01-01'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    let atom = &options.atoms[0];
    assert!(
        atom.options.is_empty(),
        "an upper-bound narrowing must refuse E4, got {:?}",
        atom.options
    );
    assert_eq!(atom.inadmissible.len(), 1, "{atom:?}");
    assert!(
        atom.inadmissible[0]
            .reason
            .contains("does not provably widen"),
        "expected the range-widening refusal reason, got: {}",
        atom.inadmissible[0].reason
    );

    harness::verify_option(&conn, "t", before_sql, after_sql, &options.full_refresh);
}

#[test]
fn e4_upper_bound_lte_widening_admits_text() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE events (ts DATE, amount INTEGER);
         INSERT INTO events VALUES
           ('2022-06-01', 1),
           ('2023-06-01', 2),
           ('2025-06-01', 3);",
    );

    let before_sql = "SELECT ts, amount FROM events WHERE ts <= '2023-01-01'";
    let after_sql = "SELECT ts, amount FROM events WHERE ts <= '2024-01-01'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    let atom = &options.atoms[0];
    assert_eq!(
        atom.options.len(),
        1,
        "a genuine upper-bound widening must admit E4, got {atom:?}"
    );
    assert_eq!(atom.options[0].technique, Technique::HorizonExtensionInsert);

    harness::verify_option(&conn, "t", before_sql, after_sql, &atom.options[0]);
}

#[test]
fn e4_upper_bound_lte_narrowing_refuses_text() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE events (ts DATE, amount INTEGER);
         INSERT INTO events VALUES
           ('2022-06-01', 1),
           ('2023-06-01', 2),
           ('2025-06-01', 3);",
    );

    let before_sql = "SELECT ts, amount FROM events WHERE ts <= '2025-01-01'";
    let after_sql = "SELECT ts, amount FROM events WHERE ts <= '2024-01-01'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    let atom = &options.atoms[0];
    assert!(
        atom.options.is_empty(),
        "an upper-bound narrowing must refuse E4, got {:?}",
        atom.options
    );

    harness::verify_option(&conn, "t", before_sql, after_sql, &options.full_refresh);
}

// ===== E2/F1/H (task-8-brief.md) =====

#[test]
fn e2_loosen_inserts_difference() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (id INTEGER, status TEXT);
         INSERT INTO orders VALUES (1, 'active'), (2, 'cancelled'), (3, 'active');",
    );

    let before_sql = "SELECT id, status FROM orders WHERE status = 'active'";
    let after_sql = "SELECT id, status FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert!(
        matches!(
            &atom.change,
            smelt_logical::backbuild::AtomicChange::RemovedConjunct { index: 0 }
        ),
        "expected a RemovedConjunct atom, got {:?}",
        atom.change
    );
    assert_eq!(atom.options.len(), 1, "{atom:?}");
    let option = &atom.options[0];
    assert_eq!(option.technique, Technique::FilterLoosenInsert);
    assert_eq!(option.statements.len(), 1, "{:?}", option.statements);
    assert!(
        option.statements[0].starts_with("INSERT INTO t"),
        "{:?}",
        option.statements
    );
    assert!(option.statements[0].contains("IS NOT TRUE"));

    // The difference slice is exactly the row the old predicate excluded.
    harness::build_before(&conn, "t", before_sql);
    for stmt in &option.statements {
        conn.execute_batch(stmt)
            .unwrap_or_else(|e| panic!("apply E2 insert `{stmt}`: {e}"));
    }
    let ids = harness::text_column(&conn, "SELECT id::TEXT FROM t ORDER BY id");
    assert_eq!(ids, vec!["1".to_string(), "2".to_string(), "3".to_string()]);

    harness::verify_option(&conn, "t", before_sql, after_sql, option);
}

/// Regression: gating E2's *pure-loosen* admission (no composition —
/// `added` empty) on `analysis::model_diff::collect_dependencies` (the
/// pre-fix shape of the disjointness check below) wrongly refused a removed
/// conjunct that calls a non-deterministic function like `NOW()` —
/// `collect_dependencies` rejects those because B1/D1 *re-derive* a stored
/// value from the expression, where determinism genuinely matters; E2's
/// difference `INSERT` evaluates the removed conjunct once, directly (same
/// as a real rebuild's `WHERE` clause would), so determinism was never a
/// real precondition — `build_e2_atom`'s own validator
/// (`requalify::requalify`) never checked it, and still doesn't.
///
/// Classification-only, not full oracle verification via
/// `harness::verify_option`: the predicate's truth value depends on
/// wall-clock time, which would make a row-level equality check needlessly
/// time-sensitive for something this test doesn't need to prove — the
/// emitted `INSERT`'s difference-slice shape is already oracle-covered by
/// `e2_loosen_inserts_difference`; this test only needs to prove
/// *admission still happens* for a conjunct `collect_dependencies` alone
/// would reject.
#[test]
fn e2_nondeterministic_removed_conjunct_still_admits() {
    let before_sql =
        "SELECT id, created_at FROM events WHERE created_at < NOW() - INTERVAL '30 days'";
    let after_sql = "SELECT id, created_at FROM events";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert!(
        matches!(
            &atom.change,
            smelt_logical::backbuild::AtomicChange::RemovedConjunct { index: 0 }
        ),
        "expected a RemovedConjunct (E2) atom despite the non-deterministic function, got {:?}",
        atom.change
    );
    assert_eq!(
        atom.options.len(),
        1,
        "expected E2 to admit unconditionally for a pure loosen: {atom:?}"
    );
    let option = &atom.options[0];
    assert_eq!(option.technique, Technique::FilterLoosenInsert);
    assert!(
        option.statements[0].contains("NOW()"),
        "expected the non-deterministic function preserved verbatim in the difference \
         predicate, got {:?}",
        option.statements
    );
}

/// E3 (research §4: "arbitrary predicate change = added ∧ removed
/// conjuncts: compose E1 + E2") via the disjoint-column middle path: the
/// removed conjunct (`region = 'EU'`) and the added conjunct (`qty > 0`)
/// reference different columns, so `classify_conjunct_diff` composes an E1
/// `DELETE` (for the added conjunct) with an E2 `INSERT` (for the removed
/// one) rather than refusing — proving the plan's committed-to E3 surface
/// end-to-end against a real DuckDB, distinct from the (still-refusing)
/// same-column pairs `e4_mixed_operator_refuses` et al. pin in
/// `backbuild_options.rs`.
#[test]
fn e3_disjoint_conjunct_swap_composes() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (id INTEGER, region TEXT, qty INTEGER);
         INSERT INTO orders VALUES
           (1, 'EU', 5),
           (2, 'EU', -1),
           (3, 'US', 10),
           (4, 'US', -5);",
    );

    let before_sql = "SELECT id, region, qty FROM orders WHERE region = 'EU'";
    let after_sql = "SELECT id, region, qty FROM orders WHERE qty > 0";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 2, "atoms: {:?}", options.atoms);
    assert!(
        matches!(
            &options.atoms[0].change,
            smelt_logical::backbuild::AtomicChange::AddedConjunct { index: 0 }
        ),
        "expected the added conjunct (qty > 0) to classify as E1, got {:?}",
        options.atoms[0].change
    );
    assert!(
        matches!(
            &options.atoms[1].change,
            smelt_logical::backbuild::AtomicChange::RemovedConjunct { index: 0 }
        ),
        "expected the removed conjunct (region = 'EU') to classify as E2, got {:?}",
        options.atoms[1].change
    );
    for atom in &options.atoms {
        assert_eq!(
            atom.options.len(),
            1,
            "expected both atoms admissible: {atom:?}"
        );
    }
    assert_eq!(
        options.atoms[0].options[0].technique,
        Technique::PredicateTightenDelete
    );
    assert_eq!(
        options.atoms[1].options[0].technique,
        Technique::FilterLoosenInsert
    );

    let script = assemble(&options, &targeted_of_len(2));
    assert_eq!(script.len(), 2, "{script:?}");
    assert!(
        script[0].starts_with("DELETE FROM t WHERE"),
        "H order puts DELETE before INSERT: {script:?}"
    );
    assert!(
        script[1].starts_with("INSERT INTO t"),
        "H order puts DELETE before INSERT: {script:?}"
    );

    harness::verify_script(&conn, "t", before_sql, after_sql, &script);
}

#[test]
fn f1_union_branch_insert() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE events_a (id INTEGER);
         CREATE TABLE events_b (id INTEGER);
         INSERT INTO events_a VALUES (1), (2);
         INSERT INTO events_b VALUES (3);",
    );

    let before_sql = "SELECT id, 'a' AS kind FROM events_a";
    let after_sql =
        "SELECT id, 'a' AS kind FROM events_a UNION ALL SELECT id, 'b' AS kind FROM events_b";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert!(
        matches!(
            &atom.change,
            smelt_logical::backbuild::AtomicChange::AddedSetOpBranch { index: 0 }
        ),
        "expected an AddedSetOpBranch atom, got {:?}",
        atom.change
    );
    assert_eq!(atom.options.len(), 1, "{atom:?}");
    let option = &atom.options[0];
    assert_eq!(option.technique, Technique::UnionBranchInsert);
    assert_eq!(option.statements.len(), 1, "{:?}", option.statements);
    assert!(
        option.statements[0].starts_with("INSERT INTO t (id, kind)"),
        "{:?}",
        option.statements
    );
    assert!(option.statements[0].contains("events_b"));

    harness::verify_option(&conn, "t", before_sql, after_sql, option);
}

// ===== F2 (task-11-brief.md) =====

#[test]
fn f2_discriminated_branch_delete() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE events_a (id INTEGER);
         CREATE TABLE events_b (id INTEGER);
         CREATE TABLE events_c (id INTEGER);
         INSERT INTO events_a VALUES (1), (2);
         INSERT INTO events_b VALUES (3);
         INSERT INTO events_c VALUES (4), (5);",
    );

    let before_sql = "SELECT id, 'a' AS src FROM events_a UNION ALL SELECT id, 'b' AS src FROM \
                       events_b UNION ALL SELECT id, 'c' AS src FROM events_c";
    let after_sql =
        "SELECT id, 'a' AS src FROM events_a UNION ALL SELECT id, 'c' AS src FROM events_c";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert!(
        matches!(
            &atom.change,
            smelt_logical::backbuild::AtomicChange::RemovedSetOpBranch { index: 0 }
        ),
        "expected a RemovedSetOpBranch atom, got {:?}",
        atom.change
    );
    assert_eq!(atom.options.len(), 1, "{atom:?}");
    let option = &atom.options[0];
    assert_eq!(option.technique, Technique::DiscriminatedBranchDelete);
    assert_eq!(option.statements.len(), 1, "{:?}", option.statements);
    assert_eq!(option.statements[0], "DELETE FROM t WHERE src = 'b'");

    harness::verify_option(&conn, "t", before_sql, after_sql, option);
}

#[test]
fn f2_duplicate_payload_rows_survive_correctly() {
    // events_a and events_b carry the *same* ids — a content-matching
    // DELETE (matching on `id` alone) would be ambiguous or wrong; only the
    // discriminator ('src') distinguishes which copies belong to the
    // removed branch.
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE events_a (id INTEGER);
         CREATE TABLE events_b (id INTEGER);
         INSERT INTO events_a VALUES (1), (2);
         INSERT INTO events_b VALUES (1), (2);",
    );

    let before_sql = "SELECT id, 'a' AS src FROM events_a UNION ALL SELECT id, 'b' AS src FROM \
                       events_b";
    let after_sql = "SELECT id, 'a' AS src FROM events_a";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(atom.options.len(), 1, "{atom:?}");
    let option = &atom.options[0];
    assert_eq!(option.technique, Technique::DiscriminatedBranchDelete);
    assert_eq!(option.statements[0], "DELETE FROM t WHERE src = 'b'");

    // Before the DELETE, `t` has 4 rows: (1,'a'),(2,'a'),(1,'b'),(2,'b').
    // A correct discriminated DELETE removes exactly the two 'b' rows,
    // leaving the two same-id 'a' rows untouched — `verify_option`'s
    // two-way `EXCEPT ALL` multiset check catches a wrong row count even
    // when the surviving id values coincide.
    harness::verify_option(&conn, "t", before_sql, after_sql, option);
}

// ===== C3 regression: multi-branch (UNION ALL) definitions must not
// classify atoms from the positional first-branch diffs unless every one of
// them is independently a no-op (final-review finding C3) — `diff.rs`
// computes select_list/where/skeleton over each side's own first branch
// only, while the branch multiset diff (`set_ops`) is reorder-insensitive; a
// branch reorder alone can make the first-branch pair diverge without any
// branch's own content changing, producing phantom atoms. =====

#[test]
fn c3_branch_reorder_of_non_first_branches_is_pure_noop() {
    // Reordering the second and third branches (the first branch is
    // byte-identical in both versions) never touches the positional
    // first-branch diffs at all — this is a genuine A0 no-op, handled by
    // the ordinary no-op short-circuit before the C3 gate is ever reached.
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE events_a (id INTEGER);
         CREATE TABLE events_b (id INTEGER);
         CREATE TABLE events_c (id INTEGER);
         INSERT INTO events_a VALUES (1);
         INSERT INTO events_b VALUES (2);
         INSERT INTO events_c VALUES (3);",
    );

    let before_sql = "SELECT id FROM events_a UNION ALL SELECT id FROM events_b UNION ALL \
                       SELECT id FROM events_c";
    let after_sql = "SELECT id FROM events_a UNION ALL SELECT id FROM events_c UNION ALL \
                      SELECT id FROM events_b";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(
        diff.is_noop(),
        "a reorder of non-first branches must be a pure no-op: {diff:?}"
    );

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert!(
        options.atoms.is_empty(),
        "an A0 no-op must yield no atoms (no phantom atoms from the branch reorder), got {:?}",
        options.atoms
    );

    harness::verify_option(&conn, "t", before_sql, after_sql, &options.full_refresh);
}

#[test]
fn c3_branch_reorder_with_first_branch_where_difference_refuses() {
    // The reproduced C3 counter-example: swapping the two branches turns
    // "branch A has the filter, branch B doesn't" into "branch A doesn't
    // have the filter, branch B does" — a pure reorder, semantically a
    // no-op for the UNION ALL as a whole (the same two clauses run either
    // way) — but the positional first-branch WHERE diff (branch1-before had
    // `WHERE ts >= '2025-01-01'`, branch1-after has no WHERE at all) reports
    // a single removed conjunct with nothing added, which pre-fix admitted
    // E2's unconditional "filter loosened" difference INSERT sourced from
    // the *whole* (both-branches) after-definition — duplicating every row
    // already present via the unconditional branch. Must refuse instead.
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE events (id INTEGER, ts DATE);
         INSERT INTO events VALUES
           (1, '2023-06-01'),
           (2, '2024-06-01'),
           (3, '2025-06-01'),
           (4, '2026-06-01');",
    );

    // `ts` is selected (not just filtered on) so it is a genuine stored
    // representative — otherwise E2's own requalification would refuse for
    // an unrelated reason (no representative for `ts` at all) without ever
    // exercising the C3 gate this test targets.
    let before_sql = "SELECT id, ts FROM events WHERE ts >= '2025-01-01' UNION ALL SELECT id, ts \
                       FROM events";
    let after_sql = "SELECT id, ts FROM events UNION ALL SELECT id, ts FROM events WHERE ts >= \
                      '2025-01-01'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(
        options.atoms.len(),
        1,
        "expected exactly one refusal atom, no phantom atoms: {:?}",
        options.atoms
    );
    let atom = &options.atoms[0];
    assert!(
        atom.options.is_empty(),
        "a branch reorder that makes the first-branch WHERE diff diverge must refuse, got {:?}",
        atom.options
    );
    assert_eq!(atom.inadmissible.len(), 1, "{atom:?}");
    assert!(
        atom.inadmissible[0]
            .reason
            .contains("multi-branch definition changed"),
        "expected the C3 multi-branch refusal reason, got: {}",
        atom.inadmissible[0].reason
    );

    let targeted = assemble(&options, &targeted_of_len(1));
    assert!(
        targeted.is_empty(),
        "a refused atom must yield no composed targeted script, got {targeted:?}"
    );

    harness::verify_option(&conn, "t", before_sql, after_sql, &options.full_refresh);
}

// ===== H composites (task-8-brief.md) =====

/// B2 (rename) + B1 (self-derived add) + E1 (filter tightened) in one diff,
/// composed through the real classifier and `assemble` — the H order is
/// `rename → alter/add → delete → update/merge → insert → drop`.
///
/// `unit_price`'s own added-column expression is the bare rename target
/// `list_price` itself — this is the B2 "one dropped, two identical added"
/// tie-break shape (research §4 B2, `classify_rename_loser`): `price` is
/// dropped, and BOTH `list_price` and `unit_price` are added with the
/// identical expression `price`, so `pair_renames` pins the
/// lexicographically-first name (`list_price`) as the rename target and
/// classifies `unit_price` as a B1-shaped atom that reads the *winning
/// rename's target name directly* (`build_b1_option(loser, renamed_to,
/// ..)`), not through the general representative-requalification path
/// (which cannot reference a renamed-away name at all — see
/// `classify_rename_loser`'s own doc comment). This makes the rename→add
/// dependency real, not merely juxtaposed: `unit_price`'s `UPDATE`
/// literally reads a column (`list_price`) that only exists once the rename
/// has run.
///
/// Why a shuffled script fails (argued by construction, not a second test —
/// each claim below was checked by reasoning through the actual statements):
/// - `UPDATE t SET unit_price = list_price` run before `ALTER TABLE t
///   RENAME COLUMN price TO list_price` errors outright: `t` has no
///   `list_price` column yet (still `price`) — DuckDB reports "column
///   \"list_price\" does not exist", not a wrong answer. This is the
///   concrete instance of the general H rationale (research §4 "H.
///   Composites"): "renames first so requalified expressions reference
///   final names."
/// - `UPDATE t SET unit_price = list_price` run before `ALTER TABLE t ADD
///   COLUMN unit_price INTEGER` errors identically: `t` has no target
///   column `unit_price` to assign into yet.
/// - `DELETE FROM t WHERE (qty > 0) IS NOT TRUE` running *before* the
///   rename/alter/update trio is, by contrast, NOT load-bearing here: its
///   predicate only ever addresses `qty`, a representative untouched by
///   either the rename or the add, so this particular reordering still
///   produces the oracle-correct result — an honest limit of this
///   composite's coverage (E1's `try_e1` can only requalify against the
///   *unchanged* representative set, which by construction never includes
///   a renamed-away name, so no admitted E1 atom in this classifier can
///   ever be made to depend on a rename this way).
#[test]
fn h_composite_rename_add_tighten() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE orders (id INTEGER, price INTEGER, qty INTEGER);
         INSERT INTO orders VALUES (1, 10, 3), (2, 20, 0), (3, 30, 5);",
    );

    let before_sql = "SELECT id, price, qty FROM orders";
    let after_sql = "SELECT id, price AS unit_price, price AS list_price, qty FROM orders \
                      WHERE qty > 0";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let mut added_column_types = BTreeMap::new();
    added_column_types.insert("unit_price".to_string(), "INTEGER".to_string());
    let backbuild_inputs = BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.to_string(),
        row_identity: None,
        not_null_columns: BTreeSet::new(),
        added_column_types,
        sources: BTreeMap::new(),
    };

    let options = derive_backbuild_options(&diff, &backbuild_inputs);
    assert_eq!(options.atoms.len(), 3, "atoms: {:?}", options.atoms);
    for atom in &options.atoms {
        assert_eq!(
            atom.options.len(),
            1,
            "expected every atom admissible: {atom:?}"
        );
    }
    assert!(
        matches!(
            &options.atoms[0].change,
            smelt_logical::backbuild::AtomicChange::RenamedColumn { from, to }
                if from == "price" && to == "list_price"
        ),
        "expected price->list_price to win the lexicographic tie-break, got {:?}",
        options.atoms[0].change
    );
    assert_eq!(
        options.atoms[1].options[0].statements,
        vec![
            "ALTER TABLE t ADD COLUMN unit_price INTEGER".to_string(),
            "UPDATE t SET unit_price = list_price".to_string(),
        ],
        "expected unit_price's UPDATE to read the rename's target name directly: {:?}",
        options.atoms[1]
    );

    let selection = targeted_of_len(3);
    let script = assemble(&options, &selection);
    assert_eq!(script.len(), 4, "{script:?}");
    assert_eq!(
        script,
        vec![
            "ALTER TABLE t RENAME COLUMN price TO list_price".to_string(),
            "ALTER TABLE t ADD COLUMN unit_price INTEGER".to_string(),
            "UPDATE t SET unit_price = list_price".to_string(),
            "DELETE FROM t WHERE (qty > 0) IS NOT TRUE".to_string(),
        ],
        "H order: rename -> alter/add -> delete -> update/merge -> insert -> drop"
    );

    harness::verify_script(&conn, "t", before_sql, after_sql, &script);
}

/// A B1 column add confined to branch 0, composed with an appended UNION
/// ALL branch, driven entirely through the real classifier (research §4
/// F1's edited-survivor generalization): branch 0's own edit is exactly
/// what the top-level (whole-definition) SELECT-list diff already
/// describes, so `derive_backbuild_options` admits both atoms without any
/// hand-assembly.
///
/// `mid` is declared *between* `id` and `a` in `after_sql`, but
/// `ALTER TABLE … ADD COLUMN` always appends physically at the end — the
/// table's physical order becomes `(id, a, b, mid)`, not the declared
/// `(id, mid, a, b)`. A positional `INSERT INTO t SELECT id, mid, a, b FROM
/// …` would silently misassign `mid`'s values into physical column `a`
/// (same-typed, so no error — just wrong data): `emit_branch_insert`'s
/// explicit target column list (research §4 "H. Composites") is what makes
/// this oracle-equal despite the reorder.
#[test]
fn h_composite_branch_add_plus_column_add_classifier_driven() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE t1 (id INTEGER, a INTEGER, b INTEGER);
         CREATE TABLE t2 (id INTEGER, a INTEGER, b INTEGER);
         INSERT INTO t1 VALUES (1, 10, 100), (2, 20, 200);
         INSERT INTO t2 VALUES (3, 30, 300);",
    );

    let before_sql = "SELECT id, a, b FROM t1";
    let after_sql = "SELECT id, id * 100 AS mid, a, b FROM t1 \
                      UNION ALL SELECT id, id * 100 AS mid, a, b FROM t2";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let mut added_column_types = BTreeMap::new();
    added_column_types.insert("mid".to_string(), "INTEGER".to_string());
    let backbuild_inputs = BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.to_string(),
        row_identity: None,
        not_null_columns: BTreeSet::new(),
        added_column_types,
        sources: BTreeMap::new(),
    };

    let options = derive_backbuild_options(&diff, &backbuild_inputs);
    assert_eq!(options.atoms.len(), 2, "atoms: {:?}", options.atoms);
    assert!(
        matches!(
            &options.atoms[0].change,
            smelt_logical::backbuild::AtomicChange::AddedColumn { name } if name == "mid"
        ),
        "expected atom 0 to be the added 'mid' column, got {:?}",
        options.atoms[0].change
    );
    assert!(
        matches!(
            &options.atoms[1].change,
            smelt_logical::backbuild::AtomicChange::AddedSetOpBranch { index: 0 }
        ),
        "expected atom 1 to be the appended UNION ALL branch, got {:?}",
        options.atoms[1].change
    );
    for atom in &options.atoms {
        assert_eq!(
            atom.options.len(),
            1,
            "expected every atom admissible: {atom:?}"
        );
    }
    assert_eq!(
        options.atoms[0].options[0].technique,
        Technique::SelfDerivedColumnAdd
    );
    assert_eq!(
        options.atoms[1].options[0].technique,
        Technique::UnionBranchInsert
    );

    let script = assemble(&options, &targeted_of_len(2));
    assert_eq!(script.len(), 3, "{script:?}");
    assert!(
        script[0].starts_with("ALTER TABLE t ADD COLUMN mid"),
        "{script:?}"
    );
    assert!(script[1].starts_with("UPDATE t SET mid"), "{script:?}");
    assert!(
        script[2].starts_with("INSERT INTO t ("),
        "the INSERT must carry an explicit target column list, got {:?}",
        script[2]
    );

    harness::verify_script(&conn, "t", before_sql, after_sql, &script);
}

/// The same composite as
/// [`h_composite_branch_add_plus_column_add_classifier_driven`] — the B1
/// add and the appended branch are still derived through the real
/// classifier — plus a third, hand-added blocked atom (a G1-shaped refusal,
/// standing in for "some atom this diff also needs has no admissible
/// option"): partial application is never offered (research §2 "Refusal
/// posture"), so the composed targeted script must be empty and
/// `FullRefresh` remains the model's only option, regardless of how many
/// *other* atoms in the same diff were admissible.
#[test]
fn h_composite_with_blocked_atom_yields_only_full_refresh() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE t1 (id INTEGER, a INTEGER, b INTEGER);
         CREATE TABLE t2 (id INTEGER, a INTEGER, b INTEGER);
         INSERT INTO t1 VALUES (1, 10, 100), (2, 20, 200);
         INSERT INTO t2 VALUES (3, 30, 300);",
    );

    let before_sql = "SELECT id, a, b FROM t1";
    let after_sql = "SELECT id, id * 100 AS mid, a, b FROM t1 \
                      UNION ALL SELECT id, id * 100 AS mid, a, b FROM t2";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let mut added_column_types = BTreeMap::new();
    added_column_types.insert("mid".to_string(), "INTEGER".to_string());
    let backbuild_inputs = BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.to_string(),
        row_identity: None,
        not_null_columns: BTreeSet::new(),
        added_column_types,
        sources: BTreeMap::new(),
    };

    let mut options = derive_backbuild_options(&diff, &backbuild_inputs);
    assert_eq!(options.atoms.len(), 2, "atoms: {:?}", options.atoms);

    let blocked_atom = smelt_logical::backbuild::AtomAnalysis {
        change: smelt_logical::backbuild::AtomicChange::Skeleton {
            reason: "GROUP BY changed".to_string(),
        },
        options: Vec::new(),
        inadmissible: vec![smelt_logical::backbuild::BackbuildRefusal {
            atom: "skeleton".to_string(),
            reason: "G1 (grain change) — GROUP BY changed".to_string(),
        }],
    };
    options.atoms.push(blocked_atom);

    // Partial application is never offered: even though two of the three
    // atoms have an admissible option, the blocked (G1-refused) third atom
    // makes the whole composed targeted script empty.
    let targeted = assemble(&options, &targeted_of_len(3));
    assert!(
        targeted.is_empty(),
        "a blocked atom must block the composed targeted script, got {targeted:?}"
    );
    assert!(
        options.atoms[2]
            .inadmissible
            .iter()
            .any(|r| r.reason.contains("G1")),
        "expected the refusal to name the blocked G1 atom, got {:?}",
        options.atoms[2].inadmissible
    );

    // FullRefresh remains the model's only option — still oracle-verified.
    let full_refresh_script = assemble(&options, &Selection::FullRefresh);
    assert_eq!(full_refresh_script, options.full_refresh.statements);
    harness::verify_option(&conn, "t", before_sql, after_sql, &options.full_refresh);
}
