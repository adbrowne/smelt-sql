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

        let project_root = temp_dir.path().to_path_buf();
        let mut db = Database::default();
        db.set_all_files(Arc::new(Vec::new()));
        db.set_project_sources_yaml(project_root.clone(), Arc::new(String::new()));
        db.set_all_project_roots(Arc::new(vec![project_root]));

        Self {
            temp_dir,
            db,
            models_dir,
            model_files: Vec::new(),
        }
    }

    fn project_root(&self) -> PathBuf {
        self.temp_dir.path().to_path_buf()
    }

    /// Add a model file to the workspace
    fn add_model(&mut self, name: &str, sql: &str) {
        let path = self.models_dir.join(format!("{}.sql", name));

        // Write file to disk (for realistic testing)
        std::fs::write(&path, sql).expect("Failed to write model file");

        // Update database
        self.db
            .set_file_text(path.clone(), Arc::new(sql.to_string()));
        self.db
            .set_file_project_root(path.clone(), self.project_root());
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
        self.db
            .set_project_sources_yaml(self.project_root(), Arc::new(content.to_string()));
    }

    /// Get the path for a model
    fn model_path(&self, name: &str) -> PathBuf {
        self.models_dir.join(format!("{}.sql", name))
    }

    /// Get code actions at a position in a model (stub — returns empty until handler is implemented)
    #[allow(dead_code)]
    fn code_actions_at(&self, _model: &str, _line: u32, _col: u32) -> Vec<String> {
        // Stub: will be wired to code action handler in Phase 3
        vec![]
    }

    /// Find all references to a symbol at a position (stub — returns empty until handler is implemented)
    #[allow(dead_code)]
    fn references_for(&self, _model: &str, _line: u32, _col: u32) -> Vec<(PathBuf, (u32, u32))> {
        // Stub: will be wired to reference queries in Phase 2
        vec![]
    }

    /// Rename a symbol at a position (stub — returns empty until handler is implemented)
    #[allow(dead_code)]
    fn rename(
        &self,
        _model: &str,
        _line: u32,
        _col: u32,
        _new_name: &str,
    ) -> Vec<(PathBuf, String)> {
        // Stub: will be wired to rename handler in Phase 5
        vec![]
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

    #[test]
    fn test_diagnostic_has_code_undefined_ref() {
        let mut ws = TestWorkspace::new();
        ws.add_model("broken", "SELECT * FROM smelt.ref('nonexistent')");

        let diags = ws.db.file_diagnostics(ws.model_path("broken"));

        assert_eq!(diags.len(), 1);
        assert_eq!(
            diags[0].code,
            Some(smelt_db::DiagnosticCode::UndefinedModelRef),
            "Undefined ref diagnostic should have UndefinedModelRef code"
        );
    }

    #[test]
    fn test_diagnostic_has_code_type_mismatch() {
        use smelt_db::DiagnosticCode;

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
          - name: name
            type: VARCHAR
"#,
        );
        ws.add_model(
            "stg_users",
            "SELECT id, name FROM smelt.source('raw.users')",
        );
        // SUM(name) expects numeric but name is VARCHAR — triggers cross-model type mismatch
        ws.add_model(
            "bad_sum",
            "SELECT SUM(name) as total FROM smelt.ref('stg_users')",
        );

        // type_diagnostics checks cross-model type mismatches
        let diags = ws.db.type_diagnostics(ws.model_path("bad_sum"));

        let type_mismatch = diags
            .iter()
            .find(|d| d.code == Some(DiagnosticCode::TypeMismatch));
        assert!(
            type_mismatch.is_some(),
            "Expected a TypeMismatch diagnostic, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_diagnostic_has_data_undefined_ref() {
        use smelt_db::DiagnosticData;

        let mut ws = TestWorkspace::new();
        ws.add_model("broken", "SELECT * FROM smelt.ref('my_model')");

        let diags = ws.db.file_diagnostics(ws.model_path("broken"));

        assert_eq!(diags.len(), 1);
        assert_eq!(
            diags[0].data,
            Some(DiagnosticData::UndefinedRef {
                model_name: "my_model".to_string(),
            }),
            "Undefined ref diagnostic should have UndefinedRef data with model name"
        );
    }

    #[test]
    fn test_diagnostic_has_data_undeclared_column() {
        use smelt_db::DiagnosticData;

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
"#,
        );
        ws.add_model(
            "model",
            "SELECT nonexistent_col FROM smelt.source('raw.users')",
        );

        // Undeclared column checks are in type_diagnostics
        let diags = ws.db.type_diagnostics(ws.model_path("model"));

        let undeclared = diags
            .iter()
            .find(|d| matches!(&d.data, Some(DiagnosticData::UndeclaredColumn { .. })));
        assert!(
            undeclared.is_some(),
            "Expected an UndeclaredColumn diagnostic data, got: {:?}",
            diags
        );
        if let Some(DiagnosticData::UndeclaredColumn { column_name, .. }) =
            &undeclared.unwrap().data
        {
            assert_eq!(column_name, "nonexistent_col");
        }
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

        let resolved =
            ws.db
                .resolve_source(ws.project_root(), "raw".to_string(), "users".to_string());

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

        let config = ws.db.sources_config(ws.project_root());

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

    #[test]
    fn test_malformed_source_without_dot_produces_diagnostic() {
        let mut ws = TestWorkspace::new();
        // Source call without dot separator (e.g., 'foo' instead of 'raw.users')
        ws.add_model("model", "SELECT * FROM smelt.source('foo')");

        let diags = ws.db.file_diagnostics(ws.model_path("model"));

        // Should produce an error for malformed/undefined source
        assert!(
            !diags.is_empty(),
            "Expected diagnostic for malformed source 'foo' without dot separator"
        );
    }
}

