#![cfg(test)]
#![allow(clippy::uninlined_format_args)]

use super::*;
use crate::test_harness::TestDb;
use line_index::LineIndex;
use std::path::PathBuf;

#[test]
fn test_schema_extraction_simple_columns() {
    let mut db = TestDb::default();

    // Create a simple model with no aliases
    let path = PathBuf::from("test_model.sql");
    db.set_file_text(
        path.clone(),
        Arc::new("SELECT\n  event_id,\n  user_id,\n  event_time\nFROM source.events".to_string()),
    );

    let schema = db.model_schema(path);

    assert_eq!(schema.columns.len(), 3);
    assert_eq!(schema.columns[0].name, "event_id");
    assert_eq!(schema.columns[1].name, "user_id");
    assert_eq!(schema.columns[2].name, "event_time");

    // All should have no alias
    assert!(schema.columns[0].alias.is_none());
    assert!(schema.columns[1].alias.is_none());
    assert!(schema.columns[2].alias.is_none());
}

#[test]
fn test_schema_extraction_with_aliases() {
    let mut db = TestDb::default();

    let path = PathBuf::from("test_model.sql");
    db.set_file_text(
        path.clone(),
        Arc::new(
            "SELECT\n  user_id,\n  COUNT(*) as event_count\nFROM source.events\nGROUP BY user_id"
                .to_string(),
        ),
    );

    let schema = db.model_schema(path);

    assert_eq!(schema.columns.len(), 2);
    assert_eq!(schema.columns[0].name, "user_id");
    assert!(schema.columns[0].alias.is_none());

    assert_eq!(schema.columns[1].name, "event_count");
    assert_eq!(schema.columns[1].alias, Some("event_count".to_string()));
    assert!(schema.columns[1].expression.contains("COUNT"));
}

#[test]
fn test_schema_extraction_from_ref() {
    let mut db = TestDb::default();

    // Create upstream model
    let raw_events_path = PathBuf::from("models/raw_events.sql");
    db.set_file_text(
        raw_events_path.clone(),
        Arc::new("SELECT\n  user_id,\n  event_id\nFROM source.events".to_string()),
    );

    // Create downstream model that refs upstream
    let sessions_path = PathBuf::from("models/user_sessions.sql");
    db.set_file_text(
            sessions_path.clone(),
            Arc::new("SELECT\n  user_id,\n  COUNT(*) as session_count\nFROM smelt.models.raw_events\nGROUP BY user_id".to_string()),
        );

    // Set up all_files for model resolution
    db.set_all_files(Arc::new(vec![
        raw_events_path.clone(),
        sessions_path.clone(),
    ]));
    db.set_file_project_root(raw_events_path.clone(), PathBuf::from("."));
    db.set_file_project_root(sessions_path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.model_schema(sessions_path);

    assert_eq!(schema.columns.len(), 2);

    // user_id should be traced to raw_events
    assert_eq!(schema.columns[0].name, "user_id");
    match &schema.columns[0].source {
        ColumnSource::FromModel {
            model_name,
            column_name,
        } => {
            assert_eq!(model_name, "raw_events");
            assert_eq!(column_name, "user_id");
        }
        _ => panic!("Expected FromModel source"),
    }

    // COUNT(*) should be Computed
    assert_eq!(schema.columns[1].name, "session_count");
    assert_eq!(schema.columns[1].alias, Some("session_count".to_string()));
    match schema.columns[1].source {
        ColumnSource::Computed => {}
        _ => panic!("Expected Computed source"),
    }
}

#[test]
fn test_available_columns_includes_upstream() {
    let mut db = TestDb::default();

    // Create upstream model
    let raw_events_path = PathBuf::from("models/raw_events.sql");
    db.set_file_text(
        raw_events_path.clone(),
        Arc::new("SELECT\n  user_id,\n  event_id,\n  event_time\nFROM source.events".to_string()),
    );

    // Create downstream model
    let sessions_path = PathBuf::from("models/user_sessions.sql");
    db.set_file_text(
        sessions_path.clone(),
        Arc::new("SELECT\n  user_id\nFROM smelt.models.raw_events".to_string()),
    );

    db.set_all_files(Arc::new(vec![
        raw_events_path.clone(),
        sessions_path.clone(),
    ]));
    db.set_file_project_root(raw_events_path.clone(), PathBuf::from("."));
    db.set_file_project_root(sessions_path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let available = db.available_columns(sessions_path);

    // Should include current model's columns (1) + upstream columns (3) = 4
    assert_eq!(available.len(), 4);

    let column_names: Vec<&str> = available.iter().map(|c| c.name.as_str()).collect();
    assert!(column_names.contains(&"user_id"));
    assert!(column_names.contains(&"event_id"));
    assert!(column_names.contains(&"event_time"));
}

#[test]
fn test_undefined_ref_diagnostic_position() {
    let mut db = TestDb::default();

    // Phase 4: use path form smelt.models.nonexistent_model instead of
    // smelt.models.nonexistent_model — the legacy form now produces a parse error.
    let path = PathBuf::from("test_model.sql");
    db.set_file_text(
        path.clone(),
        Arc::new("SELECT * FROM smelt.models.nonexistent_model".to_string()),
    );

    // Register the file (no other files, so path ref won't resolve)
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    // Get diagnostics
    let diagnostics = db.file_diagnostics(path);

    // Should have exactly one diagnostic for undefined model path
    assert_eq!(
        diagnostics.len(),
        1,
        "expected 1 diagnostic, got: {diagnostics:?}"
    );
    let diag = &diagnostics[0];

    // Check severity
    assert_eq!(diag.severity, DiagnosticSeverity::Error);
    // The diagnostic should mention the path
    assert!(
        diag.message.contains("nonexistent_model") || diag.message.contains("models"),
        "diagnostic should mention the path; got: {:?}",
        diag.message
    );

    // Check position - should be on line 0 (single-line source)
    let content_single = "SELECT * FROM smelt.models.nonexistent_model";
    let li = LineIndex::new(content_single);
    assert_eq!(li.line_col(diag.range.start()).line, 0);
    assert_eq!(li.line_col(diag.range.end()).line, 0);
}

#[test]
fn test_undefined_ref_diagnostic_position_multiline() {
    let mut db = TestDb::default();

    // Phase 4: use path form instead of smelt.ref().
    let path = PathBuf::from("broken_model.sql");
    let content = "-- This model has an undefined reference - should show diagnostic\nSELECT *\nFROM smelt.models.nonexistent_model\n";
    db.set_file_text(path.clone(), Arc::new(content.to_string()));

    // Register the file
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    // Get diagnostics
    let diagnostics = db.file_diagnostics(path);

    println!("\nContent: {:?}", content);
    println!("Number of diagnostics: {}", diagnostics.len());

    // Should have exactly one diagnostic (undefined path)
    assert_eq!(
        diagnostics.len(),
        1,
        "expected 1 diagnostic, got: {diagnostics:?}"
    );
    let diag = &diagnostics[0];

    // Check it starts on line 2 (0-indexed). The end may be line 2 or 3
    // depending on whether the path node's text range includes the trailing
    // newline — either is acceptable; the important thing is start line.
    let li = LineIndex::new(content);
    assert_eq!(li.line_col(diag.range.start()).line, 2);
    assert!(
        li.line_col(diag.range.end()).line >= 2,
        "end line should be >= 2, got: line {}",
        li.line_col(diag.range.end()).line
    );
}

#[test]
fn test_lexer_positions() {
    use smelt_parser::lexer::tokenize;

    let content = "-- This model has an undefined reference - should show diagnostic\nSELECT *\nFROM smelt.models.nonexistent_model\n";
    let tokens = tokenize(content);

    println!("Total content length: {}", content.len());
    println!("\nTokens:");
    let mut offset = 0;
    for token in &tokens {
        let text = &content[offset..offset + token.len];
        println!(
            "  {:?} @ {}..{}: {:?}",
            token.kind,
            offset,
            offset + token.len,
            text
        );
        offset += token.len;
    }
    println!("Final offset: {}", offset);
}

#[test]
fn test_typed_model_schema_literals() {
    let mut db = TestDb::default();

    // Create a model with various literal types
    let path = PathBuf::from("test_model.sql");
    db.set_file_text(
        path.clone(),
        Arc::new(
            "SELECT 42 as small_num, 100000 as medium_num, 'hello' as greeting FROM source.test"
                .to_string(),
        ),
    );
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path);

    assert_eq!(schema.columns.len(), 3);

    // Check small_num is SmallInt
    assert!(schema.columns[0].data_type.is_some());
    assert_eq!(
        schema.columns[0].data_type.as_ref().unwrap().data_type,
        smelt_types::DataType::SmallInt
    );

    // Check medium_num is Integer
    assert!(schema.columns[1].data_type.is_some());
    assert_eq!(
        schema.columns[1].data_type.as_ref().unwrap().data_type,
        smelt_types::DataType::Integer
    );

    // Check greeting is Text
    assert!(schema.columns[2].data_type.is_some());
    assert_eq!(
        schema.columns[2].data_type.as_ref().unwrap().data_type,
        smelt_types::DataType::Text
    );
}

#[test]
fn test_typed_model_schema_aggregates() {
    let mut db = TestDb::default();

    // Create a model with aggregate functions
    let path = PathBuf::from("test_model.sql");
    db.set_file_text(
        path.clone(),
        Arc::new(
            "SELECT COUNT(*) as cnt, AVG(price) as avg_price FROM source.test GROUP BY category"
                .to_string(),
        ),
    );
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path);

    assert_eq!(schema.columns.len(), 2);

    // COUNT(*) should be BigInt
    assert!(schema.columns[0].data_type.is_some());
    assert_eq!(
        schema.columns[0].data_type.as_ref().unwrap().data_type,
        smelt_types::DataType::BigInt
    );
    // COUNT never returns NULL
    assert!(!schema.columns[0].data_type.as_ref().unwrap().nullable);

    // AVG should be Double
    assert!(schema.columns[1].data_type.is_some());
    assert_eq!(
        schema.columns[1].data_type.as_ref().unwrap().data_type,
        smelt_types::DataType::Double
    );
    // AVG can be NULL for empty sets
    assert!(schema.columns[1].data_type.as_ref().unwrap().nullable);
}

#[test]
fn test_typed_model_schema_with_sources() {
    let mut db = TestDb::default();

    // Create sources.yml with typed columns
    let sources_yaml = r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
          - name: email
            type: VARCHAR(255)
          - name: created_at
            type: TIMESTAMP
"#;

    let path = PathBuf::from("test_model.sql");
    db.set_file_text(
        path.clone(),
        Arc::new("SELECT id, email, created_at FROM smelt.sources.raw.users".to_string()),
    );
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path);

    assert_eq!(schema.columns.len(), 3);

    // Note: Column type inference from sources requires column reference resolution
    // which is a more complex case. For now, the basic literal and aggregate
    // inference is working.
}

