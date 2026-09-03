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
pub mod diagnostics_boundary;
pub mod hover;
pub mod notifications;
mod python_scan;
pub mod rename_lambda;

// Re-exports for the binary and integration tests.
pub use backend::Backend;
pub use completion::{determine_completion_context, CompletionContext};
pub use hover::{
    // Phase E2 — re-exported for integration tests that verify the helper text
    // produced by the Backend hover/completion/goto-def dispatch.
    completion_for_generates_value,
    completion_for_model_def_field_key,
    goto_def_for_emitted_model_reference,
    hover_text_for_generates_frontmatter,
    hover_text_for_model_def_body_field_value,
    hover_text_for_model_def_literal_open_brace,
    hover_text_for_model_def_name_field_value,
    hover_text_for_model_def_optional_field_value,
    hover_text_for_source_clamp,
    passing_body_aggregate_labels,
    passing_body_completion_columns,
    render_expansion_frames,
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