// =============================================================================
// Completion Tests (Alias Autocomplete)
// =============================================================================

mod completion {
    use super::*;

    #[test]
    fn test_type_context_registers_explicit_source_alias() {
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
            type: VARCHAR
"#,
        );
        ws.add_model("model", "SELECT t.id FROM smelt.source('raw.users') AS t");

        // The type context should register the alias 't' -> 'raw.users'
        let ctx = ws.db.type_context(ws.model_path("model"));

        // Verify the alias is registered by looking up a column through it
        let col = ctx.lookup_column(Some("t"), "id");
        assert!(col.is_some(), "Should find column 'id' via alias 't'");
    }

    #[test]
    fn test_type_context_registers_implicit_source_alias() {
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
"#,
        );
        ws.add_model("model", "SELECT t.id FROM smelt.source('raw.users') t");

        let ctx = ws.db.type_context(ws.model_path("model"));

        // Should find column via implicit alias
        let col = ctx.lookup_column(Some("t"), "id");
        assert!(
            col.is_some(),
            "Should find column 'id' via implicit alias 't'"
        );
    }

    #[test]
    fn test_type_context_registers_table_name_as_fallback_alias() {
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
"#,
        );
        // Using table name 'users' as qualifier (implicit alias by table name)
        ws.add_model("model", "SELECT users.id FROM smelt.source('raw.users')");

        let ctx = ws.db.type_context(ws.model_path("model"));

        // Should find column via table name
        let col = ctx.lookup_column(Some("users"), "id");
        assert!(
            col.is_some(),
            "Should find column 'id' via table name 'users'"
        );
    }

    #[test]
    fn test_source_columns_available_for_completion() {
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
            type: VARCHAR
          - name: created_at
            type: TIMESTAMP
"#,
        );
        ws.add_model(
            "model",
            "SELECT t.id, t.email FROM smelt.source('raw.users') AS t",
        );

        // Get sources config and verify columns are available
        let config = ws.db.sources_config(ws.project_root());
        let raw = config.sources.iter().find(|s| s.name == "raw").unwrap();
        let users = raw.tables.iter().find(|t| t.name == "users").unwrap();

        assert_eq!(users.columns.len(), 3);
        let col_names: Vec<&str> = users.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(col_names.contains(&"id"));
        assert!(col_names.contains(&"email"));
        assert!(col_names.contains(&"created_at"));
    }

    #[test]
    fn test_model_columns_available_for_ref_alias() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT id, name, email FROM raw_users");
        ws.add_model(
            "downstream",
            "SELECT u.id, u.name FROM smelt.ref('users') AS u",
        );

        // The upstream model schema should have the columns
        let schema = ws.db.model_schema(ws.model_path("users"));
        assert_eq!(schema.columns.len(), 3);

        let col_names: Vec<&str> = schema.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(col_names.contains(&"id"));
        assert!(col_names.contains(&"name"));
        assert!(col_names.contains(&"email"));
    }

    #[test]
    fn test_join_aliases_both_registered() {
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
      orders:
        columns:
          - name: order_id
            type: INTEGER
          - name: user_id
            type: INTEGER
"#,
        );
        ws.add_model(
            "model",
            "SELECT u.id, o.order_id FROM smelt.source('raw.users') u JOIN smelt.source('raw.orders') o ON u.id = o.user_id",
        );

        let ctx = ws.db.type_context(ws.model_path("model"));

        // Both aliases should be registered
        let user_col = ctx.lookup_column(Some("u"), "id");
        assert!(user_col.is_some(), "Should find 'id' via alias 'u'");

        let order_col = ctx.lookup_column(Some("o"), "order_id");
        assert!(order_col.is_some(), "Should find 'order_id' via alias 'o'");
    }

    #[test]
    fn test_cte_names_available_for_from_completion() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: id
            type: INTEGER
          - name: amount
            type: DECIMAL(10,2)
          - name: created_at
            type: TIMESTAMP
