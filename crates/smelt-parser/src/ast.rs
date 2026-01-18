/// Typed AST wrappers over Rowan CST
use crate::syntax_kind::SyntaxNode;
use crate::SyntaxKind::*;
use rowan::TextRange;

/// Root file node
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct File(SyntaxNode);

impl File {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == FILE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn select_stmt(&self) -> Option<SelectStmt> {
        self.0.children().find_map(SelectStmt::cast)
    }

    /// Find all ref('model') function calls in the file
    pub fn refs(&self) -> impl Iterator<Item = RefCall> + '_ {
        self.0
            .descendants()
            .filter_map(FunctionCall::cast)
            .filter_map(RefCall::from_function_call)
    }

    /// Find all source('source.table') function calls in the file
    pub fn sources(&self) -> impl Iterator<Item = SourceCall> + '_ {
        self.0
            .descendants()
            .filter_map(FunctionCall::cast)
            .filter_map(SourceCall::from_function_call)
    }

    /// Get the underlying syntax node (for printer)
    #[allow(dead_code)] // Used by printer module
    pub(crate) fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

/// SELECT statement
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SelectStmt(SyntaxNode);

impl SelectStmt {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SELECT_STMT {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn with_clause(&self) -> Option<WithClause> {
        self.0.children().find_map(WithClause::cast)
    }

    pub fn select_list(&self) -> Option<SelectList> {
        self.0.children().find_map(SelectList::cast)
    }

    pub fn from_clause(&self) -> Option<FromClause> {
        self.0.children().find_map(FromClause::cast)
    }

    pub fn where_clause(&self) -> Option<WhereClause> {
        self.0.children().find_map(WhereClause::cast)
    }

    pub fn having_clause(&self) -> Option<HavingClause> {
        self.0.children().find_map(HavingClause::cast)
    }

    pub fn order_by_clause(&self) -> Option<OrderByClause> {
        self.0.children().find_map(OrderByClause::cast)
    }

    pub fn limit_clause(&self) -> Option<LimitClause> {
        self.0.children().find_map(LimitClause::cast)
    }

    pub fn is_distinct(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == DISTINCT_KW)
    }

    /// Get the underlying syntax node (for printer)
    #[allow(dead_code)] // Used by printer module
    pub(crate) fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Check if this SELECT has a UNION clause
    pub fn has_union(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == UNION_KW)
    }

    /// Check if the UNION is UNION ALL (vs regular UNION which removes duplicates)
    pub fn is_union_all(&self) -> bool {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .collect();

        for (i, token) in tokens.iter().enumerate() {
            if token.kind() == UNION_KW {
                // Skip whitespace to find next meaningful token
                for next_token in &tokens[i + 1..] {
                    match next_token.kind() {
                        WHITESPACE | COMMENT => continue,
                        ALL_KW => return true,
                        _ => break,
                    }
                }
            }
        }
        false
    }

    /// Get the SELECT statement after UNION (if any)
    pub fn union_select(&self) -> Option<SelectStmt> {
        let mut found_union = false;

        for child in self.0.children_with_tokens() {
            if let Some(token) = child.as_token() {
                if token.kind() == UNION_KW {
                    found_union = true;
                }
            } else if found_union {
                if let Some(n) = child.as_node() {
                    if n.kind() == SELECT_STMT {
                        return SelectStmt::cast(n.clone());
                    }
                }
            }
        }
        None
    }
}

/// SELECT list (columns)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SelectList(SyntaxNode);

impl SelectList {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SELECT_LIST {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn items(&self) -> impl Iterator<Item = SelectItem> + '_ {
        self.0.children().filter_map(SelectItem::cast)
    }
}

/// SELECT item (column or expression with optional alias)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SelectItem(SyntaxNode);

