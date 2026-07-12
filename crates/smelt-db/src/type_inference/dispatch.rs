//! Top-level dispatch: infer_expression_type / infer_expression_kind / select output schema / type promotion / window-in-scalar-context check.

#![allow(unused_imports)]
use rowan::TextRange;
use smelt_parser::ast::{
    BinaryExpr, CaseExpr, CastExpr, CollateExpr, Cte, Expr, ExtractExpr, FunctionCall,
    RowConstructor, SelectStmt, SmeltAsStructCall, SmeltPathCall, StructLiteral, Subquery,
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

pub fn infer_expression_type(expr: &Expr, ctx: &TypeContext) -> Option<TypedColumn> {
    let text = expr.text().trim().to_string();

    // Try COLLATE expression (§17 — binary collation passes through; non-binary → Unknown).
    // The diagnostic for non-binary collations is emitted separately by
    // `check_collation_diagnostics` in `lib.rs::check_file_diagnostics`.
    if let Some(collate_expr) = expr.as_collate() {
        return super::collation::infer_collate_expr_type(&collate_expr, ctx);
    }

    // Try CAST expression first
    if let Some(cast_expr) = expr.as_cast() {
        return infer_cast_type(&cast_expr, ctx);
    }

    // Try CASE expression
    if let Some(case_expr) = expr.as_case() {
        return infer_case_expr_type(&case_expr, ctx);
    }

    // Try subquery (scalar subquery)
    if let Some(subquery) = expr.as_subquery() {
        return infer_subquery_type(&subquery, ctx);
    }

    // Try EXTRACT expression
    if let Some(extract_expr) = expr.as_extract() {
        return infer_extract_type(&extract_expr);
    }

    // Try function call (aggregates, etc.)
    if let Some(func) = expr.as_function_call() {
        return infer_function_type(&func, ctx);
    }

    // Try smelt.functions.<name>(...) user-function call site. Returns the
    // declared return type of the resolved signature, or Unknown if the
    // function cannot be resolved / lacks a return annotation. Diagnostics for
    // unresolved functions / arg mismatches are emitted elsewhere
    // (`function_body_check::check_smelt_path_call`), so this path is
    // type-only.
    if let Some(call) = expr.as_smelt_path_call() {
        return infer_smelt_path_call_type(&call, ctx);
    }

    // Try smelt.as_struct(alias [EXCEPT cols]) — Phase 38.
    // Resolves the alias's columns from the TypeContext, filters EXCEPT
    // columns, and returns DataType::Struct(remaining_fields).
    if let Some(call) = expr.as_smelt_as_struct_call() {
        return infer_as_struct_type(&call, ctx);
    }

    // Try binary expression
    if let Some(binary) = expr.as_binary() {
        return infer_binary_expr_type(&binary, ctx);
    }

    // Try BETWEEN expression - always returns Boolean
    if expr.as_between().is_some() {
        return Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: true, // Could be NULL if any operand is NULL
        });
    }

    // Try IN expression - always returns Boolean
    if expr.as_in().is_some() {
        return Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: true, // Could be NULL if expr or any value is NULL
        });
    }

    // Try EXISTS expression - always returns Boolean (never NULL)
    if expr.as_exists().is_some() {
        return Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: false, // EXISTS always returns TRUE or FALSE, never NULL
        });
    }

    // Try list comprehension (must precede array literal: a comprehension's
    // outer node also satisfies as_array_literal via its nested source list,
    // but LIST_COMPREHENSION is a distinct top-level node kind — checking it
    // first keeps the dispatch on the actually-matching node).
    if let Some(comp) = expr.as_list_comprehension() {
        return infer_list_comprehension_type(&comp, ctx);
    }

    // Try array literal
    if let Some(array_lit) = expr.as_array_literal() {
        return infer_array_literal_type(&array_lit, ctx);
    }

    // Try array subscript
    if let Some(_subscript) = expr.as_array_subscript() {
        return infer_array_subscript_type(expr, ctx);
    }

    // Try array slice
    if let Some(_slice) = expr.as_array_slice() {
        return infer_array_slice_type(expr, ctx);
    }

    // Try ROW constructor
    if let Some(row) = expr.as_row_constructor() {
        return infer_row_constructor_type(&row, ctx);
    }

    // Try struct literal
    if let Some(struct_lit) = expr.as_struct_literal() {
        return infer_struct_literal_type(&struct_lit, ctx);
    }

    // Try MAP literal (DuckDB `MAP {'a': 1, 'b': 2}`)
    if let Some(map_lit) = expr.as_map_literal() {
        return infer_map_literal_type(&map_lit, ctx);
    }

    // Try column reference (includes struct field access for qualified refs like s.field_name)
    if let Some(col_ref) = expr.as_column_ref() {
        // Use `lookup_identifier` so that seeded function parameters (§16 #1)
        // resolve before any SQL FROM scope. For `TypeContext`s with no
        // function params seeded (the common case — all pre-Phase-5 call
        // sites), this is semantically identical to `lookup_column`.
        if let Some(typed_col) = ctx.lookup_identifier(col_ref.qualifier(), col_ref.name()) {
            return Some(typed_col.clone());
        }
        // If qualified ref didn't resolve as a column, try struct field access:
        // treat qualifier as a column name and name as a field name
        if let Some(qualifier) = col_ref.qualifier() {
            if let Some(struct_col) = ctx.lookup_column(None, qualifier) {
                if let DataType::Struct(fields) = &struct_col.data_type {
                    let field_lower = col_ref.name().to_lowercase();
                    for (name, dt) in fields {
                        if name.to_lowercase() == field_lower {
                            return Some(TypedColumn {
                                data_type: dt.clone(),
                                nullable: true, // Field access may be null
                            });
                        }
                    }
                }
            }
        }
        // Qualified column reference (e.g. "p.product_id") that couldn't be resolved —
        // return None rather than falling through to infer_literal_type which would
        // misinterpret the dot as a decimal point.
        // Unqualified refs (e.g. "INTERVAL") must still fall through so that
        // infer_literal_type can recognize typed literals like INTERVAL '1' DAY.
        if col_ref.qualifier().is_some() {
            return None;
        }
    }

    // Try literal inference (also handles typed literals like DATE '2025-01-15')
    infer_literal_type(&text)
}