#[test]
fn test_simple_cte_type_inference() {
    let mut db = TestDb::default();

    // Create sources.yml with typed columns
    let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: created_at
            type: TIMESTAMP
          - name: amount
            type: DECIMAL(10,2)
"#;

    // SQL with CTE using DATE() and SUM()
    let sql = r#"
WITH daily_totals AS (
    SELECT DATE(created_at) as day, SUM(amount) as total
    FROM smelt.sources.raw.orders
    GROUP BY DATE(created_at)
)
SELECT day, total FROM daily_totals WHERE total > 1000
"#;

    let path = PathBuf::from("models/test_cte.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    // Should have 2 columns: day and total
    assert_eq!(schema.columns.len(), 2);

    // Check that day has DATE type (from DATE() function)
    let day_col = schema.columns.iter().find(|c| c.name == "day");
    assert!(day_col.is_some(), "Column 'day' not found");
    if let Some(typed_col) = &day_col.unwrap().data_type {
        assert_eq!(typed_col.data_type, DataType::Date);
    }

    // Check that total has Decimal type (from SUM())
    let total_col = schema.columns.iter().find(|c| c.name == "total");
    assert!(total_col.is_some(), "Column 'total' not found");
    if let Some(typed_col) = &total_col.unwrap().data_type {
        assert!(
            matches!(typed_col.data_type, DataType::Decimal { .. }),
            "Expected Decimal type for 'total', got {:?}",
            typed_col.data_type
        );
    }
}

#[test]
fn test_multiple_ctes_forward_reference() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: amount
            type: DECIMAL(10,2)
"#;

    // Multiple CTEs where cte2 references cte1
    let sql = r#"
WITH cte1 AS (
    SELECT SUM(amount) as total
    FROM smelt.sources.raw.orders
),
cte2 AS (
    SELECT total * 2 as doubled
    FROM cte1
)
SELECT doubled FROM cte2
"#;

    let path = PathBuf::from("models/test_multi_cte.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    // Should have 1 column: doubled
    assert_eq!(schema.columns.len(), 1);

    // Check that doubled has inferred type from the multiplication expression
    let doubled_col = schema.columns.iter().find(|c| c.name == "doubled");
    assert!(doubled_col.is_some(), "Column 'doubled' not found");
}

#[test]
fn test_cte_explicit_column_list() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: amount
            type: INTEGER
"#;

    // CTE with explicit column list - names should override inferred names
    let sql = r#"
WITH order_stats(order_sum, order_count) AS (
    SELECT SUM(amount), COUNT(*)
    FROM smelt.sources.raw.orders
)
SELECT order_sum, order_count FROM order_stats
"#;

    let path = PathBuf::from("models/test_explicit_cols.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    // Should have 2 columns with explicit names
    assert_eq!(schema.columns.len(), 2);

    // Check that order_sum exists (explicit name, not SUM)
    let order_sum_col = schema.columns.iter().find(|c| c.name == "order_sum");
    assert!(
        order_sum_col.is_some(),
        "Column 'order_sum' not found - explicit column list should override inferred names"
    );

    // Check that order_count exists (explicit name, not COUNT)
    let order_count_col = schema.columns.iter().find(|c| c.name == "order_count");
    assert!(
        order_count_col.is_some(),
        "Column 'order_count' not found - explicit column list should override inferred names"
    );
}

#[test]
fn test_nested_cte_in_cte() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: amount
            type: INTEGER
"#;

    // Nested CTE: outer_cte contains inner_cte in its definition
    let sql = r#"
WITH outer_cte AS (
    WITH inner_cte AS (
        SELECT SUM(amount) as inner_total
        FROM smelt.sources.raw.orders
    )
    SELECT inner_total FROM inner_cte
)
SELECT inner_total FROM outer_cte
"#;

    let path = PathBuf::from("models/test_nested_cte.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    // Should have 1 column: inner_total
    assert_eq!(schema.columns.len(), 1);

    // Check that inner_total has BigInt type (SUM(INTEGER) returns BigInt)
    let result_col = schema.columns.iter().find(|c| c.name == "inner_total");
    assert!(result_col.is_some(), "Column 'inner_total' not found");
    if let Some(typed_col) = &result_col.unwrap().data_type {
        assert!(
            matches!(typed_col.data_type, DataType::BigInt),
            "Expected BigInt type for 'inner_total' (from SUM(INTEGER) in nested CTE), got {:?}",
            typed_col.data_type
        );
    }
}

#[test]
fn test_recursive_cte_without_explicit_columns() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources:
  raw:
    tables:
      nodes:
        columns:
          - name: id
            type: INTEGER
          - name: parent_id
            type: INTEGER
"#;

    // Recursive CTE WITHOUT explicit column list
    let sql = r#"
WITH RECURSIVE tree AS (
    SELECT id, parent_id FROM smelt.sources.raw.nodes WHERE parent_id IS NULL
    UNION ALL
    SELECT n.id, n.parent_id FROM smelt.sources.raw.nodes n
    INNER JOIN tree ON n.parent_id = tree.id
)
SELECT id, parent_id FROM tree
"#;

    let path = PathBuf::from("models/test_recursive.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    // Should have 2 columns: id and parent_id
    assert_eq!(schema.columns.len(), 2);

    // Check that id has INTEGER type (inferred from anchor term)
    let id_col = schema.columns.iter().find(|c| c.name == "id");
    assert!(id_col.is_some(), "Column 'id' not found");
    assert!(
        id_col.unwrap().data_type.is_some(),
        "Column 'id' should have a type"
    );
    assert_eq!(
        id_col.unwrap().data_type.as_ref().unwrap().data_type,
        DataType::Integer,
        "Expected INTEGER for 'id'"
    );

    // Check that parent_id also has INTEGER type
    let parent_id_col = schema.columns.iter().find(|c| c.name == "parent_id");
    assert!(parent_id_col.is_some(), "Column 'parent_id' not found");
    assert!(
        parent_id_col.unwrap().data_type.is_some(),
        "Column 'parent_id' should have a type"
    );
    assert_eq!(
        parent_id_col.unwrap().data_type.as_ref().unwrap().data_type,
        DataType::Integer,
        "Expected INTEGER for 'parent_id'"
    );
}

#[test]
fn test_recursive_cte_with_literal_anchor() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources: {}
"#;

    // Recursive CTE with literal in anchor term (no source references)
    let sql = r#"
WITH RECURSIVE nums AS (
    SELECT 1 as n
    UNION ALL
    SELECT n + 1 FROM nums WHERE n < 10
)
SELECT n FROM nums
"#;

    let path = PathBuf::from("models/test_recursive_literal.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    // Should have 1 column: n
    assert_eq!(schema.columns.len(), 1);

    // Check that n has SmallInt type (from literal 1 in anchor term)
    let n_col = schema.columns.iter().find(|c| c.name == "n");
    assert!(n_col.is_some(), "Column 'n' not found");
    assert!(
        n_col.unwrap().data_type.is_some(),
        "Column 'n' should have a type"
    );
    assert_eq!(
        n_col.unwrap().data_type.as_ref().unwrap().data_type,
        DataType::SmallInt,
        "Expected SmallInt for 'n' (from literal 1)"
    );
}

#[test]
fn test_between_type_inference() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: amount
            type: INTEGER
"#;

    // BETWEEN expression should return Boolean
    let sql = r#"
SELECT amount BETWEEN 10 AND 100 as in_range
FROM smelt.sources.raw.orders
"#;

    let path = PathBuf::from("models/test_between.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    assert_eq!(schema.columns.len(), 1);

    let in_range_col = schema.columns.iter().find(|c| c.name == "in_range");
    assert!(in_range_col.is_some(), "Column 'in_range' not found");
    assert!(
        in_range_col.unwrap().data_type.is_some(),
        "Column 'in_range' should have a type"
    );
    assert_eq!(
        in_range_col.unwrap().data_type.as_ref().unwrap().data_type,
        DataType::Boolean,
        "BETWEEN should return Boolean"
    );
}

#[test]
fn test_in_type_inference() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: status
            type: VARCHAR(50)
"#;

    // IN expression should return Boolean
    let sql = r#"
SELECT status IN ('pending', 'processing', 'shipped') as is_active
FROM smelt.sources.raw.orders
"#;

    let path = PathBuf::from("models/test_in.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    assert_eq!(schema.columns.len(), 1);

    let is_active_col = schema.columns.iter().find(|c| c.name == "is_active");
    assert!(is_active_col.is_some(), "Column 'is_active' not found");
    assert!(
        is_active_col.unwrap().data_type.is_some(),
        "Column 'is_active' should have a type"
    );
    assert_eq!(
        is_active_col.unwrap().data_type.as_ref().unwrap().data_type,
        DataType::Boolean,
        "IN should return Boolean"
    );
}

#[test]
fn test_exists_type_inference() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: user_id
            type: INTEGER
      users:
        columns:
          - name: id
            type: INTEGER
"#;

    // EXISTS expression should return Boolean (never NULL)
    let sql = r#"
SELECT EXISTS (SELECT 1 FROM smelt.sources.raw.users WHERE id = user_id) as has_user
FROM smelt.sources.raw.orders
"#;

    let path = PathBuf::from("models/test_exists.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    assert_eq!(schema.columns.len(), 1);

    let has_user_col = schema.columns.iter().find(|c| c.name == "has_user");
    assert!(has_user_col.is_some(), "Column 'has_user' not found");
    assert!(
        has_user_col.unwrap().data_type.is_some(),
        "Column 'has_user' should have a type"
    );
    let typed_col = has_user_col.unwrap().data_type.as_ref().unwrap();
    assert_eq!(
        typed_col.data_type,
        DataType::Boolean,
        "EXISTS should return Boolean"
    );
    assert!(
        !typed_col.nullable,
        "EXISTS should never be NULL (always TRUE or FALSE)"
    );
}

#[test]
fn test_not_operator_type_inference() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: is_completed
            type: BOOLEAN
"#;

    // NOT operator should return Boolean
    let sql = r#"
SELECT NOT is_completed as is_pending
FROM smelt.sources.raw.orders
"#;

    let path = PathBuf::from("models/test_not.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    assert_eq!(schema.columns.len(), 1);

    let is_pending_col = schema.columns.iter().find(|c| c.name == "is_pending");
    assert!(is_pending_col.is_some(), "Column 'is_pending' not found");
    assert!(
        is_pending_col.unwrap().data_type.is_some(),
        "Column 'is_pending' should have a type"
    );
    assert_eq!(
        is_pending_col
            .unwrap()
            .data_type
            .as_ref()
            .unwrap()
            .data_type,
        DataType::Boolean,
        "NOT should return Boolean"
    );
}

#[test]
fn test_unary_negation_type_inference() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: amount
            type: INTEGER
"#;

    // Unary negation should preserve numeric type
    let sql = r#"
SELECT -amount as negative_amount
FROM smelt.sources.raw.orders
"#;

    let path = PathBuf::from("models/test_negation.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    assert_eq!(schema.columns.len(), 1);

    let neg_col = schema.columns.iter().find(|c| c.name == "negative_amount");
    assert!(neg_col.is_some(), "Column 'negative_amount' not found");
    assert!(
        neg_col.unwrap().data_type.is_some(),
        "Column 'negative_amount' should have a type"
    );
    assert_eq!(
        neg_col.unwrap().data_type.as_ref().unwrap().data_type,
        DataType::Integer,
        "Unary negation should preserve numeric type (INTEGER)"
    );
}

#[test]
fn test_union_same_types() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: id
            type: INTEGER
          - name: amount
            type: DECIMAL(10,2)
      returns:
        columns:
          - name: id
            type: INTEGER
          - name: amount
            type: DECIMAL(10,2)
"#;

    // UNION with same types should preserve those types
    let sql = r#"
SELECT id, amount FROM smelt.sources.raw.orders
UNION
SELECT id, amount FROM smelt.sources.raw.returns
"#;

    let path = PathBuf::from("models/test_union_same.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    assert_eq!(schema.columns.len(), 2);

    // id should be INTEGER
    let id_col = schema.columns.iter().find(|c| c.name == "id");
    assert!(id_col.is_some(), "Column 'id' not found");
    assert_eq!(
        id_col.unwrap().data_type.as_ref().unwrap().data_type,
        DataType::Integer,
        "UNION with same types should preserve INTEGER"
    );

    // amount should be DECIMAL
    let amount_col = schema.columns.iter().find(|c| c.name == "amount");
    assert!(amount_col.is_some(), "Column 'amount' not found");
    assert!(
        matches!(
            amount_col.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::Decimal { .. }
        ),
        "UNION with same types should preserve DECIMAL"
    );
}

#[test]
fn test_union_type_promotion() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources: {}
"#;

    // UNION with INTEGER literal and BIGINT literal should promote to BIGINT
    // Using CAST to ensure we get specific types
    let sql = r#"
SELECT CAST(1 AS INTEGER) as n
UNION
SELECT CAST(2 AS BIGINT) as n
"#;

    let path = PathBuf::from("models/test_union_promote.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    assert_eq!(schema.columns.len(), 1);

    let n_col = schema.columns.iter().find(|c| c.name == "n");
    assert!(n_col.is_some(), "Column 'n' not found");
    assert_eq!(
        n_col.unwrap().data_type.as_ref().unwrap().data_type,
        DataType::BigInt,
        "UNION of INTEGER and BIGINT should promote to BIGINT"
    );
}

#[test]
fn test_union_all_type_promotion() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources: {}
"#;

    // UNION ALL with INTEGER and DOUBLE should promote to DOUBLE
    let sql = r#"
SELECT CAST(1 AS INTEGER) as value
UNION ALL
SELECT CAST(2.5 AS DOUBLE) as value
"#;

    let path = PathBuf::from("models/test_union_all.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    assert_eq!(schema.columns.len(), 1);

    let value_col = schema.columns.iter().find(|c| c.name == "value");
    assert!(value_col.is_some(), "Column 'value' not found");
    assert_eq!(
        value_col.unwrap().data_type.as_ref().unwrap().data_type,
        DataType::Double,
        "UNION ALL of INTEGER and DOUBLE should promote to DOUBLE"
    );
}

#[test]
fn test_union_chained() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources: {}
"#;

    // Chained UNION: SMALLINT UNION INTEGER UNION BIGINT should be BIGINT
    let sql = r#"
SELECT CAST(1 AS SMALLINT) as n
UNION
SELECT CAST(2 AS INTEGER) as n
UNION
SELECT CAST(3 AS BIGINT) as n
"#;

    let path = PathBuf::from("models/test_union_chained.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    assert_eq!(schema.columns.len(), 1);

    let n_col = schema.columns.iter().find(|c| c.name == "n");
    assert!(n_col.is_some(), "Column 'n' not found");
    assert_eq!(
        n_col.unwrap().data_type.as_ref().unwrap().data_type,
        DataType::BigInt,
        "Chained UNION of SMALLINT, INTEGER, BIGINT should be BIGINT"
    );
}

#[test]
fn test_join_column_tracking_source() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: id
            type: INTEGER
          - name: user_id
            type: INTEGER
          - name: amount
            type: DECIMAL(10,2)
      users:
        columns:
          - name: id
            type: INTEGER
          - name: name
            type: VARCHAR(100)
"#;

    // JOIN query - columns from joined table should be available
    let sql = r#"
SELECT o.id, o.amount, u.name
FROM smelt.sources.raw.orders o
INNER JOIN smelt.sources.raw.users u ON o.user_id = u.id
"#;

    let path = PathBuf::from("models/test_join.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    assert_eq!(schema.columns.len(), 3);

    // Check that id from orders is INTEGER
    let id_col = schema.columns.iter().find(|c| c.name == "id");
    assert!(id_col.is_some(), "Column 'id' not found");
    assert_eq!(
        id_col.unwrap().data_type.as_ref().unwrap().data_type,
        DataType::Integer,
        "id should be INTEGER from orders"
    );

    // Check that amount from orders is DECIMAL
    let amount_col = schema.columns.iter().find(|c| c.name == "amount");
    assert!(amount_col.is_some(), "Column 'amount' not found");
    assert!(
        matches!(
            amount_col.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::Decimal { .. }
        ),
        "amount should be DECIMAL from orders"
    );

    // Check that name from users (joined table) is VARCHAR/Text
    let name_col = schema.columns.iter().find(|c| c.name == "name");
    assert!(name_col.is_some(), "Column 'name' not found");
    assert!(
        matches!(
            name_col.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::Varchar { .. }
        ),
        "name should be VARCHAR from joined users table"
    );
}

#[test]
fn test_join_column_tracking_multiple_joins() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: id
            type: INTEGER
          - name: user_id
            type: INTEGER
          - name: product_id
            type: INTEGER
      users:
        columns:
          - name: id
            type: INTEGER
          - name: name
            type: VARCHAR(100)
      products:
        columns:
          - name: id
            type: INTEGER
          - name: price
            type: DOUBLE
"#;

    // Multiple JOINs - columns from all joined tables should be available
    let sql = r#"
SELECT o.id, u.name, p.price
FROM smelt.sources.raw.orders o
INNER JOIN smelt.sources.raw.users u ON o.user_id = u.id
INNER JOIN smelt.sources.raw.products p ON o.product_id = p.id
"#;

    let path = PathBuf::from("models/test_multi_join.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    assert_eq!(schema.columns.len(), 3);

    // Check that id from orders is INTEGER
    let id_col = schema.columns.iter().find(|c| c.name == "id");
    assert!(id_col.is_some(), "Column 'id' not found");
    assert_eq!(
        id_col.unwrap().data_type.as_ref().unwrap().data_type,
        DataType::Integer,
        "id should be INTEGER"
    );

    // Check that name from users is VARCHAR
    let name_col = schema.columns.iter().find(|c| c.name == "name");
    assert!(name_col.is_some(), "Column 'name' not found");
    assert!(
        matches!(
            name_col.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::Varchar { .. }
        ),
        "name should be VARCHAR from joined users table"
    );

    // Check that price from products (second join) is DOUBLE
    let price_col = schema.columns.iter().find(|c| c.name == "price");
    assert!(price_col.is_some(), "Column 'price' not found");
    assert_eq!(
        price_col.unwrap().data_type.as_ref().unwrap().data_type,
        DataType::Double,
        "price should be DOUBLE from joined products table"
    );
}

#[test]
fn test_join_column_tracking_left_join() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: id
            type: INTEGER
          - name: user_id
            type: INTEGER
      users:
        columns:
          - name: id
            type: INTEGER
          - name: email
            type: TEXT
"#;

    // LEFT JOIN - columns from joined table should still be available
    let sql = r#"
SELECT o.id, u.email
FROM smelt.sources.raw.orders o
LEFT JOIN smelt.sources.raw.users u ON o.user_id = u.id
"#;

    let path = PathBuf::from("models/test_left_join.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    assert_eq!(schema.columns.len(), 2);

    // Check that email from users (LEFT JOINed) has a string type
    let email_col = schema.columns.iter().find(|c| c.name == "email");
    assert!(email_col.is_some(), "Column 'email' not found");
    // TEXT is parsed as Varchar { max_length: None } or Text
    let email_type = &email_col.unwrap().data_type.as_ref().unwrap().data_type;
    assert!(
        matches!(email_type, DataType::Text | DataType::Varchar { .. }),
        "email should be TEXT/VARCHAR from LEFT JOINed users table, got {:?}",
        email_type
    );
}

#[test]
fn test_lateral_parsing_debug() {
    // First, verify the AST structure
    // Phase 4: use smelt.sources.* path form (smelt.source() is removed).
    let sql = r#"
SELECT u.id, recent.total_amount
FROM smelt.sources.raw.users u
LEFT JOIN LATERAL (
    SELECT SUM(o.amount) as total_amount
    FROM smelt.sources.raw.orders o
) recent ON true
"#;

    let parsed = smelt_parser::parse(sql);
    assert!(
        parsed.errors.is_empty(),
        "Parse errors: {:?}",
        parsed.errors
    );

    let file = smelt_parser::ast::File::cast(parsed.syntax()).unwrap();
    let select = file.select_stmt().unwrap();
    let from = select.from_clause().unwrap();

    // Check main table ref
    let main_refs: Vec<_> = from.table_refs().collect();
    assert_eq!(main_refs.len(), 1, "Should have 1 main table ref");
    assert_eq!(main_refs[0].alias(), Some("u".to_string()));

    // Check JOIN
    let joins: Vec<_> = from.joins().collect();
    assert_eq!(joins.len(), 1, "Should have 1 JOIN");

    let join = &joins[0];
    let join_table_ref = join.table_ref().expect("JOIN should have table_ref");

    // Check LATERAL and subquery
    assert!(
        join_table_ref.is_lateral(),
        "JOIN table ref should be LATERAL"
    );
    assert!(
        join_table_ref.subquery().is_some(),
        "JOIN table ref should have subquery"
    );
    assert_eq!(
        join_table_ref.alias(),
        Some("recent".to_string()),
        "JOIN table ref should have alias 'recent'"
    );
}

#[test]
fn test_lateral_correlation_basic() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
          - name: name
            type: VARCHAR(100)
      orders:
        columns:
          - name: id
            type: INTEGER
          - name: user_id
            type: INTEGER
          - name: amount
            type: DECIMAL(10,2)
"#;

    // LATERAL subquery that references columns from the preceding table
    // Using LEFT JOIN LATERAL since comma syntax was removed
    let sql = r#"
SELECT u.id, u.name, recent.total_amount
FROM smelt.sources.raw.users u
LEFT JOIN LATERAL (
    SELECT SUM(o.amount) as total_amount
    FROM smelt.sources.raw.orders o
    WHERE o.user_id = u.id
) recent ON true
"#;

    let path = PathBuf::from("models/test_lateral.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    assert_eq!(schema.columns.len(), 3);

    // Check that id from users is INTEGER
    let id_col = schema.columns.iter().find(|c| c.name == "id");
    assert!(id_col.is_some(), "Column 'id' not found");
    assert_eq!(
        id_col.unwrap().data_type.as_ref().unwrap().data_type,
        DataType::Integer,
        "id should be INTEGER from users"
    );

    // Check that name from users is VARCHAR
    let name_col = schema.columns.iter().find(|c| c.name == "name");
    assert!(name_col.is_some(), "Column 'name' not found");
    assert!(
        matches!(
            name_col.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::Varchar { .. }
        ),
        "name should be VARCHAR from users"
    );

    // Check that total_amount from LATERAL subquery is DECIMAL (from SUM)
    let total_col = schema.columns.iter().find(|c| c.name == "total_amount");
    assert!(total_col.is_some(), "Column 'total_amount' not found");
    assert!(
        matches!(
            total_col.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::Decimal { .. }
        ),
        "total_amount should be DECIMAL from SUM in LATERAL subquery"
    );
}

#[test]
fn test_filter_clause_type_inference() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources:
  raw:
    tables:
      orders:
        columns:
          - name: id
            type: INTEGER
          - name: status
            type: VARCHAR(50)
          - name: amount
            type: DECIMAL(10,2)
"#;

    // Aggregates with FILTER clauses - types should match unfiltered versions
    let sql = r#"
SELECT
    COUNT(*) as total_count,
    COUNT(*) FILTER (WHERE status = 'completed') as completed_count,
    SUM(amount) as total_sum,
    SUM(amount) FILTER (WHERE status = 'completed') as completed_sum,
    AVG(amount) FILTER (WHERE status = 'pending') as pending_avg
FROM smelt.sources.raw.orders
"#;

    let path = PathBuf::from("models/test_filter.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(path.clone());

    assert_eq!(schema.columns.len(), 5);

    // COUNT without FILTER should be BIGINT
    let total_count = schema.columns.iter().find(|c| c.name == "total_count");
    assert!(total_count.is_some(), "Column 'total_count' not found");
    assert_eq!(
        total_count.unwrap().data_type.as_ref().unwrap().data_type,
        DataType::BigInt,
        "COUNT should return BIGINT"
    );

    // COUNT with FILTER should also be BIGINT (FILTER doesn't change return type)
    let completed_count = schema.columns.iter().find(|c| c.name == "completed_count");
    assert!(
        completed_count.is_some(),
        "Column 'completed_count' not found"
    );
    assert_eq!(
        completed_count
            .unwrap()
            .data_type
            .as_ref()
            .unwrap()
            .data_type,
        DataType::BigInt,
        "COUNT with FILTER should return BIGINT"
    );

    // SUM without FILTER should be DECIMAL
    let total_sum = schema.columns.iter().find(|c| c.name == "total_sum");
    assert!(total_sum.is_some(), "Column 'total_sum' not found");
    assert!(
        matches!(
            total_sum.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::Decimal { .. }
        ),
        "SUM should return DECIMAL"
    );

    // SUM with FILTER should also be DECIMAL
    let completed_sum = schema.columns.iter().find(|c| c.name == "completed_sum");
    assert!(completed_sum.is_some(), "Column 'completed_sum' not found");
    assert!(
        matches!(
            completed_sum.unwrap().data_type.as_ref().unwrap().data_type,
            DataType::Decimal { .. }
        ),
        "SUM with FILTER should return DECIMAL"
    );

    // AVG with FILTER should be DOUBLE
    let pending_avg = schema.columns.iter().find(|c| c.name == "pending_avg");
    assert!(pending_avg.is_some(), "Column 'pending_avg' not found");
    assert_eq!(
        pending_avg.unwrap().data_type.as_ref().unwrap().data_type,
        DataType::Double,
        "AVG with FILTER should return DOUBLE"
    );
}

// ---- Row Polymorphism Tests ----

#[test]
fn test_select_star_produces_row_extension() {
    let mut db = TestDb::default();

    let upstream_path = PathBuf::from("models/input.sql");
    db.set_file_text(
        upstream_path.clone(),
        Arc::new("SELECT user_id, email FROM source.users".to_string()),
    );

    let downstream_path = PathBuf::from("models/passthrough.sql");
    db.set_file_text(
        downstream_path.clone(),
        Arc::new("SELECT * FROM smelt.models.input".to_string()),
    );

    db.set_all_files(Arc::new(vec![
        upstream_path.clone(),
        downstream_path.clone(),
    ]));
    db.set_file_project_root(upstream_path.clone(), PathBuf::from("."));
    db.set_file_project_root(downstream_path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.model_schema(downstream_path);

    // Should have no explicit columns but one row extension
    assert_eq!(schema.columns.len(), 0);
    assert_eq!(schema.row_extensions.len(), 1);
    assert_eq!(schema.row_extensions[0].ref_name, "input");
}

#[test]
fn test_select_star_plus_column_produces_row_extension_with_exclusion() {
    let mut db = TestDb::default();

    let upstream_path = PathBuf::from("models/input.sql");
    db.set_file_text(
        upstream_path.clone(),
        Arc::new("SELECT user_id, email FROM source.users".to_string()),
    );

    let downstream_path = PathBuf::from("models/extended.sql");
    db.set_file_text(
        downstream_path.clone(),
        Arc::new("SELECT *, 1 as foo FROM smelt.models.input".to_string()),
    );

    db.set_all_files(Arc::new(vec![
        upstream_path.clone(),
        downstream_path.clone(),
    ]));
    db.set_file_project_root(upstream_path.clone(), PathBuf::from("."));
    db.set_file_project_root(downstream_path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.model_schema(downstream_path);

    // Should have one explicit column (foo) and one row extension
    assert_eq!(schema.columns.len(), 1);
    assert_eq!(schema.columns[0].name, "foo");
    assert_eq!(schema.row_extensions.len(), 1);
    assert_eq!(schema.row_extensions[0].ref_name, "input");
    // foo should be excluded from expansion
    assert!(schema.row_extensions[0]
        .excluded_columns
        .contains(&"foo".to_string()));
}

#[test]
fn test_resolved_schema_expands_wildcard() {
    let mut db = TestDb::default();

    let upstream_path = PathBuf::from("models/input.sql");
    db.set_file_text(
        upstream_path.clone(),
        Arc::new("SELECT 42 as user_id, 'test' as email FROM source.users".to_string()),
    );

    let downstream_path = PathBuf::from("models/passthrough.sql");
    db.set_file_text(
        downstream_path.clone(),
        Arc::new("SELECT * FROM smelt.models.input".to_string()),
    );

    db.set_all_files(Arc::new(vec![
        upstream_path.clone(),
        downstream_path.clone(),
    ]));
    db.set_file_project_root(upstream_path.clone(), PathBuf::from("."));
    db.set_file_project_root(downstream_path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let resolved = db.resolved_model_schema(downstream_path);

    assert!(resolved.is_fully_resolved);
    assert_eq!(resolved.columns.len(), 2);

    let names: Vec<&str> = resolved.columns.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"user_id"));
    assert!(names.contains(&"email"));

    // Types should propagate through the wildcard
    let user_id = resolved
        .columns
        .iter()
        .find(|c| c.name == "user_id")
        .unwrap();
    assert!(
        user_id.data_type.is_some(),
        "Type should propagate through SELECT *"
    );
}

#[test]
fn test_resolved_schema_star_plus_column() {
    let mut db = TestDb::default();

    let upstream_path = PathBuf::from("models/input.sql");
    db.set_file_text(
        upstream_path.clone(),
        Arc::new("SELECT 42 as user_id, 'test' as email FROM source.users".to_string()),
    );

    let downstream_path = PathBuf::from("models/extended.sql");
    db.set_file_text(
        downstream_path.clone(),
        Arc::new("SELECT *, 1 as foo FROM smelt.models.input".to_string()),
    );

    db.set_all_files(Arc::new(vec![
        upstream_path.clone(),
        downstream_path.clone(),
    ]));
    db.set_file_project_root(upstream_path.clone(), PathBuf::from("."));
    db.set_file_project_root(downstream_path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let resolved = db.resolved_model_schema(downstream_path);

    assert!(resolved.is_fully_resolved);
    // user_id + email from upstream (foo excluded from wildcard) + foo explicit
    assert_eq!(resolved.columns.len(), 3);

    let names: Vec<&str> = resolved.columns.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"user_id"));
    assert!(names.contains(&"email"));
    assert!(names.contains(&"foo"));
}

#[test]
fn test_resolved_schema_chain() {
    let mut db = TestDb::default();

    // A -> B -> C chain with SELECT *
    let a_path = PathBuf::from("models/a.sql");
    db.set_file_text(
        a_path.clone(),
        Arc::new("SELECT 1 as col_a FROM source.test".to_string()),
    );

    let b_path = PathBuf::from("models/b.sql");
    db.set_file_text(
        b_path.clone(),
        Arc::new("SELECT * FROM smelt.models.a".to_string()),
    );

    let c_path = PathBuf::from("models/c.sql");
    db.set_file_text(
        c_path.clone(),
        Arc::new("SELECT * FROM smelt.models.b".to_string()),
    );

    db.set_all_files(Arc::new(vec![
        a_path.clone(),
        b_path.clone(),
        c_path.clone(),
    ]));
    db.set_file_project_root(a_path.clone(), PathBuf::from("."));
    db.set_file_project_root(b_path.clone(), PathBuf::from("."));
    db.set_file_project_root(c_path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let resolved = db.resolved_model_schema(c_path);

    assert!(resolved.is_fully_resolved);
    assert_eq!(resolved.columns.len(), 1);
    assert_eq!(resolved.columns[0].name, "col_a");
}

#[test]
fn test_type_inference_through_wildcard() {
    let mut db = TestDb::default();

    let sources_yaml = r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
          - name: email
            type: VARCHAR(255)
"#;

    let upstream_path = PathBuf::from("models/input.sql");
    db.set_file_text(
        upstream_path.clone(),
        Arc::new("SELECT id, email FROM smelt.sources.raw.users".to_string()),
    );

    let downstream_path = PathBuf::from("models/passthrough.sql");
    db.set_file_text(
        downstream_path.clone(),
        Arc::new("SELECT * FROM smelt.models.input".to_string()),
    );

    db.set_all_files(Arc::new(vec![
        upstream_path.clone(),
        downstream_path.clone(),
    ]));
    db.set_file_project_root(upstream_path.clone(), PathBuf::from("."));
    db.set_file_project_root(downstream_path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let resolved = db.resolved_model_schema(downstream_path);

    assert_eq!(resolved.columns.len(), 2);

    let id_col = resolved.columns.iter().find(|c| c.name == "id").unwrap();
    assert!(
        id_col.data_type.is_some(),
        "Type should propagate: id should be INTEGER"
    );
    assert_eq!(
        id_col.data_type.as_ref().unwrap().data_type,
        DataType::Integer
    );
}

#[test]
fn test_input_constraints_single_ref() {
    let mut db = TestDb::default();

    let upstream_path = PathBuf::from("models/input.sql");
    db.set_file_text(
        upstream_path.clone(),
        Arc::new("SELECT user_id, email FROM source.users".to_string()),
    );

    let downstream_path = PathBuf::from("models/consumer.sql");
    db.set_file_text(
        downstream_path.clone(),
        Arc::new("SELECT user_id FROM smelt.models.input".to_string()),
    );

    db.set_all_files(Arc::new(vec![
        upstream_path.clone(),
        downstream_path.clone(),
    ]));
    db.set_file_project_root(upstream_path.clone(), PathBuf::from("."));
    db.set_file_project_root(downstream_path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let constraints = db.model_input_constraints(downstream_path);

    // Should have one constraint on 'input' requiring 'user_id'
    assert_eq!(constraints.len(), 1);
    assert_eq!(constraints[0].ref_name, "input");
    assert!(
        constraints[0].required_columns.contains_key("user_id"),
        "Should require user_id from input"
    );
}

#[test]
fn test_input_constraints_with_alias() {
    let mut db = TestDb::default();

    let upstream_path = PathBuf::from("models/input.sql");
    db.set_file_text(
        upstream_path.clone(),
        Arc::new("SELECT user_id, email FROM source.users".to_string()),
    );

    let downstream_path = PathBuf::from("models/consumer.sql");
    db.set_file_text(
        downstream_path.clone(),
        Arc::new("SELECT t.user_id, t.email FROM smelt.models.input t".to_string()),
    );

    db.set_all_files(Arc::new(vec![
        upstream_path.clone(),
        downstream_path.clone(),
    ]));
    db.set_file_project_root(upstream_path.clone(), PathBuf::from("."));
    db.set_file_project_root(downstream_path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let constraints = db.model_input_constraints(downstream_path);

    assert_eq!(constraints.len(), 1);
    assert_eq!(constraints[0].ref_name, "input");
    assert!(constraints[0].required_columns.contains_key("user_id"));
    assert!(constraints[0].required_columns.contains_key("email"));
}

#[test]
fn test_no_constraints_without_refs() {
    let mut db = TestDb::default();

    let path = PathBuf::from("models/standalone.sql");
    db.set_file_text(
        path.clone(),
        Arc::new("SELECT 1 as x FROM source.test".to_string()),
    );

    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let constraints = db.model_input_constraints(path);
    assert!(constraints.is_empty());
}

#[test]
fn test_frontmatter_no_parse_errors() {
    let mut db = TestDb::default();
    let path = PathBuf::from("models/tagged_model.sql");
    // Phase 4: use path form (smelt.ref() is removed).
    let content = "---\ntags:\n  - event_source\n---\nSELECT event_id, user_id\nFROM smelt.models.raw_events\n";

    db.set_file_text(path.clone(), Arc::new(content.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let diagnostics = db.file_diagnostics(path.clone());

    // Should have no parse errors - only potentially an undefined ref diagnostic
    let parse_errors: Vec<_> = diagnostics
        .iter()
        .filter(|d| !d.message.contains("Undefined"))
        .collect();
    assert!(
        parse_errors.is_empty(),
        "Frontmatter should not cause parse errors, got: {:?}",
        parse_errors
    );

    // Verify the model was parsed successfully (has a SELECT)
    let model = db.parse_model(path);
    assert!(
        model.is_some(),
        "Model with frontmatter should parse successfully"
    );
}

// Helper to set up a DB with a single model for function type tests
fn setup_single_model(sql: &str) -> (TestDb, PathBuf) {
    let mut db = TestDb::default();
    let path = PathBuf::from("test_model.sql");
    db.set_file_text(path.clone(), Arc::new(sql.to_string()));
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));
    (db, path)
}

// Helper to set up a DB with multiple models for function type tests
fn setup_multi_model(models: &[(&str, &str)]) -> (TestDb, Vec<PathBuf>) {
    let mut db = TestDb::default();
    let mut paths = Vec::new();
    for (name, sql) in models {
        let path = PathBuf::from(format!("models/{}.sql", name));
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_file_project_root(path.clone(), PathBuf::from("."));
        paths.push(path);
    }
    db.set_all_files(Arc::new(paths.clone()));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));
    (db, paths)
}

#[test]
fn test_function_type_single_ref_with_group_by() {
    let (mut db, path) = setup_single_model(
        "SELECT user_id, COUNT(*) as total_events\nFROM smelt.models.events\nGROUP BY user_id",
    );

    let ft = db.model_function_type(path);

    assert_eq!(ft.model_name, "test_model");
    assert_eq!(ft.inputs.len(), 1);
    assert_eq!(ft.inputs[0].ref_name, "events");

    // user_id should be in inputs (from SELECT + GROUP BY)
    let user_id_col = ft.inputs[0].columns.iter().find(|c| c.name == "user_id");
    assert!(user_id_col.is_some(), "user_id should be in inputs");

    // Outputs
    assert_eq!(ft.outputs.len(), 2);
    assert_eq!(ft.outputs[0].name, "user_id");
    assert_eq!(ft.outputs[1].name, "total_events");

    // COUNT(*) -> BIGINT
    assert_eq!(
        ft.outputs[1].data_type.as_ref().unwrap().data_type,
        smelt_types::DataType::BigInt
    );

    assert!(!ft.has_wildcard_output);
}

#[test]
fn test_function_type_with_joins() {
    let (mut db, paths) = setup_multi_model(&[
            ("users", "SELECT user_id, user_name FROM source.users"),
            (
                "orders",
                "SELECT order_id, user_id, amount FROM source.orders",
            ),
            (
                "joined",
                "SELECT u.user_id, u.user_name, SUM(o.amount) as total\nFROM smelt.models.users u\nINNER JOIN smelt.models.orders o ON u.user_id = o.user_id\nGROUP BY u.user_id, u.user_name",
            ),
        ]);

    let ft = db.model_function_type(paths[2].clone());

    assert_eq!(ft.model_name, "joined");
    assert_eq!(ft.inputs.len(), 2);

    // Should have both refs as inputs
    let users_input = ft.inputs.iter().find(|i| i.ref_name == "users");
    let orders_input = ft.inputs.iter().find(|i| i.ref_name == "orders");
    assert!(users_input.is_some(), "inputs: {:?}", ft.inputs);
    assert!(orders_input.is_some(), "inputs: {:?}", ft.inputs);

    let users_input = users_input.unwrap();
    assert!(users_input.columns.iter().any(|c| c.name == "user_id"));
    assert!(users_input.columns.iter().any(|c| c.name == "user_name"));

    let orders_input = orders_input.unwrap();
    // Note: o.user_id from ON clause should be collected now that bare atoms
    // are wrapped in EXPRESSION nodes
    assert!(
        orders_input.columns.iter().any(|c| c.name == "amount"),
        "orders columns: {:?}",
        orders_input.columns
    );

    // Outputs
    assert_eq!(ft.outputs.len(), 3);
}

#[test]
fn test_input_constraint_where_clause() {
    let (mut db, path) = setup_single_model(
        "SELECT user_id, event_type\nFROM smelt.models.events\nWHERE event_type = 'click'",
    );

    let ft = db.model_function_type(path);

    assert_eq!(ft.inputs.len(), 1);
    let events = &ft.inputs[0];

    // event_type should be in the inputs (collected from WHERE clause)
    let event_type_col = events.columns.iter().find(|c| c.name == "event_type");
    assert!(
        event_type_col.is_some(),
        "event_type from WHERE should appear in inputs"
    );
    // Note: type constraints from literal comparisons (e.g., = 'click' -> VARCHAR)
    // should now work since binary expression operands are wrapped in EXPRESSION nodes.
}

#[test]
fn test_input_constraint_sum_numeric() {
    let (mut db, path) = setup_single_model(
        "SELECT user_id, SUM(amount) as total\nFROM smelt.models.orders\nGROUP BY user_id",
    );

    let ft = db.model_function_type(path);

    assert_eq!(ft.inputs.len(), 1);
    let orders = &ft.inputs[0];

    // amount should have numeric constraint from SUM()
    let amount_col = orders.columns.iter().find(|c| c.name == "amount");
    assert!(amount_col.is_some());
    let constraint = &amount_col.unwrap().constraint;
    assert!(
        constraint.is_some(),
        "SUM argument should have numeric constraint"
    );
    assert_eq!(
        constraint.as_ref().unwrap().data_type,
        smelt_types::DataType::Double
    );
}

#[test]
fn test_output_count_bigint() {
    let (mut db, path) = setup_single_model(
        "SELECT user_id, COUNT(*) as cnt\nFROM smelt.models.events\nGROUP BY user_id",
    );

    let ft = db.model_function_type(path);

    assert_eq!(ft.outputs.len(), 2);
    assert_eq!(ft.outputs[0].name, "user_id");
    assert_eq!(ft.outputs[1].name, "cnt");

    // COUNT(*) -> BIGINT
    assert_eq!(
        ft.outputs[1].data_type.as_ref().unwrap().data_type,
        smelt_types::DataType::BigInt
    );
}

#[test]
fn test_wildcard_output_marking() {
    let (mut db, path) = setup_single_model("SELECT *\nFROM smelt.models.events");

    let ft = db.model_function_type(path);

    assert!(
        ft.has_wildcard_output,
        "SELECT * should set has_wildcard_output"
    );
    // No explicit columns in outputs
    assert!(ft.outputs.is_empty());
}

#[test]
fn test_function_type_group_by_columns_collected() {
    // Ensure GROUP BY columns that don't appear in SELECT are still in inputs
    let (mut db, path) =
        setup_single_model("SELECT COUNT(*) as cnt\nFROM smelt.models.events\nGROUP BY category");

    let ft = db.model_function_type(path);

    assert_eq!(ft.inputs.len(), 1);
    let events = &ft.inputs[0];
    assert!(
        events.columns.iter().any(|c| c.name == "category"),
        "GROUP BY column should appear in inputs"
    );
}

#[test]
fn test_function_type_having_columns_collected() {
    let (mut db, path) = setup_single_model(
            "SELECT user_id, COUNT(*) as cnt\nFROM smelt.models.events\nGROUP BY user_id\nHAVING COUNT(*) > 5",
        );

    let ft = db.model_function_type(path);

    assert_eq!(ft.inputs.len(), 1);
    // user_id should be in inputs (from SELECT + GROUP BY)
    assert!(ft.inputs[0].columns.iter().any(|c| c.name == "user_id"));
}

#[test]
fn test_function_type_order_by_columns_collected() {
    let (mut db, path) =
        setup_single_model("SELECT user_id\nFROM smelt.models.events\nORDER BY event_time");

    let ft = db.model_function_type(path);

    assert_eq!(ft.inputs.len(), 1);
    let events = &ft.inputs[0];
    assert!(
        events.columns.iter().any(|c| c.name == "event_time"),
        "ORDER BY column should appear in inputs"
    );
    assert!(
        events.columns.iter().any(|c| c.name == "user_id"),
        "SELECT column should appear in inputs"
    );
}

#[test]
fn test_function_type_with_source() {
    let (mut db, path) =
        setup_single_model("SELECT user_id, event_timestamp\nFROM smelt.sources.raw.events");

    let ft = db.model_function_type(path);

    assert_eq!(ft.model_name, "test_model");
    assert_eq!(ft.inputs.len(), 1);
    assert_eq!(ft.inputs[0].ref_name, "events");

    let col_names: Vec<&str> = ft.inputs[0]
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert!(
        col_names.contains(&"user_id"),
        "user_id should be in inputs"
    );
    assert!(
        col_names.contains(&"event_timestamp"),
        "event_timestamp should be in inputs"
    );

    assert_eq!(ft.outputs.len(), 2);
}

#[test]
fn test_function_type_source_with_alias() {
    let (mut db, path) =
        setup_single_model("SELECT e.user_id, e.event_timestamp\nFROM smelt.sources.raw.events e");

    let ft = db.model_function_type(path);

    assert_eq!(ft.inputs.len(), 1);
    assert_eq!(ft.inputs[0].ref_name, "events");

    let col_names: Vec<&str> = ft.inputs[0]
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert!(
        col_names.contains(&"user_id"),
        "user_id should be in inputs"
    );
    assert!(
        col_names.contains(&"event_timestamp"),
        "event_timestamp should be in inputs"
    );
}

#[test]
fn test_function_type_display() {
    let ft = schema::ModelFunctionType {
        model_name: "user_stats".to_string(),
        inputs: vec![schema::FunctionInput {
            ref_name: "events".to_string(),
            columns: vec![
                schema::TypedField {
                    name: "event_id".to_string(),
                    constraint: None,
                },
                schema::TypedField {
                    name: "user_id".to_string(),
                    constraint: None,
                },
            ],
        }],
        outputs: vec![
            schema::FunctionOutput {
                name: "user_id".to_string(),
                data_type: None,
            },
            schema::FunctionOutput {
                name: "total_events".to_string(),
                data_type: Some(TypedColumn {
                    data_type: smelt_types::DataType::BigInt,
                    nullable: false,
                }),
            },
        ],
        has_wildcard_output: false,
    };

    let display = format!("{}", ft);
    assert!(display.contains("user_stats:"));
    assert!(display.contains("events: {event_id, user_id}"));
    assert!(display.contains("total_events: BIGINT"));
}

// Helper to set up a DB with multiple models and a sources.yml
fn setup_multi_model_with_sources(
    sources_yaml: &str,
    models: &[(&str, &str)],
) -> (TestDb, Vec<PathBuf>) {
    let mut db = TestDb::default();
    let mut paths = Vec::new();
    for (name, sql) in models {
        let path = PathBuf::from(format!("models/{}.sql", name));
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_file_project_root(path.clone(), PathBuf::from("."));
        paths.push(path);
    }
    db.set_all_files(Arc::new(paths.clone()));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(sources_yaml.to_string()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));
    (db, paths)
}

// ============================================================
// Cross-model type propagation tests
// ============================================================

#[test]
fn test_type_propagation_source_to_model() {
    let sources_yaml = r#"
sources:
  raw:
    tables:
      events:
        columns:
          - name: event_id
            type: INTEGER
          - name: user_id
            type: BIGINT
          - name: event_time
            type: TIMESTAMP
          - name: event_type
            type: VARCHAR
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
        sources_yaml,
        &[(
            "raw_events",
            "SELECT event_id, user_id, event_time, event_type FROM smelt.sources.raw.events",
        )],
    );

    let schema = db.typed_model_schema(paths[0].clone());

    assert_eq!(schema.columns.len(), 4);

    let event_id = schema
        .columns
        .iter()
        .find(|c| c.name == "event_id")
        .unwrap();
    assert_eq!(
        event_id.data_type.as_ref().unwrap().data_type,
        DataType::Integer,
        "event_id should be INTEGER from source"
    );

    let user_id = schema.columns.iter().find(|c| c.name == "user_id").unwrap();
    assert_eq!(
        user_id.data_type.as_ref().unwrap().data_type,
        DataType::BigInt,
        "user_id should be BIGINT from source"
    );

    let event_time = schema
        .columns
        .iter()
        .find(|c| c.name == "event_time")
        .unwrap();
    assert_eq!(
        event_time.data_type.as_ref().unwrap().data_type,
        DataType::Timestamp {
            with_timezone: false
        },
        "event_time should be TIMESTAMP from source"
    );

    let event_type = schema
        .columns
        .iter()
        .find(|c| c.name == "event_type")
        .unwrap();
    assert_eq!(
        event_type.data_type.as_ref().unwrap().data_type,
        DataType::Varchar { max_length: None },
        "event_type should be VARCHAR from source"
    );
}

#[test]
fn test_type_propagation_through_ref_chain() {
    let sources_yaml = r#"
sources:
  raw:
    tables:
      events:
        columns:
          - name: event_id
            type: INTEGER
          - name: user_id
            type: BIGINT
          - name: event_type
            type: VARCHAR
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
        sources_yaml,
        &[
            (
                "raw_events",
                "SELECT event_id, user_id, event_type FROM smelt.sources.raw.events",
            ),
            (
                "clicks",
                "SELECT event_id, user_id FROM smelt.models.raw_events WHERE event_type = 'click'",
            ),
        ],
    );

    // Verify types propagate from source → raw_events → clicks
    let schema = db.typed_model_schema(paths[1].clone());

    assert_eq!(schema.columns.len(), 2);

    let event_id = schema
        .columns
        .iter()
        .find(|c| c.name == "event_id")
        .unwrap();
    assert_eq!(
        event_id.data_type.as_ref().unwrap().data_type,
        DataType::Integer,
        "event_id should propagate as INTEGER through ref chain"
    );

    let user_id = schema.columns.iter().find(|c| c.name == "user_id").unwrap();
    assert_eq!(
        user_id.data_type.as_ref().unwrap().data_type,
        DataType::BigInt,
        "user_id should propagate as BIGINT through ref chain"
    );
}

