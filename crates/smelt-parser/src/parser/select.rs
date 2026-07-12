//! `SELECT` statement grammar.
//!
//! Covers the full SELECT statement including:
//! - `SELECT` list (DISTINCT/ALL, items, wildcards)
//! - `FROM` / `JOIN` / `LATERAL` / `PIVOT` / `UNPIVOT` / `TABLESAMPLE`
//! - `WHERE`, `GROUP BY`, `HAVING`, `QUALIFY`
//! - `ORDER BY`, `LIMIT` / `OFFSET` / `FETCH FIRST`
//! - `WITH` clause / CTEs (including RECURSIVE and column lists)
//! - `UNION` / `INTERSECT` / `EXCEPT` (set-op tails on a select stmt)

use crate::SyntaxKind::*;

impl<'a> super::Parser<'a> {
    pub(super) fn parse_select_stmt(&mut self) {
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

        // WINDOW clause (after QUALIFY, before ORDER BY)
        self.skip_trivia();
        if self.at(WINDOW_KW) {
            self.parse_window_clause();
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
            // Optional BY NAME (DuckDB): unify operands by column name
            // rather than position. `BY` is the reserved `BY_KW` (shared
            // with `GROUP BY`/`ORDER BY`); `NAME` is a contextual keyword
            // (lexed as plain IDENT), matched only in this exact sequence
            // so it stays an ordinary identifier everywhere else
            // (`SELECT name FROM t`).
            if self.at(BY_KW)
                && self
                    .peek_nth_non_trivia_text(1)
                    .is_some_and(|t| t.eq_ignore_ascii_case("NAME"))
            {
                self.advance(); // BY
                self.skip_trivia();
                self.advance(); // NAME
                self.skip_trivia();
            }
            // Parse next operand: a plain SELECT/WITH statement, or a
            // parenthesized query — `A UNION (B)`, `A UNION ((B) EXCEPT C)`,
            // arbitrarily nested. The parenthesized form recurses through
            // `parse_query_expr`, so it carries its own set-op tail and
            // ORDER BY/LIMIT inside the parens.
            if self.at(SELECT_KW) || self.at(WITH_KW) || self.at(LPAREN) {
                self.parse_query_expr();
            } else {
                self.error("Expected SELECT after set operation".to_string());
            }
        }

        self.depth -= 1;
        self.finish_node();
    }

    /// Parses a full query expression: either a plain `SELECT`/`WITH`
    /// statement (`parse_select_stmt`, which already handles its own
    /// UNION/INTERSECT/EXCEPT tail and ORDER BY/LIMIT), or a parenthesized
    /// query (`parse_parenthesized_query`). Used wherever a complete query
    /// is expected as a unit: set-operation operands, and (via
    /// `parse_parenthesized_query`) nested parenthesized queries at any
    /// depth.
    pub(super) fn parse_query_expr(&mut self) {
        self.skip_trivia();
        if self.at(LPAREN) {
            self.parse_parenthesized_query();
        } else {
            self.parse_select_stmt();
        }
    }

    /// Parses `( <query> )`, wrapping the whole thing (including the
    /// parens) in a `SUBQUERY` node. The inner query is parsed via
    /// `parse_query_expr`, so `((( SELECT … )))`-style redundant nesting
    /// and a parenthesized query with its own trailing ORDER BY/LIMIT or
    /// set-op tail both work.
    pub(super) fn parse_parenthesized_query(&mut self) {
        self.start_node(SUBQUERY);
        self.advance(); // consume LPAREN
        self.skip_trivia();
        self.parse_query_expr();
        self.skip_trivia();
        self.expect(RPAREN);
        self.finish_node();
    }

    /// Lookahead-only: true if, starting at the current position, the
    /// token stream is zero or more `LPAREN`s (skipping trivia) followed
    /// directly by `SELECT_KW` or `WITH_KW`.
    ///
    /// Distinguishes a (possibly redundantly parenthesized) query —
    /// `(SELECT …)`, `((SELECT …))`, … — from a plain grouped expression
    /// that merely happens to start with nested parens, e.g. `((1))` or
    /// `((a + b))`. Callers that already know the current token opens a
    /// query context (e.g. the operand slot right after UNION/EXCEPT/
    /// INTERSECT, where nothing but a query can legally appear) don't need
    /// this guard; it exists for positions — a general expression primary,
    /// a FROM-clause table reference — where a bare `(` is ambiguous
    /// between "grouped expression" and "parenthesized query".
    pub(super) fn at_parenthesized_query_start(&self) -> bool {
        let mut n = 0usize;
        loop {
            match self.peek_nth_non_trivia(n) {
                Some(LPAREN) => n += 1,
                Some(SELECT_KW) | Some(WITH_KW) => return true,
                _ => return false,
            }
        }
    }

