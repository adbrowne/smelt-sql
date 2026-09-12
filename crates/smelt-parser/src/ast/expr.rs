use super::*;
use crate::syntax_kind::SyntaxNode;
use rowan::TextRange;

/// Expression node (represents any SQL expression)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Expr(SyntaxNode);

impl Expr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        // Accept any node that looks like an expression
        match node.kind() {
            EXPRESSION => {
                // Unwrap nested EXPRESSION wrappers to the innermost one.
                // parse_expression() wraps in EXPRESSION, and parse_primary_expr()
                // also wraps bare atoms in EXPRESSION — this avoids double-wrapping
                // issues in accessors that look for direct-child tokens.
                let mut inner = node;
                loop {
                    let children: Vec<_> = inner.children().collect();
                    if children.len() == 1 && children[0].kind() == EXPRESSION {
                        inner = children.into_iter().next().unwrap();
                    } else {
                        break;
                    }
                }
                Some(Self(inner))
            }
            BINARY_EXPR | FUNCTION_CALL | CASE_EXPR | CAST_EXPR | EXTRACT_EXPR | COLLATE_EXPR
            | AT_TIME_ZONE_EXPR | GROUPING_SETS_CLAUSE | GROUPING_SET
            | SUBQUERY | BETWEEN_EXPR | IN_EXPR | EXISTS_EXPR | SMELT_AS_STRUCT_CALL
            | SMELT_PATH_REF | SMELT_PATH_CALL
            // Phase B (meta-language): lambdas and pipe expressions are expressions.
            | LAMBDA | PIPE_EXPR
            // Phase F (meta-language): ternary expressions and reducer calls are expressions.
            | TERNARY_EXPR | REDUCER_CALL
            // Phase E1 (meta-language): type-reference expressions parsed via
            // `is_generic_type_start` in argument positions (e.g. `List<Cohort>`,
            // `Map<Text, {f: T}>` as loader schema arguments).  The TYPE_REF node
            // must be castable to Expr so that `positional_args()` in ArgList
            // returns it and the schema-text extraction in `check_loader_call_diagnostics`
            // can read it via `schema_expr.syntax().text()`.
            | TYPE_REF | RECORD_TYPE_INLINE
            // P7d: Map API method calls with complex receivers
            // (e.g. `smelt.config.load_yaml(...).keys()`).
            | MAP_METHOD_CALL
            // Array literals are directly Expr-castable so that LIST_SPREAD operands
            // (parsed at pipe level, no EXPRESSION wrapper) can be cast when the
            // operand is an inline list — including the empty-list case `...[]`
            // whose ARRAY_LITERAL has no child nodes.
            | ARRAY_LITERAL
            // List comprehensions are directly Expr-castable for the same reason
            // (and so a comprehension nested as an outer comprehension's source
            // list expression, e.g. `[y FOR y IN x FOR-outer...]`, casts cleanly).
            | LIST_COMPREHENSION => Some(Self(node)),
            _ => {
                // Also try to wrap the node if it contains expression-like children
                if node.children().any(|n| {
                    matches!(
                        n.kind(),
                        EXPRESSION
                            | BINARY_EXPR
                            | FUNCTION_CALL
                            | CASE_EXPR
                            | CAST_EXPR
                            | EXTRACT_EXPR
                            | COLLATE_EXPR
                            | AT_TIME_ZONE_EXPR
                            | SUBQUERY
                            | BETWEEN_EXPR
                            | IN_EXPR
                            | EXISTS_EXPR
                            | SMELT_PATH_REF
                            | SMELT_PATH_CALL
                            // Phase B (meta-language)
                            | LAMBDA
                            | PIPE_EXPR
                            // Phase F (meta-language)
                            | TERNARY_EXPR
                            | REDUCER_CALL
                    )
                }) {
                    Some(Self(node))
                } else {
                    None
                }
            }
        }
    }

    /// Try to infer a column name from this expression
    /// Used when there's no explicit alias
    pub fn infer_name(&self) -> Option<String> {
        // Check for wildcard (*)
        if self.text().trim() == "*" {
            return Some("*".to_string());
        }

        // CAST(<inner> AS <type>) — propagate the inner expression's name so
        // an aliased SELECT like `SUM(x)::DOUBLE AS revenue` keeps `revenue`,
        // and a bare `CAST(line_gross AS DOUBLE)` keeps `line_gross`. Without
        // this branch, callers fall through to the IDENT fallback below and
        // pick up the type-spec identifier ("DOUBLE"), or to the SELECT-list
        // numeric placeholder (`_col1`) — both wrong.
        if let Some(cast) = self.as_cast() {
            if let Some(inner) = cast.expression() {
                if let Some(name) = inner.infer_name() {
                    return Some(name);
                }
            }
        }

        // Check if this is a function call
        if let Some(_func) = self.as_function_call() {
            // For function calls without alias, use the full function text
            return Some(self.text());
        }

        // Check if this is a simple column reference
        if let Some(col_ref) = self.as_column_ref() {
            // For qualified names (table.column), use just the column part
            return Some(col_ref.name().to_string());
        }

        // For other complex expressions, try to find the first identifier
        for child in self.0.children_with_tokens() {
            if let Some(token) = child.as_token() {
                if token.kind() == IDENT {
                    return Some(token.text().to_string());
                }
            }
        }

        None
    }

    /// Get the underlying syntax node
    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Get the full text of this expression
    pub fn text(&self) -> String {
        self.0.text().to_string()
    }

    /// Get the text range of this expression
    pub fn text_range(&self) -> TextRange {
        self.0.text_range()
    }

    /// Check if this is a simple column reference (identifier possibly qualified)
    pub fn as_column_ref(&self) -> Option<ColumnRef> {
        ColumnRef::from_expr(self)
    }

    /// Check if this is a function call
    pub fn as_function_call(&self) -> Option<FunctionCall> {
        self.0.children().find_map(FunctionCall::cast).or_else(|| {
            // Check if this node itself is a function call
            FunctionCall::cast(self.0.clone())
        })
    }

    /// Check if this expression wraps a `SMELT_PATH_CALL` node
    /// (`smelt.functions.*` form). Matches both when this `Expr` node IS the
    /// `SMELT_PATH_CALL` and when it wraps one as a direct child.
    pub fn as_smelt_path_call(&self) -> Option<SmeltPathCall> {
        SmeltPathCall::cast(self.0.clone())
            .or_else(|| self.0.children().find_map(SmeltPathCall::cast))
    }

    /// Check if this expression is a `smelt.as_struct(alias [EXCEPT cols])`
    /// call (Phase 38). Matches both when this `Expr` node IS the
    /// `SMELT_AS_STRUCT_CALL` and when it wraps one as a direct child.
    pub fn as_smelt_as_struct_call(&self) -> Option<SmeltAsStructCall> {
        SmeltAsStructCall::cast(self.0.clone())
            .or_else(|| self.0.children().find_map(SmeltAsStructCall::cast))
    }

    /// Check if this expression is a `MAP_METHOD_CALL` node (P7d).
    /// Matches when this `Expr` node IS the `MAP_METHOD_CALL` or wraps one as
    /// a direct child.
    pub fn as_map_method_call(&self) -> Option<MapMethodCall> {
        MapMethodCall::cast(self.0.clone())
            .or_else(|| self.0.children().find_map(MapMethodCall::cast))
    }

    /// Check if this is a CASE expression
    pub fn as_case(&self) -> Option<CaseExpr> {
        CaseExpr::cast(self.0.clone()).or_else(|| self.0.children().find_map(CaseExpr::cast))
    }

    /// Check if this is an EXTRACT expression
    pub fn as_extract(&self) -> Option<ExtractExpr> {
        ExtractExpr::cast(self.0.clone()).or_else(|| self.0.children().find_map(ExtractExpr::cast))
    }

    /// Check if this is an AT TIME ZONE expression
    pub fn as_at_time_zone(&self) -> Option<AtTimeZoneExpr> {
        AtTimeZoneExpr::cast(self.0.clone())
            .or_else(|| self.0.children().find_map(AtTimeZoneExpr::cast))
    }

    /// Check if this is a COLLATE expression
    pub fn as_collate(&self) -> Option<CollateExpr> {
        CollateExpr::cast(self.0.clone()).or_else(|| self.0.children().find_map(CollateExpr::cast))
    }

    /// Check if this is a CAST expression
    pub fn as_cast(&self) -> Option<CastExpr> {
        CastExpr::cast(self.0.clone()).or_else(|| self.0.children().find_map(CastExpr::cast))
    }

    /// Check if this is a subquery
    pub fn as_subquery(&self) -> Option<Subquery> {
        Subquery::cast(self.0.clone()).or_else(|| self.0.children().find_map(Subquery::cast))
    }

    /// Check if this is a BETWEEN expression
    pub fn as_between(&self) -> Option<BetweenExpr> {
        BetweenExpr::cast(self.0.clone()).or_else(|| self.0.children().find_map(BetweenExpr::cast))
    }

    /// Check if this is an IN expression
    pub fn as_in(&self) -> Option<InExpr> {
        InExpr::cast(self.0.clone()).or_else(|| self.0.children().find_map(InExpr::cast))
    }

    /// Check if this is an EXISTS expression
    pub fn as_exists(&self) -> Option<ExistsExpr> {
        ExistsExpr::cast(self.0.clone()).or_else(|| self.0.children().find_map(ExistsExpr::cast))
    }

    /// Check if this is a binary expression.
    ///
    /// Self-cast is tried FIRST: a node that is itself a `BINARY_EXPR` can
    /// have a same-kind first child (left-associative chains parse
    /// `a AND b AND c` as `(a AND b) AND c` with bare `BINARY_EXPR`
    /// operands), and a child-first cast would silently return that child,
    /// dropping the right operand from every recursive walk. The child
    /// lookup remains only to unwrap `EXPRESSION` wrapper nodes, which can
    /// never themselves cast.
    pub fn as_binary(&self) -> Option<BinaryExpr> {
        BinaryExpr::cast(self.0.clone()).or_else(|| self.0.children().find_map(BinaryExpr::cast))
    }

    /// Check if this is an array literal (ARRAY[1, 2, 3])
    pub fn as_array_literal(&self) -> Option<ArrayLiteral> {
        ArrayLiteral::cast(self.0.clone())
            .or_else(|| self.0.children().find_map(ArrayLiteral::cast))
    }

    /// Check if this is a list comprehension (`[expr FOR x IN list]`)
    pub fn as_list_comprehension(&self) -> Option<ListComprehension> {
        ListComprehension::cast(self.0.clone())
            .or_else(|| self.0.children().find_map(ListComprehension::cast))
    }

    /// Check if this contains an array subscript (expr[index])
    pub fn as_array_subscript(&self) -> Option<ArraySubscript> {
        ArraySubscript::cast(self.0.clone())
            .or_else(|| self.0.children().find_map(ArraySubscript::cast))
    }

    /// Check if this contains an array slice (expr[start:end])
    pub fn as_array_slice(&self) -> Option<ArraySlice> {
        ArraySlice::cast(self.0.clone()).or_else(|| self.0.children().find_map(ArraySlice::cast))
    }

    /// Check if this is a ROW constructor (ROW(1, 2, 3))
    pub fn as_row_constructor(&self) -> Option<RowConstructor> {
        RowConstructor::cast(self.0.clone())
            .or_else(|| self.0.children().find_map(RowConstructor::cast))
    }

    /// Check if this is a struct literal (STRUCT(1 AS a, 'hello' AS b))
    pub fn as_struct_literal(&self) -> Option<StructLiteral> {
        StructLiteral::cast(self.0.clone())
            .or_else(|| self.0.children().find_map(StructLiteral::cast))
    }

    /// Check if this is a MAP literal (MAP {'a': 1, 'b': 2})
    pub fn as_map_literal(&self) -> Option<MapLiteral> {
        MapLiteral::cast(self.0.clone()).or_else(|| self.0.children().find_map(MapLiteral::cast))
    }

    /// Check if this is a brace-struct literal (`{expr AS name, ..spread}`
    /// meta-language form, or a DuckDB struct/dict literal `{'a': 1}`).
    pub fn as_brace_struct_literal(&self) -> Option<BraceStructLiteral> {
        BraceStructLiteral::cast(self.0.clone())
            .or_else(|| self.0.children().find_map(BraceStructLiteral::cast))
    }

    /// Check if this expression has a window specification (OVER clause)
    pub fn window_spec(&self) -> Option<WindowSpec> {
        self.0.children().find_map(WindowSpec::cast)
    }
}