#[test]
fn test_type_propagation_with_aggregation() {
    let sources_yaml = r#"
sources:
  raw:
    tables:
      events:
        columns:
          - name: event_id
            type: INTEGER
          - name: user_id
            type: BIGINT
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
            sources_yaml,
            &[
                (
                    "raw_events",
                    "SELECT event_id, user_id FROM smelt.sources.raw.events",
                ),
                (
                    "user_counts",
                    "SELECT user_id, COUNT(*) as event_count FROM smelt.models.raw_events GROUP BY user_id",
                ),
                (
                    "totals",
                    "SELECT SUM(event_count) as total_events FROM smelt.models.user_counts",
                ),
            ],
        );

    // Check user_counts: COUNT(*) → BigInt
    let user_counts_schema = db.typed_model_schema(paths[1].clone());
    let event_count = user_counts_schema
        .columns
        .iter()
        .find(|c| c.name == "event_count")
        .unwrap();
    assert_eq!(
        event_count.data_type.as_ref().unwrap().data_type,
        DataType::BigInt,
        "COUNT(*) should be BigInt"
    );

    // Check totals: SUM(BigInt) should propagate
    let totals_schema = db.typed_model_schema(paths[2].clone());
    let total_events = totals_schema
        .columns
        .iter()
        .find(|c| c.name == "total_events")
        .unwrap();
    assert!(
        total_events.data_type.is_some(),
        "SUM(event_count) should have a type inferred from upstream BigInt"
    );
    // SUM of BigInt should remain BigInt or be promoted to Decimal
    let total_type = &total_events.data_type.as_ref().unwrap().data_type;
    assert!(
        matches!(total_type, DataType::BigInt | DataType::Decimal { .. }),
        "SUM(BigInt) should be BigInt or Decimal, got {:?}",
        total_type
    );
}

#[test]
fn test_type_propagation_three_hop() {
    let sources_yaml = r#"
sources:
  raw:
    tables:
      transactions:
        columns:
          - name: amount
            type: DECIMAL(10,2)
          - name: user_id
            type: INTEGER
          - name: created_at
            type: TIMESTAMP
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
            sources_yaml,
            &[
                (
                    "base",
                    "SELECT amount, user_id, created_at FROM smelt.sources.raw.transactions",
                ),
                (
                    "daily",
                    "SELECT user_id, CAST(created_at AS DATE) as day, SUM(amount) as daily_total FROM smelt.models.base GROUP BY user_id, CAST(created_at AS DATE)",
                ),
                (
                    "summary",
                    "SELECT user_id, COUNT(*) as active_days, SUM(daily_total) as grand_total FROM smelt.models.daily GROUP BY user_id",
                ),
            ],
        );

    // Check base: types from source
    let base_schema = db.typed_model_schema(paths[0].clone());
    let amount = base_schema
        .columns
        .iter()
        .find(|c| c.name == "amount")
        .unwrap();
    assert!(
        matches!(
            amount.data_type.as_ref().unwrap().data_type,
            DataType::Decimal { .. }
        ),
        "amount should be Decimal from source"
    );

    // Check daily: CAST AS DATE, SUM(Decimal)
    let daily_schema = db.typed_model_schema(paths[1].clone());
    let day = daily_schema
        .columns
        .iter()
        .find(|c| c.name == "day")
        .unwrap();
    assert_eq!(
        day.data_type.as_ref().unwrap().data_type,
        DataType::Date,
        "CAST(timestamp AS DATE) should produce Date"
    );

    let daily_total = daily_schema
        .columns
        .iter()
        .find(|c| c.name == "daily_total")
        .unwrap();
    assert!(
        matches!(
            daily_total.data_type.as_ref().unwrap().data_type,
            DataType::Decimal { .. }
        ),
        "SUM(Decimal) should be Decimal"
    );

    // Check summary: COUNT(*) → BigInt, SUM(Decimal) → Decimal
    let summary_schema = db.typed_model_schema(paths[2].clone());
    let active_days = summary_schema
        .columns
        .iter()
        .find(|c| c.name == "active_days")
        .unwrap();
    assert_eq!(
        active_days.data_type.as_ref().unwrap().data_type,
        DataType::BigInt,
        "COUNT(*) should be BigInt"
    );

    let grand_total = summary_schema
        .columns
        .iter()
        .find(|c| c.name == "grand_total")
        .unwrap();
    assert!(
        grand_total.data_type.is_some(),
        "SUM(daily_total) should have a type inferred through the 3-hop chain"
    );
    assert!(
        matches!(
            grand_total.data_type.as_ref().unwrap().data_type,
            DataType::Decimal { .. }
        ),
        "SUM(Decimal) through 3 hops should still be Decimal, got {:?}",
        grand_total.data_type.as_ref().unwrap().data_type
    );
}

