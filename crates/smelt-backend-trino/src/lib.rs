//! A pure-Rust HTTP client and `Backend` implementation for Trino's
//! `/v1/statement` protocol over an Iceberg REST catalog.
//!
//! `client`/`arrow_convert`/`config`/`error`/`protocol` are the HTTP client
//! (phase 5 of `docs/outcomes/20260913-trino-target-spine/`); `backend` is
//! the `Backend` trait implementation (phase 6) driving that client against
//! DDL, existence, row count, preview and `execute_model`.

pub mod arrow_convert;
pub mod backend;
pub mod client;
pub mod config;
pub mod error;
pub mod protocol;

pub use backend::TrinoBackend;
pub use client::TrinoClient;
pub use config::TrinoClientConfig;