/// Column reference (identifier, possibly qualified like "table.column")
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ColumnRef {
    qualifier: Option<String>,
    name: String,
}

impl ColumnRef {
    /// Try to parse a column reference from an expression
    pub fn from_expr(expr: &Expr) -> Option<Self> {
        let tokens: Vec<_> = expr
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == IDENT || t.kind() == DOT)
            .collect();

        if tokens.is_empty() {
            return None;
        }

        // Simple identifier — but NOT a typed literal such as `INTERVAL '1 day'`,
        // `DATE '2026-01-01'`, etc. The lexer emits those as a type-keyword
        // `IDENT` followed by a `STRING` inside one EXPRESSION node; the IDENT
        // alone would otherwise be mistaken for a column named "INTERVAL".
        if tokens.len() == 1 && tokens[0].kind() == IDENT {
            let is_type_keyword = matches!(
                tokens[0].text().to_uppercase().as_str(),
                "DATE" | "TIME" | "TIMESTAMP" | "INTERVAL"
            );
            let has_string_literal = expr
                .0
                .children_with_tokens()
                .filter_map(|e| e.into_token())
                .any(|t| t.kind() == STRING);
            if is_type_keyword && has_string_literal {
                return None;
            }
            return Some(ColumnRef {
                qualifier: None,
                name: tokens[0].text().to_string(),
            });
        }

