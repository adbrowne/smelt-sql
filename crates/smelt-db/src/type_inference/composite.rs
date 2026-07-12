//! Array, row, and struct literal type inference (incl. subscript and slice).

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

/// Infer the type of an array literal (ARRAY[1, 2, 3]).
/// All elements must have the same type; mixed-type arrays return None (error).
/// Empty arrays return Array(Unknown).
pub fn infer_array_literal_type(
    array_lit: &smelt_parser::ArrayLiteral,
    ctx: &TypeContext,
) -> Option<TypedColumn> {
    let elements = array_lit.elements();

    if elements.is_empty() {
        return Some(TypedColumn {
            data_type: DataType::Array(Box::new(DataType::unknown_dynamic())),
            nullable: false,
        });
    }

    // Infer element types
    let mut element_typed: Option<TypedColumn> = None;

    for elem in &elements {
        // Can't infer element type
        let typed = infer_expression_type(elem, ctx)?;
        match &element_typed {
            None => {
                // First element sets the type (skip Null — it's compatible with anything)
                if typed.data_type != DataType::Null {
                    element_typed = Some(typed);
                }
            }
            Some(existing) => {
                if typed.data_type == DataType::Null {
                    // NULL is compatible with any element type
                    continue;
                }
                if typed.data_type != existing.data_type {
                    // Try promotion
                    let promoted = promote_types(existing, &typed);
                    if promoted.data_type.is_unknown() {
                        // Mixed types that can't be promoted — reject
                        return None;
                    }
                    element_typed = Some(promoted);
                }
            }
        }
    }

    let elem_type = element_typed.map(|t| t.data_type).unwrap_or(DataType::Null);
    Some(TypedColumn {
        data_type: DataType::Array(Box::new(elem_type)),
        nullable: false, // The array itself is not nullable; elements may be
    })
}

/// Infer the type of a list comprehension (`[expr FOR x IN list (IF cond)?]`,
/// DuckDB). The result is always `Array<T>`, matching DuckDB's typing.
///
/// Typing strategy: the loop variable `x` binds inside `expr` (and the
/// optional `IF` filter), but smelt's `TypeContext` has no mechanism to bind
/// a scoped scalar name for the duration of a sub-expression's inference —
/// that's meta-language lambda-parameter machinery this expression form
/// doesn't otherwise need. Rather than build that machinery for one
/// construct, we special-case the common, staticaly-resolvable shape:
///
/// - **Bare-variable element** (`[x FOR x IN list]`, filter present or not):
///   the result element type is exactly the source list's element type — no
///   binding is needed because the "expression" IS the loop variable.
/// - **Any other element expression** (`[x + 1 FOR x IN list]`,
///   `[f(x) FOR x IN list]`, …): the element type depends on `x`'s bound
///   type inside a scope this function cannot construct, so the element type
///   is classified `Unknown` (`unknown_dynamic` — legitimately unknowable
///   here, not a diagnosable gap; see `.claude/unknown-census.toml` census
///   discipline, which this call is exempt from since it never spells
///   `DataType::Unknown` directly, matching the empty-array-literal
///   precedent above).
pub fn infer_list_comprehension_type(
    comp: &smelt_parser::ast::ListComprehension,
    ctx: &TypeContext,
) -> Option<TypedColumn> {
    let source = comp.source()?;
    let source_typed = infer_expression_type(&source, ctx)?;
    let source_elem_type = match &source_typed.data_type {
        DataType::Array(inner) => (**inner).clone(),
        _ => DataType::unknown_dynamic(),
    };

    let element = comp.element()?;
    let is_bare_loop_var = comp
        .var_name()
        .zip(element.as_column_ref())
        .is_some_and(|(var, col)| col.qualifier().is_none() && col.name() == var);

    let result_elem_type = if is_bare_loop_var {
        source_elem_type
    } else {
        DataType::unknown_dynamic()
    };

    Some(TypedColumn {
        data_type: DataType::Array(Box::new(result_elem_type)),
        nullable: false,
    })
}

/// Infer the type of an array subscript (arr[i]).
/// Returns the element type of the array.
pub fn infer_array_subscript_type(
    expr: &smelt_parser::Expr,
    ctx: &TypeContext,
) -> Option<TypedColumn> {
    // The expr contains both the base expression and the ARRAY_SUBSCRIPT node as children.
    // We need to find the base expression (which should be a column ref or other expr
    // that evaluates to an array type) and extract the element type.

    // Find the first child Expr that is NOT inside the ARRAY_SUBSCRIPT
    let base_exprs: Vec<_> = expr
        .syntax()
        .children()
        .filter_map(smelt_parser::Expr::cast)
        .collect();

    // The first Expr child should be the base (e.g., the column reference)
    if let Some(base_expr) = base_exprs.first() {
        if let Some(base_type) = infer_expression_type(base_expr, ctx) {
            if let DataType::Array(inner) = base_type.data_type {
                return Some(TypedColumn {
                    data_type: *inner,
                    nullable: true, // Array element access can always be NULL (out of bounds)
                });
            }
        }
    }

    None
}

