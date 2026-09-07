//! `smelt.path.call(...)` inference: config vars, models/sources reflection,
//! generic function-signature lookup, and `AS_STRUCT`.

use smelt_parser::ast::{SmeltAsStructCall, SmeltPathCall};
use smelt_types::signatures::{SmeltType, TypeConstraint};
use smelt_types::{DataType, TypedColumn};

use crate::type_inference::type_context::TypeContext;

/// Infer the return type of a `smelt.functions.<name>(...)` call site.
///
/// Uses the workspace function-signature index seeded on [`TypeContext`]
/// (via [`TypeContext::add_function_signature`]) — no Salsa access. Returns:
///   - `Some(TypedColumn)` with the declared return type when the signature
///     resolves and carries a `-> Expr<Concrete(T)>` annotation.
///   - `Some(TypedColumn { data_type: Double, .. })` when the return is
///     `Expr<Numeric>` — matches `param_binding_type`'s widening rule in
///     `function_body_check.rs` so callers doing `CAST(... AS DOUBLE) /
///     safe_divide(...)` stay well-typed.
///   - `Some(TypedColumn { data_type: Unknown, .. })` for `Expr<Any>`,
///     malformed annotations, or missing annotations — diagnostic emission
///     is the call-site checker's job, not inference's.
///   - `None` only when the function cannot be resolved in this context.
pub fn infer_smelt_path_call_type(call: &SmeltPathCall, ctx: &TypeContext) -> Option<TypedColumn> {
    let segments = call.segments();

    // Phase B rule 10: `smelt.config.var(...)` always synthesises nullable
    // `Varchar` (Text).  The value is sourced from CLI / env / YAML at
    // compile time and may be absent when no default is provided — hence
    // nullable.  This must be handled before the generic signature lookup
    // because "var" is not in the function-signature index.
    if segments.len() >= 2
        && segments[segments.len() - 2].eq_ignore_ascii_case("config")
        && segments[segments.len() - 1].eq_ignore_ascii_case("var")
    {
        return Some(TypedColumn::nullable(DataType::Varchar {
            max_length: None,
        }));
    }

    // Phase D: `smelt.models.<accessor>(...)` and `smelt.sources.<accessor>(...)`.
    // Both `with_tag` and `all` return `List<ModelRef|SourceRef>`. We synthesise
    // `Unknown` (the DataType projection of a meta-list) — the `SmeltType` is
    // resolved at the HOF inference layer. Unknown / miss accessors also return
    // Unknown (the error is emitted by `check_wide_reflection_diagnostics`).
    // Use segments() here (IDENT-only) since "models" and "sources" are plain identifiers.
    // "all" is a keyword so segments().len() == 1 for `smelt.models.all`, but we detect
    // `models`/`sources` as the first segment regardless.
    {
        // Check first segment (always an IDENT) for "models" or "sources".
        let first_seg = segments.first();
        if first_seg
            .map(|s| s.eq_ignore_ascii_case("models") || s.eq_ignore_ascii_case("sources"))
            .unwrap_or(false)
        {
            return Some(TypedColumn::nullable(DataType::Unknown(
                smelt_types::UnknownReason::Dynamic,
            )));
        }
    }

    let name = segments.last()?;
    let sig = ctx.lookup_function_signature(name)?;

    let dt = match &sig.return_type {
        Some(Ok(SmeltType::Expr(TypeConstraint::Concrete(dt)))) => dt.clone(),
        Some(Ok(SmeltType::Expr(TypeConstraint::Numeric))) => DataType::Double,
        // `Ordered` (Phase 7) is only reachable via generics in v1 signatures
        // (§16 #14) — Phase 8 adds the inference machinery. In the monomorphic
        // `smelt.define` path we stay conservative: no precise return type
        // known yet, surface `Unknown` like `Any`.
        Some(Ok(SmeltType::Expr(TypeConstraint::Ordered))) => {
            DataType::Unknown(smelt_types::UnknownReason::Dynamic)
        }
        Some(Ok(SmeltType::Expr(TypeConstraint::Any))) => {
            DataType::Unknown(smelt_types::UnknownReason::Dynamic)
        }
        // `TableExpr` return (Phase 15) — scalar inference has no
        // DataType for a whole row set. Downstream Phase 17 plumbs the
        // inferred output schema; for now the call-site sees an opaque
        // Unknown.
        Some(Ok(SmeltType::TableExpr(_))) => DataType::Unknown(smelt_types::UnknownReason::Dynamic),
        // `SelectItems<Kind>` (Phase 21) is not a scalar type.
        Some(Ok(SmeltType::SelectItems { .. })) => {
            DataType::Unknown(smelt_types::UnknownReason::Dynamic)
        }
        // Phase 37: `Struct<{declared_fields, ..r}>` return type — resolve
        // the row variable `r` by examining the call-site argument that
        // corresponds to the first struct parameter.  When the extras can
        // be determined we build a concrete `DataType::Struct` from the
        // declared fields plus the extras; otherwise fall back to Unknown.
        Some(Ok(SmeltType::Struct {
            fields: ret_fields,
            tail,
        })) => resolve_struct_return_type(call, ctx, sig, ret_fields, tail),
        // `List<T>` and `Unknown` (Phase A meta-language) — compile-time only; no
        // scalar DataType equivalent in Phase A.
        Some(Ok(SmeltType::List(_))) | Some(Ok(SmeltType::Unknown)) => {
            DataType::Unknown(smelt_types::UnknownReason::Dynamic)
        }
        // `Lambda<params, U>` (Phase B/F meta-language) — meta-only; not a valid return type.
        Some(Ok(SmeltType::Lambda(_, _))) => DataType::Unknown(smelt_types::UnknownReason::Dynamic),
        // `ColumnRef` (Phase C meta-language) — meta-only; not a SQL DataType.
        Some(Ok(SmeltType::ColumnRef)) => DataType::Unknown(smelt_types::UnknownReason::Dynamic),
        // `ModelRef` / `SourceRef` (Phase D meta-language) — meta-only; not a SQL DataType.
        Some(Ok(SmeltType::ModelRef)) | Some(Ok(SmeltType::SourceRef)) => {
            DataType::Unknown(smelt_types::UnknownReason::Dynamic)
        }
        // `Record<{…}>` / `Map<K, V>` (Phase E1 meta-language) — meta-only; not a SQL DataType.
        // Inference wiring lands in Phase 3/5.
        Some(Ok(SmeltType::Record { .. })) | Some(Ok(SmeltType::Map { .. })) => {
            DataType::Unknown(smelt_types::UnknownReason::Dynamic)
        }
        // `ModelDef` — meta-only; not a SQL DataType.
        Some(Ok(SmeltType::ModelDef)) => DataType::Unknown(smelt_types::UnknownReason::Dynamic),
        Some(Err(_)) => DataType::Unknown(smelt_types::UnknownReason::Dynamic),
        None => DataType::Unknown(smelt_types::UnknownReason::Dynamic),
    };
    Some(TypedColumn::nullable(dt))
}

