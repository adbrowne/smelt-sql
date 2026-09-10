use super::helpers::{extract_group_by_expressions, get_set_operation, SetOperand};
use super::*;
use std::fmt::{self, Display};

// ===== Basic Display implementations =====

impl Display for File {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(stmt) = self.select_stmt() {
            write!(f, "{}", stmt)?;
        } else if let Some(values) = self.values_clause() {
            // No dedicated pretty-printer for a bare top-level VALUES body
            // (same rationale as `TableRef`'s subquery-VALUES fallback):
            // raw text is a faithful, lossless rendering, and it correctly
            // preserves a trailing comma after the last row.
            write!(f, "{}", values.syntax().text())?;
        }
        Ok(())
    }
}

impl Display for SelectStmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // WITH clause
        if let Some(with_clause) = self.with_clause() {
            write!(f, "{} ", with_clause)?;
        }

        // SELECT
        write!(f, "SELECT")?;

        // DISTINCT
        if self.is_distinct() {
            write!(f, " DISTINCT")?;
        }

        // SELECT list
        if let Some(select_list) = self.select_list() {
            write!(f, " {}", select_list)?;
        }

        // FROM clause
        if let Some(from_clause) = self.from_clause() {
            write!(f, " FROM {}", from_clause)?;
        }

        // WHERE clause
        if let Some(where_clause) = self.where_clause() {
            if let Some(expr) = where_clause.expression() {
                write!(f, " WHERE {}", expr.text())?;
            }
        }

        // GROUP BY clause
        if let Some(group_by) = self
            .syntax()
            .children()
            .find(|n| n.kind() == GROUP_BY_CLAUSE)
        {
            write!(f, " GROUP BY {}", extract_group_by_expressions(&group_by))?;
        }

        // HAVING clause
        if let Some(having_clause) = self.having_clause() {
            write!(f, " HAVING {}", having_clause)?;
        }

        // QUALIFY clause
        if let Some(qualify_clause) = self.qualify_clause() {
            write!(f, " QUALIFY {}", qualify_clause)?;
        }

        // WINDOW clause
        if let Some(window_clause) = self.window_clause() {
            write!(f, " WINDOW {}", window_clause)?;
        }

        let order_by_clause = self.order_by_clause();
        let limit_clause = self.limit_clause();
        let set_op = get_set_operation(self.syntax());

        // A trailing ORDER BY/LIMIT parsed *after* a parenthesized set-op
        // operand (`A UNION (B) ORDER BY x`) is a sibling of the set-op
        // keyword that sits later in source order — its own text range
        // starts after the keyword's. Distinguish that from the historical
        // pre-set-op position (a per-operand clause on a SELECT_STMT that
        // also happens to carry a nested set-op tail) so printing doesn't
        // silently move the clause across the UNION and re-attach it to
        // the wrong operand (see `keyword_offset` doc on `SetOperation`).
        let is_trailing = |clause_start: usize| {
            set_op
                .as_ref()
                .is_some_and(|op| clause_start > op.keyword_offset)
        };

        let order_by_is_trailing = order_by_clause
            .as_ref()
            .is_some_and(|c| is_trailing(usize::from(c.syntax().text_range().start())));
        let limit_is_trailing = limit_clause
            .as_ref()
            .is_some_and(|c| is_trailing(usize::from(c.syntax().text_range().start())));

        // ORDER BY clause (pre-set-op position)
        if !order_by_is_trailing {
            if let Some(order_by_clause) = &order_by_clause {
                write!(f, " {}", order_by_clause)?;
            }
        }

        // LIMIT clause (pre-set-op position)
        if !limit_is_trailing {
            if let Some(limit_clause) = &limit_clause {
                write!(f, " {}", limit_clause)?;
            }
        }

        // Set operations: UNION / INTERSECT / EXCEPT
        if let Some(set_op) = set_op {
            write!(f, " {}", set_op.keyword)?;
            if set_op.all {
                write!(f, " ALL")?;
            }
            if set_op.by_name {
                write!(f, " BY NAME")?;
            }
            match set_op.operand {
                SetOperand::Select(select) => write!(f, " {}", select)?,
                SetOperand::Paren(subquery) => write!(f, " {}", subquery)?,
                SetOperand::None => {}
            }
        }

        // Trailing ORDER BY/LIMIT after a parenthesized set-op operand.
        if order_by_is_trailing {
            if let Some(order_by_clause) = &order_by_clause {
                write!(f, " {}", order_by_clause)?;
            }
        }
        if limit_is_trailing {
            if let Some(limit_clause) = &limit_clause {
                write!(f, " {}", limit_clause)?;
            }
        }

        Ok(())
    }
}