    pub(super) fn parse_select_list(&mut self) {
        self.start_node(SELECT_LIST);
        self.skip_trivia();

        // Parse comma-separated select items (including * and table.*)
        loop {
            if self.at(DOT_DOT_DOT) {
                // List spread in SELECT list: `...metric_exprs`
                self.parse_list_spread();
            } else if self.at(STAR) {
                // Handle SELECT * as a special select item
                self.start_node(SELECT_ITEM);
                self.advance();
                self.finish_node();
            } else if self.at(IDENT)
                && self.peek_nth_non_trivia(1) == Some(DOT)
                && self.peek_nth_non_trivia(2) == Some(STAR)
            {
                // `table.*` qualified star. Emit as a SELECT_ITEM carrying
                // IDENT DOT STAR tokens directly — downstream consumers
                // treat `SELECT_ITEM` with a trailing STAR as a star-item.
                // The peek check skipped trivia, so we must do the same
                // between advances or the comment between `.` and `*`
                // would leave STAR outside the SELECT_ITEM.
                self.start_node(SELECT_ITEM);
                self.advance(); // IDENT
                self.skip_trivia();
                self.advance(); // DOT
                self.skip_trivia();
                self.advance(); // STAR
                self.finish_node();
            } else {
                self.parse_select_item();
            }

            self.skip_trivia();
            if self.at(COMMA) {
                // Peek BEFORE consuming the comma: if the token after the comma is
                // `IDENT COLON`, this is a record-literal field boundary
                // (e.g. `owner: 'team'` following `body: SELECT 1, …` inside a
                // `ModelDef { … }`).  In SQL, `IDENT :` is not a valid select-item
                // start; breaking WITHOUT consuming the comma leaves it for the
                // enclosing record-literal parser to use as its field separator.
                if self.peek_nth_non_trivia(1) == Some(IDENT)
                    && self.peek_nth_non_trivia(2) == Some(COLON)
                {
                    break; // leave the COMMA in the stream
                }
                self.advance();
                self.skip_trivia();
                // Allow trailing comma - break if next token ends the SELECT list.
                //
                // Exception: LEFT_KW and RIGHT_KW can be SQL function names (e.g.
                // `LEFT(str, 3)`, `RIGHT(str, 2)`) when followed by `(`.  Only
                // treat them as JOIN/SELECT-list terminators when they are NOT
                // immediately followed by `(` (after trivia).
                let is_join_keyword = (self.at(LEFT_KW) || self.at(RIGHT_KW))
                    && !self.is_keyword_followed_by_lparen();
                if is_join_keyword
                    || self.at_any(&[
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
                        FULL_KW,
                        CROSS_KW,
                        JOIN_KW,
                        UNION_KW,
                        INTERSECT_KW,
                        EXCEPT_KW,
                        RBRACE,
                    ])
                {
                    break;
                }
            } else {
                break;
            }
        }

        self.finish_node();
    }