"#,
        );
        ws.add_model(
            "model",
            r#"WITH daily_totals AS (
    SELECT DATE(created_at) as day, SUM(amount) as total
    FROM smelt.source('raw.orders')
    GROUP BY DATE(created_at)
)
SELECT day, total FROM daily_totals WHERE total > 1000"#,
        );

        let ctx = ws.db.type_context(ws.model_path("model"));

        // CTE name should be registered
        assert!(ctx.is_cte("daily_totals"));

        // CTE columns should be available
        let columns = ctx.cte_columns("daily_totals");
        assert!(!columns.is_empty(), "CTE should have inferred columns");

        let col_names: Vec<&str> = columns.iter().map(|(name, _)| *name).collect();
        assert!(col_names.contains(&"day"), "Should have 'day' column");
        assert!(col_names.contains(&"total"), "Should have 'total' column");
    }

    #[test]
    fn test_multiple_ctes_available_for_completion() {
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
          - name: name
            type: VARCHAR
      orders:
        columns:
          - name: user_id
            type: INTEGER
          - name: amount
            type: DECIMAL(10,2)
"#,
        );
        ws.add_model(
            "model",
            r#"WITH
active_users AS (
    SELECT id, name FROM smelt.source('raw.users')
),
user_orders AS (
    SELECT user_id, SUM(amount) as total FROM smelt.source('raw.orders') GROUP BY user_id
)
SELECT u.id, u.name, o.total
FROM active_users u
JOIN user_orders o ON u.id = o.user_id"#,
        );

        let ctx = ws.db.type_context(ws.model_path("model"));

        // Both CTEs should be registered
        assert!(ctx.is_cte("active_users"));
        assert!(ctx.is_cte("user_orders"));

        // active_users columns
        let au_columns = ctx.cte_columns("active_users");
        let au_names: Vec<&str> = au_columns.iter().map(|(name, _)| *name).collect();
        assert!(au_names.contains(&"id"));
        assert!(au_names.contains(&"name"));

        // user_orders columns
        let uo_columns = ctx.cte_columns("user_orders");
        let uo_names: Vec<&str> = uo_columns.iter().map(|(name, _)| *name).collect();
        assert!(uo_names.contains(&"user_id"));
        assert!(uo_names.contains(&"total"));
    }

    #[test]
    fn test_python_model_ref_resolution() {
        // Simulate the LSP registering a Python model's generated SQL.
        // The virtual path must use `<name>.sql` so parse_model derives the correct name.
        let mut ws = TestWorkspace::new();

        // Register the Python model's SQL under the correct virtual path
        let virtual_path = ws.models_dir.join("combined_events.sql");
        let sql = "SELECT event_type, COUNT(*) as cnt FROM raw_events GROUP BY event_type";
        std::fs::write(&virtual_path, sql).expect("write virtual sql");
        ws.db
            .set_file_text(virtual_path.clone(), Arc::new(sql.to_string()));
        ws.db
            .set_file_project_root(virtual_path.clone(), ws.project_root());
        ws.model_files.push(virtual_path);
        ws.db.set_all_files(Arc::new(ws.model_files.clone()));

        // Add a SQL model that references the Python model
        ws.add_model(
            "event_summary",
            "SELECT * FROM smelt.ref('combined_events')",
        );

        let diags = ws.db.file_diagnostics(ws.model_path("event_summary"));
        assert!(
            diags.is_empty(),
            "Expected no diagnostics when Python model is registered with <name>.sql path, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_py_gen_prefix_breaks_ref_resolution() {
        // Regression guard: the old __py_gen__ naming scheme causes ref resolution failure
        let mut ws = TestWorkspace::new();

        // Register the Python model's SQL under the OLD broken virtual path
        let virtual_path = ws.models_dir.join("__py_gen__combined_events.sql");
        let sql = "SELECT event_type, COUNT(*) as cnt FROM raw_events GROUP BY event_type";
        std::fs::write(&virtual_path, sql).expect("write virtual sql");
        ws.db
            .set_file_text(virtual_path.clone(), Arc::new(sql.to_string()));
        ws.db
            .set_file_project_root(virtual_path.clone(), ws.project_root());
        ws.model_files.push(virtual_path);
        ws.db.set_all_files(Arc::new(ws.model_files.clone()));

        // A SQL model referencing 'combined_events' should NOT resolve
        ws.add_model(
            "event_summary",
            "SELECT * FROM smelt.ref('combined_events')",
        );

        let diags = ws.db.file_diagnostics(ws.model_path("event_summary"));
        assert!(
            !diags.is_empty(),
            "Expected undefined ref diagnostic with __py_gen__ prefix path"
        );
        assert!(diags[0].message.contains("Undefined model"));
    }

    #[test]
    fn test_recursive_cte_available_for_completion() {
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "model",
            r#"WITH RECURSIVE counter(n) AS (
    SELECT 1
    UNION ALL
    SELECT n + 1 FROM counter WHERE n < 10
)
SELECT n FROM counter"#,
        );

        let ctx = ws.db.type_context(ws.model_path("model"));

        // Recursive CTE should be registered
        assert!(ctx.is_cte("counter"));

        // Should have explicit column from column list
        let columns = ctx.cte_columns("counter");
        let col_names: Vec<&str> = columns.iter().map(|(name, _)| *name).collect();
        assert!(
            col_names.contains(&"n"),
            "Should have 'n' column from explicit list"
        );
    }
}

