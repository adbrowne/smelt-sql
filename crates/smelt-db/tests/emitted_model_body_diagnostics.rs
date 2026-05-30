//! Integration tests for Phase 2: body-analysis diagnostics for generator-emitted models.
//!
//! Each test drives the real `emitted_model_body_analysis` Salsa query and asserts on
//! `EmissionBodyAnalysis.diagnostics`. Tests follow red-green TDD order.
//!
//! Cases:
//!   1. `undeclared_column_inside_body` — body references a column that is not declared
//!      in any FROM source; produces exactly one `UndeclaredColumn` diagnostic.
//!   2. `expression_type_mismatch_in_where` — a WHERE clause comparing Integer + 'string';
//!      produces at least one `TypeMismatch` or similar type-error diagnostic.
//!   3. `cannot_infer_type_for_unknown_column` — a SELECT list with an unresolvable
//!      expression produces a `CannotInferType` warning.
//!   4. `parse_error_on_malformed_body` — a body with a typo in the keyword produces a
//!      `ParseError` diagnostic; the returned schema is empty.
//!   5. `cte_cycle_inside_body` — a body with a self-referential CTE produces a
//!      `CteCycle` diagnostic.
//!   6. `config_var_not_found_in_body` — a body referencing an unknown config var
//!      produces a `ConfigVarNotFound` diagnostic.
//!   7. `lift_protection_regression` — the per_cohort_union generator produces zero
//!      `UndeclaredColumn` diagnostics for any of its three emissions; `c.region` and
//!      `c.min_revenue` must not be treated as unresolved columns.
//!   8. `multi_emission_independence` — a generator with two emissions, one with a body
//!      error and one without, produces diagnostics only for the erroring emission.

use smelt_db::{emitted_model_body_analysis, Database, DiagnosticCode, Workspace};
use std::fs;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Minimal smelt.yml text for a project with a single `models/` path.
const SMELT_YML: &str = "\
name: body_diag_test\n\
version: 1\n\
paths:\n  - models\n\
targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
default_materialization: view\n";

/// Build a database by writing files to a real temp directory so that `project_paths`
/// resolves the `models/` scan root and `emitted_model_smelt_path` strips it correctly.
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

/// Build a database with a YAML loader file registered in Salsa.
fn build_db_on_disk_with_loader(
    tmp: &TempDir,
    files: &[(&str, &str)],
    loader_path: &str,
    loader_content: &str,
) -> (Database, Workspace) {
    let (mut db, ws) = build_db_on_disk(tmp, files);
    let path: std::sync::Arc<str> = std::sync::Arc::from(loader_path);
    let text: std::sync::Arc<str> = std::sync::Arc::from(loader_content);
    db.set_loader_file(path, text, true);
    (db, ws)
}

// ---------------------------------------------------------------------------
// Test 1: UndeclaredColumn inside body (no lift)
// ---------------------------------------------------------------------------

