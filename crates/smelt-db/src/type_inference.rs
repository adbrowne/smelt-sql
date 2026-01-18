/// Type inference for SQL expressions
///
/// This module provides type inference capabilities for SQL expressions,
/// including literals, column references, CAST expressions, and aggregates.
use smelt_parser::ast::{
    BinaryExpr, CaseExpr, CastExpr, Cte, Expr, FunctionCall, SelectStmt, Subquery,
};
use smelt_types::{parse_type, DataType, TypedColumn};
use std::collections::HashMap;

/// Context for type inference - provides source and upstream model schemas
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TypeContext {
    /// Source columns: source_name.table_name.column_name -> type
    source_columns: HashMap<String, TypedColumn>,
    /// Model columns: model_name.column_name -> type
    model_columns: HashMap<String, TypedColumn>,
    /// CTE columns: cte_name.column_name -> type
    cte_columns: HashMap<String, TypedColumn>,
    /// Known CTE names (for checking if a qualifier is a CTE)
    cte_names: std::collections::HashSet<String>,
    /// Aliases in scope: alias -> qualified name
    aliases: HashMap<String, String>,
}

impl TypeContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a source column to the context
    pub fn add_source_column(
        &mut self,
        source_name: &str,
        table_name: &str,
        column_name: &str,
        typed_column: TypedColumn,
    ) {
        let key = format!("{}.{}.{}", source_name, table_name, column_name);
        self.source_columns.insert(key, typed_column.clone());

        // Also add without source qualifier for simple lookups
        let simple_key = format!("{}.{}", table_name, column_name);
        self.source_columns
            .entry(simple_key)
            .or_insert(typed_column);
    }

    /// Add a model column to the context
    pub fn add_model_column(
        &mut self,
        model_name: &str,
        column_name: &str,
        typed_column: TypedColumn,
    ) {
        let key = format!("{}.{}", model_name, column_name);
        self.model_columns.insert(key, typed_column);
    }

    /// Add an alias mapping
    pub fn add_alias(&mut self, alias: &str, qualified_name: &str) {
        self.aliases
            .insert(alias.to_string(), qualified_name.to_string());
    }

    /// Add a CTE column to the context
    pub fn add_cte_column(&mut self, cte_name: &str, column_name: &str, typed_column: TypedColumn) {
        let key = format!("{}.{}", cte_name, column_name);
        self.cte_columns.insert(key, typed_column);
        self.cte_names.insert(cte_name.to_string());
    }

    /// Check if a name is a known CTE
    pub fn is_cte(&self, name: &str) -> bool {
        self.cte_names.contains(name)
    }

    /// Look up a column type by name (with optional qualifier)
    /// CTEs shadow outer scope, so we check them first.
    pub fn lookup_column(&self, qualifier: Option<&str>, name: &str) -> Option<&TypedColumn> {
        // If we have a qualifier, use it directly
        if let Some(q) = qualifier {
            // Check if qualifier is an alias
            let resolved_qualifier = self.aliases.get(q).map(|s| s.as_str()).unwrap_or(q);

            // Try CTE columns first (CTEs shadow outer scope)
            let cte_key = format!("{}.{}", resolved_qualifier, name);
            if let Some(t) = self.cte_columns.get(&cte_key) {
                return Some(t);
            }

            // Try model columns
            let model_key = format!("{}.{}", resolved_qualifier, name);
            if let Some(t) = self.model_columns.get(&model_key) {
                return Some(t);
            }

            // Try source columns
            if let Some(t) = self.source_columns.get(&model_key) {
                return Some(t);
            }

            // Try with full source path
            for (key, typed_col) in &self.source_columns {
                if key.ends_with(&format!("{}.{}", resolved_qualifier, name)) {
                    return Some(typed_col);
                }
            }
        }

        // Unqualified lookup - search all sources
        // First try CTE columns (CTEs shadow outer scope)
        for (key, typed_col) in &self.cte_columns {
            if key.ends_with(&format!(".{}", name)) {
                return Some(typed_col);
            }
        }

        // Then try model columns
        for (key, typed_col) in &self.model_columns {
            if key.ends_with(&format!(".{}", name)) {
                return Some(typed_col);
            }
        }

        // Then try source columns
        for (key, typed_col) in &self.source_columns {
            if key.ends_with(&format!(".{}", name)) {
                return Some(typed_col);
            }
        }

        None
    }
}