// =============================================================================
// Goto-Definition Extended Tests
// =============================================================================

mod goto_definition_extended {
    use super::*;

    #[test]
    fn test_source_resolves_to_table_def() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"version: 1
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
          - name: name
            type: VARCHAR
"#,
        );
        ws.add_model("model", "SELECT id FROM smelt.source('raw.users')");

        let resolved =
            ws.db
                .resolve_source(ws.project_root(), "raw".to_string(), "users".to_string());
        assert!(resolved.is_some());
        let table_def = resolved.unwrap();
        assert_eq!(table_def.name, "users");
        assert_eq!(table_def.columns.len(), 2);
    }

    #[test]
    fn test_model_schema_columns_have_ranges() {
        let mut ws = TestWorkspace::new();
        ws.add_model("upstream", "SELECT 1 AS user_id, 'hello' AS user_name");

        let schema = ws.db.model_schema(ws.model_path("upstream"));
        assert_eq!(schema.columns.len(), 2);

        // Each column should have a non-zero range
        for col in &schema.columns {
            let start: usize = col.range.start().into();
            let end: usize = col.range.end().into();
            assert!(
                end > start,
                "Column '{}' should have a valid range",
                col.name
            );
        }

        assert_eq!(schema.columns[0].name, "user_id");
        assert_eq!(schema.columns[1].name, "user_name");
    }

    #[test]
    fn test_column_traced_through_single_ref() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 AS user_id, 'alice' AS user_name");
        ws.add_model("orders", "SELECT user_id FROM smelt.ref('users')");

        // The downstream model's schema should have user_id with FromModel source
        let schema = ws.db.model_schema(ws.model_path("orders"));
        assert_eq!(schema.columns.len(), 1);
        assert_eq!(schema.columns[0].name, "user_id");

        // The upstream model should have user_id with a valid range
        let upstream_schema = ws.db.model_schema(ws.model_path("users"));
        let user_id_col = upstream_schema.find_column("user_id");
        assert!(user_id_col.is_some(), "Upstream should have user_id column");
    }

    #[test]
    fn test_wildcard_model_has_row_extensions() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 AS user_id");
        ws.add_model("passthrough", "SELECT * FROM smelt.ref('users')");

        let schema = ws.db.model_schema(ws.model_path("passthrough"));
        // SELECT * should create row extensions, not explicit columns
        assert!(
            !schema.row_extensions.is_empty(),
            "Should have row extensions for SELECT *"
        );
        assert_eq!(schema.row_extensions[0].ref_name, "users");
    }

    #[test]
    fn test_resolved_schema_expands_wildcards() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 AS user_id, 'alice' AS user_name");
        ws.add_model("passthrough", "SELECT * FROM smelt.ref('users')");

        let resolved = ws.db.resolved_model_schema(ws.model_path("passthrough"));
        assert!(resolved.is_fully_resolved);
        assert_eq!(resolved.columns.len(), 2);

        let col_names: Vec<&str> = resolved.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(col_names.contains(&"user_id"));
        assert!(col_names.contains(&"user_name"));
    }

    #[test]
    fn test_column_through_wildcard_chain() {
        let mut ws = TestWorkspace::new();
        ws.add_model("base", "SELECT 1 AS col_a, 2 AS col_b");
        ws.add_model("middle", "SELECT * FROM smelt.ref('base')");
        ws.add_model("top", "SELECT col_a FROM smelt.ref('middle')");

        // The 'base' model should have col_a as an explicit column
        let base_schema = ws.db.model_schema(ws.model_path("base"));
        assert!(base_schema.find_column("col_a").is_some());

        // 'middle' has wildcard, so resolved schema should include col_a
        let middle_resolved = ws.db.resolved_model_schema(ws.model_path("middle"));
        let col_names: Vec<&str> = middle_resolved
            .columns
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert!(col_names.contains(&"col_a"));

        // 'top' explicitly selects col_a
        let top_schema = ws.db.model_schema(ws.model_path("top"));
        assert_eq!(top_schema.columns.len(), 1);
        assert_eq!(top_schema.columns[0].name, "col_a");
    }

    #[test]
    fn test_cte_columns_available_in_context() {
        let mut ws = TestWorkspace::new();
        ws.add_model(
            "model",
            r#"WITH totals AS (
    SELECT 1 AS total_count, 2 AS total_amount
)
SELECT total_count FROM totals"#,
        );

        let ctx = ws.db.type_context(ws.model_path("model"));
        assert!(ctx.is_cte("totals"));

        let columns = ctx.cte_columns("totals");
        let col_names: Vec<&str> = columns.iter().map(|(name, _)| *name).collect();
        assert!(col_names.contains(&"total_count"));
        assert!(col_names.contains(&"total_amount"));
    }

    #[test]
    fn test_source_column_available_in_context() {
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"version: 1
sources:
  raw:
    tables:
      events:
        columns:
          - name: event_id
            type: INTEGER
          - name: user_id
            type: INTEGER
"#,
        );
        ws.add_model("model", "SELECT event_id FROM smelt.source('raw.events')");

        let ctx = ws.db.type_context(ws.model_path("model"));
        // Should be able to look up source columns
        let result = ctx.lookup_column(Some("events"), "event_id");
        assert!(result.is_some(), "Should find event_id in source context");
    }

    #[test]
    fn test_model_sources_extracts_source_call_info() {
        let mut ws = TestWorkspace::new();
        ws.add_model("model", "SELECT event_id FROM smelt.source('raw.events') e");

        let sources = ws.db.model_sources(ws.model_path("model"));
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source_name, "raw");
        assert_eq!(sources[0].table_name, "events");
        assert_eq!(sources[0].qualified_name, "raw.events");
    }

    #[test]
    fn test_cte_with_wildcard_resolves_upstream_columns() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 AS user_id, 'alice' AS user_name");
        ws.add_model(
            "model",
            r#"WITH user_cte AS (
    SELECT * FROM smelt.ref('users')
)
SELECT user_id FROM user_cte"#,
        );

        // The CTE should expose upstream columns through wildcard
        let ctx = ws.db.type_context(ws.model_path("model"));
        assert!(ctx.is_cte("user_cte"), "user_cte should be recognized");

        // The final model should resolve user_id
        let result = ctx.lookup_column(None, "user_id");
        assert!(result.is_some(), "Should find user_id through CTE wildcard");
    }

    #[test]
    fn test_cte_with_explicit_and_wildcard_columns() {
        let mut ws = TestWorkspace::new();
        ws.add_model("base", "SELECT 1 AS col_a, 2 AS col_b");
        ws.add_model(
            "model",
            r#"WITH enriched AS (
    SELECT *, 3 AS col_c FROM smelt.ref('base')
)
SELECT col_a, col_c FROM enriched"#,
        );

        let ctx = ws.db.type_context(ws.model_path("model"));
        assert!(ctx.is_cte("enriched"));

        // col_c is explicit in the CTE
        let col_c = ctx.lookup_column(Some("enriched"), "col_c");
        assert!(col_c.is_some(), "Should find explicit col_c in CTE");

        // col_a comes through the wildcard
        let col_a = ctx.lookup_column(Some("enriched"), "col_a");
        assert!(
            col_a.is_some(),
            "Should find col_a through CTE wildcard from upstream"
        );
    }

    #[test]
    fn test_table_alias_in_type_context() {
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 AS user_id");
        ws.add_model("model", "SELECT u.user_id FROM smelt.ref('users') AS u");

        let ctx = ws.db.type_context(ws.model_path("model"));
        // Alias 'u' should resolve to 'users'
        let resolved = ctx.resolve_alias("u");
        assert_eq!(resolved, Some("users".to_string()));

        // Should find user_id through the alias
        let result = ctx.lookup_column(Some("u"), "user_id");
        assert!(result.is_some(), "Should find user_id through alias 'u'");
    }
}