impl SelectItem {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SELECT_ITEM {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the expression node for this select item
    pub fn expression(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    /// Get the explicit alias if present (the identifier after AS keyword)
    pub fn alias(&self) -> Option<String> {
        let mut found_as = false;

        for child in self.0.children_with_tokens() {
            if let Some(token) = child.as_token() {
                if token.kind() == AS_KW {
                    found_as = true;
                } else if found_as && token.kind() == IDENT {
                    return Some(token.text().to_string());
                }
            }
        }
        None
    }

    /// Get the effective column name (alias if present, otherwise inferred from expression)
    pub fn column_name(&self) -> Option<String> {
        // If there's an alias, use it
        if let Some(alias) = self.alias() {
            return Some(alias);
        }

        // Otherwise, try to infer from expression
        if let Some(expr) = self.expression() {
            expr.infer_name()
        } else {
            None
        }
    }

    /// Get the text range of this select item
    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }

    /// Get the underlying syntax node (for printer)
    #[allow(dead_code)] // Used by printer module
    pub(crate) fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

/// FROM clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FromClause(SyntaxNode);

impl FromClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == FROM_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn table_refs(&self) -> impl Iterator<Item = TableRef> + '_ {
        self.0.children().filter_map(TableRef::cast)
    }

    pub fn joins(&self) -> impl Iterator<Item = JoinClause> + '_ {
        self.0.children().filter_map(JoinClause::cast)
    }

    /// Get the text range of this FROM clause
    pub fn text_range(&self) -> TextRange {
        self.0.text_range()
    }

    /// Get the full text of this FROM clause
    pub fn text(&self) -> String {
        self.0.text().to_string()
    }
}

/// JOIN clause (JOIN type + table + condition)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JoinClause(SyntaxNode);

impl JoinClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == JOIN_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the JOIN type (INNER, LEFT, RIGHT, FULL, CROSS)
    /// Returns None for bare JOIN (defaults to INNER)
    pub fn join_type(&self) -> Option<JoinType> {
        for token in self.0.children_with_tokens().filter_map(|e| e.into_token()) {
            match token.kind() {
                INNER_KW => return Some(JoinType::Inner),
                LEFT_KW => return Some(JoinType::Left),
                RIGHT_KW => return Some(JoinType::Right),
                FULL_KW => return Some(JoinType::Full),
                CROSS_KW => return Some(JoinType::Cross),
                _ => continue,
            }
        }
        None // Bare JOIN, defaults to INNER
    }

    /// Get the table reference being joined
    pub fn table_ref(&self) -> Option<TableRef> {
        self.0.children().find_map(TableRef::cast)
    }

    /// Get the join condition (ON or USING clause)
    pub fn condition(&self) -> Option<JoinCondition> {
        self.0.children().find_map(JoinCondition::cast)
    }
}

/// JOIN type enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Full,
    Cross,
}

/// JOIN condition (ON expr or USING cols)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JoinCondition(SyntaxNode);

impl JoinCondition {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == JOIN_CONDITION {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Check if this is an ON condition (vs USING)
    pub fn is_on(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == ON_KW)
    }

    /// Check if this is a USING condition
    pub fn is_using(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == USING_KW)
    }

    /// Get the ON expression (if this is an ON condition)
    pub fn on_expression(&self) -> Option<Expr> {
        if self.is_on() {
            self.0.children().find_map(Expr::cast)
        } else {
            None
        }
    }

    /// Get the USING column list as strings
    pub fn using_columns(&self) -> Vec<String> {
        if !self.is_using() {
            return Vec::new();
        }

        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
            .collect()
    }
}

/// Table reference (identifier or template expression)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TableRef(SyntaxNode);

impl TableRef {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == TABLE_REF {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Check if this is a function call reference (like ref('model'))
    pub fn is_function_call(&self) -> bool {
        self.0.children().any(|n| n.kind() == FUNCTION_CALL)
    }

    /// Get the function call if this table ref is a function (like ref('model'))
    pub fn function_call(&self) -> Option<FunctionCall> {
        self.0.children().find_map(FunctionCall::cast)
    }