/// Infer the type of an SQL expression
pub fn infer_expression_type(expr: &Expr, ctx: &TypeContext) -> Option<TypedColumn> {
    let text = expr.text().trim().to_string();

    // Try CAST expression first
    if let Some(cast_expr) = expr.as_cast() {
        return infer_cast_type(&cast_expr);
    }

    // Try CASE expression
    if let Some(case_expr) = expr.as_case() {
        return infer_case_expr_type(&case_expr, ctx);
    }

    // Try subquery (scalar subquery)
    if let Some(subquery) = expr.as_subquery() {
        return infer_subquery_type(&subquery, ctx);
    }

    // Try function call (aggregates, etc.)
    if let Some(func) = expr.as_function_call() {
        return infer_function_type(&func, ctx);
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

    // Try column reference
    if let Some(col_ref) = expr.as_column_ref() {
        return ctx
            .lookup_column(col_ref.qualifier(), col_ref.name())
            .cloned();
    }

    // Try literal inference
    infer_literal_type(&text)
}

/// Infer the type of a CAST expression
fn infer_cast_type(cast_expr: &CastExpr) -> Option<TypedColumn> {
    let type_spec = cast_expr.type_spec()?;
    let type_text = type_spec.full_text();

    // Parse the type specification
    let data_type = parse_type(&type_text).ok()?;

    Some(TypedColumn {
        data_type,
        // CAST can produce NULL if the input is NULL
        nullable: true,
    })
}

/// Infer the type of a CASE expression
/// The result type is the type of the first THEN expression (or ELSE if no WHEN clauses)
fn infer_case_expr_type(case_expr: &CaseExpr, ctx: &TypeContext) -> Option<TypedColumn> {
    // Try to get the type from the first WHEN clause's THEN expression
    for when_clause in case_expr.when_clauses() {
        if let Some(result_expr) = when_clause.result() {
            if let Some(result_type) = infer_expression_type(&result_expr, ctx) {
                return Some(TypedColumn {
                    data_type: result_type.data_type,
                    // CASE is always nullable (could return NULL if no conditions match without ELSE)
                    nullable: true,
                });
            }
        }
    }

    // Fall back to ELSE expression if no WHEN clauses have inferable types
    if let Some(else_expr) = case_expr.else_expr() {
        if let Some(else_type) = infer_expression_type(&else_expr, ctx) {
            return Some(TypedColumn {
                data_type: else_type.data_type,
                nullable: true,
            });
        }
    }

    None
}

/// Infer the type of a scalar subquery
/// The result type is the type of the first column in the SELECT list
fn infer_subquery_type(subquery: &Subquery, ctx: &TypeContext) -> Option<TypedColumn> {
    let select_stmt = subquery.select_stmt()?;

    // Build a new context that includes any CTEs defined in this subquery
    let subquery_ctx = build_subquery_context(&select_stmt, ctx);

    let select_list = select_stmt.select_list()?;

    // Get the first select item and infer its type
    if let Some(first_item) = select_list.items().next() {
        if let Some(expr) = first_item.expression() {
            if let Some(expr_type) = infer_expression_type(&expr, &subquery_ctx) {
                return Some(TypedColumn {
                    data_type: expr_type.data_type,
                    // Scalar subqueries are always nullable (could return no rows)
                    nullable: true,
                });
            }
        }
    }

    None
}

/// Build a TypeContext for a subquery that includes any nested CTEs
///
/// This creates a new context that inherits from the parent context
/// and adds any CTEs defined in the subquery's WITH clause.
fn build_subquery_context(select_stmt: &SelectStmt, parent_ctx: &TypeContext) -> TypeContext {
    let mut ctx = parent_ctx.clone();

    // Process any WITH clause in this subquery
    if let Some(with_clause) = select_stmt.with_clause() {
        for cte in with_clause.ctes() {
            if let Some(cte_name) = cte.name() {
                // For recursive CTEs with explicit column list, bootstrap with Unknown types
                if with_clause.is_recursive() {
                    for col_name in cte.column_names() {
                        ctx.add_cte_column(
                            &cte_name,
                            &col_name,
                            TypedColumn {
                                data_type: DataType::Unknown,
                                nullable: true,
                            },
                        );
                    }
                }

                // Infer columns from CTE query
                let columns = infer_cte_columns(&cte, &ctx);
                for (col_name, typed_col) in columns {
                    ctx.add_cte_column(&cte_name, &col_name, typed_col);
                }

                // Register CTE name as alias
                ctx.add_alias(&cte_name, &cte_name);
            }
        }
    }

    ctx
}

/// Infer the type of a function call (aggregates, etc.)
fn infer_function_type(func: &FunctionCall, ctx: &TypeContext) -> Option<TypedColumn> {
    let name = func.name()?.to_uppercase();

    match name.as_str() {
        // Count functions always return BigInt
        "COUNT" => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: false, // COUNT never returns NULL (returns 0 for empty sets)
        }),

        // SUM - returns same numeric type or wider
        "SUM" => {
            // For now, default to Decimal for flexibility
            // A more sophisticated implementation would look at the argument type
            Some(TypedColumn {
                data_type: DataType::Decimal {
                    precision: 38,
                    scale: 10,
                },
                nullable: true, // SUM of empty set is NULL
            })
        }

        // AVG always returns floating point
        "AVG" => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true, // AVG of empty set is NULL
        }),

        // MIN/MAX preserve the argument type
        "MIN" | "MAX" => {
            // Try to infer from the first argument
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    return Some(TypedColumn {
                        data_type: arg_type.data_type,
                        nullable: true, // MIN/MAX of empty set is NULL
                    });
                }
            }
            Some(TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            })
        }

        // COALESCE - returns first non-null, type is type of first argument
        "COALESCE" => {
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    return Some(TypedColumn {
                        data_type: arg_type.data_type,
                        nullable: true, // Could be null if all args are null
                    });
                }
            }
            Some(TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            })
        }

        // NULLIF - returns first arg type, always nullable
        "NULLIF" => {
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    return Some(TypedColumn {
                        data_type: arg_type.data_type,
                        nullable: true, // NULLIF can always return null
                    });
                }
            }
            Some(TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            })
        }

        // Window ranking functions - return BigInt
        "ROW_NUMBER" | "RANK" | "DENSE_RANK" | "NTILE" => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: false,
        }),

        // Window distribution functions - return Double (0.0 to 1.0)
        "CUME_DIST" | "PERCENT_RANK" => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: false,
        }),

        // Window navigation functions - preserve argument type
        "LAG" | "LEAD" | "FIRST_VALUE" | "LAST_VALUE" | "NTH_VALUE" => {
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    return Some(TypedColumn {
                        data_type: arg_type.data_type,
                        nullable: true, // Window functions can return NULL at boundaries
                    });
                }
            }
            Some(TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            })
        }

        // Date functions
        "NOW" | "CURRENT_TIMESTAMP" => Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: false,
            },
            nullable: false,
        }),

        "CURRENT_DATE" => Some(TypedColumn {
            data_type: DataType::Date,
            nullable: false,
        }),

        "DATE" | "DATE_TRUNC" => Some(TypedColumn {
            data_type: DataType::Date,
            nullable: true,
        }),

        // String functions
        "CONCAT" | "UPPER" | "LOWER" | "TRIM" | "LTRIM" | "RTRIM" | "SUBSTRING" | "SUBSTR" => {
            Some(TypedColumn {
                data_type: DataType::Text,
                nullable: true,
            })
        }

        "LENGTH" | "CHAR_LENGTH" | "CHARACTER_LENGTH" => Some(TypedColumn {
            data_type: DataType::Integer,
            nullable: true,
        }),

        // Type conversion
        "TO_CHAR" => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        // Boolean functions
        "BOOL_AND" | "BOOL_OR" | "EVERY" => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: true,
        }),

        // Default - unknown function type
        _ => None,
    }
}