// =============================================================================
// Column Goto-Definition Tests
// =============================================================================
//
// These tests verify the expression-finding logic used by the goto-definition
// handler. The handler walks AST descendants to find the tightest Expr at the
// cursor, then calls as_column_ref() to extract the column reference.

mod column_goto_definition {
    use super::*;
    use smelt_parser::ast::{Expr, File as AstFile};

    /// Helper: find the tightest Expr at a byte offset, using the same logic
    /// as the goto-definition handler in main.rs.
    fn find_expr_at_offset(file: &AstFile, cursor_offset: usize) -> Option<Expr> {
        let mut best_expr: Option<Expr> = None;
        let mut best_len = usize::MAX;

        for node in file.syntax().descendants() {
            if let Some(expr) = Expr::cast(node) {
                let range = expr.text_range();
                let start: usize = range.start().into();
                let end: usize = range.end().into();
                let len = end - start;

                if cursor_offset >= start && cursor_offset <= end && len <= best_len {
                    best_len = len;
                    best_expr = Some(expr);
                }
            }
        }

        best_expr
    }

    #[test]
    fn test_bare_column_in_select_resolves_column_ref() {
        // This is the core bug: bare `event_timestamp` in SELECT has the same
        // text range for SELECT_ITEM and inner EXPRESSION nodes. The fix
        // (len <= best_len) ensures the deeper EXPRESSION is selected.
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"version: 1
sources:
  raw:
    tables:
      events:
        columns:
          - name: event_timestamp
            type: TIMESTAMP
          - name: user_id
            type: INTEGER
"#,
        );
        ws.add_model(
            "model",
            "SELECT\n    event_timestamp\nFROM smelt.source('raw.events')",
        );

