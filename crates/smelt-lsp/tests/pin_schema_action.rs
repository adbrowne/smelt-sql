//! Phase 7 — "Pin schema to sidecar YAML" code-action tests.
//!
//! Test: `action_writes_sidecar_yml`
//!   Invoking the PinSeedSchema action on a MissingSeedSidecar diagnostic
//!   produces a PinSeedSchema CodeActionKind with the correct sidecar path.
//!
//! Test: `action_includes_inferred_types`
//!   The PinSeedSchemaSuggestion carries the inferred column types from the
//!   CSV file, so the LSP handler can render the YAML body.
//!
//! Test: `rendered_yaml_round_trips_through_sidecar_validator`
//!   The YAML string the PinSeedSchema handler would write can be parsed back
//!   by `smelt_core::seeds::sidecar::parse_sidecar_from_str` without error.

use std::path::PathBuf;

use smelt_db::code_actions::{generate_all_code_actions, CodeActionKind};
use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Test: rendered_yaml_round_trips_through_sidecar_validator
// ---------------------------------------------------------------------------

/// The YAML that the PinSeedSchema handler renders must parse cleanly through
/// `parse_sidecar_from_str`.  This guards against the handler producing YAML
/// that the sidecar validator then rejects.
#[test]
fn rendered_yaml_round_trips_through_sidecar_validator() {
    let tmp = TempDir::new().unwrap();
    let seeds_dir = tmp.path().join("seeds");
    std::fs::create_dir_all(&seeds_dir).unwrap();

    // CSV with varied types: INTEGER, VARCHAR, DATE, BOOLEAN.
    let csv_path = seeds_dir.join("events.csv");
    let csv_content =
        "event_id,event_name,event_date,is_cancelled\n1,Launch,2025-01-01,false\n2,Review,2025-06-15,true\n";
    std::fs::write(&csv_path, csv_content).unwrap();

    // Step 1: infer columns using the same inferencer as the handler.
    let (headers, rows_iter) = smelt_core::read_csv(&csv_path).expect("CSV must be readable");
    let rows: Vec<_> = rows_iter.filter_map(|r| r.ok()).collect();
    let inferred_columns = smelt_core::infer_columns(&rows, &headers, None);

    assert!(
        !inferred_columns.is_empty(),
        "inferencer must produce columns; got none"
    );

    // Step 2: build the YAML string the PinSeedSchema handler would write.
    let mut yaml_content = String::from("columns:\n");
    for (name, dtype) in &inferred_columns {
        yaml_content.push_str(&format!("  - name: {}\n    type: {}\n", name, dtype));
    }

    // Step 3: write to a temp file so parse_sidecar can read it.
    let sidecar_path = csv_path.with_extension("yml");
    std::fs::write(&sidecar_path, &yaml_content).unwrap();

    // Step 4: parse through the sidecar validator — must succeed.
    let sidecar = smelt_core::parse_sidecar(&sidecar_path)
        .unwrap_or_else(|e| panic!("rendered YAML failed to parse: {e}\nYAML:\n{yaml_content}"));

    // Step 5: the parsed sidecar must have the same number of columns.
    let sidecar_cols = sidecar
        .columns
        .expect("sidecar columns must be present after round-trip");
    assert_eq!(
        sidecar_cols.len(),
        inferred_columns.len(),
        "column count must survive the round-trip"
    );

    // Verify each column name is preserved.
    let sidecar_names: Vec<&str> = sidecar_cols.iter().map(|c| c.name.as_str()).collect();
    let inferred_names: Vec<&str> = inferred_columns.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        sidecar_names, inferred_names,
        "column names must be in CSV header order after round-trip"
    );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_db_with_csv(
    project_root: PathBuf,
    csv_path: PathBuf,
    csv_content: &str,
) -> (Database, Workspace, SourceFile) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    let sf = db.set_source_file(csv_path, csv_content.to_string(), project_root);
    db.set_workspace(vec![sf], vec![project]);
    let ws = db.workspace();
    (db, ws, sf)
}

