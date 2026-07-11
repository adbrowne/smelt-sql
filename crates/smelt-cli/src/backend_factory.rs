//! CLI `BackendFactory` implementation.
//!
//! Wraps `smelt_backends::create_backend` into the `smelt_runtime::BackendFactory`
//! trait so `commands/run.rs` can delegate backend creation to `execute_project`.

use std::path::{Path, PathBuf};

use smelt_core::config::Target;
use smelt_runtime::execute::{BackendFactory, BackendFuture};

/// A `BackendFactory` that delegates to `smelt_backends::create_backend`.
///
/// An optional `database_override` lets `smelt run --database <path>` redirect
/// DuckDB output to a different file without touching the config.
pub struct CliBackendFactory {
    pub database_override: Option<PathBuf>,
}

impl BackendFactory for CliBackendFactory {
    fn create<'a>(
        &'a self,
        target_name: &'a str,
        target_config: &'a Target,
        project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        let database_override = self.database_override.clone();
        Box::pin(async move {
            smelt_backends::create_backend(
                target_name,
                target_config,
                project_dir,
                database_override,
            )
            .await
        })
    }
}
