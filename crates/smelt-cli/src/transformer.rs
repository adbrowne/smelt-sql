//! Query transformation helpers.
//!
//! The implementations live in `smelt-runtime`; this module re-exports the
//! public surface so existing internal callers in `smelt-cli` continue to
//! compile via `crate::transformer::*`. New callers should import from
//! `smelt_runtime` directly.

pub use smelt_runtime::transformer::{
    inject_source_filters, inject_time_filter, SourceBound, TimeRange, TransformError,
};
