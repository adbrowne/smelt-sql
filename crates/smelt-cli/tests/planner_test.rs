#![cfg(feature = "duckdb")]
//! Integration tests for planner: cube split, incremental, and composed.

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_planner::{ExecutionStep, Frontmatter, ModelGraph, ModelInfo, Planner};
use tempfile::TempDir;

/// Create an in-memory DuckDB backend for testing.
async fn create_backend() -> (DuckDbBackend, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main").await.unwrap();
    (backend, temp_dir)
}

/// Seed events table with synthetic data spanning 5 days.
async fn seed_events(backend: &DuckDbBackend) {
    backend
        .execute_sql(
            r#"
            CREATE TABLE events AS
            SELECT
                i as id,
                'user_' || (i % 50 + 1) as user_id,
                'session_' || (i % 200 + 1) as session_id,
                CASE i % 5
                    WHEN 0 THEN 'US'
                    WHEN 1 THEN 'UK'
                    WHEN 2 THEN 'DE'
                    WHEN 3 THEN 'FR'
                    ELSE 'JP'
                END as country,
                (i % 7 + 1) * 10.0 as revenue,
                TIMESTAMP '2024-01-01' + INTERVAL (i % 5) DAY
                    + INTERVAL (i * 37 % 1440) MINUTE as event_time
            FROM generate_series(1, 1000) t(i)
        "#,
        )
        .await
        .unwrap();
}

/// Execute steps against DuckDB, resolving refs.
async fn execute_steps(backend: &DuckDbBackend, steps: &[ExecutionStep], model_name: &str) {
    for step in steps {
        match step {
            ExecutionStep::CreateTemp { name, sql } => {
                let create_sql = format!("CREATE TEMP TABLE {} AS {}", name, sql);
                backend.execute_sql(&create_sql).await.unwrap();
            }
            ExecutionStep::FinalQuery { sql } => {
                let _ = backend
                    .execute_sql(&format!("DROP TABLE IF EXISTS main.{}", model_name))
                    .await;
                let create_sql = format!("CREATE TABLE main.{} AS {}", model_name, sql);
                backend.execute_sql(&create_sql).await.unwrap();
            }
            ExecutionStep::DropTemp { name } => {
                let _ = backend
                    .execute_sql(&format!("DROP TABLE IF EXISTS {}", name))
                    .await;
            }
            ExecutionStep::AppendToTemp { name, sql } => {
                let insert_sql = format!("INSERT INTO {} {}", name, sql);
                backend.execute_sql(&insert_sql).await.unwrap();
            }
        }
    }
}

// ─── Cube Split Tests ───────────────────────────────────────────────