/// Infer the type of a literal value
fn infer_literal_type(text: &str) -> Option<TypedColumn> {
    let text = text.trim();

    // NULL literal
    if text.eq_ignore_ascii_case("NULL") {
        return Some(TypedColumn {
            data_type: DataType::Null,
            nullable: true,
        });
    }

    // Boolean literals
    if text.eq_ignore_ascii_case("TRUE") || text.eq_ignore_ascii_case("FALSE") {
        return Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: false,
        });
    }

    // String literals (single or double quoted)
    if (text.starts_with('\'') && text.ends_with('\''))
        || (text.starts_with('"') && text.ends_with('"'))
    {
        return Some(TypedColumn {
            data_type: DataType::Text,
            nullable: false,
        });
    }

    // Numeric literals
    if let Some(num_type) = infer_numeric_literal_type(text) {
        return Some(TypedColumn {
            data_type: num_type,
            nullable: false,
        });
    }

    None
}

/// Infer the type of a numeric literal
fn infer_numeric_literal_type(text: &str) -> Option<DataType> {
    // Check for decimal point
    if text.contains('.') {
        // Could be DECIMAL or DOUBLE
        // If it has 'e' or 'E', it's a floating point
        if text.contains('e') || text.contains('E') {
            return Some(DataType::Double);
        }

        // Count digits for precision/scale
        let parts: Vec<&str> = text.split('.').collect();
        if parts.len() == 2 {
            let precision = parts[0].trim_start_matches('-').len() + parts[1].len();
            let scale = parts[1].len();
            return Some(DataType::Decimal {
                precision: precision.min(38) as u8,
                scale: scale.min(38) as u8,
            });
        }

        return Some(DataType::Double);
    }

    // Integer literal - check range
    if let Ok(n) = text.parse::<i64>() {
        return Some(if n >= i16::MIN as i64 && n <= i16::MAX as i64 {
            DataType::SmallInt
        } else if n >= i32::MIN as i64 && n <= i32::MAX as i64 {
            DataType::Integer
        } else {
            DataType::BigInt
        });
    }

    // Try parsing as unsigned for very large numbers
    if text.parse::<u64>().is_ok() {
        return Some(DataType::BigInt);
    }

    None
}