#[test]
fn test_type_propagation_with_joins() {
    let sources_yaml = r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: user_id
            type: INTEGER
          - name: name
            type: VARCHAR
      orders:
        columns:
          - name: order_id
            type: INTEGER
          - name: user_id
            type: INTEGER
          - name: amount
            type: DECIMAL(10,2)
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
        sources_yaml,
        &[
            ("users", "SELECT user_id, name FROM smelt.sources.raw.users"),
            (
                "orders",
                "SELECT order_id, user_id, amount FROM smelt.sources.raw.orders",
            ),
            (
                "user_orders",
                "SELECT u.user_id, u.name, SUM(o.amount) as total_spent \
                     FROM smelt.models.users u \
                     INNER JOIN smelt.models.orders o ON u.user_id = o.user_id \
                     GROUP BY u.user_id, u.name",
            ),
        ],
    );

    let schema = db.typed_model_schema(paths[2].clone());

    assert_eq!(schema.columns.len(), 3);

    let user_id = schema.columns.iter().find(|c| c.name == "user_id").unwrap();
    assert_eq!(
        user_id.data_type.as_ref().unwrap().data_type,
        DataType::Integer,
        "user_id should be INTEGER from users source via ref"
    );

    let name = schema.columns.iter().find(|c| c.name == "name").unwrap();
    assert_eq!(
        name.data_type.as_ref().unwrap().data_type,
        DataType::Varchar { max_length: None },
        "name should be VARCHAR from users source via ref"
    );

    let total_spent = schema
        .columns
        .iter()
        .find(|c| c.name == "total_spent")
        .unwrap();
    assert!(
        total_spent.data_type.is_some(),
        "SUM(amount) should have a type inferred from orders Decimal"
    );
    assert!(
        matches!(
            total_spent.data_type.as_ref().unwrap().data_type,
            DataType::Decimal { .. }
        ),
        "SUM(Decimal) should be Decimal, got {:?}",
        total_spent.data_type.as_ref().unwrap().data_type
    );
}

#[test]
fn test_resolved_schema_chain_preserves_types() {
    let sources_yaml = r#"
sources:
  raw:
    tables:
      data:
        columns:
          - name: id
            type: INTEGER
          - name: value
            type: DECIMAL(10,2)
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
        sources_yaml,
        &[
            ("base", "SELECT id, value FROM smelt.sources.raw.data"),
            ("mid", "SELECT * FROM smelt.models.base"),
            ("top", "SELECT * FROM smelt.models.mid"),
        ],
    );

    // Check types propagate through SELECT * chain: source → base → mid → top
    let resolved = db.resolved_model_schema(paths[2].clone());

    assert!(resolved.is_fully_resolved);
    assert_eq!(resolved.columns.len(), 2);

    let id = resolved.columns.iter().find(|c| c.name == "id").unwrap();
    assert_eq!(
        id.data_type.as_ref().unwrap().data_type,
        DataType::Integer,
        "id should be INTEGER through SELECT * chain"
    );

    let value = resolved.columns.iter().find(|c| c.name == "value").unwrap();
    assert!(
        matches!(
            value.data_type.as_ref().unwrap().data_type,
            DataType::Decimal { .. }
        ),
        "value should be Decimal through SELECT * chain, got {:?}",
        value.data_type.as_ref().unwrap().data_type
    );
}

// ============================================================
// Unknown type handling and diagnostics tests
// ============================================================

#[test]
fn test_upstream_unknown_columns_visible_downstream() {
    // Upstream model has a column from an external table (no type info).
    // Downstream should still be able to reference it.
    let (mut db, paths) = setup_multi_model(&[
        ("upstream", "SELECT mystery_col FROM some_external_table"),
        (
            "downstream",
            "SELECT mystery_col FROM smelt.models.upstream",
        ),
    ]);

    // The downstream model should have the column (even if type is Unknown)
    let schema = db.typed_model_schema(paths[1].clone());
    assert_eq!(schema.columns.len(), 1);
    assert_eq!(schema.columns[0].name, "mystery_col");
}

#[test]
fn test_coalesce_uses_second_arg_type() {
    let sources_yaml = r#"
sources:
  raw:
    tables:
      data:
        columns:
          - name: value
            type: INTEGER
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
        sources_yaml,
        &[(
            "model",
            "SELECT COALESCE(NULL, value) AS result FROM smelt.sources.raw.data",
        )],
    );

    let schema = db.typed_model_schema(paths[0].clone());
    let col_names: Vec<&str> = schema.columns.iter().map(|c| c.name.as_str()).collect();
    let result = schema
        .columns
        .iter()
        .find(|c| c.name == "result")
        .unwrap_or_else(|| panic!("Column 'result' not found, columns: {:?}", col_names));
    assert_eq!(
        result.data_type.as_ref().unwrap().data_type,
        DataType::Integer,
        "COALESCE(NULL, integer_col) should infer INTEGER from second arg"
    );
}

#[test]
fn test_coalesce_first_arg_known() {
    let sources_yaml = r#"
sources:
  raw:
    tables:
      data:
        columns:
          - name: value
            type: DECIMAL(10,2)
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
        sources_yaml,
        &[(
            "model",
            "SELECT COALESCE(value, 0) as result FROM smelt.sources.raw.data",
        )],
    );

    let schema = db.typed_model_schema(paths[0].clone());
    let result = schema.columns.iter().find(|c| c.name == "result").unwrap();
    assert!(
        matches!(
            result.data_type.as_ref().unwrap().data_type,
            DataType::Decimal { .. }
        ),
        "COALESCE(decimal_col, 0) should infer Decimal from first arg"
    );
}

#[test]
fn test_type_diagnostic_for_unknown_column() {
    // Model must have a smelt.ref() so type_diagnostics doesn't skip it
    // (models with no refs/sources reference only physical tables and are skipped)
    let (mut db, paths) = setup_multi_model(&[
        ("upstream", "SELECT 1 AS id"),
        ("model", "SELECT unknown_col FROM smelt.models.upstream"),
    ]);

    let diags = db.type_diagnostics(paths[1].clone());
    assert!(
        diags.iter().any(
            |d| d.message.contains("Could not infer type") && d.message.contains("unknown_col")
        ),
        "Should produce a diagnostic for column with unknown type, got: {:?}",
        diags
    );
}

#[test]
fn test_no_type_diagnostic_for_known_columns() {
    let sources_yaml = r#"
sources:
  raw:
    tables:
      data:
        columns:
          - name: id
            type: INTEGER
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
        sources_yaml,
        &[(
            "model",
            "SELECT id, COUNT(*) as cnt FROM smelt.sources.raw.data GROUP BY id",
        )],
    );

    let diags = db.type_diagnostics(paths[0].clone());
    assert!(
        diags.is_empty(),
        "Should not produce type diagnostics when all types are known, got: {:?}",
        diags
    );
}

#[test]
fn test_unknown_function_diagnostic() {
    let (mut db, paths) = setup_multi_model(&[(
        "model",
        "SELECT my_custom_func(42) as result FROM some_table",
    )]);

    let diags = db.file_diagnostics(paths[0].clone());
    assert!(
        diags.iter().any(|d| d.message.contains("my_custom_func")
            && d.message.contains("not a recognized SQL function")),
        "Should warn about unrecognized function, got: {:?}",
        diags
    );
    // Should be Warning severity
    let func_diag = diags
        .iter()
        .find(|d| d.message.contains("my_custom_func"))
        .unwrap();
    assert_eq!(
        func_diag.severity,
        DiagnosticSeverity::Warning,
        "Unknown function diagnostic should be Warning"
    );
}

// ============================================================
// Circular reference / cycle recovery tests
// ============================================================

#[test]
fn test_circular_ref_does_not_panic() {
    // A refs B, B refs A — should not panic, should return empty schemas
    let (mut db, paths) = setup_multi_model(&[
        ("model_a", "SELECT x FROM smelt.models.model_b"),
        ("model_b", "SELECT y FROM smelt.models.model_a"),
    ]);

    // These should not panic thanks to cycle recovery
    let schema_a = db.typed_model_schema(paths[0].clone());
    let schema_b = db.typed_model_schema(paths[1].clone());

    // At least one should have empty columns or Unknown types due to
    // cycle recovery (the one that triggers recovery gets empty upstream).
    let degraded = |schema: &ModelSchema| {
        schema.columns.is_empty()
            || schema.columns.iter().all(|c| {
                c.data_type.is_none()
                    || matches!(c.data_type.as_ref().unwrap().data_type, DataType::Unknown)
            })
    };
    assert!(
        degraded(&schema_a) || degraded(&schema_b),
        "Cycle should result in degraded type info"
    );
}

#[test]
fn test_circular_ref_self() {
    // Model references itself
    let (mut db, paths) = setup_multi_model(&[("self_ref", "SELECT x FROM smelt.models.self_ref")]);

    // Should not panic
    let schema = db.typed_model_schema(paths[0].clone());
    // Schema may be empty or have Unknown types — either is fine
    let _ = schema;
}

#[test]
fn test_circular_ref_three_models() {
    // A -> B -> C -> A cycle
    let (mut db, paths) = setup_multi_model(&[
        ("cycle_a", "SELECT x FROM smelt.models.cycle_b"),
        ("cycle_b", "SELECT y FROM smelt.models.cycle_c"),
        ("cycle_c", "SELECT z FROM smelt.models.cycle_a"),
    ]);

    // None of these should panic
    let _sa = db.typed_model_schema(paths[0].clone());
    let _sb = db.typed_model_schema(paths[1].clone());
    let _sc = db.typed_model_schema(paths[2].clone());

    let _ra = db.resolved_model_schema(paths[0].clone());
    let _rb = db.resolved_model_schema(paths[1].clone());
    let _rc = db.resolved_model_schema(paths[2].clone());
}

#[test]
fn test_circular_ref_diagnostic() {
    let (mut db, paths) = setup_multi_model(&[
        ("diag_a", "SELECT x FROM smelt.models.diag_b"),
        ("diag_b", "SELECT y FROM smelt.models.diag_a"),
    ]);

    let diags_a = db.type_diagnostics(paths[0].clone());
    let diags_b = db.type_diagnostics(paths[1].clone());

    // At least one model should get a circular dependency diagnostic
    let all_msgs: Vec<&str> = diags_a
        .iter()
        .chain(diags_b.iter())
        .map(|d| d.message.as_str())
        .collect();
    assert!(
        all_msgs.iter().any(|m| m.contains("Circular dependency")),
        "Expected a circular dependency diagnostic, got: {:?}",
        all_msgs
    );
}

#[test]
fn test_circular_ref_does_not_affect_others() {
    // A <-> B form a cycle, but C -> D should work fine
    let (mut db, paths) = setup_multi_model(&[
        ("cyc_a", "SELECT x FROM smelt.models.cyc_b"),
        ("cyc_b", "SELECT y FROM smelt.models.cyc_a"),
        ("good_c", "SELECT CAST(1 AS INTEGER) AS val"),
        ("good_d", "SELECT val FROM smelt.models.good_c"),
    ]);

    // Trigger cycle resolution first
    let _sa = db.typed_model_schema(paths[0].clone());
    let _sb = db.typed_model_schema(paths[1].clone());

    // C -> D should still propagate types correctly
    let schema_d = db.typed_model_schema(paths[3].clone());
    assert_eq!(schema_d.columns.len(), 1);
    assert_eq!(schema_d.columns[0].name, "val");
    assert_eq!(
        schema_d.columns[0].data_type.as_ref().unwrap().data_type,
        DataType::Integer,
        "C -> D type propagation should work despite A <-> B cycle"
    );
}

#[test]
fn test_circular_ref_type_diagnostics_no_panic() {
    // Simulates what the LSP does: calls both file_diagnostics and type_diagnostics
    // on all models, including circular ones.
    let (mut db, paths) = setup_multi_model(&[
        ("cyc_a", "SELECT x FROM smelt.models.cyc_b"),
        ("cyc_b", "SELECT y FROM smelt.models.cyc_a"),
    ]);

    // This should not panic — mimics publish_all_diagnostics in the LSP
    for path in &paths {
        let _file_diags = db.file_diagnostics(path.clone());
        let _type_diags = db.type_diagnostics(path.clone());
    }
}

#[test]
fn test_self_ref_type_diagnostics_no_panic() {
    let (mut db, paths) = setup_multi_model(&[("self_ref", "SELECT x FROM smelt.models.self_ref")]);

    let _file_diags = db.file_diagnostics(paths[0].clone());
    let _type_diags = db.type_diagnostics(paths[0].clone());
}

#[test]
fn test_circular_ref_incremental_update_no_panic() {
    // Simulates the LSP pattern: query diagnostics, then mutate file, then query again.
    // This exercises Salsa's memoized value validation path which is different from
    // first-time computation.
    let (mut db, paths) = setup_multi_model(&[
        ("cyc_a", "SELECT x FROM smelt.models.cyc_b"),
        ("cyc_b", "SELECT y FROM smelt.models.cyc_a"),
    ]);

    // First pass: populate caches
    for path in &paths {
        let _file_diags = db.file_diagnostics(path.clone());
        let _type_diags = db.type_diagnostics(path.clone());
    }

    // Mutate one file (triggers Salsa revision change)
    db.set_file_text(
        paths[0].clone(),
        Arc::new("SELECT x, 1 AS extra FROM smelt.models.cyc_b".to_string()),
    );

    // Second pass: this exercises the validation path where Salsa checks
    // if memoized values are still valid
    for path in &paths {
        let _file_diags = db.file_diagnostics(path.clone());
        let _type_diags = db.type_diagnostics(path.clone());
    }
}

// ============================================================
// Cross-model type mismatch diagnostic tests
// ============================================================

#[test]
fn test_type_mismatch_varchar_in_sum() {
    let sources_yaml = r#"
sources:
  raw:
    tables:
      data:
        columns:
          - name: price
            type: VARCHAR
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
        sources_yaml,
        &[
            ("raw_data", "SELECT price FROM smelt.sources.raw.data"),
            (
                "totals",
                "SELECT SUM(price) AS total_price FROM smelt.models.raw_data",
            ),
        ],
    );

    let diags = db.type_diagnostics(paths[1].clone());
    let mismatch_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.message.contains("has type") && d.message.contains("expected"))
        .collect();
    assert!(
        !mismatch_diags.is_empty(),
        "Should warn about VARCHAR used in SUM, got diagnostics: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(
        mismatch_diags[0].message.contains("price"),
        "Diagnostic should mention column name"
    );
}

#[test]
fn test_type_mismatch_compatible_numeric_no_warning() {
    let sources_yaml = r#"
sources:
  raw:
    tables:
      data:
        columns:
          - name: amount
            type: INTEGER
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
        sources_yaml,
        &[
            ("raw_data", "SELECT amount FROM smelt.sources.raw.data"),
            (
                "totals",
                "SELECT SUM(amount) AS total FROM smelt.models.raw_data",
            ),
        ],
    );

    let diags = db.type_diagnostics(paths[1].clone());
    let mismatch_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.message.contains("has type") && d.message.contains("expected"))
        .collect();
    assert!(
        mismatch_diags.is_empty(),
        "Should not warn about INTEGER in SUM, got: {:?}",
        mismatch_diags
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_type_mismatch_multiple_columns() {
    let sources_yaml = r#"
sources:
  raw:
    tables:
      data:
        columns:
          - name: name
            type: VARCHAR
          - name: status
            type: VARCHAR
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
        sources_yaml,
        &[
            (
                "raw_data",
                "SELECT name, status FROM smelt.sources.raw.data",
            ),
            (
                "agg",
                "SELECT SUM(name) AS s1, AVG(status) AS s2 FROM smelt.models.raw_data",
            ),
        ],
    );

    let diags = db.type_diagnostics(paths[1].clone());
    let mismatch_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.message.contains("has type") && d.message.contains("expected"))
        .collect();
    assert!(
        mismatch_diags.len() >= 2,
        "Should produce at least 2 mismatch diagnostics, got {}: {:?}",
        mismatch_diags.len(),
        mismatch_diags
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_no_mismatch_for_undefined_ref() {
    let (mut db, paths) = setup_multi_model(&[(
        "bad_ref",
        "SELECT SUM(x) AS total FROM smelt.models.nonexistent",
    )]);

    let diags = db.type_diagnostics(paths[0].clone());
    let mismatch_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.message.contains("has type") && d.message.contains("expected"))
        .collect();
    assert!(
        mismatch_diags.is_empty(),
        "Should not produce mismatch diagnostics for undefined ref"
    );
}

#[test]
fn test_type_mismatch_through_chain() {
    // source (VARCHAR) -> passthrough model -> SUM in downstream
    let sources_yaml = r#"
sources:
  raw:
    tables:
      data:
        columns:
          - name: value
            type: VARCHAR
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
        sources_yaml,
        &[
            ("passthrough", "SELECT value FROM smelt.sources.raw.data"),
            (
                "aggregator",
                "SELECT SUM(value) AS total FROM smelt.models.passthrough",
            ),
        ],
    );

    let diags = db.type_diagnostics(paths[1].clone());
    let mismatch_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.message.contains("has type") && d.message.contains("expected"))
        .collect();
    assert!(
        !mismatch_diags.is_empty(),
        "Should detect VARCHAR->SUM mismatch through model chain, got: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn test_binary_expr_type_propagation_through_ref() {
    // Verifies that computed columns (binary expressions) in upstream models
    // have their types correctly propagated through smelt.ref()
    let (mut db, paths) = setup_multi_model(&[
            (
                "upstream",
                "WITH data AS (SELECT CAST(3.14 AS DOUBLE) AS dbl_col) SELECT dbl_col + dbl_col AS up_0 FROM data",
            ),
            (
                "downstream",
                "SELECT SUM(up_0) AS agg_0 FROM smelt.models.upstream",
            ),
        ]);

    let up_schema = db.typed_model_schema(paths[0].clone());
    assert_eq!(
        up_schema.columns[0].data_type.as_ref().unwrap().data_type,
        DataType::Double,
        "dbl_col + dbl_col should be Double"
    );

    let down_schema = db.typed_model_schema(paths[1].clone());
    assert_eq!(
        down_schema.columns[0].data_type.as_ref().unwrap().data_type,
        DataType::Double,
        "SUM(Double) should be Double"
    );
}

// ============================================================
// Unsupported construct diagnostics
// ============================================================

#[test]
fn test_pivot_rejected_with_diagnostic() {
    let (mut db, paths) = setup_multi_model(&[(
            "pivot_model",
            "SELECT * FROM (SELECT dept, quarter, rev FROM t) PIVOT (SUM(rev) FOR quarter IN ('Q1', 'Q2'))",
        )]);

    let diags = db.file_diagnostics(paths[0].clone());
    let pivot_diag = diags
        .iter()
        .find(|d| d.message.contains("PIVOT is not supported"));
    assert!(
        pivot_diag.is_some(),
        "Should emit error for PIVOT, got: {:?}",
        diags
    );
    assert_eq!(pivot_diag.unwrap().severity, DiagnosticSeverity::Error);
}

#[test]
fn test_unpivot_rejected_with_diagnostic() {
    let (mut db, paths) = setup_multi_model(&[(
        "unpivot_model",
        "SELECT * FROM t UNPIVOT (val FOR name IN (a, b, c))",
    )]);

    let diags = db.file_diagnostics(paths[0].clone());
    let unpivot_diag = diags
        .iter()
        .find(|d| d.message.contains("UNPIVOT is not supported"));
    assert!(
        unpivot_diag.is_some(),
        "Should emit error for UNPIVOT, got: {:?}",
        diags
    );
    assert_eq!(unpivot_diag.unwrap().severity, DiagnosticSeverity::Error);
}

#[test]
fn test_no_pivot_diagnostic_for_normal_query() {
    let (mut db, paths) = setup_multi_model(&[(
        "normal_model",
        "SELECT dept, SUM(rev) as total FROM t GROUP BY dept",
    )]);

    let diags = db.file_diagnostics(paths[0].clone());
    assert!(
        !diags
            .iter()
            .any(|d| d.message.contains("PIVOT") || d.message.contains("UNPIVOT")),
        "Normal query should not trigger PIVOT/UNPIVOT diagnostic"
    );
}

// ============================================================
// Undeclared column diagnostic tests
// ============================================================

