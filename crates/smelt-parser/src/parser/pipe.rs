//! Pipe SQL (`|>`) grammar productions.
//!
//! A **pipe query** is a FROM-first query body:
//!
//! ```sql
//! [WITH … AS (…)]
//! FROM <table_ref>
//! |> OPERATOR body
//! |> OPERATOR body
//! …
//! ```
//!
//! Entry point: `parse_pipe_query`. Called from `smelt_ext::parse_file` when
//! the body begins with `FROM_KW` (or `WITH_KW` followed by a FROM-first
//! body — detected by `peek_from_first_after_with`).

use super::Parser;
use crate::SyntaxKind::*;

/// Deferred operators: recognised keywords that are not yet supported.
/// Using one produces `PipeOperatorUnsupported`.
const DEFERRED_OPERATORS: &[(&str, &str)] = &[
    (
        "PIVOT",
        "output columns depend on data values and cannot be determined at compile time",
    ),
    (
        "UNPIVOT",
        "output columns depend on data values and cannot be determined at compile time",
    ),
    (
        "WINDOW",
        "the |> WINDOW operator form is not supported; use window functions inside SELECT/EXTEND",
    ),
    (
        "CALL",
        "table-valued function piping has no end-to-end smelt support",
    ),
    (
        "TABLESAMPLE",
        "sampling is available on a FROM table reference only, not as a pipe stage",
    ),
    (
        "ASSERT",
        "row-level runtime assertions have no smelt equivalent construct",
    ),
];

impl<'a> Parser<'a> {
    /// Parse a FROM-first (pipe) query into a `PIPE_QUERY` node.
    ///
    /// Pre-condition: the current non-trivia token is either `WITH_KW` (WITH +
    /// FROM-first detected by caller) or `FROM_KW`.
    ///
    /// Emits: `PIPE_QUERY { [WITH_CLAUSE] FROM_CLAUSE PIPE_STAGE* }`
    pub(super) fn parse_pipe_query(&mut self) {
        self.start_node(PIPE_QUERY);

        if self.too_deep() {
            self.finish_node();
            return;
        }
        self.depth += 1;

        self.skip_trivia();

        // Optional WITH clause (already consumed by the lookahead but not yet in the tree).
        if self.at(WITH_KW) {
            self.parse_with_clause();
        }

        // FROM entry — required.
        self.skip_trivia();
        self.parse_from_clause();

        // Pipe stages: loop over `|> OPERATOR …` chains.
        loop {
            self.skip_trivia();
            if !self.at(PIPE_ARROW) {
                break;
            }
            self.advance(); // consume `|>`
            self.skip_trivia();

            self.parse_pipe_stage();
        }

        self.depth -= 1;
        self.finish_node();
    }

