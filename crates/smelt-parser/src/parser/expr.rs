//! Expression grammar.
//!
//! Covers:
//! - the full precedence ladder (`parse_expression` → `parse_pipe_expr` →
//!   `parse_or_expr` → … → `parse_unary_expr` → `parse_primary_expr`),
//! - primary forms: literals, identifiers, function calls, parens,
//!   subqueries, array/struct/row literals, CASE, CAST, EXTRACT,
//! - call-related sub-grammars: arg lists, named params, WITHIN GROUP,
//!   FILTER,
//! - subscript / slice on primary expressions,
//! - VALUES rows and FETCH FIRST clauses,
//! - window specifications (OVER, PARTITION BY, frame & frame bounds),
//! - small lookahead helpers used to disambiguate keywords-as-identifiers
//!   and generic type expressions in call argument positions.

use crate::SyntaxKind::*;

impl<'a> super::Parser<'a> {
    pub(super) fn parse_expression(&mut self) {
        self.start_node(EXPRESSION);
        self.skip_trivia();

        if self.too_deep() {
            self.finish_node();
            return;
        }
        self.depth += 1;
        // Phase F (meta-language): ternary `if COND then THEN else ELSE` is the
        // lowest-precedence meta-language construct — lower than `|>`.
        // The full expression entry point is `parse_expression_inner` which handles
        // both the pure-pipe case and the pipe-then-ternary case.
        self.parse_expression_inner();
        self.depth -= 1;

        self.finish_node();
    }

    /// Inner entry point for expressions. Handles the precedence hierarchy:
    ///   ternary (lowest) > pipe > or > and > comparison > concat > ... > primary (highest)
    ///
    /// When the first token is `if`, the whole expression is a TERNARY_EXPR.
    /// In all other cases this is a pass-through to `parse_pipe_expr`.
    ///
    /// The ternary syntax is `if COND then THEN else ELSE`. Inside the COND slot,
    /// pipe expressions parse with their normal (higher) precedence, so
    /// `if xs |> f() then a else b` correctly makes `xs |> f()` the COND.
    pub(super) fn parse_expression_inner(&mut self) {
        let checkpoint = self.builder.checkpoint();

        if self.at(IF_KW) {
            // Ternary expression: `if COND then THEN else ELSE`.
            self.parse_ternary_from_if(checkpoint);
        } else {
            // Non-ternary expression — pass through to pipe.
            self.parse_pipe_expr();
        }
    }

    /// Parse a full ternary expression starting at the `if` keyword.
    ///
    /// The `checkpoint` must be taken *before* any tokens of this ternary are consumed
    /// (i.e. before the `if` token). This allows the TERNARY_EXPR node to span from
    /// the `if` through the `else` branch.
    ///
    /// When called for a nested ternary (right-associative `else if`), the caller
    /// passes a fresh checkpoint taken at the `if` position.
    fn parse_ternary_from_if(&mut self, checkpoint: rowan::Checkpoint) {
        self.start_node_at(checkpoint, TERNARY_EXPR);
        self.advance(); // IF_KW
        self.skip_trivia();

        // COND slot — wrapped in EXPRESSION.
        self.start_node(EXPRESSION);
        self.depth += 1;
        if self.is_fn_lambda_start() {
            self.parse_fn_lambda();
        } else {
            self.parse_pipe_expr();
        }
        self.depth -= 1;
        self.finish_node(); // EXPRESSION (COND)

        // `then` keyword.
        self.skip_trivia();
        if self.at(THEN_KW) {
            self.advance();
        } else {
            self.error("Expected 'then' in ternary expression".to_string());
        }

        // THEN_EXPR slot — wrapped in EXPRESSION.
        self.skip_trivia();
        self.start_node(EXPRESSION);
        self.depth += 1;
        if self.is_fn_lambda_start() {
            self.parse_fn_lambda();
        } else {
            self.parse_pipe_expr();
        }
        self.depth -= 1;
        self.finish_node(); // EXPRESSION (THEN_EXPR)

        // `else` keyword + ELSE_EXPR.
        self.skip_trivia();
        if self.at(ELSE_KW) {
            self.advance();
            self.skip_trivia();
            // ELSE_EXPR slot — wrapped in EXPRESSION for uniform structure.
            // Right-associative: if the else branch starts with `if`, it recurses.
            self.start_node(EXPRESSION);
            self.depth += 1;
            if self.at(IF_KW) {
                // Right-associative nested ternary.
                self.parse_ternary_from_if(self.builder.checkpoint());
            } else if self.is_fn_lambda_start() {
                self.parse_fn_lambda();
            } else {
                self.parse_pipe_expr();
            }
            self.depth -= 1;
            self.finish_node(); // EXPRESSION (ELSE_EXPR)
        }
        // Missing else → TernaryDanglingElse diagnostic (Phase 3).

        self.finish_node(); // TERNARY_EXPR
    }

    /// Parse a pipe expression: `EXPR |> EXPR |> ...` (left-associative, lowest
    /// precedence).  When no `|>` follows, this is a pass-through to
    /// `parse_or_expr`.  Each `|>` pair is wrapped in a `PIPE_EXPR` node whose
    /// two `EXPRESSION` children are the LHS and RHS.
    ///
    /// Phase B (meta-language): `|>` is meta-world only; Phase 3 rejects pipe
    /// in Data-World positions.  The parser does not gate — it produces the CST
    /// node unconditionally.
    pub(super) fn parse_pipe_expr(&mut self) {
        // Scope the `in_at_time_zone_operand` guard to the current nesting
        // level. The guard exists only to keep a *directly chained* `AT TIME
        // ZONE` out of the tz operand — that chain path runs through
        // `parse_unary_expr` → `parse_primary_expr` and never enters this
        // function, so clearing here cannot break left-associative chaining.
        // Every nested sub-context — parenthesized group, subquery select
        // item, function-call argument, CASE WHEN/THEN/ELSE arm, ternary
        // slot, list spread, bracket list element — funnels through
        // `parse_pipe_expr` (directly or via `parse_expression`), and each
        // starts a fresh expression where `AT TIME ZONE` must parse normally
        // again (e.g. `ts AT TIME ZONE (b AT TIME ZONE 'UTC')` or
        // `ts AT TIME ZONE CASE WHEN p THEN b AT TIME ZONE 'UTC' ELSE c END`,
        // both of which DuckDB's parser accepts).
        let saved_in_at_time_zone_operand = self.in_at_time_zone_operand;
        self.in_at_time_zone_operand = false;

        self.parse_pipe_expr_impl();

        self.in_at_time_zone_operand = saved_in_at_time_zone_operand;
    }

