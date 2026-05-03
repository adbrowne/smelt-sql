//! TDD tests for Phase 5: Sidecar YAML semantics validation.
//!
//! Covers:
//! - Column set mismatch between sidecar and CSV header
//! - Type coercion failures as hard errors
//! - `nullable: false` violations
//! - `name:` key rejection
//! - `view`/`materialized_view` materialization rejection

use smelt_core::seeds::csv::CsvRecord;
use smelt_core::seeds::sidecar::{parse_sidecar, SeedMaterialization, SeedSidecar, SidecarColumn};
use smelt_core::seeds::validate::validate_against_sidecar;
use smelt_types::DataType;
use std::fs;
use tempfile::TempDir;

fn make_record(row_index: usize, fields: &[Option<&str>]) -> CsvRecord {
    CsvRecord {
        row_index,
        fields: fields.iter().map(|f| f.map(|s| s.to_string())).collect(),
    }
}

// ---------------------------------------------------------------------------
// parse_sidecar tests
// ---------------------------------------------------------------------------

#[test]
fn name_override_rejected_for_seed() {
    let tmp = TempDir::new().unwrap();
    let yml = tmp.path().join("regions.yml");
    fs::write(
        &yml,
        r#"
name: foo.bar
columns:
  - name: region_id
    type: INTEGER
"#,
    )
    .unwrap();

    let err = parse_sidecar(&yml).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("name"),
        "Expected error about 'name:' key, got: {msg}"
    );
}

#[test]
fn sidecar_parses_valid_yaml() {
    let tmp = TempDir::new().unwrap();
    let yml = tmp.path().join("regions.yml");
    fs::write(
        &yml,
        r#"
columns:
  - name: region_id
    type: INTEGER
    nullable: false
  - name: region_name
    type: VARCHAR
    nullable: true
    description: "Human-readable region name"
materialization: ephemeral
"#,
    )
    .unwrap();

    let sidecar = parse_sidecar(&yml).unwrap();
    assert_eq!(sidecar.materialization, SeedMaterialization::Ephemeral);
    let cols = sidecar.columns.unwrap();
    assert_eq!(cols.len(), 2);
    assert_eq!(cols[0].name, "region_id");
    assert_eq!(cols[0].data_type, DataType::Integer);
    assert!(!cols[0].nullable);
    assert_eq!(cols[1].name, "region_name");
    assert!(
        matches!(
            &cols[1].data_type,
            DataType::Text | DataType::Varchar { .. }
        ),
        "expected text/varchar type, got {:?}",
        cols[1].data_type
    );
    assert!(cols[1].nullable);
    assert_eq!(
        cols[1].description.as_deref(),
        Some("Human-readable region name")
    );
}

#[test]
fn sidecar_defaults_materialization_to_table() {
    let tmp = TempDir::new().unwrap();
    let yml = tmp.path().join("data.yml");
    fs::write(&yml, "columns:\n  - name: x\n    type: INTEGER\n").unwrap();

    let sidecar = parse_sidecar(&yml).unwrap();
    assert_eq!(sidecar.materialization, SeedMaterialization::Table);
}

#[test]
fn sidecar_parses_view_materialization() {
    let tmp = TempDir::new().unwrap();
    let yml = tmp.path().join("data.yml");
    fs::write(&yml, "materialization: view\n").unwrap();

    let sidecar = parse_sidecar(&yml).unwrap();
    assert_eq!(sidecar.materialization, SeedMaterialization::View);
}

#[test]
fn sidecar_parses_materialized_view_materialization() {
    let tmp = TempDir::new().unwrap();
    let yml = tmp.path().join("data.yml");
    fs::write(&yml, "materialization: materialized_view\n").unwrap();

    let sidecar = parse_sidecar(&yml).unwrap();
    assert_eq!(
        sidecar.materialization,
        SeedMaterialization::MaterializedView
    );
}

// ---------------------------------------------------------------------------
// validate_against_sidecar tests
// ---------------------------------------------------------------------------

#[test]
fn column_set_must_match_header_extra_in_csv() {
    // sidecar declares [a, b], CSV header is a, b, c → mismatch
    let sidecar = SeedSidecar {
        materialization: SeedMaterialization::Table,
        columns: Some(vec![
            SidecarColumn {
                name: "a".to_string(),
                data_type: DataType::Text,
                nullable: true,
                description: None,
            },
            SidecarColumn {
                name: "b".to_string(),
                data_type: DataType::Text,
                nullable: true,
                description: None,
            },
        ]),
    };
    let headers = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let rows: Vec<CsvRecord> = vec![];
    let path = std::path::Path::new("test.csv");

    let err = validate_against_sidecar(path, &headers, &rows, &sidecar).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("mismatch") || msg.contains("Mismatch") || msg.contains("column"),
        "Expected mismatch error, got: {msg}"
    );
}

