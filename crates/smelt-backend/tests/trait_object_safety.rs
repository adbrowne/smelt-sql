//! Compile-time check that `&dyn Backend` is object-safe with `load_table` in the trait.
//!
//! This test has no runtime assertions — if it compiles, the trait is object-safe.

use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use smelt_backend::Backend;
use std::sync::Arc;

/// Coerce a concrete backend reference to `&dyn Backend`.
///
/// If `load_table` introduces any generic parameter or `impl Trait` argument,
/// this will fail to compile with "the trait `Backend` cannot be made into an
/// object" and the test suite will report a build error.
fn _coerce_to_dyn(_b: &dyn Backend) {}

/// Accept a `Box<dyn Backend>` — this proves the trait is object-safe.
fn _accept_boxed(_b: Box<dyn Backend>) {}

/// Call `load_table` through a `&dyn Backend` reference.
///
/// The async block is never polled; we only verify it compiles.
fn _call_load_table_via_dyn(b: &dyn Backend, schema: SchemaRef, batches: Vec<RecordBatch>) {
    // Forming this future is enough to prove object-safety at compile time.
    // Drop it immediately — we never want to poll it here.
    drop(b.load_table("main", "t", schema, batches));
}

#[test]
fn backend_is_object_safe() {
    // The presence of the functions above (compiled into this binary) is the
    // proof.  If any of them fail to compile, the test binary won't link and
    // the CI gate will catch it.  No runtime assertions are needed.
    let _schema: SchemaRef = Arc::new(Schema::new(vec![Field::new("x", DataType::Int32, true)]));
    // Passes trivially — all proofs are in the compile step above.
}