    fn parse_pipe_expr_impl(&mut self) {
        // Use a checkpoint so we can wrap the already-parsed LHS inside
        // a PIPE_EXPR node when we encounter `|>`.
        let checkpoint = self.builder.checkpoint();

        // Handle `fn` at the top level of an expression — a lambda that
        // appears outside a function argument list (Phase 3 will reject with
        // `LambdaInForbiddenPosition`).
        if self.is_fn_lambda_start() {
            self.parse_fn_lambda();
        } else {
            self.parse_or_expr();
        }

        // Fold left-associative pipe operators.
        // Guard: when inside a pipe SQL stage body, `|>` is the next stage
        // delimiter — do NOT consume it as a meta-language pipe operator.
        while self.at(PIPE_ARROW) && !self.in_pipe_stage {
            self.start_node_at(checkpoint, PIPE_EXPR);
            // Wrap the already-parsed LHS in an EXPRESSION node.  The builder
            // checkpoint mechanism means the LHS tokens are already consumed;
            // we're retroactively wrapping them.
            self.advance(); // consume `|>`
            self.skip_trivia();

            // Parse the RHS as a new EXPRESSION child of PIPE_EXPR.
            // The parser does not validate that RHS is a call — Phase 3 does.
            self.start_node(EXPRESSION);
            self.depth += 1;
            // RHS may itself be a lambda (unusual but syntactically valid for
            // error recovery); otherwise it's a normal expression.
            if self.is_fn_lambda_start() {
                self.parse_fn_lambda();
            } else {
                self.parse_or_expr();
            }
            self.depth -= 1;
            self.finish_node(); // EXPRESSION (RHS)

            self.finish_node(); // PIPE_EXPR
        }
    }

    pub(super) fn parse_or_expr(&mut self) {
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

    pub(super) fn parse_and_expr(&mut self) {
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

    pub(super) fn parse_comparison_expr(&mut self) {
        let checkpoint = self.builder.checkpoint();
        self.parse_collate_expr();

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
                    self.parse_collate_expr();
                }
                self.finish_node();
            } else if self.at(IS_KW) {
                // IS [NOT] NULL  |  IS [NOT] DISTINCT FROM expr
                self.start_node_at(checkpoint, BINARY_EXPR);
                self.advance(); // consume IS
                self.skip_trivia();
                if self.at(NOT_KW) {
                    self.advance(); // consume NOT
                    self.skip_trivia();
                }
                if self.at(NULL_KW) {
                    self.advance(); // consume NULL
                } else if self.at(DISTINCT_KW) {
                    // Null-safe comparison: `a IS [NOT] DISTINCT FROM b`.
                    self.advance(); // consume DISTINCT
                    self.skip_trivia();
                    self.expect(FROM_KW);
                    self.skip_trivia();
                    self.parse_collate_expr(); // right operand
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
            } else if self.at(LIKE_KW) || self.at(ILIKE_KW) || self.at(GLOB_KW) {
                // LIKE / ILIKE / GLOB pattern. DuckDB rejects `NOT GLOB` (verified
                // via the oracle), matching the existing NOT LIKE limitation, so
                // there is no dedicated NOT-prefixed form here.
                self.start_node_at(checkpoint, BINARY_EXPR);
                self.advance(); // consume LIKE/ILIKE/GLOB
                self.skip_trivia();
                self.parse_collate_expr();
                self.finish_node();
            } else {
                break;
            }
        }
    }

    /// Parse a COLLATE expression: `expr COLLATE collation_name`.
    ///
    /// COLLATE binds tighter than comparison operators (=, <, >, LIKE, etc.)
    /// but looser than concatenation (||) and lower, so `a COLLATE c = b`
    /// groups as `(a COLLATE c) = b`.
    fn parse_collate_expr(&mut self) {
        let checkpoint = self.builder.checkpoint();
        self.parse_concat_expr();

        self.skip_trivia();
        if self.at(COLLATE_KW) {
            self.start_node_at(checkpoint, COLLATE_EXPR);
            self.advance(); // consume COLLATE_KW
            self.skip_trivia();
            // Expect IDENT or STRING for the collation name.
            if self.at(IDENT) || self.at(STRING) {
                self.advance(); // consume the collation name token
            } else {
                self.error("Expected collation name after COLLATE".to_string());
            }
            self.finish_node(); // COLLATE_EXPR
        }
    }

