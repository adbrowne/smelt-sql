//! Integration tests for Phase 3: `model_input_constraints` walks set-operation chains.
//!
//! Before this fix, only the leading SELECT branch's FROM entries were collected.
//! After the fix, every branch of UNION/INTERSECT/EXCEPT contributes its FROM entries
//! to the input-constraint list.
//!
//! Cases:
//!   1. `two_branch_union_all_collects_both_refs` — `SELECT … FROM smelt.foo UNION ALL
//!      SELECT … FROM smelt.bar` must produce input entries for both `foo` and `bar`.
//!   2. `three_branch_union_all_per_cohort_union` — reproducer for
//!      `examples/per_cohort_union/all_cohorts_unioned.sql`: three UNION ALL branches
//!      over `us_west`, `us_east`, `eu` must each appear in the input list with five
//!      typed columns.
//!   3. `intersect_and_except_collect_both_refs` — UNION (bare), INTERSECT, and EXCEPT
//!      variants produce the same input-list shape as UNION ALL.
//!   4. `single_select_regression` — a model with no set operation still works (no
//!      regression).
//!   5. `duplicate_ref_deduplicated` — the same `smelt.<path>` appearing in two UNION
//!      branches produces exactly one `InputConstraint` entry (deduplicated).

use smelt_db::{model_input_constraints, Database, Workspace};
use smelt_types::DataType;
use std::fs;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Shared helper
// ---------------------------------------------------------------------------

const SMELT_YML: &str = "\
name: set_ops_test\n\
version: 1\n\
paths:\n  - models\n\
targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
default_materialization: view\n";

fn build_db_on_disk(tmp: &TempDir, files: &[(&str, &str)]) -> (Database, Workspace) {
    let root = tmp.path().to_path_buf();
    fs::write(root.join("smelt.yml"), SMELT_YML).expect("write smelt.yml");

    let mut handles = Vec::new();
    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), SMELT_YML.to_string());

    for (rel_path, content) in files {
        let full_path = root.join(rel_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).expect("create dirs");
        }
        fs::write(&full_path, content).expect("write file");
        let sf = db.set_source_file(full_path, content.to_string(), root.clone());
        handles.push(sf);
    }

    db.set_workspace(handles, vec![project]);
    let ws = db.workspace();
    (db, ws)
}

// ---------------------------------------------------------------------------
// Test 1: Two-branch UNION ALL collects both refs
// ---------------------------------------------------------------------------

/// `SELECT id, name FROM smelt.models.foo UNION ALL SELECT id, name FROM smelt.models.bar`
/// must produce two input entries: one for `foo` and one for `bar`.
#[test]
fn two_branch_union_all_collects_both_refs() {
    let tmp = TempDir::new().expect("tempdir");

    let foo_src = "SELECT CAST(1 AS INTEGER) AS id, CAST('a' AS TEXT) AS name\n";
    let bar_src = "SELECT CAST(2 AS INTEGER) AS id, CAST('b' AS TEXT) AS name\n";
    let consumer_src =
        "SELECT id, name FROM smelt.models.foo\nUNION ALL\nSELECT id, name FROM smelt.models.bar\n";

    let (db, ws) = build_db_on_disk(
        &tmp,
        &[
            ("models/foo.sql", foo_src),
            ("models/bar.sql", bar_src),
            ("models/consumer.sql", consumer_src),
        ],
    );

    let consumer_path = tmp.path().join("models/consumer.sql");
    let consumer_file = db
        .source_file(&consumer_path)
        .expect("consumer file registered");
    let constraints = model_input_constraints(&db, ws, consumer_file);

    let ref_names: Vec<&str> = constraints.iter().map(|c| c.ref_name.as_str()).collect();

    assert!(
        ref_names.contains(&"foo"),
        "input constraints must include 'foo', got: {:?}",
        ref_names
    );
    assert!(
        ref_names.contains(&"bar"),
        "input constraints must include 'bar', got: {:?}",
        ref_names
    );
    assert_eq!(
        constraints.len(),
        2,
        "should have exactly 2 input entries (foo + bar), got: {:?}",
        ref_names
    );
}

// ---------------------------------------------------------------------------
// Test 2: Three-branch UNION ALL — reproducer for per_cohort_union
// ---------------------------------------------------------------------------

