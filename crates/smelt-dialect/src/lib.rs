//! SQL dialect definitions, backend capabilities, and dialect-aware CST printer.
//!
//! This crate provides lightweight, sync-only types that can be used by any
//! consumer (LSP, CLI, optimizer) without pulling in heavy async/native deps.

mod dialect;

pub use dialect::{BackendCapabilities, SqlDialect};
