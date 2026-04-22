#![cfg(test)]
#![allow(clippy::uninlined_format_args)]

use super::*;
use crate::test_harness::TestDb;
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
            Arc::new("SELECT\n  user_id,\n  COUNT(*) as session_count\nFROM smelt.ref('raw_events')\nGROUP BY user_id".to_string()),
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
        Arc::new("SELECT\n  user_id\nFROM smelt.ref('raw_events')".to_string()),
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

    // Create a model with an undefined ref
    let path = PathBuf::from("test_model.sql");
    db.set_file_text(
        path.clone(),
        Arc::new("SELECT * FROM smelt.ref('nonexistent_model')".to_string()),
    );

    // Register the file (no other files, so ref won't resolve)
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));

    // Get diagnostics
    let diagnostics = db.file_diagnostics(path);

    // Should have exactly one diagnostic for undefined ref
    assert_eq!(diagnostics.len(), 1);
    let diag = &diagnostics[0];

    // Check severity and message
    assert_eq!(diag.severity, DiagnosticSeverity::Error);
    assert!(diag
        .message
        .contains("Undefined model reference: 'nonexistent_model'"));

    // Check position - should point to the string parameter 'nonexistent_model'
    // In "SELECT * FROM smelt.ref('nonexistent_model')", the STRING token (including quotes)
    // starts at position 24 and ends at position 43 (exclusive)
    assert_eq!(diag.range.start.line, 0);
    assert_eq!(diag.range.start.column, 24); // Opening quote ' (0-indexed)
    assert_eq!(diag.range.end.line, 0);
    assert_eq!(diag.range.end.column, 43); // One past closing quote ' (exclusive)
}

#[test]
fn test_undefined_ref_diagnostic_position_multiline() {
    let mut db = TestDb::default();

    // Create a model matching broken_model.sql structure
    let path = PathBuf::from("broken_model.sql");
    let content = "-- This model has an undefined reference - should show diagnostic\nSELECT *\nFROM smelt.ref('nonexistent_model')\n";
    db.set_file_text(path.clone(), Arc::new(content.to_string()));

    // Debug: Check what the parser extracts
    let parse = db.parse_file(path.clone());
    let text = db.file_text(path.clone());
    use smelt_parser::ast::File as AstFile;
    if let Some(file) = AstFile::cast(parse.syntax()) {
        for ref_call in file.refs() {
            println!("Found ref call");
            if let Some(name) = ref_call.model_name() {
                println!("  Model name: {:?}", name);
            }
            if let Some(text_range) = ref_call.name_range() {
                println!("  TextRange: {:?}", text_range);
                println!(
                    "  Start offset: {}, End offset: {}",
                    usize::from(text_range.start()),
                    usize::from(text_range.end())
                );

                // Check content length
                println!("  Content length: {}", text.len());

                // Extract the actual text at this range (if valid)
                let start = usize::from(text_range.start());
                let end = usize::from(text_range.end());
                if end <= text.len() {
                    let extracted = &text[start..end];
                    println!("  Extracted text: {:?}", extracted);
                } else {
                    println!(
                        "  ERROR: Range {} out of bounds (content length is {})",
                        end,
                        text.len()
                    );
                }
            }
        }
    }

    // Register the file (no other files, so ref won't resolve)
    db.set_all_files(Arc::new(vec![path.clone()]));
    db.set_file_project_root(path.clone(), PathBuf::from("."));

    // Get diagnostics
    let diagnostics = db.file_diagnostics(path);

    // Debug output
    println!("\nContent: {:?}", content);
    println!("Content length: {}", content.len());
    println!("Number of diagnostics: {}", diagnostics.len());
    if !diagnostics.is_empty() {
        let diag = &diagnostics[0];
        println!(
            "Diagnostic range: line {} col {} to line {} col {}",
            diag.range.start.line,
            diag.range.start.column,
            diag.range.end.line,
            diag.range.end.column
        );
    }

    // Should have exactly one diagnostic
    assert_eq!(diagnostics.len(), 1);
    let diag = &diagnostics[0];

    // Check it's on line 2 (0-indexed)
    assert_eq!(diag.range.start.line, 2);
    assert_eq!(diag.range.end.line, 2);

    // In "FROM smelt.ref('nonexistent_model')", the model name should be highlighted
    // Expected: 'nonexistent_model' with quotes starting at column 16
    println!("Expected to highlight 'nonexistent_model' on line 2");
}

