//! Type inference for SQL expressions.
//!
//! This module provides type inference capabilities for SQL expressions,
//! including literals, column references, CAST expressions, aggregates,
//! higher-order functions, records, loaders, and reflection witnesses.
//!
//! # Pure-function rule
//!
//! Every item in this module tree is a **pure function** — no Salsa imports,
//! no `#[salsa::tracked]`. Callers (`crate::lib`, `crate::queries`,
//! `planner`) build inputs via Salsa queries and pass them in as plain
//! data structures. This invariant enables a future `smelt-check` crate
//! extraction.

pub mod binary;
pub mod collation;
pub mod composite;
pub mod dispatch;
pub mod function_call;
pub mod hof;
pub mod literal;
pub mod loader_and_reflection;
pub mod multi_model;
pub mod pipe;
pub mod record;
pub mod subquery;
pub mod ternary;
pub mod type_context;
pub mod values;

#[cfg(test)]
mod tests;

// ─── Internal helpers shared across submodules ──────────────────────────────
// `pub(super)` re-exports here let any sibling submodule reach into another
// without writing `super::<sibling>::<item>` at each call site. All originally
// private helpers in `type_inference.rs` that crossed family boundaries are
// listed here.

pub(super) use binary::infer_binary_expr_type;
pub(super) use composite::{
    infer_array_literal_type, infer_array_slice_type, infer_array_subscript_type,
    infer_row_constructor_type, infer_struct_literal_type,
};
pub(super) use function_call::{
    infer_as_struct_type, infer_function_type, infer_smelt_path_call_type,
};
pub(super) use literal::{
    infer_case_expr_type, infer_cast_type, infer_extract_type, infer_literal_type,
};
pub(super) use subquery::infer_subquery_type;

// ─── Public surface re-exports ───────────────────────────────────────────────

pub use type_context::TypeContext;

pub use dispatch::{
    check_mixed_tz_case_diagnostics, check_mixed_tz_setop_diagnostics,
    check_window_in_scalar_contexts, infer_expression_kind, infer_expression_type,
    infer_select_column_types, infer_select_output_schema, promote_types,
    WindowInScalarContextInfo,
};

pub use binary::{
    check_crossfamily_arithmetic_diagnostics, check_decimal_division_diagnostics,
    check_decimal_precision_overflow_diagnostics, check_mixed_tz_arithmetic_diagnostics,
    check_undeclared_columns, infer_cte_columns, walk_expression_columns,
    walk_expression_columns_with_visitor, walk_select_columns, walk_select_columns_with_visitor,
    ColumnRefVisitor, UndeclaredColumnInfo,
};

pub use subquery::build_subquery_context;

pub use hof::{
    check_define_name_shadowing, check_forbidden_position_spreads, check_hof_position_diagnostics,
    check_select_list_spreads, disambiguate_list_literal, expand_spread_into_position,
    infer_hof_call, infer_hof_call_from_function_call,
    infer_hof_call_from_function_call_with_expected, infer_list_literal,
    infer_parameterised_reducer_call, infer_pipe_expr, infer_reduce_call,
    list_literal_sentinels_to_diagnostics, lookup_parameterised_reducer, lookup_reducer,
    EmptyIdentity, HofInferResult, HofInferSentinel, HofKind, HofSecondArg, ListDisambiguation,
    ListInferSentinel, ListLiteralInferResult, OriginTag, ParameterisedReducerResult,
    ParameterisedReducerSentinel, ParameterisedReducerSpec, ReducerInputConstraint,
    ReducerOutputSort, ReducerSpec, SelectListSpreadResult, SplicePosition, SynthesizedReason,
    PARAMETERISED_REDUCER_REGISTRY, REDUCER_REGISTRY,
};

pub use loader_and_reflection::{
    check_column_ref_field_diagnostics, check_columns_of_diagnostics,
    check_config_var_call_diagnostics, check_loader_call_diagnostics, check_loader_path,
    check_model_ref_source_ref_field_diagnostics, check_wide_reflection_diagnostics,
    infer_field_on_column_ref, infer_field_on_model_ref, infer_field_on_source_ref,
    infer_loader_call_smelt_type, is_compile_time_text_arg, schema_arg_text_is_admissible,
    LoaderPathOutcome,
};

pub use record::{
    check_meta_text_lift_diagnostics, check_record_in_data_world, check_record_literal,
    infer_map_method_call, infer_record_field_projection, is_meta_text_value,
    record_registry_for_workspace, registry_code_to_diagnostic_code, validate_map_type_expression,
    MapCallArg, MapMethodCallResult, MetaTextLiftPosition, RecordFieldProjectionResult,
    RecordLiteralResult, RecordLiteralSentinel, StaticResolution,
};

pub use multi_model::{
    check_generator_body_reflection_forbid, infer_generator_file_body, infer_model_def_literal,
    validate_model_def_materialization, validate_model_def_name, GeneratorBodyInferResult,
    ModelDefLiteralInferResult, MultiModelSentinel,
};

pub use ternary::{
    check_dangling_ternary_keywords, check_ternary_expr_diagnostics, infer_ternary_type,
    ShortCircuitHint, TernaryResult, TernarySentinel,
};

pub use values::{
    check_cte_alias_arity, check_mixed_temporal_values_diagnostics, check_table_ref_values_arity,
    infer_values_columns, ValuesError,
};

pub use collation::{check_collation_diagnostics, infer_collate_expr_type};

pub use pipe::{check_pipe_undeclared_columns, infer_pipe_stage_output_schema};