/// Resolve a `Struct<{declared_fields, ..r}>` return type to a concrete
/// `DataType::Struct` by consulting the call-site argument schema (Phase 37).
///
/// Algorithm:
/// 1. Find the first struct parameter (one whose type is `SmeltType::Struct`).
/// 2. Get the corresponding call-site argument expression.
/// 3. Resolve the argument to a column set via `ctx.columns_for_qualifier`.
/// 4. Run `check_struct_row_var_binding` to compute the extras for the row var.
/// 5. Return `DataType::Struct(ret_fields + extras)`.
///
/// Falls back to `DataType::Unknown` whenever any step cannot be completed.
fn resolve_struct_return_type(
    call: &SmeltPathCall,
    ctx: &TypeContext,
    sig: &smelt_types::signatures::FunctionSig,
    ret_fields: &[(String, DataType)],
    tail: &smelt_types::signatures::StructRowTail,
) -> DataType {
    use crate::function_body_check::{check_struct_row_var_binding, struct_param_fields};
    use smelt_types::signatures::StructRowTail;

    // If no named row var, just return the declared fields as a concrete struct.
    let var_name = match tail {
        StructRowTail::Named(n) => n.as_str(),
        StructRowTail::Anon | StructRowTail::None => {
            // No row variable — build concrete struct from declared fields only.
            let concrete: Vec<(String, DataType)> = ret_fields.to_vec();
            return DataType::Struct(concrete);
        }
    };

    // Find the struct parameter index.
    let struct_param_idx = sig
        .params
        .iter()
        .position(|p| matches!(&p.type_ref, Some(Ok(SmeltType::Struct { .. }))));
    let Some(idx) = struct_param_idx else {
        return DataType::Unknown(smelt_types::UnknownReason::Dynamic);
    };

    // Get the corresponding argument expression.
    let arg_list = call.arg_list();
    let positional: Vec<_> = arg_list
        .as_ref()
        .map(|al| al.positional_args())
        .unwrap_or_default();
    let arg_expr = positional.get(idx).cloned().or_else(|| {
        // Named argument lookup.
        let param_name = &sig.params[idx].name;
        let named: Vec<_> = arg_list
            .as_ref()
            .map(|al| al.named_params().collect())
            .unwrap_or_default();
        named.into_iter().find_map(|np| {
            if np.name().as_deref() == Some(param_name.as_str()) {
                np.value_expr()
            } else {
                None
            }
        })
    });
    let Some(arg) = arg_expr else {
        return DataType::Unknown(smelt_types::UnknownReason::Dynamic);
    };

    // Resolve the argument to a column set.
    let qualifier = arg.text().trim().to_string();
    if qualifier.is_empty() {
        return DataType::Unknown(smelt_types::UnknownReason::Dynamic);
    }
    let cols: Vec<(String, DataType)> = ctx
        .columns_for_qualifier(&qualifier)
        .into_iter()
        .map(|(col_name, tc)| (col_name.to_string(), tc.data_type.clone()))
        .collect();
    if cols.is_empty() {
        return DataType::Unknown(smelt_types::UnknownReason::Dynamic);
    }

    // Extract declared fields from the struct parameter to compute extras.
    let param = &sig.params[idx];
    let Some((declared_fields, param_tail)) = struct_param_fields(param) else {
        return DataType::Unknown(smelt_types::UnknownReason::Dynamic);
    };

    // Run struct row-var unification to get extras.
    let extras = match check_struct_row_var_binding(declared_fields, &cols, param_tail) {
        Ok(Some(extras)) => extras,
        Ok(None) => vec![],
        // intentionally ignored: binding failure means the argument struct is
        // incompatible; returning Unknown lets the call-site check emit
        // RowRequirementUnsatisfied rather than cascading inference errors.
        Err(_) => return DataType::unknown_dynamic(),
    };

    // Check that the row var name matches between param and return type.
    let param_var_matches = match param_tail {
        StructRowTail::Named(param_var) => param_var.as_str() == var_name,
        _ => false,
    };
    if !param_var_matches {
        return DataType::Unknown(smelt_types::UnknownReason::Dynamic);
    }

    // Build the concrete return type: declared return fields + extras.
    let mut concrete: Vec<(String, DataType)> = ret_fields.to_vec();
    concrete.extend(extras);
    DataType::Struct(concrete)
}

/// Infer the type of a `smelt.as_struct(alias [EXCEPT col1, col2])` call
/// (Phase 38).
///
/// Algorithm:
/// 1. Read the alias name from the `SmeltAsStructCall`.
/// 2. Collect columns for that qualifier via `ctx.columns_for_qualifier`.
/// 3. Remove columns named in the EXCEPT list.
/// 4. Return `DataType::Struct(remaining_fields)`.
///
/// Returns `None` when the alias cannot be resolved in the context.
pub fn infer_as_struct_type(call: &SmeltAsStructCall, ctx: &TypeContext) -> Option<TypedColumn> {
    let alias = call.alias()?;
    let except = call.except_columns();
    let cols = ctx.columns_for_qualifier(&alias);
    if cols.is_empty() {
        return None;
    }
    let fields: Vec<(String, DataType)> = cols
        .into_iter()
        .filter(|(name, _)| !except.contains(&name.to_string()))
        .map(|(name, tc)| (name.to_string(), tc.data_type.clone()))
        .collect();
    Some(TypedColumn {
        data_type: DataType::Struct(fields),
        nullable: false,
    })
}