/// Infer the [`ExprKind`] (Scalar / Agg / Window) of an expression
/// (Phase 14, §16 #24).
///
/// The kind is synthesised in the same pure pass as [`infer_expression_type`]
/// — column references and literals are `Scalar`; arithmetic / case / cast
/// take the ceiling of their sub-kinds; function calls consult the
/// [`BuiltinRegistry`]. A call site that carries an `OVER (…)` clause
/// produces [`ExprKind::Window`] regardless of the callee's seeded kind
/// (the canonical SQL dual-mode behaviour for aggregates).
///
/// Pure: no Salsa access, deterministic, side-effect free.
pub fn infer_expression_kind(expr: &Expr, ctx: &TypeContext) -> ExprKind {
    // Any expression with an attached OVER (...) clause is a window
    // expression. This dominates the callee's seeded kind — `SUM(x) OVER
    // (...)` is `Window`, not `Agg`.
    if expr.window_spec().is_some() {
        return ExprKind::Window;
    }

    // CAST(<inner> AS T) inherits the inner expression's kind.
    if let Some(cast_expr) = expr.as_cast() {
        return cast_expr
            .expression()
            .as_ref()
            .map(|inner| infer_expression_kind(inner, ctx))
            .unwrap_or(ExprKind::Scalar);
    }

    // CASE: ceiling over WHEN result branches and the optional ELSE.
    if let Some(case_expr) = expr.as_case() {
        let mut kinds: Vec<ExprKind> = Vec::new();
        for when in case_expr.when_clauses() {
            if let Some(result) = when.result() {
                kinds.push(infer_expression_kind(&result, ctx));
            }
            if let Some(cond) = when.condition() {
                kinds.push(infer_expression_kind(&cond, ctx));
            }
        }
        if let Some(else_expr) = case_expr.else_expr() {
            kinds.push(infer_expression_kind(&else_expr, ctx));
        }
        return kind_ceiling(&kinds);
    }

    // EXTRACT(field FROM expr) inherits the inner expression's kind.
    if let Some(extract_expr) = expr.as_extract() {
        return extract_expr
            .expression()
            .as_ref()
            .map(|inner| infer_expression_kind(inner, ctx))
            .unwrap_or(ExprKind::Scalar);
    }

    // Subquery: scalar — the subquery's inner kinds are checked against
    // its own splice points, not propagated outward. The subquery itself
    // is a Scalar value at the outer position.
    if expr.as_subquery().is_some() {
        return ExprKind::Scalar;
    }

    // SQL built-in / aggregate / window function call.
    if let Some(func) = expr.as_function_call() {
        return infer_function_call_kind(&func, ctx);
    }

    // smelt.functions.* user-defined call: scalar today (kind tracking
    // through user-defined fragments is a later phase). Until then, treat
    // as the most permissive kind so call sites in WHERE don't false-positive.
    if expr.as_smelt_path_call().is_some() {
        return ExprKind::Scalar;
    }

    // Binary expr: ceiling over LHS and RHS.
    if let Some(binary) = expr.as_binary() {
        let lhs = binary
            .left()
            .as_ref()
            .map(|e| infer_expression_kind(e, ctx))
            .unwrap_or(ExprKind::Scalar);
        let rhs = binary
            .right()
            .as_ref()
            .map(|e| infer_expression_kind(e, ctx))
            .unwrap_or(ExprKind::Scalar);
        return kind_ceiling(&[lhs, rhs]);
    }

    // BETWEEN / IN / EXISTS / array / row / struct: walk children and
    // take their ceiling. Most are scalar but if any sub-expr is Agg or
    // Window the wrapper inherits it.
    let mut kinds: Vec<ExprKind> = Vec::new();
    for child in expr.syntax().children() {
        if let Some(child_expr) = Expr::cast(child) {
            kinds.push(infer_expression_kind(&child_expr, ctx));
        }
    }
    if !kinds.is_empty() {
        return kind_ceiling(&kinds);
    }

    // Column refs, literals, identifiers — Scalar.
    // Phase 44b exception: a bare unqualified identifier that matches a
    // registered fragment-typed parameter inherits that parameter's declared
    // kind. This lets `PASSING metrics AS (metrics)` forward a
    // `SelectItems<Agg>` parameter without producing a `FragmentKindMismatch`.
    if let Some(col_ref) = expr.as_column_ref() {
        if col_ref.qualifier().is_none() {
            if let Some(kind) = ctx.lookup_fragment_param_kind(col_ref.name()) {
                return kind;
            }
        }
    }
    ExprKind::Scalar
}