    /// Parse a single `PIPE_STAGE` after the `|>` has been consumed.
    fn parse_pipe_stage(&mut self) {
        self.start_node(PIPE_STAGE);

        // Dispatch on the current keyword (after `|>` and trivia).
        if self.at(WHERE_KW) {
            self.parse_pipe_where();
        } else if self.at(SELECT_KW) {
            self.parse_pipe_select();
        } else if self.at(DISTINCT_KW) {
            self.parse_pipe_distinct();
        } else if self.at(ORDER_KW) {
            self.parse_pipe_order_by();
        } else if self.at(LIMIT_KW) {
            self.parse_pipe_limit();
        } else if self.at(AS_KW) {
            self.parse_pipe_as();
        } else if self.at_any(&[JOIN_KW, INNER_KW, LEFT_KW, RIGHT_KW, FULL_KW, CROSS_KW]) {
            self.parse_pipe_join();
        } else if self.at_any(&[UNION_KW, INTERSECT_KW, EXCEPT_KW]) {
            self.parse_pipe_set_op();
        } else if self.at(IDENT) {
            // Contextual keywords and deferred operators.
            let text = self.current_text().to_ascii_uppercase();
            match text.as_str() {
                "EXTEND" => self.parse_pipe_extend(),
                "SET" => self.parse_pipe_set(),
                "DROP" => self.parse_pipe_drop(),
                "RENAME" => self.parse_pipe_rename(),
                "AGGREGATE" => self.parse_pipe_aggregate(),
                _ => {
                    // Check for deferred operators.
                    if let Some(reason) = deferred_reason(&text) {
                        // Emit error, consume to next `|>` or EOF for recovery.
                        let msg = format!("pipe operator '{}' is not supported — {}", text, reason);
                        self.error(msg);
                        self.advance(); // consume the deferred keyword
                                        // Consume stage body (skip to next `|>` or EOF).
                        self.skip_to_next_pipe_or_eof();
                    } else {
                        // Unknown operator.
                        let msg = format!("unknown pipe operator '{}'", text);
                        self.error(msg);
                        self.advance(); // consume the unknown keyword
                        self.skip_to_next_pipe_or_eof();
                    }
                }
            }
        } else if self.at_any(&[PIVOT_KW, UNPIVOT_KW, WINDOW_KW, TABLESAMPLE_KW]) {
            // Dedicated-keyword deferred operators (not contextual IDENTs).
            let text = self.current_text().to_ascii_uppercase();
            let reason = deferred_reason(&text).unwrap_or("unsupported pipe operator");
            let msg = format!("pipe operator '{}' is not supported — {}", text, reason);
            self.error(msg);
            self.advance();
            self.skip_to_next_pipe_or_eof();
        } else if self.at(EOF) {
            // Nothing after the last `|>` — error.
            self.error("expected pipe operator after '|>'".to_string());
        } else {
            // Non-IDENT, non-keyword token — unknown operator.
            let text = self.current_text().to_string();
            let msg = format!("unknown pipe operator '{}'", text);
            self.error(msg);
            self.advance();
            self.skip_to_next_pipe_or_eof();
        }

        self.finish_node();
    }

    /// Skip tokens until the next `PIPE_ARROW` or EOF (error recovery).
    fn skip_to_next_pipe_or_eof(&mut self) {
        while !self.at(EOF) && !self.at(PIPE_ARROW) {
            self.advance();
        }
    }

    // ── Per-operator stage parsers ───────────────────────────────────────────

    /// `|> WHERE <predicate>`
    fn parse_pipe_where(&mut self) {
        self.start_node(PIPE_OP_WHERE);
        self.finish_node(); // zero-width marker
        self.advance(); // consume WHERE_KW
        self.skip_trivia();
        // Set in_pipe_stage so that `|>` inside the predicate is not consumed
        // as a meta-language PIPE_EXPR operator — it is the next stage delimiter.
        let prev = self.in_pipe_stage;
        self.in_pipe_stage = true;
        if self.at_expression_start() {
            self.parse_expression();
        } else {
            self.error("malformed 'WHERE' pipe stage: expected predicate expression".to_string());
        }
        self.in_pipe_stage = prev;
    }

    /// `|> SELECT <expr> [AS <alias>], …`
    fn parse_pipe_select(&mut self) {
        self.start_node(PIPE_OP_SELECT);
        self.finish_node(); // zero-width marker
        self.advance(); // consume SELECT_KW
        self.skip_trivia();
        // Set in_pipe_stage so that `|>` inside select items is not consumed
        // as a meta-language PIPE_EXPR — it is the next stage delimiter.
        let prev = self.in_pipe_stage;
        self.in_pipe_stage = true;
        self.parse_select_list();
        self.in_pipe_stage = prev;
    }

