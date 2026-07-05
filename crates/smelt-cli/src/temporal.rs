//! Temporal window computation — shim over `smelt_runtime::windowing`.
//!
//! The shared logic (batch production, filter widening, window alignment,
//! and run-window-vs-partition-granularity validation) lives in
//! `smelt-runtime` so both CLI and UI consume the same implementation. This
//! module re-exports the canonical types.

pub use smelt_runtime::windowing::{
    compute_incremental_windows, validate_run_window_against_partition_grid,
    validate_run_window_alignment, EffectiveWindow, IncrementalBatch, IncrementalWindows,
};
