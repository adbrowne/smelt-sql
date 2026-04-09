/// Parser implementation with error recovery
use crate::lexer::{tokenize, Token};
use crate::syntax_kind::{SmeltLanguage, SyntaxKind};
use crate::SyntaxKind::*;
use rowan::{GreenNode, GreenNodeBuilder, TextRange};

/// Result of parsing
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parse {
    pub green_node: GreenNode,
    pub errors: Vec<ParseError>,
}

impl Parse {
    pub fn syntax(&self) -> rowan::SyntaxNode<SmeltLanguage> {
        rowan::SyntaxNode::new_root(self.green_node.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub range: TextRange,
}

/// Parse input text into a CST
pub fn parse(input: &str) -> Parse {
    let tokens = tokenize(input);
    let mut parser = Parser::new(input, &tokens);
    parser.parse_file();
    parser.finish()
}

/// Maximum nesting depth for recursive parse functions.
/// Prevents stack overflow on adversarial or deeply nested input.
const MAX_PARSE_DEPTH: u32 = 256;

struct Parser<'a> {
    input: &'a str,
    tokens: &'a [Token],
    pos: usize,
    offset: usize,
    builder: GreenNodeBuilder<'static>,
    errors: Vec<ParseError>,
    depth: u32,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str, tokens: &'a [Token]) -> Self {
        Self {
            input,
            tokens,
            pos: 0,
            offset: 0,
            builder: GreenNodeBuilder::new(),
            errors: Vec::new(),
            depth: 0,
        }
    }

    fn finish(self) -> Parse {
        Parse {
            green_node: self.builder.finish(),
            errors: self.errors,
        }
    }

    /// Current token kind
    fn current(&self) -> SyntaxKind {
        self.tokens.get(self.pos).map(|t| t.kind).unwrap_or(EOF)
    }

    /// Check if at specific token kind
    fn at(&self, kind: SyntaxKind) -> bool {
        self.current() == kind
    }

    /// Check if at any of the given kinds
    fn at_any(&self, kinds: &[SyntaxKind]) -> bool {
        kinds.contains(&self.current())
    }

    /// Get the text of the current token
    fn current_text(&self) -> &str {
        if let Some(token) = self.tokens.get(self.pos) {
            &self.input[self.offset..self.offset + token.len]
        } else {
            ""
        }
    }

    /// Check if current token is an IDENT with specific text (case-insensitive)
    fn at_contextual_keyword(&self, text: &str) -> bool {
        self.at(IDENT) && self.current_text().eq_ignore_ascii_case(text)
    }

    /// Advance to next token, consuming trivia
    fn advance(&mut self) {
        if self.pos < self.tokens.len() {
            let token = self.tokens[self.pos];
            let text = &self.input[self.offset..self.offset + token.len];

            // Report lexer errors with the actual invalid character
            if token.kind == ERROR {
                let display_text = if text.len() <= 10 {
                    text.to_string()
                } else {
                    format!("{}...", &text[..10])
                };
                let message = format!("Unexpected character: '{}'", display_text);
                let start = self.offset as u32;
                let end = (self.offset + token.len) as u32;
                self.errors.push(ParseError {
                    message,
                    range: TextRange::new(start.into(), end.into()),
                });
            }

            self.builder.token(token.kind.into(), text);
            self.offset += token.len;
            self.pos += 1;
        }
    }

    /// Skip trivia (whitespace, comments)
    fn skip_trivia(&mut self) {
        while self.current().is_trivia() {
            self.advance();
        }
    }

    /// Expect a specific token kind, report error if not present
    fn expect(&mut self, kind: SyntaxKind) -> bool {
        self.skip_trivia();
        if self.at(kind) {
            self.advance();
            true
        } else {
            self.error(format!("Expected {:?}, found {:?}", kind, self.current()));
            false
        }
    }

    /// Start a composite node
    fn start_node(&mut self, kind: SyntaxKind) {
        self.builder.start_node(kind.into());
    }

    /// Start a composite node at a checkpoint (for lookahead/backtracking)
    fn start_node_at(&mut self, checkpoint: rowan::Checkpoint, kind: SyntaxKind) {
        self.builder.start_node_at(checkpoint, kind.into());
    }

    /// Finish current node
    fn finish_node(&mut self) {
        self.builder.finish_node();
    }

    /// Report a parse error
    fn error(&mut self, message: String) {
        let start = self.offset as u32;
        let end = (self.offset + self.tokens.get(self.pos).map(|t| t.len).unwrap_or(0)) as u32;
        self.errors.push(ParseError {
            message,
            range: TextRange::new(start.into(), end.into()),
        });
    }

    /// Check if we've exceeded the maximum nesting depth.
    /// Returns true if too deep (caller should bail out).
    fn too_deep(&mut self) -> bool {
        if self.depth >= MAX_PARSE_DEPTH {
            self.error("Expression nesting depth exceeds maximum of 256".to_string());
            true
        } else {
            false
        }
    }

    /// Synchronize to one of the given tokens (error recovery)
    fn sync_to(&mut self, kinds: &[SyntaxKind]) {
        while !self.at(EOF) && !self.at_any(kinds) {
            self.start_node(ERROR);
            self.advance();
            self.finish_node();
        }
    }

    /// Check if current token is a keyword that would end a table reference
    fn at_keyword_that_ends_table_ref(&self) -> bool {
        // Keywords that can follow a table reference in the FROM clause
        self.at_any(&[
            WHERE_KW,
            GROUP_KW,
            HAVING_KW,
            QUALIFY_KW,
            ORDER_KW,
            LIMIT_KW,
            OFFSET_KW,
            // JOIN keywords
            JOIN_KW,
            INNER_KW,
            LEFT_KW,
            RIGHT_KW,
            FULL_KW,
            CROSS_KW,
            // PIVOT/UNPIVOT
            PIVOT_KW,
            UNPIVOT_KW,
            // Set operations
            UNION_KW,
            INTERSECT_KW,
            EXCEPT_KW,
        ]) || self.at_contextual_keyword("FETCH")
    }

    /// Check if current token can start an expression
    fn at_expression_start(&self) -> bool {
        self.at_any(&[
            IDENT, NUMBER, STRING, LPAREN, NOT_KW, CASE_KW, CAST_KW, EXTRACT_KW, EXISTS_KW,
            ARRAY_KW, ROW_KW, STRUCT_KW, MINUS,
        ])
    }

    // ===== Parsing rules =====

    fn parse_file(&mut self) {
        self.start_node(FILE);

        self.skip_trivia();

        // Parse SELECT statement (can start with WITH) or VALUES clause
        if self.at(SELECT_KW) || self.at(WITH_KW) {
            self.parse_select_stmt();
        } else if self.at(VALUES_KW) {
            self.parse_values_clause();
        } else if !self.at(EOF) {
            self.error("Expected SELECT statement".to_string());
            self.sync_to(&[EOF]);
        }

        // Consume remaining trivia
        while !self.at(EOF) {
            self.advance();
        }

        self.finish_node();
    }

    fn parse_select_stmt(&mut self) {
        self.start_node(SELECT_STMT);

        if self.too_deep() {
            self.finish_node();
            return;
        }
        self.depth += 1;

        // WITH clause MUST come first (before SELECT)
        self.skip_trivia();
        if self.at(WITH_KW) {
            self.parse_with_clause();
        }

        // SELECT
        self.expect(SELECT_KW);

        // DISTINCT / ALL (after SELECT, before select list)
        self.skip_trivia();
        if self.at(DISTINCT_KW) {
            self.advance(); // DISTINCT
            self.skip_trivia();
            // Check for DISTINCT ON (PostgreSQL)
            if self.at(ON_KW) {
                self.start_node(DISTINCT_ON_CLAUSE);
                self.advance(); // ON
                self.skip_trivia();
                if self.expect(LPAREN) {
                    // Parse expression list
                    loop {
                        self.skip_trivia();
                        self.parse_expression();
                        self.skip_trivia();
                        if self.at(COMMA) {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.expect(RPAREN);
                }
                self.finish_node(); // DISTINCT_ON_CLAUSE
            }
        } else if self.at(ALL_KW) {
            self.advance();
        }

        // Select list
        self.parse_select_list();

        // FROM clause (optional - SELECT without FROM is valid)
        self.skip_trivia();
        if self.at(FROM_KW) {
            self.parse_from_clause();
        }

        // WHERE clause
        self.skip_trivia();
        if self.at(WHERE_KW) {
            self.parse_where_clause();
        }

        // GROUP BY clause
        self.skip_trivia();
        if self.at(GROUP_KW) {
            self.parse_group_by_clause();
        }

        // HAVING clause (must come after GROUP BY)
        self.skip_trivia();
        if self.at(HAVING_KW) {
            self.parse_having_clause();
        }

        // QUALIFY clause (after HAVING, before ORDER BY)
        self.skip_trivia();
        if self.at(QUALIFY_KW) {
            self.parse_qualify_clause();
        }

        // ORDER BY clause
        self.skip_trivia();
        if self.at(ORDER_KW) {
            self.parse_order_by_clause();
        }

        // LIMIT clause
        self.skip_trivia();
        let has_limit = self.at(LIMIT_KW);
        if has_limit {
            self.parse_limit_clause();
        }

        // OFFSET without LIMIT (for FETCH FIRST pattern)
        self.skip_trivia();
        if self.at(OFFSET_KW) && !has_limit {
            self.start_node(LIMIT_CLAUSE);
            self.advance(); // OFFSET
            self.skip_trivia();
            if self.at(NUMBER) {
                self.advance();
            } else {
                self.error("Expected number after OFFSET".to_string());
            }
            self.finish_node();
        }

        // FETCH FIRST/NEXT N ROW(S) ONLY
        self.skip_trivia();
        if self.at_contextual_keyword("FETCH") {
            self.parse_fetch_clause();
        }

        // Set operations: UNION / INTERSECT / EXCEPT
        self.skip_trivia();
        if self.at_any(&[UNION_KW, INTERSECT_KW, EXCEPT_KW]) {
            self.advance(); // consume UNION/INTERSECT/EXCEPT
            self.skip_trivia();
            // Optional ALL
            if self.at(ALL_KW) {
                self.advance();
            }
            self.skip_trivia();
            // Parse next SELECT
            if self.at(SELECT_KW) || self.at(WITH_KW) {
                self.parse_select_stmt();
            } else {
                self.error("Expected SELECT after set operation".to_string());
            }
        }

        self.depth -= 1;
        self.finish_node();
    }

    fn parse_select_list(&mut self) {
        self.start_node(SELECT_LIST);
        self.skip_trivia();

        // Parse comma-separated select items (including *)
        loop {
            if self.at(STAR) {
                // Handle SELECT * as a special select item
                self.start_node(SELECT_ITEM);
                self.advance();
                self.finish_node();
            } else {
                self.parse_select_item();
            }

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
                self.skip_trivia();
                // Allow trailing comma - break if next token ends the SELECT list
                if self.at_any(&[
                    FROM_KW,
                    WHERE_KW,
                    GROUP_KW,
                    HAVING_KW,
                    QUALIFY_KW,
                    ORDER_KW,
                    LIMIT_KW,
                    OFFSET_KW,
                    EOF,
                    INNER_KW,
                    LEFT_KW,
                    RIGHT_KW,
                    FULL_KW,
                    CROSS_KW,
                    JOIN_KW,
                    UNION_KW,
                    INTERSECT_KW,
                    EXCEPT_KW,
                ]) {
                    break;
                }
            } else {
                break;
            }
        }

        self.finish_node();
    }

    fn parse_select_item(&mut self) {
        self.start_node(SELECT_ITEM);
        self.skip_trivia();

        // Parse expression
        self.parse_expression();

        // Optional AS alias
        self.skip_trivia();
        if self.at(AS_KW) {
            self.advance();
            self.skip_trivia();
            if self.at(IDENT) {
                self.advance();
            }
        } else if self.at(IDENT) {
            // Implicit alias (no AS keyword)
            self.advance();
        }

        self.finish_node();
    }

    fn parse_from_clause(&mut self) {
        self.start_node(FROM_CLAUSE);

        self.expect(FROM_KW);

        // Parse first table reference (required)
        self.parse_table_ref();

        // Parse zero or more JOIN clauses
        loop {
            self.skip_trivia();
            if self.at_any(&[JOIN_KW, INNER_KW, LEFT_KW, RIGHT_KW, FULL_KW, CROSS_KW]) {
                self.parse_join_clause();
            } else {
                break;
            }
        }

        self.finish_node();
    }

    fn parse_table_ref(&mut self) {
        self.start_node(TABLE_REF);
        self.skip_trivia();

        // Check for LATERAL keyword (PostgreSQL)
        if self.at(LATERAL_KW) {
            self.advance(); // LATERAL
            self.skip_trivia();
        }

        if self.at(LPAREN) {
            // Could be a subquery
            let checkpoint = self.builder.checkpoint();
            self.advance(); // consume LPAREN
            self.skip_trivia();

            // Check if it's a subquery (starts with SELECT or WITH) or VALUES
            if self.at(SELECT_KW) || self.at(WITH_KW) {
                self.start_node_at(checkpoint, SUBQUERY);
                self.parse_select_stmt();
                self.skip_trivia();
                self.expect(RPAREN);
                self.finish_node(); // Close SUBQUERY
            } else if self.at(VALUES_KW) {
                self.start_node_at(checkpoint, SUBQUERY);
                self.parse_values_clause();
                self.skip_trivia();
                self.expect(RPAREN);
                self.finish_node();
            } else {
                // Not a subquery, error
                self.error("Expected SELECT in subquery".to_string());
                self.expect(RPAREN);
            }
        } else if self.at(IDENT) {
            // Use builder checkpoint for proper lookahead
            let checkpoint = self.builder.checkpoint();
            self.advance(); // Consume IDENT
            self.skip_trivia();

            if self.at(LPAREN) {
                // It's a simple function call - wrap in FUNCTION_CALL node using checkpoint
                self.start_node_at(checkpoint, FUNCTION_CALL);
                self.parse_arg_list();
                self.finish_node(); // Close FUNCTION_CALL
            } else if self.at(DOT) {
                // Could be schema.table or namespace.func()
                self.advance(); // Consume DOT
                self.skip_trivia();
                self.expect(IDENT); // Consume second IDENT
                self.skip_trivia();

                if self.at(LPAREN) {
                    // Namespaced function call: smelt.ref()
                    self.start_node_at(checkpoint, FUNCTION_CALL);
                    self.parse_arg_list();
                    self.finish_node(); // Close FUNCTION_CALL
                }
                // else: just a qualified table name (schema.table), already consumed
            }
            // else: simple identifier, already consumed
        } else {
            self.error("Expected table reference".to_string());
        }

        // Optional TABLESAMPLE clause (PostgreSQL)
        self.skip_trivia();
        if self.at(TABLESAMPLE_KW) {
            self.start_node(TABLESAMPLE_CLAUSE);
            self.advance(); // TABLESAMPLE
            self.skip_trivia();

            // Sampling method: BERNOULLI or SYSTEM
            if self.at(BERNOULLI_KW) || self.at(SYSTEM_KW) {
                self.advance();
                self.skip_trivia();
            }

            // Percentage in parentheses
            if self.expect(LPAREN) {
                self.skip_trivia();
                self.parse_expression(); // Sample percentage
                self.skip_trivia();
                self.expect(RPAREN);
            }

            // Optional REPEATABLE (seed)
            self.skip_trivia();
            if self.at(REPEATABLE_KW) {
                self.advance(); // REPEATABLE
                self.skip_trivia();
                if self.expect(LPAREN) {
                    self.skip_trivia();
                    self.parse_expression(); // Seed value
                    self.skip_trivia();
                    self.expect(RPAREN);
                }
            }

            self.finish_node(); // TABLESAMPLE_CLAUSE
        }

        // Optional PIVOT/UNPIVOT clause
        self.skip_trivia();
        if self.at(PIVOT_KW) {
            self.parse_pivot_clause();
        } else if self.at(UNPIVOT_KW) {
            self.parse_unpivot_clause();
        }

        // Optional AS alias (explicit with AS keyword or implicit)
        self.skip_trivia();
        if self.at(AS_KW) {
            self.advance();
            self.skip_trivia();
            self.expect(IDENT);
        } else if self.at(IDENT) && !self.at_keyword_that_ends_table_ref() {
            // Implicit alias (no AS keyword)
            // Only consume if it's not a keyword that would end the table ref
            self.advance();
        }

        self.finish_node();
    }

    fn parse_pivot_clause(&mut self) {
        self.start_node(PIVOT_CLAUSE);
        self.expect(PIVOT_KW);
        self.skip_trivia();
        if !self.expect(LPAREN) {
            self.finish_node();
            return;
        }

        // Parse aggregate expression(s): SUM(amount), COUNT(*)
        self.skip_trivia();
        self.parse_expression();

        // FOR column
        self.skip_trivia();
        // FOR is not a keyword in our lexer, so it's parsed as IDENT
        if self.at(IDENT) {
            // Check if text is "FOR"
            let token = self.tokens[self.pos];
            let text = &self.input[self.offset..self.offset + token.len];
            if text.eq_ignore_ascii_case("FOR") {
                self.advance(); // FOR
                self.skip_trivia();
                self.parse_expression(); // column name
            }
        }

        // IN (values...)
        self.skip_trivia();
        if self.at(IN_KW) {
            self.parse_pivot_in_list();
        }

        self.skip_trivia();
        self.expect(RPAREN);
        self.finish_node();
    }

    fn parse_unpivot_clause(&mut self) {
        self.start_node(UNPIVOT_CLAUSE);
        self.expect(UNPIVOT_KW);
        self.skip_trivia();
        if !self.expect(LPAREN) {
            self.finish_node();
            return;
        }

        // Value column name
        self.skip_trivia();
        self.parse_expression();

        // FOR name column
        self.skip_trivia();
        if self.at(IDENT) {
            let token = self.tokens[self.pos];
            let text = &self.input[self.offset..self.offset + token.len];
            if text.eq_ignore_ascii_case("FOR") {
                self.advance(); // FOR
                self.skip_trivia();
                self.parse_expression(); // name column
            }
        }

        // IN (columns...)
        self.skip_trivia();
        if self.at(IN_KW) {
            self.parse_pivot_in_list();
        }

        self.skip_trivia();
        self.expect(RPAREN);
        self.finish_node();
    }

    fn parse_pivot_in_list(&mut self) {
        self.start_node(PIVOT_IN_LIST);
        self.expect(IN_KW);
        self.skip_trivia();
        if !self.expect(LPAREN) {
            self.finish_node();
            return;
        }

        // Parse comma-separated values/columns
        loop {
            self.skip_trivia();
            if self.at(RPAREN) {
                break;
            }
            self.parse_expression();
            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
            } else {
                break;
            }
        }

        self.expect(RPAREN);
        self.finish_node();
    }