        let parse = ws.db.parse_file(ws.model_path("model"));
        let file = AstFile::cast(parse.syntax()).unwrap();

        // Cursor on "event_timestamp" (byte offset within the identifier)
        let text = "SELECT\n    event_timestamp\nFROM smelt.source('raw.events')";
        let col_start = text.find("event_timestamp").unwrap();
        let cursor = col_start + 5; // middle of the identifier

        let expr = find_expr_at_offset(&file, cursor);
        assert!(expr.is_some(), "Should find an expression at cursor");

        let col_ref = expr.unwrap().as_column_ref();
        assert!(
            col_ref.is_some(),
            "Expression should resolve to a ColumnRef"
        );

        let col_ref = col_ref.unwrap();
        assert_eq!(col_ref.name(), "event_timestamp");
        assert!(col_ref.qualifier().is_none());
    }

    #[test]
    fn test_qualified_column_in_select_resolves_column_ref() {
        // Open question 1: qualified columns like e.event_timestamp
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"version: 1
sources:
  raw:
    tables:
      events:
        columns:
          - name: event_timestamp
            type: TIMESTAMP
"#,
        );
        ws.add_model(
            "model",
            "SELECT\n    e.event_timestamp\nFROM smelt.source('raw.events') e",
        );

        let parse = ws.db.parse_file(ws.model_path("model"));
        let file = AstFile::cast(parse.syntax()).unwrap();

        let text = "SELECT\n    e.event_timestamp\nFROM smelt.source('raw.events') e";
        // Cursor on the column name part (after the dot)
        let col_start = text.find("event_timestamp").unwrap();
        let cursor = col_start + 3;

        let expr = find_expr_at_offset(&file, cursor);
        assert!(expr.is_some(), "Should find an expression at cursor");

        let col_ref = expr.unwrap().as_column_ref();
        assert!(
            col_ref.is_some(),
            "Qualified expression should resolve to a ColumnRef"
        );

        let col_ref = col_ref.unwrap();
        assert_eq!(col_ref.name(), "event_timestamp");
        assert_eq!(col_ref.qualifier(), Some("e"));
    }

    #[test]
    fn test_column_from_ref_model_resolves() {
        // Open question 2: columns from smelt.ref() models
        let mut ws = TestWorkspace::new();
        ws.add_model("users", "SELECT 1 AS user_id, 'alice' AS user_name");
        ws.add_model("model", "SELECT\n    user_id\nFROM smelt.ref('users')");

        let parse = ws.db.parse_file(ws.model_path("model"));
        let file = AstFile::cast(parse.syntax()).unwrap();

        let text = "SELECT\n    user_id\nFROM smelt.ref('users')";
        let col_start = text.find("user_id").unwrap();
        let cursor = col_start + 3;

        let expr = find_expr_at_offset(&file, cursor);
        assert!(expr.is_some(), "Should find an expression at cursor");

        let col_ref = expr.unwrap().as_column_ref();
        assert!(
            col_ref.is_some(),
            "Column from ref model should resolve to ColumnRef"
        );

        let col_ref = col_ref.unwrap();
        assert_eq!(col_ref.name(), "user_id");
        assert!(col_ref.qualifier().is_none());

        // Also verify the column can be found in the upstream model schema
        let upstream_schema = ws.db.model_schema(ws.model_path("users"));
        assert!(
            upstream_schema.find_column("user_id").is_some(),
            "Upstream model should have user_id column"
        );
    }

    #[test]
    fn test_column_in_where_clause_resolves() {
        // Open question 3: columns in WHERE should work because the parent
        // node (WHERE_CLAUSE) has a larger range than the EXPRESSION
        let mut ws = TestWorkspace::new();
        ws.set_sources_yml(
            r#"version: 1
sources:
  raw:
    tables:
      events:
        columns:
          - name: event_id
            type: INTEGER
          - name: is_active
            type: BOOLEAN
"#,
        );
        ws.add_model(
            "model",
            "SELECT event_id FROM smelt.source('raw.events') WHERE is_active",
        );

        let parse = ws.db.parse_file(ws.model_path("model"));
        let file = AstFile::cast(parse.syntax()).unwrap();

        let text = "SELECT event_id FROM smelt.source('raw.events') WHERE is_active";
        let col_start = text.find("is_active").unwrap();
        let cursor = col_start + 3;

        let expr = find_expr_at_offset(&file, cursor);
        assert!(expr.is_some(), "Should find expression in WHERE clause");

        let col_ref = expr.unwrap().as_column_ref();
        assert!(
            col_ref.is_some(),
            "Column in WHERE clause should resolve to ColumnRef"
        );

        let col_ref = col_ref.unwrap();
        assert_eq!(col_ref.name(), "is_active");
        assert!(col_ref.qualifier().is_none());
    }
}

