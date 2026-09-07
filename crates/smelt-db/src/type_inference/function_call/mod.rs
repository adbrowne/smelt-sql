//! Function call and smelt-path-call type inference (incl. registry-driven inference and AS_STRUCT).
//!
//! Split into three cohesive submodules:
//! - [`smelt_path`] — `smelt.path.call(...)` inference (config vars, models/sources
//!   reflection, generic signature lookup, `AS_STRUCT`).
//! - [`registry`] — registry-first inference for the [`REGISTRY_MIGRATED`](registry::REGISTRY_MIGRATED)
//!   allowlist, plus the grouped-nullability helpers it depends on.
//! - [`legacy`] — the hand-written `match` over [`smelt_types::SqlFunction`] for
//!   everything not yet migrated to the registry.

mod legacy;
mod registry;
mod smelt_path;

pub use legacy::infer_function_type;
pub use registry::registry_migrated_names;
pub use smelt_path::{infer_as_struct_type, infer_smelt_path_call_type};
