//! Function call and smelt-path-call type inference (incl. registry-driven inference and AS_STRUCT).

#![allow(unused_imports)]
use rowan::TextRange;
use smelt_parser::ast::{
    BinaryExpr, CaseExpr, CastExpr, Cte, Expr, ExtractExpr, FunctionCall, RowConstructor,
    SelectStmt, SmeltAsStructCall, SmeltPathCall, StructLiteral, Subquery,
};
use smelt_types::signatures::{
    kind_ceiling, unify_call_with_expected, BuiltinRegistry, ExprKind, FunctionSig, RecordRegistry,
    SmeltType, TypeConstraint,
};
use smelt_types::{parse_type, DataType, SqlFunction, TypedColumn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::type_context::TypeContext;
#[allow(unused_imports)]
use super::*;

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

/// LUB adapter: the canonical numeric-promotion routine lives in
/// [`promote_types`] (this module) but signatures-side [`unify_call`] needs a
/// plain `Fn(&DataType, &DataType) -> DataType`. This wrapper keeps
/// `smelt-types` dependency-free per the plan's cross-phase design choice.
fn registry_lub(a: &DataType, b: &DataType) -> DataType {
    let lhs = TypedColumn {
        data_type: a.clone(),
        nullable: true,
    };
    let rhs = TypedColumn {
        data_type: b.clone(),
        nullable: true,
    };
    promote_types(&lhs, &rhs).data_type
}

/// Names whose typing is driven registry-first (via [`try_registry_inference`])
/// rather than by the hand-written `match` in [`infer_function_type`]. Every
/// entry's `BuiltinRegistry` signature reproduces the legacy arm's return type,
/// and [`registry_result_nullable`] reproduces its nullability. Names NOT on
/// this list stay on the legacy path; that residual set is the
/// `legacy_match_ratchet` count and shrinks as functions migrate.
///
/// Excluded on purpose (the **named exception list** — their return type or
/// nullability depends on argument types/values in a way a static `Signature`
/// cannot express, so they remain hand-written):
///   * `SUM` — precision/width widening (`SUM(DECIMAL(p,s)) → DECIMAL(38,s)`).
///   * `CEIL`/`CEILING`/`FLOOR` — `DECIMAL(p,s) → DECIMAL(p,0)`, else Double.
///   * `ROUND`/`TRUNC`/`TRUNCATE`/`MOD` — first-argument identity else Double/Integer.
///   * `MEDIAN` — integer inputs widen to Double, others keep their type.
///   * `ARRAY_AGG` — wraps the argument type in `Array<…>`.
///   * `DATE_TRUNC` — return tz-axis mirrors the second argument.
///   * `COALESCE`/`IFNULL`/`NULLIF`/`GREATEST`/`LEAST` — first-concrete-of-N
///     with argument-derived nullability.
///   * `MODE`/`ANY_VALUE`/`ARG_MAX`/`FIRST`/`LAST` and the window-navigation
///     family (`LAG`/`LEAD`/`FIRST_VALUE`/`LAST_VALUE`/`NTH_VALUE`) — first-
///     argument identity with optional trailing arguments (variable arity).
///   * `BIT_AND`/`BIT_OR`/`BIT_XOR` — first-argument identity else BigInt.
///   * `EXTRACT` — routed as `ExtractExpr` syntax, not a plain call.
///
/// The list is ordered by family for easy review.
const REGISTRY_MIGRATED: &[&str] = &[
    // ── Aggregates ──────────────────────────────────────────────────────────
    "AVG",   // <T: Numeric>(T) → Double
    "MIN",   // <T: Ordered>(T) → T (first-arg identity, nullable=true)
    "MAX",   // <T: Ordered>(T) → T
    "COUNT", // (Any) → BigInt (nullable=false)
    "STDDEV",
    "STDDEV_POP",
    "STDDEV_SAMP",
    "VARIANCE",
    "VAR_POP",
    "VAR_SAMP",
    "CORR",
    "COVAR_POP",
    "COVAR_SAMP",
    "REGR_SLOPE",
    "PERCENTILE_CONT",
    "PERCENTILE_DISC",
    "BOOL_AND",
    "BOOL_OR",
    "EVERY",
    "STRING_AGG",
    "LISTAGG",
    "GROUP_CONCAT",
    "APPROX_COUNT_DISTINCT", // (Any) → BigInt (nullable=false)
    // ── Window (fixed return) ───────────────────────────────────────────────
    "ROW_NUMBER",
    "RANK",
    "DENSE_RANK",
    "NTILE",
    "CUME_DIST",
    "PERCENT_RANK",
    // ── Arithmetic / numeric scalars (fixed return) ─────────────────────────
    "ABS", // <T: Numeric>(T) → T (preserves arg nullability)
    "SIGN",
    "POWER",
    "POW",
    "SQRT",
    "EXP",
    "LN",
    "LOG",
    "LOG10",
    "LOG2",
    "SIN",
    "COS",
    "TAN",
    "ASIN",
    "ACOS",
    "ATAN",
    "ATAN2",
    "SINH",
    "COSH",
    "TANH",
    "PI",     // () → Double (nullable=false)
    "RANDOM", // () → Double (nullable=false)
    // ── Text scalars ────────────────────────────────────────────────────────
    "LOWER",
    "UPPER",
    "TRIM",
    "LTRIM",
    "RTRIM",
    "CONCAT",
    "REPLACE",
    "TRANSLATE",
    "REVERSE",
    "REPEAT",
    "LPAD",
    "RPAD",
    "INITCAP",
    "QUOTE_IDENT",
    "QUOTE_LITERAL",
    "LEFT",
    "RIGHT",
    "SUBSTRING",
    "SUBSTR",
    "SPLIT_PART",
    "TO_CHAR",
    "LENGTH",           // → BigInt
    "CHAR_LENGTH",      // → BigInt
    "CHARACTER_LENGTH", // → BigInt
    "POSITION",         // → BigInt
    "STRPOS",           // → BigInt
    // ── Date / time (fixed return) ──────────────────────────────────────────
    "DATE",
    "CURRENT_DATE",
    "NOW",               // () → Timestamp{tz} (nullable=false)
    "CURRENT_TIMESTAMP", // () → Timestamp{tz} (nullable=false)
    "MAKE_DATE",
    "MAKE_TIME",
    "MAKE_TIMESTAMP",
    "MAKE_TIMESTAMPTZ",
    "AGE",
    "YEAR",
    "MONTH",
    "DAY",
    "DAYOFWEEK",
    "QUARTER",
    "DATE_PART",
    // ── JSON (fixed return) ─────────────────────────────────────────────────
    "JSON_OBJECT",
    "JSON_ARRAY",
    "TO_JSON",
    "JSON_EXTRACT",
    "JSON_EXTRACT_TEXT",
    "JSON_ARRAY_LENGTH",
    "JSON_OBJECT_KEYS",
    "JSON_CONTAINS",
];

/// The residual set of recognised functions still typed by the hand-written
/// match, exposed for the `legacy_match_ratchet` gate. A name is registry-first
/// iff it appears in [`REGISTRY_MIGRATED`].
pub fn registry_migrated_names() -> &'static [&'static str] {
    REGISTRY_MIGRATED
}

