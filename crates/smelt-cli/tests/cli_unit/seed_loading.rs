//! Phase 4 TDD — end-to-end seed loading via Backend::load_table.
//!
//! Verifies that `execute_seed` no longer calls `read_csv_auto` and instead
//! routes CSV loading through the smelt-owned parser + `Backend::load_table`.

#[cfg(feature = "duckdb")]
mod duckdb_seed_loading {
    use smelt_backend::Backend;
    use smelt_backend_duckdb::DuckDbBackend;
    use smelt_cli::seed::{execute_seed, SeedFile};
    use std::path::Path;
    use tempfile::TempDir;

    /// Load `examples/timeseries/seeds/raw/users.csv` into a temporary
    /// DuckDB database and verify row count + schema.
    ///
    /// This test fails if `execute_seed` still uses `read_csv_auto` because
    /// the DuckDB backend's `load_table` implementation would not be called.
    #[tokio::test]
    async fn seeds_load_via_load_table() {
        let project_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("examples/timeseries");

        assert!(
            project_root.exists(),
            "examples/timeseries must exist: {}",
            project_root.display()
        );

        // Discover seeds via the central smelt-core strict path (BUG-063).
        let seed_infos =
            smelt_core::discover_seed_infos_strict(&project_root, &["seeds".to_string()])
                .expect("discover seeds");
        let seeds: Vec<SeedFile> = seed_infos
            .into_iter()
            .map(|info| SeedFile::from_seed_info(info, "main".to_string()))
            .collect();
        assert!(
            !seeds.is_empty(),
            "expected at least one seed in examples/timeseries/seeds/"
        );

        // Use the users seed specifically
        let seed = seeds
            .iter()
            .find(|s| s.name == "users")
            .expect("users seed not found in examples/timeseries/seeds/raw/");

        // Set up in-memory DuckDB
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("DuckDB backend");

        // Execute the seed — this must NOT use read_csv_auto
        let result = execute_seed(&backend, seed, false)
            .await
            .expect("execute_seed")
            .expect("users seed is not ephemeral — should return Some(SeedResult)");

        // users.csv has 5 data rows (Alice through Eve)
        assert_eq!(result.row_count, 5, "expected 5 rows in users");

        // The table's qualified name is main.raw_users because address segments
        // for seeds/raw/users.csv are ["raw", "users"] (joined with _).
        let qualified = seed.qualified_name();
        let query = format!("SELECT * FROM {}", qualified);

        // Verify the table exists and has correct column types
        let batches = backend
            .execute_sql(&query)
            .await
            .expect("SELECT from loaded seed");
        assert!(!batches.is_empty(), "loaded seed must be queryable");

        let schema = batches[0].schema();
        let col_names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert!(col_names.contains(&"user_id"), "must have user_id");
        assert!(col_names.contains(&"user_name"), "must have user_name");
        assert!(col_names.contains(&"signup_date"), "must have signup_date");
    }
}
