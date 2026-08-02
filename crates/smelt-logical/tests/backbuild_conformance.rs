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
    assert!(atom.inadmissible.is_empty());
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
    assert!(atom.inadmissible.is_empty());

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
    assert!(atom.inadmissible.is_empty());
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