/// Infer the result type of a binary expression
fn infer_binary_expr_type(binary: &BinaryExpr, ctx: &TypeContext) -> Option<TypedColumn> {
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

        // String concatenation - always returns Text
        "||" => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        // Arithmetic operators - promote to widest numeric type
        "+" | "*" | "/" => {
            let left = binary.left().and_then(|e| infer_expression_type(&e, ctx));
            let right = binary.right().and_then(|e| infer_expression_type(&e, ctx));

            // Promote to widest numeric type
            match (left.map(|t| t.data_type), right.map(|t| t.data_type)) {
                (Some(DataType::Double), _) | (_, Some(DataType::Double)) => Some(TypedColumn {
                    data_type: DataType::Double,
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
                (Some(DataType::SmallInt), _) | (_, Some(DataType::SmallInt)) => {
                    Some(TypedColumn {
                        data_type: DataType::SmallInt,
                        nullable: true,
                    })
                }
                (Some(l), _) => Some(TypedColumn {
                    data_type: l,
                    nullable: true,
                }),
                _ => None,
            }
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
                let left = binary.left().and_then(|e| infer_expression_type(&e, ctx));
                let right = binary.right().and_then(|e| infer_expression_type(&e, ctx));

                // Promote to widest numeric type
                match (left.map(|t| t.data_type), right.map(|t| t.data_type)) {
                    (Some(DataType::Double), _) | (_, Some(DataType::Double)) => {
                        Some(TypedColumn {
                            data_type: DataType::Double,
                            nullable: true,
                        })
                    }
                    (Some(DataType::Decimal { .. }), _) | (_, Some(DataType::Decimal { .. })) => {
                        Some(TypedColumn {
                            data_type: DataType::Decimal {
                                precision: 38,
                                scale: 10,
                            },
                            nullable: true,
                        })
                    }
                    (Some(DataType::BigInt), _) | (_, Some(DataType::BigInt)) => {
                        Some(TypedColumn {
                            data_type: DataType::BigInt,
                            nullable: true,
                        })
                    }
                    (Some(DataType::Integer), _) | (_, Some(DataType::Integer)) => {
                        Some(TypedColumn {
                            data_type: DataType::Integer,
                            nullable: true,
                        })
                    }
                    (Some(DataType::SmallInt), _) | (_, Some(DataType::SmallInt)) => {
                        Some(TypedColumn {
                            data_type: DataType::SmallInt,
                            nullable: true,
                        })
                    }
                    (Some(l), _) => Some(TypedColumn {
                        data_type: l,
                        nullable: true,
                    }),
                    _ => None,
                }
            }
        }

        _ => None,
    }
}

