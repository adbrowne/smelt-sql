//! Execute a Python model file via a subprocess (`python -m smelt.runner`) —
//! the default path when the `python` feature (embedded PyO3) is off.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Command, Stdio};

use smelt_core::python_utils;

use super::PythonModelOutput;

/// Execute a Python model file and return the generated SQL models (subprocess path).
pub(super) fn run_python_model(
    python: &str,
    file_path: &Path,
    project_context_json: &str,
    python_sdk_path: &Path,
) -> Result<Vec<PythonModelOutput>> {
    let pythonpath = python_utils::build_pythonpath(python_sdk_path, file_path);
    // Pass context via stdin to avoid OS argument size limits (E2BIG).
    let mut child = Command::new(python)
        .arg("-m")
        .arg("smelt.runner")
        .arg(file_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PYTHONPATH", pythonpath)
        .spawn()
        .with_context(|| format!("Failed to execute Python model: {}", file_path.display()))?;
    let pid = child.id();
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(project_context_json.as_bytes())
            .with_context(|| {
                format!(
                    "Failed to write context to Python model: {} (pid {pid})",
                    file_path.display()
                )
            })?;
    }
    let output = child.wait_with_output().with_context(|| {
        format!(
            "Failed to execute Python model: {} (pid {pid})",
            file_path.display()
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // The pid is included so a misattributed traceback (this file's error
        // context paired with another concurrently-spawned child's stderr) is
        // diagnosable rather than silently confusing — see issue #189.
        return Err(anyhow::anyhow!(
            "Python model error in {} (pid {pid}):\n{}",
            file_path.display(),
            stderr
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).with_context(|| {
        format!(
            "Failed to parse Python model output from {}: {}",
            file_path.display(),
            stdout
        )
    })
}