/// Compute the [`ExprKind`] of a SQL function-call site.
///
/// Looks the function up in the [`BuiltinRegistry`] for its seeded kind.
/// Unknown functions fall back to [`ExprKind::Scalar`]. (Aggregates with
/// an attached `OVER (…)` clause are handled by the caller — see
/// [`infer_expression_kind`]'s window check.)
fn infer_function_call_kind(func: &FunctionCall, _ctx: &TypeContext) -> ExprKind {
    let Some(name) = func.name() else {
        return ExprKind::Scalar;
    };
    let upper = name.to_uppercase();
    BuiltinRegistry::resolve(&upper)
        .map(|sig| sig.kind)
        .unwrap_or(ExprKind::Scalar)
}

/// Structured info about a window-in-scalar-context error (Phase 14).
///
/// Returned by [`check_window_in_scalar_contexts`] for each WHERE /
/// GROUP BY position whose expression resolves to [`ExprKind::Window`].
/// The caller (`check_file_diagnostics`) maps these into
/// [`crate::DiagnosticCode::WindowInScalarContext`] entries.
#[derive(Debug, Clone)]
pub struct WindowInScalarContextInfo {
    /// Free-form clause name (`"WHERE"`, `"GROUP BY"`) for the message.
    pub clause: &'static str,
    /// Source span of the offending expression.
    pub range: TextRange,
    /// Trimmed text of the offending expression — quoted in the message.
    pub expression_text: String,
}

/// Pure check: collect every expression in WHERE / GROUP BY / HAVING whose
/// synthesised kind is [`ExprKind::Window`] (Phase 14, §16 #24).
///
/// Also recurses into scalar subqueries nested inside those clauses so that
/// `WHERE col > (SELECT ROW_NUMBER() OVER (...) FROM t)` is flagged as a
/// `"WHERE"` violation (Phase 49).
///
/// FROM-clause subqueries are intentionally excluded: they are not scalar
/// contexts, and window functions are valid inside derived-table SELECT lists.
pub fn check_window_in_scalar_contexts(
    select_stmt: &SelectStmt,
    ctx: &TypeContext,
) -> Vec<WindowInScalarContextInfo> {
    let mut out = Vec::new();

    if let Some(where_clause) = select_stmt.where_clause() {
        if let Some(expr) = where_clause.expression() {
            check_expr_and_scalar_subqueries(&expr, "WHERE", ctx, &mut out);
        }
    }

    if let Some(group_by) = select_stmt.group_by_clause() {
        for expr in group_by.expressions() {
            check_expr_and_scalar_subqueries(&expr, "GROUP BY", ctx, &mut out);
        }
    }

    if let Some(having_clause) = select_stmt.having_clause() {
        if let Some(expr) = having_clause.expression() {
            check_expr_and_scalar_subqueries(&expr, "HAVING", ctx, &mut out);
        }
    }

    out
}