    /// Parse the body of a BETWEEN expression (BETWEEN low AND high).
    /// Caller is responsible for creating the BETWEEN_EXPR node with the left operand.
    pub(super) fn parse_between_body(&mut self) {
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
    pub(super) fn parse_in_body(&mut self) {
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

                // List spread in IN list: `x IN (...ids)`
                if self.at(DOT_DOT_DOT) {
                    self.parse_list_spread();
                } else {
                    self.parse_expression();
                }

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

    pub(super) fn parse_concat_expr(&mut self) {
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

    pub(super) fn parse_json_expr(&mut self) {
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

    pub(super) fn parse_additive_expr(&mut self) {
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

    pub(super) fn parse_multiplicative_expr(&mut self) {
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

    pub(super) fn parse_unary_expr(&mut self) {
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

    pub(super) fn parse_primary_expr(&mut self) {
        self.skip_trivia();

        // Take a checkpoint BEFORE the if-else chain so that the postfix
        // MAP_METHOD_CALL loop below can retroactively wrap the entire primary
        // (SMELT_PATH_CALL, FUNCTION_CALL, etc.) inside a MAP_METHOD_CALL node
        // when the primary is followed by `.method()` with a known Map API name.
        // This is safe for NULL_KW (which returns early) because NULL cannot be
        // a Map receiver.
        let primary_checkpoint = self.builder.checkpoint();

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
        } else if self.at(LBRACE) {
            // Phase 2 (meta-language): record literal `{key: value, ...}` or
            // Phase 35: brace-struct literal `{expr AS alias, ..spread}`.
            // Disambiguate by peeking: if `{` is followed by `IDENT COLON`,
            // it's a record literal; otherwise it's a brace-struct literal.
            if self.is_record_literal_start() {
                self.parse_record_literal();
            } else {
                self.parse_brace_struct_literal();
            }
        } else if self.at(CASE_KW) {
            self.parse_case_expr();
        } else if self.at(CAST_KW) || self.at(TRY_CAST_KW) {
            self.parse_cast_expr();
        } else if self.at(EXTRACT_KW) {
            self.parse_extract_expr();
        } else if self.at(EXISTS_KW) {
            self.parse_exists_expr();
        } else if self.at(LPAREN) {
            // Could be: parenthesized expression, subquery, or empty tuple `()`.
            let checkpoint = self.builder.checkpoint();
            self.advance(); // consume LPAREN
            self.skip_trivia();

            // Check if it's a subquery (starts with SELECT, or WITH for a
            // CTE-scoped subquery, or is itself a nested parenthesized
            // query — e.g. `((SELECT 2) UNION SELECT 2)`, whose set-op
            // left operand is parenthesized).
            if self.at(SELECT_KW) || self.at(WITH_KW) {
                self.start_node_at(checkpoint, SUBQUERY);
                self.parse_select_stmt();
                self.skip_trivia();
                self.expect(RPAREN);
                self.finish_node();
            } else if self.at(LPAREN) && self.at_parenthesized_query_start() {
                self.start_node_at(checkpoint, SUBQUERY);
                self.parse_query_expr();
                self.skip_trivia();
                self.expect(RPAREN);
                self.finish_node();
            } else if self.at(RPAREN) {
                // Empty tuple `()` — valid as a default value for
                // `SelectItems<Kind>` parameters (Phase 22, research §6).
                // Emit a bare EXPRESSION node with no children; the type-checker
                // treats this as an empty fragment (no aggregates / columns).
                self.start_node_at(checkpoint, EXPRESSION);
                self.advance(); // consume RPAREN
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
        } else if self.at(IDENT) && self.is_map_literal_start() {
            // DuckDB MAP {…} literal: contextual `MAP` identifier directly
            // followed by `{`. `MAP` is otherwise a plain identifier — this
            // guard only fires on `IDENT("MAP") LBRACE`, so `MAP(a, b)` calls
            // and a bare column named `map` are unaffected.
            self.parse_map_literal();
        } else if self.at(IDENT) && self.is_typed_literal() {
            // Typed literal: DATE '2024-01-01', TIMESTAMP '...', etc.
            // Wrap in EXPRESSION so Expr::cast() works
            self.start_node(EXPRESSION);
            self.advance(); // type keyword (IDENT)
            self.skip_trivia();
            self.advance(); // string literal
            self.finish_node();
        } else if self.at(IDENT) && self.is_numeric_interval() {
            // Numeric INTERVAL forms: `INTERVAL 1 DAY`, `INTERVAL (n) DAY`.
            // (The `n * INTERVAL 1 DAY` form falls out of binary-expression
            // grammar once this primary parses correctly.)
            self.start_node(EXPRESSION);
            self.advance(); // INTERVAL keyword
            self.skip_trivia();
            if self.at(LPAREN) {
                // INTERVAL (expr) unit-kw
                self.advance(); // (
                self.skip_trivia();
                self.parse_expression();
                self.skip_trivia();
                self.expect(RPAREN);
            } else {
                // INTERVAL number unit-kw
                self.advance(); // NUMBER literal
            }
            self.skip_trivia();
            if self.at(IDENT) {
                self.advance(); // unit keyword (DAY, MONTH, YEAR, HOUR, …)
            }
            self.finish_node();
        } else if self.at(IDENT) && self.at_smelt_as_struct_trigger() {
            // smelt.as_struct(alias [EXCEPT col1, col2]) — Phase 38.
            // Must be checked BEFORE the generic IDENT branch.
            self.parse_smelt_as_struct();
        } else if self.at(IDENT) && self.at_smelt_fn_trigger() {
            // Phase 5b: reject legacy smelt.fn.* call syntax in expression
            // position. Emit an error pointing the user toward the unified
            // `smelt.functions.*` form, then parse the call as a generic
            // FUNCTION_CALL for error recovery.
            self.error("smelt.fn.* is removed; use smelt.functions.<name> instead".to_string());
            // Consume smelt . fn . <name> and then the arg list as a FUNCTION_CALL
            // for error recovery so parsing can continue.
            let checkpoint = self.builder.checkpoint();
            self.advance(); // consume "smelt"
            self.skip_trivia();
            self.advance(); // consume "."
            self.skip_trivia();
            self.advance(); // consume "fn"
                            // Consume any remaining path segments
            loop {
                self.skip_trivia();
                if !self.at(DOT) {
                    break;
                }
                let mut la = 1;
                while let Some(t) = self.tokens.get(self.pos + la) {
                    if t.kind.is_trivia() {
                        la += 1;
                    } else {
                        break;
                    }
                }
                if self.tokens.get(self.pos + la).map(|t| t.kind) == Some(IDENT) {
                    self.advance(); // DOT
                    self.skip_trivia();
                    self.advance(); // IDENT
                } else {
                    break;
                }
            }
            self.skip_trivia();
            self.start_node_at(checkpoint, FUNCTION_CALL);
            if self.at(LPAREN) {
                self.parse_arg_list();
            }
            self.finish_node();
        } else if self.at(IDENT) && self.at_smelt_legacy_ref_or_source_trigger() {
            // Phase 4: reject legacy smelt.ref() and smelt.source() in
            // expression position. Emit the error and fall through to the
            // generic IDENT → FUNCTION_CALL path for error recovery.
            self.error(
                "smelt.ref() and smelt.source() are removed; \
                 use smelt.models.<name> or smelt.sources.<schema>.<table> instead"
                    .to_string(),
            );
            // The IDENT path below will consume the tokens and produce a
            // FUNCTION_CALL node so parsing continues without a hard stop.
            let checkpoint = self.builder.checkpoint();
            self.advance(); // consume "smelt"
            self.skip_trivia();
            self.advance(); // consume DOT
            self.skip_trivia();
            self.advance(); // consume "ref" or "source"
            self.skip_trivia();
            // Now parse the argument list as part of the FUNCTION_CALL.
            self.start_node_at(checkpoint, FUNCTION_CALL);
            self.parse_arg_list();
            self.finish_node();
        } else if self.at(IDENT) && self.at_smelt_path_trigger() {
            // smelt.<path> value/call form (smelt.<path> migration, Phase 1).
            // Must be checked BEFORE the generic IDENT branch — the generic
            // namespaced-call path would consume only `smelt.<seg>` and leave
            // any further segments dangling. The trigger excludes the legacy
            // second-segments (`fn`, `define`, `extern`, `as_struct`, `ref`,
            // `source`, `metric`) so existing grammar paths stay intact.
            self.parse_smelt_path_form();
        } else if self.at(IDENT) {
            // Could be column reference, qualified name, or function call
            let checkpoint = self.builder.checkpoint();
            // Peek at the identifier text before consuming, so we can detect `reduce`.
            let ident_text = self.current_text().to_string();
            self.advance(); // consume first IDENT
            self.skip_trivia();

            if self.at(LPAREN) {
                // Special case: `reduce(xs, reducer)` — the second argument gets
                // REDUCER_CALL treatment when it is an `IDENT (` call form.
                if ident_text == "reduce" {
                    self.start_node_at(checkpoint, FUNCTION_CALL);
                    self.parse_reduce_arg_list();
                    self.finish_node();
                    // Check for OVER clause (window function on reduce — rare but valid structurally)
                    self.skip_trivia();
                    if self.at(OVER_KW) {
                        self.parse_window_spec();
                    }
                } else {
                    // Simple function call: func()
                    self.start_node_at(checkpoint, FUNCTION_CALL);
                    // TRIM/SUBSTRING/POSITION additionally accept SQL-standard
                    // keyword-argument forms (`TRIM(BOTH … FROM …)`,
                    // `SUBSTRING(x FROM i FOR n)`, `POSITION(sub IN str)`);
                    // dispatch to their dedicated arg-list parsers, which also
                    // handle the regular comma-separated call forms.
                    if ident_text.eq_ignore_ascii_case("trim") {
                        self.parse_trim_arg_list();
                    } else if ident_text.eq_ignore_ascii_case("substring") {
                        self.parse_substring_arg_list();
                    } else if ident_text.eq_ignore_ascii_case("position") {
                        self.parse_position_arg_list();
                    } else {
                        self.parse_arg_list();
                    }
                    self.parse_within_group_if_present();
                    self.parse_filter_clause_if_present();
                    self.finish_node();

                    // Check for OVER clause (window function)
                    self.skip_trivia();
                    if self.at(OVER_KW) {
                        self.parse_window_spec();
                    }
                }
            } else if self.at(DOT) {
                // Could be table.column, namespace.func(), or map.method()
                self.advance(); // consume DOT
                self.skip_trivia();
                // Peek at the method name before consuming, so we can decide
                // whether to emit MAP_METHOD_CALL vs FUNCTION_CALL.
                let method_name = if self.at(IDENT) {
                    Some(self.current_text().to_string())
                } else {
                    None
                };
                self.expect(IDENT); // consume second IDENT
                self.skip_trivia();

                if self.at(LPAREN) {
                    // Determine whether to emit MAP_METHOD_CALL or FUNCTION_CALL.
                    // MAP_METHOD_CALL is used for known Map<K,V> API method names.
                    // This is parser-level minimal: type inference (Phase 4) validates
                    // that the LHS is actually Map<K,V>.
                    let is_map_method = method_name
                        .as_deref()
                        .map(Self::is_map_method_name)
                        .unwrap_or(false);
                    if is_map_method {
                        self.start_node_at(checkpoint, MAP_METHOD_CALL);
                        self.parse_arg_list();
                        self.finish_node();
                    } else {
                        // Namespaced function call (e.g. schema.func())
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
                    }
                } else {
                    // Qualified name (table.column) — wrap in EXPRESSION
                    self.start_node_at(checkpoint, EXPRESSION);
                    self.finish_node();
                }
            } else if self.at(LBRACE) && self.is_record_literal_start() {
                // Named record literal: `TypeName { field: value, … }`.
                // The checkpoint includes the leading IDENT (the type name), so
                // the RECORD_LITERAL node spans `TypeName { … }` in full.
                self.start_node_at(checkpoint, RECORD_LITERAL);
                self.parse_record_literal_body();
                self.finish_node(); // RECORD_LITERAL
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
        } else if self.at(LBRACKET) {
            // Bracket-only list literal: `[a, b, c]` — Phase 1 meta-language.
            // The same surface token lifts to either a meta List<T> or a
            // Data-World Array<U>; the type checker disambiguates (Phase 2/3).
            // We reuse the ARRAY_LITERAL CST kind per the spec.
            self.parse_bracket_list_literal();
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

        // Postfix: Map method call — `expr.method(args)` where `method` is a
        // known Map API name (entries, keys, values, get, has).
        //
        // This fires for any primary expression — most importantly
        // SMELT_PATH_CALL receivers like
        //   `smelt.config.load_yaml('p', Map<Text, S>).keys()`.
        //
        // The IDENT-receiver case (`m.entries()`) is handled inside the IDENT
        // branch above, which already consumed the DOT before reaching this
        // loop, so `self.at(DOT)` is false there — no double-parsing.
        loop {
            self.skip_trivia();
            if !self.at(DOT) {
                break;
            }
            // Peek: DOT must be followed (past trivia) by a map-method IDENT
            // and then LPAREN.  Only commit to MAP_METHOD_CALL if all three are
            // present; otherwise leave the DOT for the caller (e.g. a qualified
            // table.column reference in a larger expression context).
            if !self.peek_dot_map_method_call() {
                break;
            }
            // Commit: retroactively wrap the entire parsed primary expression
            // (from primary_checkpoint) together with the DOT + IDENT + args
            // into a MAP_METHOD_CALL node.
            self.start_node_at(primary_checkpoint, MAP_METHOD_CALL);
            self.advance(); // consume DOT
            self.skip_trivia();
            self.advance(); // consume method-name IDENT
            self.skip_trivia();
            self.parse_arg_list();
            self.finish_node(); // MAP_METHOD_CALL
        }

        // Postfix: PostgreSQL-style cast `expr::type` for non-IDENT primaries.
        //
        // The IDENT branch already handles `ident::type` by checking for
        // DOUBLE_COLON inside the branch and consuming it there.  All other
        // primary expression kinds (NUMBER, STRING, parenthesized expressions,
        // subqueries, CAST(…) results, etc.) reach this postfix loop so that
        // `1.0::DOUBLE`, `'txt'::TEXT`, `(expr)::TYPE`, etc. are handled
        // uniformly.  Each iteration wraps everything from `primary_checkpoint`
        // onwards into a CAST_EXPR node (same retroactive-wrap pattern used by
        // the MAP_METHOD_CALL loop above).
        self.skip_trivia();
        while self.at(DOUBLE_COLON) {
            self.start_node_at(primary_checkpoint, CAST_EXPR);
            self.advance(); // consume ::
            self.skip_trivia();
            self.parse_type_spec();
            self.finish_node(); // CAST_EXPR
            self.skip_trivia();
        }

        // Postfix: `expr AT TIME ZONE tz_expr` (PostgreSQL/DuckDB timezone
        // conversion). `AT`, `TIME`, `ZONE` are contextual keywords (lexed as
        // IDENT) — `peek_at_time_zone` verifies the full three-token sequence
        // before committing, so a bare `AT` (e.g. an implicit select-item
        // alias, `SELECT ts at FROM t`) is left untouched. Binds tighter than
        // arithmetic/comparison (verified via the DuckDB oracle: `ts AT TIME
        // ZONE 'UTC' + INTERVAL 1 HOUR` groups as `(ts AT TIME ZONE 'UTC') +
        // INTERVAL 1 HOUR`) and left-associates so it chains: `ts AT TIME
        // ZONE 'UTC' AT TIME ZONE 'EST'` groups as `(ts AT TIME ZONE 'UTC')
        // AT TIME ZONE 'EST'` (also verified via the oracle). The right-hand
        // timezone operand parses at `parse_unary_expr` level (a bare
        // string/identifier/parenthesized expr, per the oracle), so a
        // trailing lower-precedence operator (`+`, comparison, …) is left for
        // the outer expression rather than swallowed into the tz operand.
        self.skip_trivia();
        while !self.in_at_time_zone_operand && self.peek_at_time_zone() {
            self.start_node_at(primary_checkpoint, AT_TIME_ZONE_EXPR);
            self.advance(); // consume AT
            self.skip_trivia();
            self.advance(); // consume TIME
            self.skip_trivia();
            self.advance(); // consume ZONE
            self.skip_trivia();
            // Parse the timezone operand with the guard set so a chained `AT
            // TIME ZONE` is left for *this* loop (left-associative), not
            // swallowed into the operand (see `in_at_time_zone_operand` doc).
            let was_in_operand = self.in_at_time_zone_operand;
            self.in_at_time_zone_operand = true;
            self.parse_unary_expr();
            self.in_at_time_zone_operand = was_in_operand;
            self.finish_node(); // AT_TIME_ZONE_EXPR
            self.skip_trivia();
        }
    }

    pub(super) fn parse_array_subscript(&mut self) {
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

    pub(super) fn parse_fetch_clause(&mut self) {
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

    pub(super) fn parse_values_clause(&mut self) {
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
                    // List spread in VALUES row: `VALUES (...vals)`
                    if self.at(DOT_DOT_DOT) {
                        self.parse_list_spread();
                    } else {
                        self.parse_expression();
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

    pub(super) fn parse_array_literal(&mut self) {
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

    /// Parse a bracket-only list literal: `[a, b, c]` (Phase 1 meta-language),
    /// or a DuckDB list comprehension: `[expr FOR ident IN list (IF cond)?]`.
    ///
    /// Reuses the `ARRAY_LITERAL` CST kind for the plain-list form — the type
    /// checker distinguishes meta `List<T>` from Data-World `Array<U>` in a
    /// later phase. The comprehension form is a dedicated `LIST_COMPREHENSION`
    /// node (see its doc comment in `syntax_kind.rs`).
    ///
    /// Features:
    /// - Trailing comma allowed: `[a, b, c,]`
    /// - Singleton: `[x]`
    /// - Empty: `[]`
    /// - Nested: `[[1, 2], [3, 4]]`
    /// - Spread elements: `[...xs, a]`
    /// - Error recovery: unterminated `[a, b` does not crash the parser.
    /// - List comprehension: `[x + 1 FOR x IN [1, 2, 3]]`, optionally filtered
    ///   with `[x FOR x IN [1, 2, 3] IF x > 1]`. `FOR` is only recognized
    ///   immediately after the first (and only) element expression — a
    ///   comma-separated list never switches into comprehension form, matching
    ///   DuckDB (`[1, 2 FOR x IN y]` is a syntax error there too).
    pub(super) fn parse_bracket_list_literal(&mut self) {
        let checkpoint = self.builder.checkpoint();
        self.advance(); // consume `[`
        self.skip_trivia();

        if self.at(RBRACKET) || self.at(EOF) {
            // Empty list literal: `[]`.
            self.start_node_at(checkpoint, ARRAY_LITERAL);
            if self.at(RBRACKET) {
                self.advance();
            } else {
                self.error("Expected `]` to close list literal".to_string());
            }
            self.finish_node(); // ARRAY_LITERAL
            return;
        }

        // Parse the first element (spread or expression) before deciding
        // whether this is a plain list literal or a comprehension.
        if self.at(DOT_DOT_DOT) {
            self.parse_list_spread();
        } else {
            self.parse_expression();
        }
        self.skip_trivia();

        if self.is_comprehension_for_start() {
            self.start_node_at(checkpoint, LIST_COMPREHENSION);
            self.parse_list_comprehension_tail();
            self.finish_node(); // LIST_COMPREHENSION
            return;
        }

        self.start_node_at(checkpoint, ARRAY_LITERAL);
        loop {
            if self.at(COMMA) {
                self.advance();
                self.skip_trivia();
                // Allow trailing comma — stop if the next token closes the list.
                if self.at(RBRACKET) || self.at(EOF) {
                    break;
                }
                if self.at(DOT_DOT_DOT) {
                    self.parse_list_spread();
                } else {
                    self.parse_expression();
                }
                self.skip_trivia();
            } else {
                break;
            }
        }

        if self.at(RBRACKET) {
            self.advance(); // consume `]`
        } else {
            self.error("Expected `]` to close list literal".to_string());
        }

        self.finish_node(); // ARRAY_LITERAL
    }

    /// Check whether the current token is the contextual `FOR` keyword that
    /// starts a list comprehension tail. `FOR` is not a reserved keyword —
    /// it is lexed as a plain `IDENT` and only recognized here, immediately
    /// after a bracket list literal's first element expression.
    pub(super) fn is_comprehension_for_start(&self) -> bool {
        self.at(IDENT) && self.current_text().eq_ignore_ascii_case("FOR")
    }

    /// Parse the tail of a list comprehension after the element expression
    /// has already been parsed: `FOR ident IN list (IF cond)? ]`.
    ///
    /// Verified against DuckDB: exactly one `FOR … IN …` clause is accepted
    /// (chained `FOR x IN a FOR y IN b` is a syntax error there), and `IF`
    /// may only appear after the IN-list expression.
    fn parse_list_comprehension_tail(&mut self) {
        self.advance(); // consume `FOR` (contextual IDENT)
        self.skip_trivia();

        self.start_node(LIST_COMPREHENSION_VAR);
        if self.at(IDENT) {
            self.advance();
        } else {
            self.error(
                "Expected loop variable identifier after FOR in list comprehension".to_string(),
            );
        }
        self.finish_node(); // LIST_COMPREHENSION_VAR
        self.skip_trivia();

        if self.at(IN_KW) {
            self.advance();
        } else {
            self.error("Expected IN after loop variable in list comprehension".to_string());
        }
        self.skip_trivia();
        self.parse_expression(); // source list expression
        self.skip_trivia();

        if self.at(IF_KW) {
            self.advance();
            self.skip_trivia();
            self.parse_expression(); // filter condition
            self.skip_trivia();
        }

        if self.at(RBRACKET) {
            self.advance();
        } else {
            self.error("Expected `]` to close list comprehension".to_string());
        }
    }

    /// Parse a list spread: `...expr`.
    ///
    /// Produces a `LIST_SPREAD` CST node wrapping the operand expression.
    /// Valid in any comma-separated grammar position (SELECT list, GROUP BY,
    /// ORDER BY, function args, IN-list, VALUES rows, list-literal elements).
    /// Forbidden-position validation is the type-checker's job (Phase 3).
    ///
    /// The operand is parsed at the pipe level so that `...xs |> map(f)` parses
    /// as `...(xs |> map(f))` — spread is the outermost operator.
    pub(super) fn parse_list_spread(&mut self) {
        self.start_node(LIST_SPREAD);
        self.advance(); // consume `...` (DOT_DOT_DOT)
        self.skip_trivia();
        self.parse_pipe_expr();
        self.finish_node(); // LIST_SPREAD
    }

    pub(super) fn parse_row_constructor(&mut self) {
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

    pub(super) fn parse_struct_literal(&mut self) {
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

    pub(super) fn parse_case_expr(&mut self) {
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

    pub(super) fn parse_when_clause(&mut self) {
        self.start_node(WHEN_CLAUSE);
        self.expect(WHEN_KW);

        // Parse condition (full expression including OR/AND and pipe for searched CASE).
        // Use parse_pipe_expr so that `|>` inside WHEN conditions produces a PIPE_EXPR
        // CST node; Phase 3 then emits PipeInDataPosition for the semantic error.
        self.skip_trivia();
        self.parse_pipe_expr();

        // Expect THEN
        self.skip_trivia();
        if !self.expect(THEN_KW) {
            self.error("Expected THEN in WHEN clause".to_string());
        }

        // Parse result expression (full expression, WHEN/ELSE/END terminate naturally).
        // Use parse_pipe_expr so that `|>` inside THEN results produces a PIPE_EXPR node.
        self.skip_trivia();
        self.parse_pipe_expr();

        self.finish_node();
    }

    /// Parse `CAST(expr AS type)` or the DuckDB error-tolerant `TRY_CAST(expr AS
    /// type)`. Both share the `CAST_EXPR` node kind; a `TRY_CAST` is distinguished
    /// by the leading `TRY_CAST_KW` token (see `CastExpr::is_try_cast`).
    pub(super) fn parse_cast_expr(&mut self) {
        self.start_node(CAST_EXPR);
        if self.at(TRY_CAST_KW) {
            self.advance(); // TRY_CAST
        } else {
            self.expect(CAST_KW);
        }

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
    pub(super) fn parse_extract_expr(&mut self) {
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
    pub(super) fn parse_type_spec(&mut self) {
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

    pub(super) fn parse_subquery(&mut self) {
        self.start_node(SUBQUERY);
        self.parse_select_stmt();
        self.finish_node();
    }

    pub(super) fn parse_exists_expr(&mut self) {
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

    pub(super) fn parse_arg_list(&mut self) {
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

        // Null-treatment modifier: `IGNORE NULLS` / `RESPECT NULLS` inside the
        // argument list (e.g. `last_value(a IGNORE NULLS)`). This is a generic
        // call clause, not keyed to any specific function name.
        self.parse_null_treatment_if_present();

        self.expect(RPAREN);
        self.finish_node();
    }

    /// Look ahead from the current position (which must be immediately after
    /// the `(` of a call's argument list) for a top-level `FROM` keyword
    /// before the matching `)`, without consuming any tokens. Depth tracks
    /// nested parens so a `FROM` inside a nested call/subquery doesn't count.
    ///
    /// Used only to disambiguate the SQL-standard `TRIM([BOTH|LEADING|TRAILING]
    /// … FROM …)` form from a plain call whose sole/first argument happens to
    /// be a bare identifier that reads like a trim-modifier keyword (`BOTH`,
    /// `LEADING`, `TRAILING` are contextual, not reserved, keywords).
    fn peek_top_level_from_before_close_paren(&self) -> bool {
        let mut depth: i32 = 0;
        let mut idx = self.pos;
        while let Some(tok) = self.tokens.get(idx) {
            match tok.kind {
                LPAREN => depth += 1,
                RPAREN => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                FROM_KW if depth == 0 => return true,
                _ => {}
            }
            idx += 1;
        }
        false
    }

    /// Parse the argument list of a `TRIM(...)` call.
    ///
    /// Handles both the regular comma-separated form (`TRIM(x)`,
    /// `TRIM(x, chars)`) and the SQL-standard keyword form
    /// `TRIM([BOTH|LEADING|TRAILING] [chars] FROM string)` / `TRIM(FROM
    /// string)`, which DuckDB also accepts (closes the `duckdb_trim_modifier`
    /// gap in `crates/smelt-parser-compat/src/gaps.rs`). The value
    /// sub-expressions are parsed as ordinary `EXPRESSION` nodes (via
    /// `parse_expression`), in source order, so `FunctionCall::arguments()`
    /// sees them exactly as it would for a plain call — the modifier/`FROM`
    /// keyword tokens are plain children of `ARG_LIST`, not wrapped in an
    /// argument node, so type inference (which reuses the existing
    /// TRIM/SUBSTRING/POSITION function-call typing) is unaffected.
    pub(super) fn parse_trim_arg_list(&mut self) {
        self.start_node(ARG_LIST);
        self.expect(LPAREN);
        self.skip_trivia();

        let at_modifier_kw = self.at_contextual_keyword("BOTH")
            || self.at_contextual_keyword("LEADING")
            || self.at_contextual_keyword("TRAILING");
        let is_std_form =
            self.at(FROM_KW) || (at_modifier_kw && self.peek_top_level_from_before_close_paren());

        if is_std_form {
            if at_modifier_kw {
                self.advance(); // consume BOTH / LEADING / TRAILING
                self.skip_trivia();
            }
            if !self.at(FROM_KW) {
                self.parse_expression(); // optional trim characters
                self.skip_trivia();
            }
            if !self.expect(FROM_KW) {
                self.error("Expected FROM in TRIM expression".to_string());
            }
            self.skip_trivia();
            self.parse_expression(); // string to trim
            self.skip_trivia();
        } else if !self.at(RPAREN) {
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

        self.parse_null_treatment_if_present();
        self.expect(RPAREN);
        self.finish_node();
    }

    /// Parse the argument list of a `SUBSTRING(...)` call.
    ///
    /// Handles the regular comma-separated form (`SUBSTRING(x)`,
    /// `SUBSTRING(x, start[, len])`) and the SQL-standard keyword form
    /// `SUBSTRING(x FROM start [FOR len])` (also `SUBSTRING(x FOR len)`,
    /// which DuckDB accepts with an implied start of 1) — closes the
    /// `duckdb_substring_from_for` gap. `FROM`/`FOR` are unambiguous once the
    /// first argument has been parsed: no other grammar construct expects a
    /// bare `FROM` or contextual `FOR` immediately after a call argument, so
    /// no lookahead is needed (unlike TRIM's modifier keywords).
    pub(super) fn parse_substring_arg_list(&mut self) {
        self.start_node(ARG_LIST);
        self.expect(LPAREN);
        self.skip_trivia();

        if !self.at(RPAREN) {
            self.parse_argument(); // string expression
            self.skip_trivia();

            if self.at(FROM_KW) {
                self.advance();
                self.skip_trivia();
                self.parse_expression(); // start position
                self.skip_trivia();
                if self.at_contextual_keyword("FOR") {
                    self.advance();
                    self.skip_trivia();
                    self.parse_expression(); // length
                    self.skip_trivia();
                }
            } else if self.at_contextual_keyword("FOR") {
                self.advance();
                self.skip_trivia();
                self.parse_expression(); // length (implied start of 1)
                self.skip_trivia();
            } else {
                while self.at(COMMA) {
                    self.advance();
                    self.skip_trivia();
                    self.parse_argument();
                    self.skip_trivia();
                }
            }
        }

        self.parse_null_treatment_if_present();
        self.expect(RPAREN);
        self.finish_node();
    }

    /// Parse the argument list of a `POSITION(...)` call.
    ///
    /// Handles the SQL-standard keyword form `POSITION(sub IN string)` —
    /// closes the `duckdb_position_in` gap. DuckDB does not accept a
    /// comma-separated `POSITION(sub, string)` form (use `STRPOS` for that),
    /// so no comma fallback is provided here; a plain `IDENT(...)` call with
    /// commas still reaches this function but will error on the unexpected
    /// `,`/trailing tokens the same way it would for any other malformed
    /// call, which is acceptable since DuckDB itself rejects that form.
    ///
    /// The left operand is parsed at `parse_concat_expr` precedence (below
    /// the comparison-operator level that owns `IN`) — parsing it via the
    /// full `parse_expression` would let the comparison-expression loop
    /// greedily consume the `IN` as the start of an `IN (values…)` clause
    /// and then fail expecting `(`. This mirrors how `BETWEEN`'s bounds are
    /// parsed at `parse_additive_expr` to avoid `AND` ambiguity.
    pub(super) fn parse_position_arg_list(&mut self) {
        self.start_node(ARG_LIST);
        self.expect(LPAREN);
        self.skip_trivia();

        if !self.at(RPAREN) {
            self.parse_concat_expr(); // substring to search for
            self.skip_trivia();
            if !self.expect(IN_KW) {
                self.error("Expected IN in POSITION expression".to_string());
            }
            self.skip_trivia();
            self.parse_expression(); // string to search within
            self.skip_trivia();
        }

        self.parse_null_treatment_if_present();
        self.expect(RPAREN);
        self.finish_node();
    }

    /// Parse an optional `IGNORE NULLS` / `RESPECT NULLS` null-treatment clause.
    ///
    /// `IGNORE` / `RESPECT` are contextual keywords (lexed as `IDENT`), so this
    /// never reserves either word as an identifier; `NULLS` is the existing
    /// `NULLS_KW` keyword. Emitted as a `NULL_TREATMENT_CLAUSE` child so it is a
    /// structured, generic call modifier (like `FILTER` / `WITHIN GROUP`).
    pub(super) fn parse_null_treatment_if_present(&mut self) {
        self.skip_trivia();
        if self.at_contextual_keyword("IGNORE") || self.at_contextual_keyword("RESPECT") {
            self.start_node(NULL_TREATMENT_CLAUSE);
            self.advance(); // IGNORE / RESPECT
            self.skip_trivia();
            self.expect(NULLS_KW);
            self.finish_node();
        }
    }

    /// Parse the argument list of a `reduce(collection, reducer)` call.
    ///
    /// This is a specialised version of `parse_arg_list` that gives the SECOND argument
    /// (the reducer) special treatment: if the second argument is an `IDENT (` call form
    /// (e.g. `concat_with(' OR ')`), it is wrapped in a `REDUCER_CALL` node instead of
    /// a generic `FUNCTION_CALL`. A bare identifier (e.g. `and_all`) is parsed as a
    /// normal expression (no `REDUCER_CALL` node).
    ///
    /// For all other argument positions (first argument, extra arguments beyond two),
    /// this falls back to `parse_argument()`.
    pub(super) fn parse_reduce_arg_list(&mut self) {
        self.start_node(ARG_LIST);
        self.expect(LPAREN);
        self.skip_trivia();

        if !self.at(RPAREN) {
            // First argument: the collection — parse normally.
            self.parse_argument();
            self.skip_trivia();

            if self.at(COMMA) {
                self.advance();
                self.skip_trivia();

                // Second argument: the reducer.
                // Detect `IDENT (` — parameterised reducer call.
                if self.at(IDENT) && self.is_ident_followed_by_lparen() {
                    // Parameterised reducer: `concat_with(' OR ')`, `join_with(', ')`, etc.
                    self.start_node(REDUCER_CALL);
                    self.advance(); // IDENT (reducer name)
                    self.skip_trivia();
                    self.parse_arg_list(); // the reducer's own argument list
                    self.finish_node(); // REDUCER_CALL
                } else {
                    // Bare reducer: `and_all`, `or_all`, etc. — parse as a normal expression.
                    self.parse_argument();
                }

                self.skip_trivia();
                // Any remaining arguments beyond the second.
                while self.at(COMMA) {
                    self.advance();
                    self.skip_trivia();
                    if self.at(RPAREN) {
                        break;
                    }
                    self.parse_argument();
                    self.skip_trivia();
                }
            }
        }

        self.expect(RPAREN);
        self.finish_node();
    }

    /// Check if the current IDENT is immediately followed by `(` (with optional trivia).
    fn is_ident_followed_by_lparen(&self) -> bool {
        debug_assert!(
            self.at(IDENT),
            "is_ident_followed_by_lparen requires current == IDENT"
        );
        let mut la = 1;
        while let Some(t) = self.tokens.get(self.pos + la) {
            if t.kind.is_trivia() {
                la += 1;
            } else {
                break;
            }
        }
        matches!(self.tokens.get(self.pos + la).map(|t| t.kind), Some(LPAREN))
    }

    /// Parse optional WITHIN GROUP clause for ordered-set aggregate functions
    /// WITHIN GROUP (ORDER BY expr)
    pub(super) fn parse_within_group_if_present(&mut self) {
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
    pub(super) fn parse_filter_clause_if_present(&mut self) {
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
    /// Check if current keyword is followed by LBRACKET (skipping trivia)
    pub(super) fn is_keyword_followed_by_lbracket(&self) -> bool {
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

    /// Check if the current token is the reserved keyword `fn` (FN_KW) AND is
    /// followed by a valid lambda parameter form.
    ///
    /// Accepted forms (Phase F):
    ///   - `fn IDENT => body`        — single-arg bare form
    ///   - `fn ( IDENT ... ) => body` — parenthesised single- or multi-arg form
    ///
    /// Rejected forms:
    ///   - `fn(args)` — FN_KW immediately followed by LPAREN with NO space and no ARROW
    ///     is a regular function call (e.g. `fn(x, y)` where `fn` is a SQL function name).
    ///
    /// Disambiguation for `fn (`:
    ///   - `fn (a, b) => body` — parenthesised lambda start: FN_KW followed by
    ///     `( IDENT ... ) =>`.  We do a lookahead to find `=>` (ARROW) after the `)`.
    ///   - `fn(x, y)` — function call: FN_KW immediately followed by LPAREN (no space)
    ///     or the paren-group does not end with `=>`.
    ///
    /// In practice: `fn LPAREN` is a lambda start only when the parenthesised list
    /// closes with `)` followed (with optional trivia) by `=>` (ARROW).
    pub(super) fn is_fn_lambda_start(&self) -> bool {
        if !self.at(FN_KW) {
            return false;
        }
        // Skip the FN_KW token and any trivia to see what follows.
        let mut la = 1;
        while let Some(t) = self.tokens.get(self.pos + la) {
            if t.kind.is_trivia() {
                la += 1;
            } else {
                break;
            }
        }
        match self.tokens.get(self.pos + la).map(|t| t.kind) {
            // `fn <IDENT>` — single-arg bare lambda: `fn x => body`.
            Some(IDENT) => true,
            // `fn ( ... ) => body` — parenthesised lambda form (Phase F).
            // Disambiguate from `fn(args)` function call by scanning for `)` then `=>`.
            Some(LPAREN) => self.is_fn_paren_lambda_start(la),
            // Anything else is NOT a lambda.
            _ => false,
        }
    }

    /// Lookahead: given that we are at `fn` and position `lparen_la` points to `(`,
    /// determine whether this is a parenthesised lambda `fn ( ... ) => body` vs a
    /// function call `fn(...)`.
    ///
    /// Returns `true` if and only if the LPAREN group is followed by `=>` (ARROW),
    /// i.e. the form is `fn ( IDENT* ) =>`.
    fn is_fn_paren_lambda_start(&self, lparen_la: usize) -> bool {
        // Scan forward past the LPAREN, skipping any IDENT and COMMA tokens,
        // to find the matching RPAREN and then check for ARROW.
        let mut la = lparen_la + 1; // skip LPAREN
        let mut depth = 1usize;
        while let Some(t) = self.tokens.get(self.pos + la) {
            match t.kind {
                k if k.is_trivia() => {
                    la += 1;
                }
                LPAREN => {
                    depth += 1;
                    la += 1;
                }
                RPAREN => {
                    depth -= 1;
                    la += 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {
                    la += 1;
                }
            }
        }
        // Skip trivia after RPAREN.
        while let Some(t) = self.tokens.get(self.pos + la) {
            if t.kind.is_trivia() {
                la += 1;
            } else {
                break;
            }
        }
        // Must be `=>` (ARROW) for this to be a lambda.
        matches!(self.tokens.get(self.pos + la).map(|t| t.kind), Some(ARROW))
    }

    /// Check if the current IDENT starts a generic type expression like
    /// `List<T>`, `Map<K, V>`, `List<{field: Type}>`, etc.
    ///
    /// Returns `true` when:
    ///   1. The next non-trivia token is `<` (LT), AND
    ///   2. The token after that `<` is an IDENT or `{` (LBRACE).
    ///
    /// This heuristic distinguishes `List<Cohort>` (generic type) from
    /// `x < 5` (comparison where the RHS is a literal) and allows the
    /// parser to route generic-type arguments in smelt path call arg lists
    /// through `parse_record_field_type_ref` instead of `parse_expression`.
    ///
    /// The only false-positive risk is `a < b` where `b` is an IDENT (e.g.
    /// a column comparison like `price < threshold`). In smelt loader schema
    /// argument positions this never occurs; comparisons with IDENT RHS are
    /// exceedingly rare in function argument positions in practice.
    pub(super) fn is_generic_type_start(&self) -> bool {
        debug_assert!(
            self.at(IDENT),
            "is_generic_type_start requires current == IDENT"
        );
        // Skip past the current IDENT and any trivia to find the `<`.
        let mut la = 1;
        while let Some(t) = self.tokens.get(self.pos + la) {
            if t.kind.is_trivia() {
                la += 1;
            } else {
                break;
            }
        }
        if self.tokens.get(self.pos + la).map(|t| t.kind) != Some(LT) {
            return false;
        }
        // Skip past `<` and any trivia to find what's inside the angle brackets.
        la += 1;
        while let Some(t) = self.tokens.get(self.pos + la) {
            if t.kind.is_trivia() {
                la += 1;
            } else {
                break;
            }
        }
        matches!(
            self.tokens.get(self.pos + la).map(|t| t.kind),
            Some(IDENT) | Some(LBRACE)
        )
    }

    /// Check if current keyword is followed by LPAREN (skipping trivia)
    pub(super) fn is_keyword_followed_by_lparen(&self) -> bool {
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
    pub(super) fn at_keyword_as_function_name(&self) -> bool {
        if !self.at_any(&[
            FILTER_KW, QUALIFY_KW, PIVOT_KW, UNPIVOT_KW, VALUES_KW, LEFT_KW, RIGHT_KW,
            // Phase B: FN_KW — `fn(args)` where `fn` is used as a SQL function name.
            // is_fn_lambda_start() excludes LPAREN already, so fn(args) reaches here.
            FN_KW,
            // GLOB is a comparison operator keyword, but DuckDB also ships a
            // glob(pattern) file-listing function. When directly followed by
            // `(` it is a function name (LEFT()/RIGHT() precedent); otherwise
            // it is the infix operator.
            GLOB_KW,
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

    /// Check if current IDENT is the contextual `MAP` keyword immediately
    /// followed by `{` (skipping trivia). DuckDB's `MAP {'a': 1, 'b': 2}`
    /// literal syntax. `MAP` is not reserved: `MAP(a, b)` function calls and
    /// a bare identifier `map` still parse as before, since this only
    /// matches when `{` follows.
    pub(super) fn is_map_literal_start(&self) -> bool {
        let token = self.tokens[self.pos];
        let text = &self.input[self.offset..self.offset + token.len];
        if !text.eq_ignore_ascii_case("MAP") {
            return false;
        }
        let mut lookahead = 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                return t.kind == LBRACE;
            }
        }
        false
    }

    /// Parse a DuckDB MAP literal: `MAP {key: value, …}`.
    ///
    /// Items are `MAP_ENTRY` nodes, each wrapping a key `Expr` and a value
    /// `Expr` separated by `:`. Keys are full expressions (typically string
    /// or numeric literals), unlike `RECORD_LITERAL`'s bare-IDENT keys.
    /// Empty `MAP {}` and a trailing comma are both accepted (verified
    /// against DuckDB).
    pub(super) fn parse_map_literal(&mut self) {
        self.start_node(MAP_LITERAL);
        self.advance(); // consume `MAP` (IDENT)
        self.skip_trivia();
        self.expect(LBRACE);

        loop {
            self.skip_trivia();
            if self.at(RBRACE) || self.at(EOF) {
                break;
            }

            self.start_node(MAP_ENTRY);
            self.parse_expression(); // key
            self.skip_trivia();
            if self.at(COLON) {
                self.advance(); // COLON
                self.skip_trivia();
                if !self.at_any(&[COMMA, RBRACE, EOF]) {
                    self.parse_expression(); // value
                } else {
                    self.error(
                        "Expected expression value after ':' in MAP literal entry".to_string(),
                    );
                }
            } else {
                self.error("Expected ':' after key in MAP literal entry".to_string());
                self.sync_to(&[COMMA, RBRACE, EOF]);
            }
            self.finish_node(); // MAP_ENTRY

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
                self.skip_trivia();
                // Trailing comma allowed.
                if self.at(RBRACE) {
                    break;
                }
            } else {
                break;
            }
        }

        self.skip_trivia();
        if self.at(RBRACE) {
            self.advance(); // `}`
        } else {
            self.error("Expected '}' to close MAP literal".to_string());
        }

        self.finish_node(); // MAP_LITERAL
    }

    /// Check if current IDENT is a type keyword followed by a string literal (e.g., DATE '2024-01-01')
    pub(super) fn is_typed_literal(&self) -> bool {
        // Check if current IDENT is a type keyword
        let token = self.tokens[self.pos];
        let text = &self.input[self.offset..self.offset + token.len];
        let upper = text.to_uppercase();
        if !matches!(
            upper.as_str(),
            "DATE" | "TIME" | "TIMESTAMP" | "TIMESTAMPTZ" | "INTERVAL"
        ) {
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

    /// Check if current IDENT is `INTERVAL` followed by a NUMBER or `(` (numeric forms).
    /// Covers: `INTERVAL 1 DAY`, `INTERVAL (n) DAY`, and the RHS of `n * INTERVAL 1 DAY`.
    pub(super) fn is_numeric_interval(&self) -> bool {
        let token = self.tokens[self.pos];
        let text = &self.input[self.offset..self.offset + token.len];
        if !text.eq_ignore_ascii_case("INTERVAL") {
            return false;
        }
        let mut la = 1;
        while let Some(t) = self.tokens.get(self.pos + la) {
            if t.kind.is_trivia() {
                la += 1;
            } else {
                return matches!(t.kind, NUMBER | LPAREN);
            }
        }
        false
    }
    // ===== Phase 12: Window Function Support =====

    pub(super) fn parse_window_spec(&mut self) {
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

    pub(super) fn parse_partition_by(&mut self) {
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

    pub(super) fn parse_window_frame(&mut self) {
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

    pub(super) fn parse_frame_bound(&mut self) {
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
        } else if self.at(IDENT) && self.is_typed_literal() {
            // INTERVAL '…' PRECEDING / FOLLOWING — time-based RANGE frame bound
            // (the spec's Form A lookback declaration; DuckDB executes it natively).
            self.advance(); // INTERVAL keyword (IDENT)
            self.skip_trivia();
            self.advance(); // string literal
            self.skip_trivia();
            if self.at(PRECEDING_KW) || self.at(FOLLOWING_KW) {
                self.advance();
            } else {
                self.error("Expected PRECEDING or FOLLOWING after INTERVAL literal".to_string());
            }
        } else {
            self.error(
                "Expected frame bound (UNBOUNDED, CURRENT ROW, number, or INTERVAL literal)"
                    .to_string(),
            );
        }

        self.finish_node();
    }
}
