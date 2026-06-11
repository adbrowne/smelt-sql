//! Binary expression type inference plus column-walking visitors and CTE/undeclared-column checks.

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

/// Infer the type of a binary operand by finding the nth Expr child node.
fn infer_binary_operand(binary: &BinaryExpr, nth: usize, ctx: &TypeContext) -> Option<TypedColumn> {
    let expr = binary.node().children().filter_map(Expr::cast).nth(nth)?;
    infer_expression_type(&expr, ctx)
}

/// Promote two numeric operands to their common widest type.
/// Priority: Double > Float > Decimal > BigInt > Integer > SmallInt
///
/// Cross-family guard: if both operands are known and either is not numeric,
/// the pair is incompatible — return `Unknown` instead of the left type.
/// This implements spec §1 "no implicit cross-family cast" and §14
/// strict-rejection examples (`42 + '3'` → Unknown; `TRUE + 1` → Unknown).
/// Temporal arithmetic is handled by early-return arms in the callers,
/// so by the time this function is reached both operands should be numeric.
fn promote_numeric_operands(
    left: Option<DataType>,
    right: Option<DataType>,
) -> Option<TypedColumn> {
    if let (Some(ref l), Some(ref r)) = (&left, &right) {
        if !l.is_numeric() || !r.is_numeric() {
            return Some(TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            });
        }
    }

    match (left, right) {
        (Some(DataType::Double), _) | (_, Some(DataType::Double)) => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),
        (Some(DataType::Float), _) | (_, Some(DataType::Float)) => Some(TypedColumn {
            data_type: DataType::Float,
            nullable: true,
        }),
        (Some(DataType::Decimal { .. }), _) | (_, Some(DataType::Decimal { .. })) => {
            Some(TypedColumn {
                data_type: DataType::Decimal {
                    precision: 38,
                    scale: 10,
                },
                nullable: true,
            })
        }
        (Some(DataType::BigInt), _) | (_, Some(DataType::BigInt)) => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: true,
        }),
        (Some(DataType::Integer), _) | (_, Some(DataType::Integer)) => Some(TypedColumn {
            data_type: DataType::Integer,
            nullable: true,
        }),
        (Some(DataType::SmallInt), _) | (_, Some(DataType::SmallInt)) => Some(TypedColumn {
            data_type: DataType::SmallInt,
            nullable: true,
        }),
        (Some(l), _) => Some(TypedColumn {
            data_type: l,
            nullable: true,
        }),
        _ => None,
    }
}

