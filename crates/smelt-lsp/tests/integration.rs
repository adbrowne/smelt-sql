//! Integration tests for smelt-lsp
//!
//! These tests verify LSP functionality by directly testing the smelt-db
//! Database queries that power the LSP features.

use std::path::PathBuf;
use std::sync::Arc;

use smelt_db::{Database, DiagnosticSeverity, Inputs, Schema, Semantic, Syntax, TypeChecking};
use tempfile::TempDir;

/// Test workspace that simulates a smelt project
struct TestWorkspace {
    #[allow(dead_code)]
    temp_dir: TempDir,
    db: Database,
    models_dir: PathBuf,
    model_files: Vec<PathBuf>,
}

impl TestWorkspace {
    /// Create a new empty test workspace
    fn new() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let models_dir = temp_dir.path().join("models");
        std::fs::create_dir_all(&models_dir).expect("Failed to create models directory");

        let mut db = Database::default();
        db.set_all_files(Arc::new(Vec::new()));
        db.set_sources_yaml(Arc::new(String::new()));

        Self {
            temp_dir,
            db,
            models_dir,
            model_files: Vec::new(),
        }
    }

    /// Add a model file to the workspace
    fn add_model(&mut self, name: &str, sql: &str) {
        let path = self.models_dir.join(format!("{}.sql", name));

        // Write file to disk (for realistic testing)
        std::fs::write(&path, sql).expect("Failed to write model file");

        // Update database
        self.db
            .set_file_text(path.clone(), Arc::new(sql.to_string()));
        self.model_files.push(path);
        self.db.set_all_files(Arc::new(self.model_files.clone()));
    }

    /// Update an existing model file
    fn update_model(&mut self, name: &str, sql: &str) {
        let path = self.models_dir.join(format!("{}.sql", name));

        // Write file to disk
        std::fs::write(&path, sql).expect("Failed to write model file");

        // Update database
        self.db.set_file_text(path, Arc::new(sql.to_string()));
    }

    /// Set sources.yml content
    fn set_sources_yml(&mut self, content: &str) {
        let path = self.temp_dir.path().join("sources.yml");
        std::fs::write(&path, content).expect("Failed to write sources.yml");
        self.db.set_sources_yaml(Arc::new(content.to_string()));
    }

    /// Get the path for a model
    fn model_path(&self, name: &str) -> PathBuf {
        self.models_dir.join(format!("{}.sql", name))
    }
}

// =============================================================================
// Diagnostic Tests
// =============================================================================

mod diagnostics {
    use super::*;

    #[test]
    fn test_parse_error_produces_diagnostic() {
        let mut ws = TestWorkspace::new();
        ws.add_model("broken", "SELEC * FROM table"); // Typo in SELECT

        let diags = ws.db.file_diagnostics(ws.model_path("broken"));

        assert!(!diags.is_empty(), "Expected parse error diagnostic");
        assert_eq!(diags[0].severity, DiagnosticSeverity::Error);
    }

    #[test]
    fn test_undefined_ref_produces_diagnostic() {
        let mut ws = TestWorkspace::new();
        ws.add_model("broken", "SELECT * FROM smelt.ref('nonexistent')");

        let diags = ws.db.file_diagnostics(ws.model_path("broken"));

        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, DiagnosticSeverity::Error);
        assert!(diags[0].message.contains("Undefined model"));
        assert!(diags[0].message.contains("nonexistent"));
    }

    #[test]
    fn test_valid_ref_produces_no_diagnostic() {
        let mut ws = TestWorkspace::new();
        ws.add_model("upstream", "SELECT 1 as id");
        ws.add_model("downstream", "SELECT * FROM smelt.ref('upstream')");

        let diags = ws.db.file_diagnostics(ws.model_path("downstream"));

        assert!(
            diags.is_empty(),
            "Expected no diagnostics for valid ref, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_undefined_source_produces_diagnostic() {
        let mut ws = TestWorkspace::new();
        ws.add_model("broken", "SELECT * FROM smelt.source('raw.nonexistent')");

        let diags = ws.db.file_diagnostics(ws.model_path("broken"));

        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, DiagnosticSeverity::Error);
        assert!(diags[0].message.contains("Undefined source"));
    }

    #[test]
    fn test_valid_source_produces_no_diagnostic() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
