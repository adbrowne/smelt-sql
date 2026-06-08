//! Schema evolution helpers — re-exported from `smelt_runtime::schema_evolution`.
//!
//! The implementation was moved to `smelt-runtime` so that `execute_project`
//! can run the schema-evolution gate without a dependency on `smelt-cli`.
//! This shim preserves the existing public API for any callers that still
//! reach into `smelt_cli::migration::*`.
pub use smelt_runtime::schema_evolution::*;
