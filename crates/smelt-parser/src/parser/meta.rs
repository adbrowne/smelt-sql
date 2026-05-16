//! Meta-language (Phase B+, Phase F) grammar — lambdas, ternary, reducer calls.
//!
//! Covers:
//!   - `fn IDENT => EXPR` reserved-keyword lambda (Phase B)
//!   - `fn ( IDENT, ... ) => EXPR` parenthesised multi-arg lambda (Phase F)
//!   - `if COND then THEN else ELSE` meta-world ternary (Phase F)
//!   - Parameterised reducer call at second-arg of `reduce` (Phase F)
//!   - Legacy single/multi-param `x -> EXPR` / `(x, y) -> EXPR` lambdas

use crate::SyntaxKind::*;

impl<'a> super::Parser<'a> {
    /// Check if current position starts a single-param lambda: ident ->
    /// Lambda arrow is MINUS GT (two adjacent tokens with no space between)
    pub(super) fn is_lambda_single_param(&self) -> bool {
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
    pub(super) fn is_thin_arrow_at(&self, offset: usize) -> bool {
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
    pub(super) fn is_lambda_multi_param(&self) -> bool {
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
    /// Parse a Phase B/F meta-language lambda: `fn IDENT => EXPR` or `fn ( IDENT, ... ) => EXPR`.
    ///
    /// Produces a `LAMBDA` node whose children are:
    ///   - `FN_KW` token (the reserved `fn` keyword)
    ///   - `LAMBDA_PARAM_LIST` — contains one or more `LAMBDA_PARAM` children, each wrapping
    ///     a single IDENT. For zero-param `fn () => body`, the LAMBDA_PARAM_LIST is empty
    ///     (downstream emits `LambdaZeroParameters`).
    ///   - `ARROW` token (`=>`)
    ///   - `EXPRESSION` — the lambda body
    ///
    /// The caller must have verified `self.is_fn_lambda_start()` before calling this.
    pub(super) fn parse_fn_lambda(&mut self) {
        self.start_node(LAMBDA);

        // Consume the `fn` reserved keyword (FN_KW).
        self.advance(); // FN_KW
        self.skip_trivia();

        // Parse the parameter list.
        if self.at(LPAREN) {
            // Parenthesised form: `fn (a, b) => body` (Phase F).
            // Also handles single-arg `fn (x) => body` and zero-arg `fn () => body`.
            self.start_node(LAMBDA_PARAM_LIST);
            self.advance(); // LPAREN
            self.skip_trivia();
            loop {
                // Trailing comma: if we see RPAREN, break.
                if self.at(RPAREN) || self.at(EOF) {
                    break;
                }
                if self.at(IDENT) {
                    // Wrap each parameter in a LAMBDA_PARAM node.
                    self.start_node(LAMBDA_PARAM);
                    self.advance(); // parameter IDENT
                    self.finish_node(); // LAMBDA_PARAM
                    self.skip_trivia();
                }
                if self.at(COMMA) {
                    self.advance(); // ,
                    self.skip_trivia();
                    // Allow trailing comma: loop continues and RPAREN check at top will break.
                } else {
                    break;
                }
            }
            if self.at(RPAREN) {
                self.advance(); // RPAREN
            } else {
                self.error("Expected ')' to close lambda parameter list".to_string());
            }
            self.finish_node(); // LAMBDA_PARAM_LIST
        } else if self.at(IDENT) {
            // Single-arg bare form: `fn IDENT => body`.
            // Wrap the parameter in a LAMBDA_PARAM node for consistency with multi-arg form.
            self.start_node(LAMBDA_PARAM_LIST);
            self.start_node(LAMBDA_PARAM);
            self.advance(); // IDENT
            self.finish_node(); // LAMBDA_PARAM
            self.finish_node(); // LAMBDA_PARAM_LIST
        } else {
            self.error("Expected identifier or '(' after 'fn' in lambda expression".to_string());
            // Emit an empty LAMBDA_PARAM_LIST for error recovery.
            self.start_node(LAMBDA_PARAM_LIST);
            self.finish_node();
        }

        // Expect `=>` (ARROW token).
        self.skip_trivia();
        if self.at(ARROW) {
            self.advance(); // ARROW (=>)
        } else {
            self.error("Expected '=>' in lambda expression after parameter".to_string());
        }

        // Parse the body expression (using parse_expression which includes ternary).
        self.skip_trivia();
        self.parse_expression();

        self.finish_node(); // LAMBDA
    }

    pub(super) fn parse_lambda_expr(&mut self) {
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
}