    pub fn identifier(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// Get the alias if present (explicit AS alias or implicit alias after table ref)
    pub fn alias(&self) -> Option<String> {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .collect();

        // Look for AS keyword followed by IDENT
        let mut found_as = false;
        let mut last_ident: Option<String> = None;
        let mut ident_count = 0;

        for token in &tokens {
            match token.kind() {
                AS_KW => found_as = true,
                IDENT => {
                    ident_count += 1;
                    if found_as {
                        // This is the explicit alias after AS
                        return Some(token.text().to_string());
                    }
                    last_ident = Some(token.text().to_string());
                }
                _ => {}
            }
        }

        // If we have more than one IDENT and no AS keyword, the last IDENT is an implicit alias
        // But we need to be careful: for function calls like smelt.source('raw.users'),
        // we don't want to return 'smelt' or 'source' as aliases
        if !found_as && ident_count > 1 && !self.is_function_call() {
            return last_ident;
        }

        // For function calls with implicit alias (smelt.source('raw.users') t),
        // check if the last token is an IDENT that's not part of the function call
        if self.is_function_call() {
            // Get the function call's text range
            if let Some(func) = self.function_call() {
                let func_range = func.syntax().text_range();
                // Check if last_ident is after the function call
                for token in tokens.iter().rev() {
                    if token.kind() == IDENT {
                        let token_start = token.text_range().start();
                        if token_start >= func_range.end() {
                            return Some(token.text().to_string());
                        }
                        break;
                    }
                }
            }
        }

        // For subqueries with implicit alias (LATERAL (...) alias_name),
        // check if the last token is an IDENT that's after the subquery
        if self.subquery().is_some() {
            if let Some(subquery) = self.subquery() {
                let subquery_range = subquery.0.text_range();
                // Check if last_ident is after the subquery
                for token in tokens.iter().rev() {
                    if token.kind() == IDENT {
                        let token_start = token.text_range().start();
                        if token_start > subquery_range.end() {
                            return Some(token.text().to_string());
                        }
                        break;
                    }
                }
            }
        }

        None
    }

    /// Get the underlying syntax node (for printer)
    #[allow(dead_code)] // Used by printer module
    pub(crate) fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Check if this table reference is LATERAL (allows correlated subquery)
    pub fn is_lateral(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == LATERAL_KW)
    }