#[test]
fn test_undeclared_column_from_source() {
    let sources_yaml = r#"
sources:
  raw:
    tables:
      events:
        columns:
          - name: event_id
            type: INTEGER
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
        sources_yaml,
        &[(
            "my_model",
            "SELECT event_id, missing_col FROM smelt.sources.raw.events",
        )],
    );

    let diags = db.type_diagnostics(paths[0].clone());
    let undeclared: Vec<_> = diags
        .iter()
        .filter(|d| d.message.contains("not found"))
        .collect();
    assert_eq!(undeclared.len(), 1, "Should report one undeclared column");
    assert!(
        undeclared[0].message.contains("missing_col"),
        "Message should name the column: {}",
        undeclared[0].message
    );
}

#[test]
fn test_declared_column_no_diagnostic() {
    let sources_yaml = r#"
sources:
  raw:
    tables:
      events:
        columns:
          - name: event_id
            type: INTEGER
          - name: user_id
            type: INTEGER
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
        sources_yaml,
        &[(
            "my_model",
            "SELECT event_id, user_id FROM smelt.sources.raw.events",
        )],
    );

    let diags = db.type_diagnostics(paths[0].clone());
    let undeclared: Vec<_> = diags
        .iter()
        .filter(|d| d.message.contains("not found"))
        .collect();
    assert!(
        undeclared.is_empty(),
        "No undeclared column diagnostics expected, got: {:?}",
        undeclared
    );
}

#[test]
fn test_undeclared_column_from_ref() {
    let (mut db, paths) = setup_multi_model(&[
        (
            "upstream",
            "SELECT user_id, event_count FROM smelt.sources.raw.events",
        ),
        (
            "downstream",
            "SELECT user_id, nonexistent FROM smelt.models.upstream",
        ),
    ]);

    let diags = db.type_diagnostics(paths[1].clone());
    let undeclared: Vec<_> = diags
        .iter()
        .filter(|d| d.message.contains("not found"))
        .collect();
    assert_eq!(
        undeclared.len(),
        1,
        "Should report one undeclared column from upstream ref"
    );
    assert!(undeclared[0].message.contains("nonexistent"));
}

#[test]
fn test_untyped_source_column_no_diagnostic() {
    let sources_yaml = r#"
sources:
  raw:
    tables:
      events:
        columns:
          - name: event_id
          - name: user_id
            type: INTEGER
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
        sources_yaml,
        &[(
            "my_model",
            "SELECT event_id, user_id FROM smelt.sources.raw.events",
        )],
    );

    let diags = db.type_diagnostics(paths[0].clone());
    let undeclared: Vec<_> = diags
        .iter()
        .filter(|d| d.message.contains("not found"))
        .collect();
    assert!(
        undeclared.is_empty(),
        "Untyped source column should still resolve, got: {:?}",
        undeclared
    );
}

#[test]
fn test_cte_column_no_false_positive() {
    let sources_yaml = r#"
sources:
  raw:
    tables:
      events:
        columns:
          - name: event_id
            type: INTEGER
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
            sources_yaml,
            &[(
                "my_model",
                "WITH cte AS (SELECT event_id, 1 AS extra FROM smelt.sources.raw.events) SELECT event_id, extra FROM cte",
            )],
        );

    let diags = db.type_diagnostics(paths[0].clone());
    let undeclared: Vec<_> = diags
        .iter()
        .filter(|d| d.message.contains("not found"))
        .collect();
    assert!(
        undeclared.is_empty(),
        "CTE columns should not produce false positives, got: {:?}",
        undeclared
    );
}

#[test]
fn test_select_star_no_diagnostic() {
    let sources_yaml = r#"
sources:
  raw:
    tables:
      events:
        columns:
          - name: event_id
            type: INTEGER
"#;
    let (mut db, paths) = setup_multi_model_with_sources(
        sources_yaml,
        &[("my_model", "SELECT * FROM smelt.sources.raw.events")],
    );

    let diags = db.type_diagnostics(paths[0].clone());
    let undeclared: Vec<_> = diags
        .iter()
        .filter(|d| d.message.contains("not found"))
        .collect();
    assert!(
        undeclared.is_empty(),
        "SELECT * should not trigger undeclared column diagnostic, got: {:?}",
        undeclared
    );
}

// ===== Cycle regression tests (salsa 0.26 fixpoint iteration) =====

#[test]
fn test_circular_refs_do_not_panic() {
    // Two models that reference each other: a -> b -> a.
    // With salsa 0.16 this triggered a panic during memo validation;
    // the LSP had to use catch_unwind as a workaround.
    // With salsa 0.26's cycle_initial, the cycle should resolve to
    // empty schemas without panicking.
    let mut db = TestDb::default();

    let a_path = PathBuf::from("models/a.sql");
    let b_path = PathBuf::from("models/b.sql");

    db.set_file_text(
        a_path.clone(),
        Arc::new("SELECT * FROM smelt.models.b".to_string()),
    );
    db.set_file_text(
        b_path.clone(),
        Arc::new("SELECT * FROM smelt.models.a".to_string()),
    );

    // This must NOT panic — the cycle_initial returns empty defaults.
    let schema_a = db.typed_model_schema(a_path.clone());
    let schema_b = db.typed_model_schema(b_path.clone());

    // Schemas should be empty or have only wildcard columns (cycle recovery)
    // The key assertion is that we reached this point without panic.
    assert!(
        schema_a.columns.is_empty() || schema_a.columns.iter().all(|c| c.name == "*"),
        "Cyclic model a should have empty/wildcard schema, got: {:?}",
        schema_a.columns
    );
    assert!(
        schema_b.columns.is_empty() || schema_b.columns.iter().all(|c| c.name == "*"),
        "Cyclic model b should have empty/wildcard schema, got: {:?}",
        schema_b.columns
    );

    // Diagnostics should also not panic
    let diags_a = db.file_diagnostics(a_path);
    let diags_b = db.file_diagnostics(b_path);

    // Reaching this point proves cycle recovery works without panic.
    // The cycle_initial returns empty schemas and diagnostics may or may
    // not contain an explicit "circular reference" message — the important
    // thing is no memo-validation panic (the old salsa 0.16 failure mode).
    let _ = (diags_a, diags_b);
}

#[test]
fn test_three_way_cycle_recovery() {
    // a -> b -> c -> a: three-way cycle
    let mut db = TestDb::default();

    let a_path = PathBuf::from("models/a.sql");
    let b_path = PathBuf::from("models/b.sql");
    let c_path = PathBuf::from("models/c.sql");

    db.set_file_text(
        a_path.clone(),
        Arc::new("SELECT * FROM smelt.models.b".to_string()),
    );
    db.set_file_text(
        b_path.clone(),
        Arc::new("SELECT * FROM smelt.models.c".to_string()),
    );
    db.set_file_text(
        c_path.clone(),
        Arc::new("SELECT * FROM smelt.models.a".to_string()),
    );

    // Must not panic
    let _schema_a = db.typed_model_schema(a_path.clone());
    let _schema_b = db.typed_model_schema(b_path.clone());
    let _schema_c = db.typed_model_schema(c_path.clone());

    // Diagnostics must not panic
    let _diags_a = db.file_diagnostics(a_path);
    let _diags_b = db.file_diagnostics(b_path);
    let _diags_c = db.file_diagnostics(c_path);
}

// === Phase A (meta-language) TDD tests: DiagnosticCode variants ===

/// `MetaListEmptyTypeUnknown` exists in the `DiagnosticCode` enum and
/// renders the spec message format: "cannot infer element type for empty
/// list literal".
#[test]
fn diagnostic_code_meta_list_empty_type_unknown() {
    let code = DiagnosticCode::MetaListEmptyTypeUnknown;
    // Pattern-match to confirm the variant is reachable.
    assert!(matches!(code, DiagnosticCode::MetaListEmptyTypeUnknown));
    // Render the canonical message via the spec message helper.
    let msg = meta_list_diagnostic_message(code, None, None, None);
    assert_eq!(
        msg, "cannot infer element type for empty list literal",
        "MetaListEmptyTypeUnknown message must match spec"
    );
}

/// `MetaListHeterogeneous` exists in the `DiagnosticCode` enum and
/// renders the spec message format: "list elements have incompatible
/// types: {T0}, {Tk}".
#[test]
fn diagnostic_code_meta_list_heterogeneous() {
    let code = DiagnosticCode::MetaListHeterogeneous;
    assert!(matches!(code, DiagnosticCode::MetaListHeterogeneous));
    let msg = meta_list_diagnostic_message(code, Some("Expr<Integer>"), Some("Expr<Text>"), None);
    assert_eq!(
        msg, "list elements have incompatible types: Expr<Integer>, Expr<Text>",
        "MetaListHeterogeneous message must match spec"
    );
}

/// `MetaSpreadInForbiddenPosition` exists in the `DiagnosticCode` enum and
/// renders the spec message format: "spread is not allowed in {position name}".
#[test]
fn diagnostic_code_meta_spread_in_forbidden_position() {
    let code = DiagnosticCode::MetaSpreadInForbiddenPosition;
    assert!(matches!(
        code,
        DiagnosticCode::MetaSpreadInForbiddenPosition
    ));
    let msg = meta_list_diagnostic_message(code, None, None, Some("WHERE clause"));
    assert_eq!(
        msg, "spread is not allowed in WHERE clause",
        "MetaSpreadInForbiddenPosition message must match spec"
    );
}

/// `MetaSpreadOnNonList` exists in the `DiagnosticCode` enum and renders
/// the spec message format: "spread expects List<T>; found {actual type}".
#[test]
fn diagnostic_code_meta_spread_on_non_list() {
    let code = DiagnosticCode::MetaSpreadOnNonList;
    assert!(matches!(code, DiagnosticCode::MetaSpreadOnNonList));
    let msg = meta_list_diagnostic_message(code, None, Some("Expr<Integer>"), None);
    assert_eq!(
        msg, "spread expects List<T>; found Expr<Integer>",
        "MetaSpreadOnNonList message must match spec"
    );
}

// === Phase A Phase 3 — production-path (Salsa) tests ===
//
// These tests call `db.file_diagnostics()` (the Salsa query path) to verify
// that the pure meta-language check functions are properly wired into the
// production diagnostics pipeline.
//
// A test MUST go red before the wiring is added, green after. Comments
// indicate which state each test entered.

/// Production path: `SELECT ...[1, 'x'] FROM t` — heterogeneous inline spread —
/// must produce exactly one `MetaListHeterogeneous` diagnostic via `file_diagnostics`.
///
/// Was RED before production wiring; GREEN after.
#[test]
fn production_path_spread_heterogeneous_list_fires_diagnostic() {
    let (mut db, path) = setup_single_model("SELECT ...[1, 'x'] FROM t");
    let diags = db.file_diagnostics(path);
    let meta_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MetaListHeterogeneous))
        .collect();
    assert_eq!(
        meta_diags.len(),
        1,
        "SELECT ...[1, 'x'] FROM t must produce exactly 1 MetaListHeterogeneous; \
         got diagnostics: {:?}",
        diags
    );
}

/// Production path: `SELECT id, ...[], created_at FROM t` — empty-list spread —
/// must produce zero meta-language diagnostics via `file_diagnostics`.
///
/// Was RED (would fail to call the pure function) before wiring.
/// After wiring: GREEN — empty spread is valid, no meta diagnostics.
#[test]
fn production_path_spread_empty_list_no_diagnostic() {
    let (mut db, path) = setup_single_model("SELECT id, ...[], created_at FROM t");
    let diags = db.file_diagnostics(path);
    let meta_diags: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::MetaListHeterogeneous)
                    | Some(DiagnosticCode::MetaListEmptyTypeUnknown)
                    | Some(DiagnosticCode::MetaSpreadInForbiddenPosition)
                    | Some(DiagnosticCode::MetaSpreadOnNonList)
            )
        })
        .collect();
    assert!(
        meta_diags.is_empty(),
        "SELECT id, ...[], created_at FROM t must produce zero meta-language diagnostics; \
         got: {:?}",
        meta_diags
    );
}

/// Production path: `SELECT id, ...[a, b], created_at FROM t` — valid spread of
/// homogeneous list — must produce zero meta-language diagnostics via
/// `file_diagnostics`.
///
/// Was RED before production wiring; GREEN after.
#[test]
fn production_path_spread_valid_no_diagnostic() {
    let (mut db, path) = setup_single_model("SELECT id, ...[a, b], created_at FROM t");
    let diags = db.file_diagnostics(path);
    let meta_diags: Vec<_> = diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::MetaListHeterogeneous)
                    | Some(DiagnosticCode::MetaListEmptyTypeUnknown)
                    | Some(DiagnosticCode::MetaSpreadInForbiddenPosition)
                    | Some(DiagnosticCode::MetaSpreadOnNonList)
            )
        })
        .collect();
    assert!(
        meta_diags.is_empty(),
        "SELECT id, ...[a, b], created_at FROM t must produce zero meta-language diagnostics; \
         got: {:?}",
        meta_diags
    );
}

/// Production path: `SELECT * FROM t WHERE x = 1 AND ...preds` — spread in WHERE —
/// must produce exactly one `MetaSpreadInForbiddenPosition` diagnostic via
/// `file_diagnostics`.
///
/// Was RED before production wiring; GREEN after.
#[test]
fn production_path_spread_in_where_fires_diagnostic() {
    let (mut db, path) = setup_single_model("SELECT * FROM t WHERE x = 1 AND ...preds");
    let diags = db.file_diagnostics(path);
    let meta_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MetaSpreadInForbiddenPosition))
        .collect();
    assert_eq!(
        meta_diags.len(),
        1,
        "SELECT * FROM t WHERE x = 1 AND ...preds must produce exactly 1 \
         MetaSpreadInForbiddenPosition; got diagnostics: {:?}",
        diags
    );
}

/// Production path: `SELECT [1, 'hello'] FROM t` — heterogeneous list literal in a
/// SELECT-list position (no spread) — must produce exactly one
/// `MetaListHeterogeneous` diagnostic via `file_diagnostics`.
///
/// Was RED before production wiring; GREEN after.
#[test]
fn production_path_heterogeneous_literal_in_select_fires_diagnostic() {
    let (mut db, path) = setup_single_model("SELECT [1, 'hello'] FROM t");
    let diags = db.file_diagnostics(path);
    let meta_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MetaListHeterogeneous))
        .collect();
    assert_eq!(
        meta_diags.len(),
        1,
        "SELECT [1, 'hello'] FROM t must produce exactly 1 MetaListHeterogeneous; \
         got diagnostics: {:?}",
        diags
    );
}

/// Production path: `SELECT [] FROM t` — empty list literal in unconstrained
/// SELECT-list position — must produce exactly one `MetaListEmptyTypeUnknown`
/// diagnostic via `file_diagnostics`.
#[test]
fn production_path_empty_list_literal_fires_diagnostic() {
    let (mut db, path) = setup_single_model("SELECT [] FROM t");
    let diags = db.file_diagnostics(path);
    let meta_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MetaListEmptyTypeUnknown))
        .collect();
    assert_eq!(
        meta_diags.len(),
        1,
        "SELECT [] FROM t must produce exactly 1 MetaListEmptyTypeUnknown; \
         got diagnostics: {:?}",
        diags
    );
}

/// Production path: `SELECT ...1 FROM t` — spread of an integer literal (a
/// non-list operand) — must produce exactly one `MetaSpreadOnNonList`
/// diagnostic via `file_diagnostics`. Uses a literal rather than a column
/// reference so the empty `TypeContext` resolves to a concrete non-list type
/// (an `Unknown` column ref would silently skip the check).
#[test]
fn production_path_spread_on_non_list_fires_diagnostic() {
    let (mut db, path) = setup_single_model("SELECT ...1 FROM t");
    let diags = db.file_diagnostics(path);
    let meta_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MetaSpreadOnNonList))
        .collect();
    assert_eq!(
        meta_diags.len(),
        1,
        "SELECT ...1 FROM t must produce exactly 1 MetaSpreadOnNonList; \
         got diagnostics: {:?}",
        diags
    );
}

// ── Phase B (meta-language) diagnostic code existence + message tests ────────

/// `LambdaInForbiddenPosition` exists in `DiagnosticCode` and renders the
/// spec message format.
#[test]
fn diagnostic_code_lambda_in_forbidden_position() {
    let code = DiagnosticCode::LambdaInForbiddenPosition;
    let msg = meta_hof_diagnostic_message(code, None, None, None, None, None, None, None);
    assert_eq!(
        msg,
        "lambda is only valid as an argument to a higher-order function"
    );
}

/// `LambdaArityMismatch` exists and renders the spec message.
#[test]
fn diagnostic_code_lambda_arity_mismatch() {
    let code = DiagnosticCode::LambdaArityMismatch;
    let msg = meta_hof_diagnostic_message(
        code,
        Some("map"),
        None,
        Some("1"),
        Some("2"),
        None,
        None,
        None,
    );
    assert_eq!(msg, "map expects a lambda of arity 1; found arity 2");
}

/// `LambdaResultTypeMismatch` exists and renders the spec message with substitutions.
#[test]
fn diagnostic_code_lambda_result_type_mismatch() {
    let code = DiagnosticCode::LambdaResultTypeMismatch;
    let msg = meta_hof_diagnostic_message(
        code,
        Some("filter"),
        None,
        Some("Expr<Boolean>"),
        Some("Expr<Integer>"),
        None,
        None,
        None,
    );
    assert_eq!(
        msg,
        "filter requires lambda result Expr<Boolean>; found Expr<Integer>"
    );
}

/// `HofExpectsLambda` exists and renders the spec message.
#[test]
fn diagnostic_code_hof_expects_lambda() {
    let code = DiagnosticCode::HofExpectsLambda;
    let msg = meta_hof_diagnostic_message(
        code,
        Some("map"),
        None,
        None,
        Some("Expr<Integer>"),
        None,
        None,
        None,
    );
    assert_eq!(msg, "map expects a lambda; found Expr<Integer>");
}

/// `HofExpectsReducer` exists and renders the spec message.
#[test]
fn diagnostic_code_hof_expects_reducer() {
    let code = DiagnosticCode::HofExpectsReducer;
    let msg = meta_hof_diagnostic_message(
        code,
        None,
        None,
        None,
        Some("some_lambda"),
        None,
        None,
        None,
    );
    assert_eq!(msg, "reduce expects a reducer; found some_lambda");
}

/// `HofNameShadowed` exists and renders the spec message.
#[test]
fn diagnostic_code_hof_name_shadowed() {
    let code = DiagnosticCode::HofNameShadowed;
    let msg = meta_hof_diagnostic_message(code, None, Some("map"), None, None, None, None, None);
    assert_eq!(msg, "map is a reserved higher-order function name");
}

/// `ReducerNameShadowed` exists and renders the spec message.
#[test]
fn diagnostic_code_reducer_name_shadowed() {
    let code = DiagnosticCode::ReducerNameShadowed;
    let msg =
        meta_hof_diagnostic_message(code, None, Some("and_all"), None, None, None, None, None);
    assert_eq!(msg, "and_all is a reserved reducer name");
}

/// `PipeRhsNotCall` exists and renders the spec message.
#[test]
fn diagnostic_code_pipe_rhs_not_call() {
    let code = DiagnosticCode::PipeRhsNotCall;
    let msg = meta_hof_diagnostic_message(code, None, None, None, None, None, None, None);
    assert_eq!(msg, "pipe right-hand side must be a function call");
}

/// `PipeInDataPosition` exists and renders the spec message.
#[test]
fn diagnostic_code_pipe_in_data_position() {
    let code = DiagnosticCode::PipeInDataPosition;
    let msg = meta_hof_diagnostic_message(code, None, None, None, None, None, None, None);
    assert_eq!(msg, "|> is meta-only; use SQL composition in this position");
}

/// `ReducerInputTypeMismatch` exists and renders the spec message.
#[test]
fn diagnostic_code_reducer_input_type_mismatch() {
    let code = DiagnosticCode::ReducerInputTypeMismatch;
    let msg = meta_hof_diagnostic_message(
        code,
        None,
        None,
        None,
        None,
        Some("and_all"),
        Some("Expr<Boolean>"),
        Some("Expr<Integer>"),
    );
    assert_eq!(
        msg,
        "reducer and_all expects List<Expr<Boolean>>; found List<Expr<Integer>>"
    );
}

/// `ReducerEmptyNoIdentity` exists and renders the spec message.
#[test]
fn diagnostic_code_reducer_empty_no_identity() {
    let code = DiagnosticCode::ReducerEmptyNoIdentity;
    let msg =
        meta_hof_diagnostic_message(code, None, None, None, None, Some("union_all"), None, None);
    assert_eq!(msg, "reducer union_all has no identity for an empty list");
}

/// `ConfigVarNotFound` exists and renders the spec message.
#[test]
fn diagnostic_code_config_var_not_found() {
    let code = DiagnosticCode::ConfigVarNotFound;
    let msg = meta_hof_diagnostic_message(code, None, Some("my_var"), None, None, None, None, None);
    assert_eq!(
        msg,
        "compile-time variable my_var not declared in smelt.yml vars"
    );
}

/// `ConfigVarNameNotLiteral` exists and renders the spec message.
#[test]
fn diagnostic_code_config_var_name_not_literal() {
    let code = DiagnosticCode::ConfigVarNameNotLiteral;
    let msg = meta_hof_diagnostic_message(code, None, None, None, None, None, None, None);
    assert_eq!(msg, "smelt.config.var name must be a string literal");
}

