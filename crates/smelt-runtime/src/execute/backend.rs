use std::path::Path;

use anyhow::Result;

use smelt_backend::Backend;

/// Future returned by `BackendFactory::create`. Pinned + boxed so the trait
/// stays object-safe; a `type` alias keeps the trait signature readable.
pub type BackendFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Box<dyn Backend>>> + Send + 'a>>;

/// Backend factory injected by the consumer. The UI and CLI know how to
/// build their backends (DuckDB, Spark, etc.) and may differ in cred
/// resolution / feature gating; the runtime stays agnostic.
pub trait BackendFactory: Send + Sync {
    fn create<'a>(
        &'a self,
        target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        project_dir: &'a Path,
    ) -> BackendFuture<'a>;
}