    /// Get the subquery if this table reference contains one
    pub fn subquery(&self) -> Option<Subquery> {
        self.0.children().find_map(Subquery::cast)
    }
}

/// WHERE clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WhereClause(SyntaxNode);

impl WhereClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == WHERE_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the expression in this WHERE clause
    pub fn expression(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    /// Get the text range of this WHERE clause
    pub fn text_range(&self) -> TextRange {
        self.0.text_range()
    }

    /// Get the full text of this WHERE clause
    #[allow(dead_code)] // Keep for debugging, but prefer expression()
    pub fn text(&self) -> String {
        self.0.text().to_string()
    }
}

/// Expression node (represents any SQL expression)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Expr(SyntaxNode);

impl Expr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        // Accept any node that looks like an expression
        match node.kind() {
            EXPRESSION | BINARY_EXPR | FUNCTION_CALL | CASE_EXPR | CAST_EXPR | SUBQUERY
            | BETWEEN_EXPR | IN_EXPR | EXISTS_EXPR => Some(Self(node)),
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
                            | SUBQUERY
                            | BETWEEN_EXPR
                            | IN_EXPR
                            | EXISTS_EXPR
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

    /// Get the full text of this expression
    pub fn text(&self) -> String {
        self.0.text().to_string()
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

    /// Check if this is a CASE expression
    pub fn as_case(&self) -> Option<CaseExpr> {
        self.0
            .children()
            .find_map(CaseExpr::cast)
            .or_else(|| CaseExpr::cast(self.0.clone()))
    }

    /// Check if this is a CAST expression
    pub fn as_cast(&self) -> Option<CastExpr> {
        self.0
            .children()
            .find_map(CastExpr::cast)
            .or_else(|| CastExpr::cast(self.0.clone()))
    }

    /// Check if this is a subquery
    pub fn as_subquery(&self) -> Option<Subquery> {
        self.0
            .children()
            .find_map(Subquery::cast)
            .or_else(|| Subquery::cast(self.0.clone()))
    }

    /// Check if this is a BETWEEN expression
    pub fn as_between(&self) -> Option<BetweenExpr> {
        self.0
            .children()
            .find_map(BetweenExpr::cast)
            .or_else(|| BetweenExpr::cast(self.0.clone()))
    }

    /// Check if this is an IN expression
    pub fn as_in(&self) -> Option<InExpr> {
        self.0
            .children()
            .find_map(InExpr::cast)
            .or_else(|| InExpr::cast(self.0.clone()))
    }

    /// Check if this is an EXISTS expression
    pub fn as_exists(&self) -> Option<ExistsExpr> {
        self.0
            .children()
            .find_map(ExistsExpr::cast)
            .or_else(|| ExistsExpr::cast(self.0.clone()))
    }

    /// Check if this is a binary expression
    pub fn as_binary(&self) -> Option<BinaryExpr> {
        self.0
            .children()
            .find_map(BinaryExpr::cast)
            .or_else(|| BinaryExpr::cast(self.0.clone()))
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
        for child in self.0.children_with_tokens() {
            if let Some(token) = child.as_token() {
                match token.kind() {
                    PLUS => return Some("+".to_string()),
                    MINUS => return Some("-".to_string()),
                    STAR | MULTIPLY => return Some("*".to_string()),
                    DIVIDE => return Some("/".to_string()),
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
                    NOT_KW => return Some("NOT".to_string()),
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

        // Simple call: just IDENT
        tokens
            .iter()
            .find(|t| t.kind() == IDENT)
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
}

/// ref('model_name') function call wrapper
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RefCall(FunctionCall);

impl RefCall {
    /// Create a RefCall from a FunctionCall if it's a smelt.ref() call
    pub fn from_function_call(func: FunctionCall) -> Option<Self> {
        let name = func.name()?.to_lowercase();
        let namespace = func.namespace()?; // Require namespace

        // Only accept smelt.ref() - namespace is required
        if name == "ref" && namespace.to_lowercase() == "smelt" {
            Some(Self(func))
        } else {
            None
        }
    }

    /// Get the underlying FunctionCall
    pub fn function_call(&self) -> &FunctionCall {
        &self.0
    }

    /// Get the model name from the ref call (first argument)
    pub fn model_name(&self) -> Option<String> {
        // Look for the first STRING token in the function call arguments
        self.0
             .0
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == STRING)
            .map(|t| {
                let text = t.text();
                // Strip quotes
                text.trim_start_matches('\'')
                    .trim_start_matches('"')
                    .trim_end_matches('\'')
                    .trim_end_matches('"')
                    .to_string()
            })
    }

    /// Get the text range of the entire ref call
    pub fn range(&self) -> TextRange {
        self.0 .0.text_range()
    }

    /// Get the text range of just the model name string (inside quotes)
    pub fn name_range(&self) -> Option<TextRange> {
        self.0
             .0
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == STRING)
            .map(|t| t.text_range())
    }

    /// Get all named parameters from this ref call
    pub fn named_params(&self) -> impl Iterator<Item = NamedParam> + '_ {
        self.0.named_params()
    }
}

/// source('source.table') function call wrapper
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceCall(FunctionCall);

impl SourceCall {
    /// Create a SourceCall from a FunctionCall if it's a smelt.source() call
    pub fn from_function_call(func: FunctionCall) -> Option<Self> {
        let name = func.name()?.to_lowercase();
        let namespace = func.namespace()?;

        if name == "source" && namespace.to_lowercase() == "smelt" {
            Some(Self(func))
        } else {
            None
        }
    }

    /// Get the qualified name from the source call (e.g., "raw.users")
    pub fn qualified_name(&self) -> Option<String> {
        self.0
             .0
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == STRING)
            .map(|t| {
                let text = t.text();
                text.trim_start_matches('\'')
                    .trim_start_matches('"')
                    .trim_end_matches('\'')
                    .trim_end_matches('"')
                    .to_string()
            })
    }

    /// Get the source name (first part before the dot)
    pub fn source_name(&self) -> Option<String> {
        let qualified = self.qualified_name()?;
        qualified.split('.').next().map(|s| s.to_string())
    }

    /// Get the table name (second part after the dot)
    pub fn table_name(&self) -> Option<String> {
        let qualified = self.qualified_name()?;
        qualified.split('.').nth(1).map(|s| s.to_string())
    }

    /// Get the text range of the entire source call
    pub fn range(&self) -> TextRange {
        self.0 .0.text_range()
    }

    /// Get the text range of just the qualified name string
    pub fn name_range(&self) -> Option<TextRange> {
        self.0
             .0
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == STRING)
            .map(|t| t.text_range())
    }
}

/// Helper to convert TextRange offset to line/column position
pub fn offset_to_position(text: &str, offset: usize) -> Position {
    let mut line = 0u32;
    let mut column = 0u32;

    for (i, ch) in text.chars().enumerate() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 0;
        } else {
            column += 1;
        }
    }

    Position { line, column }
}

