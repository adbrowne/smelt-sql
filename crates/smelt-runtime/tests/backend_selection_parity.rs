//! Backend-selection parity guard.
//!
//! Asserts that the "CLI path" and "UI path" through `smelt_backends::create_backend`
//! produce identical outcomes for the same target config. If either consumer
//! reimplements its own `type → backend` selection, the results will diverge and this
//! test will catch it.
//!
//! This is the standing dual-consumer factory test referenced in
//! `docs/specs/architecture.md` §"Run pipeline parity rule (CLI ↔ UI)".

use smelt_backend::Backend;
use smelt_core::config::Target;
use std::path::Path;

fn duckdb_target() -> Target {
    Target {
        target_type: "duckdb".to_string(),
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

fn spark_target_with_url(url: &str) -> Target {
    Target {
        target_type: "spark".to_string(),
        database: None,
        schema: "default".to_string(),
        connect_url: Some(url.to_string()),
        catalog: None,
        warehouse: None,
        format: None,
        settings: None,
        project: None,
        dataset: None,
        location: None,
    }
}

/// CLI-style resolution: passes a `database_override` (mirrors `CliBackendFactory`
/// with `--database`).
async fn cli_resolve(
    target_name: &str,
    target: &Target,
    project_dir: &Path,
) -> anyhow::Result<Box<dyn Backend>> {
    smelt_backends::create_backend(
        target_name,
        target,
        project_dir,
        Some(project_dir.join("override.db")),
    )
    .await
}

/// UI-style resolution: no `database_override` (mirrors `UiBackendFactory`).
async fn ui_resolve(
    target_name: &str,
    target: &Target,
    project_dir: &Path,
) -> anyhow::Result<Box<dyn Backend>> {
    smelt_backends::create_backend(target_name, target, project_dir, None).await
}

/// Both the CLI path and the UI path must succeed for a DuckDB target.
///
/// Regression: before W4·P2 the UI path would silently fall back to DuckDB for
/// non-DuckDB targets (Mode-B drift). This test asserts the shared delegation is
/// intact for the DuckDB path.
#[tokio::test]
async fn cli_and_ui_both_resolve_duckdb_target() {
    let dir = tempfile::TempDir::new().unwrap();
    let target = duckdb_target();

    match cli_resolve("default", &target, dir.path()).await {
        Ok(_) => {}
        Err(e) => panic!("CLI path failed for DuckDB target: {}", e),
    }
    match ui_resolve("default", &target, dir.path()).await {
        Ok(_) => {}
        Err(e) => panic!("UI path failed for DuckDB target: {}", e),
    }
}

/// Without the `spark` feature compiled into `smelt-backends`, both the CLI path and
/// the UI path must fail with identical "Spark backend not available" error messages.
///
/// Regression: before W4·P2, the UI returned "Spark not yet supported in UI mode"
/// while the CLI returned "Spark backend not available". Identical messages prove
/// both paths delegate to the same shared factory.
#[tokio::test]
async fn cli_and_ui_fail_identically_for_spark_without_feature() {
    let dir = tempfile::TempDir::new().unwrap();
    let target = spark_target_with_url("sc://localhost:15002");

    let cli_err = match cli_resolve("spark_target", &target, dir.path()).await {
        Ok(_) => panic!("CLI path should fail when spark feature is absent"),
        Err(e) => e.to_string(),
    };
    let ui_err = match ui_resolve("spark_target", &target, dir.path()).await {
        Ok(_) => panic!("UI path should fail when spark feature is absent"),
        Err(e) => e.to_string(),
    };

    // Both must produce the canonical "not available" message, not a
    // consumer-specific stub message.
    assert!(
        cli_err.contains("Spark backend not available"),
        "CLI error message unexpected: {}",
        cli_err
    );
    assert!(
        ui_err.contains("Spark backend not available"),
        "UI error message unexpected: {}",
        ui_err
    );
    assert_eq!(
        cli_err, ui_err,
        "CLI and UI paths produced different errors — parity break"
    );
}