impl Display for SelectList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use crate::ast::SelectEntry;
        let mut first = true;
        for entry in self.entries() {
            if !first {
                write!(f, ", ")?;
            }
            first = false;
            match entry {
                SelectEntry::Item(item) => write!(f, "{}", item)?,
                SelectEntry::Spread(spread) => {
                    write!(f, "...")?;
                    if let Some(operand) = spread.operand() {
                        write!(f, "{}", operand.text())?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl Display for SelectItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Print the expression's own source text, not the `Expr::cast`-unwrapped
        // node's — see `SelectItem::expression_source_text`, which keeps a
        // parenthesis wrapper the cast would otherwise strip.
        if let Some(text) = self.expression_source_text() {
            write!(f, "{}", text)?;
        } else {
            // For simple tokens like * that don't have an EXPRESSION wrapper,
            // extract the text directly (excluding AS and alias if present)
            let text = self.syntax().text().to_string();
            if self.alias().is_some() {
                // Remove "AS alias" part
                if let Some(as_pos) = text.to_uppercase().find(" AS ") {
                    write!(f, "{}", text[..as_pos].trim())?;
                } else {
                    write!(f, "{}", crate::ast::trim_source_text(self.syntax()))?;
                }
            } else {
                write!(f, "{}", crate::ast::trim_source_text(self.syntax()))?;
            }
        }

        // Use the raw alias token text (quotes intact for `AS "quoted"`),
        // not `alias()`'s unquoted semantic name — re-emitting an unquoted
        // form for an alias that needs quoting (whitespace, matches a
        // keyword, ...) would print SQL DuckDB/PostgreSQL reject.
        if let Some(alias) = self.alias_token_text() {
            write!(f, " AS {}", alias)?;
        }

        Ok(())
    }
}

impl Display for FromClause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Get first table ref
        let mut table_refs = self.table_refs();
        if let Some(first_table) = table_refs.next() {
            write!(f, "{}", first_table)?;
        }

        // Get all JOINs. A comma-join (`FROM a, b`) prints as `, b` rather
        // than ` JOIN b` — fidelity requires preserving the comma form
        // rather than rewriting it to `CROSS JOIN`.
        for join in self.joins() {
            if join.is_comma_join() {
                write!(f, ", {}", join)?;
            } else {
                write!(f, " {}", join)?;
            }
        }

        Ok(())
    }
}

impl Display for TableRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_lateral() {
            write!(f, "LATERAL ")?;
        }

        if let Some(func_call) = self.function_call() {
            write!(f, "{}", func_call.text())?;
        } else if let Some(subquery) = self.subquery() {
            // Print the parenthesised body via raw source text rather than
            // `Subquery`'s Display impl: that impl only knows how to print a
            // SELECT body (`select_stmt()`) and silently drops VALUES rows
            // (`values_clause()` has no pretty-printer). Raw text is a
            // faithful, lossless rendering of either form and matches the
            // rest of this printer's approach of falling back to source text
            // for constructs without a dedicated Display impl.
            write!(f, "{}", subquery.syntax().text())?;
        } else if let Some(quoted) = self.quoted_identifier_path_text() {
            // Double-quoted table/schema name(s) — re-emit with quotes
            // intact (`identifier()` strips them for resolution callers).
            write!(f, "{}", quoted)?;
        } else if let Some(ident) = self.identifier() {
            write!(f, "{}", ident)?;
        } else if let Some(inner) = self.syntax().children().find_map(TableRef::cast) {
            // Parenthesized table reference or joined-table sequence:
            // `(t1)`, `(t1 NATURAL JOIN t2)`. Printed structurally (rather
            // than via the raw-text fallback below) so a trailing alias on
            // the outer TABLE_REF — printed separately further down — isn't
            // double-printed.
            write!(f, "({}", inner)?;
            for join in self.syntax().children().filter_map(JoinClause::cast) {
                if join.is_comma_join() {
                    write!(f, ", {}", join)?;
                } else {
                    write!(f, " {}", join)?;
                }
            }
            write!(f, ")")?;
        } else {
            write!(f, "{}", self.syntax().text())?;
            // The raw-text fallback above already prints this whole
            // TABLE_REF node's source text verbatim, including any
            // TABLESAMPLE/PIVOT/UNPIVOT clauses and alias it contains —
            // printing them again below would double-print. This branch is
            // believed unreachable now that the subquery/nested/identifier
            // branches above are explicit, but guard defensively (see
            // docs/TODO.md "TABLESAMPLE/PIVOT/UNPIVOT vs alias ordering").
            return Ok(());
        }

        // Raw alias token text (quotes intact for `AS "quoted"`) — see the
        // matching note on `Display for SelectItem`. Printed *before*
        // TABLESAMPLE/PIVOT/UNPIVOT: DuckDB v1.5.4 requires
        // `base AS alias TABLESAMPLE(...)` and rejects the reverse order
        // (oracle-verified). PIVOT/UNPIVOT accept the alias either before or
        // after (also oracle-verified), so printing alias-first here is safe
        // for those too, and keeps a single consistent print order.
        if let Some(alias) = self.alias_token_text().or_else(|| self.alias()) {
            write!(f, " AS {}", alias)?;
            if let Some(cols) = self.alias_column_names() {
                if !cols.is_empty() {
                    write!(f, "({})", cols.join(", "))?;
                }
            }
        }

        // TABLESAMPLE / PIVOT / UNPIVOT clauses; print them verbatim (no
        // dedicated pretty-printer exists for these yet).
        for clause in self
            .syntax()
            .children()
            .filter(|n| matches!(n.kind(), TABLESAMPLE_CLAUSE | PIVOT_CLAUSE | UNPIVOT_CLAUSE))
        {
            write!(f, " {}", clause.text())?;
        }

        Ok(())
    }
}