/// Check a single expression in a scalar clause (WHERE / GROUP BY / HAVING):
///
/// 1. If the expression itself is `Window`-kinded, emit an error.
/// 2. Recursively descend into any scalar subqueries found within the
///    expression tree. For each scalar subquery found:
///    - Check its SELECT list for Window-kinded expressions (a window function
///      inside a scalar-subquery SELECT list is invalid because the subquery
///      must return a scalar value).
///    - Call [`check_window_in_scalar_contexts`] on it to catch violations in
///      its nested WHERE / GROUP BY / HAVING clauses.
///
/// FROM-clause subqueries are **not** visited here — they live under
/// `TABLE_REF` nodes, which are children of `FROM_CLAUSE`, which is a child
/// of `SELECT_STMT`, not of an expression node.  Since we only enter this
/// function from expression positions (WHERE / GROUP BY / HAVING), every
/// `SUBQUERY` descendant of `expr` is guaranteed to be a scalar subquery.
fn check_expr_and_scalar_subqueries(
    expr: &Expr,
    clause: &'static str,
    ctx: &TypeContext,
    out: &mut Vec<WindowInScalarContextInfo>,
) {
    use smelt_parser::SyntaxKind;

    // Top-level: if this expression is Window-kinded, report it directly.
    if infer_expression_kind(expr, ctx) == ExprKind::Window {
        out.push(WindowInScalarContextInfo {
            clause,
            range: expr.text_range(),
            expression_text: expr.text().trim().to_string(),
        });
    }

    // Recurse into scalar subqueries nested inside this expression.
    // All SUBQUERY nodes in an expression tree are scalar contexts (they are
    // not FROM-clause derived tables), so we check their inner SELECT
    // statements with the same outer clause name.
    for node in expr.syntax().descendants() {
        if node.kind() == SyntaxKind::SUBQUERY {
            if let Some(subquery) = Subquery::cast(node) {
                if let Some(inner_select) = subquery.select_stmt() {
                    // (a) Check the inner SELECT's own SELECT list: a window
                    // function in the select list of a scalar subquery is
                    // invalid because the subquery must produce a scalar value.
                    check_scalar_subquery_select_list(&inner_select, clause, out);

                    // (b) Recurse into the inner SELECT's WHERE/GROUP BY/HAVING
                    // clauses (and any further nested scalar subqueries there).
                    out.extend(check_window_in_scalar_contexts(&inner_select, ctx));
                }
            }
        }
    }
}

/// For a [`SelectStmt`] that appears as a scalar subquery, check each item in
/// its SELECT list. If any item contains a Window-kinded expression (directly
/// or buried inside an aggregate wrapping a window call), emit an entry with
/// `clause` preserved from the outer scalar context.
///
/// This is needed because `infer_expression_kind` treats outer function calls
/// by their registry kind (e.g. `MAX(ROW_NUMBER() OVER (...))` → `Agg`), so a
/// raw top-level kind check would miss the nested window expression. Here we
/// walk the expression's descendants looking for any node with an OVER clause.
fn check_scalar_subquery_select_list(
    inner_select: &SelectStmt,
    clause: &'static str,
    out: &mut Vec<WindowInScalarContextInfo>,
) {
    use smelt_parser::SyntaxKind;

    let Some(select_list) = inner_select.select_list() else {
        return;
    };
    for item in select_list.items() {
        let Some(item_expr) = item.expression() else {
            continue;
        };
        // Walk every descendant of this select-item expression looking for
        // any EXPRESSION node that carries an OVER clause (i.e. a window
        // function call).
        //
        // We look for EXPRESSION nodes (not FUNCTION_CALL) because the parser
        // puts the WINDOW_SPEC as a sibling of the FUNCTION_CALL inside a
        // parent EXPRESSION: `EXPRESSION { FUNCTION_CALL { ARG_LIST } WINDOW_SPEC }`.
        // An `Expr` wrapping that EXPRESSION node will find the WINDOW_SPEC via
        // `window_spec()`, while an `Expr` wrapping the inner FUNCTION_CALL won't.
        //
        // We do NOT use `infer_expression_kind` on the top-level item because an
        // aggregate wrapping a window call (e.g. `MAX(ROW_NUMBER() OVER (...))`)
        // would be classified as `Agg` by the registry lookup, hiding the inner
        // window function.
        for desc_node in item_expr.syntax().descendants() {
            if desc_node.kind() == SyntaxKind::EXPRESSION {
                if let Some(desc_expr) = Expr::cast(desc_node) {
                    if desc_expr.window_spec().is_some() {
                        out.push(WindowInScalarContextInfo {
                            clause,
                            range: desc_expr.text_range(),
                            expression_text: desc_expr.text().trim().to_string(),
                        });
                        // One hit per select item is sufficient.
                        break;
                    }
                }
            }
        }
    }
}