    /// `|> EXTEND <expr> AS <alias>, …`
    fn parse_pipe_extend(&mut self) {
        self.start_node(PIPE_OP_EXTEND);
        self.finish_node(); // zero-width marker
        self.advance(); // consume EXTEND ident
        self.skip_trivia();
        let prev = self.in_pipe_stage;
        self.in_pipe_stage = true;
        // Comma-separated `expr AS alias` pairs.
        loop {
            if !self.at_expression_start() {
                break;
            }
            self.parse_expression();
            self.skip_trivia();
            // Optional AS alias
            if self.at(AS_KW) {
                self.advance();
                self.skip_trivia();
                if self.at(IDENT) {
                    self.advance();
                }
            } else if self.at(IDENT) {
                // Implicit alias
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
        self.in_pipe_stage = prev;
    }

    /// `|> SET <col> = <expr>, …`
    fn parse_pipe_set(&mut self) {
        self.start_node(PIPE_OP_SET);
        self.finish_node(); // zero-width marker
        self.advance(); // consume SET ident
        self.skip_trivia();
        let prev = self.in_pipe_stage;
        self.in_pipe_stage = true;
        // Comma-separated `col = expr` pairs.
        loop {
            if !self.at(IDENT) {
                break;
            }
            self.advance(); // column name
            self.skip_trivia();
            if self.at(EQ) {
                self.advance(); // =
                self.skip_trivia();
                self.parse_expression();
            } else {
                self.error("expected '=' in SET pipe stage".to_string());
            }
            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
                self.skip_trivia();
            } else {
                break;
            }
        }
        self.in_pipe_stage = prev;
    }

    /// `|> DROP <col>, …`
    fn parse_pipe_drop(&mut self) {
        self.start_node(PIPE_OP_DROP);
        self.finish_node(); // zero-width marker
        self.advance(); // consume DROP ident
        self.skip_trivia();
        // Comma-separated identifiers.
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
    }

    /// `|> RENAME <old> AS <new>, …`
    fn parse_pipe_rename(&mut self) {
        self.start_node(PIPE_OP_RENAME);
        self.finish_node(); // zero-width marker
        self.advance(); // consume RENAME ident
        self.skip_trivia();
        loop {
            if !self.at(IDENT) {
                break;
            }
            self.advance(); // old name
            self.skip_trivia();
            if self.at(AS_KW) {
                self.advance();
                self.skip_trivia();
                if self.at(IDENT) {
                    self.advance(); // new name
                } else {
                    self.error("expected identifier after AS in RENAME pipe stage".to_string());
                }
            } else {
                self.error("expected AS in RENAME pipe stage".to_string());
            }
            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
                self.skip_trivia();
            } else {
                break;
            }
        }
    }

    /// `|> AS <alias>`
    fn parse_pipe_as(&mut self) {
        self.start_node(PIPE_OP_AS);
        self.finish_node(); // zero-width marker
        self.advance(); // consume AS_KW
        self.skip_trivia();
        if self.at(IDENT) {
            self.advance();
        } else {
            self.error("expected identifier after AS in pipe stage".to_string());
        }
    }