/// Infer the result type of a binary expression
pub fn infer_binary_expr_type(binary: &BinaryExpr, ctx: &TypeContext) -> Option<TypedColumn> {
    let op = binary.operator()?;

    match op.to_uppercase().as_str() {
        // Logical operators - always return Boolean
        "AND" | "OR" => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: true,
        }),

        // NOT operator (unary) - always returns Boolean
        "NOT" => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: true, // NOT NULL = NULL
        }),

        // Comparison operators - always return Boolean
        "=" | "<>" | "!=" | "<" | ">" | "<=" | ">=" | "IS" => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: false, // Comparisons always return true/false
        }),

        // Pattern matching operators - always return Boolean
        "LIKE" | "ILIKE" | "~" | "~*" | "!~" | "!~*" => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: true,
        }),

        // String concatenation - always returns Text
        "||" => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        // Addition — handles numeric promotion and temporal arithmetic
        "+" => {
            let left = infer_binary_operand(binary, 0, ctx);
            let right = infer_binary_operand(binary, 1, ctx);
            let lt = left.as_ref().map(|t| &t.data_type);
            let rt = right.as_ref().map(|t| &t.data_type);

            // Temporal arithmetic for +
            match (lt, rt) {
                // DATE + INTERVAL → Timestamp, INTERVAL + DATE → Timestamp
                (Some(DataType::Date), Some(DataType::Interval))
                | (Some(DataType::Interval), Some(DataType::Date)) => {
                    return Some(TypedColumn {
                        data_type: DataType::Timestamp {
                            with_timezone: false,
                        },
                        nullable: true,
                    });
                }
                // TIMESTAMP + INTERVAL → Timestamp, INTERVAL + TIMESTAMP → Timestamp
                (Some(DataType::Timestamp { with_timezone }), Some(DataType::Interval))
                | (Some(DataType::Interval), Some(DataType::Timestamp { with_timezone })) => {
                    return Some(TypedColumn {
                        data_type: DataType::Timestamp {
                            with_timezone: *with_timezone,
                        },
                        nullable: true,
                    });
                }
                // TIME + INTERVAL → Time, INTERVAL + TIME → Time
                (Some(DataType::Time), Some(DataType::Interval))
                | (Some(DataType::Interval), Some(DataType::Time)) => {
                    return Some(TypedColumn {
                        data_type: DataType::Time,
                        nullable: true,
                    });
                }
                // INTERVAL + INTERVAL → Interval
                (Some(DataType::Interval), Some(DataType::Interval)) => {
                    return Some(TypedColumn {
                        data_type: DataType::Interval,
                        nullable: true,
                    });
                }
                _ => {}
            }

            // Numeric promotion
            Some(promote_numeric_operands(
                left.map(|t| t.data_type),
                right.map(|t| t.data_type),
            )?)
        }

        // Multiplication, division, and modulo — handles numeric promotion and INTERVAL * numeric
        "*" | "/" | "%" => {
            let left = infer_binary_operand(binary, 0, ctx);
            let right = infer_binary_operand(binary, 1, ctx);
            let lt = left.as_ref().map(|t| &t.data_type);
            let rt = right.as_ref().map(|t| &t.data_type);

            // INTERVAL * numeric → Interval, numeric * INTERVAL → Interval
            // INTERVAL / numeric → Interval
            match (lt, rt) {
                (Some(DataType::Interval), Some(r)) if r.is_numeric() => {
                    return Some(TypedColumn {
                        data_type: DataType::Interval,
                        nullable: true,
                    });
                }
                (Some(l), Some(DataType::Interval)) if l.is_numeric() => {
                    return Some(TypedColumn {
                        data_type: DataType::Interval,
                        nullable: true,
                    });
                }
                _ => {}
            }

            // Numeric promotion
            Some(promote_numeric_operands(
                left.map(|t| t.data_type),
                right.map(|t| t.data_type),
            )?)
        }

        // Minus can be binary (a - b) or unary (-a)
        "-" => {
            if binary.is_unary() {
                // Unary minus: -expr preserves the numeric type
                // First try to get operand type from expression
                if let Some(operand_type) =
                    binary.left().and_then(|e| infer_expression_type(&e, ctx))
                {
                    return Some(TypedColumn {
                        data_type: operand_type.data_type,
                        nullable: operand_type.nullable,
                    });
                }

                // For unary expressions with bare identifier operands, look up the column
                if let Some(col_ref) = binary.unary_operand_column() {
                    if let Some(typed_col) = ctx.lookup_column(col_ref.qualifier(), col_ref.name())
                    {
                        return Some(TypedColumn {
                            data_type: typed_col.data_type.clone(),
                            nullable: typed_col.nullable,
                        });
                    }
                }

                None
            } else {
                // Binary minus: a - b
                let left = infer_binary_operand(binary, 0, ctx);
                let right = infer_binary_operand(binary, 1, ctx);
                let lt = left.as_ref().map(|t| &t.data_type);
                let rt = right.as_ref().map(|t| &t.data_type);

                // Temporal arithmetic for -
                match (lt, rt) {
                    // DATE - DATE → Interval
                    (Some(DataType::Date), Some(DataType::Date)) => {
                        return Some(TypedColumn {
                            data_type: DataType::Interval,
                            nullable: true,
                        });
                    }
                    // TIMESTAMP - TIMESTAMP → Interval
                    (Some(DataType::Timestamp { .. }), Some(DataType::Timestamp { .. })) => {
                        return Some(TypedColumn {
                            data_type: DataType::Interval,
                            nullable: true,
                        });
                    }
                    // TIME - TIME → Interval
                    (Some(DataType::Time), Some(DataType::Time)) => {
                        return Some(TypedColumn {
                            data_type: DataType::Interval,
                            nullable: true,
                        });
                    }
                    // DATE - INTERVAL → Timestamp
                    (Some(DataType::Date), Some(DataType::Interval)) => {
                        return Some(TypedColumn {
                            data_type: DataType::Timestamp {
                                with_timezone: false,
                            },
                            nullable: true,
                        });
                    }
                    // TIMESTAMP - INTERVAL → Timestamp
                    (Some(DataType::Timestamp { with_timezone }), Some(DataType::Interval)) => {
                        return Some(TypedColumn {
                            data_type: DataType::Timestamp {
                                with_timezone: *with_timezone,
                            },
                            nullable: true,
                        });
                    }
                    // TIME - INTERVAL → Time
                    (Some(DataType::Time), Some(DataType::Interval)) => {
                        return Some(TypedColumn {
                            data_type: DataType::Time,
                            nullable: true,
                        });
                    }
                    // INTERVAL - INTERVAL → Interval
                    (Some(DataType::Interval), Some(DataType::Interval)) => {
                        return Some(TypedColumn {
                            data_type: DataType::Interval,
                            nullable: true,
                        });
                    }
                    _ => {}
                }

                // Numeric promotion
                Some(promote_numeric_operands(
                    left.map(|t| t.data_type),
                    right.map(|t| t.data_type),
                )?)
            }
        }

        // JSON operators — both return Text because smelt represents JSON as Text
        // internally (no DataType::Json variant). Semantically, -> returns JSON
        // (navigable further) while ->> returns plain text. We keep separate arms
        // to preserve this distinction for future DataType::Json support.
        "->" | "#>" => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),
        "->>" | "#>>" => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),
        "@>" | "<@" => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: false,
        }),

        _ => None,
    }
}

