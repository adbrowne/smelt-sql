use smelt_backends::create_backend;
use smelt_core::config::Target;

fn unknown_type_target() -> Target {
    Target {
        target_type: "not_a_backend".to_string(),
        database: Some("test.db".to_string()),
        schema: "main".to_string(),
        connect_url: None,
        catalog: None,
        warehouse: None,
        format: None,
        settings: None,
        project: None,
        dataset: None,
        location: None,
    }
}

#[tokio::test]
async fn unknown_backend_type_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let target = unknown_type_target();
    let result = create_backend("unknown_test", &target, dir.path(), None).await;
    let err = result
        .err()
        .expect("Expected unknown backend type to be rejected with an error");
    let msg = err.to_string();
    assert!(
        msg.contains("not_a_backend"),
        "Expected error message to name the unknown type, got: {msg}"
    );
}

fn duckdb_target(db_name: &str) -> Target {
    Target {
        target_type: "duckdb".to_string(),
        database: Some(db_name.to_string()),
        schema: "main".to_string(),
        connect_url: None,
        catalog: None,
        warehouse: None,
        format: None,
        settings: None,
        project: None,
        dataset: None,
        location: None,
    }
}

#[tokio::test]
async fn creates_duckdb_backend_from_duckdb_target() {
    let dir = tempfile::tempdir().unwrap();
    let target = duckdb_target("test.db");
    let result = create_backend("test", &target, dir.path(), None).await;
    assert!(
        result.is_ok(),
        "Expected DuckDB backend creation to succeed, got: {:?}",
        result.err()
    );
}

#[tokio::test]
#[cfg(feature = "spark")]
async fn creates_spark_backend_from_spark_target() {
    let url = match std::env::var("SPARK_CONNECT_URL") {
        Ok(u) => u,
        Err(_) => return, // skip when no server available
    };
    let dir = tempfile::tempdir().unwrap();
    let target = Target {
        target_type: "spark".to_string(),
        database: None,
        schema: "default".to_string(),
        connect_url: Some(url),
        catalog: None,
        warehouse: None,
        format: None,
        settings: None,
        project: None,
        dataset: None,
        location: None,
    };
    let result = create_backend("spark_test", &target, dir.path(), None).await;
    assert!(
        result.is_ok(),
        "Expected Spark backend creation to succeed, got: {:?}",
        result.err()
    );
}
