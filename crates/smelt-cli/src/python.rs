//! Python model discovery — thin re-export wrapper over `smelt-runtime`.
//!
//! The discovery implementation now lives in `smelt_runtime::python` so both
//! the CLI and UI consume it through the shared pipeline (Run Pipeline Parity
//! rule). This module re-exports the public API so existing callers in this
//! crate are unaffected.

pub use smelt_core::PythonModelQuery;
pub use smelt_runtime::discover_python_models;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::ModelDiscovery;
    use std::path::Path;

    // ── CLI re-export regression tests ────────────────────────────────────────
    //
    // These tests verify that `smelt_cli::discover_python_models` (which now
    // delegates to `smelt_runtime::discover_python_models`) produces correct
    // results — i.e., the re-export chain is byte-identical to calling the
    // runtime function directly.

    /// `cli_python_unchanged` — the CLI entry point (`smelt_cli::discover_python_models`,
    /// which is a re-export of `smelt_runtime::discover_python_models`) produces
    /// the same model set as before the migration.
    ///
    /// Uses the subprocess path (no PyO3 required) against the real Python SDK.
    #[test]
    fn cli_python_unchanged() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();

        // Create python SDK
        let sdk_dir = project_dir.join("python").join("smelt");
        std::fs::create_dir_all(&sdk_dir).unwrap();

        let repo_sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("python")
            .join("smelt");

        for entry in std::fs::read_dir(&repo_sdk).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                std::fs::copy(entry.path(), sdk_dir.join(entry.file_name())).unwrap();
            }
        }

        let models_dir = project_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        // Python model with a single @model function — stem "gen", function "cli_model".
        // Expected address_segments = ["gen", "cli_model"] (D-26 path-derived).
        std::fs::write(
            models_dir.join("gen.py"),
            r#"
from smelt import model

@model
def cli_model(project):
    return "SELECT 42 as answer"
"#,
        )
        .unwrap();

        let discovery = ModelDiscovery::new(project_dir.to_path_buf(), vec!["models".to_string()]);
        let sql_models = discovery.discover_models().unwrap();
        let python_files = discovery.discover_python_files().unwrap();
        assert_eq!(python_files.len(), 1, "expected 1 Python file");

        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            paths: vec!["models".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
            target: None,
            state: Default::default(),
            maintenance: None,
        };

        // Call via CLI's re-export (now delegates to smelt-runtime)
        let python_models =
            discover_python_models(&python_files, &sql_models, &config, project_dir, None).unwrap();

        assert_eq!(python_models.len(), 1, "CLI re-export must find the model");
        assert_eq!(python_models[0].name, "cli_model");
        assert!(python_models[0].content.contains("SELECT 42"));
        // D-26: path-derived address
        assert_eq!(
            python_models[0].address_segments,
            vec!["gen", "cli_model"],
            "address_segments must be path-derived via runtime"
        );
    }

    /// Regression: `discover_python_models` with no Python files returns empty vec.
    #[test]
    fn cli_python_empty_files_returns_empty() {
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path();
        let config = crate::config::Config {
            name: "test".to_string(),
            version: 1,
            paths: vec!["models".to_string()],
            targets: std::collections::HashMap::new(),
            default_materialization: crate::config::Materialization::View,
            models: std::collections::HashMap::new(),
            python: None,
            target: None,
            state: Default::default(),
            maintenance: None,
        };
        let result = discover_python_models(&[], &[], &config, project_dir, None).unwrap();
        assert!(result.is_empty());
    }
}