#[tokio::test]
async fn test_cube_split_matches_naive() {
    let (backend, _dir) = create_backend().await;
    seed_events(&backend).await;

    // Run naive query
    let naive_sql = r#"
        SELECT
            date_trunc('day', event_time) as event_date,
            country,
            COUNT(DISTINCT user_id) as unique_users,
            COUNT(DISTINCT session_id) as unique_sessions,
            COUNT(*) as total_events,
            SUM(revenue) as total_revenue
        FROM events
        GROUP BY 1, 2
        ORDER BY 1, 2
    "#;
    backend
        .execute_sql(&format!("CREATE TABLE main.naive_result AS {}", naive_sql))
        .await
        .unwrap();

    // Run planner on annotated SQL
    let annotated_sql = r#"SELECT
        date_trunc('day', event_time) as event_date,
        country,
        COUNT(DISTINCT user_id) as unique_users,
        COUNT(DISTINCT session_id) as unique_sessions,
        COUNT(*) as total_events,
        SUM(revenue) as total_revenue
    FROM events
    GROUP BY 1, 2 -- smelt:cube_split"#;

    let model = ModelInfo {
        name: "cube_result".to_string(),
        sql: annotated_sql.to_string(),
        refs: vec![],
        incremental_config: None,
    };

    let mut graph = ModelGraph::new();
    graph.add_model(model.clone());

    let planner = Planner::new();
    let (transformations, errors) = planner.plan(&graph);
    assert!(errors.is_empty(), "Planner errors: {:?}", errors);
    assert_eq!(transformations.len(), 1);

    // Extract steps and execute
    let steps = match &transformations[0] {
        smelt_planner::Transformation::ReplaceWithPlan { steps, .. } => steps,
        _ => panic!("Expected ReplaceWithPlan"),
    };

    execute_steps(&backend, steps, "cube_result").await;

    // Compare results
    let diff = backend
        .execute_sql(
            r#"
            SELECT COUNT(*) as diff_count FROM (
                SELECT event_date, country, unique_users, unique_sessions, total_events, total_revenue
                FROM main.naive_result
                EXCEPT
                SELECT event_date, country, unique_users, unique_sessions, total_events, total_revenue
                FROM main.cube_result
            ) t
        "#,
        )
        .await
        .unwrap();

    let diff_count = diff[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    assert_eq!(diff_count, 0, "Cube split result differs from naive query");

    // Also check reverse direction
    let diff_reverse = backend
        .execute_sql(
            r#"
            SELECT COUNT(*) as diff_count FROM (
                SELECT event_date, country, unique_users, unique_sessions, total_events, total_revenue
                FROM main.cube_result
                EXCEPT
                SELECT event_date, country, unique_users, unique_sessions, total_events, total_revenue
                FROM main.naive_result
            ) t
        "#,
        )
        .await
        .unwrap();

    let diff_count_reverse = diff_reverse[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    assert_eq!(
        diff_count_reverse, 0,
        "Cube split result has extra rows vs naive"
    );

    // Verify row counts match
    let naive_count = backend.get_row_count("main", "naive_result").await.unwrap();
    let cube_count = backend.get_row_count("main", "cube_result").await.unwrap();
    assert_eq!(
        naive_count, cube_count,
        "Row counts differ: naive={}, cube={}",
        naive_count, cube_count
    );
}

#[tokio::test]
async fn test_cube_split_with_ref_calls() {
    let (backend, _dir) = create_backend().await;
    seed_events(&backend).await;

    // Model uses smelt.ref('events') — planner preserves it, we resolve manually
    let annotated_sql = r#"SELECT
        country,
        COUNT(DISTINCT user_id) as unique_users,
        COUNT(DISTINCT session_id) as unique_sessions
    FROM smelt.ref('events')
    GROUP BY 1 -- smelt:cube_split"#;

    let model = ModelInfo {
        name: "ref_cube".to_string(),
        sql: annotated_sql.to_string(),
        refs: vec!["events".to_string()],
        incremental_config: None,
    };

    let mut graph = ModelGraph::new();
    graph.add_model(model);

    let planner = Planner::new();
    let (transformations, errors) = planner.plan(&graph);
    assert!(errors.is_empty());

    let steps = match &transformations[0] {
        smelt_planner::Transformation::ReplaceWithPlan { steps, .. } => steps,
        _ => panic!("Expected ReplaceWithPlan"),
    };

    // Resolve refs in steps (simulate what the CLI does)
    let resolved_steps: Vec<ExecutionStep> = steps
        .iter()
        .map(|step| match step {
            ExecutionStep::CreateTemp { name, sql } => ExecutionStep::CreateTemp {
                name: name.clone(),
                sql: smelt_cli::resolve_refs_in_sql(sql, "main"),
            },
            other => other.clone(),
        })
        .collect();

    // Verify refs were resolved
    for step in &resolved_steps {
        if let ExecutionStep::CreateTemp { sql, .. } = step {
            assert!(!sql.contains("smelt.ref"), "Refs not resolved in: {}", sql);
            assert!(
                sql.contains("main.events"),
                "Missing resolved ref in: {}",
                sql
            );
        }
    }

    execute_steps(&backend, &resolved_steps, "ref_cube").await;

    let count = backend.get_row_count("main", "ref_cube").await.unwrap();
    assert!(count > 0, "Expected results from cube split with refs");
}

// ─── Incremental Tests ──────────────────────────────────────────────

#[tokio::test]
async fn test_incremental_full_then_partial() {
    let (backend, _dir) = create_backend().await;

    // Seed raw_events spanning 5 days
    backend
        .execute_sql(
            r#"
            CREATE TABLE raw_events AS
            SELECT
                i as id,
                'user_' || (i % 20 + 1) as user_id,
                TIMESTAMP '2024-01-01' + INTERVAL (i % 5) DAY
                    + INTERVAL (i * 17 % 1440) MINUTE as event_time
            FROM generate_series(1, 500) t(i)
        "#,
        )
        .await
        .unwrap();

    let model_sql = r#"---
materialized: table
incremental:
  partition_column: event_date
  event_time_column: event_time
  granularity: day
---
SELECT
    date_trunc('day', event_time) as event_date,
    user_id,
    COUNT(*) as event_count
FROM raw_events
GROUP BY 1, 2"#;

    // Detect incremental config via planner
    let frontmatter = Frontmatter::parse(model_sql).unwrap();
    let model = ModelInfo {
        name: "daily_events".to_string(),
        sql: model_sql.to_string(),
        refs: vec![],
        incremental_config: frontmatter.incremental,
    };

    let mut graph = ModelGraph::new();
    graph.add_model(model);

    let planner = Planner::new();
    let (transformations, errors) = planner.plan(&graph);
    assert!(errors.is_empty());
    assert_eq!(transformations.len(), 1);

    let (event_time_col, partition_col) = match &transformations[0] {
        smelt_planner::Transformation::SetIncremental {
            event_time_column,
            partition_column,
            ..
        } => (event_time_column.clone(), partition_column.clone()),
        _ => panic!("Expected SetIncremental"),
    };

    assert_eq!(event_time_col, "event_time");
    assert_eq!(partition_col, "event_date");

    // Full refresh: days 1-5
    let stripped_sql = Frontmatter::strip(model_sql);
    let range = smelt_cli::TimeRange {
        start: "2024-01-01".to_string(),
        end: "2024-01-06".to_string(),
    };
    let filtered_sql =
        smelt_cli::inject_time_filter(stripped_sql, &event_time_col, &range).unwrap();

    backend
        .execute_sql(&format!(
            "CREATE TABLE main.daily_events AS {}",
            filtered_sql
        ))
        .await
        .unwrap();

    let initial_count = backend.get_row_count("main", "daily_events").await.unwrap();
    assert!(initial_count > 0);

    // Add day 6 data
    backend
        .execute_sql(
            r#"
            INSERT INTO raw_events
            SELECT
                500 + i as id,
                'user_' || (i % 10 + 1) as user_id,
                TIMESTAMP '2024-01-06' + INTERVAL (i * 17 % 1440) MINUTE as event_time
            FROM generate_series(1, 100) t(i)
        "#,
        )
        .await
        .unwrap();

    // Incremental run for day 6
    let range_day6 = smelt_cli::TimeRange {
        start: "2024-01-06".to_string(),
        end: "2024-01-07".to_string(),
    };
    let filtered_day6 =
        smelt_cli::inject_time_filter(stripped_sql, &event_time_col, &range_day6).unwrap();

    // DELETE + INSERT pattern
    backend
        .execute_sql("DELETE FROM main.daily_events WHERE event_date IN ('2024-01-06')")
        .await
        .unwrap();
    backend
        .execute_sql(&format!("INSERT INTO main.daily_events {}", filtered_day6))
        .await
        .unwrap();

    let final_count = backend.get_row_count("main", "daily_events").await.unwrap();
    assert!(
        final_count > initial_count,
        "Expected more rows after incremental: initial={}, final={}",
        initial_count,
        final_count
    );

    // Verify day 6 data exists
    let day6_rows = backend
        .execute_sql("SELECT COUNT(*) FROM main.daily_events WHERE event_date = '2024-01-06'")
        .await
        .unwrap();
    let day6_count = day6_rows[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    assert!(day6_count > 0, "Day 6 should have rows");
}

// ─── Composed Tests (Cube Split + Incremental) ─────────────────────

#[tokio::test]
async fn test_composed_cube_split_incremental() {
    let (backend, _dir) = create_backend().await;
    seed_events(&backend).await;

    let model_sql = r#"---
materialized: table
incremental:
  partition_column: event_date
  event_time_column: event_time
  granularity: day
---
SELECT
    date_trunc('day', event_time) as event_date,
    country,
    COUNT(DISTINCT user_id) as unique_users,
    COUNT(DISTINCT session_id) as unique_sessions,
    COUNT(*) as total_events,
    SUM(revenue) as total_revenue
FROM events
GROUP BY 1, 2 -- smelt:cube_split"#;

    let frontmatter = Frontmatter::parse(model_sql).unwrap();
    let model = ModelInfo {
        name: "cube_metrics".to_string(),
        sql: model_sql.to_string(),
        refs: vec![],
        incremental_config: frontmatter.incremental,
    };

    let mut graph = ModelGraph::new();
    graph.add_model(model);

    let planner = Planner::new();
    let (transformations, errors) = planner.plan(&graph);
    assert!(errors.is_empty());
    assert_eq!(
        transformations.len(),
        2,
        "Expected both cube_split and incremental transformations"
    );

    // Extract both transformations
    let mut plan_steps = None;
    let mut inc_config = None;

    for t in &transformations {
        match t {
            smelt_planner::Transformation::ReplaceWithPlan { steps, .. } => {
                plan_steps = Some(steps.clone());
            }
            smelt_planner::Transformation::SetIncremental {
                event_time_column,
                partition_column,
                ..
            } => {
                inc_config = Some((event_time_column.clone(), partition_column.clone()));
            }
            _ => {}
        }
    }

    let steps = plan_steps.unwrap();
    let (event_time_col, _partition_col) = inc_config.unwrap();

    // Run naive query for full date range as reference
    let naive_sql = r#"
        SELECT
            date_trunc('day', event_time) as event_date,
            country,
            COUNT(DISTINCT user_id) as unique_users,
            COUNT(DISTINCT session_id) as unique_sessions,
            COUNT(*) as total_events,
            SUM(revenue) as total_revenue
        FROM events
        WHERE event_time >= '2024-01-01' AND event_time < '2024-01-06'
        GROUP BY 1, 2
    "#;
    backend
        .execute_sql(&format!(
            "CREATE TABLE main.naive_composed AS {}",
            naive_sql
        ))
        .await
        .unwrap();

    // Execute cube split steps with time filtering for full range
    let range = smelt_cli::TimeRange {
        start: "2024-01-01".to_string(),
        end: "2024-01-06".to_string(),
    };

    for step in &steps {
        match step {
            ExecutionStep::CreateTemp { name, sql } => {
                let filtered = smelt_cli::inject_time_filter(sql, &event_time_col, &range).unwrap();
                let create_sql = format!("CREATE TEMP TABLE {} AS {}", name, filtered);
                backend.execute_sql(&create_sql).await.unwrap();
            }
            ExecutionStep::FinalQuery { sql } => {
                let _ = backend
                    .execute_sql("DROP TABLE IF EXISTS main.cube_metrics")
                    .await;
                backend
                    .execute_sql(&format!("CREATE TABLE main.cube_metrics AS {}", sql))
                    .await
                    .unwrap();
            }
            ExecutionStep::DropTemp { name } => {
                let _ = backend
                    .execute_sql(&format!("DROP TABLE IF EXISTS {}", name))
                    .await;
            }
            _ => {}
        }
    }

    // Compare composed result with naive
    let diff = backend
        .execute_sql(
            r#"
            SELECT COUNT(*) as diff_count FROM (
                SELECT event_date, country, unique_users, unique_sessions, total_events, total_revenue
                FROM main.naive_composed
                EXCEPT
                SELECT event_date, country, unique_users, unique_sessions, total_events, total_revenue
                FROM main.cube_metrics
            ) t
        "#,
        )
        .await
        .unwrap();

    let diff_count = diff[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    assert_eq!(
        diff_count, 0,
        "Composed cube+incremental result differs from naive"
    );
}

// ─── Mandatory Time Range Validation ────────────────────────────────

#[test]
fn test_mandatory_time_range_detection() {
    let model_sql = r#"---
materialized: table
incremental:
  partition_column: event_date
  event_time_column: event_time
  granularity: day
---
SELECT date_trunc('day', event_time) as event_date, COUNT(*) as cnt FROM events GROUP BY 1"#;

    let frontmatter = Frontmatter::parse(model_sql).unwrap();
    let model = ModelInfo {
        name: "inc_model".to_string(),
        sql: model_sql.to_string(),
        refs: vec![],
        incremental_config: frontmatter.incremental,
    };

    let mut graph = ModelGraph::new();
    graph.add_model(model);

    let planner = Planner::new();
    let (transformations, errors) = planner.plan(&graph);
    assert!(errors.is_empty());

    // Check that SetIncremental was produced
    let has_incremental = transformations
        .iter()
        .any(|t| matches!(t, smelt_planner::Transformation::SetIncremental { .. }));
    assert!(
        has_incremental,
        "Planner should detect incremental model from frontmatter"
    );
}