        // Qualified identifier: table.column
        if tokens.len() >= 3
            && tokens[0].kind() == IDENT
            && tokens[1].kind() == DOT
            && tokens[2].kind() == IDENT
        {
            return Some(ColumnRef {
                qualifier: Some(tokens[0].text().to_string()),
                name: tokens[2].text().to_string(),
            });
        }

        None
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn qualifier(&self) -> Option<&str> {
        self.qualifier.as_deref()
    }
}

/// Binary expression (e.g., a + b, x AND y, col = 'value')
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BinaryExpr(SyntaxNode);

impl BinaryExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == BINARY_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the underlying syntax node
    pub fn node(&self) -> &SyntaxNode {
        &self.0
    }

    /// Get the left operand expression
    pub fn left(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    /// Get the right operand expression
    pub fn right(&self) -> Option<Expr> {
        self.0.children().filter_map(Expr::cast).nth(1)
    }

    /// Get the operator as a string
    pub fn operator(&self) -> Option<String> {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|c| c.into_token())
            .collect();
        for (i, token) in tokens.iter().enumerate() {
            match token.kind() {
                PLUS => return Some("+".to_string()),
                MINUS => return Some("-".to_string()),
                STAR | MULTIPLY => return Some("*".to_string()),
                DIVIDE => return Some("/".to_string()),
                PERCENT => return Some("%".to_string()),
                DOUBLE_STAR => return Some("**".to_string()),
                CARET => return Some("^".to_string()),
                FLOOR_DIVIDE => return Some("//".to_string()),
                EQ => return Some("=".to_string()),
                NE => return Some("<>".to_string()),
                LT => return Some("<".to_string()),
                GT => return Some(">".to_string()),
                LE => return Some("<=".to_string()),
                GE => return Some(">=".to_string()),
                CONCAT => return Some("||".to_string()),
                AND_KW => return Some("AND".to_string()),
                OR_KW => return Some("OR".to_string()),
                IS_KW => return Some("IS".to_string()),
                NOT_KW => {
                    // A leading `NOT` in a BINARY_EXPR is either the unary
                    // boolean NOT (this node kind is reused for unary
                    // operators — `right()` is `None` in that case) or the
                    // prefix of a NOT-prefixed binary pattern-match operator
                    // (`NOT LIKE`, `NOT ILIKE`, `NOT SIMILAR TO`) or the bare
                    // `expr NOT NULL` sugar for `expr IS NOT NULL`. `NOT
                    // IN`/`NOT BETWEEN` are distinct node kinds
                    // (IN_EXPR/BETWEEN_EXPR), not BINARY_EXPR, so they never
                    // reach this arm. DuckDB itself rejects `NOT GLOB`
                    // (verified against a live DuckDB), so GLOB has no
                    // compound form here.
                    let next_kw_text = tokens[i + 1..]
                        .iter()
                        .find(|t| !t.kind().is_trivia())
                        .map(|t| t.text().to_string());
                    return match next_kw_text.as_deref() {
                        Some(t) if t.eq_ignore_ascii_case("LIKE") => Some("NOT LIKE".to_string()),
                        Some(t) if t.eq_ignore_ascii_case("ILIKE") => Some("NOT ILIKE".to_string()),
                        Some(t) if t.eq_ignore_ascii_case("SIMILAR") => {
                            Some("NOT SIMILAR TO".to_string())
                        }
                        // `expr NOT NULL` — treated as `IS` so it flows
                        // through the same nullability-checking dispatch arm
                        // as `expr IS NOT NULL`.
                        Some(t) if t.eq_ignore_ascii_case("NULL") => Some("IS".to_string()),
                        _ => Some("NOT".to_string()),
                    };
                }
                LIKE_KW => return Some("LIKE".to_string()),
                ILIKE_KW => return Some("ILIKE".to_string()),
                GLOB_KW => return Some("GLOB".to_string()),
                TILDE => return Some("~".to_string()),
                TILDE_STAR => return Some("~*".to_string()),
                NOT_TILDE => return Some("!~".to_string()),
                NOT_TILDE_STAR => return Some("!~*".to_string()),
                JSON_ARROW => return Some("->".to_string()),
                JSON_ARROW_TEXT => return Some("->>".to_string()),
                HASH_ARROW => return Some("#>".to_string()),
                HASH_ARROW_TEXT => return Some("#>>".to_string()),
                AT_GT => return Some("@>".to_string()),
                LT_AT => return Some("<@".to_string()),
                IDENT if token.text().eq_ignore_ascii_case("SIMILAR") => {
                    return Some("SIMILAR TO".to_string())
                }
                _ => {}
            }
        }
        None
    }

    /// Get the TextRange of the arithmetic operator token (+, -, *, /, %).
    /// Returns `None` for non-arithmetic operators or if no token is found.
    /// Used to anchor `TypeMismatch` diagnostics at the operator span.
    pub fn operator_token_range(&self) -> Option<rowan::TextRange> {
        for child in self.0.children_with_tokens() {
            if let Some(token) = child.as_token() {
                match token.kind() {
                    PLUS | MINUS | STAR | MULTIPLY | DIVIDE | PERCENT | DOUBLE_STAR | CARET
                    | FLOOR_DIVIDE => {
                        return Some(token.text_range());
                    }
                    _ => {}
                }
            }
        }
        None
    }

    /// Check if this is a unary expression (e.g., -x, NOT y)
    /// Unary expressions have no right operand
    pub fn is_unary(&self) -> bool {
        self.right().is_none()
    }

    /// Get the unary operand as a column reference
    /// For unary expressions where the operand is a simple identifier (not wrapped in a node),
    /// this extracts the column reference from tokens.
    pub fn unary_operand_column(&self) -> Option<ColumnRef> {
        // First try getting as a normal expression
        if let Some(expr) = self.left() {
            return expr.as_column_ref();
        }

        // For unary expressions, the operand might be a bare identifier token
        // not wrapped in an expression node. Extract column ref from tokens.
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == IDENT || t.kind() == DOT)
            .collect();

        if tokens.is_empty() {
            return None;
        }

        // Simple identifier
        if tokens.len() == 1 && tokens[0].kind() == IDENT {
            return Some(ColumnRef {
                qualifier: None,
                name: tokens[0].text().to_string(),
            });
        }

        // Qualified identifier: table.column
        if tokens.len() >= 3
            && tokens[0].kind() == IDENT
            && tokens[1].kind() == DOT
            && tokens[2].kind() == IDENT
        {
            return Some(ColumnRef {
                qualifier: Some(tokens[0].text().to_string()),
                name: tokens[2].text().to_string(),
            });
        }

        None
    }
}

