//! Phase 3 (TB-2) — end-to-end check that the compile-time seed
//! inferencer recognises `DATE` and `TIMESTAMP`-shaped columns when
//! reached through the Salsa data plane (`project_seeds`).
//!
//! The unit-level coverage lives in `smelt-core::seeds::tests`; this
//! test guards the Salsa wiring so a regression in either layer is
//! caught.

use std::fs;

use smelt_db::{project_seeds, Database};
use smelt_types::DataType;
use tempfile::TempDir;

#[test]
fn project_seeds_infers_date_and_timestamp_columns() {
    let tmp = TempDir::new().unwrap();
    let project_root = tmp.path().to_path_buf();
    let seeds_dir = project_root.join("seeds");
    fs::create_dir_all(&seeds_dir).unwrap();
    fs::write(
        seeds_dir.join("orders.csv"),
        "order_id,order_date,order_timestamp\n\
         1,2025-01-01,2025-01-01 08:00:00\n\
         2,2025-01-02,2025-01-02 09:30:00\n\
         3,2025-01-03,2025-01-03 12:15:30\n",
    )
    .unwrap();

    // Phase 1 (`smelt_yml.md` Surface §"Top-level keys"): the unified
    // `paths:` list is the single scan list. The default is `["models"]`,
    // so a workspace whose seeds live under `seeds/` must declare it
    // explicitly.
    fs::write(
        project_root.join("smelt.yml"),
        "name: temporal_inference_fixture\nversion: 1\npaths:\n  - seeds\ntargets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n",
    )
    .unwrap();

    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());

    let seeds = project_seeds(&db, project);
    assert_eq!(seeds.len(), 1);
    let seed = &seeds[0];
    assert_eq!(seed.name, "orders");

    let cols: std::collections::HashMap<_, _> = seed.columns.iter().cloned().collect();
    assert_eq!(cols["order_id"], DataType::Integer);
    assert_eq!(cols["order_date"], DataType::Date);
    assert_eq!(
        cols["order_timestamp"],
        DataType::Timestamp {
            with_timezone: false
        }
    );
}
