use super::declarations::strip_ident_quotes;
use super::*;
use crate::syntax_kind::SyntaxNode;
use crate::SyntaxKind;
use rowan::TextRange;

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

    pub fn group_by_clause(&self) -> Option<GroupByClause> {
        self.0.children().find_map(GroupByClause::cast)
    }

    pub fn having_clause(&self) -> Option<HavingClause> {
        self.0.children().find_map(HavingClause::cast)
    }

    pub fn qualify_clause(&self) -> Option<QualifyClause> {
        self.0.children().find_map(QualifyClause::cast)
    }

    pub fn window_clause(&self) -> Option<WindowClause> {
        self.0.children().find_map(WindowClause::cast)
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

    /// Get the underlying syntax node.
    ///
    /// Originally `pub(crate)` and named for the printer module; made
    /// `pub` in Phase 15 so the SELECT-body walker in smelt-db can
    /// descendants-walk a SELECT_STMT body to dispatch nested
    /// `smelt.fn.*` calls.
    pub fn syntax(&self) -> &SyntaxNode {
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

    /// Get the SELECT statement after UNION (if any). The operand may be a
    /// bare `SELECT_STMT` or a parenthesized `SUBQUERY` wrapping one
    /// (`A UNION (B)`); both unwrap to the inner `SelectStmt`.
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
                    if n.kind() == SUBQUERY {
                        if let Some(select) =
                            Subquery::cast(n.clone()).and_then(|sq| sq.select_stmt())
                        {
                            return Some(select);
                        }
                    }
                }
            }
        }
        None
    }

    /// Check if this SELECT has any set operation (UNION, INTERSECT, or EXCEPT)
    pub fn has_set_operation(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| matches!(t.kind(), UNION_KW | INTERSECT_KW | EXCEPT_KW))
    }

    /// Whether this SELECT's set operation (if any) carries a `BY NAME`
    /// modifier (DuckDB): operands are unified by column name rather than
    /// position, and the result widens to the union of column names across
    /// all operands — a different algorithm from smelt's positional set-op
    /// column combination. Returns `false` when there is no set operation.
    pub fn is_set_operation_by_name(&self) -> bool {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| !t.kind().is_trivia())
            .collect();
        let Some(op_idx) = tokens
            .iter()
            .position(|t| matches!(t.kind(), UNION_KW | INTERSECT_KW | EXCEPT_KW))
        else {
            return false;
        };
        let mut idx = op_idx + 1;
        if tokens.get(idx).is_some_and(|t| t.kind() == ALL_KW) {
            idx += 1;
        }
        matches!(
            (tokens.get(idx), tokens.get(idx + 1)),
            (Some(by), Some(name))
                if by.kind() == BY_KW
                    && name.kind() == IDENT
                    && name.text().eq_ignore_ascii_case("NAME")
        )
    }

    /// Get the SELECT statement after any set operation (UNION, INTERSECT,
    /// or EXCEPT). The operand may be a bare `SELECT_STMT` or a
    /// parenthesized `SUBQUERY` wrapping one (`A EXCEPT (B)`); both unwrap
    /// to the inner `SelectStmt`.
    pub fn set_operation_select(&self) -> Option<SelectStmt> {
        let mut found_set_op = false;

        for child in self.0.children_with_tokens() {
            if let Some(token) = child.as_token() {
                if matches!(token.kind(), UNION_KW | INTERSECT_KW | EXCEPT_KW) {
                    found_set_op = true;
                }
            } else if found_set_op {
                if let Some(n) = child.as_node() {
                    if n.kind() == SELECT_STMT {
                        return SelectStmt::cast(n.clone());
                    }
                    if n.kind() == SUBQUERY {
                        if let Some(select) =
                            Subquery::cast(n.clone()).and_then(|sq| sq.select_stmt())
                        {
                            return Some(select);
                        }
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

/// One entry in a SELECT list: either a regular item or a spread (`...expr`).
#[derive(Debug, Clone)]
pub enum SelectEntry {
    Item(SelectItem),
    Spread(ListSpread),
}

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

    /// Iterate over all entries in declaration order, including `LIST_SPREAD` nodes.
    pub fn entries(&self) -> impl Iterator<Item = SelectEntry> + '_ {
        self.0.children().filter_map(|node| {
            if let Some(item) = SelectItem::cast(node.clone()) {
                Some(SelectEntry::Item(item))
            } else {
                ListSpread::cast(node).map(SelectEntry::Spread)
            }
        })
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
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

    /// Raw source text of this item's expression, *before* the paren-unwrapping
    /// [`Expr::cast`] performs.
    ///
    /// `Expr::cast` descends through single-child `EXPRESSION` wrappers so that
    /// semantic callers (type inference, ref resolution) see the operative
    /// expression rather than a parenthesis wrapper. The printer must not follow
    /// it there: the wrapper's `LPAREN`/`RPAREN` are tokens of the outer node, so
    /// printing the unwrapped node drops them and `SELECT (*) x` re-prints as
    /// `SELECT * AS x`, which no dialect accepts.
    pub fn expression_source_text(&self) -> Option<String> {
        let node = self
            .0
            .children()
            .find(|c| Expr::cast(c.clone()).is_some())?;
        // Trivia between the expression and a following `AS`/alias falls inside
        // the node's range; the alias is printed with its own separator.
        Some(trim_source_text(&node))
    }

    /// Get the alias if present (explicit `AS alias` or implicit `expr alias`)
    pub fn alias(&self) -> Option<String> {
        let mut found_as = false;
        let mut found_expr = false;

        for child in self.0.children_with_tokens() {
            match &child {
                rowan::NodeOrToken::Token(token) => {
                    if token.kind() == AS_KW {
                        found_as = true;
                    } else if token.kind() == IDENT {
                        if found_as {
                            // Explicit alias: `expr AS alias`
                            return Some(token.text().to_string());
                        } else if found_expr {
                            // Implicit alias: `expr alias` (IDENT after expression node)
                            return Some(token.text().to_string());
                        }
                    } else if token.kind() == STRING && (found_as || found_expr) {
                        // Double-quoted-identifier alias (`AS "median_delay"`
                        // or implicit `expr "alias"`) — lexed as STRING since
                        // smelt's lexer does not distinguish quote characters
                        // at the token-kind level; strip the quotes.
                        return Some(strip_ident_quotes(token.text()).to_string());
                    }
                }
                rowan::NodeOrToken::Node(_) => {
                    found_expr = true;
                }
            }
        }
        None
    }

    /// Get the alias's raw source text, quotes intact if it was written as
    /// `AS "quoted alias"`. Used by the printer (`Display for SelectItem`),
    /// which must re-quote an alias that needs it (contains whitespace,
    /// matches a keyword, ...) rather than emit `alias()`'s unquoted name —
    /// re-emitting `AS median_delay` for a quoted `AS "median delay"` would
    /// silently mis-print into SQL DuckDB/PostgreSQL reject. Every other
    /// caller wants the unquoted semantic name and should use `alias()`.
    pub fn alias_token_text(&self) -> Option<String> {
        let mut found_as = false;
        let mut found_expr = false;

        for child in self.0.children_with_tokens() {
            match &child {
                rowan::NodeOrToken::Token(token) => {
                    if token.kind() == AS_KW {
                        found_as = true;
                    } else if (token.kind() == IDENT || token.kind() == STRING)
                        && (found_as || found_expr)
                    {
                        return Some(token.text().to_string());
                    }
                }
                rowan::NodeOrToken::Node(_) => {
                    found_expr = true;
                }
            }
        }
        None
    }

    /// Get the text range of the alias token, if present
    pub fn alias_range(&self) -> Option<TextRange> {
        let mut found_as = false;
        let mut found_expr = false;

        for child in self.0.children_with_tokens() {
            match &child {
                rowan::NodeOrToken::Token(token) => {
                    if token.kind() == AS_KW {
                        found_as = true;
                    } else if (token.kind() == IDENT || token.kind() == STRING)
                        && (found_as || found_expr)
                    {
                        return Some(token.text_range());
                    }
                }
                rowan::NodeOrToken::Node(_) => {
                    found_expr = true;
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

    /// Check if this select item is a wildcard (*)
    pub fn is_wildcard(&self) -> bool {
        self.0.children_with_tokens().any(|child| {
            child
                .as_token()
                .is_some_and(|t| t.kind() == STAR || t.kind() == MULTIPLY)
        }) && self.expression().is_none()
    }

    /// Whether this select item is a `smelt.<path>(args).*` struct-spread
    /// call (a [`SyntaxKind::SMELT_PATH_CALL_STAR`] node anywhere under this
    /// item). Like a bare `*`, this item expands to however many columns the
    /// called function's struct return type has — a count only knowable
    /// once the call is resolved and printed, never from this item alone.
    /// Neither [`Self::is_wildcard`] (its STAR token is nested inside the
    /// `SMELT_PATH_CALL_STAR` node, not a direct child of this item) nor
    /// [`Self::expression`] (`SMELT_PATH_CALL_STAR` is not an `Expr`
    /// variant) sees this shape, so a caller deriving a select list's
    /// column count/names (e.g. a projection consumer that must agree with
    /// what the printer will actually emit) needs this dedicated check
    /// alongside `is_wildcard`.
    pub fn is_struct_spread_call(&self) -> bool {
        self.0
            .descendants()
            .any(|n| n.kind() == SyntaxKind::SMELT_PATH_CALL_STAR)
    }

    /// If this select item is a qualified wildcard `<qualifier>.*`,
    /// return the qualifier identifier text. Returns `None` for a bare
    /// `*` (see [`Self::is_wildcard`]) or any non-wildcard item.
    ///
    /// The parser emits a `SELECT_ITEM` whose tokens are `IDENT DOT
    /// STAR` for this shape (no wrapping `EXPRESSION` node). Phase 17
    /// uses this accessor to expand `source.*` inside a
    /// `TableExpr`-returning function body.
    pub fn qualified_wildcard_target(&self) -> Option<String> {
        if !self.is_wildcard() {
            return None;
        }
        // Walk tokens; if we see `IDENT DOT STAR` before the STAR, the
        // IDENT is the qualifier. Bare `*` has no leading IDENT.
        let mut last_ident: Option<String> = None;
        let mut last_was_dot = false;
        for child in self.0.children_with_tokens() {
            if let Some(token) = child.as_token() {
                match token.kind() {
                    IDENT => {
                        last_ident = Some(token.text().to_string());
                        last_was_dot = false;
                    }
                    DOT => {
                        last_was_dot = true;
                    }
                    STAR | MULTIPLY => {
                        if last_was_dot {
                            return last_ident;
                        }
                        return None;
                    }
                    _ => {}
                }
            }
        }
        None
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

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

/// JOIN clause (JOIN type + table + condition)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JoinClause(SyntaxNode);

impl JoinClause {
    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == JOIN_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the JOIN type (INNER, LEFT, RIGHT, FULL, CROSS)
    /// Returns None for bare JOIN (defaults to INNER)
    ///
    /// The ANSI-89 implicit comma-separated form (`FROM a, b`, see
    /// [`Self::is_comma_join`]) is classified as `Cross` (ratified
    /// 2026-07-18, master `docs/plans/20260718-quality-grind.md` D-QG-2):
    /// DuckDB/PostgreSQL both treat a bare comma in FROM as a cross join.
    pub fn join_type(&self) -> Option<JoinType> {
        if self.is_comma_join() {
            return Some(JoinType::Cross);
        }
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

    /// Whether this JOIN carries a `NATURAL` prefix (`NATURAL [INNER |
    /// LEFT [OUTER] | RIGHT [OUTER] | FULL [OUTER]] JOIN`). `NATURAL` is a
    /// contextual keyword (lexed as a plain IDENT) and, when present, is
    /// always the first non-trivia token of the `JOIN_CLAUSE`.
    pub fn is_natural(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| !t.kind().is_trivia())
            .is_some_and(|t| t.kind() == IDENT && t.text().eq_ignore_ascii_case("NATURAL"))
    }

    /// Whether this join is the ANSI-89 implicit comma-separated form
    /// (`FROM a, b`) rather than an explicit `JOIN` keyword. Ratified
    /// 2026-07-18 (master `docs/plans/20260718-quality-grind.md` D-QG-2) as a
    /// cross join; the comma token is always the first non-trivia token of
    /// the `JOIN_CLAUSE`, mirroring how `is_natural` reads its marker.
    pub fn is_comma_join(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| !t.kind().is_trivia())
            .is_some_and(|t| t.kind() == COMMA)
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

    /// Get the unified `smelt.<path>` value-form reference if this table ref
    /// is one (smelt.<path> migration, Phase 1).
    pub fn smelt_path_ref(&self) -> Option<SmeltPathRef> {
        self.0.children().find_map(SmeltPathRef::cast)
    }

    /// Get the unified `smelt.<path>(args)` call if this table ref is one
    /// (smelt.<path> migration, Phase 1).
    pub fn smelt_path_call(&self) -> Option<SmeltPathCall> {
        self.0.children().find_map(SmeltPathCall::cast)
    }

    pub fn identifier(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT || (t.kind() == STRING && t.text().starts_with('"')))
            .map(|t| strip_ident_quotes(t.text()).to_string())
    }

    /// The nested `TABLE_REF` child of a parenthesised table primary
    /// (`FROM (a JOIN b ON …)`, `FROM (a)`) — the parser recurses
    /// `parse_table_ref` through the `LPAREN` branch for this shape (see
    /// `parser/select.rs`'s comment on that branch), so the group's first
    /// member is a direct `TABLE_REF` child rather than a `SUBQUERY`,
    /// `FUNCTION_CALL`, or bare identifier. `None` for every other
    /// `TableRef` shape.
    pub fn nested_table_ref(&self) -> Option<TableRef> {
        self.0.children().find_map(TableRef::cast)
    }

    /// The `JOIN_CLAUSE` children of a parenthesised join group — the
    /// members joined to [`Self::nested_table_ref`] inside the same
    /// parentheses, in source order.
    pub fn nested_joins(&self) -> impl Iterator<Item = JoinClause> + '_ {
        self.0.children().filter_map(JoinClause::cast)
    }

    /// Raw text of the primary table/schema-name path when it begins with a
    /// double-quoted identifier lexed as STRING (`"flights"`,
    /// `"schema"."table"`), quotes intact. DuckDB requires re-quoting names
    /// that need it, so the printer must not go through `identifier()` here
    /// — that method strips quotes (for resolution callers that want the
    /// bare name) and only returns the first segment. Walks contiguous
    /// `(IDENT|STRING) (DOT (IDENT|STRING))*` segments starting at the
    /// first non-trivia, non-`LATERAL` token. Returns `None` when that
    /// first token is not a double-quoted STRING — plain unquoted names
    /// keep using the existing `identifier()` printer path unchanged.
    pub fn quoted_identifier_path_text(&self) -> Option<String> {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| !t.kind().is_trivia() && t.kind() != LATERAL_KW)
            .collect();
        let first = tokens.first()?;
        if !(first.kind() == STRING && first.text().starts_with('"')) {
            return None;
        }
        let mut out = first.text().to_string();
        let mut i = 1;
        while i + 1 < tokens.len() && tokens[i].kind() == DOT {
            let seg = &tokens[i + 1];
            let seg_is_ident_like =
                seg.kind() == IDENT || (seg.kind() == STRING && seg.text().starts_with('"'));
            if !seg_is_ident_like {
                break;
            }
            out.push('.');
            out.push_str(seg.text());
            i += 2;
        }
        Some(out)
    }

    /// Raw dotted path text of the primary table/schema-name path when it
    /// begins with a plain (unquoted) `IDENT` — the sibling of
    /// [`Self::quoted_identifier_path_text`] for the unquoted case, and
    /// unlike [`Self::identifier`] (which returns only the first segment),
    /// walks every contiguous `IDENT (DOT IDENT)*` segment starting at the
    /// first non-trivia, non-`LATERAL` token. Returns `None` when that
    /// first token is not a plain `IDENT` (a quoted name, a function call,
    /// a `smelt.<path>` ref/call, a subquery). For a physical `schema.table`
    /// reference in already-compiled SQL (a `smelt.<path>` ref already
    /// resolved to its physical name) this is the caller's only way to
    /// recover the full path — `identifier()` alone would truncate
    /// `main.dim` to `main`.
    pub fn bare_path_text(&self) -> Option<String> {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| !t.kind().is_trivia() && t.kind() != LATERAL_KW)
            .collect();
        let first = tokens.first()?;
        if first.kind() != IDENT {
            return None;
        }
        let mut out = first.text().to_string();
        let mut i = 1;
        while i + 1 < tokens.len() && tokens[i].kind() == DOT {
            let seg = &tokens[i + 1];
            if seg.kind() != IDENT {
                break;
            }
            out.push('.');
            out.push_str(seg.text());
            i += 2;
        }
        Some(out)
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
                STRING if found_as => {
                    // Double-quoted-identifier alias (`AS "alias"`), lexed as
                    // STRING since smelt's lexer does not distinguish quote
                    // characters at the token-kind level; strip the quotes.
                    return Some(strip_ident_quotes(token.text()).to_string());
                }
                _ => {}
            }
        }

        // If we have more than one IDENT and no AS keyword, the last IDENT is an implicit alias
        // But we need to be careful: for function calls like smelt.sources.raw.users,
        // we don't want to return 'smelt' or 'source' as aliases
        if !found_as && ident_count > 1 && !self.is_function_call() {
            return last_ident;
        }

        // For function calls with implicit alias (smelt.sources.raw.users t),
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

        // For smelt.<path> value-form refs with implicit alias
        // (smelt.models.users u), the SMELT_PATH_REF child is a node whose
        // tokens don't appear in `children_with_tokens()` above. The alias
        // IDENT IS a direct child token. If we have exactly one direct IDENT
        // token (the alias) and a SMELT_PATH_REF child, return that token.
        if let Some(path_ref) = self.smelt_path_ref() {
            let path_range = path_ref.syntax().text_range();
            if let Some(tok) = tokens.iter().rfind(|t| t.kind() == IDENT) {
                if tok.text_range().start() >= path_range.end() {
                    return Some(tok.text().to_string());
                }
            }
        }

        // For smelt.<path> call-form refs with implicit alias
        // (smelt.models.f(x) u), same logic.
        if let Some(path_call) = self.smelt_path_call() {
            let call_range = path_call.syntax().text_range();
            if let Some(tok) = tokens.iter().rfind(|t| t.kind() == IDENT) {
                if tok.text_range().start() >= call_range.end() {
                    return Some(tok.text().to_string());
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

    /// Get the explicit `AS alias`'s raw source text, quotes intact if it
    /// was written as `AS "quoted alias"`. Used by the printer (`Display
    /// for TableRef`), which must re-quote an alias that needs it rather
    /// than emit `alias()`'s unquoted name — see `SelectItem::alias_token_text`
    /// for the rationale. Only covers the explicit-`AS` form: an implicit
    /// quoted alias (`FROM t "alias"`, no `AS`) is not accepted by the
    /// parser (see the comment in `parse_table_ref`'s implicit-alias arm).
    pub fn alias_token_text(&self) -> Option<String> {
        let mut found_as = false;
        for token in self.0.children_with_tokens().filter_map(|e| e.into_token()) {
            match token.kind() {
                AS_KW => found_as = true,
                IDENT | STRING if found_as => return Some(token.text().to_string()),
                _ => {}
            }
        }
        None
    }

    pub fn syntax(&self) -> &SyntaxNode {
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

    /// Get the column names from the optional alias column list, e.g. `AS t(c1, c2, …)`.
    /// Returns `Some(names)` when an `ALIAS_COLUMN_LIST` node is present, `None` otherwise.
    pub fn alias_column_names(&self) -> Option<Vec<String>> {
        let acl = self.0.children().find(|n| n.kind() == ALIAS_COLUMN_LIST)?;
        let names = acl
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
            .collect();
        Some(names)
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

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
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

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
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

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Get the CTE name
    pub fn name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// Get the text range of just the CTE name identifier
    pub fn name_range(&self) -> Option<TextRange> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text_range())
    }

    /// Get the query (SELECT statement)
    pub fn query(&self) -> Option<Subquery> {
        self.0.children().find_map(Subquery::cast)
    }

    /// Get the column names from the optional column list, e.g. `cte(a, b, c) AS (…)`.
    /// Returns an empty `Vec` when no column list is declared.
    /// Reads from the `ALIAS_COLUMN_LIST` child node produced by the parser.
    pub fn column_names(&self) -> Vec<String> {
        match self.0.children().find(|n| n.kind() == ALIAS_COLUMN_LIST) {
            None => Vec::new(),
            Some(acl) => acl
                .children_with_tokens()
                .filter_map(|e| e.into_token())
                .filter(|t| t.kind() == IDENT)
                .map(|t| t.text().to_string())
                .collect(),
        }
    }

    /// Whether this CTE carries an explicit `MATERIALIZED` hint
    /// (`AS MATERIALIZED (…)`). Purely informational (DuckDB/PostgreSQL
    /// materialization directive) — does not change the CTE's schema.
    pub fn is_materialized(&self) -> bool {
        self.materialization_hint().0
    }

    /// Whether this CTE carries an explicit `NOT MATERIALIZED` hint
    /// (`AS NOT MATERIALIZED (…)`).
    pub fn is_not_materialized(&self) -> bool {
        self.materialization_hint().1
    }

    /// `(is_materialized, is_not_materialized)`, scanning the direct child
    /// tokens for `AS [NOT] MATERIALIZED`. `MATERIALIZED` is a contextual
    /// keyword (lexed as a plain IDENT), matched only immediately after
    /// `AS` or `AS NOT`.
    fn materialization_hint(&self) -> (bool, bool) {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| !t.kind().is_trivia())
            .collect();
        let Some(as_idx) = tokens.iter().position(|t| t.kind() == AS_KW) else {
            return (false, false);
        };
        let rest = &tokens[as_idx + 1..];
        let mut it = rest.iter();
        if let Some(first) = it.next() {
            if first.kind() == IDENT && first.text().eq_ignore_ascii_case("MATERIALIZED") {
                return (true, false);
            }
            if first.kind() == NOT_KW {
                if let Some(second) = it.next() {
                    if second.kind() == IDENT && second.text().eq_ignore_ascii_case("MATERIALIZED")
                    {
                        return (false, true);
                    }
                }
            }
        }
        (false, false)
    }
}

// ===== Pipe SQL (Data-World |> pipe query) =====

/// A FROM-first pipe query: `[WITH …] FROM <table_ref> |> STAGE … |> STAGE …`.
///
/// Children (in order):
/// - optional `WITH_CLAUSE`
/// - `FROM_CLAUSE` (the entry source)
/// - zero or more `PIPE_STAGE` nodes
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PipeQuery(SyntaxNode);

impl PipeQuery {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == PIPE_QUERY {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The `WITH_CLAUSE` node, if present.
    pub fn with_clause(&self) -> Option<WithClause> {
        self.0.children().find_map(WithClause::cast)
    }

    /// The `FROM_CLAUSE` entry source.
    pub fn from_clause(&self) -> Option<FromClause> {
        self.0.children().find_map(FromClause::cast)
    }

    /// Iterator over all `PIPE_STAGE` children in declaration order.
    pub fn stages(&self) -> impl Iterator<Item = PipeStage> + '_ {
        self.0.children().filter_map(PipeStage::cast)
    }
}

/// One `|> OPERATOR body` stage inside a `PIPE_QUERY`.
///
/// Children:
/// - a zero-width `PIPE_OP_*` marker identifying the operator
/// - body tokens/nodes for the stage
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PipeStage(SyntaxNode);

impl PipeStage {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == PIPE_STAGE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The `PIPE_OP_*` marker kind identifying which operator this stage is.
    /// Returns `None` only for error-recovery stages with no recognised operator.
    pub fn op_kind(&self) -> Option<SyntaxKind> {
        self.0.children().find_map(|c| {
            let k = c.kind();
            if matches!(
                k,
                PIPE_OP_WHERE
                    | PIPE_OP_SELECT
                    | PIPE_OP_EXTEND
                    | PIPE_OP_SET
                    | PIPE_OP_DROP
                    | PIPE_OP_RENAME
                    | PIPE_OP_AS
                    | PIPE_OP_AGGREGATE
                    | PIPE_OP_ORDER_BY
                    | PIPE_OP_LIMIT
                    | PIPE_OP_JOIN
                    | PIPE_OP_UNION
                    | PIPE_OP_INTERSECT
                    | PIPE_OP_EXCEPT
                    | PIPE_OP_DISTINCT
            ) {
                Some(k)
            } else {
                None
            }
        })
    }

    /// The first non-marker child node (the body of the stage), if any.
    pub fn body(&self) -> Option<SyntaxNode> {
        self.0.children().find(|c| {
            !matches!(
                c.kind(),
                PIPE_OP_WHERE
                    | PIPE_OP_SELECT
                    | PIPE_OP_EXTEND
                    | PIPE_OP_SET
                    | PIPE_OP_DROP
                    | PIPE_OP_RENAME
                    | PIPE_OP_AS
                    | PIPE_OP_AGGREGATE
                    | PIPE_OP_ORDER_BY
                    | PIPE_OP_LIMIT
                    | PIPE_OP_JOIN
                    | PIPE_OP_UNION
                    | PIPE_OP_INTERSECT
                    | PIPE_OP_EXCEPT
                    | PIPE_OP_DISTINCT
            )
        })
    }
}