impl Display for JoinClause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_comma_join() {
            // No `JOIN` keyword, no condition — the caller (`FromClause`/
            // `TableRef` Display) prints the leading `, ` separator.
            if let Some(table_ref) = self.table_ref() {
                write!(f, "{}", table_ref)?;
            }
            return Ok(());
        }

        if self.is_natural() {
            write!(f, "NATURAL ")?;
        }

        // Join type
        match self.join_type() {
            Some(JoinType::Inner) => write!(f, "INNER JOIN")?,
            Some(JoinType::Left) => write!(f, "LEFT JOIN")?,
            Some(JoinType::Right) => write!(f, "RIGHT JOIN")?,
            Some(JoinType::Full) => write!(f, "FULL JOIN")?,
            Some(JoinType::Cross) => write!(f, "CROSS JOIN")?,
            None => write!(f, "JOIN")?, // Bare JOIN (defaults to INNER)
        }

        // Table reference
        if let Some(table_ref) = self.table_ref() {
            write!(f, " {}", table_ref)?;
        }

        // Join condition
        if let Some(condition) = self.condition() {
            write!(f, " {}", condition)?;
        }

        Ok(())
    }
}

impl Display for JoinCondition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_on() {
            write!(f, "ON ")?;
            if let Some(expr) = self.on_expression() {
                write!(f, "{}", expr.text())?;
            }
        } else if self.is_using() {
            write!(f, "USING (")?;
            let columns = self.using_columns();
            for (i, col) in columns.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{}", col)?;
            }
            write!(f, ")")?;
        }
        Ok(())
    }
}

impl Display for HavingClause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(expr) = self.expression() {
            write!(f, "{}", expr.text())?;
        }
        Ok(())
    }
}

impl Display for QualifyClause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(expr) = self.expression() {
            write!(f, "{}", expr.text())?;
        }
        Ok(())
    }
}

impl Display for WindowClause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let windows: Vec<_> = self.named_windows().collect();
        for (i, nw) in windows.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", nw)?;
        }
        Ok(())
    }
}

impl Display for NamedWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // name AS (body)
        if let Some(name) = self.name() {
            write!(f, "{} AS (", name)?;
        } else {
            write!(f, "? AS (")?;
        }

        let mut needs_space = false;

        if let Some(partition_by) = self.partition_by() {
            write!(f, "{}", partition_by)?;
            needs_space = true;
        }

        if let Some(order_by) = self.order_by() {
            if needs_space {
                write!(f, " ")?;
            }
            write!(f, "{}", order_by)?;
            needs_space = true;
        }

        if let Some(frame) = self.window_frame() {
            if needs_space {
                write!(f, " ")?;
            }
            write!(f, "{}", frame)?;
        }

        write!(f, ")")?;
        Ok(())
    }
}

impl Display for OrderByClause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // DuckDB `ORDER BY ALL [ASC|DESC] [NULLS FIRST|LAST]`: the clause carries
        // a bare ALL_KW marker with an optional direction / NULLS ordering and
        // no per-key OrderByItem children.
        if self.is_all() {
            write!(f, "ORDER BY ALL")?;
            let tokens: Vec<_> = self
                .syntax()
                .children_with_tokens()
                .filter_map(|e| e.into_token())
                .collect();
            let mut seen_all = false;
            for token in tokens {
                match token.kind() {
                    ALL_KW => seen_all = true,
                    ASC_KW if seen_all => write!(f, " ASC")?,
                    DESC_KW if seen_all => write!(f, " DESC")?,
                    NULLS_KW if seen_all => write!(f, " NULLS")?,
                    FIRST_KW if seen_all => write!(f, " FIRST")?,
                    LAST_KW if seen_all => write!(f, " LAST")?,
                    _ => {}
                }
            }
            return Ok(());
        }

        write!(f, "ORDER BY ")?;
        let items: Vec<_> = self.items().collect();
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", item)?;
        }
        Ok(())
    }
}