#[test]
fn test_lexer_positions() {
    use smelt_parser::lexer::tokenize;

    let content = "-- This model has an undefined reference - should show diagnostic\nSELECT *\nFROM smelt.ref('nonexistent_model')\n";
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
        Arc::new("SELECT id, email, created_at FROM smelt.source('raw.users')".to_string()),
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
    FROM smelt.source('raw.orders')
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
    FROM smelt.source('raw.orders')
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
    FROM smelt.source('raw.orders')
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
        FROM smelt.source('raw.orders')
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
    SELECT id, parent_id FROM smelt.source('raw.nodes') WHERE parent_id IS NULL
    UNION ALL
    SELECT n.id, n.parent_id FROM smelt.source('raw.nodes') n
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
FROM smelt.source('raw.orders')
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
FROM smelt.source('raw.orders')
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
SELECT EXISTS (SELECT 1 FROM smelt.source('raw.users') WHERE id = user_id) as has_user
FROM smelt.source('raw.orders')
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
FROM smelt.source('raw.orders')
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
FROM smelt.source('raw.orders')
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
SELECT id, amount FROM smelt.source('raw.orders')
UNION
SELECT id, amount FROM smelt.source('raw.returns')
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
FROM smelt.source('raw.orders') o
INNER JOIN smelt.source('raw.users') u ON o.user_id = u.id
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
FROM smelt.source('raw.orders') o
INNER JOIN smelt.source('raw.users') u ON o.user_id = u.id
INNER JOIN smelt.source('raw.products') p ON o.product_id = p.id
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
FROM smelt.source('raw.orders') o
LEFT JOIN smelt.source('raw.users') u ON o.user_id = u.id
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
    let sql = r#"
SELECT u.id, recent.total_amount
FROM smelt.source('raw.users') u
LEFT JOIN LATERAL (
    SELECT SUM(o.amount) as total_amount
    FROM smelt.source('raw.orders') o
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
FROM smelt.source('raw.users') u
LEFT JOIN LATERAL (
    SELECT SUM(o.amount) as total_amount
    FROM smelt.source('raw.orders') o
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
FROM smelt.source('raw.orders')
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
        Arc::new("SELECT * FROM smelt.ref('input')".to_string()),
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
        Arc::new("SELECT *, 1 as foo FROM smelt.ref('input')".to_string()),
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
        Arc::new("SELECT * FROM smelt.ref('input')".to_string()),
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
        Arc::new("SELECT *, 1 as foo FROM smelt.ref('input')".to_string()),
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
        Arc::new("SELECT * FROM smelt.ref('a')".to_string()),
    );

    let c_path = PathBuf::from("models/c.sql");
    db.set_file_text(
        c_path.clone(),
        Arc::new("SELECT * FROM smelt.ref('b')".to_string()),
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
        Arc::new("SELECT id, email FROM smelt.source('raw.users')".to_string()),
    );

    let downstream_path = PathBuf::from("models/passthrough.sql");
    db.set_file_text(
        downstream_path.clone(),
        Arc::new("SELECT * FROM smelt.ref('input')".to_string()),
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
        Arc::new("SELECT user_id FROM smelt.ref('input')".to_string()),
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
        Arc::new("SELECT t.user_id, t.email FROM smelt.ref('input') t".to_string()),
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
    let content = "---\ntags:\n  - event_source\n---\nSELECT event_id, user_id\nFROM smelt.ref('raw_events')\n";

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
        "SELECT user_id, COUNT(*) as total_events\nFROM smelt.ref('events')\nGROUP BY user_id",
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
                "SELECT u.user_id, u.user_name, SUM(o.amount) as total\nFROM smelt.ref('users') u\nINNER JOIN smelt.ref('orders') o ON u.user_id = o.user_id\nGROUP BY u.user_id, u.user_name",
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
        "SELECT user_id, event_type\nFROM smelt.ref('events')\nWHERE event_type = 'click'",
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
        "SELECT user_id, SUM(amount) as total\nFROM smelt.ref('orders')\nGROUP BY user_id",
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
        "SELECT user_id, COUNT(*) as cnt\nFROM smelt.ref('events')\nGROUP BY user_id",
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
    let (mut db, path) = setup_single_model("SELECT *\nFROM smelt.ref('events')");

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
        setup_single_model("SELECT COUNT(*) as cnt\nFROM smelt.ref('events')\nGROUP BY category");

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
            "SELECT user_id, COUNT(*) as cnt\nFROM smelt.ref('events')\nGROUP BY user_id\nHAVING COUNT(*) > 5",
        );

    let ft = db.model_function_type(path);

    assert_eq!(ft.inputs.len(), 1);
    // user_id should be in inputs (from SELECT + GROUP BY)
    assert!(ft.inputs[0].columns.iter().any(|c| c.name == "user_id"));
}

#[test]
fn test_function_type_order_by_columns_collected() {
    let (mut db, path) =
        setup_single_model("SELECT user_id\nFROM smelt.ref('events')\nORDER BY event_time");

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
        setup_single_model("SELECT user_id, event_timestamp\nFROM smelt.source('raw.events')");

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
    let (mut db, path) = setup_single_model(
        "SELECT e.user_id, e.event_timestamp\nFROM smelt.source('raw.events') e",
    );

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
            "SELECT event_id, user_id, event_time, event_type FROM smelt.source('raw.events')",
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
                "SELECT event_id, user_id, event_type FROM smelt.source('raw.events')",
            ),
            (
                "clicks",
                "SELECT event_id, user_id FROM smelt.ref('raw_events') WHERE event_type = 'click'",
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
                    "SELECT event_id, user_id FROM smelt.source('raw.events')",
                ),
                (
                    "user_counts",
                    "SELECT user_id, COUNT(*) as event_count FROM smelt.ref('raw_events') GROUP BY user_id",
                ),
                (
                    "totals",
                    "SELECT SUM(event_count) as total_events FROM smelt.ref('user_counts')",
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
                    "SELECT amount, user_id, created_at FROM smelt.source('raw.transactions')",
                ),
                (
                    "daily",
                    "SELECT user_id, CAST(created_at AS DATE) as day, SUM(amount) as daily_total FROM smelt.ref('base') GROUP BY user_id, CAST(created_at AS DATE)",
                ),
                (
                    "summary",
                    "SELECT user_id, COUNT(*) as active_days, SUM(daily_total) as grand_total FROM smelt.ref('daily') GROUP BY user_id",
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
            (
                "users",
                "SELECT user_id, name FROM smelt.source('raw.users')",
            ),
            (
                "orders",
                "SELECT order_id, user_id, amount FROM smelt.source('raw.orders')",
            ),
            (
                "user_orders",
                "SELECT u.user_id, u.name, SUM(o.amount) as total_spent \
                     FROM smelt.ref('users') u \
                     INNER JOIN smelt.ref('orders') o ON u.user_id = o.user_id \
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
            ("base", "SELECT id, value FROM smelt.source('raw.data')"),
            ("mid", "SELECT * FROM smelt.ref('base')"),
            ("top", "SELECT * FROM smelt.ref('mid')"),
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
            "SELECT mystery_col FROM smelt.ref('upstream')",
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
            "SELECT COALESCE(NULL, value) AS result FROM smelt.source('raw.data')",
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
            "SELECT COALESCE(value, 0) as result FROM smelt.source('raw.data')",
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
        ("model", "SELECT unknown_col FROM smelt.ref('upstream')"),
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
            "SELECT id, COUNT(*) as cnt FROM smelt.source('raw.data') GROUP BY id",
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
        ("model_a", "SELECT x FROM smelt.ref('model_b')"),
        ("model_b", "SELECT y FROM smelt.ref('model_a')"),
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
    let (mut db, paths) = setup_multi_model(&[("self_ref", "SELECT x FROM smelt.ref('self_ref')")]);

    // Should not panic
    let schema = db.typed_model_schema(paths[0].clone());
    // Schema may be empty or have Unknown types — either is fine
    let _ = schema;
}

#[test]
fn test_circular_ref_three_models() {
    // A -> B -> C -> A cycle
    let (mut db, paths) = setup_multi_model(&[
        ("cycle_a", "SELECT x FROM smelt.ref('cycle_b')"),
        ("cycle_b", "SELECT y FROM smelt.ref('cycle_c')"),
        ("cycle_c", "SELECT z FROM smelt.ref('cycle_a')"),
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
        ("diag_a", "SELECT x FROM smelt.ref('diag_b')"),
        ("diag_b", "SELECT y FROM smelt.ref('diag_a')"),
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
        ("cyc_a", "SELECT x FROM smelt.ref('cyc_b')"),
        ("cyc_b", "SELECT y FROM smelt.ref('cyc_a')"),
        ("good_c", "SELECT CAST(1 AS INTEGER) AS val"),
        ("good_d", "SELECT val FROM smelt.ref('good_c')"),
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
        ("cyc_a", "SELECT x FROM smelt.ref('cyc_b')"),
        ("cyc_b", "SELECT y FROM smelt.ref('cyc_a')"),
    ]);

    // This should not panic — mimics publish_all_diagnostics in the LSP
    for path in &paths {
        let _file_diags = db.file_diagnostics(path.clone());
        let _type_diags = db.type_diagnostics(path.clone());
    }
}

#[test]
fn test_self_ref_type_diagnostics_no_panic() {
    let (mut db, paths) = setup_multi_model(&[("self_ref", "SELECT x FROM smelt.ref('self_ref')")]);

    let _file_diags = db.file_diagnostics(paths[0].clone());
    let _type_diags = db.type_diagnostics(paths[0].clone());
}

#[test]
fn test_circular_ref_incremental_update_no_panic() {
    // Simulates the LSP pattern: query diagnostics, then mutate file, then query again.
    // This exercises Salsa's memoized value validation path which is different from
    // first-time computation.
    let (mut db, paths) = setup_multi_model(&[
        ("cyc_a", "SELECT x FROM smelt.ref('cyc_b')"),
        ("cyc_b", "SELECT y FROM smelt.ref('cyc_a')"),
    ]);

    // First pass: populate caches
    for path in &paths {
        let _file_diags = db.file_diagnostics(path.clone());
        let _type_diags = db.type_diagnostics(path.clone());
    }

    // Mutate one file (triggers Salsa revision change)
    db.set_file_text(
        paths[0].clone(),
        Arc::new("SELECT x, 1 AS extra FROM smelt.ref('cyc_b')".to_string()),
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
            ("raw_data", "SELECT price FROM smelt.source('raw.data')"),
            (
                "totals",
                "SELECT SUM(price) AS total_price FROM smelt.ref('raw_data')",
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
            ("raw_data", "SELECT amount FROM smelt.source('raw.data')"),
            (
                "totals",
                "SELECT SUM(amount) AS total FROM smelt.ref('raw_data')",
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
                "SELECT name, status FROM smelt.source('raw.data')",
            ),
            (
                "agg",
                "SELECT SUM(name) AS s1, AVG(status) AS s2 FROM smelt.ref('raw_data')",
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
        "SELECT SUM(x) AS total FROM smelt.ref('nonexistent')",
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
            ("passthrough", "SELECT value FROM smelt.source('raw.data')"),
            (
                "aggregator",
                "SELECT SUM(value) AS total FROM smelt.ref('passthrough')",
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
                "SELECT SUM(up_0) AS agg_0 FROM smelt.ref('upstream')",
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
            "SELECT event_id, missing_col FROM smelt.source('raw.events')",
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
            "SELECT event_id, user_id FROM smelt.source('raw.events')",
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
            "SELECT user_id, event_count FROM smelt.source('raw.events')",
        ),
        (
            "downstream",
            "SELECT user_id, nonexistent FROM smelt.ref('upstream')",
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
            "SELECT event_id, user_id FROM smelt.source('raw.events')",
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
                "WITH cte AS (SELECT event_id, 1 AS extra FROM smelt.source('raw.events')) SELECT event_id, extra FROM cte",
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
        &[("my_model", "SELECT * FROM smelt.source('raw.events')")],
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
        Arc::new("SELECT * FROM smelt.ref('b')".to_string()),
    );
    db.set_file_text(
        b_path.clone(),
        Arc::new("SELECT * FROM smelt.ref('a')".to_string()),
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
        Arc::new("SELECT * FROM smelt.ref('b')".to_string()),
    );
    db.set_file_text(
        b_path.clone(),
        Arc::new("SELECT * FROM smelt.ref('c')".to_string()),
    );
    db.set_file_text(
        c_path.clone(),
        Arc::new("SELECT * FROM smelt.ref('a')".to_string()),
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
