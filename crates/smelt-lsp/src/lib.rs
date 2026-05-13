//! Smelt LSP server.
//!
//! Submodules:
//! - [`db_helpers`] — thin path→input lookups over the salsa DB.
//! - [`column_resolution`] — column tracing for goto-def and hover.
//! - [`hover`] — pure formatters for hover/goto/completion data.
//! - [`completion`] — completion context detection.
//! - [`backend`] — `Backend` struct and `LanguageServer` impl.
//! - [`python_scan`] — Python model scanning and caching.

mod backend;
mod column_resolution;
mod completion;
mod db_helpers;
mod hover;
mod python_scan;

// Re-exports for the binary and integration tests.
pub use backend::Backend;
pub use completion::{determine_completion_context, CompletionContext};
pub use hover::{
    passing_body_aggregate_labels, passing_body_completion_columns, render_expansion_frames,
};

// In-crate re-exports so the unit-test module (`#[cfg(test)] mod tests;`)
// can keep its existing `use super::*;` pattern without juggling
// submodule paths.
#[cfg(test)]
pub(crate) use completion::*;
#[cfg(test)]
pub(crate) use hover::*;

#[cfg(test)]
mod tests;
