//! A pure-Rust HTTP client for Trino's `/v1/statement` protocol.
//!
//! This crate is the client half of the Trino backend (phase 5 of
//! `docs/outcomes/20260913-trino-target-spine/`): request submission,
//! `nextUri` paging, result-page decoding to Arrow, and typed error
//! mapping. It has no `Backend` trait implementation — that is phase 6.

pub mod arrow_convert;
pub mod client;
pub mod config;
pub mod error;
pub mod protocol;

pub use client::TrinoClient;
pub use config::TrinoClientConfig;