/// Function call expression
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionCall(SyntaxNode);

impl FunctionCall {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == FUNCTION_CALL {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the function name (e.g., "COUNT", "SUM", "ref")
    /// For namespaced calls like smelt.ref(), returns just "ref"
    ///
    /// Keyword tokens that are valid function names (e.g. `LEFT_KW`, `RIGHT_KW`,
    /// `FILTER_KW` — the same set handled by `at_keyword_as_function_name`) are
    /// also recognised here so that `LEFT(str, 3)` returns `Some("LEFT")`.
    pub fn name(&self) -> Option<String> {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .collect();

        // Check for namespaced call: IDENT DOT IDENT
        if tokens.len() >= 3
            && tokens[0].kind() == IDENT
            && tokens[1].kind() == DOT
            && tokens[2].kind() == IDENT
        {
            return Some(tokens[2].text().to_string());
        }

        // Simple call: IDENT or a keyword that can be used as a function name.
        // The parser's `at_keyword_as_function_name()` recognises
        // `LEFT_KW`, `RIGHT_KW`, `FILTER_KW`, `QUALIFY_KW`, `PIVOT_KW`,
        // `UNPIVOT_KW`, `VALUES_KW`, and `FN_KW` as valid function-name tokens
        // when followed by `(`. We mirror that set here so that
        // e.g. `LEFT(str, 3)` (which has a `LEFT_KW` name token) returns
        // `Some("LEFT")` instead of `None`.
        tokens
            .iter()
            .find(|t| {
                let k = t.kind();
                k == IDENT
                    || k == LEFT_KW
                    || k == RIGHT_KW
                    || k == FILTER_KW
                    || k == QUALIFY_KW
                    || k == PIVOT_KW
                    || k == UNPIVOT_KW
                    || k == VALUES_KW
                    || k == FN_KW
                    || k == GLOB_KW
            })
            .map(|t| t.text().to_string())
    }

