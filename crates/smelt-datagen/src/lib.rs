//! Deterministic data generation for smelt.
//!
//! This crate provides proptest-inspired composable generators for creating
//! test data with deterministic output based on a seed value.

pub mod config;
pub mod gen;
pub mod generators;
pub mod generic;
pub mod generic_parquet;
pub mod parquet;
pub mod session;

pub use gen::Gen;
pub use generators::*;
pub use session::{
    generate_day_seeds, DayGenerator, Session, SessionGenerator, Visitor, VisitorPool,
};
