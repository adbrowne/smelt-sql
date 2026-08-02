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

use std::collections::BTreeMap;

use duckdb::Connection;

use smelt_logical::backbuild::{
    assemble, definition_diff, derive_backbuild_options, BackbuildInputs, DefinitionDiff, Selection,
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
