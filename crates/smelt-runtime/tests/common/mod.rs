//! Shared live-Trino harness for `smelt-runtime` integration tests
//! (`statement_parity/trino.rs`). Mirrors `crates/smelt-cli/tests/
//! common/mod.rs`'s `trino_env`/`trino_schema`/`trino_backend`/
//! `drop_trino_schema` shape exactly — kept as a separate copy because
//! cargo compiles each crate's `tests/` tree as its own set of binaries
//! with no cross-crate module sharing, but the naming/uniqueness rule
//! itself must not drift (`crates/smelt-cli/tests/trino_ci_wiring.rs::
//! every_live_trino_test_schema_name_comes_from_the_shared_helper` enforces
//! that no individual test FILE within a crate defines its own private
//! copy; this `common/mod.rs` is that crate's single, authorized
//! definition point, per the same convention `smelt-cli` and
//! `smelt-backend-trino` already follow).
#![allow(dead_code)]

use smelt_backend::Backend;
use smelt_backend_trino::{TrinoBackend, TrinoClientConfig};

pub struct TrinoEnv {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub catalog: String,
    pub tls: bool,
}

/// Reads `SMELT_TRINO_URL` (`scheme://host[:port]`) plus the optional
/// `SMELT_TRINO_USER`/`SMELT_TRINO_CATALOG` overrides. `None` when
/// `SMELT_TRINO_URL` is unset, meaning the caller should skip.
pub fn trino_env() -> Option<TrinoEnv> {
    let url = std::env::var("SMELT_TRINO_URL").ok()?;
    let user = std::env::var("SMELT_TRINO_USER").unwrap_or_else(|_| "smelt".to_string());
    let catalog = std::env::var("SMELT_TRINO_CATALOG").unwrap_or_else(|_| "iceberg".to_string());
    let tls = url.starts_with("https://");
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(&url);
    let (host, port_str) = rest
        .split_once(':')
        .unwrap_or((rest, if tls { "443" } else { "8080" }));
    let port: u16 = port_str
        .parse()
        .unwrap_or_else(|e| panic!("SMELT_TRINO_URL has an unparseable port '{port_str}': {e}"));
    Some(TrinoEnv {
        host: host.to_string(),
        port,
        user,
        catalog,
        tls,
    })
}

/// Process-local counter backing [`trino_schema`]'s uniqueness: a monotonic
/// counter guarantees no two calls in this process ever collide, and the
/// nanosecond entropy suffix guards against two *processes* racing on the
/// same counter value.
static TRINO_SCHEMA_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A schema name unique to this run: guaranteed unique within this process
/// via [`TRINO_SCHEMA_COUNTER`], and overwhelmingly likely unique across
/// concurrent processes via the nanosecond entropy suffix. `label` scopes
/// two suites in the same binary apart.
pub fn trino_schema(label: &str) -> String {
    let n = TRINO_SCHEMA_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!(
        "smelt_stmt_parity_{label}_{}_{n}_{nanos}",
        std::process::id()
    )
}

/// Connects a `TrinoBackend` to `schema` from the ambient test environment.
/// Panics if `SMELT_TRINO_URL` is unset — callers gate on [`trino_env`]
/// first.
pub fn trino_backend(schema: &str) -> TrinoBackend {
    let env = trino_env().expect("SMELT_TRINO_URL must be set to connect to Trino");
    let scheme = if env.tls { "https" } else { "http" };
    TrinoBackend::new(TrinoClientConfig {
        base_url: format!("{scheme}://{}:{}", env.host, env.port),
        user: env.user,
        catalog: env.catalog,
        schema: schema.to_string(),
        password: None,
    })
}

/// Drops the suite's Trino schema. Best-effort — a failure here should not
/// fail a test whose assertions already ran.
pub async fn drop_trino_schema(schema: &str) {
    let Some(env) = trino_env() else { return };
    let backend = trino_backend(schema);
    let _ = backend
        .execute_sql(&format!(
            "DROP SCHEMA IF EXISTS \"{}\".\"{schema}\" CASCADE",
            env.catalog
        ))
        .await;
}