/// Phase 46: infer the output schema of a SELECT statement (shared
/// helper used by CTE inference and by `TableExpr` argument resolution
/// for derived tables / inline subqueries).
///
/// Walks the SELECT list, deriving each column's name (from explicit
/// `AS alias`, falling back to a name inferred from the expression, or
/// a generated `colN` when neither applies) and inferring its type via
/// `infer_expression_type` against a context that includes any nested
/// `WITH` clauses in the SELECT.
pub fn infer_select_output_schema(
    select_stmt: &SelectStmt,
    ctx: &TypeContext,
) -> Vec<(String, TypedColumn)> {
    let mut columns = Vec::new();

    // Build a context that includes any nested CTEs in this SELECT
    let inner_ctx = build_subquery_context(select_stmt, ctx);

    let select_list = match select_stmt.select_list() {
        Some(l) => l,
        None => return columns,
    };

    for (i, item) in select_list.items().enumerate() {
        // Handle `smelt.functions.f(...).*` struct-spread items.
        //
        // These produce SMELT_PATH_CALL_STAR nodes with no single column
        // name. They must expand to N typed columns (one per struct field)
        // so that struct fields propagate through CTE bodies transparently
        // (function_schema_inference.md §"Struct returns and .* spread",
        // §"Propagation through CTEs, subqueries, and joins").
        if let Some(expr) = item.expression() {
            if let Some(expanded) = try_expand_struct_spread_item(expr.syntax(), &inner_ctx) {
                columns.extend(expanded);
                continue;
            }
        }

        let col_name = if let Some(alias) = item.alias() {
            alias
        } else if let Some(expr) = item.expression() {
            infer_column_name(&expr).unwrap_or_else(|| format!("col{}", i + 1))
        } else {
            format!("col{}", i + 1)
        };

        let typed_col = if let Some(expr) = item.expression() {
            infer_expression_type(&expr, &inner_ctx).unwrap_or(TypedColumn {
                data_type: DataType::Unknown(smelt_types::UnknownReason::Dynamic),
                nullable: true,
            })
        } else {
            TypedColumn {
                data_type: DataType::Unknown(smelt_types::UnknownReason::Dynamic),
                nullable: true,
            }
        };

        columns.push((col_name, typed_col));
    }

    columns
}

/// If `expr_node` contains a `SMELT_PATH_CALL_STAR` wrapping a closed-struct
/// returning function, expand it into `(field_name, TypedColumn)` pairs.
///
/// Returns `None` when the expression is not a struct spread, allowing
/// the caller to fall through to the normal single-column path.
///
/// Mirrors `collect_struct_spread_columns` in `schema.rs` for the CTE /
/// subquery context (function_schema_inference.md §"Struct returns and .*
/// spread" and §"Propagation through CTEs, subqueries, and joins").
/// Pure — no Salsa access.
fn try_expand_struct_spread_item(
    expr_node: &smelt_parser::syntax_kind::SyntaxNode,
    ctx: &TypeContext,
) -> Option<Vec<(String, TypedColumn)>> {
    use crate::type_inference::function_call::infer_smelt_path_call_type;
    use smelt_parser::ast::SmeltPathCall;
    use smelt_parser::SyntaxKind::{SMELT_PATH_CALL, SMELT_PATH_CALL_STAR};
    use smelt_types::signatures::{SmeltType, StructRowTail};

    // Find a SMELT_PATH_CALL_STAR child of the expression node.
    let inner_call: SmeltPathCall = expr_node.children().find_map(|child| {
        if child.kind() == SMELT_PATH_CALL_STAR {
            child.children().find_map(|inner| {
                if inner.kind() == SMELT_PATH_CALL {
                    SmeltPathCall::cast(inner)
                } else {
                    None
                }
            })
        } else {
            None
        }
    })?;

    // Only expand closed structs (no row-tail marker).
    let fn_name = inner_call.segments().last().cloned().unwrap_or_default();
    let is_closed_struct = ctx
        .lookup_function_signature(&fn_name)
        .map(|sig| {
            matches!(
                &sig.return_type,
                Some(Ok(SmeltType::Struct {
                    tail: StructRowTail::None,
                    ..
                }))
            )
        })
        .unwrap_or(false);
    if !is_closed_struct {
        return None;
    }

    // Resolve the call's return type and expand struct fields.
    let typed = infer_smelt_path_call_type(&inner_call, ctx)?;
    if let DataType::Struct(fields) = typed.data_type {
        let cols = fields
            .into_iter()
            .map(|(name, dt)| {
                (
                    name,
                    TypedColumn {
                        data_type: dt,
                        nullable: true,
                    },
                )
            })
            .collect();
        Some(cols)
    } else {
        None
    }
}