/// Policy for deriving [`TypedColumn::nullable`] on a registry-resolved call.
///
/// The registry itself doesn't track nullability (§16 defers it — see "Out of
/// scope" in the plan), so Phase 9 mirrors the legacy per-function rule via a
/// tiny lookup table. Migrating a new entry to the registry means adding a row
/// here.
fn registry_result_nullable(name: &str, arg_nullable: &[bool]) -> bool {
    match name {
        // Non-nullable aggregates / niladic clocks / ranking windows /
        // deterministic niladic scalars — mirrors the legacy per-function rule.
        "COUNT"
        | "NOW"
        | "CURRENT_DATE"
        | "CURRENT_TIMESTAMP"
        | "APPROX_COUNT_DISTINCT"
        | "ROW_NUMBER"
        | "RANK"
        | "DENSE_RANK"
        | "NTILE"
        | "CUME_DIST"
        | "PERCENT_RANK"
        | "PI"
        | "RANDOM" => false,
        // ABS preserves its arg's nullability — legacy returns the arg
        // TypedColumn verbatim when a single-arg inference succeeds.
        "ABS" => arg_nullable.first().copied().unwrap_or(true),
        // Everything else is nullable per legacy.
        _ => true,
    }
}

/// Registry-first inference for the allowlisted subset of built-ins.
///
/// Returns:
/// * `Some(Some(tc))` when the registry resolved the call cleanly — the caller
///   uses this directly and skips the legacy match.
/// * `Some(None)` when the function is known to the registry but arg types
///   couldn't be inferred up-front — the caller should fall through to the
///   legacy match which handles Unknown args more gracefully.
/// * `None` when the function isn't on [`REGISTRY_MIGRATED`] or the registry
///   doesn't know about it — caller uses the legacy match.
fn try_registry_inference(
    upper_name: &str,
    func: &FunctionCall,
    ctx: &TypeContext,
) -> Option<Option<TypedColumn>> {
    if !REGISTRY_MIGRATED.contains(&upper_name) {
        return None;
    }
    let sig = BuiltinRegistry::resolve(upper_name)?;

    // Collect arg DataTypes + nullability. If any arg fails to infer, defer
    // to the legacy match — it has per-function fallback behaviour for
    // Unknown args that the registry doesn't model.
    let args = func.arguments();
    let mut arg_types: Vec<DataType> = Vec::with_capacity(args.len());
    let mut arg_nullable: Vec<bool> = Vec::with_capacity(args.len());
    for arg in &args {
        match infer_expression_type(arg, ctx) {
            Some(tc) => {
                arg_types.push(tc.data_type);
                arg_nullable.push(tc.nullable);
            }
            None => {
                // Missing arg inference — fall back to legacy, which has
                // function-specific Unknown handling (e.g. `first_arg_type_or`
                // supplies a sensible default for MIN/MAX).
                return Some(None);
            }
        }
    }

    match unify_call_with_expected(sig, &arg_types, ctx.expected_return.as_ref(), &registry_lub) {
        Ok(result) => {
            let nullable = registry_result_nullable(upper_name, &arg_nullable);
            Some(Some(TypedColumn {
                data_type: result.return_type,
                nullable,
            }))
        }
        // intentionally ignored: unification failed → fall back to the legacy
        // match so permissive behaviour (LOWER on Integer, MIN on Unknown) is
        // preserved. The caller handles None as "type unknown, use legacy path".
        Err(_) => Some(None),
    }
}