/// Walk all BINARY_EXPR nodes in a SELECT statement and emit one
/// `TypeMismatch` Error at the operator span for each cross-family arithmetic
/// operation (spec §1 and §14: `42 + '3'` → `TypeMismatch`).
///
/// A cross-family pair is one where both operand types are known (not `Unknown`
/// / not `None`) and they belong to different type families (numeric vs string
/// vs boolean vs temporal). Temporal arithmetic special-cases
/// (`DATE + INTERVAL`, etc.) are handled by the `infer_binary_expr_type`
/// callers before reaching `promote_numeric_operands`; by the time we check
/// the result type here, those arms have already returned a concrete Temporal
/// type, so they never produce `Unknown` and this check skips them.
pub fn check_crossfamily_arithmetic_diagnostics(
    select_stmt: &SelectStmt,
    ctx: &TypeContext,
) -> Vec<crate::Diagnostic> {
    use smelt_parser::SyntaxKind::BINARY_EXPR;

    let mut diags: Vec<crate::Diagnostic> = Vec::new();
    let root = select_stmt.syntax();

    for node in root.descendants() {
        if node.kind() != BINARY_EXPR {
            continue;
        }
        let binary = match BinaryExpr::cast(node.clone()) {
            Some(b) => b,
            None => continue,
        };

        let op = match binary.operator() {
            Some(op) => op,
            None => continue,
        };

        if !matches!(op.as_str(), "+" | "-" | "*" | "/" | "%") {
            continue;
        }

        // Skip unary minus — no right operand.
        if binary.is_unary() {
            continue;
        }

        let left_tc = infer_binary_operand(&binary, 0, ctx);
        let right_tc = infer_binary_operand(&binary, 1, ctx);

        let lt = match left_tc.as_ref().map(|t| &t.data_type) {
            Some(dt) if !matches!(dt, DataType::Unknown) => dt,
            _ => continue, // unknown/unresolved — skip, no spurious diagnostic
        };
        let rt = match right_tc.as_ref().map(|t| &t.data_type) {
            Some(dt) if !matches!(dt, DataType::Unknown) => dt,
            _ => continue,
        };

        // Temporal arithmetic special-cases produce a concrete temporal type,
        // not Unknown — infer_binary_expr_type handles them in its early-return
        // arms. We only need to catch the cases where the result IS Unknown,
        // i.e. where neither operand's family is consistent with the other.
        //
        // INTERVAL arms: if either operand is Interval we skip (handled above).
        if matches!(lt, DataType::Interval) || matches!(rt, DataType::Interval) {
            continue;
        }

        // Same-family pairs are fine; different families → TypeMismatch.
        let same_family = (lt.is_numeric() && rt.is_numeric())
            || (lt.is_string() && rt.is_string())
            || (lt.is_temporal() && rt.is_temporal())
            || (matches!(lt, DataType::Boolean) && matches!(rt, DataType::Boolean));

        if same_family {
            continue;
        }

        // Anchor at the operator token if available, else the full expression.
        let range = binary
            .operator_token_range()
            .unwrap_or_else(|| node.text_range());

        diags.push(crate::Diagnostic {
            severity: crate::DiagnosticSeverity::Error,
            message: format!(
                "Cross-family arithmetic: cannot mix {} and {} with `{}`; \
                 consider an explicit CAST",
                lt, rt, op
            ),
            range,
            code: Some(crate::DiagnosticCode::TypeMismatch),
            data: None,
        });
    }

    diags
}

