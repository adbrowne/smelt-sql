/// SQL printer for converting AST back to SQL
///
/// This module provides Display implementations for AST nodes to enable
/// round-trip testing (parse → print → parse).
///
/// Formatting rules:
/// - Keywords: UPPERCASE
/// - Identifiers: preserve case
/// - Indentation: 2 spaces (in Pretty mode)
/// - Line breaks: at major clauses (in Pretty mode)
use crate::ast::*;
use crate::syntax_kind::SyntaxNode;
use crate::SyntaxKind::*;
use std::fmt::{self, Display};

/// Format mode for SQL printing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatMode {
    /// Single-line output (no line breaks)
    Compact,
    /// Multi-line with indentation
    Pretty,
}

/// Context for formatting SQL
#[derive(Debug, Clone)]
#[allow(dead_code)] // Will be used for pretty printing in future
pub struct FormatContext {
    mode: FormatMode,
    indent_level: usize,
}

#[allow(dead_code)] // Will be used for pretty printing in future
impl FormatContext {
    pub fn new(mode: FormatMode) -> Self {
        Self {
            mode,
            indent_level: 0,
        }
    }

    pub fn compact() -> Self {
        Self::new(FormatMode::Compact)
    }

    pub fn pretty() -> Self {
        Self::new(FormatMode::Pretty)
    }

    fn indent(&self) -> String {
        if self.mode == FormatMode::Compact {
            String::new()
        } else {
            "  ".repeat(self.indent_level)
        }
    }

    fn newline(&self) -> &str {
        if self.mode == FormatMode::Compact {
            " "
        } else {
            "\n"
        }
    }

    fn with_indent(&self) -> Self {
        Self {
            mode: self.mode,
            indent_level: self.indent_level + 1,
        }
    }
}

// ===== Basic Display implementations =====

impl Display for File {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(stmt) = self.select_stmt() {
            write!(f, "{}", stmt)?;
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

        // ORDER BY clause
        if let Some(order_by_clause) = self.order_by_clause() {
            write!(f, " {}", order_by_clause)?;
        }

        // LIMIT clause
        if let Some(limit_clause) = self.limit_clause() {
            write!(f, " {}", limit_clause)?;
        }