    /// Get the namespace prefix if this is a namespaced call (e.g., "smelt" from smelt.ref())
    pub fn namespace(&self) -> Option<String> {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .collect();

        // Check for namespaced call: IDENT DOT IDENT
        if tokens.len() >= 3
            && tokens[0].kind() == IDENT
            && tokens[1].kind() == DOT
            && tokens[2].kind() == IDENT
        {
            Some(tokens[0].text().to_string())
        } else {
            None
        }
    }

    /// Get the text of the full function call
    pub fn text(&self) -> String {
        self.0.text().to_string()
    }

    /// Get all named parameters from this function call
    pub fn named_params(&self) -> impl Iterator<Item = NamedParam> + '_ {
        self.0.descendants().filter_map(NamedParam::cast)
    }

    /// Get the underlying syntax node
    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Get the arguments from this function call
    pub fn arguments(&self) -> Vec<Expr> {
        // Find ARG_LIST child and return EXPRESSION children
        self.0
            .children()
            .filter(|n| n.kind() == ARG_LIST)
            .flat_map(|arg_list| arg_list.children().filter_map(Expr::cast))
            .collect()
    }

    /// Get the FILTER clause if present (PostgreSQL aggregate filter)
    pub fn filter_clause(&self) -> Option<FilterClause> {
        self.0.children().find_map(FilterClause::cast)
    }
}

/// FILTER clause for aggregate functions (e.g., FILTER (WHERE status = 'active'))
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FilterClause(SyntaxNode);

impl FilterClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == FILTER_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the filter condition expression
    pub fn expression(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }
}

/// Array literal: ARRAY[1, 2, 3]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArrayLiteral(SyntaxNode);

impl ArrayLiteral {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == ARRAY_LITERAL {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get all element expressions in the array literal
    pub fn elements(&self) -> Vec<Expr> {
        self.0.children().filter_map(Expr::cast).collect()
    }
}

/// List comprehension: `[expr FOR ident IN list (IF cond)?]` (DuckDB).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ListComprehension(SyntaxNode);

impl ListComprehension {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == LIST_COMPREHENSION {
            Some(Self(node))
        } else {
            None
        }
    }