/// Infer a column name from an expression
///
/// For simple column references, returns the column name.
/// For function calls, returns the function name.
/// For other expressions, returns None.
fn infer_column_name(expr: &Expr) -> Option<String> {
    // Try column reference
    if let Some(col_ref) = expr.as_column_ref() {
        return Some(col_ref.name().to_string());
    }

    // Try EXTRACT expression
    if let Some(_extract) = expr.as_extract() {
        return Some("extract".to_string());
    }

    // Try CASE expression — no natural name, but return a placeholder
    if expr.as_case().is_some() {
        return Some("case_expr".to_string());
    }

    // Try function call - use function name
    if let Some(func) = expr.as_function_call() {
        return func.name();
    }

    // For other expressions, we can't infer a name
    None
}

/// Promote two types to their widest compatible type for UNION operations.
///
/// The result type is the type that can hold values from both input types.
/// For example:
/// - INTEGER + BIGINT → BIGINT
/// - VARCHAR(10) + VARCHAR(20) → Text (we don't track length)
/// - INTEGER + DOUBLE → DOUBLE
/// - Unknown + T → T (Unknown is dominated by any known type)
pub fn promote_types(t1: &TypedColumn, t2: &TypedColumn) -> TypedColumn {
    // If either is Unknown or Null, prefer the other (Null makes result nullable)
    if matches!(t1.data_type, DataType::Unknown(_) | DataType::Null) {
        return TypedColumn {
            data_type: t2.data_type.clone(),
            nullable: t1.nullable || t2.nullable || matches!(t1.data_type, DataType::Null),
        };
    }
    if matches!(t2.data_type, DataType::Unknown(_) | DataType::Null) {
        return TypedColumn {
            data_type: t1.data_type.clone(),
            nullable: t1.nullable || t2.nullable || matches!(t2.data_type, DataType::Null),
        };
    }

    // If same type, return it
    if std::mem::discriminant(&t1.data_type) == std::mem::discriminant(&t2.data_type) {
        // For decimals, take the larger precision/scale
        if let (
            DataType::Decimal {
                precision: p1,
                scale: s1,
            },
            DataType::Decimal {
                precision: p2,
                scale: s2,
            },
        ) = (&t1.data_type, &t2.data_type)
        {
            return TypedColumn {
                data_type: DataType::Decimal {
                    precision: (*p1).max(*p2),
                    scale: (*s1).max(*s2),
                },
                nullable: t1.nullable || t2.nullable,
            };
        }
        return TypedColumn {
            data_type: t1.data_type.clone(),
            nullable: t1.nullable || t2.nullable,
        };
    }

    // Check if both types are in the same family before cross-type promotion
    let both_numeric = t1.data_type.is_numeric() && t2.data_type.is_numeric();
    let both_string = t1.data_type.is_string() && t2.data_type.is_string();
    let both_temporal = t1.data_type.is_temporal() && t2.data_type.is_temporal();

    let promoted_type = match (&t1.data_type, &t2.data_type) {
        // Numeric type promotion: SmallInt < Integer < BigInt < Float < Decimal < Double
        _ if both_numeric => match (&t1.data_type, &t2.data_type) {
            (DataType::Double, _) | (_, DataType::Double) => DataType::Double,
            (DataType::Float, _) | (_, DataType::Float) => DataType::Float,
            // When a Decimal combines with an integer type, widen to Decimal(38,10)
            // to avoid overflow. E.g. CASE WHEN ... THEN 150::INTEGER ELSE 0.5::DECIMAL(2,1)
            // should not produce DECIMAL(2,1) which can only hold up to 9.9.
            (
                DataType::Decimal { .. },
                DataType::SmallInt | DataType::Integer | DataType::BigInt,
            )
            | (
                DataType::SmallInt | DataType::Integer | DataType::BigInt,
                DataType::Decimal { .. },
            ) => DataType::Decimal {
                precision: 38,
                scale: 10,
            },
            (DataType::Decimal { precision, scale }, _)
            | (_, DataType::Decimal { precision, scale }) => DataType::Decimal {
                precision: *precision,
                scale: *scale,
            },
            (DataType::BigInt, _) | (_, DataType::BigInt) => DataType::BigInt,
            (DataType::Integer, _) | (_, DataType::Integer) => DataType::Integer,
            _ => t1.data_type.clone(),
        },

        // String type promotion: all string types → Text
        _ if both_string => DataType::Text,

        // Temporal type promotion
        _ if both_temporal => match (&t1.data_type, &t2.data_type) {
            (
                DataType::Timestamp { with_timezone: tz1 },
                DataType::Timestamp { with_timezone: tz2 },
            ) => {
                if tz1 == tz2 {
                    // Same tz variant: keep it.
                    DataType::Timestamp {
                        with_timezone: *tz1,
                    }
                } else {
                    // Mixed naive / tz-aware: strict rejection. The caller that
                    // walks CASE branches or UNION columns must emit the
                    // TypeMismatch diagnostic; promote_types is pure and cannot
                    // push diagnostics itself.
                    DataType::unknown_unresolved()
                }
            }
            (DataType::Timestamp { with_timezone }, _)
            | (_, DataType::Timestamp { with_timezone }) => DataType::Timestamp {
                with_timezone: *with_timezone,
            },
            (DataType::Date, DataType::Time) | (DataType::Time, DataType::Date) => {
                DataType::Timestamp {
                    with_timezone: false,
                }
            }
            _ => DataType::Unknown(smelt_types::UnknownReason::Dynamic),
        },

        // For incompatible type families, return Unknown (could be an error in strict mode)
        _ => DataType::Unknown(smelt_types::UnknownReason::Dynamic),
    };

    TypedColumn {
        data_type: promoted_type,
        nullable: t1.nullable || t2.nullable,
    }
}