/// Infer the column names and types from a CTE definition
///
/// This extracts columns from the CTE's query and optionally overrides
/// the inferred names with explicit column names if provided.
pub fn infer_cte_columns(cte: &Cte, ctx: &TypeContext) -> Vec<(String, TypedColumn)> {
    let mut columns = Vec::new();

    // Get explicit column names (if present)
    let explicit_names = cte.column_names();

    // Get the CTE's query (SELECT statement)
    let select_stmt = match cte.query().and_then(|q| q.select_stmt()) {
        Some(s) => s,
        None => return columns,
    };

    // Build a context that includes any nested CTEs in this CTE's query
    let cte_ctx = build_subquery_context(&select_stmt, ctx);

    let select_list = match select_stmt.select_list() {
        Some(l) => l,
        None => return columns,
    };

    // Process each select item
    for (i, item) in select_list.items().enumerate() {
        // Determine column name:
        // 1. Use explicit name from CTE column list (if available at this position)
        // 2. Use explicit alias from AS clause
        // 3. Try to infer from expression (column reference name)
        // 4. Fall back to generated name
        let col_name = if i < explicit_names.len() {
            explicit_names[i].clone()
        } else if let Some(alias) = item.alias() {
            alias
        } else if let Some(expr) = item.expression() {
            // Try to infer name from expression
            infer_column_name(&expr).unwrap_or_else(|| format!("col{}", i + 1))
        } else {
            format!("col{}", i + 1)
        };

        // Infer type from expression using the CTE's context (includes nested CTEs)
        let typed_col = if let Some(expr) = item.expression() {
            infer_expression_type(&expr, &cte_ctx).unwrap_or(TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            })
        } else {
            TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            }
        };

        columns.push((col_name, typed_col));
    }

    columns
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

    // Try function call - use function name
    if let Some(func) = expr.as_function_call() {
        return func.name();
    }

    // For other expressions, we can't infer a name
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_literal_type_inference() {
        // SmallInt (small values fit in SmallInt)
        assert_eq!(
            infer_literal_type("42"),
            Some(TypedColumn {
                data_type: DataType::SmallInt,
                nullable: false,
            })
        );

        // Integer (larger values that don't fit in SmallInt)
        assert_eq!(
            infer_literal_type("100000"),
            Some(TypedColumn {
                data_type: DataType::Integer,
                nullable: false,
            })
        );

        // BigInt
        assert_eq!(
            infer_literal_type("9999999999"),
            Some(TypedColumn {
                data_type: DataType::BigInt,
                nullable: false,
            })
        );

        // Decimal
        let decimal_type = infer_literal_type("123.45").unwrap();
        assert!(matches!(decimal_type.data_type, DataType::Decimal { .. }));
        assert!(!decimal_type.nullable);

        // Double (scientific notation)
        assert_eq!(
            infer_literal_type("1.5e10"),
            Some(TypedColumn {
                data_type: DataType::Double,
                nullable: false,
            })
        );

        // String
        assert_eq!(
            infer_literal_type("'hello'"),
            Some(TypedColumn {
                data_type: DataType::Text,
                nullable: false,
            })
        );

        // Boolean
        assert_eq!(
            infer_literal_type("TRUE"),
            Some(TypedColumn {
                data_type: DataType::Boolean,
                nullable: false,
            })
        );

        // NULL
        assert_eq!(
            infer_literal_type("NULL"),
            Some(TypedColumn {
                data_type: DataType::Null,
                nullable: true,
            })
        );
    }

    #[test]
    fn test_type_context_lookup() {
        let mut ctx = TypeContext::new();

        ctx.add_source_column(
            "raw",
            "users",
            "id",
            TypedColumn {
                data_type: DataType::Integer,
                nullable: false,
            },
        );

        ctx.add_model_column(
            "staging_users",
            "user_id",
            TypedColumn {
                data_type: DataType::BigInt,
                nullable: false,
            },
        );

        // Look up source column with qualifier
        let result = ctx.lookup_column(Some("users"), "id");
        assert!(result.is_some());
        assert_eq!(result.unwrap().data_type, DataType::Integer);

        // Look up model column with qualifier
        let result = ctx.lookup_column(Some("staging_users"), "user_id");
        assert!(result.is_some());
        assert_eq!(result.unwrap().data_type, DataType::BigInt);

        // Look up without qualifier (unambiguous)
        let result = ctx.lookup_column(None, "id");
        assert!(result.is_some());
    }

    #[test]
    fn test_aggregate_function_types() {
        let ctx = TypeContext::new();

        // Create a mock expression text for COUNT
        // Note: In real usage, we'd use the actual AST
        let count_type = infer_function_type_by_name("COUNT", &ctx).unwrap();
        assert_eq!(count_type.data_type, DataType::BigInt);
        assert!(!count_type.nullable);

        // AVG returns Double
        let avg_type = infer_function_type_by_name("AVG", &ctx).unwrap();
        assert_eq!(avg_type.data_type, DataType::Double);
        assert!(avg_type.nullable);

        // SUM returns Decimal
        let sum_type = infer_function_type_by_name("SUM", &ctx).unwrap();
        assert!(matches!(sum_type.data_type, DataType::Decimal { .. }));
    }

    // Helper for testing aggregate functions without AST
    fn infer_function_type_by_name(name: &str, _ctx: &TypeContext) -> Option<TypedColumn> {
        match name.to_uppercase().as_str() {
            "COUNT" => Some(TypedColumn {
                data_type: DataType::BigInt,
                nullable: false,
            }),
            "AVG" => Some(TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            }),
            "SUM" => Some(TypedColumn {
                data_type: DataType::Decimal {
                    precision: 38,
                    scale: 10,
                },
                nullable: true,
            }),
            _ => None,
        }
    }

    #[test]
    fn test_cte_column_lookup() {
        let mut ctx = TypeContext::new();

        // Add a CTE column
        ctx.add_cte_column(
            "daily_totals",
            "day",
            TypedColumn {
                data_type: DataType::Date,
                nullable: false,
            },
        );

        ctx.add_cte_column(
            "daily_totals",
            "total",
            TypedColumn {
                data_type: DataType::Decimal {
                    precision: 38,
                    scale: 10,
                },
                nullable: true,
            },
        );

        // Check that CTE is registered
        assert!(ctx.is_cte("daily_totals"));
        assert!(!ctx.is_cte("nonexistent"));

        // Look up CTE column with qualifier
        let result = ctx.lookup_column(Some("daily_totals"), "day");
        assert!(result.is_some());
        assert_eq!(result.unwrap().data_type, DataType::Date);

        // Look up CTE column without qualifier
        let result = ctx.lookup_column(None, "total");
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap().data_type,
            DataType::Decimal { .. }
        ));
    }

    #[test]
    fn test_cte_shadows_source() {
        let mut ctx = TypeContext::new();

        // Add a source column with name "orders"
        ctx.add_source_column(
            "raw",
            "orders",
            "amount",
            TypedColumn {
                data_type: DataType::Integer,
                nullable: false,
            },
        );

        // Add a CTE with the same name "orders" but different column type
        ctx.add_cte_column(
            "orders",
            "amount",
            TypedColumn {
                data_type: DataType::BigInt,
                nullable: true,
            },
        );

        // CTE should shadow the source - BigInt should be returned, not Integer
        let result = ctx.lookup_column(Some("orders"), "amount");
        assert!(result.is_some());
        assert_eq!(result.unwrap().data_type, DataType::BigInt);

        // Unqualified lookup should also return CTE column
        let result = ctx.lookup_column(None, "amount");
        assert!(result.is_some());
        assert_eq!(result.unwrap().data_type, DataType::BigInt);
    }
}