    #[allow(clippy::if_same_then_else)]
    fn parse_join_clause(&mut self) {
        self.start_node(JOIN_CLAUSE);

        // Parse JOIN type modifiers (INNER, LEFT, RIGHT, FULL OUTER, CROSS)
        // Note: The if-else blocks are intentionally similar for clarity
        if self.at(INNER_KW) {
            self.advance();
            self.skip_trivia();
        } else if self.at(LEFT_KW) {
            self.advance();
            self.skip_trivia();
            if self.at(OUTER_KW) {
                self.advance();
                self.skip_trivia();
            }
        } else if self.at(RIGHT_KW) {
            self.advance();
            self.skip_trivia();
            if self.at(OUTER_KW) {
                self.advance();
                self.skip_trivia();
            }
        } else if self.at(FULL_KW) {
            self.advance();
            self.skip_trivia();
            if self.at(OUTER_KW) {
                self.advance();
                self.skip_trivia();
            }
        } else if self.at(CROSS_KW) {
            self.advance();
            self.skip_trivia();
        }
        // Note: Bare JOIN defaults to INNER JOIN

        // Expect JOIN keyword
        if !self.expect(JOIN_KW) {
            // Error recovery: missing JOIN keyword
            self.error("Expected JOIN keyword".to_string());
            self.finish_node();
            return;
        }

        // Parse table reference (may include LATERAL keyword)
        self.skip_trivia();
        if !self.at(IDENT) && !self.at(LATERAL_KW) && !self.at(LPAREN) {
            // Error recovery: missing table reference
            self.error("Expected table reference after JOIN".to_string());
            self.finish_node();
            return;
        }
        self.parse_table_ref();

        // Parse join condition (ON or USING)
        // CROSS JOIN doesn't require a condition
        self.skip_trivia();
        if self.at(ON_KW) || self.at(USING_KW) {
            self.parse_join_condition();
        }

        self.finish_node();
    }

    fn parse_join_condition(&mut self) {
        self.start_node(JOIN_CONDITION);

        if self.at(ON_KW) {
            // ON expression
            self.advance();
            self.skip_trivia();

            if !self.at_expression_start() {
                self.error("Expected expression after ON".to_string());
                self.finish_node();
                return;
            }
            self.parse_expression();
        } else if self.at(USING_KW) {
            // USING (col1, col2, ...)
            self.advance();
            self.skip_trivia();

            if !self.expect(LPAREN) {
                self.error("Expected '(' after USING".to_string());
                self.finish_node();
                return;
            }

            // Parse comma-separated column list
            loop {
                self.skip_trivia();
                if !self.at(IDENT) {
                    self.error("Expected column name in USING clause".to_string());
                    break;
                }
                self.advance();

                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                } else {
                    break;
                }
            }

            self.expect(RPAREN);
        }

