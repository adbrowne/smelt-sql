//! End-to-end test for ephemeral seed CTE injection.
//!
//! Verifies that a model referencing an ephemeral seed (`materialization: ephemeral`
//! in a sidecar YAML) gets the seed rows inlined as a VALUES CTE at compile
//! time, and that no persistent table is created for the seed.
//!
//! Phase 5 review checklist: "Compile-time VALUES-CTE rewrite has a real-fixture
//! test that DuckDB executes."

#[cfg(feature = "duckdb")]
mod ephemeral_seed_cte_injection {
    use smelt_backend::Backend;
    use smelt_backend_duckdb::DuckDbBackend;
    use smelt_cli::{compiler::EphemeralResolver, Config, ModelDiscovery, SqlCompiler};
    use std::path::Path;
    use tempfile::TempDir;

    fn project_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("examples/ephemeral_demo")
    }

    /// Build an `EphemeralResolver` pre-loaded with ephemeral seed CTEs
    /// discovered from the project's configured paths.
    fn build_seed_resolver(project_dir: &std::path::Path, config: &Config) -> EphemeralResolver {
        use smelt_core::discover_seed_infos_with_sidecars;
        use smelt_core::seeds::csv::read_csv;
        use smelt_core::seeds::ephemeral::build_values_cte;

        let seeds = discover_seed_infos_with_sidecars(project_dir, &config.paths);
        let ephemeral_seeds: Vec<_> = seeds.iter().filter(|s| s.is_ephemeral()).collect();

        let mut resolver = EphemeralResolver::empty();
        let mut seed_ctes = Vec::new();

        for seed in ephemeral_seeds {
            if let Ok((_headers, rows_iter)) = read_csv(&seed.path) {
                let rows: Vec<_> = rows_iter.filter_map(|r| r.ok()).collect();
                let canonical_name = seed.address_segments.join("_");
                let col_names: Vec<&str> = seed.columns.iter().map(|(n, _)| n.as_str()).collect();
                let alias_with_cols =
                    format!("__smelt_{}({})", canonical_name, col_names.join(", "));
                let cte_body = build_values_cte(&seed.columns, &rows);
                seed_ctes.push((canonical_name, alias_with_cols, cte_body));
            }
        }

        resolver.add_seed_ctes(seed_ctes);
        resolver
    }

    /// The key assertion: `region_report` returns rows matching `regions.csv`,
    /// and no persistent `lookup_regions` table is created.
    #[tokio::test]
    async fn ephemeral_seed_values_cte_executes_in_duckdb() {
        let project_dir = project_dir();
        assert!(
            project_dir.exists(),
            "examples/ephemeral_demo must exist: {}",
            project_dir.display()
        );

        let config: Config =
            serde_yaml::from_str(&std::fs::read_to_string(project_dir.join("smelt.yml")).unwrap())
                .unwrap();

        // Discover all models and find region_report.
        let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
        let models = discovery.discover_models().unwrap();
        let region_report = models
            .iter()
            .find(|m| m.name == "region_report")
            .expect("region_report model must exist in examples/ephemeral_demo/models/");

        // Build a resolver with the ephemeral seed CTEs.
        let resolver = build_seed_resolver(&project_dir, &config);

        // Compile region_report with ephemeral injection.
        let default_target = config.targets.values().next().unwrap();
        let compiler = SqlCompiler::new(config.clone(), default_target);
        let compiled = compiler
            .compile_with_ephemerals(region_report, "main", &resolver)
            .expect("compile_with_ephemerals must succeed");

        // The compiled SQL must contain a WITH / VALUES CTE (ephemeral inlined).
        assert!(
            compiled.sql.contains("WITH") || compiled.sql.contains("with"),
            "Compiled SQL must contain a WITH clause (VALUES CTE injected):\n{}",
            compiled.sql
        );
        assert!(
            compiled.sql.contains("VALUES") || compiled.sql.contains("values"),
            "Compiled SQL must contain VALUES (ephemeral seed rows):\n{}",
            compiled.sql
        );
        // No residual smelt. namespace in non-comment SQL lines.
        let non_comment_sql: String = compiled
            .sql
            .lines()
            .filter(|l| !l.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !non_comment_sql.contains("smelt."),
            "Compiled SQL must not contain 'smelt.' prefix in non-comment lines:\n{}",
            compiled.sql
        );

        // Execute the compiled SQL in a real DuckDB database.
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("DuckDB backend");

        // Execute as a SELECT (region_report has default_materialization: view,
        // but for the test we just run it as a plain SELECT to check the data).
        let batches = backend
            .execute_sql(&compiled.sql)
            .await
            .expect("DuckDB must execute the compiled ephemeral-seed query without error");

        // regions.csv has 3 rows: (1, North), (2, South), (3, East).
        let total_rows: usize = batches
            .iter()
            .map(|b| b.num_rows())
            .collect::<Vec<_>>()
            .iter()
            .sum();
        assert_eq!(
            total_rows, 3,
            "region_report must return 3 rows (matching regions.csv), got {}",
            total_rows
        );

        // Key assertion: NO lookup_regions table was created in the database.
        // The seed is ephemeral — its rows were inlined as a VALUES CTE, not
        // loaded into a persistent table.
        let table_check = backend
            .execute_sql("SELECT * FROM main.lookup_regions LIMIT 1")
            .await;
        assert!(
            table_check.is_err(),
            "lookup_regions table must NOT exist — ephemeral seed must not be materialized"
        );
    }

    /// Verify the compiled SQL text: the VALUES CTE alias contains the column
    /// names declared in regions.yml and the literal CSV values.
    #[tokio::test]
    async fn ephemeral_seed_cte_contains_regions_data() {
        let project_dir = project_dir();
        let config: Config =
            serde_yaml::from_str(&std::fs::read_to_string(project_dir.join("smelt.yml")).unwrap())
                .unwrap();

        let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
        let models = discovery.discover_models().unwrap();
        let region_report = models.iter().find(|m| m.name == "region_report").unwrap();

        let resolver = build_seed_resolver(&project_dir, &config);
        let default_target = config.targets.values().next().unwrap();
        let compiler = SqlCompiler::new(config.clone(), default_target);
        let compiled = compiler
            .compile_with_ephemerals(region_report, "main", &resolver)
            .expect("compile_with_ephemerals must succeed");

        // The CTE alias must use the canonical name __smelt_lookup_regions.
        assert!(
            compiled.sql.contains("__smelt_lookup_regions"),
            "Compiled SQL must contain __smelt_lookup_regions CTE alias:\n{}",
            compiled.sql
        );

        // The region names from regions.csv must appear as literals.
        assert!(
            compiled.sql.contains("North"),
            "Compiled SQL must contain 'North' from regions.csv:\n{}",
            compiled.sql
        );
        assert!(
            compiled.sql.contains("South"),
            "Compiled SQL must contain 'South' from regions.csv:\n{}",
            compiled.sql
        );
        assert!(
            compiled.sql.contains("East"),
            "Compiled SQL must contain 'East' from regions.csv:\n{}",
            compiled.sql
        );
    }
}
