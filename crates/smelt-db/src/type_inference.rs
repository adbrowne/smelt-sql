/// Type inference for SQL expressions
///
/// This module provides type inference capabilities for SQL expressions,
/// including literals, column references, CAST expressions, and aggregates.
use rowan::TextRange;
use smelt_parser::ast::{
    BinaryExpr, CaseExpr, CastExpr, Cte, Expr, FunctionCall, SelectStmt, Subquery,
};
use smelt_types::{parse_type, DataType, SqlFunction, TypedColumn};
use std::collections::HashMap;
use std::sync::Mutex;

/// Context for type inference - provides source and upstream model schemas
#[derive(Debug, Default)]
pub struct TypeContext {
    // NOTE: PartialEq, Eq, Clone are implemented manually below to handle missed_lookups
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
    /// Column lookups that returned None (for property-based test column detection)
    missed_lookups: Mutex<Vec<(Option<String>, String)>>,
}

impl PartialEq for TypeContext {
    fn eq(&self, other: &Self) -> bool {
        self.source_columns == other.source_columns
            && self.model_columns == other.model_columns
            && self.cte_columns == other.cte_columns
            && self.cte_names == other.cte_names
            && self.aliases == other.aliases
        // missed_lookups is intentionally excluded — it's transient tracking state
    }
}

impl Eq for TypeContext {}

impl Clone for TypeContext {
    fn clone(&self) -> Self {
        Self {
            source_columns: self.source_columns.clone(),
            model_columns: self.model_columns.clone(),
            cte_columns: self.cte_columns.clone(),
            cte_names: self.cte_names.clone(),
            aliases: self.aliases.clone(),
            missed_lookups: Mutex::new(Vec::new()), // Don't clone tracking state
        }
    }
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

    /// Resolve an alias to its qualified name
    pub fn resolve_alias(&self, alias: &str) -> Option<String> {
        self.aliases.get(alias).cloned()
    }

    /// Get all CTE names in scope
    pub fn cte_names(&self) -> impl Iterator<Item = &str> {
        self.cte_names.iter().map(|s| s.as_str())
    }

    /// Get columns for a specific CTE
    pub fn cte_columns(&self, cte_name: &str) -> Vec<(&str, &TypedColumn)> {
        let prefix = format!("{}.", cte_name);
        self.cte_columns
            .iter()
            .filter_map(move |(key, typed_col)| {
                key.strip_prefix(&prefix)
                    .map(|col_name| (col_name, typed_col))
            })
            .collect()
    }

    /// Look up a column type by name (with optional qualifier).
    /// CTEs shadow outer scope, so we check them first.
    /// Records missed lookups (when None is returned) for property-based test
    /// column detection via `take_missed_lookups()`.
    pub fn lookup_column(&self, qualifier: Option<&str>, name: &str) -> Option<&TypedColumn> {
        let result = self.lookup_column_inner(qualifier, name);
        if result.is_none() {
            if let Ok(mut lookups) = self.missed_lookups.lock() {
                lookups.push((qualifier.map(|s| s.to_string()), name.to_string()));
            }
        }
        result
    }