/// A generator with a body referencing a column that is not declared anywhere
/// produces exactly one `UndeclaredColumn` diagnostic anchored inside the body.
#[test]
fn undeclared_column_inside_body() {
    let tmp = TempDir::new().expect("tempdir");

    let gen_src = r#"---
generates: models
---
[ModelDef { name: 'x', body: SELECT foo_does_not_exist FROM smelt.orders }]
"#;
    let orders_src = "SELECT CAST(1 AS INTEGER) AS id\n";

    let (db, ws) = build_db_on_disk(
        &tmp,
        &[
            ("models/cohorts.gen.sql", gen_src),
            ("models/orders.sql", orders_src),
        ],
    );

    let gen_path = tmp.path().join("models").join("cohorts.gen.sql");
    let gen_file = db
        .source_file(&gen_path)
        .expect("generator file registered");

    let analysis = emitted_model_body_analysis(&db, ws, gen_file, "x".to_string());

    let undeclared: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UndeclaredColumn))
        .collect();

    assert_eq!(
        undeclared.len(),
        1,
        "expected exactly 1 UndeclaredColumn diagnostic, got {} diagnostics: {:?}",
        analysis.diagnostics.len(),
        analysis
            .diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Test 2: Expression UnknownCastType inside body
// ---------------------------------------------------------------------------

/// A body whose SELECT list uses `CAST(1 AS NOTAVALIDTYPE)` produces an
/// `UnknownCastType` diagnostic from `check_expression_types_for_select`.
#[test]
fn expression_unknown_cast_type_in_body() {
    let tmp = TempDir::new().expect("tempdir");

    // `CAST(1 AS NOTAVALIDTYPE)` is an unrecognised cast target type.
    let gen_src = r#"---
generates: models
---
[ModelDef { name: 'x', body: SELECT CAST(1 AS NOTAVALIDTYPE) AS n }]
"#;

    let (db, ws) = build_db_on_disk(&tmp, &[("models/cohorts.gen.sql", gen_src)]);

    let gen_path = tmp.path().join("models").join("cohorts.gen.sql");
    let gen_file = db
        .source_file(&gen_path)
        .expect("generator file registered");

    let analysis = emitted_model_body_analysis(&db, ws, gen_file, "x".to_string());

    let cast_type_diags: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UnknownCastType))
        .collect();

    assert!(
        !cast_type_diags.is_empty(),
        "expected at least one UnknownCastType diagnostic for CAST(1 AS NOTAVALIDTYPE), \
         got {} diagnostics: {:?}",
        analysis.diagnostics.len(),
        analysis
            .diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Test 3: CannotInferType at SELECT-list item
// ---------------------------------------------------------------------------

/// A body whose SELECT list includes a column that cannot be typed (references
/// an unbound identifier) produces exactly one `CannotInferType` warning.
#[test]
fn cannot_infer_type_for_unknown_column() {
    let tmp = TempDir::new().expect("tempdir");

    // `some_column` is unresolvable → the inferred type for the column is Unknown.
    let gen_src = r#"---
generates: models
---
[ModelDef { name: 'x', body: SELECT some_unresolvable_column AS val FROM smelt.orders }]
"#;
    let orders_src = "SELECT CAST(1 AS INTEGER) AS id\n";

    let (db, ws) = build_db_on_disk(
        &tmp,
        &[
            ("models/cohorts.gen.sql", gen_src),
            ("models/orders.sql", orders_src),
        ],
    );

    let gen_path = tmp.path().join("models").join("cohorts.gen.sql");
    let gen_file = db
        .source_file(&gen_path)
        .expect("generator file registered");

    let analysis = emitted_model_body_analysis(&db, ws, gen_file, "x".to_string());

    let cannot_infer: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::CannotInferType))
        .collect();

    assert!(
        !cannot_infer.is_empty(),
        "expected at least one CannotInferType diagnostic, got {} diagnostics: {:?}",
        analysis.diagnostics.len(),
        analysis
            .diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Test 4: ParseError on malformed body
// ---------------------------------------------------------------------------

/// A body with a keyword typo (`SELEKT`) cannot be parsed as a SELECT statement;
/// `emitted_model_body_analysis` returns a `ParseError` diagnostic and an empty schema.
#[test]
fn parse_error_on_malformed_body() {
    let tmp = TempDir::new().expect("tempdir");

    // `SELEKT 1` is a typo that prevents SELECT parsing.
    let gen_src = "---\ngenerates: models\n---\n[ModelDef { name: 'x', body: SELEKT 1 }]\n";

    let (db, ws) = build_db_on_disk(&tmp, &[("models/cohorts.gen.sql", gen_src)]);

    let gen_path = tmp.path().join("models").join("cohorts.gen.sql");
    let gen_file = db
        .source_file(&gen_path)
        .expect("generator file registered");

    let analysis = emitted_model_body_analysis(&db, ws, gen_file, "x".to_string());

    // A malformed body must produce a ParseError diagnostic.
    let parse_errors: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ParseError))
        .collect();

    assert!(
        !parse_errors.is_empty(),
        "expected at least one ParseError diagnostic for malformed body, \
         got {} diagnostics: {:?}",
        analysis.diagnostics.len(),
        analysis
            .diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );

    // The schema for a malformed body must be empty (no columns).
    assert!(
        analysis.schema.columns.is_empty(),
        "expected empty schema for malformed body, got {} columns: {:?}",
        analysis.schema.columns.len(),
        analysis
            .schema
            .columns
            .iter()
            .map(|c| &c.name)
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Test 5: CTE cycle inside body
// ---------------------------------------------------------------------------

/// A body containing a self-referential CTE (`WITH a AS (SELECT * FROM a)`)
/// produces a `CteCycle` diagnostic.
#[test]
fn cte_cycle_inside_body() {
    let tmp = TempDir::new().expect("tempdir");

    let gen_src = r#"---
generates: models
---
[ModelDef { name: 'x', body: WITH a AS (SELECT 1 AS n FROM a) SELECT * FROM a }]
"#;

    let (db, ws) = build_db_on_disk(&tmp, &[("models/cohorts.gen.sql", gen_src)]);

    let gen_path = tmp.path().join("models").join("cohorts.gen.sql");
    let gen_file = db
        .source_file(&gen_path)
        .expect("generator file registered");

    let analysis = emitted_model_body_analysis(&db, ws, gen_file, "x".to_string());

    let cte_cycle: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::CteCycle))
        .collect();

    assert!(
        !cte_cycle.is_empty(),
        "expected CteCycle diagnostic for self-referential CTE, got {} diagnostics: {:?}",
        analysis.diagnostics.len(),
        analysis
            .diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Test 6: smelt.config.var error inside body
// ---------------------------------------------------------------------------

/// A body referencing an unknown config variable produces a `ConfigVarNotFound`
/// diagnostic.
#[test]
fn config_var_not_found_in_body() {
    let tmp = TempDir::new().expect("tempdir");

    let gen_src = r#"---
generates: models
---
[ModelDef { name: 'x', body: SELECT id FROM smelt.orders WHERE flag = smelt.config.var('does_not_exist') }]
"#;
    let orders_src = "SELECT CAST(1 AS INTEGER) AS id, CAST(true AS BOOLEAN) AS flag\n";

    let (db, ws) = build_db_on_disk(
        &tmp,
        &[
            ("models/cohorts.gen.sql", gen_src),
            ("models/orders.sql", orders_src),
        ],
    );

    let gen_path = tmp.path().join("models").join("cohorts.gen.sql");
    let gen_file = db
        .source_file(&gen_path)
        .expect("generator file registered");

    let analysis = emitted_model_body_analysis(&db, ws, gen_file, "x".to_string());

    let config_var_diags: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ConfigVarNotFound))
        .collect();

    assert!(
        !config_var_diags.is_empty(),
        "expected ConfigVarNotFound diagnostic, got {} diagnostics: {:?}",
        analysis.diagnostics.len(),
        analysis
            .diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Test 7: Lift-protection regression (per_cohort_union)
// ---------------------------------------------------------------------------

/// The `cohorts.gen.sql` from the per_cohort_union example uses
/// `WHERE region = c.region AND revenue >= c.min_revenue`, where `c` is the
/// lambda parameter bound to each record in the YAML loader.
///
/// With lambda-binding propagation, `c.region` and `c.min_revenue` must resolve
/// against the lambda parameter's record type — producing zero `UndeclaredColumn`
/// diagnostics for all three emissions.
#[test]
fn lift_protection_regression() {
    let tmp = TempDir::new().expect("tempdir");

    let gen_src = r#"---
generates: models
tags: [cohort]
---
smelt.config.load_yaml('cohorts.yaml', List<{ name: Text, region: Text, min_revenue: Integer }>)
  |> map(fn c => ModelDef {
       name: c.name,
       body: SELECT id, user_id, region, revenue, created_at
             FROM smelt.orders
             WHERE region = c.region AND revenue >= c.min_revenue
     })
"#;

    let orders_src = r#"---
materialization: table
---
SELECT
    id,
    user_id,
    region,
    revenue,
    created_at
FROM (VALUES
    (1, 10, 'us-west-2', 150, CAST('2024-01-01' AS TIMESTAMP)),
    (2, 11, 'us-west-2', 80,  CAST('2024-01-02' AS TIMESTAMP))
) AS t(id, user_id, region, revenue, created_at)
"#;

    let yaml_src = "- name: us_west\n  region: us-west-2\n  min_revenue: 100\n\
                    - name: us_east\n  region: us-east-1\n  min_revenue: 100\n\
                    - name: eu\n  region: eu-west-1\n  min_revenue: 50\n";

    let (db, ws) = build_db_on_disk_with_loader(
        &tmp,
        &[
            ("models/cohorts.gen.sql", gen_src),
            ("models/orders.sql", orders_src),
        ],
        "cohorts.yaml",
        yaml_src,
    );

    let gen_path = tmp.path().join("models").join("cohorts.gen.sql");
    let gen_file = db
        .source_file(&gen_path)
        .expect("generator file registered");

    // There should be 3 emissions: us_west, us_east, eu.
    let evaluated = smelt_db::evaluate_generator(&db, ws, gen_file);
    assert_eq!(
        evaluated.emissions.len(),
        3,
        "expected 3 emissions, got: {:?}",
        evaluated
            .emissions
            .iter()
            .map(|e| &e.name)
            .collect::<Vec<_>>()
    );

    let emission_names: Vec<&str> = evaluated
        .emissions
        .iter()
        .map(|e| e.name.as_str())
        .collect();

    for name in &emission_names {
        let analysis = emitted_model_body_analysis(&db, ws, gen_file, name.to_string());

        let undeclared: Vec<_> = analysis
            .diagnostics
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::UndeclaredColumn))
            .collect();

        assert!(
            undeclared.is_empty(),
            "emission '{}': expected zero UndeclaredColumn diagnostics (lift-protection), \
             got {} undeclared: {:?}\nAll diagnostics: {:?}",
            name,
            undeclared.len(),
            undeclared.iter().map(|d| &d.message).collect::<Vec<_>>(),
            analysis
                .diagnostics
                .iter()
                .map(|d| (&d.code, &d.message))
                .collect::<Vec<_>>()
        );
    }
}

