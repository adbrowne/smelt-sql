//! SQL compiler — pipeline that resolves `smelt.<path>` references, expands
//! `smelt.fn.*` calls, inlines ephemeral CTEs, applies type casts, and prints
//! per-dialect SQL.
//!
//! The implementations live in `smelt-runtime`; this module re-exports the
//! public surface so existing internal callers in `smelt-cli` continue to
//! compile via `crate::compiler::*`. New callers should import from
//! `smelt_runtime` directly.

pub use smelt_runtime::{
    bind_named_args, build_fn_body_map, build_fn_body_map_from_model_files, build_source_bound_map,
    prepend_ephemeral_ctes, resolve_refs_in_sql, substitute_params_with_named, CompiledModel,
    CompilerRegistry, EphemeralResolver, FnBodyMap, SqlCompiler, UpstreamSchemas,
};