// =============================================================================
// Symbol At Cursor Tests (Phase 1)
// =============================================================================

mod symbol_at_cursor {
    use smelt_parser::symbol::{position_to_offset, symbol_at_cursor, SymbolAtCursor};

    /// Helper: parse text and call symbol_at_cursor at (line, col)
    fn resolve(sql: &str, line: u32, col: u32) -> Option<SymbolAtCursor> {
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::ast::File::cast(parse.syntax()).unwrap();
        let offset = position_to_offset(sql, line, col);
        symbol_at_cursor(&file, sql, offset)
    }

    #[test]
    fn test_symbol_at_cursor_ref_call() {
        let sql = "SELECT * FROM smelt.ref('users')";
        // Cursor inside the ref call (on 'users')
        let sym = resolve(sql, 0, 25);
        assert_eq!(
            sym,
            Some(SymbolAtCursor::RefCall {
                name: "users".to_string()
            })
        );
    }

    #[test]
    fn test_symbol_at_cursor_source_call() {
        let sql = "SELECT * FROM smelt.source('raw.users')";
        // Cursor inside the source call
        let sym = resolve(sql, 0, 30);
        assert_eq!(
            sym,
            Some(SymbolAtCursor::SourceCall {
                source_name: "raw".to_string(),
                table_name: "users".to_string(),
            })
        );
    }

