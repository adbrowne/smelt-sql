/// Typed AST wrappers over Rowan CST

use crate::syntax_kind::{SyntaxNode, SyntaxToken};
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

    pub fn refs(&self) -> impl Iterator<Item = RefCall> + '_ {
        self.0.descendants().filter_map(RefCall::cast)
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

    pub fn select_list(&self) -> Option<SelectList> {
        self.0.children().find_map(SelectList::cast)
    }

    pub fn from_clause(&self) -> Option<FromClause> {
        self.0.children().find_map(FromClause::cast)
    }

    pub fn where_clause(&self) -> Option<WhereClause> {
        self.0.children().find_map(WhereClause::cast)
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

    pub fn is_template(&self) -> bool {
        self.0.children().any(|n| n.kind() == TEMPLATE_EXPR)
    }

    pub fn template_expr(&self) -> Option<TemplateExpr> {
        self.0.children().find_map(TemplateExpr::cast)
    }

    pub fn identifier(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
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
}

/// Expression node (represents any SQL expression)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Expr(SyntaxNode);

impl Expr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        // Accept any node that looks like an expression
        match node.kind() {
            EXPRESSION | BINARY_EXPR | FUNCTION_CALL => Some(Self(node)),
            _ => {
                // Also try to wrap the node if it contains expression-like children
                if node.children().any(|n| matches!(n.kind(), EXPRESSION | BINARY_EXPR | FUNCTION_CALL)) {
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
        if let Some(func) = self.as_function_call() {
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
        self.0.children().find_map(FunctionCall::cast)
            .or_else(|| {
                // Check if this node itself is a function call
                FunctionCall::cast(self.0.clone())
            })
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
        let tokens: Vec<_> = expr.0
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

    /// Get the function name (e.g., "COUNT", "SUM")
    pub fn name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// Get the text of the full function call
    pub fn text(&self) -> String {
        self.0.text().to_string()
    }
}

/// Template expression {{ ... }}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TemplateExpr(SyntaxNode);

impl TemplateExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == TEMPLATE_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn ref_call(&self) -> Option<RefCall> {
        self.0.children().find_map(RefCall::cast)
    }
}

/// ref('model_name') call
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RefCall(SyntaxNode);

impl RefCall {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == REF_CALL {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the model name from the ref call
    pub fn model_name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
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

    /// Get the text range of the model name (for diagnostics)
    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }

    /// Get the text range of just the model name string (inside quotes)
    pub fn name_range(&self) -> Option<TextRange> {
        self.0
            .children_with_tokens()
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
