//! TDD tests for Phase 5: Ephemeral seed semantics.
//!
//! Covers:
//! - ephemeral seed produces no table (skipped in discover/execute path)
//! - ephemeral seed compiles to VALUES CTE when referenced

use smelt_core::seeds::sidecar::SeedMaterialization;
use smelt_core::seeds::{discover_seed_infos_with_sidecars, SeedInfo};
use smelt_types::DataType;
use std::fs;
use tempfile::TempDir;

#[test]
fn ephemeral_seed_produces_no_table() {
    // A seed with materialization: ephemeral should NOT be in the list of seeds
    // that need to be loaded (i.e., it should be filtered out before execute_seed).
    let tmp = TempDir::new().unwrap();
    let models_dir = tmp.path().join("models").join("lookup");
    fs::create_dir_all(&models_dir).unwrap();

    // Write CSV
    fs::write(
        models_dir.join("regions.csv"),
        "region_id,region_name\n1,North\n2,South\n",
    )
    .unwrap();

    // Write sidecar with ephemeral
    fs::write(
        models_dir.join("regions.yml"),
        "materialization: ephemeral\n",
    )
    .unwrap();

    // Write a regular table seed
    fs::write(
        models_dir.join("status.csv"),
        "code,label\nactive,Active\ninactive,Inactive\n",
    )
    .unwrap();

    // discover_seed_infos_with_sidecars is the new function that reads sidecars
    let seeds = discover_seed_infos_with_sidecars(tmp.path(), &["models".to_string()]);

    // Count seeds by materialization
    let ephemeral_count = seeds
        .iter()
        .filter(|s| {
            s.sidecar
                .as_ref()
                .map(|sc| sc.materialization == SeedMaterialization::Ephemeral)
                .unwrap_or(false)
        })
        .count();
    let table_count = seeds
        .iter()
        .filter(|s| {
            s.sidecar
                .as_ref()
                .map(|sc| sc.materialization != SeedMaterialization::Ephemeral)
                .unwrap_or(true) // no sidecar = table (default)
        })
        .count();

    assert_eq!(ephemeral_count, 1, "regions should be ephemeral");
    assert_eq!(table_count, 1, "status should be table");

    // The ephemeral seed is identified so the loader can skip it
    let regions = seeds
        .iter()
        .find(|s| s.name == "regions")
        .expect("regions seed should be found");
    assert!(
        regions.is_ephemeral(),
        "regions seed should report is_ephemeral() = true"
    );
}

#[test]
fn ephemeral_seed_values_cte_body() {
    // build_values_cte produces a valid CTE body for a simple 2-col, 2-row seed
    use smelt_core::seeds::ephemeral::build_values_cte;

    let seed = SeedInfo {
        name: "regions".to_string(),
        path: std::path::PathBuf::from("/tmp/regions.csv"),
        columns: vec![
            ("region_id".to_string(), DataType::Integer),
            ("region_name".to_string(), DataType::Text),
        ],
        address_segments: vec!["lookup".to_string(), "regions".to_string()],
        sidecar: None,
    };

    // rows: 2 data rows
    let rows = vec![
        smelt_core::seeds::csv::CsvRecord {
            row_index: 1,
            fields: vec![Some("1".to_string()), Some("North".to_string())],
        },
        smelt_core::seeds::csv::CsvRecord {
            row_index: 2,
            fields: vec![Some("2".to_string()), Some("South".to_string())],
        },
    ];

    let cte_body = build_values_cte(&seed.columns, &rows);

    // Must contain VALUES
    assert!(
        cte_body.contains("VALUES"),
        "CTE body should contain VALUES, got: {cte_body}"
    );
    // Must contain CAST for integer column
    assert!(
        cte_body.contains("CAST") || cte_body.contains("cast"),
        "CTE body should contain CAST, got: {cte_body}"
    );
    // Must contain column values
    assert!(
        cte_body.contains("1"),
        "CTE body should contain row value 1"
    );
    assert!(
        cte_body.contains("North"),
        "CTE body should contain 'North'"
    );
    assert!(
        cte_body.contains("South"),
        "CTE body should contain 'South'"
    );
}