/// Recursively walk all sub-expressions, calling `visitor` for each column
/// reference encountered. Also triggers `ctx.lookup_column()` for
/// missed-lookup tracking. Unlike `infer_expression_type` which
/// short-circuits (e.g., `||` returns Text without inspecting operands),
/// this function visits ALL operands.
///
/// `type_hint` propagates type context from the parent expression (e.g.,
/// SUM/AVG arguments get a Double hint, binary expression operands get
/// cross-side type inference).
/// Callback type for column reference visitors.
/// Parameters: (qualifier, column_name, type_hint, text_range)
#[allow(clippy::type_complexity)]
pub type ColumnRefVisitor<'a> =
    &'a mut dyn FnMut(Option<&str>, &str, Option<&TypedColumn>, TextRange);

pub fn walk_expression_columns_with_visitor(
    expr: &Expr,
    ctx: &TypeContext,
    type_hint: Option<&TypedColumn>,
    visitor: ColumnRefVisitor<'_>,
) {
    // Leaf: column reference — trigger lookup and visitor
    // Only treat as a leaf if there are no child expression nodes
    // (avoids false-positive from as_column_ref on complex expressions
    // where a bare IDENT token coexists with BINARY_EXPR children)
    let has_expr_children = expr.syntax().children().any(|c| Expr::cast(c).is_some());
    if !has_expr_children {
        if let Some(col_ref) = expr.as_column_ref() {
            // intentionally ignored: called for its side-effect of recording
            // missed column lookups (for proptest discovery); the return value
            // is not used here — the visitor callback carries it forward.
            let _ = ctx.lookup_column(col_ref.qualifier(), col_ref.name());
            visitor(
                col_ref.qualifier(),
                col_ref.name(),
                type_hint,
                expr.text_range(),
            );
            return;
        }
    }

    // Subquery/EXISTS — skip (different scope)
    if expr.as_exists().is_some() || expr.as_subquery().is_some() {
        return;
    }

    // CASE — special handling for when_clauses/else (no hint propagation)
    if let Some(case_expr) = expr.as_case() {
        if let Some(case_value) = case_expr.case_value() {
            walk_expression_columns_with_visitor(&case_value, ctx, None, visitor);
        }
        for when_clause in case_expr.when_clauses() {
            if let Some(condition) = when_clause.condition() {
                walk_expression_columns_with_visitor(&condition, ctx, None, visitor);
            }
            if let Some(result) = when_clause.result() {
                walk_expression_columns_with_visitor(&result, ctx, None, visitor);
            }
        }
        if let Some(else_expr) = case_expr.else_expr() {
            walk_expression_columns_with_visitor(&else_expr, ctx, None, visitor);
        }
        return;
    }

    // Function call — walk all arguments with type hints for aggregates
    if let Some(func) = expr.as_function_call() {
        let func_name = func.name().map(|n| n.to_uppercase()).unwrap_or_default();
        let arg_hint = match SqlFunction::from_name(&func_name) {
            Some(SqlFunction::Sum | SqlFunction::Avg) => Some(TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            }),
            _ => None,
        };
        for arg in func.arguments() {
            walk_expression_columns_with_visitor(&arg, ctx, arg_hint.as_ref(), visitor);
        }
        if let Some(filter) = func.filter_clause() {
            if let Some(filter_expr) = filter.expression() {
                walk_expression_columns_with_visitor(&filter_expr, ctx, None, visitor);
            }
        }
        return;
    }

    // Binary expression — apply cross-side type inference when there are
    // exactly 2 child Expr operands (simple binary like `a = 1`). For
    // chained operators (3+ operands) we fall through to the generic handler.
    if expr.as_binary().is_some() {
        let child_exprs: Vec<Expr> = expr.syntax().children().filter_map(Expr::cast).collect();
        if child_exprs.len() == 2 {
            let lhs = &child_exprs[0];
            let rhs = &child_exprs[1];

            let lhs_type = infer_expression_type(lhs, ctx);
            let rhs_type = infer_expression_type(rhs, ctx);

            let lhs_is_col = lhs.as_column_ref().is_some();
            let rhs_is_col = rhs.as_column_ref().is_some();

            let lhs_hint = if lhs_is_col && !rhs_is_col {
                rhs_type.as_ref()
            } else {
                type_hint
            };
            walk_expression_columns_with_visitor(lhs, ctx, lhs_hint, visitor);

            let rhs_hint = if rhs_is_col && !lhs_is_col {
                lhs_type.as_ref()
            } else {
                type_hint
            };
            walk_expression_columns_with_visitor(rhs, ctx, rhs_hint, visitor);
            return;
        }
        // For chained binary operators, fall through to the generic handler
    }

    // For all other expression types (CAST, BETWEEN, IN, chained binary, etc.):
    // Walk all child nodes that can be cast to Expr.
    for child in expr.syntax().children() {
        if let Some(child_expr) = Expr::cast(child) {
            walk_expression_columns_with_visitor(&child_expr, ctx, type_hint, visitor);
        }
    }
}

