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

struct Parser<'a> {
    input: &'a str,
    tokens: &'a [Token],
    pos: usize,
    offset: usize,
    builder: GreenNodeBuilder<'static>,
    errors: Vec<ParseError>,
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

    /// Advance to next token, consuming trivia
    fn advance(&mut self) {
        if self.pos < self.tokens.len() {
            let token = self.tokens[self.pos];
            let text = &self.input[self.offset..self.offset + token.len];
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
        self.at_any(&[WHERE_KW, GROUP_KW])
    }

    // ===== Parsing rules =====

    fn parse_file(&mut self) {
        self.start_node(FILE);

        self.skip_trivia();

        // Parse SELECT statement
        if self.at(SELECT_KW) {
            self.parse_select_stmt();
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

        // SELECT
        self.expect(SELECT_KW);

        // Select list
        self.parse_select_list();

        // FROM clause
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

        self.finish_node();
    }

    fn parse_select_list(&mut self) {
        self.start_node(SELECT_LIST);
        self.skip_trivia();

        // Handle SELECT *
        if self.at(STAR) {
            self.start_node(SELECT_ITEM);
            self.advance();
            self.finish_node();
        } else {
            // Parse comma-separated select items
            loop {
                self.parse_select_item();

                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                } else {
                    break;
                }
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

        // Parse table references (could be identifier or template)
        loop {
            self.parse_table_ref();

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
            } else {
                break;
            }
        }

        self.finish_node();
    }

    fn parse_table_ref(&mut self) {
        self.start_node(TABLE_REF);
        self.skip_trivia();

        if self.at(IDENT) {
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
            } else {
                break;
            }
        }

        self.finish_node();
    }

    fn parse_expression(&mut self) {
        self.start_node(EXPRESSION);
        self.skip_trivia();

        self.parse_or_expr();

        self.finish_node();
    }

    fn parse_or_expr(&mut self) {
        self.parse_and_expr();

        while self.at(OR_KW) {
            self.start_node(BINARY_EXPR);
            self.advance();
            self.skip_trivia();
            self.parse_and_expr();
            self.finish_node();
        }
    }

    fn parse_and_expr(&mut self) {
        self.parse_comparison_expr();

        while self.at(AND_KW) {
            self.start_node(BINARY_EXPR);
            self.advance();
            self.skip_trivia();
            self.parse_comparison_expr();
            self.finish_node();
        }
    }

    fn parse_comparison_expr(&mut self) {
        self.parse_additive_expr();

        while self.at_any(&[EQ, NE, LT, GT, LE, GE]) {
            self.start_node(BINARY_EXPR);
            self.advance();
            self.skip_trivia();
            self.parse_additive_expr();
            self.finish_node();
        }
    }

    fn parse_additive_expr(&mut self) {
        self.parse_multiplicative_expr();

        while self.at_any(&[PLUS, MINUS]) {
            self.start_node(BINARY_EXPR);
            self.advance();
            self.skip_trivia();
            self.parse_multiplicative_expr();
            self.finish_node();
        }
    }

    fn parse_multiplicative_expr(&mut self) {
        self.parse_primary_expr();

        while self.at_any(&[STAR, DIVIDE]) {
            self.start_node(BINARY_EXPR);
            self.advance();
            self.skip_trivia();
            self.parse_primary_expr();
            self.finish_node();
        }
    }

    fn parse_primary_expr(&mut self) {
        self.skip_trivia();

        if self.at(LPAREN) {
            // Parenthesized expression or function call
            let _checkpoint = self.pos;
            self.advance();
            self.skip_trivia();

            // Check if it's a function call (preceded by identifier)
            // For now, just parse as grouped expression
            self.parse_expression();
            self.skip_trivia();
            self.expect(RPAREN);
        } else if self.at(IDENT) {
            // Could be column reference, qualified name, or function call
            let checkpoint = self.builder.checkpoint();
            self.advance(); // consume first IDENT
            self.skip_trivia();

            if self.at(LPAREN) {
                // Simple function call: func()
                self.start_node_at(checkpoint, FUNCTION_CALL);
                self.parse_arg_list();
                self.finish_node();
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
                    self.finish_node();
                }
                // else: just a qualified name (table.column), no extra node needed
            }
            // else: just an identifier, no extra node needed
        } else if self.current().is_literal() || self.at(STAR) {
            self.advance();
        } else {
            self.error(format!("Expected expression, found {:?}", self.current()));
        }
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

    fn parse_argument(&mut self) {
        self.skip_trivia();

        // Check for named parameter: IDENT => expression
        if self.at(IDENT) {
            // Look ahead to check for ARROW
            let checkpoint = self.builder.checkpoint();
            self.advance(); // consume IDENT
            self.skip_trivia();

            if self.at(ARROW) {
                // It's a named parameter
                self.start_node_at(checkpoint, NAMED_PARAM);
                self.advance(); // consume ARROW
                self.skip_trivia();
                self.parse_expression();
                self.finish_node();
            } else {
                // Not a named parameter, need to parse the rest as expression
                // The IDENT is already consumed, continue parsing expression from here
                self.skip_trivia();

                // Handle cases where IDENT might be followed by operators or function calls
                if self.at(LPAREN) {
                    // Function call - wrap in FUNCTION_CALL
                    self.start_node_at(checkpoint, FUNCTION_CALL);
                    self.parse_arg_list();
                    self.finish_node();
                } else if self.at(DOT) {
                    // Qualified name or namespaced function
                    self.advance();
                    self.skip_trivia();
                    self.expect(IDENT);
                    self.skip_trivia();

                    if self.at(LPAREN) {
                        // Namespaced function call
                        self.start_node_at(checkpoint, FUNCTION_CALL);
                        self.parse_arg_list();
                        self.finish_node();
                    }
                }
                // Otherwise, the IDENT alone is the expression (already consumed)
            }
        } else {
            // Not starting with IDENT, parse as regular expression
            self.parse_expression();
        }
    }
}