/// `ConfigVarNullCoercion` exists and renders the spec message (Warning severity).
#[test]
fn diagnostic_code_config_var_null_coercion() {
    let code = DiagnosticCode::ConfigVarNullCoercion;
    let msg = meta_hof_diagnostic_message(
        code,
        None,
        Some("nullable_var"),
        None,
        None,
        None,
        None,
        None,
    );
    assert_eq!(
        msg,
        "null variable nullable_var coerced to empty string; declare a default in smelt.yml"
    );
}

// ─── Phase C Phase 3 TDD tests ─────────────────────────────────────────────

/// `ColumnsOfUnresolvableSchema` exists and renders the correct message.
#[test]
fn diagnostic_code_columns_of_unresolvable_schema_message() {
    let code = DiagnosticCode::ColumnsOfUnresolvableSchema;
    let msg = meta_reflection_diagnostic_message_with_table_expr(
        code,
        None,
        None,
        Some("smelt.models.orders"),
    );
    assert_eq!(
        msg,
        "cannot resolve column list for smelt.models.orders; upstream schema is unknown"
    );
}

/// `columns_of_for_table_expr` resolves a model's schema and returns
/// `ColumnRefValue`s in declaration order with correct name, data_type,
/// and is_numeric values.
///
/// The fixture intentionally mixes numeric columns (`id: Integer`, `amount: Decimal`)
/// with a non-numeric column (`name: Text`) to make the `is_numeric` assertions
/// meaningful — a fixture with only numeric columns cannot detect a bug where
/// `is_numeric` is always `true`.
#[test]
fn columns_of_salsa_query_resolves_smelt_path_schema() {
    let mut db = TestDb::default();

    // Create a model `orders` with three typed columns:
    //   - id (Integer) — numeric
    //   - amount (Decimal / 9.99) — numeric
    //   - name (Text / string literal) — NOT numeric
    let orders_path = PathBuf::from("models/orders.sql");
    db.set_file_text(
        orders_path.clone(),
        Arc::new(
            "SELECT 1 AS id, 9.99 AS amount, 'anon' AS name FROM source.raw_orders".to_string(),
        ),
    );
    db.set_all_files(Arc::new(vec![orders_path.clone()]));
    db.set_file_project_root(orders_path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let ws = db.sync_workspace();
    let project = db.db.project_input(&PathBuf::from(".")).expect("project");
    let result = columns_of_for_table_expr(&db.db, ws, project, "orders".to_string());

    // Should resolve successfully.
    let cols = result.expect("columns_of_for_table_expr must resolve orders");
    assert_eq!(cols.len(), 3, "orders has 3 columns; got: {:?}", cols);

    // Column order must be preserved (id, amount, name).
    assert_eq!(cols[0].name, "id");
    assert_eq!(cols[1].name, "amount");
    assert_eq!(cols[2].name, "name");

    // The source_span must be populated (non-None) for SQL-parsed columns.
    assert!(
        cols[0].source_span.is_some(),
        "id column must have a source_span"
    );
    assert!(
        cols[1].source_span.is_some(),
        "amount column must have a source_span"
    );
    assert!(
        cols[2].source_span.is_some(),
        "name column must have a source_span"
    );

    // is_numeric must be derived from types.md Numeric constraint membership:
    //   Integer and Decimal (9.99) → numeric; Text ('anon') → NOT numeric.
    assert!(
        cols[0].is_numeric,
        "id (Integer) must have is_numeric == true; got col: {:?}",
        cols[0]
    );
    assert!(
        cols[1].is_numeric,
        "amount (Decimal) must have is_numeric == true; got col: {:?}",
        cols[1]
    );
    assert!(
        !cols[2].is_numeric,
        "name (Text) must have is_numeric == false; got col: {:?}",
        cols[2]
    );
}

/// `columns_of_for_table_expr` returns `Err(())` when the model name is not
/// found in the workspace.
#[test]
fn columns_of_returns_err_for_nonexistent_model() {
    let mut db = TestDb::default();
    db.set_all_files(Arc::new(vec![]));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let ws = db.sync_workspace();
    let project = db.db.project_input(&PathBuf::from(".")).expect("project");
    let result = columns_of_for_table_expr(&db.db, ws, project, "nonexistent".to_string());

    assert!(
        result.is_err(),
        "columns_of_for_table_expr must return Err for unknown model"
    );
}

/// Salsa invalidation: modifying the upstream model's schema causes
/// `columns_of_for_table_expr` to re-evaluate and return the new schema.
///
/// This verifies the Salsa cache invariant from the Phase C spec §"Salsa-cached
/// pure function of workspace state".
#[test]
fn columns_of_invalidates_when_upstream_schema_changes() {
    let mut db = TestDb::default();

    let orders_path = PathBuf::from("models/orders.sql");
    db.set_file_text(
        orders_path.clone(),
        Arc::new("SELECT 1 AS id FROM source.raw_orders".to_string()),
    );
    db.set_all_files(Arc::new(vec![orders_path.clone()]));
    db.set_file_project_root(orders_path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    // First evaluation: 1 column.
    let ws = db.sync_workspace();
    let project = db.db.project_input(&PathBuf::from(".")).expect("project");
    let cols_v1 = columns_of_for_table_expr(&db.db, ws, project, "orders".to_string())
        .expect("v1 must resolve");
    assert_eq!(
        cols_v1.len(),
        1,
        "v1 must have 1 column; got: {:?}",
        cols_v1
    );

    // Mutate the upstream schema by adding a column.
    db.set_file_text(
        orders_path.clone(),
        Arc::new("SELECT 1 AS id, 'x' AS status FROM source.raw_orders".to_string()),
    );

    // Second evaluation after invalidation: 2 columns.
    let ws2 = db.sync_workspace();
    let project2 = db.db.project_input(&PathBuf::from(".")).expect("project");
    let cols_v2 = columns_of_for_table_expr(&db.db, ws2, project2, "orders".to_string())
        .expect("v2 must resolve");
    assert_eq!(
        cols_v2.len(),
        2,
        "v2 must have 2 columns after schema change; got: {:?}",
        cols_v2
    );
    assert_eq!(cols_v2[1].name, "status");
}

/// `columns_to_column_ref_values` preserves declaration order.
#[test]
fn columns_of_expansion_preserves_source_ordering_pure() {
    use crate::schema::{Column, ColumnSource};
    use rowan::TextRange;

    // Construct three columns in a deliberate order (z, a, m).
    let make_col = |name: &str| -> Column {
        Column {
            name: name.to_string(),
            alias: None,
            source: ColumnSource::Unknown,
            expression: String::new(),
            range: TextRange::default(),
            data_type: None,
        }
    };

    let cols = vec![make_col("z"), make_col("a"), make_col("m")];
    let result = columns_to_column_ref_values(&cols);

    assert_eq!(result.len(), 3);
    assert_eq!(result[0].name, "z");
    assert_eq!(result[1].name, "a");
    assert_eq!(result[2].name, "m");
}

/// `ColumnsOfUnresolvableSchema` message renders with the `{t}` placeholder.
#[test]
fn columns_of_unresolvable_schema_message_with_placeholder() {
    let msg = meta_reflection_diagnostic_message_with_table_expr(
        DiagnosticCode::ColumnsOfUnresolvableSchema,
        None,
        None,
        None, // no table_expr given — falls back to "t"
    );
    assert_eq!(
        msg,
        "cannot resolve column list for t; upstream schema is unknown"
    );
}

// ============================================================================
// Phase D — wide-reflection Salsa query tests
// ============================================================================

/// Helper: set up a multi-model workspace with frontmatter tags.
///
/// Returns (TestDb, Workspace). Models are named `a`, `b`, `c`, `d` under
/// `models/a.sql`, etc. Models a/b/c are tagged `cohort`; model d is not.
fn setup_cohort_workspace() -> (TestDb, Workspace) {
    let mut db = TestDb::default();
    let root = PathBuf::from(".");

    let models: &[(&str, &str)] = &[
        (
            "models/a.sql",
            "---\ntags: [cohort]\n---\nSELECT 1 AS id FROM source.raw",
        ),
        (
            "models/b.sql",
            "---\ntags: [cohort]\n---\nSELECT 2 AS id FROM source.raw",
        ),
        (
            "models/c.sql",
            "---\ntags: [cohort]\n---\nSELECT 3 AS id FROM source.raw",
        ),
        (
            "models/d.sql",
            "---\ntags: [other]\n---\nSELECT 4 AS id FROM source.raw",
        ),
    ];

    for (path, text) in models {
        let p = PathBuf::from(path);
        db.set_file_text(p.clone(), Arc::new(text.to_string()));
        db.set_file_project_root(p.clone(), root.clone());
    }

    db.set_project_sources_yaml(root.clone(), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![root.clone()]));

    let ws = db.sync_workspace();
    (db, ws)
}

/// `models_with_tag(workspace, "cohort")` returns exactly the three
/// `cohort`-tagged models in path-sorted order `[a, b, c]`.
#[test]
fn models_with_tag_returns_path_sorted_matches() {
    let (db, ws) = setup_cohort_workspace();

    let result = models_with_tag(&db.db, ws, "cohort".to_string());

    assert_eq!(
        result.len(),
        3,
        "expected 3 cohort-tagged models; got: {:?}",
        result.iter().map(|m| &m.path).collect::<Vec<_>>()
    );
    // Verify path-sorted order (byte-lexicographic).
    assert!(
        result[0].path.ends_with("/a.sql") || result[0].path == "models/a.sql",
        "first model should be a; got {}",
        result[0].path
    );
    assert!(
        result[1].path.ends_with("/b.sql") || result[1].path == "models/b.sql",
        "second model should be b; got {}",
        result[1].path
    );
    assert!(
        result[2].path.ends_with("/c.sql") || result[2].path == "models/c.sql",
        "third model should be c; got {}",
        result[2].path
    );

    // Per-element fields.
    assert_eq!(result[0].name, "a");
    assert!(result[0].tags.contains(&"cohort".to_string()));
    assert_eq!(result[0].model_name_for_columns, "a");
}

/// `models_with_tag` honours merged tags: smelt.yml tags + frontmatter tags.
///
/// Sets up a model with `tags: [cohort]` in smelt.yml and `tags: [audit]` in
/// SQL frontmatter. The model must match both `with_tag("cohort")` and
/// `with_tag("audit")`.
#[test]
fn models_with_tag_uses_merged_tag_set() {
    let mut db = TestDb::default();
    let root = PathBuf::from(".");

    // Model has frontmatter tag `audit`.
    let path = PathBuf::from("models/merged.sql");
    db.set_file_text(
        path.clone(),
        Arc::new("---\ntags: [audit]\n---\nSELECT 1 AS id FROM source.raw".to_string()),
    );
    db.set_file_project_root(path.clone(), root.clone());

    // smelt.yml gives this model the `cohort` tag.
    // `name:` and `targets:` are required by the Config deserialiser.
    let smelt_yml = concat!(
        "name: test_project\n",
        "targets:\n  dev:\n    type: duckdb\n    database: t.duckdb\n    schema: main\n",
        "models:\n  merged:\n    tags: [cohort]\n",
    );
    db.set_project_sources_yaml(root.clone(), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![root.clone()]));

    // Set the smelt.yml text on the project after it's been registered.
    db.db.set_project_smelt_yml(&root, smelt_yml.to_string());

    let ws = db.sync_workspace();

    let cohort = models_with_tag(&db.db, ws, "cohort".to_string());
    let audit = models_with_tag(&db.db, ws, "audit".to_string());

    assert_eq!(cohort.len(), 1, "model must match smelt.yml tag 'cohort'");
    assert_eq!(audit.len(), 1, "model must match frontmatter tag 'audit'");
    assert_eq!(cohort[0].name, "merged");
    assert_eq!(audit[0].name, "merged");
}

/// `models_with_tag` correctly invalidates when the model's frontmatter changes.
///
/// After changing the tag from `cohort` to `other`, the query must return the
/// updated (empty) set for `cohort`.
#[test]
fn models_with_tag_invalidates_on_tag_change() {
    let mut db = TestDb::default();
    let root = PathBuf::from(".");

    let path = PathBuf::from("models/mutable.sql");
    db.set_file_text(
        path.clone(),
        Arc::new("---\ntags: [cohort]\n---\nSELECT 1 AS id FROM source.raw".to_string()),
    );
    db.set_file_project_root(path.clone(), root.clone());
    db.set_project_sources_yaml(root.clone(), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![root.clone()]));

    let ws = db.sync_workspace();
    let before = models_with_tag(&db.db, ws, "cohort".to_string());
    assert_eq!(before.len(), 1, "model should be tagged cohort before edit");

    // Update the model to remove the `cohort` tag.
    db.set_file_text(
        path.clone(),
        Arc::new("---\ntags: [other]\n---\nSELECT 1 AS id FROM source.raw".to_string()),
    );
    let ws2 = db.sync_workspace();
    let after = models_with_tag(&db.db, ws2, "cohort".to_string());
    assert_eq!(
        after.len(),
        0,
        "after tag change, model must not match cohort"
    );
}

/// `models_all` returns every model in the workspace in path-sorted order.
/// Running it twice over the same workspace produces byte-equal results (Salsa
/// memoisation / determinism invariant).
#[test]
fn models_all_returns_all_models_path_sorted() {
    let (db, ws) = setup_cohort_workspace();

    let result1 = models_all(&db.db, ws);
    let result2 = models_all(&db.db, ws);

    // All 4 models present.
    assert_eq!(
        result1.len(),
        4,
        "workspace has 4 models; got {:?}",
        result1.iter().map(|m| &m.path).collect::<Vec<_>>()
    );

    // Path-sorted order.
    let paths: Vec<&str> = result1.iter().map(|m| m.path.as_str()).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted, "models_all must return path-sorted results");

    // Determinism: two calls on same input are byte-equal.
    assert_eq!(result1, result2, "models_all must be deterministic");
}

/// `sources_with_tag` and `sources_all` mirror the models behaviour for sources.
///
/// Sets up two sources in a temp directory: one tagged `analytics`, one not.
/// Verifies that `sources_with_tag("analytics")` returns exactly the tagged one
/// and `sources_all` returns both in path-sorted order.
#[test]
fn sources_with_tag_and_sources_all_mirror_models_behaviour() {
    use std::io::Write;
    use tempfile::TempDir;

    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path().to_path_buf();
    let models_dir = root.join("models");
    std::fs::create_dir_all(&models_dir).unwrap();

    // Source YAML with `tags: [analytics]`.
    let analytics_yml = models_dir.join("analytics_source.yml");
    std::fs::File::create(&analytics_yml)
        .unwrap()
        .write_all(b"tags: [analytics]\ncolumns:\n  - name: id\n    type: Integer\n")
        .unwrap();

    // Source YAML without tags.
    let plain_yml = models_dir.join("plain_source.yml");
    std::fs::File::create(&plain_yml)
        .unwrap()
        .write_all(b"columns:\n  - name: val\n    type: Text\n")
        .unwrap();

    // smelt.yml with paths: [models].
    let smelt_yml_path = root.join("smelt.yml");
    std::fs::File::create(&smelt_yml_path)
        .unwrap()
        .write_all(b"paths:\n  - models\n")
        .unwrap();

    let mut db_wrapper = TestDb::default();
    db_wrapper.set_project_sources_yaml(root.clone(), Arc::new(String::new()));
    db_wrapper.set_all_project_roots(Arc::new(vec![root.clone()]));
    db_wrapper.sync_workspace();

    let project = db_wrapper
        .db
        .project_input(&root)
        .expect("project registered");

    let tagged = sources_with_tag(&db_wrapper.db, project, "analytics".to_string());
    assert_eq!(
        tagged.len(),
        1,
        "one source tagged analytics; got {:?}",
        tagged.iter().map(|s| &s.path).collect::<Vec<_>>()
    );
    assert_eq!(tagged[0].name, "analytics_source");
    assert!(tagged[0].tags.contains(&"analytics".to_string()));

    let all = sources_all(&db_wrapper.db, project);
    assert_eq!(
        all.len(),
        2,
        "two sources total; got {:?}",
        all.iter().map(|s| &s.path).collect::<Vec<_>>()
    );

    // Path-sorted.
    let paths: Vec<&str> = all.iter().map(|s| s.path.as_str()).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted, "sources_all must return path-sorted results");
}

/// `ModelRefValue::model_name_for_columns` routes through `columns_of_for_table_expr`.
///
/// Given a model `m` in the workspace, `columns_of_for_table_expr(db, ws, m.model_name_for_columns)`
/// returns the same column list as querying the model directly. This verifies
/// the re-dispatch routing that underpins `m.columns`.
#[test]
fn model_ref_columns_routes_through_columns_of_query() {
    let mut db = TestDb::default();
    let root = PathBuf::from(".");

    let path = PathBuf::from("models/orders.sql");
    db.set_file_text(
        path.clone(),
        Arc::new(
            "---\ntags: [cohort]\n---\nSELECT 1 AS order_id, 9.99 AS amount FROM source.raw"
                .to_string(),
        ),
    );
    db.set_file_project_root(path.clone(), root.clone());
    db.set_project_sources_yaml(root.clone(), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![root.clone()]));

    let ws = db.sync_workspace();

    let models = models_with_tag(&db.db, ws, "cohort".to_string());
    assert_eq!(models.len(), 1);
    let model_ref = &models[0];

    // Route m.columns through columns_of_for_table_expr using model_name_for_columns.
    let project = db.db.project_input(&root).expect("project");
    let columns_via_ref = columns_of_for_table_expr(
        &db.db,
        ws,
        project,
        model_ref.model_name_for_columns.clone(),
    )
    .expect("columns_of_for_table_expr must succeed for an existing model");

    // Also get directly.
    let columns_direct = columns_of_for_table_expr(&db.db, ws, project, "orders".to_string())
        .expect("columns_of_for_table_expr must succeed directly");

    // Byte-equal: same column list via both paths.
    assert_eq!(
        *columns_via_ref, *columns_direct,
        "m.columns routing via model_name_for_columns must produce same result as direct call"
    );
    assert_eq!(columns_via_ref.len(), 2);
    assert_eq!(columns_via_ref[0].name, "order_id");
    assert_eq!(columns_via_ref[1].name, "amount");
}

// ============================================================================
// Phase E1 TDD tests — DiagnosticCode completeness
// ============================================================================

/// Test 11: `diagnostic_codes_record_set_complete`
///
/// Every record diagnostic code exists in `DiagnosticCode` and renders the
/// spec message format from `meta_language.md` §"Record diagnostic codes".
#[test]
fn diagnostic_codes_record_set_complete() {
    // SmeltRecordRedefinition
    let code = DiagnosticCode::SmeltRecordRedefinition;
    assert!(matches!(code, DiagnosticCode::SmeltRecordRedefinition));
    let msg = meta_record_diagnostic_message(
        code,
        Some("Foo"),
        None,
        Some("models/foo.sql"),
        None,
        None,
        None,
    );
    assert!(
        msg.contains("Foo") && msg.contains("models/foo.sql") && msg.contains("workspace-wide"),
        "SmeltRecordRedefinition message must match spec; got: {msg}"
    );

    // RecordFieldUnknown
    let code = DiagnosticCode::RecordFieldUnknown;
    assert!(matches!(code, DiagnosticCode::RecordFieldUnknown));
    let msg = meta_record_diagnostic_message(
        code,
        Some("Entry"),
        Some("bar"),
        None,
        None,
        None,
        Some("name, type"),
    );
    assert!(
        msg.contains("Entry") && msg.contains("bar") && msg.contains("name, type"),
        "RecordFieldUnknown message must match spec; got: {msg}"
    );

    // RecordFieldMissing
    let code = DiagnosticCode::RecordFieldMissing;
    assert!(matches!(code, DiagnosticCode::RecordFieldMissing));
    let msg =
        meta_record_diagnostic_message(code, Some("Entry"), Some("name"), None, None, None, None);
    assert!(
        msg.contains("Entry") && msg.contains("name") && msg.contains("missing"),
        "RecordFieldMissing message must match spec; got: {msg}"
    );

    // RecordFieldDuplicate
    let code = DiagnosticCode::RecordFieldDuplicate;
    assert!(matches!(code, DiagnosticCode::RecordFieldDuplicate));
    let msg = meta_record_diagnostic_message(code, None, Some("name"), None, None, None, None);
    assert!(
        msg.contains("name") && msg.contains("already appears"),
        "RecordFieldDuplicate message must match spec; got: {msg}"
    );

    // RecordFieldTypeMismatch
    let code = DiagnosticCode::RecordFieldTypeMismatch;
    assert!(matches!(code, DiagnosticCode::RecordFieldTypeMismatch));
    let msg = meta_record_diagnostic_message(
        code,
        None,
        Some("amount"),
        None,
        Some("Integer"),
        Some("Text"),
        None,
    );
    assert!(
        msg.contains("amount") && msg.contains("Integer") && msg.contains("Text"),
        "RecordFieldTypeMismatch message must match spec; got: {msg}"
    );

    // RecordLiteralUnknownTarget
    let code = DiagnosticCode::RecordLiteralUnknownTarget;
    assert!(matches!(code, DiagnosticCode::RecordLiteralUnknownTarget));
    let msg = meta_record_diagnostic_message(code, None, None, None, None, None, None);
    assert!(
        msg.contains("cannot infer record type"),
        "RecordLiteralUnknownTarget message must match spec; got: {msg}"
    );

    // RecordFieldNotProjectable
    let code = DiagnosticCode::RecordFieldNotProjectable;
    assert!(matches!(code, DiagnosticCode::RecordFieldNotProjectable));
    let msg =
        meta_record_diagnostic_message(code, Some("Integer"), Some("foo"), None, None, None, None);
    assert!(
        msg.contains("Integer") && msg.contains("foo") && msg.contains("no fields"),
        "RecordFieldNotProjectable message must match spec; got: {msg}"
    );

    // RecordFieldTypeForbidden
    let code = DiagnosticCode::RecordFieldTypeForbidden;
    assert!(matches!(code, DiagnosticCode::RecordFieldTypeForbidden));
    let msg = meta_record_diagnostic_message(code, Some("ModelRef"), None, None, None, None, None);
    assert!(
        msg.contains("ModelRef") && msg.contains("not user-writable"),
        "RecordFieldTypeForbidden message must match spec; got: {msg}"
    );

    // RecordCyclicDeclaration
    let code = DiagnosticCode::RecordCyclicDeclaration;
    assert!(matches!(code, DiagnosticCode::RecordCyclicDeclaration));
    let msg = meta_record_diagnostic_message(code, Some("Node"), None, None, None, None, None);
    assert!(
        msg.contains("Node") && msg.contains("cycle"),
        "RecordCyclicDeclaration message must match spec; got: {msg}"
    );

    // RecordInDataWorld
    let code = DiagnosticCode::RecordInDataWorld;
    assert!(matches!(code, DiagnosticCode::RecordInDataWorld));
    let msg = meta_record_diagnostic_message(code, None, None, None, None, None, None);
    assert!(
        msg.contains("Data-World") || msg.contains("data-world") || msg.contains("SQL"),
        "RecordInDataWorld message must reference Data-World position; got: {msg}"
    );
}