    /// The result-element expression, e.g. `x + 1` in `[x + 1 FOR x IN l]`.
    pub fn element(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    /// The loop variable name, e.g. `x` in `[x FOR x IN l]`.
    pub fn var_name(&self) -> Option<String> {
        self.0
            .children()
            .find(|n| n.kind() == LIST_COMPREHENSION_VAR)
            .and_then(|n| {
                n.children_with_tokens()
                    .filter_map(|t| t.into_token())
                    .find(|t| t.kind() == IDENT)
            })
            .map(|t| t.text().to_string())
    }

    /// The source list expression, e.g. `l` in `[x FOR x IN l]`.
    pub fn source(&self) -> Option<Expr> {
        self.0.children().filter_map(Expr::cast).nth(1)
    }

    /// The optional `IF` filter condition, e.g. `x > 1` in
    /// `[x FOR x IN l IF x > 1]`.
    pub fn filter(&self) -> Option<Expr> {
        self.0.children().filter_map(Expr::cast).nth(2)
    }
}

/// Array subscript: expr[index]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArraySubscript(SyntaxNode);

impl ArraySubscript {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == ARRAY_SUBSCRIPT {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn index(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }
}

/// Array slice: expr[start:end]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArraySlice(SyntaxNode);

impl ArraySlice {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == ARRAY_SLICE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn start(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    pub fn end(&self) -> Option<Expr> {
        // Get the second expression (after the colon)
        self.0.children().filter_map(Expr::cast).nth(1)
    }
}

/// ROW constructor: ROW(1, 2, 3)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RowConstructor(SyntaxNode);

impl RowConstructor {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == ROW_CONSTRUCTOR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get all element expressions in the ROW constructor
    pub fn elements(&self) -> Vec<Expr> {
        self.0.children().filter_map(Expr::cast).collect()
    }
}

/// Struct literal: STRUCT(1 AS a, 'hello' AS b)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StructLiteral(SyntaxNode);

impl StructLiteral {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == STRUCT_LITERAL {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get field expressions and their optional names.
    /// Returns (expression, optional_name) pairs.
    pub fn fields(&self) -> Vec<(Expr, Option<String>)> {
        let mut result = Vec::new();
        let mut current_expr: Option<Expr> = None;

        for child in self.0.children_with_tokens() {
            match child {
                rowan::NodeOrToken::Node(node) => {
                    if let Some(expr) = Expr::cast(node) {
                        // If we had a previous expression without a name, push it
                        if let Some(prev) = current_expr.take() {
                            result.push((prev, None));
                        }
                        current_expr = Some(expr);
                    }
                }
                rowan::NodeOrToken::Token(token) => {
                    if token.kind() == AS_KW {
                        // Next IDENT token is the field name
                        continue;
                    }
                    if token.kind() == IDENT {
                        if let Some(expr) = current_expr.take() {
                            result.push((expr, Some(token.text().to_string())));
                        }
                    }
                    if token.kind() == COMMA {
                        // Flush any pending unnamed expression
                        if let Some(expr) = current_expr.take() {
                            result.push((expr, None));
                        }
                    }
                }
            }
        }
        // Flush last expression
        if let Some(expr) = current_expr.take() {
            result.push((expr, None));
        }
        result
    }
}

/// MAP literal: MAP {'a': 1, 'b': 2}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MapLiteral(SyntaxNode);

impl MapLiteral {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == MAP_LITERAL {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Get all (key, value) entry expression pairs.
    pub fn entries(&self) -> Vec<MapEntry> {
        self.0.children().filter_map(MapEntry::cast).collect()
    }
}

/// A single `key : value` entry inside a `MapLiteral`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MapEntry(SyntaxNode);

impl MapEntry {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == MAP_ENTRY {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn key(&self) -> Option<Expr> {
        self.0.children().filter_map(Expr::cast).next()
    }