/// Reproducer: `all_cohorts_unioned.sql` unions `us_west`, `us_east`, `eu`.
/// All three must appear in the input constraint list, each with five typed columns.
#[test]
fn three_branch_union_all_per_cohort_union() {
    let tmp = TempDir::new().expect("tempdir");

    // Generator that emits the three cohort models with typed columns
    let gen_src = r#"---
generates: models
tags: [cohort]
---
[
  ModelDef {
    name: 'us_west',
    body: SELECT CAST(1 AS SMALLINT) AS id,
                 CAST(10 AS SMALLINT) AS user_id,
                 CAST('us_west' AS TEXT) AS region,
                 CAST(100 AS SMALLINT) AS revenue,
                 CAST('2024-01-01'::TIMESTAMP AS TIMESTAMP) AS created_at
  },
  ModelDef {
    name: 'us_east',
    body: SELECT CAST(2 AS SMALLINT) AS id,
                 CAST(20 AS SMALLINT) AS user_id,
                 CAST('us_east' AS TEXT) AS region,
                 CAST(200 AS SMALLINT) AS revenue,
                 CAST('2024-01-02'::TIMESTAMP AS TIMESTAMP) AS created_at
  },
  ModelDef {
    name: 'eu',
    body: SELECT CAST(3 AS SMALLINT) AS id,
                 CAST(30 AS SMALLINT) AS user_id,
                 CAST('eu' AS TEXT) AS region,
                 CAST(300 AS SMALLINT) AS revenue,
                 CAST('2024-01-03'::TIMESTAMP AS TIMESTAMP) AS created_at
  }
]
"#;

    let consumer_src = "\
SELECT id, user_id, region, revenue, created_at FROM smelt.cohorts.us_west
UNION ALL
SELECT id, user_id, region, revenue, created_at FROM smelt.cohorts.us_east
UNION ALL
SELECT id, user_id, region, revenue, created_at FROM smelt.cohorts.eu
";

    let (db, ws) = build_db_on_disk(
        &tmp,
        &[
            ("models/cohorts.gen.sql", gen_src),
            ("models/all_cohorts_unioned.sql", consumer_src),
        ],
    );

    let consumer_path = tmp.path().join("models/all_cohorts_unioned.sql");
    let consumer_file = db
        .source_file(&consumer_path)
        .expect("consumer file registered");
    let constraints = model_input_constraints(&db, ws, consumer_file);

    let ref_names: Vec<&str> = constraints.iter().map(|c| c.ref_name.as_str()).collect();

    assert!(
        ref_names.contains(&"us_west"),
        "input constraints must include 'us_west', got: {:?}",
        ref_names
    );
    assert!(
        ref_names.contains(&"us_east"),
        "input constraints must include 'us_east', got: {:?}",
        ref_names
    );
    assert!(
        ref_names.contains(&"eu"),
        "input constraints must include 'eu', got: {:?}",
        ref_names
    );
    assert_eq!(
        constraints.len(),
        3,
        "should have exactly 3 input entries (us_west, us_east, eu), got: {:?}",
        ref_names
    );

    // Each entry must have 5 typed columns
    for constraint in constraints.iter() {
        assert_eq!(
            constraint.required_columns.len(),
            5,
            "entry '{}' should have 5 columns (id, user_id, region, revenue, created_at), \
             got: {:?}",
            constraint.ref_name,
            constraint.required_columns.keys().collect::<Vec<_>>()
        );

        // Verify the columns have resolved types (not Unknown)
        for (col_name, col_constraint) in &constraint.required_columns {
            if let Some(typed_col) = &col_constraint.expected_type {
                assert_ne!(
                    typed_col.data_type,
                    DataType::unknown_dynamic(),
                    "column '{}' in '{}' should be a concrete type, got Unknown",
                    col_name,
                    constraint.ref_name
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Test 3: INTERSECT and EXCEPT variants
// ---------------------------------------------------------------------------

/// UNION (bare, no ALL), INTERSECT, and EXCEPT variants must also collect FROM entries from
/// both branches, the same as the UNION ALL code path.
#[test]
fn intersect_and_except_collect_both_refs() {
    let tmp = TempDir::new().expect("tempdir");

    let foo_src = "SELECT CAST(1 AS INTEGER) AS id\n";
    let bar_src = "SELECT CAST(2 AS INTEGER) AS id\n";

    // UNION (bare — no ALL)
    let consumer_union =
        "SELECT id FROM smelt.models.foo\nUNION\nSELECT id FROM smelt.models.bar\n";
    // INTERSECT
    let consumer_intersect =
        "SELECT id FROM smelt.models.foo\nINTERSECT\nSELECT id FROM smelt.models.bar\n";
    // EXCEPT
    let consumer_except =
        "SELECT id FROM smelt.models.foo\nEXCEPT\nSELECT id FROM smelt.models.bar\n";

    // Test UNION (bare, no ALL)
    {
        let tmp_union = TempDir::new().expect("tempdir");
        let (db, ws) = build_db_on_disk(
            &tmp_union,
            &[
                ("models/foo.sql", foo_src),
                ("models/bar.sql", bar_src),
                ("models/consumer.sql", consumer_union),
            ],
        );

        let consumer_path = tmp_union.path().join("models/consumer.sql");
        let consumer_file = db
            .source_file(&consumer_path)
            .expect("consumer file registered");
        let constraints = model_input_constraints(&db, ws, consumer_file);

        let ref_names: Vec<&str> = constraints.iter().map(|c| c.ref_name.as_str()).collect();
        assert!(
            ref_names.contains(&"foo"),
            "UNION: input constraints must include 'foo', got: {:?}",
            ref_names
        );
        assert!(
            ref_names.contains(&"bar"),
            "UNION: input constraints must include 'bar', got: {:?}",
            ref_names
        );
        assert_eq!(
            constraints.len(),
            2,
            "UNION: should have exactly 2 entries (foo + bar), got: {:?}",
            ref_names
        );
    }

    // Test INTERSECT
    {
        let tmp2 = TempDir::new().expect("tempdir");
        let (db, ws) = build_db_on_disk(
            &tmp2,
            &[
                ("models/foo.sql", foo_src),
                ("models/bar.sql", bar_src),
                ("models/consumer.sql", consumer_intersect),
            ],
        );

        let consumer_path = tmp2.path().join("models/consumer.sql");
        let consumer_file = db
            .source_file(&consumer_path)
            .expect("consumer file registered");
        let constraints = model_input_constraints(&db, ws, consumer_file);

        let ref_names: Vec<&str> = constraints.iter().map(|c| c.ref_name.as_str()).collect();
        assert!(
            ref_names.contains(&"foo"),
            "INTERSECT: input constraints must include 'foo', got: {:?}",
            ref_names
        );
        assert!(
            ref_names.contains(&"bar"),
            "INTERSECT: input constraints must include 'bar', got: {:?}",
            ref_names
        );
    }

    // Test EXCEPT
    {
        let (db, ws) = build_db_on_disk(
            &tmp,
            &[
                ("models/foo.sql", foo_src),
                ("models/bar.sql", bar_src),
                ("models/consumer.sql", consumer_except),
            ],
        );

        let consumer_path = tmp.path().join("models/consumer.sql");
        let consumer_file = db
            .source_file(&consumer_path)
            .expect("consumer file registered");
        let constraints = model_input_constraints(&db, ws, consumer_file);

        let ref_names: Vec<&str> = constraints.iter().map(|c| c.ref_name.as_str()).collect();
        assert!(
            ref_names.contains(&"foo"),
            "EXCEPT: input constraints must include 'foo', got: {:?}",
            ref_names
        );
        assert!(
            ref_names.contains(&"bar"),
            "EXCEPT: input constraints must include 'bar', got: {:?}",
            ref_names
        );
    }
}

// ---------------------------------------------------------------------------
// Test 4: Single-SELECT regression — no change to single-branch models
// ---------------------------------------------------------------------------

/// A plain single-SELECT model must still produce the same input constraint
/// as before the Phase 3 changes (no regression).
#[test]
fn single_select_regression() {
    let tmp = TempDir::new().expect("tempdir");

    let upstream_src = "SELECT CAST(1 AS INTEGER) AS id, CAST('a' AS TEXT) AS name\n";
    let consumer_src = "SELECT id, name FROM smelt.models.upstream\n";

    let (db, ws) = build_db_on_disk(
        &tmp,
        &[
            ("models/upstream.sql", upstream_src),
            ("models/consumer.sql", consumer_src),
        ],
    );

    let consumer_path = tmp.path().join("models/consumer.sql");
    let consumer_file = db
        .source_file(&consumer_path)
        .expect("consumer file registered");
    let constraints = model_input_constraints(&db, ws, consumer_file);

    assert_eq!(
        constraints.len(),
        1,
        "single-SELECT consumer should have exactly 1 input entry, got: {:?}",
        constraints
            .iter()
            .map(|c| c.ref_name.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(constraints[0].ref_name, "upstream");
    assert!(
        constraints[0].required_columns.contains_key("id"),
        "should require 'id' from upstream"
    );
    assert!(
        constraints[0].required_columns.contains_key("name"),
        "should require 'name' from upstream"
    );
}

// ---------------------------------------------------------------------------
// Test 5: Duplicate ref in two branches → single InputConstraint entry
// ---------------------------------------------------------------------------

/// `FROM smelt.models.foo UNION ALL ... FROM smelt.models.foo` — the same ref
/// in both branches must produce exactly one `InputConstraint` (deduplicated),
/// with columns from both uses merged.
#[test]
fn duplicate_ref_deduplicated() {
    let tmp = TempDir::new().expect("tempdir");

    let foo_src = "SELECT CAST(1 AS INTEGER) AS id, CAST('a' AS TEXT) AS name\n";
    // Both branches reference `foo`
    let consumer_src =
        "SELECT id FROM smelt.models.foo\nUNION ALL\nSELECT id FROM smelt.models.foo\n";

    let (db, ws) = build_db_on_disk(
        &tmp,
        &[
            ("models/foo.sql", foo_src),
            ("models/consumer.sql", consumer_src),
        ],
    );

    let consumer_path = tmp.path().join("models/consumer.sql");
    let consumer_file = db
        .source_file(&consumer_path)
        .expect("consumer file registered");
    let constraints = model_input_constraints(&db, ws, consumer_file);

    assert_eq!(
        constraints.len(),
        1,
        "duplicate ref 'foo' in both UNION branches should yield exactly 1 input entry, \
         got: {:?}",
        constraints
            .iter()
            .map(|c| c.ref_name.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(constraints[0].ref_name, "foo");
    assert!(
        constraints[0].required_columns.contains_key("id"),
        "merged entry must require 'id'"
    );
}