/// Infer the type of a function call (aggregates, etc.)
pub fn infer_function_type(func: &FunctionCall, ctx: &TypeContext) -> Option<TypedColumn> {
    let name = func.name()?.to_uppercase();
    let sql_func = SqlFunction::from_name(&name)?;

    // Phase 9: registry-first lookup for the allowlisted subset. When the
    // registry returns a concrete result we use it; on miss/fall-through we
    // continue into the legacy match below.
    if let Some(Some(tc)) = try_registry_inference(&name, func, ctx) {
        return Some(tc);
    }

    /// Helper: return the type of the first argument, or `fallback` if inference fails.
    /// For COALESCE and similar, this intentionally only checks the first arg —
    /// using a later arg's type would risk being incorrect if earlier args are Unknown.
    fn first_arg_type_or(
        func: &FunctionCall,
        ctx: &TypeContext,
        fallback: DataType,
        nullable: bool,
    ) -> Option<TypedColumn> {
        if let Some(arg) = func.arguments().first() {
            if let Some(arg_type) = infer_expression_type(arg, ctx) {
                return Some(TypedColumn {
                    data_type: arg_type.data_type,
                    nullable,
                });
            }
        }
        Some(TypedColumn {
            data_type: fallback,
            nullable,
        })
    }

    match sql_func {
        SqlFunction::Count => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: false,
        }),

        SqlFunction::Sum => {
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    // DuckDB SUM widening rules:
                    //   SUM(SMALLINT|INTEGER|BIGINT)   -> BIGINT (HUGEINT in DuckDB,
                    //                                     but smelt models that as BIGINT)
                    //   SUM(DOUBLE|FLOAT)              -> DOUBLE
                    //   SUM(DECIMAL(p, s))             -> DECIMAL(38, s)
                    //
                    // The Decimal precision widen-to-38 is critical: real
                    // pipelines accumulate ~1e6 rows of DECIMAL(10,2) values
                    // which overflow precision 10 quickly. Keeping the input
                    // precision silently corrupts results.
                    let result_type = match &arg_type.data_type {
                        DataType::SmallInt | DataType::Integer => DataType::BigInt,
                        DataType::BigInt => DataType::BigInt,
                        DataType::Float | DataType::Double => DataType::Double,
                        DataType::Decimal { scale, .. } => DataType::Decimal {
                            precision: 38,
                            scale: *scale,
                        },
                        // Unknown / mixed: defer to BIGINT (the historical
                        // fallback) — but the caller is expected to give us
                        // a populated TypeContext so this path is rare.
                        _ => DataType::BigInt,
                    };
                    return Some(TypedColumn {
                        data_type: result_type,
                        nullable: true,
                    });
                }
            }
            Some(TypedColumn {
                data_type: DataType::BigInt,
                nullable: true,
            })
        }

        SqlFunction::Avg => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),

        SqlFunction::Min | SqlFunction::Max => {
            first_arg_type_or(func, ctx, DataType::unknown_dynamic(), true)
        }

        SqlFunction::Coalesce => {
            // Try all arguments, return first concrete (non-Unknown, non-Null) type.
            // COALESCE is non-nullable when at least one argument is non-nullable
            // or is a non-null literal, because the result will always have a value.
            let mut result_type = None;
            let mut has_non_nullable_arg = false;
            for arg in func.arguments() {
                if let Some(arg_type) = infer_expression_type(&arg, ctx) {
                    if !arg_type.nullable {
                        has_non_nullable_arg = true;
                    }
                    if result_type.is_none()
                        && !matches!(arg_type.data_type, DataType::Unknown(_) | DataType::Null)
                    {
                        result_type = Some(arg_type.data_type.clone());
                    }
                }
            }
            let data_type =
                result_type.unwrap_or(DataType::Unknown(smelt_types::UnknownReason::Dynamic));
            Some(TypedColumn {
                data_type,
                nullable: !has_non_nullable_arg,
            })
        }

        SqlFunction::Nullif => first_arg_type_or(func, ctx, DataType::unknown_dynamic(), true),

        SqlFunction::Ifnull => {
            // IFNULL(a, b) is equivalent to COALESCE(a, b).
            // Non-nullable when either argument is non-nullable.
            let args = func.arguments();
            let first_type = args.first().and_then(|a| infer_expression_type(a, ctx));
            let second_type = args.get(1).and_then(|a| infer_expression_type(a, ctx));
            let data_type = first_type
                .as_ref()
                .filter(|t| !matches!(t.data_type, DataType::Unknown(_) | DataType::Null))
                .or(second_type.as_ref())
                .map(|t| t.data_type.clone())
                .unwrap_or(DataType::Unknown(smelt_types::UnknownReason::Dynamic));
            let has_non_nullable = first_type.as_ref().is_some_and(|t| !t.nullable)
                || second_type.as_ref().is_some_and(|t| !t.nullable);
            Some(TypedColumn {
                data_type,
                nullable: !has_non_nullable,
            })
        }

        SqlFunction::RowNumber
        | SqlFunction::Rank
        | SqlFunction::DenseRank
        | SqlFunction::Ntile => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: false,
        }),

        SqlFunction::CumeDist | SqlFunction::PercentRank => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: false,
        }),

        SqlFunction::Lag
        | SqlFunction::Lead
        | SqlFunction::FirstValue
        | SqlFunction::LastValue
        | SqlFunction::NthValue => first_arg_type_or(func, ctx, DataType::unknown_dynamic(), true),

        SqlFunction::Now | SqlFunction::CurrentTimestamp => Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: true,
            },
            nullable: false,
        }),

        SqlFunction::CurrentDate => Some(TypedColumn {
            data_type: DataType::Date,
            nullable: false,
        }),

        SqlFunction::Date => Some(TypedColumn {
            data_type: DataType::Date,
            nullable: true,
        }),

        SqlFunction::DateTrunc => {
            // Mirror the tz-axis of the second argument (the timestamp).
            // DATE_TRUNC(part, ts) — argument index 1 is the timestamp.
            // Nullability propagates from the ts argument: DATE_TRUNC does not
            // introduce NULL, so it is nullable iff the input is nullable. When
            // the input type is unknown (None) we conservatively default to
            // NOT NULL rather than nullable — unknown inputs are typically NOT
            // NULL in practice and this avoids false-positive D-52 diagnostics
            // on partition columns that use date_trunc to bucket timestamps.
            let inner_type = func
                .arguments()
                .get(1)
                .and_then(|arg| infer_expression_type(arg, ctx));
            let with_timezone = inner_type
                .as_ref()
                .and_then(|tc| match tc.data_type {
                    DataType::Timestamp { with_timezone } => Some(with_timezone),
                    _ => None,
                })
                .unwrap_or(false);
            let nullable = inner_type.map(|tc| tc.nullable).unwrap_or(false);
            Some(TypedColumn {
                data_type: DataType::Timestamp { with_timezone },
                nullable,
            })
        }

        SqlFunction::Concat
        | SqlFunction::Upper
        | SqlFunction::Lower
        | SqlFunction::Trim
        | SqlFunction::Ltrim
        | SqlFunction::Rtrim
        | SqlFunction::Substring
        | SqlFunction::Substr => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::Length | SqlFunction::CharLength | SqlFunction::CharacterLength => {
            Some(TypedColumn {
                data_type: DataType::BigInt,
                nullable: true,
            })
        }

        SqlFunction::ToChar => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::BoolAnd | SqlFunction::BoolOr | SqlFunction::Every => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: true,
        }),

        SqlFunction::Abs => {
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    return Some(arg_type);
                }
            }
            Some(TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            })
        }

        SqlFunction::Sign => Some(TypedColumn {
            data_type: DataType::SmallInt,
            nullable: true,
        }),

        SqlFunction::Round | SqlFunction::Trunc | SqlFunction::Truncate => {
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    return Some(arg_type);
                }
            }
            Some(TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            })
        }

        SqlFunction::Ceil | SqlFunction::Ceiling | SqlFunction::Floor => {
            // DuckDB: CEIL/FLOOR(DECIMAL(p,s)) → Decimal(p,0), all others → Double
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    let result_type = match &arg_type.data_type {
                        DataType::Decimal { precision, .. } => DataType::Decimal {
                            precision: *precision,
                            scale: 0,
                        },
                        _ => DataType::Double,
                    };
                    return Some(TypedColumn {
                        data_type: result_type,
                        nullable: arg_type.nullable,
                    });
                }
            }
            Some(TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            })
        }

        SqlFunction::Power
        | SqlFunction::Pow
        | SqlFunction::Sqrt
        | SqlFunction::Exp
        | SqlFunction::Ln
        | SqlFunction::Log
        | SqlFunction::Log10
        | SqlFunction::Log2 => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),

        SqlFunction::Mod => first_arg_type_or(func, ctx, DataType::Integer, true),

        SqlFunction::Sin
        | SqlFunction::Cos
        | SqlFunction::Tan
        | SqlFunction::Asin
        | SqlFunction::Acos
        | SqlFunction::Atan
        | SqlFunction::Atan2
        | SqlFunction::Sinh
        | SqlFunction::Cosh
        | SqlFunction::Tanh => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),

        SqlFunction::Pi | SqlFunction::Random => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: false,
        }),

        SqlFunction::Extract
        | SqlFunction::DatePart
        | SqlFunction::Year
        | SqlFunction::Month
        | SqlFunction::Day
        | SqlFunction::DayOfWeek
        | SqlFunction::Quarter => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: true,
        }),

        SqlFunction::MakeDate => Some(TypedColumn {
            data_type: DataType::Date,
            nullable: true,
        }),

        SqlFunction::MakeTime => Some(TypedColumn {
            data_type: DataType::Time,
            nullable: true,
        }),

        SqlFunction::MakeTimestamp => Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: false,
            },
            nullable: true,
        }),

        SqlFunction::MakeTimestamptz => Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: true,
            },
            nullable: true,
        }),

        SqlFunction::Age => Some(TypedColumn {
            data_type: DataType::Interval,
            nullable: true,
        }),

        SqlFunction::Replace
        | SqlFunction::Translate
        | SqlFunction::Reverse
        | SqlFunction::Repeat
        | SqlFunction::Lpad
        | SqlFunction::Rpad
        | SqlFunction::Initcap
        | SqlFunction::QuoteIdent
        | SqlFunction::QuoteLiteral => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::Left | SqlFunction::Right => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::Position | SqlFunction::Strpos => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: true,
        }),

        SqlFunction::SplitPart => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::Greatest | SqlFunction::Least => {
            // Try all arguments, return first concrete type
            for arg in func.arguments() {
                if let Some(arg_type) = infer_expression_type(&arg, ctx) {
                    if !matches!(arg_type.data_type, DataType::Unknown(_) | DataType::Null) {
                        return Some(TypedColumn {
                            data_type: arg_type.data_type,
                            nullable: true,
                        });
                    }
                }
            }
            first_arg_type_or(func, ctx, DataType::unknown_dynamic(), true)
        }

        SqlFunction::ArrayAgg => {
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    return Some(TypedColumn {
                        data_type: DataType::Array(Box::new(arg_type.data_type)),
                        nullable: true,
                    });
                }
            }
            Some(TypedColumn {
                data_type: DataType::Array(Box::new(DataType::unknown_dynamic())),
                nullable: true,
            })
        }

        SqlFunction::StringAgg | SqlFunction::Listagg => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::JsonObject
        | SqlFunction::JsonArray
        | SqlFunction::ToJson
        | SqlFunction::JsonExtract
        | SqlFunction::JsonExtractText => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::JsonArrayLength => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: true,
        }),

        SqlFunction::JsonObjectKeys => Some(TypedColumn {
            data_type: DataType::Array(Box::new(DataType::Text)),
            nullable: true,
        }),

        // json_contains is NULL-propagating per spec §11:
        // json_contains(NULL, ...) = NULL and json_contains(..., NULL) = NULL.
        SqlFunction::JsonContains => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: true,
        }),

        // Aggregate functions from optimizer that don't have specialized type inference yet
        SqlFunction::Stddev
        | SqlFunction::Variance
        | SqlFunction::StddevPop
        | SqlFunction::StddevSamp
        | SqlFunction::VarPop
        | SqlFunction::VarSamp
        | SqlFunction::Corr
        | SqlFunction::CovarPop
        | SqlFunction::CovarSamp
        | SqlFunction::RegrSlope => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),

        SqlFunction::Median => {
            // MEDIAN interpolates: integer-family inputs widen to Double (DuckDB
            // and Spark both return DOUBLE for integer medians); Decimal/Double/
            // temporal inputs keep their own type.
            // Regression: median_integer_infers_double.
            first_arg_type_or(func, ctx, DataType::Double, true).map(|tc| match tc.data_type {
                DataType::SmallInt | DataType::Integer | DataType::BigInt => TypedColumn {
                    data_type: DataType::Double,
                    nullable: tc.nullable,
                },
                _ => tc,
            })
        }

        SqlFunction::Mode => first_arg_type_or(func, ctx, DataType::unknown_dynamic(), true),

        SqlFunction::PercentileCont | SqlFunction::PercentileDisc => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),

        SqlFunction::ApproxCountDistinct => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: false,
        }),

        SqlFunction::AnyValue | SqlFunction::ArgMax | SqlFunction::First | SqlFunction::Last => {
            first_arg_type_or(func, ctx, DataType::unknown_dynamic(), true)
        }

        SqlFunction::GroupConcat => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::BitAnd | SqlFunction::BitOr | SqlFunction::BitXor => {
            first_arg_type_or(func, ctx, DataType::BigInt, true)
        }
    }
}