/// Infer column types for a SELECT statement, handling UNION if present.
///
/// For a simple SELECT, returns the types of each column in the select list.
/// For a UNION, combines types from all branches using type promotion.
pub fn infer_select_column_types(select_stmt: &SelectStmt, ctx: &TypeContext) -> Vec<TypedColumn> {
    let mut column_types = Vec::new();

    // Get types from the first SELECT's select list
    if let Some(select_list) = select_stmt.select_list() {
        for item in select_list.items() {
            let typed_col = if let Some(expr) = item.expression() {
                infer_expression_type(&expr, ctx).unwrap_or(TypedColumn {
                    data_type: DataType::Unknown(smelt_types::UnknownReason::Dynamic),
                    nullable: true,
                })
            } else {
                TypedColumn {
                    data_type: DataType::Unknown(smelt_types::UnknownReason::Dynamic),
                    nullable: true,
                }
            };
            column_types.push(typed_col);
        }
    }

    // If there's a set operation (UNION/INTERSECT/EXCEPT), recursively get types and combine
    if select_stmt.has_set_operation() {
        if let Some(next_select) = select_stmt.set_operation_select() {
            let next_types = infer_select_column_types(&next_select, ctx);

            // Combine types - use the wider type for each column position
            for (i, next_type) in next_types.into_iter().enumerate() {
                if i < column_types.len() {
                    column_types[i] = promote_types(&column_types[i], &next_type);
                }
                // If next has more columns, they're ignored (SQL requires same column count)
            }
        }
    }

    column_types
}