/// Walk all sub-expressions, calling `lookup_column` on every column reference.
/// Thin wrapper around `walk_expression_columns_with_visitor` with no visitor
/// or type hints — used by property-based tests to detect missing columns.
pub fn walk_expression_columns(expr: &Expr, ctx: &TypeContext) {
    walk_expression_columns_with_visitor(expr, ctx, None, &mut |_, _, _, _| {});
}

/// Walk all expressions in a SELECT statement with a visitor callback.
/// Covers SELECT list, WHERE, GROUP BY, HAVING, QUALIFY, JOIN ON, and ORDER BY.
pub fn walk_select_columns_with_visitor(
    select_stmt: &SelectStmt,
    ctx: &TypeContext,
    type_hint: Option<&TypedColumn>,
    visitor: ColumnRefVisitor<'_>,
) {
    if let Some(select_list) = select_stmt.select_list() {
        for item in select_list.items() {
            if let Some(expr) = item.expression() {
                walk_expression_columns_with_visitor(&expr, ctx, type_hint, visitor);
            }
        }
    }
    if let Some(where_clause) = select_stmt.where_clause() {
        if let Some(expr) = where_clause.expression() {
            walk_expression_columns_with_visitor(&expr, ctx, type_hint, visitor);
        }
    }
    if let Some(from_clause) = select_stmt.from_clause() {
        for join in from_clause.joins() {
            if let Some(condition) = join.condition() {
                if let Some(on_expr) = condition.on_expression() {
                    walk_expression_columns_with_visitor(&on_expr, ctx, type_hint, visitor);
                }
            }
        }
    }
    if let Some(group_by) = select_stmt.group_by_clause() {
        for expr in group_by.expressions() {
            walk_expression_columns_with_visitor(&expr, ctx, type_hint, visitor);
        }
    }
    if let Some(having) = select_stmt.having_clause() {
        if let Some(expr) = having.expression() {
            walk_expression_columns_with_visitor(&expr, ctx, type_hint, visitor);
        }
    }
    if let Some(order_by) = select_stmt.order_by_clause() {
        for item in order_by.items() {
            if let Some(expr) = item.expression() {
                walk_expression_columns_with_visitor(&expr, ctx, type_hint, visitor);
            }
        }
    }
    if let Some(qualify) = select_stmt.qualify_clause() {
        if let Some(expr) = qualify.expression() {
            walk_expression_columns_with_visitor(&expr, ctx, type_hint, visitor);
        }
    }
}