// ---------------------------------------------------------------------------
// Test: action_writes_sidecar_yml
// ---------------------------------------------------------------------------

/// When the MissingSeedSidecar diagnostic is present, `generate_all_code_actions`
/// must return a `PinSeedSchema` action whose `sidecar_path` is the `.yml` sibling.
#[test]
fn action_writes_sidecar_yml() {
    let tmp = TempDir::new().unwrap();
    let seeds_dir = tmp.path().join("seeds");
    std::fs::create_dir_all(&seeds_dir).unwrap();

    let csv_path = seeds_dir.join("orders.csv");
    let csv_content = "id,amount\n1,100\n2,200\n";
    std::fs::write(&csv_path, csv_content).unwrap();

    let (db, ws, sf) = build_db_with_csv(tmp.path().to_path_buf(), csv_path.clone(), csv_content);

    let diags = file_diagnostics(&db, ws, sf);

    let sidecar_diag = diags
        .iter()
        .find(|d| d.code == Some(DiagnosticCode::MissingSeedSidecar))
        .expect("expected a MissingSeedSidecar diagnostic");

    let actions = generate_all_code_actions(sidecar_diag, csv_content, "");

    let pin_actions: Vec<_> = actions
        .iter()
        .filter_map(|a| {
            if let CodeActionKind::PinSeedSchema(s) = a {
                Some(s)
            } else {
                None
            }
        })
        .collect();

    assert!(
        !pin_actions.is_empty(),
        "expected a PinSeedSchema code action; got: {actions:#?}"
    );

    let action = pin_actions[0];
    let expected_sidecar = csv_path.with_extension("yml");
    assert_eq!(
        action.sidecar_path, expected_sidecar,
        "sidecar_path should be the .yml sibling of the CSV"
    );
    assert_eq!(
        action.csv_path, csv_path,
        "csv_path should point to the CSV file"
    );
}

// ---------------------------------------------------------------------------
// Test: action_includes_inferred_types
// ---------------------------------------------------------------------------

/// The PinSeedSchemaSuggestion must include inferred columns from the CSV,
/// so the handler can render the YAML body with `name:` and `type:` entries.
#[test]
fn action_includes_inferred_types() {
    let tmp = TempDir::new().unwrap();
    let seeds_dir = tmp.path().join("seeds");
    std::fs::create_dir_all(&seeds_dir).unwrap();

    let csv_path = seeds_dir.join("products.csv");
    // Deliberately include varied types: integer, text, boolean.
    let csv_content = "sku,price,in_stock\nA001,9,true\nB002,12,false\n";
    std::fs::write(&csv_path, csv_content).unwrap();

    let (db, ws, sf) = build_db_with_csv(tmp.path().to_path_buf(), csv_path.clone(), csv_content);

    let diags = file_diagnostics(&db, ws, sf);

    let sidecar_diag = diags
        .iter()
        .find(|d| d.code == Some(DiagnosticCode::MissingSeedSidecar))
        .expect("expected a MissingSeedSidecar diagnostic");

    let actions = generate_all_code_actions(sidecar_diag, csv_content, "");

    let action = actions
        .iter()
        .find_map(|a| {
            if let CodeActionKind::PinSeedSchema(s) = a {
                Some(s)
            } else {
                None
            }
        })
        .expect("expected a PinSeedSchema action");

    // The suggestion must carry inferred columns.
    assert!(
        !action.inferred_columns.is_empty(),
        "inferred_columns should be non-empty; got: {:?}",
        action.inferred_columns
    );

    // Verify columns are in CSV order: sku, price, in_stock.
    let names: Vec<&str> = action
        .inferred_columns
        .iter()
        .map(|c| c.0.as_str())
        .collect();
    assert_eq!(names, vec!["sku", "price", "in_stock"]);

    // price must be INTEGER (9 and 12 are integers).
    let price = action
        .inferred_columns
        .iter()
        .find(|(n, _)| n == "price")
        .expect("price column expected");
    assert_eq!(
        price.1,
        smelt_types::DataType::Integer,
        "price should be inferred as Integer"
    );
}