impl Display for OrderByItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(expr) = self.expression() {
            write!(f, "{}", expr.text())?;
        }

        if let Some(direction) = self.direction() {
            match direction {
                SortDirection::Asc => write!(f, " ASC")?,
                SortDirection::Desc => write!(f, " DESC")?,
            }
        }

        if let Some(null_ordering) = self.null_ordering() {
            match null_ordering {
                NullOrdering::First => write!(f, " NULLS FIRST")?,
                NullOrdering::Last => write!(f, " NULLS LAST")?,
            }
        }

        Ok(())
    }
}

impl Display for LimitClause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Try structured extraction first
        if let Some(limit_val) = self.limit_value() {
            write!(f, "LIMIT ")?;
            match limit_val {
                LimitValue::Number(n) => write!(f, "{}", n)?,
                LimitValue::All => write!(f, "ALL")?,
            }

            if let Some(offset) = self.offset_value() {
                write!(f, " OFFSET {}", offset)?;
            }
        } else {
            // Fall back to raw text if structured extraction fails
            write!(f, "{}", self.syntax().text())?;
        }

        Ok(())
    }
}

// ===== Window Functions (Phase 12) =====

impl Display for WindowSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OVER (")?;

        let mut needs_space = false;

        if let Some(partition_by) = self.partition_by() {
            write!(f, "{}", partition_by)?;
            needs_space = true;
        }

        if let Some(order_by) = self.order_by() {
            if needs_space {
                write!(f, " ")?;
            }
            write!(f, "{}", order_by)?;
            needs_space = true;
        }

        if let Some(frame) = self.frame() {
            if needs_space {
                write!(f, " ")?;
            }
            write!(f, "{}", frame)?;
        }

        write!(f, ")")?;
        Ok(())
    }
}

impl Display for PartitionByClause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PARTITION BY ")?;
        let exprs: Vec<_> = self.expressions().collect();
        for (i, expr) in exprs.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", expr.text())?;
        }
        Ok(())
    }
}

impl Display for WindowFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.unit() {
            Some(FrameUnit::Rows) => write!(f, "ROWS")?,
            Some(FrameUnit::Range) => write!(f, "RANGE")?,
            Some(FrameUnit::Groups) => write!(f, "GROUPS")?,
            None => {}
        }

        let bounds = self.bounds();
        if bounds.len() == 1 {
            write!(f, " {}", bounds[0].text())?;
        } else if bounds.len() == 2 {
            write!(f, " BETWEEN {} AND {}", bounds[0].text(), bounds[1].text())?;
        }

        Ok(())
    }
}

// ===== CTEs (Phase 13) =====

impl Display for WithClause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WITH")?;

        if self.is_recursive() {
            write!(f, " RECURSIVE")?;
        }

        let ctes: Vec<_> = self.ctes().collect();
        for (i, cte) in ctes.iter().enumerate() {
            if i > 0 {
                write!(f, ",")?;
            }
            write!(f, " {}", cte)?;
        }

        Ok(())
    }
}

impl Display for Cte {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(name) = self.name() {
            write!(f, "{}", name)?;
        }

        // Column list
        let columns = self.column_names();
        if !columns.is_empty() {
            write!(f, "(")?;
            for (i, col) in columns.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{}", col)?;
            }
            write!(f, ")")?;
        }

        write!(f, " AS ")?;

        if self.is_materialized() {
            write!(f, "MATERIALIZED ")?;
        } else if self.is_not_materialized() {
            write!(f, "NOT MATERIALIZED ")?;
        }

        if let Some(query) = self.query() {
            write!(f, "{}", query)?;
        }

        Ok(())
    }
}

impl Display for Subquery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(")?;
        if let Some(select) = self.select_stmt() {
            write!(f, "{}", select)?;
        }
        write!(f, ")")?;
        Ok(())
    }
}

// ===== Lambda Expressions =====

impl Display for LambdaExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let params = self.params();
        if params.len() == 1 {
            write!(f, "{}", params[0])?;
        } else {
            write!(f, "(")?;
            for (i, p) in params.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{}", p)?;
            }
            write!(f, ")")?;
        }
        write!(f, " -> ")?;
        if let Some(body) = self.body() {
            write!(f, "{}", body.text())?;
        }
        Ok(())
    }
}