    pub fn value(&self) -> Option<Expr> {
        self.0.children().filter_map(Expr::cast).nth(1)
    }
}

/// PIVOT clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PivotClause(SyntaxNode);

impl PivotClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == PIVOT_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

/// UNPIVOT clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnpivotClause(SyntaxNode);

impl UnpivotClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == UNPIVOT_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

/// Lambda expression (e.g., x -> x + 1 or (acc, x) -> acc + x)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LambdaExpr(SyntaxNode);

impl LambdaExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == LAMBDA_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the parameter names
    pub fn params(&self) -> Vec<String> {
        self.0
            .children()
            .find(|n| n.kind() == LAMBDA_PARAM_LIST)
            .map(|param_list| {
                param_list
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .filter(|t| t.kind() == IDENT)
                    .map(|t| t.text().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get the body expression
    pub fn body(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }
}

/// Named parameter in a function call (e.g., filter => expr)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NamedParam(SyntaxNode);

impl NamedParam {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == NAMED_PARAM {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the parameter name (the identifier before =>)
    pub fn name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// Get the parameter value as text (everything after =>)
    pub fn value_text(&self) -> String {
        // Get the full text and extract everything after the =>
        let full_text = self.0.text().to_string();

        // Find the => and return everything after it, trimmed
        if let Some(arrow_pos) = full_text.find("=>") {
            full_text[arrow_pos + 2..].trim().to_string()
        } else {
            String::new()
        }
    }

    /// The value expression (the sub-expression after `=>`), if one can be
    /// extracted as an `Expr`. Phase 6 uses this to type-check the value
    /// against the declared parameter's type at a call site.
    pub fn value_expr(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    /// Text range of the parameter-name identifier (before `=>`), suitable
    /// for anchoring `MissingArgument` / duplicate-name diagnostics at the
    /// call site.
    pub fn name_range(&self) -> Option<TextRange> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text_range())
    }

    /// Full text range of this NAMED_PARAM node (name + `=>` + value).
    pub fn text_range(&self) -> TextRange {
        self.0.text_range()
    }
}

/// Position (line, column) — codepoint-based.
///
/// Used only by the LSP boundary-converter helpers in
/// `smelt-lsp::diagnostics_boundary`. The `column` field counts Unicode
/// codepoints, **not** bytes or UTF-16 code units.
///
/// Diagnostic positions must be carried as `rowan::TextRange` (byte offsets)
/// and converted at the boundary via `line_index::LineIndex`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

/// Range (start, end positions) — codepoint-based.
///
/// See `Position` for encoding notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

// ===== Phase 10: Expression Enhancement AST Wrappers =====

/// CASE expression (CASE WHEN ... THEN ... ELSE ... END)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CaseExpr(SyntaxNode);

impl CaseExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == CASE_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the underlying syntax node
    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Get the case value expression (for simple CASE)
    /// Returns None for searched CASE (CASE WHEN ...)
    pub fn case_value(&self) -> Option<Expr> {
        // The case value is the first EXPRESSION-like child, before any WHEN_CLAUSE
        self.0
            .children()
            .take_while(|n| n.kind() != WHEN_CLAUSE)
            .find_map(Expr::cast)
    }

    /// Get all WHEN clauses
    pub fn when_clauses(&self) -> impl Iterator<Item = WhenClause> + '_ {
        self.0.children().filter_map(WhenClause::cast)
    }

    /// Get the ELSE expression if present
    pub fn else_expr(&self) -> Option<Expr> {
        // The ELSE expression is the last EXPRESSION child, after all WHEN clauses
        let mut found_else = false;
        for child in self.0.children_with_tokens() {
            if let Some(token) = child.as_token() {
                if token.kind() == ELSE_KW {
                    found_else = true;
                }
            } else if found_else {
                if let Some(node) = child.as_node() {
                    if let Some(expr) = Expr::cast(node.clone()) {
                        return Some(expr);
                    }
                }
            }
        }
        None
    }
}

/// WHEN clause in a CASE expression
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WhenClause(SyntaxNode);

impl WhenClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == WHEN_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the condition expression (after WHEN)
    pub fn condition(&self) -> Option<Expr> {
        // First EXPRESSION child
        self.0.children().find_map(Expr::cast)
    }

    /// Get the result expression (after THEN)
    pub fn result(&self) -> Option<Expr> {
        // Second EXPRESSION child
        self.0.children().filter_map(Expr::cast).nth(1)
    }
}

/// CAST expression (CAST(expr AS type) or expr::type)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CastExpr(SyntaxNode);

impl CastExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == CAST_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the expression being cast
    pub fn expression(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    /// Get the type specification
    pub fn type_spec(&self) -> Option<TypeSpec> {
        self.0.children().find_map(TypeSpec::cast)
    }

    /// Check if this is a PostgreSQL :: cast (vs CAST(...))
    pub fn is_double_colon_cast(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == DOUBLE_COLON)
    }

    /// Check if this is a `TRY_CAST(...)` (DuckDB error-tolerant cast) rather
    /// than a plain `CAST(...)` / `::` cast. A `TRY_CAST` returns NULL on a
    /// failed conversion, so its result is always nullable.
    pub fn is_try_cast(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == TRY_CAST_KW)
    }
}

/// EXTRACT expression (EXTRACT(field FROM expr))
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExtractExpr(SyntaxNode);

impl ExtractExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == EXTRACT_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the field name (EPOCH, YEAR, MONTH, DAY, HOUR, MINUTE, SECOND)
    pub fn field_name(&self) -> Option<String> {
        // The field is the first IDENT or keyword token after EXTRACT_KW and LPAREN
        let mut after_lparen = false;
        for elem in self.0.children_with_tokens() {
            if let Some(token) = elem.as_token() {
                match token.kind() {
                    LPAREN => after_lparen = true,
                    IDENT if after_lparen => return Some(token.text().to_uppercase()),
                    k if k.is_keyword() && after_lparen && k != FROM_KW => {
                        return Some(token.text().to_uppercase())
                    }
                    _ => {}
                }
            }
        }
        None
    }