/// Infer the type of an array slice (arr[start:end]).
/// Returns the same array type as the base.
pub fn infer_array_slice_type(expr: &smelt_parser::Expr, ctx: &TypeContext) -> Option<TypedColumn> {
    // Similar to subscript — find the base expression
    let base_exprs: Vec<_> = expr
        .syntax()
        .children()
        .filter_map(smelt_parser::Expr::cast)
        .collect();

    if let Some(base_expr) = base_exprs.first() {
        if let Some(base_type) = infer_expression_type(base_expr, ctx) {
            if let DataType::Array(_) = &base_type.data_type {
                return Some(TypedColumn {
                    data_type: base_type.data_type,
                    nullable: true, // Slice result could be NULL
                });
            }
        }
    }

    None
}

/// Infer the type of a ROW constructor: ROW(1, 2, 3) → Struct with positional fields.
pub fn infer_row_constructor_type(row: &RowConstructor, ctx: &TypeContext) -> Option<TypedColumn> {
    let elements = row.elements();
    let mut fields = Vec::new();

    for (i, elem) in elements.iter().enumerate() {
        let typed = infer_expression_type(elem, ctx)?;
        // Positional fields: v1, v2, v3, ...
        fields.push((format!("v{}", i + 1), typed.data_type));
    }

    Some(TypedColumn {
        data_type: DataType::Struct(fields),
        nullable: false, // The struct itself is not nullable
    })
}

/// Infer the type of a struct literal: STRUCT(1 AS a, 'hello' AS b) → Struct with named fields.
pub fn infer_struct_literal_type(
    struct_lit: &StructLiteral,
    ctx: &TypeContext,
) -> Option<TypedColumn> {
    let fields_ast = struct_lit.fields();
    let mut fields = Vec::new();

    for (i, (expr, name)) in fields_ast.iter().enumerate() {
        let typed = infer_expression_type(expr, ctx)?;
        let field_name = name.clone().unwrap_or_else(|| format!("v{}", i + 1));
        fields.push((field_name, typed.data_type));
    }

    Some(TypedColumn {
        data_type: DataType::Struct(fields),
        nullable: false, // The struct itself is not nullable
    })
}

/// Infer the type of a MAP literal (DuckDB `MAP {'a': 1, 'b': 2}`) as
/// `Map(key_type, value_type)`.
///
/// Key types are unified across all entries the same way array-literal
/// elements are unified (first non-NULL entry sets the type; later entries
/// must match or promote); value types are unified independently. Mixed,
/// non-promotable key or value types reject inference (`None`), matching
/// `infer_array_literal_type`. Empty `MAP {}` infers `Map(Unknown, Unknown)`
/// — DuckDB itself defaults an empty map's key/value types to INTEGER, but
/// smelt follows the array-literal precedent (`Array(Unknown)` for `[]`)
/// rather than encoding that engine-specific quirk.
pub fn infer_map_literal_type(
    map_lit: &smelt_parser::ast::MapLiteral,
    ctx: &TypeContext,
) -> Option<TypedColumn> {
    let entries = map_lit.entries();

    if entries.is_empty() {
        return Some(TypedColumn {
            data_type: DataType::Map(
                Box::new(DataType::unknown_dynamic()),
                Box::new(DataType::unknown_dynamic()),
            ),
            nullable: false,
        });
    }

    let mut key_typed: Option<TypedColumn> = None;
    let mut value_typed: Option<TypedColumn> = None;

    for entry in &entries {
        let key_expr = entry.key()?;
        let value_expr = entry.value()?;

        let key_ty = infer_expression_type(&key_expr, ctx)?;
        let value_ty = infer_expression_type(&value_expr, ctx)?;

        key_typed = unify_entry_type(key_typed, key_ty)?;
        value_typed = unify_entry_type(value_typed, value_ty)?;
    }

    let key_type = key_typed.map(|t| t.data_type).unwrap_or(DataType::Null);
    let value_type = value_typed.map(|t| t.data_type).unwrap_or(DataType::Null);

    Some(TypedColumn {
        data_type: DataType::Map(Box::new(key_type), Box::new(value_type)),
        nullable: false, // The map itself is not nullable; keys/values may be
    })
}

/// Fold one more entry's typed column into a running unification, mirroring
/// `infer_array_literal_type`'s element-unification loop: NULL is compatible
/// with anything, the first non-NULL entry sets the type, and later entries
/// must match or promote. Returns `None` when types can't be promoted.
fn unify_entry_type(
    running: Option<TypedColumn>,
    next: TypedColumn,
) -> Option<Option<TypedColumn>> {
    match running {
        None => {
            if next.data_type == DataType::Null {
                Some(None)
            } else {
                Some(Some(next))
            }
        }
        Some(existing) => {
            if next.data_type == DataType::Null {
                return Some(Some(existing));
            }
            if next.data_type != existing.data_type {
                let promoted = promote_types(&existing, &next);
                if promoted.data_type.is_unknown() {
                    return None;
                }
                return Some(Some(promoted));
            }
            Some(Some(existing))
        }
    }
}