"#,
        );
        ws.add_model("model", "SELECT * FROM smelt.source('raw.users')");

        let diags = ws.db.file_diagnostics(ws.model_path("model"));

        assert!(
            diags.is_empty(),
            "Expected no diagnostics for valid source, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_diagnostic_position_accuracy() {
        let mut ws = TestWorkspace::new();
        // Position the ref on line 2 (0-indexed)
        ws.add_model(
            "broken",
            "-- Comment\nSELECT *\nFROM smelt.ref('nonexistent')",
        );

        let diags = ws.db.file_diagnostics(ws.model_path("broken"));

        assert_eq!(diags.len(), 1);
        // The ref is on line 2 (0-indexed)
        assert_eq!(diags[0].range.start.line, 2);
        assert_eq!(diags[0].range.end.line, 2);
    }

    #[test]
    fn test_multiple_errors_in_file() {
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "broken",
            "SELECT * FROM smelt.ref('missing1') CROSS JOIN smelt.ref('missing2')",
        );

        let diags = ws.db.file_diagnostics(ws.model_path("broken"));

        assert_eq!(diags.len(), 2, "Expected 2 undefined ref diagnostics");
        assert!(diags.iter().any(|d| d.message.contains("missing1")));
        assert!(diags.iter().any(|d| d.message.contains("missing2")));
    }
}

// =============================================================================
// Go-to-Definition Tests
// =============================================================================

mod goto_definition {
    use super::*;

    #[test]
    fn test_resolve_ref_to_model_path() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 as id");
        ws.add_model("orders", "SELECT * FROM smelt.ref('users')");

        let resolved = ws.db.resolve_ref("users".to_string());

        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap(), ws.model_path("users"));
    }

    #[test]
    fn test_resolve_nonexistent_ref_returns_none() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 as id");

        let resolved = ws.db.resolve_ref("nonexistent".to_string());

        assert!(resolved.is_none());
    }

    #[test]
    fn test_model_refs_extracts_ref_calls() {
        let mut ws = TestWorkspace::new();
        ws.add_model("upstream1", "SELECT 1 as id");
        ws.add_model("upstream2", "SELECT 2 as id");
        ws.add_model(
            "downstream",
            "SELECT * FROM smelt.ref('upstream1') CROSS JOIN smelt.ref('upstream2')",
        );

        let refs = ws.db.model_refs(ws.model_path("downstream"));

        assert_eq!(refs.len(), 2);
        let ref_names: Vec<&str> = refs.iter().map(|r| r.name.as_str()).collect();
        assert!(ref_names.contains(&"upstream1"));
        assert!(ref_names.contains(&"upstream2"));
    }
}

// =============================================================================
// Hover Tests (Schema Information)
// =============================================================================

mod hover {
    use super::*;

    #[test]
    fn test_model_schema_extracts_columns() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT id, name, email FROM raw.users");

        let schema = ws.db.model_schema(ws.model_path("users"));

        assert_eq!(schema.columns.len(), 3);
        let col_names: Vec<&str> = schema.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(col_names.contains(&"id"));
        assert!(col_names.contains(&"name"));
        assert!(col_names.contains(&"email"));
    }

    #[test]
    fn test_model_schema_with_aliases() {
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "stats",
            "SELECT user_id, COUNT(*) as total_count FROM events GROUP BY user_id",
        );

        let schema = ws.db.model_schema(ws.model_path("stats"));

        assert_eq!(schema.columns.len(), 2);
        assert_eq!(schema.columns[0].name, "user_id");
        assert_eq!(schema.columns[1].name, "total_count");
        assert_eq!(schema.columns[1].alias, Some("total_count".to_string()));
    }

    #[test]
    fn test_typed_schema_infers_literal_types() {
        let mut ws = TestWorkspace::new();
        ws.add_model("literals", "SELECT 42 as num, 'hello' as str FROM dual");

        let schema = ws.db.typed_model_schema(ws.model_path("literals"));

        assert_eq!(schema.columns.len(), 2);

        // First column should be numeric
        assert!(schema.columns[0].data_type.is_some());

        // Second column should be text
        assert!(schema.columns[1].data_type.is_some());
        assert_eq!(
            schema.columns[1].data_type.as_ref().unwrap().data_type,
            smelt_types::DataType::Text
        );
    }

    #[test]
    fn test_typed_schema_infers_aggregate_types() {
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "aggs",
            "SELECT COUNT(*) as cnt, SUM(amount) as total FROM orders",
        );

        let schema = ws.db.typed_model_schema(ws.model_path("aggs"));

        assert_eq!(schema.columns.len(), 2);

        // COUNT should be BigInt
        assert!(schema.columns[0].data_type.is_some());
        assert_eq!(
            schema.columns[0].data_type.as_ref().unwrap().data_type,
            smelt_types::DataType::BigInt
        );
    }

    #[test]
    fn test_available_columns_includes_upstream() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT id, name, email FROM raw.users");
        ws.add_model("orders", "SELECT user_id FROM smelt.ref('users')");

        let available = ws.db.available_columns(ws.model_path("orders"));

        // Should include columns from both the current model and upstream
        let col_names: Vec<&str> = available.iter().map(|c| c.name.as_str()).collect();
        assert!(col_names.contains(&"user_id")); // From current model
        assert!(col_names.contains(&"id")); // From upstream
        assert!(col_names.contains(&"name")); // From upstream
        assert!(col_names.contains(&"email")); // From upstream
    }
}