    /// Get the source expression (the expression after FROM)
    pub fn expression(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

/// COLLATE expression (expr COLLATE collation_name)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CollateExpr(SyntaxNode);

impl CollateExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == COLLATE_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the operand expression (the left-hand side of COLLATE)
    pub fn operand(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    /// Get the collation name. If the token is a quoted STRING, the surrounding
    /// quotes are stripped. Returns `None` if no name token is found.
    pub fn collation_name(&self) -> Option<String> {
        let mut after_collate_kw = false;
        for elem in self.0.children_with_tokens() {
            if let Some(token) = elem.as_token() {
                match token.kind() {
                    COLLATE_KW => after_collate_kw = true,
                    IDENT if after_collate_kw => return Some(token.text().to_string()),
                    STRING if after_collate_kw => {
                        let raw = token.text();
                        // Strip surrounding single or double quotes
                        let inner = raw
                            .strip_prefix('"')
                            .and_then(|s| s.strip_suffix('"'))
                            .or_else(|| raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                            .unwrap_or(raw);
                        return Some(inner.to_string());
                    }
                    _ => {}
                }
            }
        }
        None
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

/// `expr AT TIME ZONE tz_expr` timezone-conversion expression.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AtTimeZoneExpr(SyntaxNode);

impl AtTimeZoneExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == AT_TIME_ZONE_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the operand expression (the left-hand side of AT TIME ZONE).
    pub fn operand(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    /// Get the timezone expression (the right-hand side, after ZONE).
    pub fn timezone_expr(&self) -> Option<Expr> {
        self.0.children().filter_map(Expr::cast).nth(1)
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

/// Type specification (e.g., INTEGER, VARCHAR(255), DECIMAL(10,2))
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeSpec(SyntaxNode);

impl TypeSpec {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == TYPE_SPEC {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the type name (e.g., "INTEGER", "VARCHAR")
    pub fn type_name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// Get the full text including parameters (e.g., "VARCHAR(255)")
    pub fn full_text(&self) -> String {
        self.0.text().to_string()
    }
}

/// VALUES clause: `VALUES (expr, …), …`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValuesClause(SyntaxNode);

impl ValuesClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == VALUES_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

/// Subquery (SELECT statement in parentheses)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Subquery(pub(crate) SyntaxNode);

impl Subquery {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SUBQUERY {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the SELECT statement (returns `None` for VALUES subqueries).
    ///
    /// Unwraps arbitrarily deep redundant parenthesization —
    /// `(((SELECT …)))` — by descending through nested `SUBQUERY` children
    /// until a direct `SELECT_STMT` child is found.
    pub fn select_stmt(&self) -> Option<SelectStmt> {
        let mut node = self.0.clone();
        loop {
            if let Some(select) = node.children().find_map(SelectStmt::cast) {
                return Some(select);
            }
            node = node.children().find(|n| n.kind() == SUBQUERY)?;
        }
    }

    /// Get the VALUES clause if this is a `(VALUES …)` subquery.
    pub fn values_clause(&self) -> Option<ValuesClause> {
        self.0.children().find_map(ValuesClause::cast)
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

/// BETWEEN expression (expr BETWEEN low AND high)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BetweenExpr(SyntaxNode);

impl BetweenExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == BETWEEN_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the lower bound expression
    pub fn lower_bound(&self) -> Option<Expr> {
        // First EXPRESSION child
        self.0.children().find_map(Expr::cast)
    }

    /// Get the upper bound expression
    pub fn upper_bound(&self) -> Option<Expr> {
        // Second EXPRESSION child
        self.0.children().filter_map(Expr::cast).nth(1)
    }

    /// True for `expr NOT BETWEEN low AND high`. The leading `NOT` (if
    /// present) is a direct token child of this node — not nested inside
    /// either operand — so a plain token scan is unambiguous.
    pub fn is_negated(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|c| c.into_token())
            .any(|t| t.kind() == NOT_KW)
    }
}

/// IN expression (expr IN (values...) or expr IN (subquery))
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InExpr(SyntaxNode);

impl InExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == IN_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Check if this is a subquery IN (vs value list)
    pub fn is_subquery(&self) -> bool {
        self.0.children().any(|n| n.kind() == SUBQUERY)
    }

    /// Get the subquery (if this is IN (subquery))
    pub fn subquery(&self) -> Option<Subquery> {
        self.0.children().find_map(Subquery::cast)
    }

    /// Get the value expressions (if this is IN (value1, value2, ...))
    pub fn values(&self) -> Vec<Expr> {
        if self.is_subquery() {
            Vec::new()
        } else {
            self.0.children().filter_map(Expr::cast).collect()
        }
    }

    /// True for `expr NOT IN (...)`. The leading `NOT` (if present) is a
    /// direct token child of this node — not nested inside the left operand
    /// or the value list — so a plain token scan is unambiguous.
    pub fn is_negated(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|c| c.into_token())
            .any(|t| t.kind() == NOT_KW)
    }
}

/// EXISTS expression (EXISTS (subquery))
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExistsExpr(SyntaxNode);

impl ExistsExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == EXISTS_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the subquery
    pub fn subquery(&self) -> Option<Subquery> {
        self.0.children().find_map(Subquery::cast)
    }
}
