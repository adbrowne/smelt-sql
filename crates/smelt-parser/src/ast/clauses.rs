use super::*;
use crate::syntax_kind::SyntaxNode;

// ===== Phase 11: SQL Clause AST Wrappers =====

/// GROUP BY clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GroupByClause(SyntaxNode);

impl GroupByClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == GROUP_BY_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Get the expressions in the GROUP BY clause
    pub fn expressions(&self) -> impl Iterator<Item = Expr> + '_ {
        self.0.children().filter_map(Expr::cast)
    }

    /// True for the DuckDB `GROUP BY ALL` form — group by every non-aggregate
    /// select item. In this form there are no explicit grouping-key
    /// expressions ([`expressions`](Self::expressions) is empty).
    pub fn is_all(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == ALL_KW)
    }
}

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

/// QUALIFY clause (window function filtering)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QualifyClause(SyntaxNode);

impl QualifyClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == QUALIFY_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn expression(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }
}

/// WINDOW clause — holds one or more named window definitions
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WindowClause(SyntaxNode);

impl WindowClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == WINDOW_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn named_windows(&self) -> impl Iterator<Item = NamedWindow> + '_ {
        self.0.children().filter_map(NamedWindow::cast)
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

/// A single named window definition: `name AS (window-body)`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NamedWindow(SyntaxNode);

impl NamedWindow {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == NAMED_WINDOW {
            Some(Self(node))
        } else {
            None
        }
    }

    /// The window name (the identifier before `AS`).
    pub fn name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    pub fn partition_by(&self) -> Option<PartitionByClause> {
        self.0.children().find_map(PartitionByClause::cast)
    }

    pub fn order_by(&self) -> Option<OrderByClause> {
        self.0.children().find_map(OrderByClause::cast)
    }

    pub fn window_frame(&self) -> Option<WindowFrame> {
        self.0.children().find_map(WindowFrame::cast)
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
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

    /// True for the DuckDB `ORDER BY ALL` form — order by every select item,
    /// left to right. In this form there are no explicit `OrderByItem`
    /// children ([`items`](Self::items) is empty); an optional direction /
    /// NULLS ordering may still follow the `ALL` marker.
    pub fn is_all(&self) -> bool {
        // The `ALL` marker is a direct token child of the clause (the ordinary
        // form wraps each key in an ORDER_BY_ITEM, so a bare ALL_KW token here
        // unambiguously signals `ORDER BY ALL`).
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == ALL_KW)
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
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
            .filter(|t| !matches!(t.kind(), WHITESPACE | COMMENT))
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
            .filter(|t| !matches!(t.kind(), WHITESPACE | COMMENT))
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

    /// Get the underlying syntax node.
    pub fn syntax(&self) -> &SyntaxNode {
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

// ===== Phase 35: Struct type references and brace-struct literals =====

/// Structured view over a `STRUCT_TYPE` CST node produced by
/// `Expr<Struct<{field: Type, ..tail}>>` type references (Phase 35).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StructType(SyntaxNode);

impl StructType {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == STRUCT_TYPE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Iterate over the declared `STRUCT_FIELD` children in source order.
    pub fn fields(&self) -> impl Iterator<Item = StructField> + '_ {
        self.0.children().filter_map(StructField::cast)
    }

    /// The trailing row-tail marker, if any.
    pub fn row_tail(&self) -> Option<StructRowTailNode> {
        self.0.children().find_map(StructRowTailNode::cast)
    }
}

/// A single `name: Type` field declaration inside a `STRUCT_TYPE` node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StructField(SyntaxNode);

impl StructField {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == STRUCT_FIELD {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The field's declared name (first IDENT token).
    pub fn name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// The field's declared `TYPE_REF`, if present.
    pub fn type_ref(&self) -> Option<TypeRef> {
        self.0.children().find_map(TypeRef::cast)
    }
}

/// The trailing row-variable marker (`ROW_TAIL` node) inside a `STRUCT_TYPE`.
///
/// A `ROW_TAIL` with an IDENT child is a *named* tail (`..r`).
/// A `ROW_TAIL` with no IDENT child is an *anonymous* tail (`..`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StructRowTailNode(SyntaxNode);

impl StructRowTailNode {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == ROW_TAIL {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The row-variable name for a named tail (e.g. `r` for `..r`).
    /// Returns `None` for an anonymous tail (`..`).
    pub fn var_name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }
}

/// A `{expr AS name, ..spread}` brace-struct literal (Phase 35).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BraceStructLiteral(SyntaxNode);

impl BraceStructLiteral {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == BRACE_STRUCT_LITERAL {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Iterate over `STRUCT_FIELD_ITEM` children.
    pub fn field_items(&self) -> impl Iterator<Item = StructFieldItem> + '_ {
        self.0.children().filter_map(StructFieldItem::cast)
    }

    /// Iterate over `SPREAD_ITEM` children.
    pub fn spread_items(&self) -> impl Iterator<Item = SpreadItem> + '_ {
        self.0.children().filter_map(SpreadItem::cast)
    }
}

/// A single field inside a `BRACE_STRUCT_LITERAL`: either the meta-language
/// `expr AS alias` form, or a DuckDB struct/dict literal `key : value` form
/// (`{'a': 1}` — the canonical form uses a string-literal key; a bare
/// identifier key, `{a: 1}`, parses the same way).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StructFieldItem(SyntaxNode);

impl StructFieldItem {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == STRUCT_FIELD_ITEM {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The value expression: the operand before `AS` in the meta-language
    /// form, or the expression after `:` in the DuckDB `key: value` form.
    /// Both forms have exactly one Expr child in the "value" position; the
    /// `key: value` form has a second (earlier) Expr child for the key,
    /// which `duckdb_key()` returns instead.
    pub fn expression(&self) -> Option<Expr> {
        self.0.children().filter_map(Expr::cast).last()
    }

    /// The key expression of a DuckDB struct/dict literal `key: value` field.
    /// `None` for the meta-language `expr AS alias` form, which has no key —
    /// distinguished structurally by child count (the `key: value` form has
    /// two Expr children, key then value; the `expr AS alias` form has one).
    pub fn duckdb_key(&self) -> Option<Expr> {
        let exprs: Vec<Expr> = self.0.children().filter_map(Expr::cast).collect();
        if exprs.len() >= 2 {
            exprs.into_iter().next()
        } else {
            None
        }
    }

    /// The declared alias (after `AS`) in the meta-language `expr AS alias`
    /// form. `None` for the DuckDB `key: value` form.
    pub fn alias(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }
}

/// A `..name` spread item inside a `BRACE_STRUCT_LITERAL`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SpreadItem(SyntaxNode);

impl SpreadItem {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SPREAD_ITEM {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The identifier being spread (e.g. `event` for `..event`).
    pub fn name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }
}