/// Walk the set-operation chain of a SELECT statement and emit one
/// `TypeMismatch` Error at the UNION/INTERSECT/EXCEPT keyword span for each
/// column position where one branch carries a naive `Timestamp` and the
/// other carries `Timestamp WITH TIME ZONE` (spec §16 — strict mixing rule).
///
/// Only the direct (top-level) set operation is checked per call; the Salsa
/// orchestrator already iterates over every top-level SELECT per model, so
/// nested set ops surface when their inner SELECT is processed as a model.
///
/// This function is PURE — no Salsa calls; it uses only the AST and the
/// TypeContext provided by the caller.
pub fn check_mixed_tz_setop_diagnostics(
    select_stmt: &SelectStmt,
    ctx: &TypeContext,
) -> Vec<crate::Diagnostic> {
    use smelt_parser::SyntaxKind::{EXCEPT_KW, INTERSECT_KW, UNION_KW};

    let mut diags: Vec<crate::Diagnostic> = Vec::new();

    if !select_stmt.has_set_operation() {
        return diags;
    }

    let next_select = match select_stmt.set_operation_select() {
        Some(s) => s,
        None => return diags,
    };

    // Collect types from the left branch (this SELECT).
    let left_types = if let Some(select_list) = select_stmt.select_list() {
        select_list
            .items()
            .map(|item| {
                item.expression()
                    .and_then(|e| infer_expression_type(&e, ctx))
            })
            .collect::<Vec<_>>()
    } else {
        return diags;
    };

    // Collect types from the right branch.
    let right_types = if let Some(select_list) = next_select.select_list() {
        select_list
            .items()
            .map(|item| {
                item.expression()
                    .and_then(|e| infer_expression_type(&e, ctx))
            })
            .collect::<Vec<_>>()
    } else {
        return diags;
    };

    // Find the set-operator token range for the diagnostic span.
    let op_range = select_stmt
        .syntax()
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| matches!(t.kind(), UNION_KW | INTERSECT_KW | EXCEPT_KW))
        .map(|t| t.text_range())
        .unwrap_or_else(|| select_stmt.syntax().text_range());

    // For each column position, check for a naive/tz-aware mismatch.
    for (i, (l, r)) in left_types.iter().zip(right_types.iter()).enumerate() {
        let l_dt = match l {
            Some(tc) if !matches!(tc.data_type, DataType::Unknown(_)) => &tc.data_type,
            _ => continue,
        };
        let r_dt = match r {
            Some(tc) if !matches!(tc.data_type, DataType::Unknown(_)) => &tc.data_type,
            _ => continue,
        };

        let (tz_l, tz_r) = match (l_dt, r_dt) {
            (
                DataType::Timestamp {
                    with_timezone: tz_l,
                },
                DataType::Timestamp {
                    with_timezone: tz_r,
                },
            ) => (*tz_l, *tz_r),
            _ => continue,
        };

        if tz_l != tz_r {
            diags.push(crate::Diagnostic {
                severity: crate::DiagnosticSeverity::Error,
                message: format!(
                    "Timezone mismatch in set operation: column {} mixes naive Timestamp and \
                     Timestamp WITH TIME ZONE; add an explicit CAST to align timezone variants",
                    i + 1
                ),
                range: op_range,
                code: Some(crate::DiagnosticCode::TypeMismatch),
                data: None,
            });
        }
    }

    diags
}

/// Walk all CASE expressions in a SELECT statement and emit one `TypeMismatch`
/// Error at the CASE keyword span when any pair of THEN/ELSE branches mixes a
/// naive `Timestamp` with a `Timestamp WITH TIME ZONE` (spec §16 — strict
/// mixing rule).
///
/// This function is PURE — no Salsa calls.
pub fn check_mixed_tz_case_diagnostics(
    select_stmt: &SelectStmt,
    ctx: &TypeContext,
) -> Vec<crate::Diagnostic> {
    use smelt_parser::SyntaxKind::CASE_EXPR;

    let mut diags: Vec<crate::Diagnostic> = Vec::new();
    let root = select_stmt.syntax();

    for node in root.descendants() {
        if node.kind() != CASE_EXPR {
            continue;
        }
        let case_expr = match CaseExpr::cast(node.clone()) {
            Some(c) => c,
            None => continue,
        };

        // Collect inferred types of all result branches (THEN + ELSE).
        let mut branch_types: Vec<DataType> = Vec::new();

        for when_clause in case_expr.when_clauses() {
            if let Some(result_expr) = when_clause.result() {
                if let Some(tc) = infer_expression_type(&result_expr, ctx) {
                    if !matches!(tc.data_type, DataType::Unknown(_) | DataType::Null) {
                        branch_types.push(tc.data_type.clone());
                    }
                }
            }
        }
        if let Some(else_expr) = case_expr.else_expr() {
            if let Some(tc) = infer_expression_type(&else_expr, ctx) {
                if !matches!(tc.data_type, DataType::Unknown(_) | DataType::Null) {
                    branch_types.push(tc.data_type.clone());
                }
            }
        }

        // Check if any branch is naive Timestamp and any other is tz-aware.
        let has_naive = branch_types.iter().any(|dt| {
            matches!(
                dt,
                DataType::Timestamp {
                    with_timezone: false
                }
            )
        });
        let has_tz_aware = branch_types.iter().any(|dt| {
            matches!(
                dt,
                DataType::Timestamp {
                    with_timezone: true
                }
            )
        });

        if has_naive && has_tz_aware {
            // Anchor at the CASE node itself (the CASE keyword).
            let range = node.text_range();
            diags.push(crate::Diagnostic {
                severity: crate::DiagnosticSeverity::Error,
                message:
                    "Timezone mismatch in CASE: branches mix naive Timestamp and \
                          Timestamp WITH TIME ZONE; add an explicit CAST to align timezone variants"
                        .to_string(),
                range,
                code: Some(crate::DiagnosticCode::TypeMismatch),
                data: None,
            });
        }
    }

    diags
}