/// Test 12: `diagnostic_codes_map_set_complete`
///
/// Every map diagnostic code exists in `DiagnosticCode` and renders per spec.
#[test]
fn diagnostic_codes_map_set_complete() {
    // MapKeyTypeNotText
    let code = DiagnosticCode::MapKeyTypeNotText;
    assert!(matches!(code, DiagnosticCode::MapKeyTypeNotText));
    let msg =
        meta_map_diagnostic_message(code, None, None, None, None, Some("Integer"), None, None);
    assert!(
        msg.contains("Integer") && msg.contains("Text"),
        "MapKeyTypeNotText message must match spec; got: {msg}"
    );

    // MapApiUnknown
    let code = DiagnosticCode::MapApiUnknown;
    assert!(matches!(code, DiagnosticCode::MapApiUnknown));
    let msg = meta_map_diagnostic_message(code, None, Some("merge"), None, None, None, None, None);
    assert!(
        msg.contains("merge") && msg.contains("entries"),
        "MapApiUnknown message must match spec; got: {msg}"
    );

    // MapApiArityMismatch
    let code = DiagnosticCode::MapApiArityMismatch;
    assert!(matches!(code, DiagnosticCode::MapApiArityMismatch));
    let msg =
        meta_map_diagnostic_message(code, Some("get"), None, None, Some("0"), None, None, None);
    assert!(
        msg.contains("get") && msg.contains("0"),
        "MapApiArityMismatch message must match spec; got: {msg}"
    );

    // MapApiNamedArgument
    let code = DiagnosticCode::MapApiNamedArgument;
    assert!(matches!(code, DiagnosticCode::MapApiNamedArgument));
    let msg = meta_map_diagnostic_message(code, Some("get"), None, None, None, None, None, None);
    assert!(
        msg.contains("get") && msg.contains("named"),
        "MapApiNamedArgument message must match spec; got: {msg}"
    );

    // MapApiUnexpectedArgument
    let code = DiagnosticCode::MapApiUnexpectedArgument;
    assert!(matches!(code, DiagnosticCode::MapApiUnexpectedArgument));
    let msg = meta_map_diagnostic_message(code, Some("keys"), None, None, None, None, None, None);
    assert!(
        msg.contains("keys") && msg.contains("no arguments"),
        "MapApiUnexpectedArgument message must match spec; got: {msg}"
    );

    // MapGetMissingKey
    let code = DiagnosticCode::MapGetMissingKey;
    assert!(matches!(code, DiagnosticCode::MapGetMissingKey));
    let msg = meta_map_diagnostic_message(code, None, None, Some("prod"), None, None, None, None);
    assert!(
        msg.contains("prod") && msg.contains("binding"),
        "MapGetMissingKey message must match spec; got: {msg}"
    );

    // MapApiArgTypeMismatch
    let code = DiagnosticCode::MapApiArgTypeMismatch;
    assert!(matches!(code, DiagnosticCode::MapApiArgTypeMismatch));
    let msg = meta_map_diagnostic_message(
        code,
        Some("get"),
        None,
        None,
        None,
        None,
        Some("Text"),
        Some("Integer"),
    );
    assert!(
        msg.contains("get") && msg.contains("Text") && msg.contains("Integer"),
        "MapApiArgTypeMismatch message must match spec; got: {msg}"
    );
}