    pub(super) fn parse_select_item(&mut self) {
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

    pub(super) fn parse_from_clause(&mut self) {
        self.start_node(FROM_CLAUSE);

        self.expect(FROM_KW);

        // Parse first table reference (required)
        self.parse_table_ref();

        // Parse zero or more JOIN clauses
        loop {
            self.skip_trivia();
            if self.at_any(&[JOIN_KW, INNER_KW, LEFT_KW, RIGHT_KW, FULL_KW, CROSS_KW])
                || self.at_contextual_keyword("NATURAL")
            {
                self.parse_join_clause();
            } else {
                break;
            }
        }

        self.finish_node();
    }

    pub(super) fn parse_table_ref(&mut self) {
        self.start_node(TABLE_REF);
        self.skip_trivia();

        // Check for LATERAL keyword (PostgreSQL)
        if self.at(LATERAL_KW) {
            self.advance(); // LATERAL
            self.skip_trivia();
        }

        // Phase 5b: reject legacy smelt.fn.* in FROM position.
        if self.at(IDENT) && self.at_smelt_fn_trigger() {
            self.error("smelt.fn.* is removed; use smelt.functions.<name> instead".to_string());
            // Fall through — the generic IDENT path below handles error recovery.
        }

        // Phase 4: reject legacy smelt.ref() and smelt.source() call forms.
        // Emit an error pointing the user to the unified smelt.<path> form,
        // then parse the call as a generic FUNCTION_CALL for error recovery.
        if self.at(IDENT) && self.at_smelt_legacy_ref_or_source_trigger() {
            self.error(
                "smelt.ref() and smelt.source() are removed; \
                 use smelt.models.<name> or smelt.sources.<schema>.<table> instead"
                    .to_string(),
            );
            // Fall through — the generic IDENT path below will still consume
            // and produce a FUNCTION_CALL node so the rest of the parse continues.
        }

        // smelt.<path> migration, Phase 1: unified value/call form. The
        // trigger excludes the legacy second-segments (`fn`, `define`,
        // `extern`, `as_struct`, `ref`, `source`, `metric`) so the existing
        // grammar paths above and the FUNCTION_CALL path below stay intact.
        if self.at(IDENT) && self.at_smelt_path_trigger() {
            self.parse_smelt_path_form();
            self.skip_trivia();
            if self.at(AS_KW) {
                self.advance();
                self.skip_trivia();
                self.expect(IDENT);
            } else if self.at(IDENT) && !self.at_keyword_that_ends_table_ref() {
                self.advance();
            }
            self.finish_node(); // TABLE_REF
            return;
        }

        if self.at(LPAREN) {
            // Could be a subquery
            let checkpoint = self.builder.checkpoint();
            self.advance(); // consume LPAREN
            self.skip_trivia();

            // Check if it's a subquery (starts with SELECT or WITH, or is
            // itself a nested parenthesized query, e.g. a derived table
            // whose body is `(SELECT …) UNION SELECT …`) or VALUES
            if self.at(SELECT_KW)
                || self.at(WITH_KW)
                || (self.at(LPAREN) && self.at_parenthesized_query_start())
            {
                self.start_node_at(checkpoint, SUBQUERY);
                self.parse_query_expr();
                self.skip_trivia();
                self.expect(RPAREN);
                self.finish_node(); // Close SUBQUERY
            } else if self.at(VALUES_KW) {
                self.start_node_at(checkpoint, SUBQUERY);
                self.parse_values_clause();
                self.skip_trivia();
                self.expect(RPAREN);
                self.finish_node();
            } else if self.at(FROM_KW) {
                // Pipe query used as a parenthesised subquery:
                // `FROM (FROM t |> WHERE p) alias` — spec §"Where a pipe query
                // may appear": "As a subquery or CTE body — anywhere a
                // parenthesised query … is legal."
                self.start_node_at(checkpoint, SUBQUERY);
                self.parse_pipe_query();
                self.skip_trivia();
                self.expect(RPAREN);
                self.finish_node();
            } else if self.at(IDENT) || self.at(LATERAL_KW) || self.at(LPAREN) {
                // Parenthesized table reference or joined-table sequence:
                // `(t1)`, `(t1 JOIN t2 ON …)`, `(t1 NATURAL JOIN t2 NATURAL
                // JOIN t3)` — a table primary DuckDB/PostgreSQL both accept.
                // The nested `parse_table_ref` call recurses through this
                // same LPAREN branch, so arbitrarily nested parenthesized
                // join chains (`(a JOIN b) JOIN (c JOIN d)`) fall out for
                // free. No dedicated node kind: the printer's `TableRef`
                // Display detects a nested `TABLE_REF` child and renders it
                // structurally (see `printer.rs`).
                self.parse_table_ref();
                loop {
                    self.skip_trivia();
                    if self.at_any(&[JOIN_KW, INNER_KW, LEFT_KW, RIGHT_KW, FULL_KW, CROSS_KW])
                        || self.at_contextual_keyword("NATURAL")
                    {
                        self.parse_join_clause();
                    } else {
                        break;
                    }
                }
                self.skip_trivia();
                self.expect(RPAREN);
            } else {
                // Not a subquery, error
                self.error("Expected SELECT in subquery".to_string());
                self.expect(RPAREN);
            }
        } else if self.at(GLOB_KW) && self.is_keyword_followed_by_lparen() {
            // DuckDB's glob(pattern) file-listing table function. GLOB is an
            // operator keyword, but when directly followed by `(` it is a
            // function name (LEFT()/RIGHT() keyword-as-function-name precedent).
            let checkpoint = self.builder.checkpoint();
            self.advance(); // Consume GLOB_KW
            self.skip_trivia();
            self.start_node_at(checkpoint, FUNCTION_CALL);
            self.parse_arg_list();
            self.finish_node(); // Close FUNCTION_CALL
        } else if self.at(RANGE_KW) && self.is_keyword_followed_by_lparen() {
            // DuckDB's range(start, stop[, step]) table-generating
            // function. RANGE is also the window-frame unit keyword
            // (`RANGE BETWEEN …`), but when directly followed by `(` in
            // table-ref position it is unambiguously the table function
            // (GLOB()/LEFT()/RIGHT() keyword-as-function-name precedent).
            let checkpoint = self.builder.checkpoint();
            self.advance(); // Consume RANGE_KW
            self.skip_trivia();
            self.start_node_at(checkpoint, FUNCTION_CALL);
            self.parse_arg_list();
            self.finish_node(); // Close FUNCTION_CALL
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

        // Optional alias, with an optional column list. The alias may be
        // explicit (`AS t`) or implicit (`t`); the `(c1, c2, …)` column list
        // attaches to either form identically.
        self.skip_trivia();
        if self.at(AS_KW) {
            self.advance();
            self.skip_trivia();
            self.expect(IDENT);
            self.parse_alias_column_list();
        } else if self.at(IDENT) && !self.at_keyword_that_ends_table_ref() {
            // Implicit alias (no AS keyword). Only consume if it's not a
            // keyword that would end the table ref.
            self.advance();
            self.parse_alias_column_list();
        }

        self.finish_node();
    }

    /// Parse an optional alias column list `(c1, c2, …)` immediately following
    /// a table alias, emitting an `ALIAS_COLUMN_LIST` node. No-op when the next
    /// non-trivia token is not `LPAREN`. Shared by the explicit (`AS t(…)`) and
    /// implicit (`t(…)`) alias paths.
    fn parse_alias_column_list(&mut self) {
        self.skip_trivia();
        if !self.at(LPAREN) {
            return;
        }
        self.start_node(ALIAS_COLUMN_LIST);
        self.advance(); // consume LPAREN
        self.skip_trivia();
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
        self.finish_node(); // ALIAS_COLUMN_LIST
    }

    pub(super) fn parse_pivot_clause(&mut self) {
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

    pub(super) fn parse_unpivot_clause(&mut self) {
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

    pub(super) fn parse_pivot_in_list(&mut self) {
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
    pub(super) fn parse_join_clause(&mut self) {
        self.start_node(JOIN_CLAUSE);

        // Optional NATURAL modifier (contextual keyword): `NATURAL [INNER |
        // LEFT [OUTER] | RIGHT [OUTER] | FULL [OUTER]] JOIN`. Join columns
        // are all identically named columns between the two sides; smelt's
        // schema inference already unions all columns from every table_ref
        // without deduping by ON/USING (see `process_from_clause_node_pure`
        // in smelt-db), so NATURAL needs no special-cased column-matching
        // logic — it is consistent with the existing (non-deduping)
        // treatment of ON/USING joins.
        if self.at_contextual_keyword("NATURAL") {
            self.advance();
            self.skip_trivia();
        }

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

    pub(super) fn parse_join_condition(&mut self) {
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

    pub(super) fn parse_where_clause(&mut self) {
        self.start_node(WHERE_CLAUSE);
        self.expect(WHERE_KW);
        self.parse_expression();
        self.finish_node();
    }

    pub(super) fn parse_group_by_clause(&mut self) {
        self.start_node(GROUP_BY_CLAUSE);
        self.expect(GROUP_KW);
        self.expect(BY_KW);

        // `GROUP BY ALL` (DuckDB) — group by every non-aggregate select item.
        // The bare `ALL` keyword is kept as a marker token; there are no
        // grouping-key expressions to parse.
        self.skip_trivia();
        if self.at(ALL_KW) {
            self.advance(); // ALL marker
            self.finish_node();
            return;
        }

        // Parse comma-separated column list
        loop {
            self.skip_trivia();
            // List spread in GROUP BY: `GROUP BY ...keys`
            if self.at(DOT_DOT_DOT) {
                self.parse_list_spread();
            } else if self.peek_grouping_sets_clause() {
                // `GROUP BY GROUPING SETS ((a), (b), ())` — may also appear
                // mixed with plain keys: `GROUP BY a, GROUPING SETS ((b))`.
                self.parse_grouping_sets_clause();
            } else {
                self.parse_expression();
            }

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

    /// `GROUPING SETS ( <set> [, <set>]* )`. Caller has already verified
    /// [`peek_grouping_sets_clause`](Self::peek_grouping_sets_clause) — the
    /// exact `GROUPING SETS (` token sequence — so both keywords and the
    /// opening paren are consumed unconditionally here.
    pub(super) fn parse_grouping_sets_clause(&mut self) {
        self.start_node(GROUPING_SETS_CLAUSE);
        self.advance(); // GROUPING (contextual keyword, lexed as IDENT)
        self.skip_trivia();
        self.advance(); // SETS (contextual keyword, lexed as IDENT)
        self.skip_trivia();
        self.expect(LPAREN);

        loop {
            self.skip_trivia();
            self.parse_grouping_set();
            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
                continue;
            }
            break;
        }

        self.skip_trivia();
        self.expect(RPAREN);
        self.finish_node();
    }

    /// One element of a `GROUPING SETS` list: a parenthesized (possibly
    /// empty) comma-separated expression list — `(a, b)`, `()` — or a bare
    /// expression — `a` (PostgreSQL/DuckDB both accept unparenthesized
    /// elements; verified against DuckDB). A nested `ROLLUP(...)`/`CUBE(...)`
    /// call is just a bare expression here — there is no dedicated smelt-side
    /// CUBE/ROLLUP grammar; they already fall out of the generic
    /// function-call parse.
    fn parse_grouping_set(&mut self) {
        self.start_node(GROUPING_SET);
        self.skip_trivia();
        if self.at(LPAREN) {
            self.advance();
            self.skip_trivia();
            if !self.at(RPAREN) {
                loop {
                    self.skip_trivia();
                    self.parse_grouping_set_element();
                    self.skip_trivia();
                    if self.at(COMMA) {
                        self.advance();
                        continue;
                    }
                    break;
                }
                self.skip_trivia();
            }
            self.expect(RPAREN);
        } else {
            self.parse_grouping_set_element();
        }
        self.finish_node();
    }

    /// One element position inside a `GROUPING SETS` list (either the bare
    /// position or one slot of a parenthesized comma list): a nested
    /// `GROUPING SETS (...)` (DuckDB accepts arbitrary nesting — verified
    /// against DuckDB, e.g. `GROUPING SETS (GROUPING SETS ((a)))`), or a
    /// plain expression (covers bare columns and `ROLLUP(...)`/`CUBE(...)`
    /// function calls alike).
    fn parse_grouping_set_element(&mut self) {
        if self.peek_grouping_sets_clause() {
            self.parse_grouping_sets_clause();
        } else {
            self.parse_expression();
        }
    }

    pub(super) fn parse_having_clause(&mut self) {
        self.start_node(HAVING_CLAUSE);
        self.expect(HAVING_KW);
        self.parse_expression();
        self.finish_node();
    }

    pub(super) fn parse_qualify_clause(&mut self) {
        self.start_node(QUALIFY_CLAUSE);
        self.expect(QUALIFY_KW);
        self.parse_expression();
        self.finish_node();
    }

    pub(super) fn parse_window_clause(&mut self) {
        self.start_node(WINDOW_CLAUSE);
        self.expect(WINDOW_KW);

        // Comma-separated named window definitions
        loop {
            self.skip_trivia();
            self.parse_named_window();

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
            } else {
                break;
            }
        }

        self.finish_node();
    }

    fn parse_named_window(&mut self) {
        self.start_node(NAMED_WINDOW);

        // Window name (IDENT)
        if !self.expect(IDENT) {
            self.finish_node();
            return;
        }

        // AS keyword
        self.skip_trivia();
        if !self.expect(AS_KW) {
            self.finish_node();
            return;
        }

        // Window body in parentheses
        self.skip_trivia();
        if !self.expect(LPAREN) {
            self.finish_node();
            return;
        }

        self.skip_trivia();

        // Optional PARTITION BY
        if self.at(PARTITION_KW) {
            self.parse_partition_by();
        }

        // Optional ORDER BY
        self.skip_trivia();
        if self.at(ORDER_KW) {
            self.parse_order_by_clause();
        }

        // Optional window frame (ROWS/RANGE/GROUPS)
        self.skip_trivia();
        if self.at_any(&[ROWS_KW, RANGE_KW, GROUPS_KW]) {
            self.parse_window_frame();
        }

        self.expect(RPAREN);
        self.finish_node();
    }

    pub(super) fn parse_order_by_clause(&mut self) {
        self.start_node(ORDER_BY_CLAUSE);
        self.expect(ORDER_KW);
        self.expect(BY_KW);

        // `ORDER BY ALL` (DuckDB) — order by every select item, left to right.
        // The bare `ALL` keyword is a marker; an optional direction and NULLS
        // ordering may follow (`ORDER BY ALL DESC`, `ORDER BY ALL NULLS LAST`).
        self.skip_trivia();
        if self.at(ALL_KW) {
            self.advance(); // ALL marker
            self.skip_trivia();
            if self.at(ASC_KW) || self.at(DESC_KW) {
                self.advance();
                self.skip_trivia();
            }
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
            return;
        }

        // Comma-separated order items
        loop {
            self.skip_trivia();
            // List spread in ORDER BY: `ORDER BY ...sort_keys`
            if self.at(DOT_DOT_DOT) {
                self.parse_list_spread();
            } else {
                self.parse_order_by_item();
            }

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
            } else {
                break;
            }
        }

        self.finish_node();
    }

    pub(super) fn parse_order_by_item(&mut self) {
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

    pub(super) fn parse_limit_clause(&mut self) {
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
    pub(super) fn parse_with_clause(&mut self) {
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

    pub(super) fn parse_cte(&mut self) {
        self.start_node(CTE);

        // CTE name
        self.skip_trivia();
        if !self.expect(IDENT) {
            self.error("Expected CTE name".to_string());
            self.finish_node();
            return;
        }

        // Optional column list: name(col1, col2)
        // We see LPAREN before AS — this is the column list, not the query.
        self.skip_trivia();
        if self.at(LPAREN) {
            // Peek at the token after LPAREN to decide:
            //   IDENT → column list; SELECT/WITH → should not happen here (query comes after AS)
            // We wrap the column list in ALIAS_COLUMN_LIST.
            self.start_node(ALIAS_COLUMN_LIST);
            self.advance(); // consume LPAREN
            self.skip_trivia();

            if self.at(IDENT) {
                // Parse column list: ident (, ident)*
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
            // For SELECT/WITH or other unexpected tokens, just finish with what we have.
            self.expect(RPAREN);
            self.finish_node(); // ALIAS_COLUMN_LIST
            self.skip_trivia();
        }

        // AS (query)
        if !self.expect(AS_KW) {
            self.error("Expected AS in CTE".to_string());
            self.finish_node();
            return;
        }

        // Optional [NOT] MATERIALIZED hint (DuckDB/PostgreSQL): a purely
        // informational CTE-materialization directive that does not change
        // the CTE's schema or logic, so it needs no dedicated inference
        // support — only round-trip parse/print fidelity. `MATERIALIZED` is
        // a contextual keyword (lexed as a plain IDENT); `NOT` is only
        // consumed here when directly followed by `MATERIALIZED`, so a
        // (syntactically impossible in this position, but defensive) bare
        // `NOT` doesn't get silently swallowed.
        self.skip_trivia();
        if self.at_contextual_keyword("MATERIALIZED") {
            self.advance();
            self.skip_trivia();
        } else if self.at(NOT_KW)
            && self
                .peek_nth_non_trivia_text(1)
                .is_some_and(|t| t.eq_ignore_ascii_case("MATERIALIZED"))
        {
            self.advance(); // NOT
            self.skip_trivia();
            self.advance(); // MATERIALIZED
            self.skip_trivia();
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
        } else if self.at(FROM_KW) {
            // Pipe query as a CTE body:
            // `WITH recent AS (FROM events |> WHERE ts > 0) …`
            // Spec §"Where a pipe query may appear": "As a subquery or CTE body
            // — anywhere a parenthesised query or a WITH CTE body is legal."
            self.start_node(SUBQUERY);
            self.parse_pipe_query();
            self.finish_node();
        } else if self.at_smelt_path_trigger() {
            // smelt.<path> migration, Phase 1: a CTE body is a bare
            // `smelt.<path>(args)` call. Phase 5b removes the smelt.fn.* arm
            // here; use smelt.functions.* instead. The value form is also
            // accepted here even
            // though parameterless table-shaped paths are unusual — the
            // resolver in Phase 2a decides the kind.
            self.start_node(SUBQUERY);
            self.parse_smelt_path_form();
            self.finish_node();
        } else {
            self.error("Expected SELECT, WITH, or VALUES in CTE".to_string());
        }

        self.expect(RPAREN);
        self.finish_node();
    }
}