    fn lookup_column_inner(&self, qualifier: Option<&str>, name: &str) -> Option<&TypedColumn> {
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

    /// Take and clear the list of column lookups that returned None.
    /// Used by property-based tests to discover missing columns.
    pub fn take_missed_lookups(&self) -> Vec<(Option<String>, String)> {
        match self.missed_lookups.lock() {
            Ok(mut lookups) => std::mem::take(&mut *lookups),
            Err(_) => Vec::new(),
        }
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
    let sql_func = SqlFunction::from_name(&name)?;

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
                    let result_type = match &arg_type.data_type {
                        DataType::SmallInt | DataType::Integer => DataType::BigInt,
                        DataType::BigInt => DataType::BigInt,
                        DataType::Float | DataType::Double => DataType::Double,
                        dt @ DataType::Decimal { .. } => dt.clone(),
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
            first_arg_type_or(func, ctx, DataType::Unknown, true)
        }

        SqlFunction::Coalesce => {
            // Try all arguments, return first concrete (non-Unknown, non-Null) type
            for arg in func.arguments() {
                if let Some(arg_type) = infer_expression_type(&arg, ctx) {
                    if !matches!(arg_type.data_type, DataType::Unknown | DataType::Null) {
                        return Some(TypedColumn {
                            data_type: arg_type.data_type,
                            nullable: true,
                        });
                    }
                }
            }
            first_arg_type_or(func, ctx, DataType::Unknown, true)
        }

        SqlFunction::Nullif => first_arg_type_or(func, ctx, DataType::Unknown, true),

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
        | SqlFunction::NthValue => first_arg_type_or(func, ctx, DataType::Unknown, true),

        SqlFunction::Now | SqlFunction::CurrentTimestamp => Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: false,
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

        SqlFunction::DateTrunc => Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: false,
            },
            nullable: true,
        }),

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

        SqlFunction::Extract | SqlFunction::DatePart => Some(TypedColumn {
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

        SqlFunction::MakeTimestamp | SqlFunction::MakeTimestamptz => Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: false,
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
                    if !matches!(arg_type.data_type, DataType::Unknown | DataType::Null) {
                        return Some(TypedColumn {
                            data_type: arg_type.data_type,
                            nullable: true,
                        });
                    }
                }
            }
            first_arg_type_or(func, ctx, DataType::Unknown, true)
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
                data_type: DataType::Array(Box::new(DataType::Unknown)),
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

        SqlFunction::JsonContains => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: false,
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

        SqlFunction::Median => first_arg_type_or(func, ctx, DataType::Double, true),

        SqlFunction::Mode => first_arg_type_or(func, ctx, DataType::Unknown, true),

        SqlFunction::PercentileCont | SqlFunction::PercentileDisc => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),

        SqlFunction::ApproxCountDistinct => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: false,
        }),

        SqlFunction::AnyValue | SqlFunction::First | SqlFunction::Last => {
            first_arg_type_or(func, ctx, DataType::Unknown, true)
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
        // Also check for bare IDENT tokens as CASE value (parser artifact:
        // `CASE status WHEN ...` may have `status` as a bare token, not wrapped
        // in an EXPRESSION node). Scan the CASE_EXPR node's direct tokens.
        for child in case_expr.syntax().children_with_tokens() {
            if let Some(node) = child.as_node() {
                if node.kind() == smelt_parser::SyntaxKind::WHEN_CLAUSE {
                    break;
                }
            }
            if let Some(token) = child.as_token() {
                if token.kind() == smelt_parser::SyntaxKind::IDENT {
                    let _ = ctx.lookup_column(None, token.text());
                    visitor(None, token.text(), None, token.text_range());
                }
            }
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
    // Walk all child nodes that can be cast to Expr, plus bare IDENT tokens.
    // This handles the parser's flat structure for chained binary operators
    // (e.g., `a || b || c` creates sibling BINARY_EXPR nodes).
    for child in expr.syntax().children() {
        if let Some(child_expr) = Expr::cast(child) {
            walk_expression_columns_with_visitor(&child_expr, ctx, type_hint, visitor);
        }
    }
    for child in expr.syntax().children_with_tokens() {
        if let Some(token) = child.as_token() {
            if token.kind() == smelt_parser::SyntaxKind::IDENT {
                let _ = ctx.lookup_column(None, token.text());
                visitor(None, token.text(), type_hint, token.text_range());
            }
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

/// Promote two types to their widest compatible type for UNION operations.
///
/// The result type is the type that can hold values from both input types.
/// For example:
/// - INTEGER + BIGINT → BIGINT
/// - VARCHAR(10) + VARCHAR(20) → Text (we don't track length)
/// - INTEGER + DOUBLE → DOUBLE
/// - Unknown + T → T (Unknown is dominated by any known type)
pub fn promote_types(t1: &TypedColumn, t2: &TypedColumn) -> TypedColumn {
    // If either is Unknown, prefer the other
    if matches!(t1.data_type, DataType::Unknown) {
        return TypedColumn {
            data_type: t2.data_type.clone(),
            nullable: t1.nullable || t2.nullable,
        };
    }
    if matches!(t2.data_type, DataType::Unknown) {
        return TypedColumn {
            data_type: t1.data_type.clone(),
            nullable: t1.nullable || t2.nullable,
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

    // Numeric type promotion hierarchy: SmallInt < Integer < BigInt < Decimal < Double
    let promoted_type = match (&t1.data_type, &t2.data_type) {
        // Double dominates all numerics
        (DataType::Double, _) | (_, DataType::Double) => DataType::Double,

        // Decimal dominates integers
        (DataType::Decimal { precision, scale }, _)
        | (_, DataType::Decimal { precision, scale }) => DataType::Decimal {
            precision: *precision,
            scale: *scale,
        },

        // BigInt dominates smaller integers
        (DataType::BigInt, DataType::SmallInt | DataType::Integer)
        | (DataType::SmallInt | DataType::Integer, DataType::BigInt) => DataType::BigInt,

        // Integer dominates SmallInt
        (DataType::Integer, DataType::SmallInt) | (DataType::SmallInt, DataType::Integer) => {
            DataType::Integer
        }

        // Text is a catch-all for string types
        (DataType::Text, _) | (_, DataType::Text) => DataType::Text,
        (DataType::Varchar { .. }, _) | (_, DataType::Varchar { .. }) => DataType::Text,
        (DataType::Char { .. }, _) | (_, DataType::Char { .. }) => DataType::Text,

        // Timestamp types - prefer timezone-aware if either has it
        (
            DataType::Timestamp { with_timezone: tz1 },
            DataType::Timestamp { with_timezone: tz2 },
        ) => DataType::Timestamp {
            with_timezone: *tz1 || *tz2,
        },
        (DataType::Timestamp { with_timezone }, _) | (_, DataType::Timestamp { with_timezone }) => {
            DataType::Timestamp {
                with_timezone: *with_timezone,
            }
        }
        (DataType::Date, DataType::Time) | (DataType::Time, DataType::Date) => {
            DataType::Timestamp {
                with_timezone: false,
            }
        }

        // For incompatible types, return Unknown (could be an error in strict mode)
        _ => DataType::Unknown,
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
                    data_type: DataType::Unknown,
                    nullable: true,
                })
            } else {
                TypedColumn {
                    data_type: DataType::Unknown,
                    nullable: true,
                }
            };
            column_types.push(typed_col);
        }
    }

    // If there's a UNION, recursively get types from the second SELECT and combine
    if select_stmt.has_union() {
        if let Some(union_select) = select_stmt.union_select() {
            let union_types = infer_select_column_types(&union_select, ctx);

            // Combine types - use the wider type for each column position
            for (i, union_type) in union_types.into_iter().enumerate() {
                if i < column_types.len() {
                    column_types[i] = promote_types(&column_types[i], &union_type);
                }
                // If union has more columns, they're ignored (SQL requires same column count)
            }
        }
    }

    column_types
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

    // Helper for testing function types without AST
    fn infer_function_type_by_name(name: &str, _ctx: &TypeContext) -> Option<TypedColumn> {
        match name.to_uppercase().as_str() {
            // Aggregate functions
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
            // Math functions
            "SQRT" | "POWER" | "POW" | "EXP" | "LN" | "LOG" | "LOG10" | "LOG2" => {
                Some(TypedColumn {
                    data_type: DataType::Double,
                    nullable: true,
                })
            }
            "PI" | "RANDOM" => Some(TypedColumn {
                data_type: DataType::Double,
                nullable: false,
            }),
            "SIN" | "COS" | "TAN" | "ASIN" | "ACOS" | "ATAN" => Some(TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            }),
            // Date/time functions
            "EXTRACT" | "DATE_PART" => Some(TypedColumn {
                data_type: DataType::BigInt,
                nullable: true,
            }),
            "MAKE_DATE" => Some(TypedColumn {
                data_type: DataType::Date,
                nullable: true,
            }),
            "AGE" => Some(TypedColumn {
                data_type: DataType::Interval,
                nullable: true,
            }),
            // String functions
            "REPLACE" | "SPLIT_PART" | "LEFT" | "RIGHT" | "LPAD" | "RPAD" => Some(TypedColumn {
                data_type: DataType::Text,
                nullable: true,
            }),
            "POSITION" | "STRPOS" => Some(TypedColumn {
                data_type: DataType::BigInt,
                nullable: true,
            }),
            "STRING_AGG" | "LISTAGG" => Some(TypedColumn {
                data_type: DataType::Text,
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

    #[test]
    fn test_extended_function_types() {
        let ctx = TypeContext::new();

        // Math functions
        let sqrt = infer_function_type_by_name("SQRT", &ctx).unwrap();
        assert_eq!(sqrt.data_type, DataType::Double);

        let power = infer_function_type_by_name("POWER", &ctx).unwrap();
        assert_eq!(power.data_type, DataType::Double);

        let pi = infer_function_type_by_name("PI", &ctx).unwrap();
        assert_eq!(pi.data_type, DataType::Double);
        assert!(!pi.nullable); // PI is never null

        let sin = infer_function_type_by_name("SIN", &ctx).unwrap();
        assert_eq!(sin.data_type, DataType::Double);

        // Date/time functions
        let extract = infer_function_type_by_name("EXTRACT", &ctx).unwrap();
        assert_eq!(extract.data_type, DataType::BigInt);

        let make_date = infer_function_type_by_name("MAKE_DATE", &ctx).unwrap();
        assert_eq!(make_date.data_type, DataType::Date);

        let age = infer_function_type_by_name("AGE", &ctx).unwrap();
        assert_eq!(age.data_type, DataType::Interval);

        // String functions
        let replace = infer_function_type_by_name("REPLACE", &ctx).unwrap();
        assert_eq!(replace.data_type, DataType::Text);

        let position = infer_function_type_by_name("POSITION", &ctx).unwrap();
        assert_eq!(position.data_type, DataType::BigInt);

        let split_part = infer_function_type_by_name("SPLIT_PART", &ctx).unwrap();
        assert_eq!(split_part.data_type, DataType::Text);

        // String aggregate
        let string_agg = infer_function_type_by_name("STRING_AGG", &ctx).unwrap();
        assert_eq!(string_agg.data_type, DataType::Text);
    }
}
