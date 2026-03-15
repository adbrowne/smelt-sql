use crate::harness::{BuildMetrics, ParserMetrics, SalsaMetrics};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Combined benchmark result with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    /// Git commit hash.
    pub git_commit: String,
    /// Git branch name.
    pub git_branch: String,
    /// ISO 8601 timestamp.
    pub timestamp: String,
    /// Rust compiler version.
    pub rust_version: String,
    /// Total model count.
    pub model_count: usize,
    /// Build pipeline metrics.
    pub build: BuildMetrics,
    /// Salsa incremental compilation metrics.
    pub salsa: SalsaMetrics,
    /// Parser throughput metrics.
    pub parser: ParserMetrics,
}

impl BenchmarkResult {
    /// Write results to a JSON file in the given directory.
    pub fn save_to_dir(&self, dir: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir)?;

        let short_commit = if self.git_commit.len() >= 7 {
            &self.git_commit[..7]
        } else {
            &self.git_commit
        };

        let filename = format!("{}_{}.json", self.timestamp.replace(':', "-"), short_commit);
        let path = dir.join(filename);

        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;

        eprintln!("Saved benchmark results to {}", path.display());
        Ok(())
    }
}

/// Collect git metadata from the current repository.
pub fn git_commit() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Get the current git branch.
pub fn git_branch() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Get the Rust compiler version.
pub fn rust_version() -> String {
    std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Get current timestamp in ISO 8601 format.
pub fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