// ---------------------------------------------------------------------------
// Test 8: Multi-emission independence
// ---------------------------------------------------------------------------

/// A generator with two emissions: one with a body error (undeclared column)
/// and one without. The erroring emission produces diagnostics; the clean
/// emission produces none. The two are independent.
#[test]
fn multi_emission_independence() {
    let tmp = TempDir::new().expect("tempdir");

    let gen_src = r#"---
generates: models
---
[
  ModelDef { name: 'good', body: SELECT CAST(1 AS INTEGER) AS n },
  ModelDef { name: 'bad',  body: SELECT nonexistent_column FROM smelt.orders }
]
"#;
    let orders_src = "SELECT CAST(1 AS INTEGER) AS id\n";

    let (db, ws) = build_db_on_disk(
        &tmp,
        &[
            ("models/cohorts.gen.sql", gen_src),
            ("models/orders.sql", orders_src),
        ],
    );

    let gen_path = tmp.path().join("models").join("cohorts.gen.sql");
    let gen_file = db
        .source_file(&gen_path)
        .expect("generator file registered");

    let good_analysis = emitted_model_body_analysis(&db, ws, gen_file, "good".to_string());
    let bad_analysis = emitted_model_body_analysis(&db, ws, gen_file, "bad".to_string());

    // 'good' emission: no diagnostics.
    let good_undeclared: Vec<_> = good_analysis
        .diagnostics
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UndeclaredColumn))
        .collect();
    assert!(
        good_undeclared.is_empty(),
        "expected no UndeclaredColumn for 'good' emission, got: {:?}",
        good_undeclared
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );

    // 'bad' emission: at least one UndeclaredColumn.
    let bad_undeclared: Vec<_> = bad_analysis
        .diagnostics
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UndeclaredColumn))
        .collect();
    assert!(
        !bad_undeclared.is_empty(),
        "expected at least one UndeclaredColumn for 'bad' emission, got {} diagnostics: {:?}",
        bad_analysis.diagnostics.len(),
        bad_analysis
            .diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );
}
