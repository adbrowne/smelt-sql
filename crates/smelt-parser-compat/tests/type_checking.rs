//! Type checking tests against PostgreSQL
//!
//! These tests verify that smelt's (future) type inference matches PostgreSQL's actual types.
//! They require Docker to be running (uses testcontainers).
//!
//! Run with: cargo test -p smelt-parser-compat --test type_checking -- --ignored
//!
//! The tests are ignored by default because they require Docker.

use proptest::prelude::*;
use smelt_parser_compat::pg_generators;
use testcontainers_modules::{postgres::Postgres, testcontainers::runners::AsyncRunner};
use tokio_postgres::{types::Type, NoTls};

/// Skip type checking tests if Docker is not available
fn docker_available() -> bool {
    std::env::var("SKIP_DOCKER_TESTS").is_err()
}

/// Test schema for type checking
const TEST_SCHEMA: &str = r#"
CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    email VARCHAR(255),
    age INTEGER,
    balance DECIMAL(10, 2),
    active BOOLEAN DEFAULT true,
    created_at TIMESTAMP DEFAULT NOW()
);

CREATE TABLE orders (
    id INTEGER PRIMARY KEY,
    user_id INTEGER REFERENCES users(id),
    amount DECIMAL(10, 2),
    status VARCHAR(20),
    order_date DATE,
    metadata JSONB
);

CREATE TABLE products (
    id INTEGER PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    price DECIMAL(10, 2),
    category VARCHAR(50),
    tags TEXT[]
);
"#;

/// Get column types from a prepared statement
async fn get_query_types(
    client: &tokio_postgres::Client,
    sql: &str,
) -> Result<Vec<Type>, tokio_postgres::Error> {
    let stmt = client.prepare(sql).await?;
    let types: Vec<Type> = stmt.columns().iter().map(|c| c.type_().clone()).collect();
    Ok(types)
}

/// Initialize test database with schema
async fn init_test_db(client: &tokio_postgres::Client) -> Result<(), tokio_postgres::Error> {
    client.batch_execute(TEST_SCHEMA).await
}

#[tokio::test]
#[ignore]
async fn test_simple_select_types() {
    if !docker_available() {
        println!("Skipping: Docker not available");
        return;
    }

    let node = Postgres::default().start().await.unwrap();
    let connection_string = format!(
        "host={} port={} user=postgres password=postgres dbname=postgres",
        node.get_host().await.unwrap(),
        node.get_host_port_ipv4(5432).await.unwrap()
    );

    let (client, connection) = tokio_postgres::connect(&connection_string, NoTls)
        .await
        .unwrap();

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    init_test_db(&client).await.unwrap();

    // Test simple SELECT
    let types = get_query_types(&client, "SELECT id, name, age FROM users")
        .await
        .unwrap();

    assert_eq!(types.len(), 3);
    assert_eq!(types[0], Type::INT4); // id
    assert_eq!(types[1], Type::VARCHAR); // name
    assert_eq!(types[2], Type::INT4); // age
}

#[tokio::test]
#[ignore]
async fn test_aggregate_types() {
    if !docker_available() {
        println!("Skipping: Docker not available");
        return;
    }

    let node = Postgres::default().start().await.unwrap();
    let connection_string = format!(
        "host={} port={} user=postgres password=postgres dbname=postgres",
        node.get_host().await.unwrap(),
        node.get_host_port_ipv4(5432).await.unwrap()
    );

    let (client, connection) = tokio_postgres::connect(&connection_string, NoTls)
        .await
        .unwrap();

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    init_test_db(&client).await.unwrap();

    // COUNT returns BIGINT
    let types = get_query_types(&client, "SELECT COUNT(*) FROM users")
        .await
        .unwrap();
    assert_eq!(types[0], Type::INT8);

    // SUM of INTEGER returns BIGINT
    let types = get_query_types(&client, "SELECT SUM(age) FROM users")
        .await
        .unwrap();
    assert_eq!(types[0], Type::INT8);

    // SUM of DECIMAL returns NUMERIC
    let types = get_query_types(&client, "SELECT SUM(balance) FROM users")
        .await
        .unwrap();
    assert_eq!(types[0], Type::NUMERIC);

    // AVG returns NUMERIC
    let types = get_query_types(&client, "SELECT AVG(age) FROM users")
        .await
        .unwrap();
    assert_eq!(types[0], Type::NUMERIC);
}

#[tokio::test]
#[ignore]
async fn test_cast_types() {
    if !docker_available() {
        println!("Skipping: Docker not available");
        return;
    }

    let node = Postgres::default().start().await.unwrap();
    let connection_string = format!(
        "host={} port={} user=postgres password=postgres dbname=postgres",
        node.get_host().await.unwrap(),
        node.get_host_port_ipv4(5432).await.unwrap()
    );

    let (client, connection) = tokio_postgres::connect(&connection_string, NoTls)
        .await
        .unwrap();

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    init_test_db(&client).await.unwrap();

    // CAST to INTEGER
    let types = get_query_types(&client, "SELECT CAST(balance AS INTEGER) FROM users")
        .await
        .unwrap();
    assert_eq!(types[0], Type::INT4);

    // CAST to VARCHAR
    let types = get_query_types(&client, "SELECT CAST(age AS VARCHAR(10)) FROM users")
        .await
        .unwrap();
    assert_eq!(types[0], Type::VARCHAR);

    // CAST to TEXT
    let types = get_query_types(&client, "SELECT CAST(id AS TEXT) FROM users")
        .await
        .unwrap();
    assert_eq!(types[0], Type::TEXT);
}