/// Walk all expressions in a SELECT statement to trigger column lookups.
/// Covers SELECT list, WHERE, GROUP BY, HAVING, QUALIFY, JOIN ON, and ORDER BY.
pub fn walk_select_columns(select_stmt: &SelectStmt, ctx: &TypeContext) {
    walk_select_columns_with_visitor(select_stmt, ctx, None, &mut |_, _, _, _| {});
}

/// Check for column references that don't resolve against declared schemas.
/// Returns diagnostics with accurate source positions.
/// Structured info about an undeclared column
#[derive(Debug)]
pub struct UndeclaredColumnInfo {
    pub message: String,
    pub range: TextRange,
    pub qualifier: Option<String>,
    pub column_name: String,
}

pub fn check_undeclared_columns(
    select_stmt: &SelectStmt,
    ctx: &TypeContext,
) -> Vec<UndeclaredColumnInfo> {
    let mut undeclared = Vec::new();

    // Collect SELECT aliases — these are valid references in GROUP BY / ORDER BY / HAVING
    let mut select_aliases = std::collections::HashSet::new();
    if let Some(select_list) = select_stmt.select_list() {
        for item in select_list.items() {
            if let Some(alias) = item.alias() {
                select_aliases.insert(alias.to_lowercase());
            }
        }
    }

    walk_select_columns_with_visitor(
        select_stmt,
        ctx,
        None,
        &mut |qualifier, col_name, _, range| {
            // Skip SQL keywords that may be parsed as identifiers
            let lower = col_name.to_lowercase();
            if matches!(lower.as_str(), "true" | "false" | "null") {
                return;
            }

            // Skip unqualified references to SELECT aliases (valid in GROUP BY/ORDER BY)
            if qualifier.is_none() && select_aliases.contains(&lower) {
                return;
            }

            // Use `lookup_identifier` so bound function parameters
            // (seeded via `add_function_param` at call-site expansion
            // for `Expr<T>` kinds) resolve before falling back to the
            // FROM scopes. Phase 17 hinge: a SELECT-shaped function
            // body that references `ts_col` / `gap` must see the
            // Expr<Timestamp> / Expr<Interval> bindings populated by
            // the call-site checker.
            if ctx.lookup_identifier(qualifier, col_name).is_some() {
                return;
            }

            let message = if let Some(q) = qualifier {
                if let Some(desc) = ctx.describe_qualifier(q) {
                    format!("Column '{}' not found in {}", col_name, desc)
                } else {
                    format!("Column '{}.{}' not found", q, col_name)
                }
            } else {
                "Column '{}' not found in any source, model, or CTE".replace("{}", col_name)
            };

            undeclared.push(UndeclaredColumnInfo {
                message,
                range,
                qualifier: qualifier.map(|s| s.to_string()),
                column_name: col_name.to_string(),
            });
        },
    );

    undeclared
}

///
/// This extracts columns from the CTE's query and optionally overrides
/// the inferred names with explicit column names if provided.
pub fn infer_cte_columns(cte: &Cte, ctx: &TypeContext) -> Vec<(String, TypedColumn)> {
    // Get the CTE's query (SELECT statement)
    let select_stmt = match cte.query().and_then(|q| q.select_stmt()) {
        Some(s) => s,
        None => return Vec::new(),
    };

    // Phase 46: factor the per-select-item inference into a sibling
    // helper so derived-table / inline-subquery argument resolution
    // (in `tableexpr_schema_lookup`) can share the same path.
    let mut columns = infer_select_output_schema(&select_stmt, ctx);

    // CTE-specific concern: the WITH clause may declare explicit
    // column names that override any inferred names from the SELECT
    // list. Apply the override after the shared inference runs.
    let explicit_names = cte.column_names();
    for (i, name) in explicit_names.iter().enumerate() {
        if i < columns.len() {
            columns[i].0 = name.clone();
        }
    }

    columns
}