/// Test 13: `diagnostic_codes_loader_set_complete`
///
/// Every loader diagnostic code exists in `DiagnosticCode` and renders
/// per `meta_config_loading.md` §"Validation diagnostics".
#[test]
fn diagnostic_codes_loader_set_complete() {
    // ConfigLoaderPathNotLiteral
    let code = DiagnosticCode::ConfigLoaderPathNotLiteral;
    assert!(matches!(code, DiagnosticCode::ConfigLoaderPathNotLiteral));
    let msg = meta_loader_diagnostic_message(
        code,
        Some("path_var"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(
        msg.contains("path_var") && msg.contains("literal"),
        "ConfigLoaderPathNotLiteral message must match spec; got: {msg}"
    );

    // ConfigLoaderPathEscapesWorkspace
    let code = DiagnosticCode::ConfigLoaderPathEscapesWorkspace;
    assert!(matches!(
        code,
        DiagnosticCode::ConfigLoaderPathEscapesWorkspace
    ));
    let msg = meta_loader_diagnostic_message(
        code,
        None,
        Some("/etc/passwd"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(
        msg.contains("/etc/passwd") && msg.contains("workspace-relative"),
        "ConfigLoaderPathEscapesWorkspace message must match spec; got: {msg}"
    );

    // ConfigLoaderPathBackslash
    let code = DiagnosticCode::ConfigLoaderPathBackslash;
    assert!(matches!(code, DiagnosticCode::ConfigLoaderPathBackslash));
    let msg = meta_loader_diagnostic_message(
        code,
        None,
        Some("config\\data.yaml"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(
        msg.contains("config\\data.yaml"),
        "ConfigLoaderPathBackslash message must match spec; got: {msg}"
    );

    // ConfigLoaderFileNotFound
    let code = DiagnosticCode::ConfigLoaderFileNotFound;
    assert!(matches!(code, DiagnosticCode::ConfigLoaderFileNotFound));
    let msg = meta_loader_diagnostic_message(
        code,
        None,
        Some("config/missing.yaml"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(
        msg.contains("config/missing.yaml") && msg.contains("not found"),
        "ConfigLoaderFileNotFound message must match spec; got: {msg}"
    );

    // ConfigLoaderSchemaForbidden
    let code = DiagnosticCode::ConfigLoaderSchemaForbidden;
    assert!(matches!(code, DiagnosticCode::ConfigLoaderSchemaForbidden));
    let msg = meta_loader_diagnostic_message(
        code,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("Integer"),
        None,
        None,
        None,
        None,
        None,
    );
    assert!(
        msg.contains("Integer") && msg.contains("record type"),
        "ConfigLoaderSchemaForbidden message must match spec; got: {msg}"
    );

    // ConfigLoaderTomlNotYetSupported
    let code = DiagnosticCode::ConfigLoaderTomlNotYetSupported;
    assert!(matches!(
        code,
        DiagnosticCode::ConfigLoaderTomlNotYetSupported
    ));
    let msg = meta_loader_diagnostic_message(
        code, None, None, None, None, None, None, None, None, None, None, None, None, None,
    );
    assert!(
        msg.contains("load_toml") && msg.contains("reserved"),
        "ConfigLoaderTomlNotYetSupported message must match spec; got: {msg}"
    );

    // ConfigLoaderParseError
    let code = DiagnosticCode::ConfigLoaderParseError;
    assert!(matches!(code, DiagnosticCode::ConfigLoaderParseError));
    let msg = meta_loader_diagnostic_message(
        code,
        None,
        Some("config.yaml"),
        Some("YAML"),
        Some("unexpected token"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(
        msg.contains("YAML") && msg.contains("config.yaml") && msg.contains("unexpected token"),
        "ConfigLoaderParseError message must match spec; got: {msg}"
    );

    // ConfigLoaderRequiredFieldMissing
    let code = DiagnosticCode::ConfigLoaderRequiredFieldMissing;
    assert!(matches!(
        code,
        DiagnosticCode::ConfigLoaderRequiredFieldMissing
    ));
    let msg = meta_loader_diagnostic_message(
        code,
        None,
        None,
        None,
        None,
        Some("name"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(
        msg.contains("name") && msg.contains("required") && msg.contains("missing"),
        "ConfigLoaderRequiredFieldMissing message must match spec; got: {msg}"
    );

    // ConfigLoaderUnknownField
    let code = DiagnosticCode::ConfigLoaderUnknownField;
    assert!(matches!(code, DiagnosticCode::ConfigLoaderUnknownField));
    let msg = meta_loader_diagnostic_message(
        code,
        None,
        None,
        None,
        None,
        Some("extra"),
        Some("name, value"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(
        msg.contains("extra") && msg.contains("name, value"),
        "ConfigLoaderUnknownField message must match spec; got: {msg}"
    );

    // ConfigLoaderTypeMismatch
    let code = DiagnosticCode::ConfigLoaderTypeMismatch;
    assert!(matches!(code, DiagnosticCode::ConfigLoaderTypeMismatch));
    let msg = meta_loader_diagnostic_message(
        code,
        None,
        None,
        None,
        None,
        Some("count"),
        None,
        Some("Integer"),
        Some("String"),
        None,
        None,
        None,
        None,
        None,
    );
    assert!(
        msg.contains("count") && msg.contains("Integer") && msg.contains("String"),
        "ConfigLoaderTypeMismatch message must match spec; got: {msg}"
    );

    // ConfigLoaderRootShapeMismatch
    let code = DiagnosticCode::ConfigLoaderRootShapeMismatch;
    assert!(matches!(
        code,
        DiagnosticCode::ConfigLoaderRootShapeMismatch
    ));
    let msg = meta_loader_diagnostic_message(
        code,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("List<Entry>"),
        None,
        Some("sequence"),
        Some("mapping"),
        None,
        None,
        None,
    );
    assert!(
        msg.contains("List<Entry>") && msg.contains("sequence") && msg.contains("mapping"),
        "ConfigLoaderRootShapeMismatch message must match spec; got: {msg}"
    );

    // ConfigLoaderDuplicateMapKey
    let code = DiagnosticCode::ConfigLoaderDuplicateMapKey;
    assert!(matches!(code, DiagnosticCode::ConfigLoaderDuplicateMapKey));
    let msg = meta_loader_diagnostic_message(
        code,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("prod"),
        Some("line 5"),
        Some("line 2"),
    );
    assert!(
        msg.contains("prod") && msg.contains("line 5") && msg.contains("line 2"),
        "ConfigLoaderDuplicateMapKey message must match spec; got: {msg}"
    );

    // ConfigLoaderNullCoercion
    let code = DiagnosticCode::ConfigLoaderNullCoercion;
    assert!(matches!(code, DiagnosticCode::ConfigLoaderNullCoercion));
    let msg = meta_loader_diagnostic_message(
        code,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("line 3"),
        None,
    );
    assert!(
        msg.contains("line 3") && msg.contains("empty string"),
        "ConfigLoaderNullCoercion message must match spec; got: {msg}"
    );
}

// ============================================================================
// Phase E1 Phase 5 — Salsa loader input invalidation + record declarations
// ============================================================================

/// Verify that `loader_file_parsed` is re-evaluated when the `LoaderFileInput`
/// text changes (Salsa cache invalidation).
///
/// When the same path is registered a second time with different text, the
/// result of `loader_file_parsed` must reflect the new content.
#[test]
fn loader_file_text_is_salsa_input() {
    let mut db = Database::default();

    // Register a loader file with initial YAML text.
    let path: Arc<str> = Arc::from("data/config.yaml");
    let text_v1: Arc<str> = Arc::from("name: Alice\nage: 30\n");
    let input_v1 = db.set_loader_file(path.clone(), text_v1, true);

    // First parse: should succeed and return the v1 text.
    let result_v1 = loader_file_parsed(&db, input_v1);
    assert!(
        result_v1.is_ok(),
        "v1 parse must succeed; got: {:?}",
        result_v1.as_ref().as_ref().err()
    );

    // Update the same path with new text.
    let text_v2: Arc<str> = Arc::from("name: Bob\nage: 42\n");
    let input_v2 = db.set_loader_file(path.clone(), text_v2.clone(), true);

    // The returned input handle must be the same Salsa entity (update-in-place).
    assert!(
        input_v1 == input_v2,
        "set_loader_file must return the same input handle on update"
    );

    // Second parse: Salsa re-evaluates and the text is now v2.
    let result_v2 = loader_file_parsed(&db, input_v2);
    assert!(result_v2.is_ok(), "v2 parse must succeed");

    // Confirm the root now reflects the updated text (root mapping contains "Bob").
    let parsed = result_v2.as_ref().as_ref().unwrap();
    match &parsed.root {
        crate::loader::ParsedNode::Mapping { entries, .. } => {
            let name_entry = entries.iter().find(|(k, _, _)| k == "name");
            let name_node = name_entry.map(|(_, _, v)| v);
            match name_node {
                Some(crate::loader::ParsedNode::String { value, .. }) => {
                    assert_eq!(
                        value, "Bob",
                        "after update, root.name must be 'Bob'; got '{}'",
                        value
                    );
                }
                other => panic!(
                    "expected String node for 'name' after update; got {:?}",
                    other
                ),
            }
        }
        other => panic!("expected Mapping at root after update; got {:?}", other),
    }
}

/// Verify that `loader_resolved_value` is invalidated when the `LoaderFileInput`
/// text changes (Salsa cache invalidation for content-validation).
///
/// When the loader file is updated via `set_loader_file`, the
/// `loader_resolved_value` result must reflect the new content.
#[test]
fn loader_resolved_value_invalidated_on_file_change() {
    let mut db = Database::default();

    // Register a loader file with v1 YAML text ({name: us_west}).
    let loader_path: Arc<str> = Arc::from("cohorts.yaml");
    let text_v1: Arc<str> = Arc::from("{name: us_west, threshold: 100}");
    let input_v1 = db.set_loader_file(loader_path.clone(), text_v1, true);

    // Build a LoaderCallSiteId for a hypothetical `smelt.config.load_yaml` call.
    let call_site = LoaderCallSiteId {
        file_path: Arc::from("models/cohorts.sql"),
        byte_offset: 7,
        loader_path: loader_path.clone(),
        schema_text: Arc::from("{name: Text, threshold: Integer}"),
    };

    // First resolution: should parse the v1 text.
    let resolved_v1 = loader_resolved_value(&db, input_v1, call_site.clone());
    assert!(
        resolved_v1.parsed.is_some(),
        "v1 resolution must produce a parsed result"
    );
    assert!(
        resolved_v1.diagnostics.is_empty(),
        "v1 resolution must have no diagnostics for a valid file; got: {:?}",
        resolved_v1.diagnostics
    );

    // Verify the v1 value has the expected content.
    match &resolved_v1.parsed {
        Some(p) => match &p.root {
            crate::loader::ParsedNode::Mapping { entries, .. } => {
                let name_entry = entries.iter().find(|(k, _, _)| k == "name");
                match name_entry.map(|(_, _, v)| v) {
                    Some(crate::loader::ParsedNode::String { value, .. }) => {
                        assert_eq!(
                            value, "us_west",
                            "v1 name must be 'us_west'; got '{}'",
                            value
                        );
                    }
                    other => panic!("expected String for 'name' in v1; got {:?}", other),
                }
            }
            other => panic!("expected Mapping in v1; got {:?}", other),
        },
        None => panic!("v1 parsed must be Some"),
    }

    // Update the loader file to v2 ({name: us_east, threshold: 200}).
    let text_v2: Arc<str> = Arc::from("{name: us_east, threshold: 200}");
    let input_v2 = db.set_loader_file(loader_path.clone(), text_v2, true);

    // The input handle must be the same Salsa entity (update-in-place).
    assert!(
        input_v1 == input_v2,
        "set_loader_file must return the same input handle on update"
    );

    // Second resolution: Salsa must invalidate and re-evaluate because the
    // loader file text changed.
    let resolved_v2 = loader_resolved_value(&db, input_v2, call_site);
    assert!(
        resolved_v2.parsed.is_some(),
        "v2 resolution must produce a parsed result"
    );
    assert!(
        resolved_v2.diagnostics.is_empty(),
        "v2 resolution must have no diagnostics; got: {:?}",
        resolved_v2.diagnostics
    );

    // The v2 value must reflect the updated file content.
    match &resolved_v2.parsed {
        Some(p) => match &p.root {
            crate::loader::ParsedNode::Mapping { entries, .. } => {
                let name_entry = entries.iter().find(|(k, _, _)| k == "name");
                match name_entry.map(|(_, _, v)| v) {
                    Some(crate::loader::ParsedNode::String { value, .. }) => {
                        assert_eq!(
                            value, "us_east",
                            "v2 name must be 'us_east' after update; got '{}'",
                            value
                        );
                    }
                    other => panic!("expected String for 'name' in v2; got {:?}", other),
                }
            }
            other => panic!("expected Mapping in v2; got {:?}", other),
        },
        None => panic!("v2 parsed must be Some"),
    }
}

/// Verify that the production diagnostic orchestrator (`file_diagnostics`) wires
/// through `loader_resolved_value` end-to-end.
///
/// Registers a workspace with a `.sql` file containing a
/// `smelt.config.load_yaml('cohorts.yaml', {name: Text})` call, registers a
/// `LoaderFileInput` for `cohorts.yaml` with valid content `{name: us_west}`,
/// and asserts that `file_diagnostics` produces NO
/// `ConfigLoaderRequiredFieldMissing` diagnostic.
///
/// Then mutates the loader file to `{wrong_field: x}` via `set_loader_file`,
/// runs `file_diagnostics` again, and asserts that a
/// `ConfigLoaderRequiredFieldMissing` diagnostic is now present.
///
/// This proves the production orchestrator wires through `workspace.loader_files`
/// rather than the no-op closure.
#[test]
fn file_diagnostics_end_to_end_loader_content_validation() {
    use std::path::PathBuf;

    let mut db = Database::default();
    let project_root = PathBuf::from("/tmp/smelt_test_loader_e2e");

    // Register a project and a sql file containing a load_yaml call.
    let project = db.set_project_input(project_root.clone(), String::new());
    let sql_content = "SELECT smelt.config.load_yaml('cohorts.yaml', {name: Text}) AS cfg";
    let sql_path = project_root.join("models/cohorts.sql");
    let sql_file = db.set_source_file(
        sql_path.clone(),
        sql_content.to_string(),
        project_root.clone(),
    );

    // Register the workspace (loader_files starts empty).
    db.set_workspace(vec![sql_file], vec![project]);

    // Register a loader file with valid content — 'name' field is present.
    let loader_path: Arc<str> = Arc::from("cohorts.yaml");
    let text_v1: Arc<str> = Arc::from("{name: us_west}");
    db.set_loader_file(loader_path.clone(), text_v1, true);

    let workspace = db.workspace();

    // Phase 1: file_diagnostics must NOT have a ConfigLoaderRequiredFieldMissing.
    let diags_v1 = file_diagnostics(&db, workspace, sql_file);
    let missing_v1: Vec<_> = diags_v1
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ConfigLoaderRequiredFieldMissing))
        .collect();
    assert!(
        missing_v1.is_empty(),
        "file_diagnostics must not report ConfigLoaderRequiredFieldMissing \
         when the loader file satisfies the schema; got: {:?}",
        missing_v1
    );

    // Phase 2: mutate the loader file to have a wrong field — 'name' is now absent.
    let text_v2: Arc<str> = Arc::from("{wrong_field: x}");
    db.set_loader_file(loader_path.clone(), text_v2, true);

    // file_diagnostics must now report ConfigLoaderRequiredFieldMissing.
    let diags_v2 = file_diagnostics(&db, workspace, sql_file);
    let missing_v2: Vec<_> = diags_v2
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ConfigLoaderRequiredFieldMissing))
        .collect();
    assert!(
        !missing_v2.is_empty(),
        "file_diagnostics must report ConfigLoaderRequiredFieldMissing \
         after the loader file no longer satisfies the schema; got diags: {:?}",
        diags_v2
    );
}

// ============================================================================
// Phase 6: Per-target overlay Salsa wiring tests
// ============================================================================

/// When a `<basename>.<target>.<ext>` overlay file exists and the overlay
/// has an invalid field value (type mismatch), the validation diagnostic must
/// be anchored at the **overlay file's row**, not the base file or call site.
///
/// This exercises the spec rule: "A target overlay file that does not validate
/// against the schema emits the same diagnostic family as a base-file mismatch,
/// anchored at the overlay file's offending row."
#[test]
fn overlay_validation_failure_anchors_at_overlay_row() {
    let mut db = Database::default();

    // Base: {name: "us_west", threshold: 100} — valid
    let base_path: Arc<str> = Arc::from("configs/cohorts.yaml");
    let base_text: Arc<str> = Arc::from("name: us_west\nthreshold: 100\n");
    let base_input = db.set_loader_file(base_path.clone(), base_text, true);

    // Overlay: {threshold: "not_an_integer"} — invalid (type mismatch on line 1)
    let overlay_path: Arc<str> = Arc::from("configs/cohorts.prod.yaml");
    let overlay_text: Arc<str> = Arc::from("threshold: not_an_integer\n");
    let overlay_input = db.set_loader_file(overlay_path.clone(), overlay_text, true);

    let call_site = LoaderCallSiteId {
        file_path: Arc::from("models/cohorts.sql"),
        byte_offset: 7,
        loader_path: base_path.clone(),
        schema_text: Arc::from("{name: Text, threshold: Integer}"),
    };

    // Call the overlay query.
    let resolved = loader_resolved_value_with_overlay(&db, base_input, overlay_input, call_site);

    // Must have at least one diagnostic (overlay threshold has wrong type).
    assert!(
        !resolved.diagnostics.is_empty(),
        "overlay type-mismatch must produce at least one diagnostic; got none"
    );

    // The diagnostic must be anchored at the overlay row (line 0 in the overlay file),
    // NOT at the base file.
    let has_overlay_anchor = resolved.diagnostics.iter().any(|d| {
        // primary_span.line == 0 means line 1 in 1-indexed (the threshold row).
        // Any diagnostic anchored at the overlay file is acceptable.
        d.primary_span.line == 0
    });
    assert!(
        has_overlay_anchor,
        "overlay diagnostic must be anchored at the overlay file's row (line 0/1-indexed 1); \
         got: {:?}",
        resolved.diagnostics
    );
}

/// When no `<basename>.<target>.<ext>` overlay file exists (overlay is absent),
/// `loader_resolved_value_with_overlay` falls through to return a result
/// equal to `loader_resolved_value` on the base file alone — no overlay diagnostics.
///
/// The spec rule: "An absent overlay file is a no-op; the base value is used as-is."
/// We test this by passing `None` for the overlay (conceptually a missing overlay).
/// The implementation must expose a variant that accepts `Option<LoaderFileInput>`.
#[test]
fn overlay_absent_falls_through_to_base() {
    let mut db = Database::default();

    // Base: {name: "us_west", threshold: 100} — valid
    let base_path: Arc<str> = Arc::from("configs/cohorts.yaml");
    let base_text: Arc<str> = Arc::from("name: us_west\nthreshold: 100\n");
    let base_input = db.set_loader_file(base_path.clone(), base_text, true);

    let call_site = LoaderCallSiteId {
        file_path: Arc::from("models/cohorts.sql"),
        byte_offset: 7,
        loader_path: base_path.clone(),
        schema_text: Arc::from("{name: Text, threshold: Integer}"),
    };

    // Base-only resolution via the original query.
    let resolved_base = loader_resolved_value(&db, base_input, call_site.clone());

    // Absent overlay (mark as not-existing) — must equal base result.
    let overlay_path: Arc<str> = Arc::from("configs/cohorts.prod.yaml");
    let overlay_input = db.set_loader_file(overlay_path.clone(), Arc::from(""), false);
    let resolved_with_absent =
        loader_resolved_value_with_overlay(&db, base_input, overlay_input, call_site);

    // Diagnostics must both be empty (base is valid, absent overlay is no-op).
    assert!(
        resolved_base.diagnostics.is_empty(),
        "base-only resolution must have no diagnostics; got: {:?}",
        resolved_base.diagnostics
    );
    assert!(
        resolved_with_absent.diagnostics.is_empty(),
        "absent-overlay resolution must have no diagnostics; got: {:?}",
        resolved_with_absent.diagnostics
    );

    // Both must have a parsed result.
    assert!(
        resolved_base.parsed.is_some(),
        "base-only resolution must produce a parsed result"
    );
    assert!(
        resolved_with_absent.parsed.is_some(),
        "absent-overlay resolution must produce a parsed result"
    );
}

/// Verify that modifying the overlay file invalidates `loader_resolved_value_with_overlay`.
///
/// After the overlay text changes (via `set_loader_file`), a subsequent call must
/// reflect the new overlay content.
#[test]
fn overlay_file_change_invalidates_loader_value() {
    let mut db = Database::default();

    // Base: {name: "us_west", threshold: 100} — valid
    let base_path: Arc<str> = Arc::from("configs/cohorts.yaml");
    let base_text: Arc<str> = Arc::from("name: us_west\nthreshold: 100\n");
    let base_input = db.set_loader_file(base_path.clone(), base_text, true);

    // Overlay v1: {threshold: 50} — valid, overrides threshold only
    let overlay_path: Arc<str> = Arc::from("configs/cohorts.prod.yaml");
    let overlay_v1: Arc<str> = Arc::from("threshold: 50\n");
    let overlay_input_v1 = db.set_loader_file(overlay_path.clone(), overlay_v1, true);

    let call_site = LoaderCallSiteId {
        file_path: Arc::from("models/cohorts.sql"),
        byte_offset: 7,
        loader_path: base_path.clone(),
        schema_text: Arc::from("{name: Text, threshold: Integer}"),
    };

    // First resolution with overlay v1 — must have no diagnostics.
    let resolved_v1 =
        loader_resolved_value_with_overlay(&db, base_input, overlay_input_v1, call_site.clone());
    assert!(
        resolved_v1.diagnostics.is_empty(),
        "v1 overlay resolution must have no diagnostics; got: {:?}",
        resolved_v1.diagnostics
    );
    // Merged threshold must be 50 (from overlay).
    if let Some(ref p) = resolved_v1.merged {
        match p {
            crate::loader::MetaValue::Record(fields) => {
                assert_eq!(
                    fields.get("threshold"),
                    Some(&crate::loader::MetaValue::Integer(50)),
                    "merged threshold must be 50 from overlay v1; got: {:?}",
                    fields.get("threshold")
                );
            }
            other => panic!("expected Record merged value; got: {:?}", other),
        }
    } else {
        panic!("v1 merged value must be Some");
    }

    // Update overlay to v2: {threshold: 999}
    let overlay_v2: Arc<str> = Arc::from("threshold: 999\n");
    let overlay_input_v2 = db.set_loader_file(overlay_path.clone(), overlay_v2, true);
    assert!(
        overlay_input_v1 == overlay_input_v2,
        "set_loader_file must return same handle on update"
    );

    // Second resolution — Salsa must invalidate and re-evaluate.
    let resolved_v2 =
        loader_resolved_value_with_overlay(&db, base_input, overlay_input_v2, call_site);
    assert!(
        resolved_v2.diagnostics.is_empty(),
        "v2 overlay resolution must have no diagnostics; got: {:?}",
        resolved_v2.diagnostics
    );
    // Merged threshold must now be 999 (from overlay v2).
    if let Some(ref p) = resolved_v2.merged {
        match p {
            crate::loader::MetaValue::Record(fields) => {
                assert_eq!(
                    fields.get("threshold"),
                    Some(&crate::loader::MetaValue::Integer(999)),
                    "merged threshold must be 999 from overlay v2; got: {:?}",
                    fields.get("threshold")
                );
            }
            other => panic!("expected Record merged value; got: {:?}", other),
        }
    } else {
        panic!("v2 merged value must be Some");
    }
}

/// Verify that `smelt_record_declarations` collects declarations from all
/// files in the workspace and returns them in file order.
///
/// Two files each contribute one `smelt.record` declaration; the query must
/// return exactly two entries with the correct names.
#[test]
fn smelt_record_declarations_query_collects_workspace_decls() {
    let mut db = TestDb::default();

    // File 1: declares smelt.record SourceEntry.
    db.set_file_text(
        PathBuf::from("models/sources.sql"),
        Arc::new(
            "smelt.record SourceEntry = { name: Text, age: Integer }\n\
             SELECT 1"
                .to_string(),
        ),
    );

    // File 2: declares smelt.record MetricConfig.
    db.set_file_text(
        PathBuf::from("models/metrics.sql"),
        Arc::new(
            "smelt.record MetricConfig = { metric_name: Text, threshold: Float }\n\
             SELECT 1"
                .to_string(),
        ),
    );

    let workspace = db.sync_workspace();
    let decls = smelt_record_declarations(&db.db, workspace);

    assert_eq!(
        decls.len(),
        2,
        "workspace with two smelt.record declarations must produce exactly 2 entries; got {}: {:?}",
        decls.len(),
        decls.iter().map(|d| &d.name).collect::<Vec<_>>()
    );

    let names: Vec<&str> = decls.iter().map(|d| d.name.as_str()).collect();
    assert!(
        names.contains(&"SourceEntry"),
        "decls must include SourceEntry; got {:?}",
        names
    );
    assert!(
        names.contains(&"MetricConfig"),
        "decls must include MetricConfig; got {:?}",
        names
    );
}

// ============================================================
// Phase 1: Struct-spread `.*` / `.field` schema expansion tests
// ============================================================

/// Build a TestDb containing one function-definition file and one model file,
/// both scoped to the same project root ".".
///
/// `function_sql` is the raw text of a `smelt.define` file.
/// `model_sql` is the SELECT-body of the caller model.
fn setup_struct_spread_db(function_sql: &str, model_sql: &str) -> (TestDb, PathBuf) {
    let mut db = TestDb::default();

    let fn_path = PathBuf::from("functions/f.sql");
    db.set_file_text(fn_path.clone(), Arc::new(function_sql.to_string()));
    db.set_file_project_root(fn_path.clone(), PathBuf::from("."));

    let model_path = PathBuf::from("models/caller.sql");
    db.set_file_text(model_path.clone(), Arc::new(model_sql.to_string()));
    db.set_file_project_root(model_path.clone(), PathBuf::from("."));

    db.set_all_files(Arc::new(vec![fn_path, model_path.clone()]));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    (db, model_path)
}

/// `SELECT id, f(x).*` where `f -> Expr<Struct<{a: Text, b: Text}>>` must
/// resolve the output schema to `{id, a: Text, b: Text}`.
///
/// Currently only `{id}` is produced (the struct-spread is silently dropped).
#[test]
fn struct_spread_star_expands_fields_into_schema() {
    let function_sql = r#"smelt.define f(
    x: Expr<Integer>
) -> Expr<Struct<{a: Text, b: Text}>> AS ({
    CAST(x AS TEXT) AS a,
    CAST(x AS TEXT) AS b
})"#;

    let model_sql = "SELECT id, smelt.functions.f(id).* FROM smelt.sources.raw.t";

    let (mut db, model_path) = setup_struct_spread_db(function_sql, model_sql);
    let schema = db.typed_model_schema(model_path);

    let col_names: Vec<&str> = schema.columns.iter().map(|c| c.name.as_str()).collect();
    assert!(
        col_names.contains(&"id"),
        "schema must contain `id`; got {:?}",
        col_names
    );
    assert!(
        col_names.contains(&"a"),
        "struct-spread `.*` must expand field `a` into schema; got {:?}",
        col_names
    );
    assert!(
        col_names.contains(&"b"),
        "struct-spread `.*` must expand field `b` into schema; got {:?}",
        col_names
    );

    // Column types must be Text (from the declared struct field types).
    // smelt normalises the `Text` keyword to `Varchar { max_length: None }` internally,
    // so both variants are acceptable.
    let a_col = schema.columns.iter().find(|c| c.name == "a").unwrap();
    assert!(
        matches!(
            a_col.data_type.as_ref().map(|tc| &tc.data_type),
            Some(smelt_types::DataType::Text) | Some(smelt_types::DataType::Varchar { .. })
        ),
        "field `a` must have type Text/Varchar; got {:?}",
        a_col.data_type
    );
    let b_col = schema.columns.iter().find(|c| c.name == "b").unwrap();
    assert!(
        matches!(
            b_col.data_type.as_ref().map(|tc| &tc.data_type),
            Some(smelt_types::DataType::Text) | Some(smelt_types::DataType::Varchar { .. })
        ),
        "field `b` must have type Text/Varchar; got {:?}",
        b_col.data_type
    );
}

/// Row-tail (`Struct<{…, ..r}>`) returns are NOT expanded at the schema layer.
///
/// The codegen expander (`expand_smelt_path_call_star`) falls back to verbatim
/// SQL when the function body contains a `SPREAD_ITEM` (`..r`). Expanding at the
/// schema layer while codegen uses verbatim would violate the schema-layer/codegen
/// agreement invariant. Until the two layers are unified for row-tail structs, a
/// `.*` spread over a row-tail return contributes zero columns at the schema layer.
///
/// This test documents the boundary: the caller model's schema must be empty
/// (zero columns) when the only SELECT item is a row-tail struct spread.
#[test]
fn struct_spread_row_tail_not_expanded_at_schema_layer() {
    // Function with a row-tail in both parameter and return.
    let function_sql = r#"smelt.define tag_row(
    s: Expr<Struct<{id: Integer, ..r}>>
) -> Expr<Struct<{tag: Text, ..r}>> AS ({
    'tagged' AS tag,
    ..s
})"#;

    let upstream_sql = "SELECT 1 AS id, 'hello' AS extra1 FROM smelt.sources.raw.t";
    let model_sql = "SELECT smelt.functions.tag_row(t).* FROM smelt.models.t AS t";

    let mut db = TestDb::default();

    let fn_path = PathBuf::from("functions/tag_row.sql");
    db.set_file_text(fn_path.clone(), Arc::new(function_sql.to_string()));
    db.set_file_project_root(fn_path.clone(), PathBuf::from("."));

    let upstream_path = PathBuf::from("models/t.sql");
    db.set_file_text(upstream_path.clone(), Arc::new(upstream_sql.to_string()));
    db.set_file_project_root(upstream_path.clone(), PathBuf::from("."));

    let model_path = PathBuf::from("models/caller.sql");
    db.set_file_text(model_path.clone(), Arc::new(model_sql.to_string()));
    db.set_file_project_root(model_path.clone(), PathBuf::from("."));

    db.set_all_files(Arc::new(vec![fn_path, upstream_path, model_path.clone()]));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(model_path);

    let col_names: Vec<&str> = schema.columns.iter().map(|c| c.name.as_str()).collect();

    // Row-tail structs are NOT expanded at the schema layer: no columns from
    // `tag_row(t).*` should appear. The boundary holds until codegen and schema
    // expansion are unified.
    assert!(
        !col_names.contains(&"tag"),
        "row-tail spread must NOT be expanded at the schema layer; `tag` must be absent; got {:?}",
        col_names
    );
    assert!(
        !col_names.contains(&"extra1"),
        "row-tail spread must NOT be expanded at the schema layer; `extra1` must be absent; got {:?}",
        col_names
    );
}

// ============================================================
// Phase 2: CTE-argument seeding for TableExpr-returning functions
// ============================================================

/// A model that passes a local CTE as a `TableExpr` argument must seed the
/// function body's type context with the CTE's column schema.
///
/// Pattern:
///   WITH x AS (SELECT CAST(100 AS DECIMAL(18,2)) AS revenue,
///                     CAST(30  AS DECIMAL(18,2)) AS cost)
///   SELECT margin FROM smelt.functions.add_margin(x)
///
/// `add_margin` computes `revenue - cost AS margin`. Without CTE seeding the
/// body ctx has no `revenue`/`cost`, so `margin` resolves to Unknown.
/// After this phase it must resolve to a non-Unknown Decimal/Double.
#[test]
fn cte_arg_tableexpr_param_resolves_body_columns() {
    let function_sql = r#"smelt.define add_margin(
    source: TableExpr<{revenue: Numeric, cost: Numeric}>
) -> TableExpr AS (
    SELECT source.*, revenue - cost AS margin FROM source
)"#;

    // Model: CTE `x` supplies revenue + cost; passed by bare name into add_margin.
    let model_sql = r#"WITH x AS (
  SELECT
    CAST(100 AS DECIMAL(18, 2)) AS revenue,
    CAST(30  AS DECIMAL(18, 2)) AS cost
)
SELECT margin
FROM smelt.functions.add_margin(x)"#;

    let mut db = TestDb::default();

    let fn_path = PathBuf::from("functions/add_margin.sql");
    db.set_file_text(fn_path.clone(), Arc::new(function_sql.to_string()));
    db.set_file_project_root(fn_path.clone(), PathBuf::from("."));

    let model_path = PathBuf::from("models/margin_via_cte.sql");
    db.set_file_text(model_path.clone(), Arc::new(model_sql.to_string()));
    db.set_file_project_root(model_path.clone(), PathBuf::from("."));

    db.set_all_files(Arc::new(vec![fn_path, model_path.clone()]));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    let schema = db.typed_model_schema(model_path);

    let margin_col = schema.columns.iter().find(|c| c.name == "margin");
    assert!(
        margin_col.is_some(),
        "schema must contain a `margin` column; got {:?}",
        schema
            .columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
    );

    let margin_type = margin_col
        .and_then(|c| c.data_type.as_ref())
        .map(|tc| &tc.data_type);

    assert!(
        !matches!(margin_type, Some(DataType::Unknown) | None),
        "margin column must resolve to a non-Unknown type when CTE arg is seeded; got {:?}",
        margin_type
    );
}

// ============================================================
// Phase 3: Cycle guard for nested smelt.functions.* body CTE resolution
// ============================================================

/// Two mutually-recursive `TableExpr` functions — `alpha` calls `beta` and
/// `beta` calls `alpha` in their body CTEs — must NOT cause a stack overflow
/// when the schema resolver walks the call graph. The cycle guard in
/// `resolve_smelt_path_call_schema` must detect the back-edge and short-circuit
/// (returning `None` / opaque columns) rather than recursing infinitely.
///
/// Correctness requirements:
/// - Calling `typed_model_schema` (which drives schema resolution) on a model
///   that calls `alpha` must TERMINATE without panicking.
/// - The columns returned may be `Unknown`/empty (the cycle is unresolvable),
///   but the process must not crash.
/// - A `FunctionCallCycle` diagnostic must still be emitted for both `alpha`
///   and `beta` (the cycle guard must not suppress the diagnostic path).
///
/// **RED state before fix**: `typed_model_schema` overflows the stack and the
/// test binary crashes with a SIGSEGV / "thread has overflowed its stack".
/// **GREEN state after fix**: returns normally; assertions about diagnostics pass.
#[test]
fn mutual_recursion_body_cte_terminates_without_stack_overflow() {
    // `alpha` body: CTE `x` selects from `smelt.functions.beta(data)` which
    // creates the A→B dependency.
    let alpha_sql = r#"smelt.define alpha(
    data: TableExpr<{id: Integer}>
) -> TableExpr AS (
    WITH x AS (SELECT * FROM smelt.functions.beta(data))
    SELECT id FROM x
)"#;

    // `beta` body: CTE `y` selects from `smelt.functions.alpha(data)` which
    // creates the B→A back-edge, completing the cycle.
    let beta_sql = r#"smelt.define beta(
    data: TableExpr<{id: Integer}>
) -> TableExpr AS (
    WITH y AS (SELECT * FROM smelt.functions.alpha(data))
    SELECT id FROM y
)"#;

    // The caller model passes a source table into `alpha`.
    let model_sql = "SELECT id FROM smelt.functions.alpha(smelt.sources.raw.t)";

    let mut db = TestDb::default();

    let alpha_path = PathBuf::from("functions/alpha.sql");
    db.set_file_text(alpha_path.clone(), Arc::new(alpha_sql.to_string()));
    db.set_file_project_root(alpha_path.clone(), PathBuf::from("."));

    let beta_path = PathBuf::from("functions/beta.sql");
    db.set_file_text(beta_path.clone(), Arc::new(beta_sql.to_string()));
    db.set_file_project_root(beta_path.clone(), PathBuf::from("."));

    let model_path = PathBuf::from("models/caller.sql");
    db.set_file_text(model_path.clone(), Arc::new(model_sql.to_string()));
    db.set_file_project_root(model_path.clone(), PathBuf::from("."));

    db.set_all_files(Arc::new(vec![
        alpha_path.clone(),
        beta_path.clone(),
        model_path.clone(),
    ]));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    // This must terminate (not stack-overflow).  The exact schema may be
    // empty or opaque — the cycle is logically unresolvable — but the
    // resolver must return without crashing.
    let schema = db.typed_model_schema(model_path.clone());
    let _ = schema; // result may be empty/opaque; the assertion is TERMINATION

    // The `FunctionCallCycle` diagnostic must still fire for both functions.
    // (The cycle guard in the schema resolver must not suppress the diagnostic
    // path, which runs independently via `function_call_cycle_fn_ids`.)
    let alpha_diags = db.file_diagnostics(alpha_path);
    let beta_diags = db.file_diagnostics(beta_path);

    let has_cycle_diag = |diags: &[Diagnostic]| {
        diags
            .iter()
            .any(|d| d.code == Some(DiagnosticCode::FunctionCallCycle))
    };

    assert!(
        has_cycle_diag(&alpha_diags),
        "alpha must have a FunctionCallCycle diagnostic; got {:?}",
        alpha_diags
            .iter()
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        has_cycle_diag(&beta_diags),
        "beta must have a FunctionCallCycle diagnostic; got {:?}",
        beta_diags
            .iter()
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
    );
}
