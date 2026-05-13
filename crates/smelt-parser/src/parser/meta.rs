//! Meta-language (Phase B+) grammar — lambdas.
//!
//! Covers the `fn IDENT => EXPR` reserved-keyword lambda introduced in
//! Phase B and the legacy single/multi-param `x -> EXPR` / `(x, y) -> EXPR`
//! lambdas. Lookahead helpers (`is_lambda_single_param`, `is_thin_arrow_at`,
//! `is_lambda_multi_param`) live alongside the productions they gate
//! (`parse_fn_lambda`, `parse_lambda_expr`).

use super::Parser;
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
    /// Parse a Phase B meta-language lambda: `fn IDENT => EXPR` (single-arg only).
    ///
    /// Multi-arg lambdas (`fn (IDENT, ...) => EXPR`) are reserved for Phase F;
    /// `is_fn_lambda_start()` does not route LPAREN cases here in Phase B.
    ///
    /// Produces a `LAMBDA` node whose children are:
    ///   - `FN_KW` token (the reserved `fn` keyword)
    ///   - `LAMBDA_PARAM_LIST` — a single IDENT for the parameter
    ///   - `ARROW` token (`=>`)
    ///   - `EXPRESSION` — the lambda body
    ///
    /// The caller must have verified `self.is_fn_lambda_start()` before calling
    /// this.  `fn` is a **reserved** keyword (FN_KW); any SQL using `fn` as a
    /// column, table, or alias name must now quote it.
    pub(super) fn parse_fn_lambda(&mut self) {
        self.start_node(LAMBDA);

        // Consume the `fn` reserved keyword (FN_KW).
        self.advance(); // FN_KW
        self.skip_trivia();

        // Parse the parameter list.
        if self.at(LPAREN) {
            // Multi-arg lambda: `fn (a, b) => body` — Phase 3 rejects, parser accepts.
            self.start_node(LAMBDA_PARAM_LIST);
            self.advance(); // LPAREN
            self.skip_trivia();
            loop {
                if self.at(IDENT) {
                    self.advance(); // parameter IDENT
                    self.skip_trivia();
                }
                if self.at(COMMA) {
                    self.advance(); // ,
                    self.skip_trivia();
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
            // Single-arg lambda: `fn IDENT => body`.
            self.start_node(LAMBDA_PARAM_LIST);
            self.advance(); // IDENT
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

        // Parse the body expression.
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