#[tokio::test]
#[ignore]
async fn test_expression_types() {
    if !docker_available() {
        println!("Skipping: Docker not available");
        return;
    }

    let node = Postgres::default().start().await.unwrap();
    let connection_string = format!(
        "host={} port={} user=postgres password=postgres dbname=postgres",
        node.get_host().await.unwrap(),
        node.get_host_port_ipv4(5432).await.unwrap()
    );

    let (client, connection) = tokio_postgres::connect(&connection_string, NoTls)
        .await
        .unwrap();

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    init_test_db(&client).await.unwrap();

    // Arithmetic: INT + INT = INT
    let types = get_query_types(&client, "SELECT id + age FROM users")
        .await
        .unwrap();
    assert_eq!(types[0], Type::INT4);

    // Arithmetic: DECIMAL + DECIMAL = NUMERIC
    let types = get_query_types(&client, "SELECT balance + balance FROM users")
        .await
        .unwrap();
    assert_eq!(types[0], Type::NUMERIC);

    // Comparison returns BOOLEAN
    let types = get_query_types(&client, "SELECT age > 18 FROM users")
        .await
        .unwrap();
    assert_eq!(types[0], Type::BOOL);

    // CASE returns type of THEN branches
    let types = get_query_types(
        &client,
        "SELECT CASE WHEN age > 18 THEN 'adult' ELSE 'minor' END FROM users",
    )
    .await
    .unwrap();
    assert_eq!(types[0], Type::TEXT);
}

#[tokio::test]
#[ignore]
async fn test_window_function_types() {
    if !docker_available() {
        println!("Skipping: Docker not available");
        return;
    }

    let node = Postgres::default().start().await.unwrap();
    let connection_string = format!(
        "host={} port={} user=postgres password=postgres dbname=postgres",
        node.get_host().await.unwrap(),
        node.get_host_port_ipv4(5432).await.unwrap()
    );

    let (client, connection) = tokio_postgres::connect(&connection_string, NoTls)
        .await
        .unwrap();

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    init_test_db(&client).await.unwrap();

    // ROW_NUMBER returns BIGINT
    let types = get_query_types(&client, "SELECT ROW_NUMBER() OVER () FROM users")
        .await
        .unwrap();
    assert_eq!(types[0], Type::INT8);

    // RANK returns BIGINT
    let types = get_query_types(&client, "SELECT RANK() OVER (ORDER BY age) FROM users")
        .await
        .unwrap();
    assert_eq!(types[0], Type::INT8);

    // SUM OVER returns same as regular SUM
    let types = get_query_types(&client, "SELECT SUM(age) OVER () FROM users")
        .await
        .unwrap();
    assert_eq!(types[0], Type::INT8);
}

#[tokio::test]
#[ignore]
async fn test_join_types() {
    if !docker_available() {
        println!("Skipping: Docker not available");
        return;
    }

    let node = Postgres::default().start().await.unwrap();
    let connection_string = format!(
        "host={} port={} user=postgres password=postgres dbname=postgres",
        node.get_host().await.unwrap(),
        node.get_host_port_ipv4(5432).await.unwrap()
    );

    let (client, connection) = tokio_postgres::connect(&connection_string, NoTls)
        .await
        .unwrap();

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    init_test_db(&client).await.unwrap();

    // Columns from joined tables retain their types
    let types = get_query_types(
        &client,
        "SELECT u.name, o.amount FROM users u INNER JOIN orders o ON u.id = o.user_id",
    )
    .await
    .unwrap();

    assert_eq!(types[0], Type::VARCHAR);
    assert_eq!(types[1], Type::NUMERIC);
}

// ===== Property-based type checking tests =====

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10))] // Fewer cases since we need Docker

    /// Verify that generated simple SELECTs have consistent types
    #[test]
    #[ignore]
    fn prop_simple_select_valid_types(sql in pg_generators::simple_select()) {
        // This would need a runtime for async, so we just validate the SQL
        // can be generated properly
        assert!(!sql.is_empty());
        assert!(sql.starts_with("SELECT"));
    }
}

/// Test that PostgreSQL type information matches expected patterns
#[tokio::test]
#[ignore]
async fn test_type_catalog() {
    if !docker_available() {
        println!("Skipping: Docker not available");
        return;
    }

    let node = Postgres::default().start().await.unwrap();
    let connection_string = format!(
        "host={} port={} user=postgres password=postgres dbname=postgres",
        node.get_host().await.unwrap(),
        node.get_host_port_ipv4(5432).await.unwrap()
    );

    let (client, connection) = tokio_postgres::connect(&connection_string, NoTls)
        .await
        .unwrap();

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    // Query the type catalog to understand PostgreSQL's type system
    let rows = client
        .query(
            "SELECT typname, oid FROM pg_type WHERE typname IN ('int4', 'int8', 'numeric', 'varchar', 'text', 'bool', 'timestamp', 'date')",
            &[],
        )
        .await
        .unwrap();

    assert!(!rows.is_empty());
    for row in rows {
        let name: String = row.get(0);
        let oid: u32 = row.get(1);
        println!("Type: {} (OID: {})", name, oid);
    }
}

/// Placeholder for future smelt type inference testing
///
/// When smelt has a type checker, this test will:
/// 1. Parse SQL with smelt
/// 2. Run smelt's type inference
/// 3. Compare inferred types with PostgreSQL's actual types
#[tokio::test]
#[ignore]
async fn test_smelt_type_inference_placeholder() {
    if !docker_available() {
        println!("Skipping: Docker not available");
        return;
    }

    // TODO: When smelt has type inference, implement this test
    // let smelt_types = smelt_type_checker::infer_types(&sql);
    // let pg_types = get_query_types(&client, &sql).await.unwrap();
    // assert_types_compatible(&smelt_types, &pg_types);

    println!("Type inference testing not yet implemented - smelt needs a type checker first");
}