#[test]
fn column_set_must_match_header_extra_in_sidecar() {
    // sidecar declares [a, b, d], CSV header is a, b, c → mismatch
    let sidecar = SeedSidecar {
        materialization: SeedMaterialization::Table,
        columns: Some(vec![
            SidecarColumn {
                name: "a".to_string(),
                data_type: DataType::Text,
                nullable: true,
                description: None,
            },
            SidecarColumn {
                name: "b".to_string(),
                data_type: DataType::Text,
                nullable: true,
                description: None,
            },
            SidecarColumn {
                name: "d".to_string(),
                data_type: DataType::Text,
                nullable: true,
                description: None,
            },
        ]),
    };
    let headers = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let rows: Vec<CsvRecord> = vec![];
    let path = std::path::Path::new("test.csv");

    let err = validate_against_sidecar(path, &headers, &rows, &sidecar).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("mismatch") || msg.contains("Mismatch") || msg.contains("column"),
        "Expected mismatch error, got: {msg}"
    );
}

#[test]
fn column_set_matches_in_any_order() {
    // sidecar declares [b, a], CSV header is a, b → OK (any order)
    let sidecar = SeedSidecar {
        materialization: SeedMaterialization::Table,
        columns: Some(vec![
            SidecarColumn {
                name: "b".to_string(),
                data_type: DataType::Text,
                nullable: true,
                description: None,
            },
            SidecarColumn {
                name: "a".to_string(),
                data_type: DataType::Text,
                nullable: true,
                description: None,
            },
        ]),
    };
    let headers = vec!["a".to_string(), "b".to_string()];
    let rows: Vec<CsvRecord> = vec![];
    let path = std::path::Path::new("test.csv");

    validate_against_sidecar(path, &headers, &rows, &sidecar).unwrap();
}

#[test]
fn type_coercion_failure_is_hard_error() {
    // sidecar pins column x as INTEGER, CSV value is "not_a_number"
    let sidecar = SeedSidecar {
        materialization: SeedMaterialization::Table,
        columns: Some(vec![SidecarColumn {
            name: "x".to_string(),
            data_type: DataType::Integer,
            nullable: true,
            description: None,
        }]),
    };
    let headers = vec!["x".to_string()];
    let rows = vec![make_record(1, &[Some("not_a_number")])];
    let path = std::path::Path::new("test.csv");

    let err = validate_against_sidecar(path, &headers, &rows, &sidecar).unwrap_err();
    let msg = err.to_string();
    // Must contain ALL THREE: file path, row index, and column name.
    assert!(
        msg.contains("test.csv") && msg.contains("row") && msg.contains("column"),
        "Expected file path, row index, and column name in error, got: {msg}"
    );
    assert!(
        msg.contains("not_a_number") || msg.contains("x"),
        "Expected value/column name in error, got: {msg}"
    );
}

#[test]
fn nullable_false_blocks_null() {
    // sidecar pins column y as nullable: false, CSV has empty cell (NULL) in y
    let sidecar = SeedSidecar {
        materialization: SeedMaterialization::Table,
        columns: Some(vec![SidecarColumn {
            name: "y".to_string(),
            data_type: DataType::Text,
            nullable: false,
            description: None,
        }]),
    };
    let headers = vec!["y".to_string()];
    // Empty cell = NULL
    let rows = vec![make_record(1, &[None])];
    let path = std::path::Path::new("test.csv");

    let err = validate_against_sidecar(path, &headers, &rows, &sidecar).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("null")
            || msg.contains("NULL")
            || msg.contains("nullable")
            || msg.contains("y"),
        "Expected nullable error, got: {msg}"
    );
}

#[test]
fn view_or_materialized_view_rejected() {
    // sidecar with materialization: view → ValidationError
    let sidecar_view = SeedSidecar {
        materialization: SeedMaterialization::View,
        columns: None,
    };
    let headers = vec!["id".to_string()];
    let rows: Vec<CsvRecord> = vec![];
    let path = std::path::Path::new("test.csv");

    let err = validate_against_sidecar(path, &headers, &rows, &sidecar_view).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("view") || msg.contains("View"),
        "Expected unsupported materialization error, got: {msg}"
    );

    // materialized_view too
    let sidecar_mv = SeedSidecar {
        materialization: SeedMaterialization::MaterializedView,
        columns: None,
    };
    let err2 = validate_against_sidecar(path, &headers, &rows, &sidecar_mv).unwrap_err();
    let msg2 = err2.to_string();
    assert!(
        msg2.contains("materialized") || msg2.contains("Materialized"),
        "Expected unsupported materialization error, got: {msg2}"
    );
}