    /// `|> AGGREGATE <agg_expr> [AS <alias>], … [GROUP BY <expr> [AS <alias>], …]`
    fn parse_pipe_aggregate(&mut self) {
        self.start_node(PIPE_OP_AGGREGATE);
        self.finish_node(); // zero-width marker
        self.advance(); // consume AGGREGATE ident
        self.skip_trivia();
        let prev = self.in_pipe_stage;
        self.in_pipe_stage = true;

        // Parse aggregate expressions until GROUP BY or end of stage.
        loop {
            if !self.at_expression_start() {
                break;
            }
            self.parse_expression();
            self.skip_trivia();
            if self.at(AS_KW) {
                self.advance();
                self.skip_trivia();
                if self.at(IDENT) {
                    self.advance();
                }
            } else if self.at(IDENT) && !self.at_contextual_keyword("GROUP") && !self.at(BY_KW) {
                // Implicit alias, but not if it's the GROUP keyword.
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

        // Optional GROUP BY.
        self.skip_trivia();
        if self.at(GROUP_KW) {
            self.advance(); // GROUP
            self.skip_trivia();
            self.expect(BY_KW);
            // Parse group expressions.
            loop {
                self.skip_trivia();
                if !self.at_expression_start() {
                    break;
                }
                self.parse_expression();
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
        }
        self.in_pipe_stage = prev;
    }

    /// `|> ORDER BY <expr> [ASC|DESC] [NULLS …], …`
    fn parse_pipe_order_by(&mut self) {
        self.start_node(PIPE_OP_ORDER_BY);
        self.finish_node(); // zero-width marker
        let prev = self.in_pipe_stage;
        self.in_pipe_stage = true;
        self.parse_order_by_clause();
        self.in_pipe_stage = prev;
    }

    /// `|> LIMIT <n> [OFFSET <m>]`
    fn parse_pipe_limit(&mut self) {
        self.start_node(PIPE_OP_LIMIT);
        self.finish_node(); // zero-width marker
        let prev = self.in_pipe_stage;
        self.in_pipe_stage = true;
        self.parse_limit_clause();
        self.in_pipe_stage = prev;
    }

    /// `|> [join_type] JOIN <table_ref> [ON <cond> | USING (<cols>)]`
    fn parse_pipe_join(&mut self) {
        self.start_node(PIPE_OP_JOIN);
        self.finish_node(); // zero-width marker
        self.parse_join_clause();
    }

    /// `|> DISTINCT`
    fn parse_pipe_distinct(&mut self) {
        self.start_node(PIPE_OP_DISTINCT);
        self.finish_node(); // zero-width marker
        self.advance(); // consume DISTINCT_KW
    }

    /// `|> {UNION|INTERSECT|EXCEPT} {ALL|DISTINCT} (<query>) [, (<query>)…]`
    fn parse_pipe_set_op(&mut self) {
        // Emit the operator-specific marker.
        let marker_kind = if self.at(UNION_KW) {
            PIPE_OP_UNION
        } else if self.at(INTERSECT_KW) {
            PIPE_OP_INTERSECT
        } else {
            PIPE_OP_EXCEPT
        };
        self.start_node(marker_kind);
        self.finish_node(); // zero-width marker

        self.advance(); // consume UNION_KW / INTERSECT_KW / EXCEPT_KW
        self.skip_trivia();

        // Optional ALL / DISTINCT modifier.
        if self.at(ALL_KW) || self.at(DISTINCT_KW) {
            self.advance();
            self.skip_trivia();
        }

        // One or more parenthesised queries.
        loop {
            self.skip_trivia();
            if !self.at(LPAREN) {
                break;
            }
            self.advance(); // (
            self.skip_trivia();
            if self.at(SELECT_KW) || self.at(WITH_KW) {
                self.start_node(SUBQUERY);
                self.parse_select_stmt();
                self.finish_node();
            } else if self.at(FROM_KW) {
                // Nested pipe query as set-op operand.
                self.parse_pipe_query();
            } else {
                self.error("expected SELECT or FROM in set operation operand".to_string());
            }
            self.skip_trivia();
            self.expect(RPAREN);
            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
            } else {
                break;
            }
        }
    }

    /// Peek whether a `WITH` clause at the current position is followed by a
    /// FROM-first (pipe-query) body rather than a SELECT-first body.
    ///
    /// Scans forward from the current position (which must be `WITH_KW`),
    /// tracking paren depth to skip over CTE bodies, and returns `true` if the
    /// first token after all CTEs is `FROM_KW` (pipe) or `false` if it is
    /// `SELECT_KW` (standard SELECT) or anything else.
    pub(super) fn peek_from_first_after_with(&self) -> bool {
        let mut i = self.pos;
        // Skip the WITH keyword itself.
        i += 1;
        let mut depth: usize = 0;
        let total = self.tokens.len();

        // We need to consume: RECURSIVE?, cte_name, optional_col_list, AS, (, body, ), (, …)
        // The simplest robust approach: scan forward skipping parens until we
        // find FROM_KW or SELECT_KW at paren depth 0, skipping WITH's CTE list.
        //
        // State machine:
        //  - we treat the top-level token stream (depth == 0) as the WITH clause.
        //  - once we see a FROM_KW or SELECT_KW at depth == 0 after the WITH, we know.
        //
        // We skip the WITH keyword (already at i=1), then scan:
        while i < total {
            let kind = self.tokens[i].kind;
            if kind.is_trivia() {
                i += 1;
                continue;
            }
            match kind {
                LPAREN => {
                    depth += 1;
                    i += 1;
                }
                RPAREN => {
                    depth = depth.saturating_sub(1);
                    i += 1;
                }
                FROM_KW if depth == 0 => return true,
                SELECT_KW if depth == 0 => return false,
                EOF => return false,
                _ => {
                    i += 1;
                }
            }
        }
        false
    }
}

/// Look up the deferral reason for a contextual or keyword operator name.
fn deferred_reason(name: &str) -> Option<&'static str> {
    for &(op, reason) in DEFERRED_OPERATORS {
        if name.eq_ignore_ascii_case(op) {
            return Some(reason);
        }
    }
    None
}