    #[test]
    fn test_symbol_at_cursor_cte_reference() {
        let sql = "WITH cte1 AS (SELECT 1 as id)\nSELECT * FROM cte1";
        // Cursor on "cte1" in FROM clause (line 1, col 14)
        let sym = resolve(sql, 1, 14);
        assert_eq!(
            sym,
            Some(SymbolAtCursor::CteReference {
                name: "cte1".to_string()
            })
        );
    }

    #[test]
    fn test_symbol_at_cursor_cte_definition() {
        let sql = "WITH cte1 AS (SELECT 1 as id)\nSELECT * FROM cte1";
        // Cursor on "cte1" in WITH clause (line 0, col 5)
        let sym = resolve(sql, 0, 5);
        assert_eq!(
            sym,
            Some(SymbolAtCursor::CteDefinition {
                name: "cte1".to_string()
            })
        );
    }

    #[test]
    fn test_symbol_at_cursor_column_ref() {
        let sql = "SELECT t.user_id FROM orders t";
        // Cursor on "user_id" part of "t.user_id" (col 9)
        let sym = resolve(sql, 0, 9);
        assert_eq!(
            sym,
            Some(SymbolAtCursor::ColumnRef {
                qualifier: Some("t".to_string()),
                name: "user_id".to_string(),
            })
        );
    }
}

// =============================================================================
// AST Range Helper Tests (Phase 1)
// =============================================================================

mod ast_range_helpers {
    #[test]
    fn test_cte_name_range() {
        let sql = "WITH my_cte AS (SELECT 1 as id)\nSELECT * FROM my_cte";
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::ast::File::cast(parse.syntax()).unwrap();
        let select_stmt = file.select_stmt().unwrap();
        let with_clause = select_stmt.with_clause().unwrap();
        let cte = with_clause.ctes().next().unwrap();

        let name_range = cte.name_range();
        assert!(name_range.is_some(), "CTE should have a name range");
        let range = name_range.unwrap();
        let name_text = &sql[usize::from(range.start())..usize::from(range.end())];
        assert_eq!(name_text, "my_cte");
    }

    #[test]
    fn test_ref_content_range() {
        let sql = "SELECT * FROM smelt.ref('my_model')";
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::ast::File::cast(parse.syntax()).unwrap();
        let ref_call = file.refs().next().unwrap();

        let content_range = ref_call.content_range();
        assert!(
            content_range.is_some(),
            "RefCall should have a content range"
        );
        let range = content_range.unwrap();
        let content_text = &sql[usize::from(range.start())..usize::from(range.end())];
        assert_eq!(
            content_text, "my_model",
            "content_range should exclude quotes"
        );
    }

    #[test]
    fn test_source_table_name_range() {
        let sql = "SELECT * FROM smelt.source('raw.users')";
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::ast::File::cast(parse.syntax()).unwrap();
        let source_call = file.sources().next().unwrap();

        let table_range = source_call.table_name_range();
        assert!(
            table_range.is_some(),
            "SourceCall should have a table_name_range"
        );
        let range = table_range.unwrap();
        let table_text = &sql[usize::from(range.start())..usize::from(range.end())];
        assert_eq!(
            table_text, "users",
            "table_name_range should cover just the table name"
        );
    }
}
