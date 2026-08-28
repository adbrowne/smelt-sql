//! SQL dialect definitions, backend capabilities, and dialect-aware CST printer.
//!
//! This crate provides lightweight, sync-only types that can be used by any
//! consumer (LSP, CLI, optimizer) without pulling in heavy async/native deps.

mod dialect;
mod emission_check;
pub mod position;
mod printer;
pub mod restructure;
mod type_conformance;

pub use dialect::{BackendCapabilities, SqlDialect};
pub use emission_check::{unsupported_emissions, UnsupportedEmission};
pub use position::classify as classify_position;
pub use printer::{
    print, AsStructEmitter, PrintContext, SmeltFnExpander, SmeltPathCallExpander,
    SmeltPathRefResolver,
};
pub use restructure::{plan as plan_restructure, RestructurePlan};
pub use type_conformance::{wrap_with_type_casts, TYPE_CAST_WRAP_ALIAS};