// =============================================================================
// Incremental Update Tests
// =============================================================================

mod incremental {
    use super::*;

    #[test]
    fn test_fixing_error_clears_diagnostic() {
        let mut ws = TestWorkspace::new();

        // Start with a broken model
        ws.add_model("model", "SELECT * FROM smelt.ref('missing')");
        let diags1 = ws.db.file_diagnostics(ws.model_path("model"));
        assert_eq!(diags1.len(), 1, "Should have undefined ref error");

        // Add the missing model
        ws.add_model("missing", "SELECT 1 as id");

        // Check diagnostics are cleared
        let diags2 = ws.db.file_diagnostics(ws.model_path("model"));
        assert!(
            diags2.is_empty(),
            "Diagnostics should be cleared after adding missing model"
        );
    }

    #[test]
    fn test_updating_model_triggers_reparse() {
        let mut ws = TestWorkspace::new();

        // Create a model with one column
        ws.add_model("model", "SELECT id FROM users");
        let schema1 = ws.db.model_schema(ws.model_path("model"));
        assert_eq!(schema1.columns.len(), 1);

        // Update to have two columns
        ws.update_model("model", "SELECT id, name FROM users");
        let schema2 = ws.db.model_schema(ws.model_path("model"));
        assert_eq!(schema2.columns.len(), 2);
    }

    #[test]
    fn test_adding_model_makes_it_available_for_refs() {
        let mut ws = TestWorkspace::new();

        // Create a downstream model first (with broken ref)
        ws.add_model("downstream", "SELECT * FROM smelt.ref('upstream')");
        assert!(
            ws.db.resolve_ref("upstream".to_string()).is_none(),
            "Upstream should not exist yet"
        );

        // Now add the upstream model
        ws.add_model("upstream", "SELECT 1 as id");

        // Verify ref now resolves
        assert!(
            ws.db.resolve_ref("upstream".to_string()).is_some(),
            "Upstream should now resolve"
        );
    }

    #[test]
    fn test_sources_yml_changes_affect_diagnostics() {
        let mut ws = TestWorkspace::new();

        // Model referencing a source that doesn't exist yet
        ws.add_model("model", "SELECT * FROM smelt.source('raw.users')");
        let diags1 = ws.db.file_diagnostics(ws.model_path("model"));
        assert_eq!(diags1.len(), 1, "Should have undefined source error");

        // Add the source
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
"#,
        );

        // Diagnostics should clear
        let diags2 = ws.db.file_diagnostics(ws.model_path("model"));
        assert!(
            diags2.is_empty(),
            "Diagnostics should clear after adding source"
        );
    }
}

// =============================================================================
// Source Resolution Tests
// =============================================================================

mod sources {
    use super::*;

    #[test]
    fn test_resolve_source_with_columns() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
          - name: email
            type: VARCHAR(255)
"#,
        );

        let resolved = ws.db.resolve_source("raw".to_string(), "users".to_string());

        assert!(resolved.is_some());
        let table = resolved.unwrap();
        assert_eq!(table.name, "users");
        assert_eq!(table.columns.len(), 2);
    }

    #[test]
    fn test_sources_config_parses_nested_format() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    database: analytics
    schema: public
    tables:
      users:
        columns:
          - name: id
      events:
        columns:
          - name: event_id
"#,
        );

        let config = ws.db.sources_config();

        assert_eq!(config.sources.len(), 1);
        let raw = &config.sources[0];
        assert_eq!(raw.name, "raw");
        assert_eq!(raw.database, Some("analytics".to_string()));
        assert_eq!(raw.schema, Some("public".to_string()));
        assert_eq!(raw.tables.len(), 2);
    }

    #[test]
    fn test_model_sources_extracts_source_calls() {
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "model",
            "SELECT * FROM smelt.source('raw.users') CROSS JOIN smelt.source('raw.events')",
        );

        let sources = ws.db.model_sources(ws.model_path("model"));

        assert_eq!(sources.len(), 2);
        let qualified_names: Vec<&str> =
            sources.iter().map(|s| s.qualified_name.as_str()).collect();
        assert!(qualified_names.contains(&"raw.users"));
        assert!(qualified_names.contains(&"raw.events"));
    }
}