        // Set operations: UNION / INTERSECT / EXCEPT
        if let Some(set_op) = get_set_operation(self.syntax()) {
            write!(f, " {}", set_op.keyword)?;
            if set_op.all {
                write!(f, " ALL")?;
            }
            if let Some(select) = set_op.select {
                write!(f, " {}", select)?;
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
        // Try to get the expression, or fall back to raw text
        if let Some(expr) = self.expression() {
            write!(f, "{}", expr.text())?;
        } else {
            // For simple tokens like * that don't have an EXPRESSION wrapper,
            // extract the text directly (excluding AS and alias if present)
            let text = self.syntax().text().to_string();
            if self.alias().is_some() {
                // Remove "AS alias" part
                if let Some(as_pos) = text.to_uppercase().find(" AS ") {
                    write!(f, "{}", text[..as_pos].trim())?;
                } else {
                    write!(f, "{}", text.trim())?;
                }
            } else {
                write!(f, "{}", text.trim())?;
            }
        }

        if let Some(alias) = self.alias() {
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

        // Get all JOINs
        for join in self.joins() {
            write!(f, " {}", join)?;
        }

        Ok(())
    }
}

impl Display for TableRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(func_call) = self.function_call() {
            write!(f, "{}", func_call.text())?;
        } else if let Some(ident) = self.identifier() {
            write!(f, "{}", ident)?;
        } else {
            // Subquery in FROM
            write!(f, "{}", self.syntax().text())?;
        }
        Ok(())
    }
}

impl Display for JoinClause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

impl Display for OrderByClause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

// ===== Helper functions =====

/// Extract GROUP BY expressions from syntax node
fn extract_group_by_expressions(node: &SyntaxNode) -> String {
    let mut expressions = Vec::new();
    for child in node.children() {
        if child.kind() == EXPRESSION || child.kind() == BINARY_EXPR {
            expressions.push(child.text().to_string());
        }
    }
    expressions.join(", ")
}

/// Info about a set operation (UNION/INTERSECT/EXCEPT)
struct SetOperation {
    keyword: &'static str,
    all: bool,
    select: Option<SelectStmt>,
}

/// Detect and extract set operation (UNION/INTERSECT/EXCEPT) from a SELECT_STMT node
fn get_set_operation(node: &SyntaxNode) -> Option<SetOperation> {
    let set_op_kinds = [UNION_KW, INTERSECT_KW, EXCEPT_KW];

    let tokens: Vec<_> = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .collect();

    // Find the set operation keyword
    let mut op_kind = None;
    let mut has_all = false;

    for (i, token) in tokens.iter().enumerate() {
        if set_op_kinds.contains(&token.kind()) {
            op_kind = Some(token.kind());
            // Check for ALL after the keyword
            for next_token in &tokens[i + 1..] {
                match next_token.kind() {
                    WHITESPACE | COMMENT => continue,
                    ALL_KW => {
                        has_all = true;
                        break;
                    }
                    _ => break,
                }
            }
            break;
        }
    }

    let op_kind = op_kind?;

    let keyword = match op_kind {
        UNION_KW => "UNION",
        INTERSECT_KW => "INTERSECT",
        EXCEPT_KW => "EXCEPT",
        _ => unreachable!(),
    };

    // Find the SELECT statement after the set operation
    let mut found_op = false;
    let mut select = None;
    for child in node.children_with_tokens() {
        if let Some(token) = child.as_token() {
            if token.kind() == op_kind {
                found_op = true;
            }
        } else if found_op {
            if let Some(n) = child.as_node() {
                if n.kind() == SELECT_STMT {
                    select = SelectStmt::cast(n.clone());
                    break;
                }
            }
        }
    }

    Some(SetOperation {
        keyword,
        all: has_all,
        select,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn assert_round_trip(sql: &str) {
        let parse1 = parse(sql);
        assert_eq!(parse1.errors.len(), 0, "Parse errors: {:?}", parse1.errors);

        let file = File::cast(parse1.syntax()).unwrap();
        let printed = file.to_string();

        let parse2 = parse(&printed);
        assert_eq!(
            parse2.errors.len(),
            0,
            "Re-parse errors: {:?}\nPrinted SQL: {}",
            parse2.errors,
            printed
        );

        // For debugging: print both versions
        if printed.trim() != sql.trim() {
            eprintln!("Original: {}", sql);
            eprintln!("Printed:  {}", printed);
        }
    }

    #[test]
    fn test_simple_select() {
        assert_round_trip("SELECT * FROM users");
    }

    #[test]
    fn test_select_with_alias() {
        assert_round_trip("SELECT name AS user_name FROM users");
    }

    #[test]
    fn test_select_join() {
        assert_round_trip("SELECT * FROM users INNER JOIN orders ON users.id = orders.user_id");
    }

    #[test]
    fn test_select_where() {
        assert_round_trip("SELECT * FROM users WHERE age > 18");
    }

    #[test]
    fn test_select_order_by() {
        assert_round_trip("SELECT * FROM users ORDER BY name ASC");
    }

    #[test]
    fn test_select_limit() {
        assert_round_trip("SELECT * FROM users LIMIT 10");
    }

    #[test]
    fn test_select_cte() {
        assert_round_trip("WITH active_users AS (SELECT * FROM users WHERE status = 'active') SELECT * FROM active_users");
    }

    #[test]
    fn test_select_window_function() {
        assert_round_trip("SELECT ROW_NUMBER() OVER (ORDER BY created_at) FROM events");
    }

    #[test]
    fn test_select_distinct() {
        assert_round_trip("SELECT DISTINCT city FROM users");
    }

    #[test]
    fn test_select_group_by_having() {
        assert_round_trip("SELECT city, COUNT(*) FROM users GROUP BY city HAVING COUNT(*) > 5");
    }

    #[test]
    fn test_round_trip_mixed_case_where() {
        // Regression test for fuzzer crash with mixed-case WHERE keyword
        assert_round_trip("SELECT x FROM t WHERE y = 1");
    }

    // ===== Mixed-case keyword regression tests =====
    // These tests verify that round-trip works correctly regardless of keyword casing.
    // SQL keywords are case-insensitive, so the parser accepts any casing.
    // The printer normalizes to uppercase.

    #[test]
    fn test_mixed_case_where_actual() {
        // The original bug: mixed-case WHERE like "WhErE" would crash
        assert_round_trip("SELECT x FROM t WhErE y = 1");
    }

    #[test]
    fn test_mixed_case_select() {
        assert_round_trip("sElEcT * FROM users");
    }

    #[test]
    fn test_mixed_case_from() {
        assert_round_trip("SELECT * fRoM users");
    }

    #[test]
    fn test_mixed_case_inner_join() {
        assert_round_trip("SELECT * FROM a InNeR jOiN b ON a.id = b.id");
    }

    #[test]
    fn test_mixed_case_left_join() {
        assert_round_trip("SELECT * FROM a LeFt JoIn b ON a.id = b.id");
    }

    #[test]
    fn test_mixed_case_group_by() {
        assert_round_trip("SELECT city FROM users GrOuP bY city");
    }

    #[test]
    fn test_mixed_case_order_by() {
        assert_round_trip("SELECT * FROM users OrDeR bY name");
    }

    #[test]
    fn test_mixed_case_order_by_asc_desc() {
        assert_round_trip("SELECT * FROM users ORDER BY name AsC, age DeSc");
    }

    #[test]
    fn test_mixed_case_having() {
        assert_round_trip("SELECT city, COUNT(*) FROM users GROUP BY city HaViNg COUNT(*) > 5");
    }

    #[test]
    fn test_mixed_case_limit() {
        assert_round_trip("SELECT * FROM users LiMiT 10");
    }

    #[test]
    fn test_mixed_case_limit_offset() {
        assert_round_trip("SELECT * FROM users LIMIT 10 oFfSeT 5");
    }

    #[test]
    fn test_mixed_case_distinct() {
        assert_round_trip("SELECT DiStInCt city FROM users");
    }

    #[test]
    fn test_mixed_case_with_cte() {
        assert_round_trip("WiTh cte aS (SELECT 1) SELECT * FROM cte");
    }

    #[test]
    fn test_mixed_case_on_using() {
        assert_round_trip("SELECT * FROM a JOIN b On a.id = b.id");
        assert_round_trip("SELECT * FROM a JOIN b UsInG (id)");
    }

    #[test]
    fn test_mixed_case_nulls_first_last() {
        assert_round_trip("SELECT * FROM users ORDER BY name NuLlS FiRsT");
        assert_round_trip("SELECT * FROM users ORDER BY name NULLS lAsT");
    }

    #[test]
    fn test_mixed_case_and_or() {
        assert_round_trip("SELECT * FROM users WHERE a = 1 AnD b = 2 oR c = 3");
    }

    // QUALIFY round-trip
    #[test]
    fn test_qualify_round_trip() {
        assert_round_trip("SELECT *, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM t QUALIFY rn = 1");
    }

    #[test]
    fn test_qualify_with_having_round_trip() {
        assert_round_trip("SELECT city, COUNT(*) FROM t GROUP BY city HAVING COUNT(*) > 1 QUALIFY ROW_NUMBER() OVER (ORDER BY city) = 1");
    }

    // Array subscript round-trip
    #[test]
    fn test_array_subscript_round_trip() {
        assert_round_trip("SELECT arr[1] FROM t");
    }

    #[test]
    fn test_array_slice_round_trip() {
        assert_round_trip("SELECT arr[1:3] FROM t");
    }

    #[test]
    fn test_date_literal_round_trip() {
        assert_round_trip("SELECT * FROM t WHERE d = DATE '2024-01-01'");
    }

    // UNION ALL printing test
    #[test]
    fn test_union_all_round_trip() {
        assert_round_trip("SELECT id FROM a UNION ALL SELECT id FROM b");
    }

    #[test]
    fn test_union_round_trip() {
        assert_round_trip("SELECT id FROM a UNION SELECT id FROM b");
    }

    // NULLS FIRST/LAST printing tests
    #[test]
    fn test_nulls_first_round_trip() {
        assert_round_trip("SELECT * FROM t ORDER BY name NULLS FIRST");
    }

    #[test]
    fn test_nulls_last_round_trip() {
        assert_round_trip("SELECT * FROM t ORDER BY name DESC NULLS LAST");
    }

    // INTERSECT / EXCEPT round-trip
    #[test]
    fn test_intersect_round_trip() {
        assert_round_trip("SELECT id FROM a INTERSECT SELECT id FROM b");
    }

    #[test]
    fn test_except_round_trip() {
        assert_round_trip("SELECT id FROM a EXCEPT SELECT id FROM b");
    }

    #[test]
    fn test_intersect_all_round_trip() {
        assert_round_trip("SELECT id FROM a INTERSECT ALL SELECT id FROM b");
    }

    #[test]
    fn test_except_all_round_trip() {
        assert_round_trip("SELECT id FROM a EXCEPT ALL SELECT id FROM b");
    }
}