/// Helper to convert TextRange to LSP Range
pub fn text_range_to_range(text: &str, range: TextRange) -> Range {
    let start = offset_to_position(text, usize::from(range.start()));
    let end = offset_to_position(text, usize::from(range.end()));
    Range { start, end }
}

/// Position (line, column)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

/// Range (start, end positions)
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

    /// Get the case value expression (for simple CASE)
    /// Returns None for searched CASE (CASE WHEN ...)
    pub fn case_value(&self) -> Option<Expr> {
        // The case value is the first EXPRESSION child, before any WHEN_CLAUSE
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

/// Subquery (SELECT statement in parentheses)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Subquery(SyntaxNode);

impl Subquery {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SUBQUERY {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the SELECT statement
    pub fn select_stmt(&self) -> Option<SelectStmt> {
        self.0.children().find_map(SelectStmt::cast)
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

// ===== Phase 11: SQL Clause AST Wrappers =====

/// HAVING clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HavingClause(SyntaxNode);

impl HavingClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == HAVING_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn expression(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }
}

/// ORDER BY clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OrderByClause(SyntaxNode);

impl OrderByClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == ORDER_BY_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn items(&self) -> impl Iterator<Item = OrderByItem> + '_ {
        self.0.children().filter_map(OrderByItem::cast)
    }
}

/// ORDER BY item
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OrderByItem(SyntaxNode);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NullOrdering {
    First,
    Last,
}

impl OrderByItem {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == ORDER_BY_ITEM {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn expression(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    pub fn direction(&self) -> Option<SortDirection> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find_map(|t| match t.kind() {
                ASC_KW => Some(SortDirection::Asc),
                DESC_KW => Some(SortDirection::Desc),
                _ => None,
            })
    }

    pub fn null_ordering(&self) -> Option<NullOrdering> {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .collect();

        for (i, token) in tokens.iter().enumerate() {
            if token.kind() == NULLS_KW {
                // Skip whitespace to find FIRST or LAST
                for next_token in &tokens[i + 1..] {
                    match next_token.kind() {
                        WHITESPACE | COMMENT => continue,
                        FIRST_KW => return Some(NullOrdering::First),
                        LAST_KW => return Some(NullOrdering::Last),
                        _ => break,
                    }
                }
            }
        }
        None
    }
}

/// LIMIT clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LimitClause(SyntaxNode);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitValue {
    Number(String),
    All,
}

impl LimitClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == LIMIT_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn limit_value(&self) -> Option<LimitValue> {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .collect();

        for i in 0..tokens.len() {
            if tokens[i].kind() == LIMIT_KW && i + 1 < tokens.len() {
                return match tokens[i + 1].kind() {
                    NUMBER => Some(LimitValue::Number(tokens[i + 1].text().to_string())),
                    ALL_KW => Some(LimitValue::All),
                    _ => None,
                };
            }
        }
        None
    }

    pub fn offset_value(&self) -> Option<String> {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .collect();

        for i in 0..tokens.len() {
            if tokens[i].kind() == OFFSET_KW
                && i + 1 < tokens.len()
                && tokens[i + 1].kind() == NUMBER
            {
                return Some(tokens[i + 1].text().to_string());
            }
        }
        None
    }

    /// Get the underlying syntax node (for printer)
    #[allow(dead_code)] // Used by printer module
    pub(crate) fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

// ===== Phase 12: Window Function AST Wrappers =====

/// Window specification (OVER clause)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WindowSpec(SyntaxNode);

impl WindowSpec {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == WINDOW_SPEC {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn partition_by(&self) -> Option<PartitionByClause> {
        self.0.children().find_map(PartitionByClause::cast)
    }

    pub fn order_by(&self) -> Option<OrderByClause> {
        self.0.children().find_map(OrderByClause::cast)
    }

    pub fn frame(&self) -> Option<WindowFrame> {
        self.0.children().find_map(WindowFrame::cast)
    }

    /// Get named window reference if this is OVER window_name
    pub fn window_name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }
}

/// PARTITION BY clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PartitionByClause(SyntaxNode);

impl PartitionByClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == PARTITION_BY_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn expressions(&self) -> impl Iterator<Item = Expr> + '_ {
        self.0.children().filter_map(Expr::cast)
    }
}