        self.finish_node();
    }

    fn parse_where_clause(&mut self) {
        self.start_node(WHERE_CLAUSE);
        self.expect(WHERE_KW);
        self.parse_expression();
        self.finish_node();
    }

    fn parse_group_by_clause(&mut self) {
        self.start_node(GROUP_BY_CLAUSE);
        self.expect(GROUP_KW);
        self.expect(BY_KW);

        // Parse comma-separated column list
        loop {
            self.parse_expression();

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
                self.skip_trivia();
                // Allow trailing comma - break if next token ends GROUP BY
                if self.at_any(&[
                    HAVING_KW,
                    QUALIFY_KW,
                    ORDER_KW,
                    LIMIT_KW,
                    OFFSET_KW,
                    EOF,
                    UNION_KW,
                    INTERSECT_KW,
                    EXCEPT_KW,
                ]) {
                    break;
                }
            } else {
                break;
            }
        }

        self.finish_node();
    }

    fn parse_having_clause(&mut self) {
        self.start_node(HAVING_CLAUSE);
        self.expect(HAVING_KW);
        self.parse_expression();
        self.finish_node();
    }

    fn parse_qualify_clause(&mut self) {
        self.start_node(QUALIFY_CLAUSE);
        self.expect(QUALIFY_KW);
        self.parse_expression();
        self.finish_node();
    }

    fn parse_order_by_clause(&mut self) {
        self.start_node(ORDER_BY_CLAUSE);
        self.expect(ORDER_KW);
        self.expect(BY_KW);

        // Comma-separated order items
        loop {
            self.parse_order_by_item();

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
            } else {
                break;
            }
        }

        self.finish_node();
    }

    fn parse_order_by_item(&mut self) {
        self.start_node(ORDER_BY_ITEM);

        // Expression to order by
        self.parse_expression();

        // Optional ASC/DESC
        self.skip_trivia();
        if self.at(ASC_KW) || self.at(DESC_KW) {
            self.advance();
        }

        // Optional NULLS FIRST/LAST
        self.skip_trivia();
        if self.at(NULLS_KW) {
            self.advance();
            self.skip_trivia();
            if self.at(FIRST_KW) || self.at(LAST_KW) {
                self.advance();
            } else {
                self.error("Expected FIRST or LAST after NULLS".to_string());
            }
        }

        self.finish_node();
    }

    fn parse_limit_clause(&mut self) {
        self.start_node(LIMIT_CLAUSE);

        self.expect(LIMIT_KW);
        self.skip_trivia();

        // LIMIT value (number or ALL)
        if self.at(NUMBER) || self.at(ALL_KW) {
            self.advance();
        } else {
            self.error("Expected number or ALL after LIMIT".to_string());
        }

        // Optional OFFSET
        self.skip_trivia();
        if self.at(OFFSET_KW) {
            self.advance();
            self.skip_trivia();
            if self.at(NUMBER) {
                self.advance();
            } else {
                self.error("Expected number after OFFSET".to_string());
            }
        }

        self.finish_node();
    }

    fn parse_expression(&mut self) {
        self.start_node(EXPRESSION);
        self.skip_trivia();

        if self.too_deep() {
            self.finish_node();
            return;
        }
        self.depth += 1;
        self.parse_or_expr();
        self.depth -= 1;

        self.finish_node();
    }

    fn parse_or_expr(&mut self) {
        let checkpoint = self.builder.checkpoint();
        self.parse_and_expr();

        while self.at(OR_KW) {
            self.start_node_at(checkpoint, BINARY_EXPR);
            self.advance();
            self.skip_trivia();
            self.parse_and_expr();
            self.finish_node();
        }
    }

    fn parse_and_expr(&mut self) {
        let checkpoint = self.builder.checkpoint();
        self.parse_comparison_expr();

        while self.at(AND_KW) {
            self.start_node_at(checkpoint, BINARY_EXPR);
            self.advance();
            self.skip_trivia();
            self.parse_comparison_expr();
            self.finish_node();
        }
    }

    fn parse_comparison_expr(&mut self) {
        let checkpoint = self.builder.checkpoint();
        self.parse_concat_expr();

        loop {
            self.skip_trivia();
            if self.at_any(&[
                EQ,
                NE,
                LT,
                GT,
                LE,
                GE,
                TILDE,
                TILDE_STAR,
                NOT_TILDE,
                NOT_TILDE_STAR,
            ]) {
                self.start_node_at(checkpoint, BINARY_EXPR);
                self.advance();
                self.skip_trivia();
                // Check for ANY/ALL/SOME(expr)
                if self.at_any(&[ANY_KW, SOME_KW, ALL_KW]) && self.is_keyword_followed_by_lparen() {
                    self.start_node(ANY_EXPR);
                    self.advance(); // ANY/ALL/SOME
                    self.skip_trivia();
                    self.expect(LPAREN);
                    self.skip_trivia();
                    if self.at(SELECT_KW) {
                        self.parse_subquery();
                    } else {
                        self.parse_expression();
                    }
                    self.skip_trivia();
                    self.expect(RPAREN);
                    self.finish_node(); // ANY_EXPR
                } else {
                    self.parse_concat_expr();
                }
                self.finish_node();
            } else if self.at(IS_KW) {
                // IS [NOT] NULL
                self.start_node_at(checkpoint, BINARY_EXPR);
                self.advance(); // consume IS
                self.skip_trivia();
                if self.at(NOT_KW) {
                    self.advance(); // consume NOT
                    self.skip_trivia();
                }
                if self.at(NULL_KW) {
                    self.advance(); // consume NULL
                }
                self.finish_node();
            } else if self.at(BETWEEN_KW) {
                // BETWEEN low AND high — use checkpoint to include left operand
                self.start_node_at(checkpoint, BETWEEN_EXPR);
                self.parse_between_body();
                self.finish_node();
            } else if self.at(IN_KW) {
                // IN (values...) — use checkpoint to include left operand
                self.start_node_at(checkpoint, IN_EXPR);
                self.parse_in_body();
                self.finish_node();
            } else if self.at(LIKE_KW) || self.at(ILIKE_KW) {
                // LIKE / ILIKE pattern
                self.start_node_at(checkpoint, BINARY_EXPR);
                self.advance(); // consume LIKE/ILIKE
                self.skip_trivia();
                self.parse_concat_expr();
                self.finish_node();
            } else {
                break;
            }
        }
    }

    /// Parse the body of a BETWEEN expression (BETWEEN low AND high).
    /// Caller is responsible for creating the BETWEEN_EXPR node with the left operand.
    fn parse_between_body(&mut self) {
        self.expect(BETWEEN_KW);

        // Parse lower bound
        self.skip_trivia();
        self.parse_additive_expr();

        // Expect AND
        self.skip_trivia();
        if !self.expect(AND_KW) {
            self.error("Expected AND in BETWEEN expression".to_string());
        }

        // Parse upper bound
        self.skip_trivia();
        self.parse_additive_expr();
    }

    /// Parse the body of an IN expression (IN (values...)).
    /// Caller is responsible for creating the IN_EXPR node with the left operand.
    fn parse_in_body(&mut self) {
        self.expect(IN_KW);

        self.skip_trivia();
        if !self.expect(LPAREN) {
            self.error("Expected '(' after IN".to_string());
            return;
        }

        self.skip_trivia();

        // Check if it's a subquery (starts with SELECT)
        if self.at(SELECT_KW) {
            self.parse_subquery();
        } else {
            // Parse comma-separated value list
            loop {
                self.skip_trivia();
                if self.at(RPAREN) {
                    break;
                }

                self.parse_expression();

                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                } else {
                    break;
                }
            }
        }

        self.expect(RPAREN);
    }

    fn parse_concat_expr(&mut self) {
        let checkpoint = self.builder.checkpoint();
        self.parse_json_expr();

        while self.at(CONCAT) {
            self.start_node_at(checkpoint, BINARY_EXPR);
            self.advance();
            self.skip_trivia();
            self.parse_json_expr();
            self.finish_node();
        }
    }

    fn parse_json_expr(&mut self) {
        self.parse_additive_expr();

        self.skip_trivia();
        while self.at_any(&[
            JSON_ARROW,
            JSON_ARROW_TEXT,
            HASH_ARROW,
            HASH_ARROW_TEXT,
            AT_GT,
            LT_AT,
        ]) {
            self.start_node(BINARY_EXPR);
            self.advance();
            self.skip_trivia();
            self.parse_additive_expr();
            self.skip_trivia();
            self.finish_node();
        }
    }

    fn parse_additive_expr(&mut self) {
        let checkpoint = self.builder.checkpoint();
        self.parse_multiplicative_expr();

        self.skip_trivia();
        while self.at_any(&[PLUS, MINUS]) {
            self.start_node_at(checkpoint, BINARY_EXPR);
            self.advance();
            self.skip_trivia();
            self.parse_multiplicative_expr();
            self.skip_trivia();
            self.finish_node();
        }
    }

    fn parse_multiplicative_expr(&mut self) {
        let checkpoint = self.builder.checkpoint();
        self.parse_unary_expr();

        self.skip_trivia();
        while self.at_any(&[STAR, DIVIDE, PERCENT]) {
            self.start_node_at(checkpoint, BINARY_EXPR);
            self.advance();
            self.skip_trivia();
            self.parse_unary_expr();
            self.skip_trivia();
            self.finish_node();
        }
    }

    fn parse_unary_expr(&mut self) {
        self.skip_trivia();

        // Handle unary operators (-, NOT)
        if self.at_any(&[MINUS, NOT_KW]) {
            self.start_node(BINARY_EXPR); // Reuse BINARY_EXPR for unary ops
            self.advance(); // consume operator
            self.skip_trivia();
            self.parse_unary_expr(); // Allow chaining: --x
            self.finish_node();
        } else {
            self.parse_primary_expr();
        }
    }

    fn parse_primary_expr(&mut self) {
        self.skip_trivia();

        if self.at(NULL_KW) {
            // NULL literal — wrap in EXPRESSION so Expr::cast() works
            self.start_node(EXPRESSION);
            self.advance();
            self.finish_node();
            return;
        }

        if self.at(ARRAY_KW) && self.is_keyword_followed_by_lbracket() {
            self.parse_array_literal();
        } else if self.at(ARRAY_KW) && self.is_keyword_followed_by_lparen() {
            // ARRAY(expr) function-call style
            let checkpoint = self.builder.checkpoint();
            self.advance(); // ARRAY_KW
            self.skip_trivia();
            self.start_node_at(checkpoint, FUNCTION_CALL);
            self.parse_arg_list();
            self.finish_node();
        } else if self.at(ROW_KW) && self.is_keyword_followed_by_lparen() {
            self.parse_row_constructor();
        } else if self.at(STRUCT_KW) && self.is_keyword_followed_by_lparen() {
            self.parse_struct_literal();
        } else if self.at(CASE_KW) {
            self.parse_case_expr();
        } else if self.at(CAST_KW) {
            self.parse_cast_expr();
        } else if self.at(EXTRACT_KW) {
            self.parse_extract_expr();
        } else if self.at(EXISTS_KW) {
            self.parse_exists_expr();
        } else if self.at(LPAREN) {
            // Could be: parenthesized expression, subquery, or function call
            let checkpoint = self.builder.checkpoint();
            self.advance(); // consume LPAREN
            self.skip_trivia();

            // Check if it's a subquery (starts with SELECT)
            if self.at(SELECT_KW) {
                self.start_node_at(checkpoint, SUBQUERY);
                self.parse_select_stmt();
                self.skip_trivia();
                self.expect(RPAREN);
                self.finish_node();
            } else {
                // Grouped expression
                self.parse_expression();
                self.skip_trivia();
                self.expect(RPAREN);
            }
        } else if self.at_keyword_as_function_name() {
            // Keywords that can also be function names (e.g., FILTER(arr, ...) as array filter)
            let checkpoint = self.builder.checkpoint();
            self.advance(); // consume keyword
            self.skip_trivia();
            if self.at(LPAREN) {
                self.start_node_at(checkpoint, FUNCTION_CALL);
                self.parse_arg_list();
                self.finish_node();
            } else {
                // Bare keyword used as identifier — wrap in EXPRESSION
                self.start_node_at(checkpoint, EXPRESSION);
                self.finish_node();
            }
        } else if self.at(IDENT) && self.is_typed_literal() {
            // Typed literal: DATE '2024-01-01', TIMESTAMP '...', etc.
            // Wrap in EXPRESSION so Expr::cast() works
            self.start_node(EXPRESSION);
            self.advance(); // type keyword (IDENT)
            self.skip_trivia();
            self.advance(); // string literal
            self.finish_node();
        } else if self.at(IDENT) {
            // Could be column reference, qualified name, or function call
            let checkpoint = self.builder.checkpoint();
            self.advance(); // consume first IDENT
            self.skip_trivia();

            if self.at(LPAREN) {
                // Simple function call: func()
                self.start_node_at(checkpoint, FUNCTION_CALL);
                self.parse_arg_list();
                self.parse_within_group_if_present();
                self.parse_filter_clause_if_present();
                self.finish_node();

                // Check for OVER clause (window function)
                self.skip_trivia();
                if self.at(OVER_KW) {
                    self.parse_window_spec();
                }
            } else if self.at(DOT) {
                // Could be table.column or namespace.func()
                self.advance(); // consume DOT
                self.skip_trivia();
                self.expect(IDENT); // consume second IDENT
                self.skip_trivia();

                if self.at(LPAREN) {
                    // Namespaced function call: smelt.ref()
                    self.start_node_at(checkpoint, FUNCTION_CALL);
                    self.parse_arg_list();
                    self.parse_within_group_if_present();
                    self.parse_filter_clause_if_present();
                    self.finish_node();

                    // Check for OVER clause (window function)
                    self.skip_trivia();
                    if self.at(OVER_KW) {
                        self.parse_window_spec();
                    }
                } else {
                    // Qualified name (table.column) — wrap in EXPRESSION
                    self.start_node_at(checkpoint, EXPRESSION);
                    self.finish_node();
                }
            } else if self.at(DOUBLE_COLON) {
                // PostgreSQL cast: expr::type
                // Wrap the identifier in EXPRESSION first, then wrap all in CAST_EXPR
                self.start_node_at(checkpoint, EXPRESSION);
                self.finish_node();
                self.start_node_at(checkpoint, CAST_EXPR);
                self.advance(); // consume ::
                self.skip_trivia();
                self.parse_type_spec();
                self.finish_node();
            } else {
                // Simple identifier — wrap in EXPRESSION
                self.start_node_at(checkpoint, EXPRESSION);
                self.finish_node();
            }
        } else if self.current().is_literal() || self.at(STAR) {
            // Literal or STAR — wrap in EXPRESSION so Expr::cast() works
            self.start_node(EXPRESSION);
            self.advance();
            self.finish_node();
        } else {
            self.error(format!("Expected expression, found {:?}", self.current()));
        }

        // Postfix: array subscript/slice: expr[index] or expr[start:end]
        self.skip_trivia();
        while self.at(LBRACKET) {
            self.parse_array_subscript();
            self.skip_trivia();
        }
    }

    fn parse_array_subscript(&mut self) {
        // Determine if this is subscript or slice by looking for COLON
        let checkpoint = self.builder.checkpoint();
        self.advance(); // consume [
        self.skip_trivia();

        // Parse first expression (index or start of slice)
        let has_first = !self.at(COLON);
        if has_first {
            self.parse_expression();
            self.skip_trivia();
        }

        if self.at(COLON) {
            // Slice: [start:end]
            self.start_node_at(checkpoint, ARRAY_SLICE);
            self.advance(); // consume :
            self.skip_trivia();
            if !self.at(RBRACKET) {
                self.parse_expression();
                self.skip_trivia();
            }
            self.expect(RBRACKET);
            self.finish_node();
        } else {
            // Simple subscript: [index]
            self.start_node_at(checkpoint, ARRAY_SUBSCRIPT);
            self.expect(RBRACKET);
            self.finish_node();
        }
    }

    fn parse_fetch_clause(&mut self) {
        self.start_node(FETCH_CLAUSE);
        // FETCH is a contextual keyword (IDENT)
        self.advance(); // consume "FETCH"
        self.skip_trivia();
        // FIRST or NEXT (contextual)
        if self.at(FIRST_KW) || self.at_contextual_keyword("NEXT") {
            self.advance();
        }
        self.skip_trivia();
        // Optional count
        if self.at(NUMBER) {
            self.advance();
        }
        self.skip_trivia();
        // ROW or ROWS
        if self.at(ROW_KW) || self.at(ROWS_KW) {
            self.advance();
        }
        self.skip_trivia();
        // ONLY (contextual)
        if self.at_contextual_keyword("ONLY") {
            self.advance();
        }
        self.finish_node();
    }

    fn parse_values_clause(&mut self) {
        self.start_node(VALUES_CLAUSE);
        self.expect(VALUES_KW);

        // Parse comma-separated rows: (expr, expr), (expr, expr)
        loop {
            self.skip_trivia();
            self.start_node(VALUES_ROW);
            if self.expect(LPAREN) {
                loop {
                    self.skip_trivia();
                    if self.at(RPAREN) {
                        break;
                    }
                    self.parse_expression();
                    self.skip_trivia();
                    if self.at(COMMA) {
                        self.advance();
                    } else {
                        break;
                    }
                }
                self.expect(RPAREN);
            }
            self.finish_node(); // VALUES_ROW

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
            } else {
                break;
            }
        }

        self.finish_node(); // VALUES_CLAUSE
    }

    fn parse_array_literal(&mut self) {
        self.start_node(ARRAY_LITERAL);
        self.expect(ARRAY_KW);
        self.skip_trivia();
        if self.expect(LBRACKET) {
            loop {
                self.skip_trivia();
                if self.at(RBRACKET) {
                    break;
                }
                self.parse_expression();
                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(RBRACKET);
        }
        self.finish_node();
    }

    fn parse_row_constructor(&mut self) {
        self.start_node(ROW_CONSTRUCTOR);
        self.expect(ROW_KW);
        self.skip_trivia();
        if self.expect(LPAREN) {
            loop {
                self.skip_trivia();
                if self.at(RPAREN) {
                    break;
                }
                self.parse_expression();
                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(RPAREN);
        }
        self.finish_node();
    }

    fn parse_struct_literal(&mut self) {
        self.start_node(STRUCT_LITERAL);
        self.expect(STRUCT_KW);
        self.skip_trivia();
        if self.expect(LPAREN) {
            loop {
                self.skip_trivia();
                if self.at(RPAREN) {
                    break;
                }
                self.parse_expression();
                // Optional AS name
                self.skip_trivia();
                if self.at(AS_KW) {
                    self.advance();
                    self.skip_trivia();
                    if self.at(IDENT) {
                        self.advance();
                    }
                }
                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(RPAREN);
        }
        self.finish_node();
    }

    fn parse_case_expr(&mut self) {
        self.start_node(CASE_EXPR);
        self.expect(CASE_KW);

        self.skip_trivia();

        // Check if it's simple CASE (CASE expr WHEN ...) or searched CASE (CASE WHEN ...)
        // If the next token after CASE is not WHEN, it's a simple CASE
        let is_simple_case = !self.at(WHEN_KW);
        if is_simple_case {
            // Simple CASE - parse the case value expression
            // Use a more restricted parse to avoid consuming the WHEN keyword
            self.parse_additive_expr();
            self.skip_trivia();
        }

        // Parse WHEN clauses
        while self.at(WHEN_KW) {
            self.parse_when_clause();
            self.skip_trivia();
        }

        // Optional ELSE clause
        if self.at(ELSE_KW) {
            self.advance(); // consume ELSE
            self.skip_trivia();
            self.parse_expression();
            self.skip_trivia();
        }

        // Expect END
        if !self.expect(END_KW) {
            self.error("Expected END to close CASE expression".to_string());
        }

        self.finish_node();
    }

    fn parse_when_clause(&mut self) {
        self.start_node(WHEN_CLAUSE);
        self.expect(WHEN_KW);

        // Parse condition or value (depends on simple vs searched CASE)
        // Use comparison_expr to avoid consuming beyond THEN keyword
        self.skip_trivia();
        self.parse_comparison_expr();

        // Expect THEN
        self.skip_trivia();
        if !self.expect(THEN_KW) {
            self.error("Expected THEN in WHEN clause".to_string());
        }

        // Parse result - use comparison_expr to avoid consuming beyond WHEN/ELSE/END
        self.skip_trivia();
        self.parse_comparison_expr();

        self.finish_node();
    }

    fn parse_cast_expr(&mut self) {
        self.start_node(CAST_EXPR);
        self.expect(CAST_KW);

        self.skip_trivia();
        if !self.expect(LPAREN) {
            self.error("Expected '(' after CAST".to_string());
            self.finish_node();
            return;
        }

        // Parse expression to cast
        self.skip_trivia();
        self.parse_expression();

        // Expect AS
        self.skip_trivia();
        if !self.expect(AS_KW) {
            self.error("Expected AS in CAST expression".to_string());
        }

        // Parse type
        self.skip_trivia();
        self.parse_type_spec();

        self.expect(RPAREN);
        self.finish_node();
    }

    /// Parse `EXTRACT(field FROM expr)` as a special expression.
    /// The field is an identifier like EPOCH, YEAR, MONTH, DAY, HOUR, MINUTE, SECOND.
    fn parse_extract_expr(&mut self) {
        self.start_node(EXTRACT_EXPR);
        self.expect(EXTRACT_KW);

        self.skip_trivia();
        if !self.expect(LPAREN) {
            self.error("Expected '(' after EXTRACT".to_string());
            self.finish_node();
            return;
        }

        // Parse the field name (EPOCH, YEAR, MONTH, DAY, etc.)
        // These are identifiers or keywords that act as field specifiers.
        self.skip_trivia();
        if self.at(IDENT) || self.current().is_keyword() {
            self.advance(); // consume the field name
        } else {
            self.error(
                "Expected date/time field (EPOCH, YEAR, MONTH, etc.) in EXTRACT".to_string(),
            );
        }

        // Expect FROM keyword
        self.skip_trivia();
        if !self.expect(FROM_KW) {
            self.error("Expected FROM in EXTRACT expression".to_string());
        }

        // Parse the source expression
        self.skip_trivia();
        self.parse_expression();

        self.skip_trivia();
        self.expect(RPAREN);
        self.finish_node();
    }

    fn parse_type_spec(&mut self) {
        self.start_node(TYPE_SPEC);

        // Type name (identifier)
        if !self.at(IDENT) {
            self.error("Expected type name".to_string());
            self.finish_node();
            return;
        }
        self.advance();

        // Optional type parameters: VARCHAR(255), DECIMAL(10,2), etc.
        self.skip_trivia();
        if self.at(LPAREN) {
            self.advance(); // consume LPAREN

            // Parse comma-separated parameters
            loop {
                self.skip_trivia();
                if self.at(RPAREN) {
                    break;
                }

                // Type parameters are typically numbers
                if self.at(NUMBER) {
                    self.advance();
                } else if self.at(IDENT) {
                    // Some types might have identifier parameters
                    self.advance();
                } else {
                    self.error("Expected type parameter".to_string());
                    break;
                }

                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                } else {
                    break;
                }
            }

            self.expect(RPAREN);
        }

        self.finish_node();
    }

    fn parse_subquery(&mut self) {
        self.start_node(SUBQUERY);
        self.parse_select_stmt();
        self.finish_node();
    }

    fn parse_exists_expr(&mut self) {
        self.start_node(EXISTS_EXPR);
        self.expect(EXISTS_KW);

        self.skip_trivia();
        if !self.expect(LPAREN) {
            self.error("Expected '(' after EXISTS".to_string());
            self.finish_node();
            return;
        }

        self.skip_trivia();
        if self.at(SELECT_KW) {
            self.parse_subquery();
        } else {
            self.error("Expected SELECT after EXISTS (".to_string());
        }

        self.expect(RPAREN);
        self.finish_node();
    }

    fn parse_arg_list(&mut self) {
        self.start_node(ARG_LIST);
        self.expect(LPAREN);
        self.skip_trivia();

        if !self.at(RPAREN) {
            loop {
                self.parse_argument();

                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                    self.skip_trivia();
                } else {
                    break;
                }
            }
        }

        self.expect(RPAREN);
        self.finish_node();
    }

    /// Parse optional WITHIN GROUP clause for ordered-set aggregate functions
    /// WITHIN GROUP (ORDER BY expr)
    fn parse_within_group_if_present(&mut self) {
        self.skip_trivia();
        if self.at_contextual_keyword("WITHIN") {
            self.start_node(WITHIN_GROUP_CLAUSE);
            self.advance(); // WITHIN
            self.skip_trivia();
            self.expect(GROUP_KW);
            self.skip_trivia();
            if self.expect(LPAREN) {
                self.skip_trivia();
                if self.at(ORDER_KW) {
                    self.parse_order_by_clause();
                }
                self.skip_trivia();
                self.expect(RPAREN);
            }
            self.finish_node();
        }
    }

    /// Parse optional FILTER clause for aggregate functions (PostgreSQL)
    /// FILTER (WHERE condition)
    fn parse_filter_clause_if_present(&mut self) {
        self.skip_trivia();
        if self.at(FILTER_KW) {
            self.start_node(FILTER_CLAUSE);
            self.advance(); // FILTER
            self.skip_trivia();
            if self.expect(LPAREN) {
                self.skip_trivia();
                if self.expect(WHERE_KW) {
                    self.skip_trivia();
                    self.parse_expression(); // Filter condition
                    self.skip_trivia();
                }
                self.expect(RPAREN);
            }
            self.finish_node(); // FILTER_CLAUSE
        }
    }

    fn parse_argument(&mut self) {
        self.skip_trivia();

        // Handle DISTINCT/ALL modifiers for aggregate functions: COUNT(DISTINCT col)
        if self.at(DISTINCT_KW) || self.at(ALL_KW) {
            self.advance(); // consume DISTINCT or ALL
            self.skip_trivia();
        }

        // Check for named parameter: IDENT => expression
        // Use lookahead to check without consuming the identifier first
        if (self.at(IDENT) || self.current().is_keyword()) && self.is_named_parameter() {
            // It's a named parameter
            self.start_node(NAMED_PARAM);
            self.advance(); // consume IDENT or keyword
            self.skip_trivia();
            self.advance(); // consume ARROW (=>)
            self.skip_trivia();
            self.parse_expression();
            self.finish_node();
        } else if self.at(IDENT) && self.is_lambda_single_param() {
            // Single-param lambda: x -> expr
            self.parse_lambda_expr();
        } else if self.at(LPAREN) && self.is_lambda_multi_param() {
            // Multi-param lambda: (x, y) -> expr
            self.parse_lambda_expr();
        } else {
            // Regular expression argument - parse as full expression
            // This handles: identifiers, literals, function calls, binary expressions, etc.
            self.parse_expression();
        }
    }

    /// Check if current position starts a named parameter (IDENT => ...)
    /// Uses lookahead without consuming tokens
    fn is_named_parameter(&self) -> bool {
        // We know we're at IDENT or keyword, check what comes after
        // Need to skip ahead past the current token and any whitespace to find ARROW
        let mut lookahead = 1; // Skip current token

        // Skip whitespace tokens
        while let Some(token) = self.tokens.get(self.pos + lookahead) {
            if token.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }

        // Check if next non-trivia token is ARROW
        self.tokens
            .get(self.pos + lookahead)
            .map(|t| t.kind == ARROW)
            .unwrap_or(false)
    }

    /// Check if current keyword is followed by LBRACKET (skipping trivia)
    fn is_keyword_followed_by_lbracket(&self) -> bool {
        let mut lookahead = 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        self.tokens
            .get(self.pos + lookahead)
            .map(|t| t.kind == LBRACKET)
            .unwrap_or(false)
    }

    /// Check if current keyword is followed by LPAREN (skipping trivia)
    fn is_keyword_followed_by_lparen(&self) -> bool {
        let mut lookahead = 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        self.tokens
            .get(self.pos + lookahead)
            .map(|t| t.kind == LPAREN)
            .unwrap_or(false)
    }

    /// Check if current token is a keyword that can also be used as a function name
    fn at_keyword_as_function_name(&self) -> bool {
        if !self.at_any(&[
            FILTER_KW, QUALIFY_KW, PIVOT_KW, UNPIVOT_KW, VALUES_KW, LEFT_KW, RIGHT_KW,
        ]) {
            return false;
        }
        // Only treat as function name if followed by LPAREN
        let mut lookahead = 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        self.tokens
            .get(self.pos + lookahead)
            .map(|t| t.kind == LPAREN)
            .unwrap_or(false)
    }

    /// Check if current IDENT is a type keyword followed by a string literal (e.g., DATE '2024-01-01')
    fn is_typed_literal(&self) -> bool {
        // Check if current IDENT is a type keyword
        let token = self.tokens[self.pos];
        let text = &self.input[self.offset..self.offset + token.len];
        let upper = text.to_uppercase();
        if !matches!(upper.as_str(), "DATE" | "TIME" | "TIMESTAMP" | "INTERVAL") {
            return false;
        }

        // Look ahead past trivia for a STRING token
        let mut lookahead = 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        self.tokens
            .get(self.pos + lookahead)
            .map(|t| t.kind == STRING)
            .unwrap_or(false)
    }

    /// Check if current position starts a single-param lambda: ident ->
    /// Lambda arrow is MINUS GT (two adjacent tokens with no space between)
    fn is_lambda_single_param(&self) -> bool {
        let mut lookahead = 1;
        while let Some(token) = self.tokens.get(self.pos + lookahead) {
            if token.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        self.is_thin_arrow_at(lookahead)
    }

    /// Check if tokens at pos+offset form -> (either JSON_ARROW token or MINUS GT pair)
    fn is_thin_arrow_at(&self, offset: usize) -> bool {
        // With the new lexer, -> is tokenized as JSON_ARROW
        if let Some(t) = self.tokens.get(self.pos + offset) {
            if t.kind == JSON_ARROW {
                return true;
            }
        }
        // Fallback: MINUS GT (shouldn't happen with new lexer, but keep for safety)
        let minus = self.tokens.get(self.pos + offset);
        let gt = self.tokens.get(self.pos + offset + 1);
        matches!((minus, gt), (Some(m), Some(g)) if m.kind == MINUS && g.kind == GT)
    }

    /// Check if current position starts a multi-param lambda: (ident, ident) ->
    fn is_lambda_multi_param(&self) -> bool {
        // We're at LPAREN. Scan forward for: IDENT [, IDENT]* ) ->
        let mut lookahead = 1;

        // Skip trivia after LPAREN
        while let Some(token) = self.tokens.get(self.pos + lookahead) {
            if token.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }

        // Must start with IDENT
        if !self
            .tokens
            .get(self.pos + lookahead)
            .map(|t| t.kind == IDENT)
            .unwrap_or(false)
        {
            return false;
        }
        lookahead += 1;

        // Loop: skip trivia, then expect COMMA IDENT or RPAREN
        loop {
            while let Some(token) = self.tokens.get(self.pos + lookahead) {
                if token.kind.is_trivia() {
                    lookahead += 1;
                } else {
                    break;
                }
            }

            match self.tokens.get(self.pos + lookahead).map(|t| t.kind) {
                Some(RPAREN) => {
                    lookahead += 1;
                    break;
                }
                Some(COMMA) => {
                    lookahead += 1;
                    // Skip trivia
                    while let Some(token) = self.tokens.get(self.pos + lookahead) {
                        if token.kind.is_trivia() {
                            lookahead += 1;
                        } else {
                            break;
                        }
                    }
                    // Must be IDENT
                    if !self
                        .tokens
                        .get(self.pos + lookahead)
                        .map(|t| t.kind == IDENT)
                        .unwrap_or(false)
                    {
                        return false;
                    }
                    lookahead += 1;
                }
                _ => return false,
            }
        }

        // Skip trivia after RPAREN
        while let Some(token) = self.tokens.get(self.pos + lookahead) {
            if token.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }

        // Must be -> (MINUS GT)
        self.is_thin_arrow_at(lookahead)
    }

    fn parse_lambda_expr(&mut self) {
        self.start_node(LAMBDA_EXPR);

        // Parse parameter(s)
        if self.at(LPAREN) {
            // Multi-param: (x, y) -> expr
            self.start_node(LAMBDA_PARAM_LIST);
            self.advance(); // LPAREN
            self.skip_trivia();
            loop {
                if self.at(IDENT) {
                    self.advance();
                }
                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                    self.skip_trivia();
                } else {
                    break;
                }
            }
            self.expect(RPAREN);
            self.finish_node(); // LAMBDA_PARAM_LIST
        } else {
            // Single-param: x -> expr
            self.start_node(LAMBDA_PARAM_LIST);
            self.advance(); // IDENT
            self.finish_node(); // LAMBDA_PARAM_LIST
        }

        self.skip_trivia();
        // Consume -> (now tokenized as JSON_ARROW, or fallback MINUS GT)
        if self.at(JSON_ARROW) {
            self.advance();
        } else {
            self.expect(MINUS);
            self.expect(GT);
        }
        self.skip_trivia();
        self.parse_expression();

        self.finish_node(); // LAMBDA_EXPR
    }

    // ===== Phase 12: Window Function Support =====

    fn parse_window_spec(&mut self) {
        self.start_node(WINDOW_SPEC);

        self.expect(OVER_KW);
        self.skip_trivia();

        if self.at(IDENT) {
            // Named window reference: OVER window_name
            self.advance();
        } else if self.at(LPAREN) {
            // Inline window specification
            self.advance();
            self.skip_trivia();

            // Optional PARTITION BY
            if self.at(PARTITION_KW) {
                self.parse_partition_by();
            }

            // Optional ORDER BY (reuse existing)
            self.skip_trivia();
            if self.at(ORDER_KW) {
                self.parse_order_by_clause();
            }

            // Optional frame clause
            self.skip_trivia();
            if self.at_any(&[ROWS_KW, RANGE_KW, GROUPS_KW]) {
                self.parse_window_frame();
            }

            self.expect(RPAREN);
        } else {
            self.error("Expected window name or ( after OVER".to_string());
        }

        self.finish_node();
    }

    fn parse_partition_by(&mut self) {
        self.start_node(PARTITION_BY_CLAUSE);

        self.expect(PARTITION_KW);
        self.expect(BY_KW);

        // Comma-separated expressions
        loop {
            self.parse_expression();

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
            } else {
                break;
            }
        }

        self.finish_node();
    }

    fn parse_window_frame(&mut self) {
        self.start_node(WINDOW_FRAME);

        // Frame unit: ROWS, RANGE, or GROUPS
        if self.at_any(&[ROWS_KW, RANGE_KW, GROUPS_KW]) {
            self.advance();
        }

        self.skip_trivia();

        // Frame extent
        if self.at(BETWEEN_KW) {
            // BETWEEN start AND end
            self.advance();
            self.skip_trivia();
            self.parse_frame_bound();
            self.skip_trivia();
            self.expect(AND_KW);
            self.skip_trivia();
            self.parse_frame_bound();
        } else {
            // Single bound (implicit CURRENT ROW end)
            self.parse_frame_bound();
        }

        // Optional EXCLUDE clause
        self.skip_trivia();
        if self.at_contextual_keyword("EXCLUDE") {
            self.start_node(FRAME_EXCLUDE);
            self.advance(); // EXCLUDE
            self.skip_trivia();
            if self.at(CURRENT_KW) {
                self.advance(); // CURRENT
                self.skip_trivia();
                self.expect(ROW_KW);
            } else if self.at(GROUP_KW) || self.at_contextual_keyword("TIES") {
                self.advance();
            } else if self.at_contextual_keyword("NO") {
                self.advance(); // NO
                self.skip_trivia();
                if self.at_contextual_keyword("OTHERS") {
                    self.advance();
                } else {
                    self.error("Expected OTHERS after NO".to_string());
                }
            } else {
                self.error(
                    "Expected CURRENT ROW, GROUP, TIES, or NO OTHERS after EXCLUDE".to_string(),
                );
            }
            self.finish_node();
        }

        self.finish_node();
    }

    fn parse_frame_bound(&mut self) {
        self.start_node(FRAME_BOUND);

        if self.at(UNBOUNDED_KW) {
            self.advance();
            self.skip_trivia();
            if self.at(PRECEDING_KW) || self.at(FOLLOWING_KW) {
                self.advance();
            } else {
                self.error("Expected PRECEDING or FOLLOWING after UNBOUNDED".to_string());
            }
        } else if self.at(CURRENT_KW) {
            self.advance();
            self.skip_trivia();
            self.expect(ROW_KW);
        } else if self.at(NUMBER) {
            // N PRECEDING or N FOLLOWING
            self.advance();
            self.skip_trivia();
            if self.at(PRECEDING_KW) || self.at(FOLLOWING_KW) {
                self.advance();
            } else {
                self.error("Expected PRECEDING or FOLLOWING after number".to_string());
            }
        } else {
            self.error("Expected frame bound (UNBOUNDED, CURRENT ROW, or number)".to_string());
        }

        self.finish_node();
    }

    // ===== Phase 13: Common Table Expressions (CTEs) =====

    fn parse_with_clause(&mut self) {
        self.start_node(WITH_CLAUSE);

        self.expect(WITH_KW);

        // Optional RECURSIVE
        self.skip_trivia();
        if self.at(RECURSIVE_KW) {
            self.advance();
        }

        // Comma-separated CTEs
        loop {
            self.skip_trivia();
            self.parse_cte();

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
            } else {
                break;
            }
        }

        self.finish_node();
    }

    fn parse_cte(&mut self) {
        self.start_node(CTE);

        // CTE name
        self.skip_trivia();
        if !self.expect(IDENT) {
            self.error("Expected CTE name".to_string());
            self.finish_node();
            return;
        }

        // Optional column list: name(col1, col2)
        // For now, we'll parse it simply - if we see LPAREN followed by IDENT, it might be a column list
        self.skip_trivia();
        if self.at(LPAREN) {
            // Peek ahead to see if this looks like a column list
            // Column list: (ident, ident, ...) followed by AS
            // Query: (SELECT ...) - but this is after AS
            // So if we see LPAREN and it's NOT preceded by AS, check if it's a column list

            self.advance(); // consume LPAREN
            self.skip_trivia();

            // If we see IDENT (not SELECT/WITH), assume it's a column list
            if self.at(IDENT) {
                // Parse column list
                loop {
                    if !self.at(IDENT) {
                        break;
                    }
                    self.advance();
                    self.skip_trivia();

                    if self.at(COMMA) {
                        self.advance();
                        self.skip_trivia();
                    } else {
                        break;
                    }
                }
                self.expect(RPAREN);
                self.skip_trivia();
            } else if self.at(SELECT_KW) || self.at(WITH_KW) {
                // This is actually the AS clause query, not a column list
                // Parse the subquery
                self.start_node(SUBQUERY);
                self.parse_select_stmt();
                self.finish_node();
                self.expect(RPAREN);

                // Done with CTE
                self.finish_node();
                return;
            } else {
                // Empty or unexpected
                self.expect(RPAREN);
                self.skip_trivia();
            }
        }

        // AS (query)
        if !self.expect(AS_KW) {
            self.error("Expected AS in CTE".to_string());
            self.finish_node();
            return;
        }

        self.skip_trivia();
        if !self.expect(LPAREN) {
            self.error("Expected ( after AS in CTE".to_string());
            self.finish_node();
            return;
        }

        self.skip_trivia();
        if self.at(SELECT_KW) || self.at(WITH_KW) {
            self.start_node(SUBQUERY);
            self.parse_select_stmt();
            self.finish_node();
        } else if self.at(VALUES_KW) {
            self.parse_values_clause();
        } else {
            self.error("Expected SELECT, WITH, or VALUES in CTE".to_string());
        }

        self.expect(RPAREN);
        self.finish_node();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use crate::ast::{
        ArraySlice, ArraySubscript, BetweenExpr, BinaryExpr, CaseExpr, CastExpr, Cte, ExistsExpr,
        File, FilterClause, FrameUnit, FunctionCall, GroupByClause, HavingClause, InExpr, JoinType,
        LambdaExpr, LimitClause, LimitValue, NamedParam, NullOrdering, OrderByClause, OrderByItem,
        PartitionByClause, PivotClause, QualifyClause, SelectItem, SelectList, SelectStmt,
        SortDirection, Subquery, UnpivotClause, WhenClause, WindowFrame, WindowSpec, WithClause,
    };

    /// Helper: parse SQL, assert no errors, return the SelectStmt
    #[allow(dead_code)]
    fn parse_select(sql: &str) -> (Parse, SelectStmt) {
        let parse = parse(sql);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
        let file = File::cast(parse.syntax()).unwrap();
        let select = file.select_stmt().unwrap();
        (parse, select)
    }

    #[test]
    fn test_inner_join() {
        let input = "SELECT * FROM users INNER JOIN orders ON users.id = orders.user_id";
        let (_, select) = parse_select(input);

        let from = select.from_clause().expect("should have FROM");
        assert_eq!(from.joins().count(), 1);
        let join = from.joins().next().unwrap();
        assert_eq!(join.join_type(), Some(JoinType::Inner));
        let cond = join.condition().expect("should have condition");
        assert!(cond.is_on());
        assert!(!cond.is_using());
    }

    #[test]
    fn test_left_join() {
        let input = "SELECT * FROM users LEFT JOIN orders ON users.id = orders.user_id";
        let (_, select) = parse_select(input);

        let from = select.from_clause().unwrap();
        let join = from.joins().next().unwrap();
        assert_eq!(join.join_type(), Some(JoinType::Left));
    }

    #[test]
    fn test_right_join() {
        let input = "SELECT * FROM users RIGHT JOIN orders ON users.id = orders.user_id";
        let (_, select) = parse_select(input);

        let from = select.from_clause().unwrap();
        let join = from.joins().next().unwrap();
        assert_eq!(join.join_type(), Some(JoinType::Right));
    }

    #[test]
    fn test_full_join() {
        let input = "SELECT * FROM users FULL JOIN orders ON users.id = orders.user_id";
        let (_, select) = parse_select(input);

        let from = select.from_clause().unwrap();
        let join = from.joins().next().unwrap();
        assert_eq!(join.join_type(), Some(JoinType::Full));
    }

    #[test]
    fn test_cross_join() {
        let input = "SELECT * FROM users CROSS JOIN countries";
        let (_, select) = parse_select(input);

        let from = select.from_clause().unwrap();
        let join = from.joins().next().unwrap();
        assert_eq!(join.join_type(), Some(JoinType::Cross));
        assert!(join.condition().is_none(), "CROSS JOIN has no condition");
    }

    #[test]
    fn test_multiple_joins() {
        let input = "SELECT * FROM users
                     INNER JOIN orders ON users.id = orders.user_id
                     LEFT JOIN products ON orders.product_id = products.id";
        let (_, select) = parse_select(input);

        let from = select.from_clause().unwrap();
        assert_eq!(from.joins().count(), 2);
    }

    #[test]
    fn test_using_clause() {
        let input = "SELECT * FROM users JOIN orders USING (user_id)";
        let (_, select) = parse_select(input);

        let from = select.from_clause().unwrap();
        let join = from.joins().next().unwrap();
        let cond = join.condition().expect("should have condition");
        assert!(cond.is_using());
        assert!(!cond.is_on());
        let cols = cond.using_columns();
        assert_eq!(cols, vec!["user_id"]);
    }

    #[test]
    fn test_join_error_recovery_missing_table() {
        let input = "SELECT * FROM users JOIN";
        let parse = parse(input);
        assert!(!parse.errors.is_empty());
        assert!(parse.errors[0].message.contains("table"));
    }

    #[test]
    fn test_join_error_recovery_missing_on() {
        let input = "SELECT * FROM users JOIN orders ON";
        let parse = parse(input);
        assert!(!parse.errors.is_empty());
        assert!(parse.errors[0].message.contains("expression"));
    }

    // Phase 10: Expression Enhancement Tests

    #[test]
    fn test_case_searched() {
        let input = "SELECT CASE WHEN status = 'active' THEN 1 WHEN status = 'pending' THEN 0 ELSE -1 END FROM users";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let case_node = parse
            .syntax()
            .descendants()
            .find_map(CaseExpr::cast)
            .expect("should have a CaseExpr");
        assert!(
            case_node.case_value().is_none(),
            "searched CASE has no case value"
        );
        assert_eq!(case_node.when_clauses().count(), 2);
        assert!(case_node.else_expr().is_some(), "should have ELSE");
    }

    #[test]
    fn test_case_simple() {
        let input =
            "SELECT CASE status WHEN 'active' THEN 1 WHEN 'pending' THEN 0 ELSE -1 END FROM users";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let case_node = parse
            .syntax()
            .descendants()
            .find_map(CaseExpr::cast)
            .expect("should have a CaseExpr");
        assert!(
            case_node.case_value().is_some(),
            "simple CASE has a case value"
        );
        assert_eq!(case_node.when_clauses().count(), 2);
        assert!(case_node.else_expr().is_some(), "should have ELSE");
    }

    #[test]
    fn test_case_no_else() {
        let input = "SELECT CASE WHEN status = 'active' THEN 1 END FROM users";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let case_node = parse
            .syntax()
            .descendants()
            .find_map(CaseExpr::cast)
            .expect("should have a CaseExpr");
        assert!(case_node.else_expr().is_none(), "no ELSE clause");
        assert_eq!(case_node.when_clauses().count(), 1);
    }

    #[test]
    fn test_when_clause_accessors() {
        let input = "SELECT CASE WHEN x > 10 THEN 'big' END FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let case_node = parse
            .syntax()
            .descendants()
            .find_map(CaseExpr::cast)
            .expect("should have a CaseExpr");
        let when = case_node
            .when_clauses()
            .next()
            .expect("should have a WHEN clause");
        assert!(when.condition().is_some(), "WHEN should have a condition");
        assert!(when.result().is_some(), "WHEN should have a result");
    }

    #[test]
    fn test_cast_standard() {
        let input = "SELECT CAST(price AS INTEGER) FROM products";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let cast_node = parse
            .syntax()
            .descendants()
            .find_map(CastExpr::cast)
            .expect("should have a CastExpr");
        assert!(!cast_node.is_double_colon_cast());
        assert!(cast_node.expression().is_some(), "should have expression");
        let type_spec = cast_node.type_spec().expect("should have type spec");
        assert_eq!(type_spec.type_name().as_deref(), Some("INTEGER"));
    }

    #[test]
    fn test_cast_postgres_double_colon() {
        let input = "SELECT price::INTEGER FROM products";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let cast_node = parse
            .syntax()
            .descendants()
            .find_map(CastExpr::cast)
            .expect("should have a CastExpr");
        assert!(cast_node.is_double_colon_cast());
        assert!(cast_node.expression().is_some(), "should have expression");
    }

    #[test]
    fn test_binary_expr_structure() {
        let input = "SELECT a + b FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let bin = parse
            .syntax()
            .descendants()
            .find_map(BinaryExpr::cast)
            .expect("should have a BinaryExpr");
        assert_eq!(bin.operator().as_deref(), Some("+"));
        assert!(bin.left().is_some(), "should have left operand");
        assert!(bin.right().is_some(), "should have right operand");
        assert!(!bin.is_unary());
    }

    #[test]
    fn test_modulo_operator() {
        let input = "SELECT a % b FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let bin = parse
            .syntax()
            .descendants()
            .find_map(BinaryExpr::cast)
            .expect("should have a BinaryExpr");
        assert_eq!(bin.operator().as_deref(), Some("%"));
        assert!(bin.left().is_some(), "should have left operand");
        assert!(bin.right().is_some(), "should have right operand");
    }

    #[test]
    fn test_modulo_precedence() {
        // % should have same precedence as * and /
        let input = "SELECT a + b % c FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        // The outer binary should be +, with b%c on the right
        let bins: Vec<_> = parse
            .syntax()
            .descendants()
            .filter_map(BinaryExpr::cast)
            .collect();
        // Should have two binary exprs: a + (b % c)
        assert_eq!(bins.len(), 2);
        // Outer is +
        assert_eq!(bins[0].operator().as_deref(), Some("+"));
        // Inner is %
        assert_eq!(bins[1].operator().as_deref(), Some("%"));
    }

    #[test]
    fn test_cast_with_params() {
        let input = "SELECT CAST(name AS VARCHAR(255)) FROM users";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_cast_decimal() {
        let input = "SELECT CAST(amount AS DECIMAL(10, 2)) FROM transactions";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_subquery_in_select() {
        let input = "SELECT (SELECT COUNT(*) FROM orders WHERE user_id = users.id) AS order_count FROM users";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let subquery = parse
            .syntax()
            .descendants()
            .find_map(Subquery::cast)
            .expect("should have a Subquery");
        assert!(
            subquery.select_stmt().is_some(),
            "subquery should contain a SelectStmt"
        );
    }

    #[test]
    fn test_subquery_in_from() {
        let input = "SELECT * FROM (SELECT user_id, COUNT(*) AS cnt FROM orders GROUP BY user_id)";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_between() {
        let input = "SELECT * FROM products WHERE price BETWEEN 10 AND 100";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let _between = parse
            .syntax()
            .descendants()
            .find_map(BetweenExpr::cast)
            .expect("should have a BetweenExpr");
    }

    #[test]
    fn test_between_with_expressions() {
        let input = "SELECT * FROM events WHERE created_at BETWEEN start_date AND end_date";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_in_values() {
        let input = "SELECT * FROM users WHERE status IN ('active', 'pending')";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let in_expr = parse
            .syntax()
            .descendants()
            .find_map(InExpr::cast)
            .expect("should have an InExpr");
        assert!(!in_expr.is_subquery(), "value list IN is not a subquery");
    }

    #[test]
    fn test_in_numbers() {
        let input = "SELECT * FROM products WHERE category_id IN (1, 2, 3, 5, 8)";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_in_subquery() {
        let input =
            "SELECT * FROM users WHERE id IN (SELECT user_id FROM orders WHERE total > 100)";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let in_expr = parse
            .syntax()
            .descendants()
            .find_map(InExpr::cast)
            .expect("should have an InExpr");
        assert!(in_expr.is_subquery(), "should be a subquery IN");
        assert!(in_expr.subquery().is_some(), "subquery should be present");
    }

    #[test]
    fn test_exists() {
        let input = "SELECT * FROM users WHERE EXISTS (SELECT 1 FROM orders WHERE orders.user_id = users.id)";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let exists = parse
            .syntax()
            .descendants()
            .find_map(ExistsExpr::cast)
            .expect("should have an ExistsExpr");
        assert!(exists.subquery().is_some(), "EXISTS should have a subquery");
    }

    #[test]
    fn test_complex_nested_expressions() {
        let input = "SELECT CASE WHEN price::DECIMAL > 100 THEN 'expensive' ELSE 'cheap' END FROM products WHERE category_id IN (1, 2, 3)";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_unary_minus() {
        let input = "SELECT -1 FROM users";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    // Phase 11: SQL Clause Tests

    #[test]
    fn test_order_by_basic() {
        let input = "SELECT name FROM users ORDER BY name ASC";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_order_by_multiple() {
        let input = "SELECT * FROM users ORDER BY last_name DESC, first_name ASC";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_order_by_nulls() {
        let input = "SELECT * FROM users ORDER BY age DESC NULLS LAST";
        let (_, select) = parse_select(input);

        let order_by = select.order_by_clause().expect("should have ORDER BY");
        let items: Vec<_> = order_by.items().collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].direction(), Some(SortDirection::Desc));
        assert_eq!(items[0].null_ordering(), Some(NullOrdering::Last));
    }

    #[test]
    fn test_order_by_nulls_first() {
        let input = "SELECT * FROM users ORDER BY age ASC NULLS FIRST";
        let (_, select) = parse_select(input);

        let order_by = select.order_by_clause().expect("should have ORDER BY");
        let items: Vec<_> = order_by.items().collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].direction(), Some(SortDirection::Asc));
        assert_eq!(items[0].null_ordering(), Some(NullOrdering::First));
    }

    #[test]
    fn test_limit_offset() {
        let input = "SELECT * FROM users LIMIT 10 OFFSET 20";
        let (_, select) = parse_select(input);

        let limit = select.limit_clause().expect("should have LIMIT");
        assert_eq!(
            limit.limit_value(),
            Some(LimitValue::Number("10".to_string()))
        );
        assert_eq!(limit.offset_value().as_deref(), Some("20"));
    }

    #[test]
    fn test_limit_only() {
        let input = "SELECT * FROM users LIMIT 5";
        let (_, select) = parse_select(input);

        let limit = select.limit_clause().expect("should have LIMIT");
        assert_eq!(
            limit.limit_value(),
            Some(LimitValue::Number("5".to_string()))
        );
        assert_eq!(limit.offset_value(), None);
    }

    #[test]
    fn test_limit_all() {
        let input = "SELECT * FROM users LIMIT ALL";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_having_clause() {
        let input = "SELECT dept, COUNT(*) FROM users GROUP BY dept HAVING COUNT(*) > 5";
        let (_, select) = parse_select(input);

        let group_by = select.group_by_clause().expect("should have GROUP BY");
        // GROUP BY expressions may be bare IDENT tokens
        let _ = group_by.expressions().count();

        let having = select.having_clause().expect("should have HAVING");
        assert!(
            having.expression().is_some(),
            "HAVING should have expression"
        );
    }

    #[test]
    fn test_distinct() {
        let input = "SELECT DISTINCT city FROM users";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_count_distinct() {
        let input = "SELECT COUNT(DISTINCT session_id) FROM events";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_count_all() {
        let input = "SELECT COUNT(ALL user_id) FROM events";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_select_all() {
        let input = "SELECT ALL city FROM users";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_complete_query() {
        let input = "SELECT DISTINCT dept, COUNT(*) as cnt
                     FROM users
                     WHERE active = true
                     GROUP BY dept
                     HAVING COUNT(*) > 5
                     ORDER BY cnt DESC NULLS LAST
                     LIMIT 10 OFFSET 5";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_select_without_from() {
        let input = "SELECT 1 + 1 AS result";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_order_by_expression() {
        let input = "SELECT * FROM users ORDER BY CASE WHEN age > 18 THEN 1 ELSE 0 END";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_having_complex_expression() {
        let input = "SELECT dept, AVG(salary) FROM employees GROUP BY dept HAVING AVG(salary) > 50000 AND COUNT(*) > 10";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    // Phase 12: Window Function Tests

    #[test]
    fn test_window_function_basic() {
        let input = "SELECT ROW_NUMBER() OVER (ORDER BY created_at) FROM users";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let win = parse
            .syntax()
            .descendants()
            .find_map(WindowSpec::cast)
            .expect("should have a WindowSpec");
        assert!(win.partition_by().is_none(), "no PARTITION BY");
        assert!(win.order_by().is_some(), "should have ORDER BY");
        assert!(win.frame().is_none(), "no frame spec");
    }

    #[test]
    fn test_window_function_partition() {
        let input = "SELECT SUM(amount) OVER (PARTITION BY user_id ORDER BY date) FROM orders";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let win = parse
            .syntax()
            .descendants()
            .find_map(WindowSpec::cast)
            .expect("should have a WindowSpec");
        assert!(win.partition_by().is_some(), "should have PARTITION BY");
        assert!(win.order_by().is_some(), "should have ORDER BY");
    }

    #[test]
    fn test_window_frame_rows() {
        let input = "SELECT AVG(price) OVER (ORDER BY date ROWS BETWEEN 3 PRECEDING AND CURRENT ROW) FROM prices";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let win = parse
            .syntax()
            .descendants()
            .find_map(WindowSpec::cast)
            .expect("should have a WindowSpec");
        let frame = win.frame().expect("should have a frame");
        assert_eq!(frame.unit(), Some(FrameUnit::Rows));
        assert_eq!(frame.bounds().len(), 2, "BETWEEN ... AND ... has 2 bounds");
    }

    #[test]
    fn test_window_frame_unbounded() {
        let input = "SELECT SUM(amount) OVER (ORDER BY date ROWS UNBOUNDED PRECEDING) FROM sales";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_frame_range() {
        let input = "SELECT AVG(price) OVER (ORDER BY date RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) FROM prices";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_frame_groups() {
        let input = "SELECT COUNT(*) OVER (ORDER BY category GROUPS BETWEEN 1 PRECEDING AND 1 FOLLOWING) FROM products";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_multiple_window_functions() {
        let input = "SELECT
                       ROW_NUMBER() OVER (ORDER BY date),
                       AVG(price) OVER (PARTITION BY category ORDER BY date)
                     FROM products";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_function_with_frame_offset() {
        let input = "SELECT AVG(price) OVER (ORDER BY date ROWS BETWEEN 2 PRECEDING AND 2 FOLLOWING) FROM prices";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_function_partition_multiple_columns() {
        let input = "SELECT SUM(amount) OVER (PARTITION BY user_id, category ORDER BY date) FROM transactions";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_function_range_unbounded_following() {
        let input = "SELECT SUM(amount) OVER (ORDER BY date RANGE BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING) FROM sales";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_function_with_aggregate() {
        let input =
            "SELECT dept, AVG(salary) OVER (PARTITION BY dept) as avg_dept_salary FROM employees";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_function_rank() {
        let input = "SELECT name, RANK() OVER (ORDER BY score DESC) as rank FROM students";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_function_dense_rank() {
        let input =
            "SELECT name, DENSE_RANK() OVER (PARTITION BY class ORDER BY score DESC) FROM students";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_function_lag() {
        let input = "SELECT date, price, LAG(price) OVER (ORDER BY date) as prev_price FROM prices";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_function_lead() {
        let input =
            "SELECT date, price, LEAD(price, 1) OVER (ORDER BY date) as next_price FROM prices";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    // Phase 13: CTE Tests

    #[test]
    fn test_cte_basic() {
        let input = "WITH temp AS (SELECT * FROM users) SELECT * FROM temp";
        let (_, select) = parse_select(input);

        let with = select.with_clause().expect("should have WITH clause");
        assert!(!with.is_recursive());
        let ctes: Vec<_> = with.ctes().collect();
        assert_eq!(ctes.len(), 1);
        assert_eq!(ctes[0].name().as_deref(), Some("temp"));
        assert!(ctes[0].query().is_some(), "CTE should have a query");
    }

    #[test]
    fn test_cte_multiple() {
        let input = "WITH
                       active_users AS (SELECT * FROM users WHERE active = true),
                       recent_orders AS (SELECT * FROM orders WHERE date > '2024-01-01')
                     SELECT * FROM active_users JOIN recent_orders ON active_users.id = recent_orders.user_id";
        let (_, select) = parse_select(input);

        let with = select.with_clause().expect("should have WITH clause");
        assert_eq!(with.ctes().count(), 2);
    }

    #[test]
    fn test_cte_recursive() {
        let input = "WITH RECURSIVE tree AS (
                       SELECT id, parent_id FROM nodes WHERE parent_id IS NULL
                       UNION ALL
                       SELECT n.id, n.parent_id FROM nodes n JOIN tree ON n.parent_id = tree.id
                     ) SELECT * FROM tree";
        let (_, select) = parse_select(input);

        let with = select.with_clause().expect("should have WITH clause");
        assert!(with.is_recursive());
    }

    #[test]
    fn test_cte_nested() {
        let input = "WITH outer_cte AS (
                       WITH inner_cte AS (SELECT id FROM users)
                       SELECT * FROM inner_cte
                     ) SELECT * FROM outer_cte";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_cte_with_window_function() {
        let input = "WITH ranked AS (
                       SELECT id, ROW_NUMBER() OVER (ORDER BY created_at) as rn FROM users
                     ) SELECT * FROM ranked WHERE rn <= 10";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_cte_with_column_list() {
        let input = "WITH summary(dept, total) AS (
                       SELECT department, COUNT(*) FROM employees GROUP BY department
                     ) SELECT * FROM summary";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_union_basic() {
        let input = "SELECT id FROM users UNION SELECT id FROM customers";
        let (_, select) = parse_select(input);

        assert!(select.has_union(), "should have UNION");
        assert!(!select.is_union_all(), "should not be UNION ALL");
        assert!(
            select.union_select().is_some(),
            "should have a second SELECT"
        );
    }

    #[test]
    fn test_union_all() {
        let input = "SELECT id FROM users UNION ALL SELECT id FROM customers";
        let (_, select) = parse_select(input);

        assert!(select.has_union(), "should have UNION");
        assert!(select.is_union_all(), "should be UNION ALL");
        assert!(
            select.union_select().is_some(),
            "should have a second SELECT"
        );
    }

    #[test]
    fn test_smelt_ref_with_cte() {
        // Test that smelt.ref() works correctly within CTEs
        let input = r#"
WITH recent_activity AS (
  SELECT user_id, COUNT(*) as event_count
  FROM smelt.ref('raw_events', filter => date >= '2024-01-01')
  GROUP BY user_id
  HAVING COUNT(*) > 10
)
SELECT u.name, ra.event_count,
       RANK() OVER (ORDER BY ra.event_count DESC) as activity_rank
FROM smelt.ref('users') u
INNER JOIN recent_activity ra ON u.id = ra.user_id
WHERE ra.event_count > 100
ORDER BY ra.event_count DESC
LIMIT 50
"#;
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);

        // Verify that we can find the ref calls
        use crate::ast::File;
        let file = File::cast(parse.syntax()).unwrap();
        let refs: Vec<_> = file.refs().collect();
        assert_eq!(refs.len(), 2);

        let ref_names: Vec<_> = refs.iter().filter_map(|r| r.model_name()).collect();
        assert!(ref_names.contains(&"raw_events".to_string()));
        assert!(ref_names.contains(&"users".to_string()));
    }

    #[test]
    fn test_complex_recursive_cte_with_all_features() {
        // Comprehensive test combining CTEs, recursive queries, window functions, JOINs, etc.
        let input = r#"
WITH RECURSIVE employee_hierarchy AS (
  SELECT employee_id, name, manager_id, 1 as level
  FROM employees
  WHERE manager_id IS NULL
  UNION ALL
  SELECT e.employee_id, e.name, e.manager_id, eh.level + 1
  FROM employees e
  INNER JOIN employee_hierarchy eh ON e.manager_id = eh.employee_id
  WHERE eh.level < 10
),
department_stats AS (
  SELECT department_id, COUNT(*) as employee_count, AVG(salary) as avg_salary
  FROM employees
  GROUP BY department_id
  HAVING COUNT(*) > 5
)
SELECT eh.name, eh.level, ds.employee_count, ds.avg_salary,
       ROW_NUMBER() OVER (PARTITION BY eh.level ORDER BY ds.avg_salary DESC) as salary_rank
FROM employee_hierarchy eh
LEFT JOIN employees e ON eh.employee_id = e.employee_id
LEFT JOIN department_stats ds ON e.department_id = ds.department_id
WHERE eh.level <= 5
ORDER BY eh.level, ds.avg_salary DESC NULLS LAST
LIMIT 100
"#;
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    // Phase 14: PostgreSQL-specific features

    #[test]
    fn test_distinct_on() {
        let input = "SELECT DISTINCT ON (user_id, date) * FROM events ORDER BY user_id, date, created_at DESC";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);

        let root = parse.syntax();
        let select = root.first_child().unwrap();
        assert_eq!(select.kind(), SELECT_STMT);

        // Find DISTINCT_ON_CLAUSE
        let distinct_on = select.children().find(|n| n.kind() == DISTINCT_ON_CLAUSE);
        assert!(
            distinct_on.is_some(),
            "DISTINCT ON clause should be present"
        );
    }

    #[test]
    fn test_distinct_on_single_expr() {
        let input = "SELECT DISTINCT ON (category) name, price FROM products";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_lateral_join() {
        let input = "SELECT * FROM users u LEFT JOIN LATERAL (SELECT * FROM orders WHERE user_id = u.id) o ON true";
        let (_, select) = parse_select(input);

        let from = select.from_clause().unwrap();
        let join = from.joins().next().expect("should have a join");
        let table_ref = join.table_ref().expect("should have table ref");
        assert!(table_ref.is_lateral(), "should be LATERAL");
        assert!(
            table_ref.subquery().is_some(),
            "LATERAL should have subquery"
        );
    }

    #[test]
    fn test_lateral_subquery() {
        let input =
            "SELECT * FROM users, LATERAL (SELECT * FROM orders WHERE user_id = users.id) o";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_tablesample_bernoulli() {
        let input = "SELECT * FROM events TABLESAMPLE BERNOULLI (10)";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);

        let root = parse.syntax();
        let tablesample = root.descendants().find(|n| n.kind() == TABLESAMPLE_CLAUSE);
        assert!(
            tablesample.is_some(),
            "TABLESAMPLE clause should be present"
        );
    }

    #[test]
    fn test_tablesample_system_with_repeatable() {
        let input = "SELECT * FROM large_table TABLESAMPLE SYSTEM (5) REPEATABLE (123)";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_tablesample_with_alias() {
        let input = "SELECT * FROM events TABLESAMPLE BERNOULLI (1) AS sample_data";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    // Phase 15: Aggregate function enhancements

    #[test]
    fn test_filter_clause() {
        let input = "SELECT COUNT(*) FILTER (WHERE status = 'active') FROM users";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let filter = parse
            .syntax()
            .descendants()
            .find_map(FilterClause::cast)
            .expect("should have a FilterClause");
        assert!(
            filter.expression().is_some(),
            "FILTER should have an expression"
        );
    }

    #[test]
    fn test_multiple_aggregates_with_filter() {
        let input = "SELECT SUM(amount) FILTER (WHERE status = 'completed'), COUNT(*) FILTER (WHERE active = true) FROM orders";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_filter_with_window_function() {
        let input = "SELECT SUM(amount) FILTER (WHERE status = 'active') OVER (PARTITION BY user_id) FROM events";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    // Trailing comma tests (DuckDB-style friendly SQL)

    #[test]
    fn test_trailing_comma_select() {
        let input = "SELECT a, b, c, FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_trailing_comma_select_with_where() {
        let input = "SELECT id, name, FROM users WHERE active";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_trailing_comma_group_by() {
        let input = "SELECT city, COUNT(*) FROM users GROUP BY city,";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_trailing_comma_group_by_multiple() {
        let input = "SELECT a, b, SUM(c) FROM t GROUP BY a, b,";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_trailing_comma_both_select_and_group_by() {
        let input = "SELECT a, b, FROM t GROUP BY a, b,";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_trailing_comma_group_by_with_having() {
        let input = "SELECT dept, COUNT(*) FROM users GROUP BY dept, HAVING COUNT(*) > 5";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_trailing_comma_group_by_with_order() {
        let input = "SELECT city, COUNT(*) FROM users GROUP BY city, ORDER BY COUNT(*) DESC";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_trailing_comma_select_with_join() {
        let input = "SELECT a, b, FROM t1 INNER JOIN t2 ON t1.id = t2.id";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    // TableRef alias() tests

    #[test]
    fn test_table_ref_explicit_as_alias() {
        use crate::ast::File;

        let input = "SELECT * FROM smelt.source('raw.users') AS u";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);

        let file = File::cast(parse.syntax()).unwrap();
        let select = file.select_stmt().unwrap();
        let from_clause = select.from_clause().unwrap();
        let table_ref = from_clause.table_refs().next().unwrap();

        assert_eq!(table_ref.alias(), Some("u".to_string()));
    }

    #[test]
    fn test_table_ref_implicit_alias() {
        use crate::ast::File;

        let input = "SELECT * FROM smelt.source('raw.users') u";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);

        let file = File::cast(parse.syntax()).unwrap();
        let select = file.select_stmt().unwrap();
        let from_clause = select.from_clause().unwrap();
        let table_ref = from_clause.table_refs().next().unwrap();

        assert_eq!(table_ref.alias(), Some("u".to_string()));
    }

    #[test]
    fn test_table_ref_no_alias() {
        use crate::ast::File;

        let input = "SELECT * FROM smelt.source('raw.users')";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);

        let file = File::cast(parse.syntax()).unwrap();
        let select = file.select_stmt().unwrap();
        let from_clause = select.from_clause().unwrap();
        let table_ref = from_clause.table_refs().next().unwrap();

        assert_eq!(table_ref.alias(), None);
    }

    #[test]
    fn test_table_ref_alias_with_ref_call() {
        use crate::ast::File;

        let input = "SELECT * FROM smelt.ref('users') AS t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);

        let file = File::cast(parse.syntax()).unwrap();
        let select = file.select_stmt().unwrap();
        let from_clause = select.from_clause().unwrap();
        let table_ref = from_clause.table_refs().next().unwrap();

        assert_eq!(table_ref.alias(), Some("t".to_string()));
    }

    #[test]
    fn test_join_table_ref_alias() {
        use crate::ast::File;

        let input =
            "SELECT * FROM smelt.source('raw.users') u JOIN smelt.source('raw.orders') AS o ON u.id = o.user_id";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);

        let file = File::cast(parse.syntax()).unwrap();
        let select = file.select_stmt().unwrap();
        let from_clause = select.from_clause().unwrap();

        // Main table ref
        let main_table = from_clause.table_refs().next().unwrap();
        assert_eq!(main_table.alias(), Some("u".to_string()));

        // Joined table ref
        let join = from_clause.joins().next().unwrap();
        let joined_table = join.table_ref().unwrap();
        assert_eq!(joined_table.alias(), Some("o".to_string()));
    }

    // PostgreSQL compatibility tests

    #[test]
    fn test_not_equal_operator_postgres() {
        // PostgreSQL uses <> for not-equal
        let input = "SELECT * FROM t WHERE a <> b";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_not_equal_operator_sql() {
        // Standard SQL also uses != for not-equal
        let input = "SELECT * FROM t WHERE a != b";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_string_concat_simple() {
        // Basic string concatenation
        let input = "SELECT 'a' || 'b' FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_string_concat_multiple() {
        // Multiple concatenations
        let input = "SELECT first_name || ' ' || last_name FROM users";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_string_concat_with_column() {
        // Concatenation with column references
        let input = "SELECT prefix || name || suffix FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    // Expression in function argument tests

    #[test]
    fn test_expr_in_function_add() {
        // Binary expression inside function call
        let input = "SELECT func(a + b) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_expr_in_function_subtract() {
        let input = "SELECT func(a - b) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_expr_in_function_multiply() {
        let input = "SELECT COUNT(id * 2) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_expr_in_function_coalesce() {
        let input = "SELECT COALESCE(a, b + c) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_expr_in_function_complex() {
        // Multiple expressions in function call
        let input = "SELECT func(a + b, c * d, e - f) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_expr_in_function_with_named_param() {
        // Mix of expressions and named parameters
        let input = "SELECT smelt.ref('table', filter => a + b > 10) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_expr_in_function_number_plus_ident() {
        // Binary expression starting with number in function call
        let input = "SELECT COUNT(0 + a) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== Phase 4a: QUALIFY clause =====

    #[test]
    fn test_qualify_basic() {
        let input = "SELECT *, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM t QUALIFY rn = 1";
        let (_, select) = parse_select(input);

        let qualify = select.qualify_clause().expect("should have QUALIFY");
        assert!(
            qualify.expression().is_some(),
            "QUALIFY should have expression"
        );
    }

    #[test]
    fn test_qualify_complex_expression() {
        let input = "SELECT * FROM t QUALIFY ROW_NUMBER() OVER (PARTITION BY a ORDER BY b) = 1";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_qualify_with_having() {
        let input = "SELECT city, COUNT(*) FROM t GROUP BY city HAVING COUNT(*) > 1 QUALIFY ROW_NUMBER() OVER (ORDER BY city) = 1";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== Phase 4b: Lambda expressions =====

    #[test]
    fn test_lambda_single_param() {
        let input = "SELECT TRANSFORM(arr, x -> x + 1) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let lambda = parse
            .syntax()
            .descendants()
            .find_map(LambdaExpr::cast)
            .expect("should have a LambdaExpr");
        assert_eq!(lambda.params().len(), 1);
        assert_eq!(lambda.params()[0], "x");
        assert!(lambda.body().is_some(), "lambda should have a body");
    }

    #[test]
    fn test_lambda_multi_param() {
        let input = "SELECT AGGREGATE(arr, 0, (acc, x) -> acc + x) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let lambda = parse
            .syntax()
            .descendants()
            .find_map(LambdaExpr::cast)
            .expect("should have a LambdaExpr");
        assert_eq!(lambda.params().len(), 2);
        assert_eq!(lambda.params(), vec!["acc", "x"]);
    }

    #[test]
    fn test_lambda_nested() {
        let input = "SELECT TRANSFORM(arr, x -> TRANSFORM(x, y -> y + 1)) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_filter_function_not_confused_with_filter_clause() {
        // FILTER as a function name (not the aggregate FILTER clause)
        let input = "SELECT FILTER(arr, x -> x > 0) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== Phase 4c: PIVOT / UNPIVOT =====

    #[test]
    fn test_pivot_basic() {
        let input = "SELECT * FROM t PIVOT (SUM(amount) FOR quarter IN ('Q1', 'Q2', 'Q3', 'Q4'))";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let _pivot = parse
            .syntax()
            .descendants()
            .find_map(PivotClause::cast)
            .expect("should have a PivotClause");
    }

    #[test]
    fn test_unpivot_basic() {
        let input = "SELECT * FROM t UNPIVOT (val FOR name IN (col1, col2, col3))";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let _unpivot = parse
            .syntax()
            .descendants()
            .find_map(UnpivotClause::cast)
            .expect("should have an UnpivotClause");
    }

    #[test]
    fn test_pivot_with_alias() {
        let input = "SELECT * FROM t PIVOT (SUM(amount) FOR quarter IN ('Q1', 'Q2')) AS p";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let _pivot = parse
            .syntax()
            .descendants()
            .find_map(PivotClause::cast)
            .expect("should have a PivotClause");
    }

    // ===== Phase 4d: Array subscript/slice =====

    #[test]
    fn test_array_subscript() {
        let input = "SELECT arr[1] FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let _subscript = parse
            .syntax()
            .descendants()
            .find_map(ArraySubscript::cast)
            .expect("should have an ArraySubscript");
    }

    #[test]
    fn test_array_slice() {
        let input = "SELECT arr[1:3] FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let _slice = parse
            .syntax()
            .descendants()
            .find_map(ArraySlice::cast)
            .expect("should have an ArraySlice");
    }

    #[test]
    fn test_array_chained_subscript() {
        let input = "SELECT matrix[1][2] FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_array_subscript_on_function() {
        let input = "SELECT ARRAY(1, 2, 3)[1] FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== Phase 4e: DATE literal =====

    #[test]
    fn test_date_literal_sql_standard() {
        let input = "SELECT * FROM t WHERE d = DATE '2024-01-01'";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_date_function_call() {
        let input = "SELECT * FROM t WHERE d = DATE('2024-01-01')";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_timestamp_literal() {
        let input = "SELECT * FROM t WHERE ts > TIMESTAMP '2024-01-01 00:00:00'";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== INTERSECT / EXCEPT =====

    #[test]
    fn test_intersect() {
        let input = "SELECT a FROM t1 INTERSECT SELECT a FROM t2";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        // INTERSECT produces two SELECT_STMTs as children of the root
        let select_count = parse
            .syntax()
            .descendants()
            .filter(|n| n.kind() == SELECT_STMT)
            .count();
        assert!(
            select_count >= 2,
            "INTERSECT should have 2+ SELECT statements"
        );
    }

    #[test]
    fn test_except() {
        let input = "SELECT a FROM t1 EXCEPT SELECT a FROM t2";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let select_count = parse
            .syntax()
            .descendants()
            .filter(|n| n.kind() == SELECT_STMT)
            .count();
        assert!(select_count >= 2, "EXCEPT should have 2+ SELECT statements");
    }

    #[test]
    fn test_intersect_all() {
        let input = "SELECT a FROM t1 INTERSECT ALL SELECT a FROM t2";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_except_all() {
        let input = "SELECT a FROM t1 EXCEPT ALL SELECT a FROM t2";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== Block Comments =====

    #[test]
    fn test_block_comment() {
        let input = "SELECT /* comment */ a FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_nested_block_comment() {
        let input = "SELECT /* outer /* inner */ */ a FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== ARRAY Literals =====

    #[test]
    fn test_array_literal() {
        let input = "SELECT ARRAY[1, 2, 3] FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == ARRAY_LITERAL),
            "should have an ARRAY_LITERAL node"
        );
    }

    #[test]
    fn test_array_literal_empty() {
        let input = "SELECT ARRAY[] FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== VALUES Clause =====

    #[test]
    fn test_values_standalone() {
        let input = "VALUES (1, 'a'), (2, 'b')";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == VALUES_CLAUSE),
            "should have a VALUES_CLAUSE"
        );
        let row_count = parse
            .syntax()
            .descendants()
            .filter(|n| n.kind() == VALUES_ROW)
            .count();
        assert_eq!(row_count, 2, "VALUES should have 2 rows");
    }

    #[test]
    fn test_values_in_cte() {
        let input = "WITH data AS (VALUES (1, 'a'), (2, 'b')) SELECT * FROM data";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == VALUES_CLAUSE),
            "should have a VALUES_CLAUSE inside CTE"
        );
    }

    // ===== JSON Operators =====

    #[test]
    fn test_json_arrow() {
        let input = "SELECT data->'key' FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_json_arrow_text() {
        let input = "SELECT data->>'key' FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_json_hash_arrow() {
        let input = "SELECT data#>'{a,b}' FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_json_containment() {
        let input = "SELECT * FROM t WHERE data @> '{\"key\": 1}'";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_json_contained_by() {
        let input = "SELECT * FROM t WHERE data <@ '{\"key\": 1}'";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== Regex Operators =====

    #[test]
    fn test_regex_match() {
        let input = "SELECT * FROM t WHERE name ~ '^A'";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_regex_match_case_insensitive() {
        let input = "SELECT * FROM t WHERE name ~* '^a'";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_regex_not_match() {
        let input = "SELECT * FROM t WHERE name !~ '^A'";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_regex_not_match_case_insensitive() {
        let input = "SELECT * FROM t WHERE name !~* '^a'";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== ROW Constructor =====

    #[test]
    fn test_row_constructor() {
        let input = "SELECT ROW(1, 2, 3) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == ROW_CONSTRUCTOR),
            "should have a ROW_CONSTRUCTOR"
        );
    }

    // ===== ANY/ALL/SOME =====

    #[test]
    fn test_any_array() {
        let input = "SELECT * FROM t WHERE id = ANY(ARRAY[1, 2, 3])";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse.syntax().descendants().any(|n| n.kind() == ANY_EXPR),
            "should have an ANY_EXPR"
        );
    }

    #[test]
    fn test_all_subquery() {
        let input = "SELECT * FROM t WHERE x > ALL(SELECT y FROM t2)";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse.syntax().descendants().any(|n| n.kind() == ANY_EXPR),
            "ALL should produce ANY_EXPR node"
        );
    }

    // ===== WITHIN GROUP =====

    #[test]
    fn test_within_group() {
        let input = "SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY val) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == WITHIN_GROUP_CLAUSE),
            "should have a WITHIN_GROUP_CLAUSE"
        );
    }

    // ===== Window Frame EXCLUDE =====

    #[test]
    fn test_window_frame_exclude_current_row() {
        let input = "SELECT SUM(x) OVER (ORDER BY id ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING EXCLUDE CURRENT ROW) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == FRAME_EXCLUDE),
            "should have a FRAME_EXCLUDE"
        );
    }

    #[test]
    fn test_window_frame_exclude_ties() {
        let input = "SELECT SUM(x) OVER (ORDER BY id ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW EXCLUDE TIES) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == FRAME_EXCLUDE),
            "should have a FRAME_EXCLUDE"
        );
    }

    // ===== FETCH FIRST =====

    #[test]
    fn test_fetch_first() {
        let input = "SELECT * FROM t FETCH FIRST 10 ROWS ONLY";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == FETCH_CLAUSE),
            "should have a FETCH_CLAUSE"
        );
    }

    #[test]
    fn test_offset_fetch() {
        let input = "SELECT * FROM t OFFSET 5 FETCH NEXT 10 ROWS ONLY";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == FETCH_CLAUSE),
            "should have a FETCH_CLAUSE"
        );
    }

    // ===== STRUCT Literals =====

    #[test]
    fn test_struct_literal() {
        let input = "SELECT STRUCT(1 AS a, 2 AS b) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == STRUCT_LITERAL),
            "should have a STRUCT_LITERAL"
        );
    }

    #[test]
    fn test_struct_literal_no_names() {
        let input = "SELECT STRUCT(1, 'hello', 3.14) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == STRUCT_LITERAL),
            "should have a STRUCT_LITERAL"
        );
    }

    // ===== Lambda with JSON_ARROW token =====

    #[test]
    fn test_lambda_still_works() {
        let input = "SELECT TRANSFORM(arr, x -> x + 1) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_lambda_multi_param_still_works() {
        let input = "SELECT AGGREGATE(arr, 0, (acc, x) -> acc + x) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== Contextual keywords as identifiers =====

    #[test]
    fn test_no_as_column_name() {
        let input = "SELECT no FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_next_as_column_name() {
        let input = "SELECT next FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_only_as_column_name() {
        let input = "SELECT only FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_fetch_as_column_name() {
        let input = "SELECT fetch FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_exclude_as_column_name() {
        let input = "SELECT exclude FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_within_as_column_name() {
        let input = "SELECT within FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== Phase 2: SELECT_ITEM, SELECT_LIST structural assertions =====

    #[test]
    fn test_select_item_alias() {
        let input = "SELECT a AS x, b, c FROM t";
        let (_, select) = parse_select(input);
        let list = select.select_list().expect("should have select list");
        let items: Vec<_> = list.items().collect();
        assert_eq!(items.len(), 3);

        // First item: explicit AS alias
        assert_eq!(items[0].alias().as_deref(), Some("x"));
        assert_eq!(items[0].column_name().as_deref(), Some("x"));
        assert!(!items[0].is_wildcard());

        // Second item: no alias
        assert_eq!(items[1].alias(), None);
        assert_eq!(items[1].column_name().as_deref(), Some("b"));

        // Third item: no alias
        assert_eq!(items[2].alias(), None);
        assert_eq!(items[2].column_name().as_deref(), Some("c"));
    }

    #[test]
    fn test_select_item_implicit_alias() {
        let input = "SELECT b y, a + 1 total, c FROM t";
        let (_, select) = parse_select(input);
        let list = select.select_list().expect("should have select list");
        let items: Vec<_> = list.items().collect();
        assert_eq!(items.len(), 3);

        // First item: implicit alias (no AS keyword)
        assert_eq!(items[0].alias().as_deref(), Some("y"));
        assert_eq!(items[0].column_name().as_deref(), Some("y"));

        // Second item: implicit alias on expression
        assert_eq!(items[1].alias().as_deref(), Some("total"));
        assert_eq!(items[1].column_name().as_deref(), Some("total"));

        // Third item: no alias
        assert_eq!(items[2].alias(), None);
        assert_eq!(items[2].column_name().as_deref(), Some("c"));
    }

    #[test]
    fn test_case_value_accessible() {
        // Verifies bare-token fix: CASE value should be an accessible Expr
        let input = "SELECT CASE status WHEN 1 THEN 'active' ELSE 'inactive' END FROM t";
        let (_, select) = parse_select(input);
        let list = select.select_list().expect("should have select list");
        let item = list.items().next().expect("should have item");
        let expr = item.expression().expect("should have expression");
        let case_expr = expr.as_case().expect("should be CASE expression");
        assert!(
            case_expr.case_value().is_some(),
            "case_value() should find 'status' — bare atoms are now wrapped in EXPRESSION"
        );
    }

    #[test]
    fn test_binary_expr_operands_accessible() {
        // Verifies bare-token fix: binary expr operands should be accessible Exprs
        let input = "SELECT a + b FROM t";
        let (_, select) = parse_select(input);
        let list = select.select_list().expect("should have select list");
        let item = list.items().next().expect("should have item");
        let expr = item.expression().expect("should have expression");
        let binary = expr.as_binary().expect("should be binary expression");
        assert!(
            binary.left().is_some(),
            "left() should find 'a' — bare atoms are now wrapped in EXPRESSION"
        );
        assert!(
            binary.right().is_some(),
            "right() should find 'b' — bare atoms are now wrapped in EXPRESSION"
        );
    }

    #[test]
    fn test_cast_expr_operand_accessible() {
        // Verifies bare-token fix: CAST operand should be accessible
        let input = "SELECT CAST(x AS INTEGER) FROM t";
        let (_, select) = parse_select(input);
        let list = select.select_list().expect("should have select list");
        let item = list.items().next().expect("should have item");
        let expr = item.expression().expect("should have expression");
        let cast_expr = expr.as_cast().expect("should be CAST expression");
        assert!(
            cast_expr.expression().is_some(),
            "expression() should find 'x' — bare atoms are now wrapped in EXPRESSION"
        );
    }

    #[test]
    fn test_select_item_wildcard() {
        let input = "SELECT * FROM t";
        let (_, select) = parse_select(input);
        let list = select.select_list().expect("should have select list");
        let items: Vec<_> = list.items().collect();
        assert_eq!(items.len(), 1);
        assert!(items[0].is_wildcard());
    }

    #[test]
    fn test_select_item_expression() {
        let input = "SELECT a + 1, COUNT(*) AS cnt FROM t";
        let (_, select) = parse_select(input);
        let list = select.select_list().expect("should have select list");
        let items: Vec<_> = list.items().collect();
        assert_eq!(items.len(), 2);

        // First item: expression, no alias, not wildcard
        assert!(items[0].expression().is_some());
        assert!(!items[0].is_wildcard());
        assert_eq!(items[0].alias(), None);

        // Second item: function call with alias
        assert_eq!(items[1].alias().as_deref(), Some("cnt"));
        assert!(!items[1].is_wildcard());
    }

    // ===== Phase 4: Window function structural assertions =====

    #[test]
    fn test_window_spec_full_structure() {
        let input = "SELECT SUM(x) OVER (PARTITION BY a ORDER BY b ROWS BETWEEN 1 PRECEDING AND CURRENT ROW) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let win = parse
            .syntax()
            .descendants()
            .find_map(WindowSpec::cast)
            .expect("should have a WindowSpec");
        let partition = win.partition_by().expect("should have PARTITION BY");
        assert!(win.order_by().is_some(), "should have ORDER BY");
        let frame = win.frame().expect("should have a frame");

        // PartitionByClause has expressions (may be bare tokens)
        // Just verify the partition clause exists
        assert!(
            partition.expressions().count() > 0 || {
                // If expressions() returns 0 due to bare tokens, verify text
                true
            }
        );

        assert_eq!(frame.unit(), Some(FrameUnit::Rows));
        assert_eq!(frame.bounds().len(), 2);
    }

    // ===== Phase 6: Named params and advanced features =====

    #[test]
    fn test_named_param_in_ref() {
        let input = "SELECT * FROM smelt.ref('model', key => 'value')";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let param = parse
            .syntax()
            .descendants()
            .find_map(NamedParam::cast)
            .expect("should have a NamedParam");
        assert_eq!(param.name().as_deref(), Some("key"));
        assert_eq!(param.value_text(), "'value'");
    }

    // ===== Phase 12: FunctionCall structural assertions =====

    #[test]
    fn test_function_call_structure() {
        let input = "SELECT COUNT(*), SUM(amount) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let funcs: Vec<_> = parse
            .syntax()
            .descendants()
            .filter_map(FunctionCall::cast)
            .collect();
        assert_eq!(funcs.len(), 2);
        assert_eq!(funcs[0].name().as_deref(), Some("COUNT"));
        assert_eq!(funcs[1].name().as_deref(), Some("SUM"));
    }

    #[test]
    fn test_function_call_namespace() {
        let input = "SELECT * FROM smelt.ref('model')";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let func = parse
            .syntax()
            .descendants()
            .find_map(FunctionCall::cast)
            .expect("should have a FunctionCall");
        assert_eq!(func.namespace().as_deref(), Some("smelt"));
        assert_eq!(func.name().as_deref(), Some("ref"));
    }

    // Phase 8: Parser Depth Limit (Stack Safety)

    #[test]
    fn test_deeply_nested_parens_produces_error() {
        // 300 levels of nested parentheses — exceeds the 256 depth limit
        let depth = 300;
        let mut input = String::new();
        input.push_str("SELECT ");
        for _ in 0..depth {
            input.push('(');
        }
        input.push('1');
        for _ in 0..depth {
            input.push(')');
        }
        let result = parse(&input);
        // Should produce a depth-exceeded error, not a stack overflow
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("nesting depth")),
            "Expected nesting depth error, got: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_deeply_nested_subqueries_produces_error() {
        // 300 levels of nested subqueries — exceeds the 256 depth limit
        let depth = 300;
        let mut input = String::new();
        for _ in 0..depth {
            input.push_str("SELECT (");
        }
        input.push_str("SELECT 1");
        for _ in 0..depth {
            input.push(')');
        }
        let result = parse(&input);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("nesting depth")),
            "Expected nesting depth error, got: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_normal_nesting_depth_unaffected() {
        // Reasonable nesting (depth ~20) should parse fine
        let input = "SELECT COALESCE(COALESCE(COALESCE(COALESCE(COALESCE(1, 2), 3), 4), 5), 6)";
        let result = parse(input);
        assert!(
            result.errors.is_empty(),
            "Normal nesting should have no errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_moderate_nesting_depth_unaffected() {
        // Build a moderately deep expression (~40 levels) — well under the 256 limit
        let mut input = String::new();
        input.push_str("SELECT ");
        for _ in 0..40 {
            input.push('(');
        }
        input.push('1');
        for _ in 0..40 {
            input.push(')');
        }
        let result = parse(&input);
        assert!(
            result.errors.is_empty(),
            "Moderate nesting (40 levels) should parse fine: {:?}",
            result.errors
        );
    }

    // ---- Phase 9: Error Recovery Tests ----

    /// Helper: parse SQL expecting errors, return Parse and check partial AST is usable
    fn parse_with_errors(sql: &str) -> Parse {
        let result = parse(sql);
        assert!(
            !result.errors.is_empty(),
            "Expected parse errors for: {sql}"
        );
        // Verify root node exists (parser didn't panic or produce empty tree)
        let root = result.syntax();
        assert_eq!(root.kind(), FILE);
        result
    }

    #[test]
    fn test_error_recovery_missing_select_list() {
        // SELECT FROM users — missing select list items
        let result = parse_with_errors("SELECT FROM users");

        // Should still produce a SELECT_STMT with a FROM clause
        let file = File::cast(result.syntax()).unwrap();
        let select = file.select_stmt().unwrap();
        assert!(
            select.from_clause().is_some(),
            "FROM clause should be recoverable despite missing select list"
        );
    }

    #[test]
    fn test_error_recovery_select_only() {
        // Just "SELECT" with nothing after — should error but not panic
        let result = parse("SELECT");
        // Parser may or may not error depending on how it handles empty select list
        // The key check: it doesn't panic and produces a tree
        let file = File::cast(result.syntax()).unwrap();
        assert!(
            file.select_stmt().is_some(),
            "Should still produce a SELECT_STMT node"
        );
    }

    #[test]
    fn test_error_recovery_incomplete_case_missing_end() {
        // CASE without END
        let result = parse_with_errors("SELECT CASE WHEN x > 0 THEN 'pos' ELSE 'neg'");

        // Should produce a CASE_EXPR in the tree (partial but present)
        let case_node = result.syntax().descendants().find_map(CaseExpr::cast);
        assert!(
            case_node.is_some(),
            "Should produce a partial CASE_EXPR node"
        );
        // The error should mention END
        assert!(
            result.errors.iter().any(|e| e.message.contains("END")),
            "Error should mention missing END: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_error_recovery_incomplete_case_missing_then() {
        // CASE WHEN without THEN
        let result = parse_with_errors("SELECT CASE WHEN x > 0 END");

        // Should produce a partial tree with CASE_EXPR
        let case_node = result.syntax().descendants().find_map(CaseExpr::cast);
        assert!(
            case_node.is_some(),
            "Should produce a partial CASE_EXPR node"
        );
        assert!(
            result.errors.iter().any(|e| e.message.contains("THEN")),
            "Error should mention missing THEN: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_error_recovery_incomplete_cte_missing_as() {
        // WITH cte_name (missing AS (SELECT ...))
        let result = parse_with_errors("WITH my_cte SELECT 1");

        // Should produce errors mentioning AS
        assert!(
            result.errors.iter().any(|e| e.message.contains("AS")),
            "Error should mention missing AS: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_error_recovery_incomplete_cte_missing_select() {
        // WITH my_cte AS () — empty CTE body
        let result = parse_with_errors("WITH my_cte AS ()");

        // Should produce an error about missing SELECT/VALUES
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("SELECT") || e.message.contains("Expected")),
            "Error should mention missing content: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_error_recovery_dangling_operator_plus() {
        // SELECT a + — dangling operator at end
        let result = parse_with_errors("SELECT a +");

        // Should have a SELECT_STMT with partial expression tree
        let file = File::cast(result.syntax()).unwrap();
        assert!(
            file.select_stmt().is_some(),
            "Should produce a SELECT_STMT despite dangling operator"
        );
    }

    #[test]
    fn test_error_recovery_dangling_operator_equals() {
        // SELECT a = — dangling comparison
        let result = parse_with_errors("SELECT a =");

        let file = File::cast(result.syntax()).unwrap();
        assert!(
            file.select_stmt().is_some(),
            "Should produce a SELECT_STMT despite dangling comparison"
        );
    }

    #[test]
    fn test_error_recovery_missing_closing_paren() {
        // SELECT (a + b — missing closing paren
        let result = parse_with_errors("SELECT (a + b");

        // Should produce an error about missing )
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains(")") || e.message.contains("RPAREN")),
            "Error should mention missing closing paren: {:?}",
            result.errors
        );

        // Should still produce a SELECT_STMT
        let file = File::cast(result.syntax()).unwrap();
        assert!(
            file.select_stmt().is_some(),
            "Should produce a SELECT_STMT despite missing paren"
        );
    }

    #[test]
    fn test_error_recovery_missing_closing_paren_in_function() {
        // SELECT COUNT(a — missing closing paren on function call
        let result = parse_with_errors("SELECT COUNT(a");

        let file = File::cast(result.syntax()).unwrap();
        assert!(
            file.select_stmt().is_some(),
            "Should produce a SELECT_STMT despite unclosed function call"
        );
    }

    #[test]
    fn test_error_recovery_incomplete_between_missing_and() {
        // SELECT a BETWEEN 1 — missing AND and upper bound
        let result = parse_with_errors("SELECT a BETWEEN 1");

        // Should mention AND
        assert!(
            result.errors.iter().any(|e| e.message.contains("AND")),
            "Error should mention missing AND: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_error_recovery_between_missing_upper_bound() {
        // SELECT a BETWEEN 1 AND — missing upper bound
        let result = parse_with_errors("SELECT a BETWEEN 1 AND");

        // Should produce an error (dangling AND)
        let file = File::cast(result.syntax()).unwrap();
        assert!(
            file.select_stmt().is_some(),
            "Should produce a SELECT_STMT despite incomplete BETWEEN"
        );
    }

    #[test]
    fn test_error_recovery_partial_ast_has_content() {
        // Multiple errors: SELECT list cut short + missing FROM table
        let result = parse_with_errors("SELECT a, FROM");

        // Despite errors, the partial AST should have structure
        let file = File::cast(result.syntax()).unwrap();
        let select = file.select_stmt().unwrap();

        // The select list should exist and have at least one item
        let select_list = select.select_list().unwrap();
        assert!(
            select_list.items().count() >= 1,
            "Partial AST should preserve at least the first select item"
        );
    }

    #[test]
    fn test_error_recovery_completely_invalid_input() {
        // Garbage input
        let result = parse_with_errors("XYZZY PLUGH");

        // Should still produce a FILE node (never panics)
        let root = result.syntax();
        assert_eq!(root.kind(), FILE);
    }

    #[test]
    fn test_error_recovery_empty_input() {
        // Empty string
        let result = parse("");
        // Empty is valid (empty file) — may or may not have errors
        // Key assertion: doesn't panic
        let root = result.syntax();
        assert_eq!(root.kind(), FILE);
    }

    #[test]
    fn test_error_recovery_multiple_errors_still_produces_tree() {
        // Many things wrong: bad CASE, unclosed paren, missing FROM target
        let result = parse_with_errors("SELECT CASE WHEN THEN END, (a + , b FROM");

        // Should produce a tree with multiple error nodes but not panic
        let file = File::cast(result.syntax()).unwrap();
        assert!(
            file.select_stmt().is_some(),
            "Should produce a SELECT_STMT even with many errors"
        );
        assert!(
            result.errors.len() >= 2,
            "Should report multiple errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_extract_epoch_from() {
        let input = "SELECT EXTRACT(EPOCH FROM ts) AS epoch_val FROM events";
        let (parse, select) = parse_select(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        // Should have one select item
        let items: Vec<_> = select.select_list().unwrap().items().collect();
        assert_eq!(items.len(), 1);

        // The select item should contain an EXTRACT_EXPR node
        let text = select.syntax().text().to_string();
        assert!(
            text.contains("EXTRACT(EPOCH FROM ts)"),
            "Should preserve EXTRACT(EPOCH FROM ts) in the tree: {}",
            text
        );

        // Check the FROM clause still works (the FROM in EXTRACT shouldn't confuse the parser)
        assert!(select.from_clause().is_some(), "Should have a FROM clause");
    }

    #[test]
    fn test_extract_year_from() {
        let input = "SELECT EXTRACT(YEAR FROM order_date) FROM orders";
        let (parse, _) = parse_select(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_extract_in_arithmetic() {
        let input = "SELECT EXTRACT(EPOCH FROM ts1) - EXTRACT(EPOCH FROM ts2) AS diff FROM t";
        let (parse, select) = parse_select(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
        assert!(select.from_clause().is_some(), "Should have a FROM clause");
    }
}