/// Window frame specification
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WindowFrame(SyntaxNode);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameUnit {
    Rows,
    Range,
    Groups,
}

impl WindowFrame {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == WINDOW_FRAME {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn unit(&self) -> Option<FrameUnit> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find_map(|t| match t.kind() {
                ROWS_KW => Some(FrameUnit::Rows),
                RANGE_KW => Some(FrameUnit::Range),
                GROUPS_KW => Some(FrameUnit::Groups),
                _ => None,
            })
    }

    pub fn bounds(&self) -> Vec<FrameBound> {
        self.0.children().filter_map(FrameBound::cast).collect()
    }
}

/// Frame bound
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FrameBound(SyntaxNode);

impl FrameBound {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == FRAME_BOUND {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn text(&self) -> String {
        self.0.text().to_string()
    }
}

// ===== Phase 13: Common Table Expressions (CTEs) =====

/// WITH clause (CTEs)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WithClause(SyntaxNode);

impl WithClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == WITH_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Check if this is a RECURSIVE CTE
    pub fn is_recursive(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == RECURSIVE_KW)
    }

    /// Get all CTEs in this WITH clause
    pub fn ctes(&self) -> impl Iterator<Item = Cte> + '_ {
        self.0.children().filter_map(Cte::cast)
    }
}

/// Common Table Expression (single CTE in a WITH clause)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Cte(SyntaxNode);

impl Cte {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == CTE {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the CTE name
    pub fn name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// Get the query (SELECT statement)
    pub fn query(&self) -> Option<Subquery> {
        self.0.children().find_map(Subquery::cast)
    }

    /// Get the column names from the optional column list
    pub fn column_names(&self) -> Vec<String> {
        // Extract column names between first LPAREN and RPAREN (before AS)
        let mut in_column_list = false;
        let mut found_as = false;
        let mut columns = Vec::new();

        for child in self.0.children_with_tokens() {
            if let Some(token) = child.as_token() {
                match token.kind() {
                    LPAREN if !found_as => in_column_list = true,
                    RPAREN if in_column_list && !found_as => in_column_list = false,
                    AS_KW => {
                        found_as = true;
                        in_column_list = false;
                    }
                    IDENT if in_column_list => {
                        columns.push(token.text().to_string());
                    }
                    _ => {}
                }
            }
        }

        columns
    }
}
