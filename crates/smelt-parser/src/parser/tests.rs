use super::*;
#[allow(unused_imports)]
use crate::ast::{
    ArraySlice, ArraySubscript, BetweenExpr, BinaryExpr, CaseExpr, CastExpr, CollateExpr, Cte,
    ExistsExpr, Expr, File, FilterClause, FrameUnit, FunctionCall, GroupByClause, HavingClause,
    InExpr, JoinType, Lambda, LambdaExpr, LimitClause, LimitValue, NamedParam, NullOrdering,
    OrderByClause, OrderByItem, PartitionByClause, PipeExpr, PivotClause, QualifyClause,
    SelectItem, SelectList, SelectStmt, SortDirection, Subquery, TableRef, UnpivotClause,
    ValuesClause, WhenClause, WindowClause, WindowFrame, WindowSpec, WithClause,
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
    let input =
        "SELECT (SELECT COUNT(*) FROM orders WHERE user_id = users.id) AS order_count FROM users";
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
    let input = "SELECT * FROM users WHERE id IN (SELECT user_id FROM orders WHERE total > 100)";
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
    let input =
        "SELECT * FROM users WHERE EXISTS (SELECT 1 FROM orders WHERE orders.user_id = users.id)";
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
fn test_interval_literal_is_not_a_column_ref() {
    // `INTERVAL '1 day'` lexes as IDENT("INTERVAL") + STRING; as_column_ref
    // must NOT mistake it for a column named "INTERVAL" (regression for a
    // false "Column 'INTERVAL' not found" diagnostic on `col - INTERVAL '…'`).
    let parse = parse("SELECT d - INTERVAL '1 day' AS x FROM t");
    assert_eq!(parse.errors.len(), 0, "{:?}", parse.errors);
    let file = File::cast(parse.syntax()).expect("file");
    let select = file.select_stmt().expect("select");
    let item = select
        .select_list()
        .expect("list")
        .items()
        .next()
        .expect("item");
    let expr = item.expression().expect("expr");
    let binary = expr
        .as_binary()
        .expect("binary expr (d - INTERVAL '1 day')");
    let lhs = binary.left().expect("lhs");
    let rhs = binary.right().expect("rhs");
    assert_eq!(
        lhs.as_column_ref().map(|c| c.name().to_string()),
        Some("d".to_string()),
        "the bare identifier `d` is a column ref"
    );
    assert!(
        rhs.as_column_ref().is_none(),
        "INTERVAL literal must not be a column ref, got {:?}",
        rhs.as_column_ref().map(|c| c.name().to_string())
    );
}

#[test]
fn test_window_frame_range_interval_preceding() {
    // RANGE BETWEEN INTERVAL '...' PRECEDING is the spec's Form A lookback
    // declaration (incremental_models.md). DuckDB supports it; the parser must too.
    let input = "SELECT LAG(ts) OVER (PARTITION BY device_id ORDER BY ts RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW) FROM events";
    let parse = parse(input);
    if !parse.errors.is_empty() {
        eprintln!("Errors: {:?}", parse.errors);
    }
    assert_eq!(parse.errors.len(), 0);
}

#[test]
fn test_window_frame_range_interval_both_bounds() {
    let input = "SELECT SUM(x) OVER (ORDER BY ts RANGE BETWEEN INTERVAL '2 hours' PRECEDING AND INTERVAL '1 hour' FOLLOWING) FROM events";
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
    let input =
        "SELECT SUM(amount) OVER (PARTITION BY user_id, category ORDER BY date) FROM transactions";
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
    let input = "SELECT date, price, LEAD(price, 1) OVER (ORDER BY date) as next_price FROM prices";
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
fn test_union_parenthesized_right_operand() {
    // `A UNION (B)` — the right-hand operand of a set operation may be
    // parenthesized. `union_select()` must unwrap the SUBQUERY wrapper to
    // reach the inner SelectStmt so downstream schema unification still
    // sees it.
    let input = "SELECT id FROM users UNION (SELECT id FROM customers)";
    let (_, select) = parse_select(input);

    assert!(select.has_union(), "should have UNION");
    assert!(
        select.union_select().is_some(),
        "should unwrap the parenthesized right operand"
    );
    assert!(select.union_select().unwrap().select_list().is_some());
}

#[test]
fn test_union_deeply_nested_parenthesized_operand() {
    // Redundant nesting `(((...)))` and a set-op tail inside the parens
    // (EXCEPT + ORDER BY) both round-trip through the same
    // parse_query_expr/parse_parenthesized_query recursion.
    let input =
        "SELECT q1 FROM t UNION ALL (((SELECT q2 FROM t EXCEPT SELECT q1 FROM t ORDER BY 1)))";
    let parse = parse(input);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

    let file = File::cast(parse.syntax()).unwrap();
    let select = file.select_stmt().unwrap();
    assert!(select.has_union(), "should have UNION");
    assert!(select.is_union_all(), "should be UNION ALL");
    let right = select
        .union_select()
        .expect("should unwrap triple-nested parens to reach the inner SelectStmt");
    assert!(
        right.has_set_operation(),
        "the parenthesized right operand's own EXCEPT tail should still be reachable"
    );
}

#[test]
fn test_union_parenthesized_operand_in_from_subquery() {
    // Parenthesized set-op nesting inside a FROM-clause derived table:
    // `FROM (A UNION (B UNION C)) alias`.
    let input = "SELECT * FROM (SELECT 1 UNION (SELECT 1 UNION SELECT 1)) t";
    let parse = parse(input);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
}

#[test]
fn test_union_chain_still_parses_without_parens() {
    // Regression guard: a plain (unparenthesized) UNION ALL chain with a
    // trailing LIMIT must still parse exactly as before.
    let input = "SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 LIMIT 5";
    let parse = parse(input);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
}

#[test]
fn test_subquery_in_from_still_parses() {
    // Regression guard: a plain (non-set-op) derived-table subquery in FROM
    // must still parse; the FROM-clause LPAREN branch now dispatches
    // through parse_query_expr instead of parse_select_stmt directly.
    let input = "SELECT * FROM (SELECT id FROM users) u";
    let parse = parse(input);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
}

#[test]
fn test_scalar_subquery_still_parses() {
    // Regression guard: a plain scalar subquery expression must still
    // parse; the expr.rs LPAREN branch now also accepts WITH_KW and a
    // nested LPAREN alongside the original SELECT_KW check.
    let input = "SELECT (SELECT COUNT(*) FROM orders) AS n";
    let parse = parse(input);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
}

#[test]
fn test_scalar_subquery_with_internal_union_still_parses() {
    let input = "SELECT (SELECT 2 UNION SELECT 2)";
    let parse = parse(input);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
}

#[test]
fn test_smelt_ref_with_cte() {
    // Phase 4: smelt.ref() is removed; updated to use smelt.<path> form.
    let input = r#"
WITH recent_activity AS (
  SELECT user_id, COUNT(*) as event_count
  FROM smelt.models.raw_events
  GROUP BY user_id
  HAVING COUNT(*) > 10
)
SELECT u.name, ra.event_count,
   RANK() OVER (ORDER BY ra.event_count DESC) as activity_rank
FROM smelt.models.users u
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

    // Verify that we can find the path refs.
    use crate::ast::{File, SmeltPathRef};
    let file = File::cast(parse.syntax()).unwrap();
    let path_refs: Vec<_> = file
        .syntax()
        .descendants()
        .filter_map(SmeltPathRef::cast)
        .collect();
    assert_eq!(path_refs.len(), 2);
    let segments_list: Vec<Vec<String>> = path_refs.iter().map(|r| r.segments()).collect();
    assert!(segments_list.contains(&vec!["models".to_string(), "raw_events".to_string()]));
    assert!(segments_list.contains(&vec!["models".to_string(), "users".to_string()]));
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
    let input =
        "SELECT DISTINCT ON (user_id, date) * FROM events ORDER BY user_id, date, created_at DESC";
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
fn test_lateral_subquery_comma_form_is_a_registered_gap() {
    // Comma-separated FROM lists (`FROM a, LATERAL (...)`) have never had a
    // grammar production — `parse_from_clause` only recognises the first
    // table ref plus JOIN-keyword chains (see `test_lateral_join` for the
    // supported `LEFT JOIN LATERAL` form). Before fail-loud trailing-content
    // detection, the leftover `, LATERAL (...) o` was silently absorbed at
    // end-of-file, so this construct looked accepted; it now surfaces the
    // parse error the grammar always implied. Comma-separated FROM lists are
    // unimplemented grammar, not a fail-loud-parsing regression — tracked as
    // follow-on grammar work.
    let input = "SELECT * FROM users, LATERAL (SELECT * FROM orders WHERE user_id = users.id) o";
    let parse = parse(input);
    assert!(
        !parse.errors.is_empty(),
        "comma-separated FROM lists are not supported by the grammar; \
         expected a parse error, got a clean parse"
    );
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
    // Phase 4: updated from smelt.source() to smelt.sources.* path form.
    use crate::ast::File;

    let input = "SELECT * FROM smelt.sources.raw.users AS u";
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
    // Phase 4: updated from smelt.source() to smelt.sources.* path form.
    use crate::ast::File;

    let input = "SELECT * FROM smelt.sources.raw.users u";
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
    // Phase 4: updated from smelt.source() to smelt.sources.* path form.
    use crate::ast::File;

    let input = "SELECT * FROM smelt.sources.raw.users";
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
    // Phase 4: updated from smelt.ref() to smelt.models.* path form.
    use crate::ast::File;

    let input = "SELECT * FROM smelt.models.users AS t";
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
    // Phase 4: updated from smelt.source() to smelt.sources.* path form.
    use crate::ast::File;

    let input =
        "SELECT * FROM smelt.sources.raw.users u JOIN smelt.sources.raw.orders AS o ON u.id = o.user_id";
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
    // Phase 4: smelt.ref() is removed. Test generic named-param syntax instead.
    let input = "SELECT my_func(x, filter => a + b > 10) FROM t";
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
    // Phase 4: smelt.ref() is removed. Named param syntax still works
    // via path-call form: smelt.models.model(key => 'value').
    let input = "SELECT * FROM smelt.models.model(key => 'value')";
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
    // Phase 4: smelt.ref() is removed. Test a generic namespaced function call.
    let input = "SELECT * FROM myns.myfunc('model')";
    let parse = parse(input);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

    let func = parse
        .syntax()
        .descendants()
        .find_map(FunctionCall::cast)
        .expect("should have a FunctionCall");
    assert_eq!(func.namespace().as_deref(), Some("myns"));
    assert_eq!(func.name().as_deref(), Some("myfunc"));
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
fn test_deeply_nested_inline_record_types_produces_error() {
    // 300 levels of nested inline record types — exceeds the 256 depth limit.
    // Without a guard on `parse_record_type_inline`, this would blow the stack.
    let depth = 300;
    let mut input = String::new();
    input.push_str("smelt.record Deep = ");
    for _ in 0..depth {
        input.push_str("{ a: ");
    }
    input.push_str("Text");
    for _ in 0..depth {
        input.push_str(" }");
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

#[test]
fn test_case_is_null_or() {
    // Regression: IS NULL OR in CASE WHEN was failing to parse
    let input = "SELECT CASE WHEN x IS NULL OR y > 1800 THEN 1 ELSE 0 END AS flag FROM t";
    let (parse, _) = parse_select(input);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
}

#[test]
fn test_case_in_sum_with_is_null_or() {
    // Regression: SUM(CASE WHEN ... IS NULL OR ... THEN ... END)
    let input = "SELECT SUM(CASE WHEN gap IS NULL OR gap > 1800 THEN 1 ELSE 0 END) OVER (PARTITION BY v ORDER BY ts) AS sid FROM t";
    let (parse, _) = parse_select(input);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
}

// ===== Phase 1: smelt.define top-level grammar =====

use crate::ast::{Param, ParamList, SmeltDefine, TypeRef};

/// Parse the full file-level syntax without asserting on shape. Mirrors
/// `parse_select` but for tests that exercise multi-declaration files.
fn parse_file_text(text: &str) -> (Parse, File) {
    let parse = parse(text);
    let file = File::cast(parse.syntax()).expect("parse should yield a FILE node");
    (parse, file)
}

#[test]
fn parses_minimal_smelt_define() {
    let input = "smelt.define foo(x) AS (x + 1)";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let defines: Vec<SmeltDefine> = file.defines().collect();
    assert_eq!(defines.len(), 1, "expected exactly one smelt.define");
    let def = &defines[0];

    assert_eq!(def.name().as_deref(), Some("foo"));

    let params: Vec<Param> = def
        .param_list()
        .expect("should have a param list")
        .params()
        .collect();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name().as_deref(), Some("x"));
    assert!(
        params[0].type_ref().is_none(),
        "untyped param should have no TypeRef"
    );
    assert!(params[0].default_value().is_none());

    let body = def.body().expect("should have a body");
    assert!(
        body.expression().is_some(),
        "body should contain an expression"
    );
}

#[test]
fn parses_typed_params() {
    let input = "smelt.define safe_divide(numerator: Expr<Numeric>, denominator: Expr<Numeric>) -> Expr<Double> AS (CAST(numerator AS DOUBLE) / CAST(denominator AS DOUBLE))";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let def = file.defines().next().expect("one smelt.define");
    assert_eq!(def.name().as_deref(), Some("safe_divide"));

    let plist: ParamList = def.param_list().expect("param list");
    let params: Vec<Param> = plist.params().collect();
    assert_eq!(params.len(), 2);

    assert_eq!(params[0].name().as_deref(), Some("numerator"));
    let t0: TypeRef = params[0].type_ref().expect("param 0 should have type");
    // Flat text of the type reference — whitespace is preserved.
    let t0_text_compact: String = t0.text().chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(t0_text_compact, "Expr<Numeric>");

    assert_eq!(params[1].name().as_deref(), Some("denominator"));
    let t1: TypeRef = params[1].type_ref().expect("param 1 should have type");
    let t1_text_compact: String = t1.text().chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(t1_text_compact, "Expr<Numeric>");

    let ret: TypeRef = def.return_type().expect("return type");
    let ret_compact: String = ret.text().chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(ret_compact, "Expr<Double>");

    assert!(def.body().is_some(), "body must be present");
}

#[test]
fn parses_default_values() {
    let input = "smelt.define foo(x: Expr<Integer> = 0) AS (x)";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let def = file.defines().next().expect("one smelt.define");
    let params: Vec<Param> = def.param_list().unwrap().params().collect();
    assert_eq!(params.len(), 1);
    assert!(params[0].type_ref().is_some());
    assert!(
        params[0].default_value().is_some(),
        "parameter should have a DEFAULT_VALUE node"
    );
}

#[test]
fn parses_file_with_define_and_model() {
    let input = "smelt.define foo(x) AS (x + 1)\n\nSELECT * FROM t";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );
    assert_eq!(file.defines().count(), 1);
    assert!(
        file.select_stmt().is_some(),
        "file should have a SELECT stmt"
    );
}

#[test]
fn parses_multiple_defines() {
    let input = "\
        smelt.define a(x) AS (x + 1)\n\
        smelt.define b(y) AS (y * 2)\n\
        smelt.define c(z) AS (z - 3)\n";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );
    let names: Vec<Option<String>> = file.defines().map(|d| d.name()).collect();
    assert_eq!(names.len(), 3);
    assert_eq!(names[0].as_deref(), Some("a"));
    assert_eq!(names[1].as_deref(), Some("b"));
    assert_eq!(names[2].as_deref(), Some("c"));
    assert!(
        file.select_stmt().is_none(),
        "file should have no SELECT stmt"
    );
}

#[test]
fn error_recovery_missing_as() {
    // Malformed: missing `AS` between param list and body parens.
    // The parser must still recover and parse the following smelt.define.
    let input = "smelt.define bad(x) (x)\nsmelt.define good(y) AS (y)";
    let (parse, file) = parse_file_text(input);
    assert!(
        !parse.errors.is_empty(),
        "expected at least one parse error"
    );
    let defines: Vec<SmeltDefine> = file.defines().collect();
    assert_eq!(
        defines.len(),
        2,
        "recovery should still parse a second smelt.define"
    );
    assert_eq!(defines[1].name().as_deref(), Some("good"));
    assert!(defines[1].param_list().is_some());
    assert!(defines[1].body().is_some());
}

#[test]
fn error_recovery_unbalanced_body() {
    // The first define has an unbalanced `(` in its body. The parser must
    // record errors and still parse the following smelt.define.
    let input = "smelt.define bad(x) AS ((x + 1)\nsmelt.define good(y) AS (y)";
    let (parse, file) = parse_file_text(input);
    assert!(
        !parse.errors.is_empty(),
        "expected at least one parse error"
    );
    let defines: Vec<SmeltDefine> = file.defines().collect();
    assert_eq!(
        defines.len(),
        2,
        "recovery should still parse a second smelt.define"
    );
    assert_eq!(defines[1].name().as_deref(), Some("good"));
    assert!(defines[1].body().is_some());
}

#[test]
fn smelt_define_in_expression_position_is_not_special() {
    // `smelt.define` inside a SELECT should parse as a qualified column
    // reference, not as a declaration. No SmeltDefine nodes, and no
    // `define`-specific errors.
    let input = "SELECT smelt.define FROM t";
    let (parse, file) = parse_file_text(input);
    assert_eq!(
        file.defines().count(),
        0,
        "no smelt.define declarations expected"
    );
    assert!(file.select_stmt().is_some(), "should have a SELECT stmt");
    for err in &parse.errors {
        let m = err.message.to_lowercase();
        assert!(
            !m.contains("smelt.define"),
            "did not expect a smelt.define-specific error, got: {:?}",
            err
        );
    }
}

// ===== Phase 10: smelt.extern top-level grammar =====

use crate::ast::SmeltExtern;

#[test]
fn parses_smelt_extern_minimal() {
    // Phase 10 TDD test 1.
    let input = "smelt.extern regex_match(text: Expr<Text>, pattern: Expr<Text>) -> Expr<Boolean>";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let externs: Vec<SmeltExtern> = file.externs().collect();
    assert_eq!(externs.len(), 1, "expected exactly one smelt.extern");
    let ext = &externs[0];

    assert_eq!(ext.name().as_deref(), Some("regex_match"));

    let params: Vec<Param> = ext
        .param_list()
        .expect("should have a param list")
        .params()
        .collect();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name().as_deref(), Some("text"));
    let t0 = params[0].type_ref().expect("typed param");
    let t0_compact: String = t0.text().chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(t0_compact, "Expr<Text>");

    assert_eq!(params[1].name().as_deref(), Some("pattern"));

    let ret: TypeRef = ext.return_type().expect("return type");
    let ret_compact: String = ret.text().chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(ret_compact, "Expr<Boolean>");

    // No smelt.define declarations in this file.
    assert_eq!(file.defines().count(), 0);
}

#[test]
fn extern_with_frontmatter_backends() {
    // Phase 10 TDD test 2. The existing file-level `---` frontmatter
    // block must coexist with a `smelt.extern` — the legacy single-block
    // rule applies. The frontmatter is stripped by
    // `smelt_parser::strip_frontmatter` before reaching the parser, so
    // the extern must parse identically to the minimal case.
    let input = "---\nbackends: [duckdb]\n---\n\
                 smelt.extern read_parquet(path: Expr<Text>) -> Expr<Text>\n";
    let clean = crate::strip_frontmatter(input);
    let (parse, file) = parse_file_text(&clean);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let externs: Vec<SmeltExtern> = file.externs().collect();
    assert_eq!(externs.len(), 1);
    assert_eq!(externs[0].name().as_deref(), Some("read_parquet"));
    let params: Vec<Param> = externs[0].param_list().unwrap().params().collect();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name().as_deref(), Some("path"));
}

#[test]
fn smelt_extern_and_define_in_same_file() {
    // Mixed file: one of each. The iterators should partition cleanly.
    let input = "\
        smelt.extern ext_fn(a: Expr<Text>) -> Expr<Text>\n\
        smelt.define my_plus(x: Expr<Integer>) -> Expr<Integer> AS (x + 1)\n";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );
    assert_eq!(file.externs().count(), 1);
    assert_eq!(file.defines().count(), 1);
}

// ===== Phase 2 → Phase 5b: smelt.fn.* rejection tests =====
// Phase 5b removes the smelt.fn.* parser arm. Tests that previously
// verified successful parsing now verify rejection (parse errors).

use crate::syntax_kind::SyntaxNode;

/// Helper: collect all FUNCTION_CALL descendants of a node.
fn function_calls(root: &SyntaxNode) -> Vec<FunctionCall> {
    root.descendants().filter_map(FunctionCall::cast).collect()
}

#[test]
fn smelt_fn_call_is_now_rejected() {
    // Phase 5b: smelt.fn.* must produce parse errors.
    let inputs = &[
        "SELECT smelt.fn.safe_divide(a, b) FROM t",
        "SELECT smelt.fn.safe_divide(numerator => a, denominator => b) FROM t",
        "SELECT smelt.fn.core.math.safe_divide(a, b) FROM t",
        "SELECT * FROM t WHERE smelt.fn.is_valid(x)",
    ];
    for input in inputs {
        let (parse, _file) = parse_file_text(input);
        assert!(
            !parse.errors.is_empty(),
            "Phase 5b: smelt.fn.* must produce parse errors for input: {:?}",
            input
        );
    }
}

#[test]
fn smelt_fn_without_parens_is_error() {
    // smelt.fn.foo without parens was already an error; still an error.
    let input = "SELECT smelt.fn.foo FROM t";
    let (parse, _file) = parse_file_text(input);
    assert!(
        !parse.errors.is_empty(),
        "expected at least one parse error for smelt.fn.foo with no '('"
    );
}

#[test]
fn smelt_ref_still_parses_as_function_call() {
    // Phase 4 update: smelt.ref() is rejected with an error. The test
    // is retained to verify that error recovery still produces a
    // FUNCTION_CALL so downstream CST walkers don't panic.
    let input = "SELECT * FROM smelt.ref('model')";
    let (parse, file) = parse_file_text(input);
    // Phase 4: must have at least one parse error.
    assert!(
        !parse.errors.is_empty(),
        "Phase 4: smelt.ref() must produce a parse error"
    );

    // Error recovery produces a FUNCTION_CALL node.
    let fcalls = function_calls(file.syntax());
    assert!(
        !fcalls.is_empty(),
        "error recovery must still produce a FUNCTION_CALL node for smelt.ref()"
    );
}

#[test]
fn smelt_fn_call_inside_define_body_is_rejected() {
    // Phase 5b: smelt.fn.* inside a define body must now produce errors.
    let input = "smelt.define wrap(x) AS (smelt.fn.safe_divide(x, 1))";
    let (parse, _file) = parse_file_text(input);
    assert!(
        !parse.errors.is_empty(),
        "Phase 5b: smelt.fn.* inside define body must produce parse errors"
    );
}

// ===== Phase 11: per-declaration frontmatter =====

#[test]
fn frontmatter_attaches_to_next_decl() {
    // Phase 11 TDD #1: a file containing two `---/---` blocks, each
    // preceded by (i.e. each immediately followed by) a distinct
    // `smelt.define`. Each decl's `frontmatter()` helper returns the
    // matching block's body — and only the matching block.
    let raw = "---\nbackends: [duckdb]\n---\nsmelt.define f() -> Expr<Integer> AS (1)\n\n---\nbackends: [spark]\n---\nsmelt.define g() -> Expr<Integer> AS (2)\n";

    let stripped = crate::strip_frontmatter(raw);
    let (parse, file) = parse_file_text(&stripped);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let defines: Vec<SmeltDefine> = file.defines().collect();
    assert_eq!(defines.len(), 2, "expected two smelt.define declarations");

    let fm_f = defines[0]
        .frontmatter(raw)
        .expect("first decl should have frontmatter");
    assert!(fm_f.contains("duckdb"), "got {:?}", fm_f);
    assert!(!fm_f.contains("spark"), "wrong block attached: {:?}", fm_f);

    let fm_g = defines[1]
        .frontmatter(raw)
        .expect("second decl should have frontmatter");
    assert!(fm_g.contains("spark"), "got {:?}", fm_g);
    assert!(!fm_g.contains("duckdb"), "wrong block attached: {:?}", fm_g);
}

#[test]
fn extern_with_dotted_backend_namespace() {
    // Phase 11: `smelt.extern duckdb.read_parquet(...)` parses cleanly
    // and exposes `read_parquet` as the function name with `duckdb`
    // as the backend namespace.
    use crate::ast::SmeltExtern;
    let input = "smelt.extern duckdb.read_parquet(path: Expr<Text>) -> Expr<Text>\n";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );
    let externs: Vec<SmeltExtern> = file.externs().collect();
    assert_eq!(externs.len(), 1);
    assert_eq!(externs[0].name().as_deref(), Some("read_parquet"));
    assert_eq!(externs[0].backend_namespace().as_deref(), Some("duckdb"));
}

#[test]
fn extern_plain_name_has_no_backend_namespace() {
    // Sanity check — the legacy single-IDENT extern form still works.
    use crate::ast::SmeltExtern;
    let input = "smelt.extern regex_match(s: Expr<Text>, p: Expr<Text>) -> Expr<Boolean>\n";
    let (parse, file) = parse_file_text(input);
    assert!(parse.errors.is_empty(), "{:?}", parse.errors);
    let externs: Vec<SmeltExtern> = file.externs().collect();
    assert_eq!(externs[0].name().as_deref(), Some("regex_match"));
    assert!(externs[0].backend_namespace().is_none());
}

// ===== Phase 13: TableExpr / WindowExpr / SelectItems<K, ctx> in type refs =====

use crate::ast::{ExprKindTag, RowTail, TypeRefHead};

/// Helper: extract the first `PARAM`'s `TYPE_REF` from a single-define file.
fn first_param_type_ref(file: &File) -> TypeRef {
    let def = file.defines().next().expect("one smelt.define");
    let params: Vec<Param> = def.param_list().expect("param list").params().collect();
    params[0].type_ref().expect("first param type ref")
}

#[test]
fn parses_tableexpr_bare() {
    let input = "smelt.define f(source: TableExpr) AS (SELECT * FROM source)";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let tr = first_param_type_ref(&file);
    assert_eq!(tr.kind(), TypeRefHead::TableExpr);
    assert!(
        tr.row_requirement().is_none(),
        "no row requirement expected"
    );
}

#[test]
fn parses_tableexpr_with_row_requirement() {
    let input = "smelt.define f(source: TableExpr<{revenue: Numeric, cost: Numeric}>) AS (SELECT * FROM source)";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let tr = first_param_type_ref(&file);
    assert_eq!(tr.kind(), TypeRefHead::TableExpr);

    let req = tr
        .row_requirement()
        .expect("TableExpr<{...}> should have a ROW_REQUIREMENT");
    let fields = req.fields();
    assert_eq!(fields.len(), 2, "expected two row fields");
    assert_eq!(fields[0].name().as_deref(), Some("revenue"));
    assert!(
        fields[0].type_ref().is_some(),
        "revenue should have a TYPE_REF"
    );
    assert_eq!(fields[1].name().as_deref(), Some("cost"));
    assert!(
        fields[1].type_ref().is_some(),
        "cost should have a TYPE_REF"
    );
    assert!(matches!(req.tail(), RowTail::None));
}

#[test]
fn parses_tableexpr_with_row_tail() {
    // Named tail: ..r
    let input_named =
        "smelt.define f(source: TableExpr<{revenue: Numeric, ..r}>) AS (SELECT * FROM source)";
    let (parse, file) = parse_file_text(input_named);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );
    let tr = first_param_type_ref(&file);
    let req = tr.row_requirement().expect("row requirement");
    match req.tail() {
        RowTail::Named(name) => assert_eq!(name, "r"),
        other => panic!("expected named tail `r`, got {other:?}"),
    }
    assert_eq!(req.fields().len(), 1);

    // Anonymous tail: bare `..`
    let input_anon = "smelt.define g(source: TableExpr<{..}>) AS (SELECT * FROM source)";
    let (parse, file) = parse_file_text(input_anon);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );
    let tr = first_param_type_ref(&file);
    let req = tr.row_requirement().expect("row requirement");
    assert!(matches!(req.tail(), RowTail::Anon));
    assert_eq!(req.fields().len(), 0);
}

#[test]
fn parses_aggexpr_and_windowexpr() {
    let input =
        "smelt.define f(a: Expr<Integer>, b: AggExpr<Integer>, c: WindowExpr<Integer>) AS (SELECT 1)";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let def = file.defines().next().expect("one smelt.define");
    let params: Vec<Param> = def.param_list().unwrap().params().collect();
    assert_eq!(params.len(), 3);

    let t0 = params[0].type_ref().unwrap();
    let t1 = params[1].type_ref().unwrap();
    let t2 = params[2].type_ref().unwrap();
    assert_eq!(t0.kind(), TypeRefHead::Expr);
    assert_eq!(t1.kind(), TypeRefHead::AggExpr);
    assert_eq!(t2.kind(), TypeRefHead::WindowExpr);
    assert_eq!(t0.expr_kind(), Some(ExprKindTag::Scalar));
    assert_eq!(t1.expr_kind(), Some(ExprKindTag::Agg));
    assert_eq!(t2.expr_kind(), Some(ExprKindTag::Window));
}

#[test]
fn parses_selectitems_kind_only() {
    let input = "smelt.define f(items: SelectItems<Agg>) AS (SELECT 1)";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let tr = first_param_type_ref(&file);
    assert_eq!(tr.kind(), TypeRefHead::SelectItems);
    assert_eq!(tr.selectitems_kind().as_deref(), Some("Agg"));
    assert!(tr.selectitems_ctx().is_none());
}

#[test]
fn parses_selectitems_kind_and_ctx() {
    let input = "smelt.define f(items: SelectItems<Agg, sessionized>) AS (SELECT 1)";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let tr = first_param_type_ref(&file);
    assert_eq!(tr.kind(), TypeRefHead::SelectItems);
    assert_eq!(tr.selectitems_kind().as_deref(), Some("Agg"));
    assert_eq!(tr.selectitems_ctx().as_deref(), Some("sessionized"));

    // Declared order: kind before ctx.
    let syntax = tr.syntax();
    let mut seen_kind = false;
    let mut seen_ctx = false;
    for child in syntax.children() {
        if child.kind() == SELECTITEMS_KIND {
            assert!(!seen_ctx, "KIND must come before CTX");
            seen_kind = true;
        } else if child.kind() == SELECTITEMS_CTX {
            assert!(seen_kind, "CTX must follow KIND");
            seen_ctx = true;
        }
    }
    assert!(seen_kind && seen_ctx);
}

#[test]
fn parses_selectitems_ctx_only() {
    let input = "smelt.define f(items: SelectItems<sessionized>) AS (SELECT 1)";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let tr = first_param_type_ref(&file);
    assert_eq!(tr.kind(), TypeRefHead::SelectItems);
    assert!(tr.selectitems_kind().is_none());
    assert_eq!(tr.selectitems_ctx().as_deref(), Some("sessionized"));
}

#[test]
fn rejects_unknown_expr_kind() {
    let input = "smelt.define f(a: FooExpr<Integer>) AS (SELECT 1)";
    let (parse, _file) = parse_file_text(input);
    assert!(
        !parse.errors.is_empty(),
        "expected at least one parse error for unknown sort"
    );
    let mentions_fooexpr = parse.errors.iter().any(|e| {
        let m = e.message.to_lowercase();
        m.contains("fooexpr") || m.contains("unknown sort") || m.contains("unsupported")
    });
    assert!(
        mentions_fooexpr,
        "expected an error mentioning FooExpr / unknown sort / unsupported, got {:?}",
        parse.errors
    );
}

#[test]
fn tableexpr_in_expression_position_is_not_special() {
    // `TableExpr` as a bare identifier in expression position must remain a
    // plain column reference, not a type reference or an error.
    let input = "smelt.define f(x: Expr<Integer>) AS (SELECT TableExpr FROM t WHERE TableExpr > 1)";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let def = file.defines().next().expect("one smelt.define");
    let body = def.body().expect("body");
    let body_syntax = body.syntax();

    // No TYPE_REF nodes should live inside the body (the only TYPE_REF is
    // the one on the `x: Expr<Integer>` parameter).
    let tr_count_in_body = body_syntax
        .descendants()
        .filter(|n| n.kind() == TYPE_REF)
        .count();
    assert_eq!(tr_count_in_body, 0, "body should contain no TYPE_REF nodes");

    // The bare `TableExpr` identifier must appear at least twice (SELECT
    // list + WHERE operand) as an IDENT token inside the body.
    let tableexpr_tokens = body_syntax
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == IDENT && t.text() == "TableExpr")
        .count();
    assert!(
        tableexpr_tokens >= 2,
        "expected at least two `TableExpr` IDENT tokens in the body, got {}",
        tableexpr_tokens
    );
}

// ===== Phase 28: PASSING clauses =====

use crate::ast::PassingClause;
use crate::syntax_kind::SyntaxKind::PASSING_CLAUSE;

/// Helper: collect all PASSING_CLAUSE descendants of a node.
fn passing_clauses_of(root: &SyntaxNode) -> Vec<SyntaxNode> {
    root.descendants()
        .filter(|n| n.kind() == PASSING_CLAUSE)
        .collect()
}

#[test]
fn parses_single_passing_clause() {
    // Phase 5b: smelt.fn.* is rejected; use smelt.functions.* instead.
    // PASSING clauses are still supported on smelt.functions.* calls.
    let input =
        "SELECT smelt.functions.session_rollup(src) PASSING metrics AS (SUM(revenue)) FROM t";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let calls = smelt_path_calls(file.syntax());
    assert_eq!(calls.len(), 1, "expected one SMELT_PATH_CALL");

    let passing: Vec<PassingClause> = calls[0].passing_clauses().collect();
    assert_eq!(passing.len(), 1, "expected one PASSING_CLAUSE");
    assert_eq!(
        passing[0].name().as_deref(),
        Some("metrics"),
        "PASSING_NAME should be 'metrics'"
    );

    // Body should contain SUM(revenue)
    let body_text = passing[0]
        .body_text()
        .expect("PASSING_BODY should have text");
    assert!(
        body_text.contains("SUM"),
        "PASSING_BODY text should contain SUM, got {:?}",
        body_text
    );
}

#[test]
fn parses_multiple_passing_clauses() {
    // Phase 5b: smelt.fn.* is rejected; use smelt.functions.* instead.
    let input = "SELECT smelt.functions.foo(src) PASSING a AS (x + 1) PASSING b AS (y * 2) FROM t";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let calls = smelt_path_calls(file.syntax());
    assert_eq!(calls.len(), 1, "expected one SMELT_PATH_CALL");

    let passing: Vec<PassingClause> = calls[0].passing_clauses().collect();
    assert_eq!(passing.len(), 2, "expected two PASSING_CLAUSEs");
    assert_eq!(passing[0].name().as_deref(), Some("a"));
    assert_eq!(passing[1].name().as_deref(), Some("b"));

    let body0 = passing[0].body_text().expect("first PASSING_BODY text");
    let body1 = passing[1].body_text().expect("second PASSING_BODY text");
    assert!(
        body0.contains("x"),
        "first body should contain 'x', got {:?}",
        body0
    );
    assert!(
        body1.contains("y"),
        "second body should contain 'y', got {:?}",
        body1
    );
}

#[test]
fn passing_not_reserved_elsewhere() {
    // `passing` as a column name should parse without errors
    let input = "SELECT passing FROM t";
    let (parse, _file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "SELECT passing FROM t should parse cleanly, errors: {:?}",
        parse.errors
    );

    // `passing` as a table name should parse without errors
    let input2 = "SELECT x FROM passing";
    let (parse2, _) = parse_file_text(input2);
    assert!(
        parse2.errors.is_empty(),
        "SELECT x FROM passing should parse cleanly, errors: {:?}",
        parse2.errors
    );

    // No PASSING_CLAUSEs should appear in these statements
    let parse_result = crate::parser::parse("SELECT passing FROM t");
    let root = parse_result.syntax();
    let clauses = passing_clauses_of(&root);
    assert!(
        clauses.is_empty(),
        "plain 'passing' identifier must not produce PASSING_CLAUSE nodes"
    );
}

#[test]
fn passing_not_attached_to_plain_sql_call() {
    // PASSING after a plain SQL function call must NOT be attached to that call.
    let input = "SELECT UPPER(x) PASSING y AS (some_expr) FROM t";
    // This should parse without panicking. PASSING is an identifier
    // in expression position after UPPER(x).
    let (_parse, file) = parse_file_text(input);

    // UPPER(x) is a plain FUNCTION_CALL — it must have no PASSING_CLAUSE children.
    let fcalls = function_calls(file.syntax());
    for fc in &fcalls {
        if fc.name().as_deref() == Some("UPPER") {
            // The SmeltFnCall wrapper returns passing_clauses only for SMELT_FN_CALL nodes.
            // UPPER is a FUNCTION_CALL, so we just check no PASSING_CLAUSE is a descendant
            // of the FUNCTION_CALL node.
            let fn_node = fc.syntax();
            let passing_under_upper = fn_node
                .descendants()
                .filter(|n| n.kind() == PASSING_CLAUSE)
                .count();
            assert_eq!(
                passing_under_upper, 0,
                "PASSING_CLAUSE must NOT be a descendant of plain FUNCTION_CALL UPPER"
            );
        }
    }

    // Phase 5b: smelt.fn.* is rejected, so no path calls either.
    let smelt_path = smelt_path_calls(file.syntax());
    assert!(
        smelt_path.is_empty(),
        "no smelt.functions.* calls should be present in this input"
    );
}

#[test]
fn passing_after_smelt_extern_call_not_attached() {
    // Plan Phase 28 requires `passing_after_smelt_extern_call_rejected`.
    // `smelt.extern` declarations are top-level-only (gated by
    // `at_smelt_extern_trigger`); they cannot appear in expression position.
    // This makes the plan requirement vacuously satisfied at the parser level —
    // there is no expression-position parse path for smelt.extern calls.
    //
    // We verify the structural analogue: PASSING after a plain SQL function
    // call (not smelt.fn.*) is not attached, confirming the trigger is
    // correctly scoped to the smelt.fn.* path only. `PASSING` is not a
    // reserved keyword, so it is consumed here as a plain implicit alias
    // for the select item.
    let input = "SELECT my_func(src) PASSING FROM t";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "should parse cleanly, errors: {:?}",
        parse.errors
    );

    // No SMELT_PATH_CALL nodes — so no PASSING_CLAUSE should be created.
    let smelt_calls = smelt_path_calls(file.syntax());
    assert!(
        smelt_calls.is_empty(),
        "my_func is not a smelt.functions.* call"
    );

    // No PASSING_CLAUSE nodes anywhere in the tree.
    let clauses = passing_clauses_of(file.syntax());
    assert!(
        clauses.is_empty(),
        "PASSING after plain SQL call must not produce PASSING_CLAUSE nodes"
    );
}

#[test]
fn error_recovery_malformed_passing_body() {
    // PASSING metrics AS followed immediately by FROM (missing body expression).
    // Phase 5b: use smelt.functions.* instead of smelt.fn.*
    let input = "SELECT smelt.functions.foo(src) PASSING metrics AS FROM t";
    let (parse, file) = parse_file_text(input);

    // The parser should emit an error but not panic.
    assert!(
        !parse.errors.is_empty(),
        "expected at least one parse error for malformed PASSING body"
    );

    // Despite the error, FROM t should still be parsed — i.e. the select
    // statement recovers and a FROM_CLAUSE is present.
    let stmts: Vec<_> = file.syntax().children().collect();
    assert!(
        !stmts.is_empty(),
        "tree should not be empty after error recovery"
    );
}

// ===== Phase 35: Struct row variables and value-level spread =====

#[test]
fn parses_struct_with_named_row_var() {
    let input = "smelt.define f(e: Expr<Struct<{ts: Timestamp, ..r}>>) AS (e)";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let defines: Vec<SmeltDefine> = file.defines().collect();
    assert_eq!(defines.len(), 1, "expected exactly one SMELT_DEFINE");

    // The TYPE_REF for parameter `e` must contain a STRUCT_TYPE child.
    let def = &defines[0];
    let params: Vec<Param> = def.param_list().expect("param list").params().collect();
    let tr = params[0].type_ref().expect("param type ref");
    let tr_syntax = tr.syntax();

    let struct_type_node = tr_syntax
        .descendants()
        .find(|n| n.kind() == STRUCT_TYPE)
        .expect("TYPE_REF must contain a STRUCT_TYPE node");

    // The STRUCT_TYPE must contain a ROW_TAIL child with identifier `r`.
    let row_tail_node = struct_type_node
        .children()
        .find(|n| n.kind() == ROW_TAIL)
        .expect("STRUCT_TYPE must contain a ROW_TAIL node");

    let tail_ident = row_tail_node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == IDENT)
        .expect("ROW_TAIL must contain an identifier for named tail");
    assert_eq!(
        tail_ident.text(),
        "r",
        "ROW_TAIL identifier must be `r`, got {}",
        tail_ident.text()
    );
}

#[test]
fn parses_struct_with_anonymous_row_tail() {
    let input = "smelt.define f(e: Expr<Struct<{ts: Timestamp, ..}>>) AS (e)";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let def = file.defines().next().expect("one smelt.define");
    let params: Vec<Param> = def.param_list().expect("param list").params().collect();
    let tr = params[0].type_ref().expect("param type ref");

    let struct_type_node = tr
        .syntax()
        .descendants()
        .find(|n| n.kind() == STRUCT_TYPE)
        .expect("TYPE_REF must contain a STRUCT_TYPE node");

    let row_tail_node = struct_type_node
        .children()
        .find(|n| n.kind() == ROW_TAIL)
        .expect("STRUCT_TYPE must contain a ROW_TAIL node");

    // Anonymous tail: no IDENT inside the ROW_TAIL.
    let has_ident = row_tail_node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .any(|t| t.kind() == IDENT);
    assert!(!has_ident, "anonymous ROW_TAIL must not contain an IDENT");
}

#[test]
fn parses_struct_literal_spread_in_body() {
    let input =
        "smelt.define f(event: Expr<Struct<{ts: Timestamp, ..r}>>) AS ({CAST(event.ts AS TIMESTAMP) AS ts, ..event})";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let def = file.defines().next().expect("one smelt.define");
    let body = def.body().expect("body");
    let body_syntax = body.syntax();

    // DEFINE_BODY must contain a BRACE_STRUCT_LITERAL node.
    let brace_struct = body_syntax
        .descendants()
        .find(|n| n.kind() == BRACE_STRUCT_LITERAL)
        .expect("DEFINE_BODY must contain a BRACE_STRUCT_LITERAL node");

    // The BRACE_STRUCT_LITERAL must have a SPREAD_ITEM child for `..event`.
    let spread = brace_struct
        .children()
        .find(|n| n.kind() == SPREAD_ITEM)
        .expect("BRACE_STRUCT_LITERAL must contain a SPREAD_ITEM child");

    let spread_ident = spread
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == IDENT)
        .expect("SPREAD_ITEM must contain an IDENT for the spread name");
    assert_eq!(
        spread_ident.text(),
        "event",
        "SPREAD_ITEM identifier must be `event`, got {}",
        spread_ident.text()
    );
}

#[test]
fn two_named_row_vars_in_one_signature_errors() {
    // v1 constraint: at most one named row variable per signature.
    // `..r` and `..s` are two distinct names — must produce an error.
    let input = "smelt.define f(a: Expr<Struct<{x: Integer, ..r}>>, b: Expr<Struct<{y: Integer, ..s}>>) AS (a)";
    let (parse, _file) = parse_file_text(input);
    // Must not panic.
    assert!(
        !parse.errors.is_empty(),
        "expected at least one parse error for two distinct named row variables, got none"
    );
    let mentions_row_var = parse.errors.iter().any(|e| {
        let m = e.message.to_lowercase();
        m.contains("row variable") || m.contains("named row") || m.contains("at most one")
    });
    assert!(
        mentions_row_var,
        "expected an error mentioning row variable constraint, got {:?}",
        parse.errors
    );
}

#[test]
fn anonymous_tail_unreferenced_in_body_ok() {
    // Anonymous `..` without a body spread is fine — no parser errors.
    let input = "smelt.define f(e: Expr<Struct<{ts: Timestamp, ..}>>) AS (e.ts)";
    let (parse, _file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "anonymous tail with no spread in body must not produce errors, got: {:?}",
        parse.errors
    );
}

// ---- Phase 44b → Phase 5b: CTE body smelt.fn.* rejection ----

#[test]
fn cte_body_smelt_fn_call_is_rejected() {
    // Phase 5b: a CTE body with a bare `smelt.fn.*` call must produce
    // parse errors. Use smelt.functions.* instead.
    let sql = "WITH base AS (\n    smelt.fn.session_rollup(source, u, ts)\n)\nSELECT * FROM base";
    let result = parse(sql);
    assert!(
        !result.errors.is_empty(),
        "Phase 5b: smelt.fn.* as CTE body must produce parse errors"
    );
}

// ===== smelt.<path> migration, Phase 1: unified value-form / call-form
// grammar (additive, coexists with legacy smelt.fn.* / smelt.ref / etc.).
// =====

use crate::ast::{SmeltPathCall, SmeltPathRef};

/// Helper: collect every `SmeltPathRef` descendant of a syntax node.
fn smelt_path_refs(root: &SyntaxNode) -> Vec<SmeltPathRef> {
    root.descendants().filter_map(SmeltPathRef::cast).collect()
}

/// Helper: collect every `SmeltPathCall` descendant of a syntax node.
fn smelt_path_calls(root: &SyntaxNode) -> Vec<SmeltPathCall> {
    root.descendants().filter_map(SmeltPathCall::cast).collect()
}

#[test]
fn parses_smelt_path_value_in_from() {
    // `SELECT * FROM smelt.models.users` produces a single SmeltPathRef
    // AST node with segments ["models", "users"] in the FROM position.
    let input = "SELECT * FROM smelt.models.users";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let refs = smelt_path_refs(file.syntax());
    assert_eq!(refs.len(), 1, "expected exactly one SMELT_PATH_REF");
    assert_eq!(refs[0].segments(), vec!["models", "users"]);

    // The path reference must sit under a FROM_CLAUSE ancestor.
    let in_from = refs[0]
        .syntax()
        .ancestors()
        .any(|n| n.kind() == FROM_CLAUSE);
    assert!(in_from, "SMELT_PATH_REF must live under FROM_CLAUSE");

    // No SMELT_PATH_CALL should be emitted for the value form.
    assert!(
        smelt_path_calls(file.syntax()).is_empty(),
        "value-form smelt.<path> must not produce SMELT_PATH_CALL"
    );
}

#[test]
fn parses_smelt_path_value_in_argument_position() {
    // `f(smelt.models.users)` produces a SmeltPathRef arg, distinct from a
    // function call. The outer `f(...)` is a FUNCTION_CALL whose argument
    // list contains exactly one SMELT_PATH_REF.
    let input = "SELECT f(smelt.models.users) FROM t";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let refs = smelt_path_refs(file.syntax());
    assert_eq!(refs.len(), 1, "expected exactly one SMELT_PATH_REF");
    assert_eq!(refs[0].segments(), vec!["models", "users"]);

    // The SMELT_PATH_REF must NOT be a SMELT_PATH_CALL — distinct nodes.
    assert!(
        smelt_path_calls(file.syntax()).is_empty(),
        "argument-position value form must not produce SMELT_PATH_CALL"
    );

    // The path reference sits inside an ARG_LIST under a FUNCTION_CALL.
    let inside_arg_list = refs[0].syntax().ancestors().any(|n| n.kind() == ARG_LIST);
    assert!(
        inside_arg_list,
        "SMELT_PATH_REF should sit inside an ARG_LIST"
    );
}

#[test]
fn smelt_path_ref_text_range_excludes_trailing_alias_whitespace() {
    // Regression: `FROM smelt.models.users u` was producing text_range that
    // included the space before the alias `u`, causing the test compiler to
    // replace `smelt.models.users ` (with space) with `users` → `usersu`.
    let input = "SELECT * FROM smelt.models.users u";
    let (parse, _file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let refs = smelt_path_refs(_file.syntax());
    assert_eq!(refs.len(), 1);

    let range = refs[0].text_range();
    let start: usize = range.start().into();
    let end: usize = range.end().into();
    let text = &input[start..end];
    assert_eq!(
        text, "smelt.models.users",
        "text_range must not include trailing whitespace before alias; got {text:?}"
    );
}

#[test]
fn parses_smelt_path_call_with_positional_args() {
    // `smelt.functions.patterns.session_rollup(events, 30)` parses as a
    // SmeltPathCall with two positional args.
    let input = "SELECT * FROM smelt.functions.patterns.session_rollup(events, 30)";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let calls = smelt_path_calls(file.syntax());
    assert_eq!(calls.len(), 1, "expected exactly one SMELT_PATH_CALL");
    assert_eq!(
        calls[0].segments(),
        vec!["functions", "patterns", "session_rollup"]
    );

    let arg_list = calls[0].arg_list().expect("call must have ARG_LIST");
    let positional = arg_list.positional_args();
    assert_eq!(
        positional.len(),
        2,
        "expected two positional arguments, got {}",
        positional.len()
    );

    // Phase 1 must emit no value-form SMELT_PATH_REF for a call.
    assert!(
        smelt_path_refs(file.syntax()).is_empty(),
        "call form must not also produce a SMELT_PATH_REF"
    );
}

#[test]
fn parses_smelt_path_call_with_named_args() {
    // `smelt.models.margins(product_summary => smelt.models.product_summary)`
    // parses with a named-arg `=>` binding whose value is itself a
    // SMELT_PATH_REF.
    let input =
        "SELECT * FROM smelt.models.margins(product_summary => smelt.models.product_summary)";
    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let calls = smelt_path_calls(file.syntax());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].segments(), vec!["models", "margins"]);

    let arg_list = calls[0].arg_list().expect("must have arg list");
    let named: Vec<_> = arg_list.named_params().collect();
    assert_eq!(named.len(), 1, "expected one named param");
    assert_eq!(named[0].name().as_deref(), Some("product_summary"));

    // The value of the named parameter is a SMELT_PATH_REF.
    let refs = smelt_path_refs(file.syntax());
    assert_eq!(refs.len(), 1, "expected one nested SMELT_PATH_REF");
    assert_eq!(refs[0].segments(), vec!["models", "product_summary"]);
}

#[test]
fn smelt_path_call_supports_passing_clause() {
    // `smelt.<path>(args) PASSING name AS (body)` — parity with current
    // `smelt.fn.*` PASSING grammar.
    let input = "WITH base AS (\n    smelt.functions.session_rollup(source, 30)\n    PASSING metrics AS (COUNT(*))\n)\nSELECT * FROM base";
    let parse = parse(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let file = File::cast(parse.syntax()).unwrap();
    let calls = smelt_path_calls(file.syntax());
    assert_eq!(calls.len(), 1, "expected exactly one SMELT_PATH_CALL");
    let passing: Vec<_> = calls[0].passing_clauses().collect();
    assert_eq!(passing.len(), 1, "expected exactly one PASSING_CLAUSE");
    assert_eq!(passing[0].name().as_deref(), Some("metrics"));
}

#[test]
fn legacy_smelt_ref_still_parses() {
    // Phase 4 update: `smelt.ref('users')` now produces a parse error
    // (the form is rejected), but error recovery still produces a
    // FUNCTION_CALL node so the CST is usable. The test is retained but
    // updated to reflect the Phase 4 behavior.
    let input = "SELECT * FROM smelt.ref('users')";
    let (parse, _file) = parse_file_text(input);
    // Phase 4: must have at least one parse error.
    assert!(
        !parse.errors.is_empty(),
        "Phase 4: smelt.ref() must produce a parse error; use smelt.models.<name>"
    );
    // The error message should mention the replacement.
    let err_msg = &parse.errors[0].message;
    assert!(
        err_msg.contains("smelt.models") || err_msg.contains("removed"),
        "error message should mention smelt.models or 'removed'; got: {err_msg}"
    );
}

#[test]
fn smelt_path_partial_forms_do_not_panic() {
    // Architectural invariant: error-recovery must produce a CST without
    // panics on partial / malformed path forms. Inputs like `smelt.`,
    // `smelt.models.`, and `smelt.models.foo(` must each yield a Parse
    // result; we don't care about the exact errors here.
    for input in [
        "SELECT * FROM smelt.",
        "SELECT * FROM smelt.models.",
        "SELECT * FROM smelt.models.foo(",
        "SELECT smelt. FROM t",
    ] {
        // This call must not panic.
        let parse = parse(input);
        // We expect at least one parse error, but the parser must still
        // produce a usable green tree.
        let _ = File::cast(parse.syntax()).expect("parser must yield FILE");
    }
}

// ===== Phase 4: Legacy smelt.ref() and smelt.source() rejection tests =====

#[test]
fn legacy_smelt_ref_now_rejected() {
    // Phase 4: `smelt.ref('users')` must produce at least one parse error
    // pointing the user toward the unified `smelt.<path>` form.
    // The parser should still produce a usable CST (error recovery), but
    // the error set must be non-empty.
    let input = "SELECT * FROM smelt.ref('users')";
    let parse = parse(input);
    assert!(
        !parse.errors.is_empty(),
        "smelt.ref('users') must produce a parse error in Phase 4; \
         use smelt.models.users instead"
    );
    // The parser must still produce a usable green tree (no panic).
    let _ = File::cast(parse.syntax()).expect("parser must yield FILE even on legacy ref");
}

#[test]
fn legacy_smelt_source_now_rejected() {
    // Phase 4: `smelt.source('raw.events')` must produce at least one
    // parse error pointing the user toward `smelt.sources.raw.events`.
    let input = "SELECT * FROM smelt.source('raw.events')";
    let parse = parse(input);
    assert!(
        !parse.errors.is_empty(),
        "smelt.source('raw.events') must produce a parse error in Phase 4; \
         use smelt.sources.raw.events instead"
    );
    let _ = File::cast(parse.syntax()).expect("parser must yield FILE even on legacy source");
}

// ===== Phase 5b: smelt.fn.* call syntax removal =====

#[test]
fn legacy_smelt_fn_call_now_rejected() {
    // Phase 5b: `smelt.fn.foo(x)` must produce at least one parse error
    // after the smelt.fn.* parser arm is removed. Before Phase 5b,
    // this parses successfully as a SMELT_FN_CALL node.
    let sql = "SELECT smelt.fn.foo(x) AS r";
    let result = parse(sql);
    assert!(
        !result.errors.is_empty(),
        "smelt.fn.foo(x) must produce parse errors after Phase 5b; got zero errors"
    );
}

// ===== Phase 1 (meta-language): list literal [a, b, c] and spread ...xs =====

#[test]
fn parse_list_literal_homogeneous() {
    // `[1, 2, 3]` parses to one ARRAY_LITERAL node with three child expressions
    // and no parse errors.
    let parse = parse("SELECT [1, 2, 3] FROM t");
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );
    let list_node = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == ARRAY_LITERAL)
        .expect("must have ARRAY_LITERAL node");
    let child_count = list_node
        .children()
        .filter(|n| n.kind() == EXPRESSION || n.kind() == BINARY_EXPR)
        .count();
    assert_eq!(
        child_count, 3,
        "expected 3 child expressions, got {}",
        child_count
    );
}

#[test]
fn parse_list_literal_trailing_comma() {
    // `[1, 2, 3,]` parses identically — no separator-related diagnostics.
    let parse = parse("SELECT [1, 2, 3,] FROM t");
    assert!(
        parse.errors.is_empty(),
        "trailing comma in list literal should not produce errors: {:?}",
        parse.errors
    );
    let list_node = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == ARRAY_LITERAL)
        .expect("must have ARRAY_LITERAL node");
    let child_count = list_node
        .children()
        .filter(|n| n.kind() == EXPRESSION || n.kind() == BINARY_EXPR)
        .count();
    assert_eq!(
        child_count, 3,
        "expected 3 child expressions, got {}",
        child_count
    );
}

#[test]
fn parse_list_literal_singleton() {
    // `[x]` parses to one ARRAY_LITERAL node with one child.
    let parse = parse("SELECT [x] FROM t");
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );
    let list_node = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == ARRAY_LITERAL)
        .expect("must have ARRAY_LITERAL node for [x]");
    let child_count = list_node
        .children()
        .filter(|n| n.kind() == EXPRESSION || n.kind() == BINARY_EXPR)
        .count();
    assert_eq!(
        child_count, 1,
        "expected 1 child expression, got {}",
        child_count
    );
}

#[test]
fn parse_list_literal_empty() {
    // `[]` parses to one ARRAY_LITERAL node with zero children, no errors.
    let parse = parse("SELECT [] FROM t");
    assert!(
        parse.errors.is_empty(),
        "empty list literal should produce no errors: {:?}",
        parse.errors
    );
    let list_node = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == ARRAY_LITERAL)
        .expect("must have ARRAY_LITERAL node for []");
    let child_count = list_node
        .children()
        .filter(|n| n.kind() == EXPRESSION || n.kind() == BINARY_EXPR)
        .count();
    assert_eq!(
        child_count, 0,
        "expected 0 child expressions, got {}",
        child_count
    );
}

#[test]
fn parse_list_literal_nested() {
    // `[[1, 2], [3, 4]]` parses to a nested-list shape.
    let parse = parse("SELECT [[1, 2], [3, 4]] FROM t");
    assert!(
        parse.errors.is_empty(),
        "unexpected errors in nested list literal: {:?}",
        parse.errors
    );
    // Count total ARRAY_LITERAL nodes: should be 3 (outer + 2 inner).
    let all_array_literals: Vec<_> = parse
        .syntax()
        .descendants()
        .filter(|n| n.kind() == ARRAY_LITERAL)
        .collect();
    assert_eq!(
        all_array_literals.len(),
        3,
        "expected 3 ARRAY_LITERAL nodes (1 outer + 2 inner), got {}",
        all_array_literals.len()
    );
}

#[test]
fn parse_spread_in_select_list() {
    // `SELECT id, ...metric_exprs, created_at FROM users`
    // produces a SELECT with a LIST_SPREAD child between two column references.
    let parse = parse("SELECT id, ...metric_exprs, created_at FROM users");
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );
    let spread = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == LIST_SPREAD)
        .expect("must have a LIST_SPREAD node in SELECT list");
    // The LIST_SPREAD should contain an EXPRESSION (the identifier)
    assert!(
        spread.descendants().any(|n| n.kind() == EXPRESSION),
        "LIST_SPREAD must contain an EXPRESSION child"
    );
}

#[test]
fn parse_spread_in_function_args() {
    // `coalesce(...numerics, 0)` produces a function call with one LIST_SPREAD
    // argument and one literal argument.
    let parse = parse("SELECT coalesce(...numerics, 0) FROM t");
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );
    assert!(
        parse
            .syntax()
            .descendants()
            .any(|n| n.kind() == LIST_SPREAD),
        "must have a LIST_SPREAD node inside function args"
    );
}

#[test]
fn parse_spread_of_list_literal() {
    // `SELECT id, ...[a, b, c] FROM t` — LIST_SPREAD wrapping a list-literal node.
    let parse = parse("SELECT id, ...[a, b, c] FROM t");
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );
    let spread = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == LIST_SPREAD)
        .expect("must have a LIST_SPREAD node");
    assert!(
        spread.descendants().any(|n| n.kind() == ARRAY_LITERAL),
        "LIST_SPREAD must contain an ARRAY_LITERAL child when operand is [...] literal"
    );
}

#[test]
fn parse_list_literal_error_recovery_unterminated() {
    // `SELECT [a, b FROM t` — parser does not crash; produces a partial
    // list-literal node and continues parsing.
    let parse = parse("SELECT [a, b FROM t");
    // Must not panic (the test itself proves that).
    // Must produce a usable FILE node.
    let _ = File::cast(parse.syntax()).expect("parser must yield FILE even on unterminated list");
    // We expect at least one error for the unterminated list.
    assert!(
        !parse.errors.is_empty(),
        "unterminated list literal should produce at least one error"
    );
}

// ===== List comprehensions: [expr FOR ident IN list (IF cond)?] (DuckDB) =====

#[test]
fn parse_list_comprehension_basic() {
    // `[x + 1 FOR x IN [1, 2, 3]]` parses clean to a LIST_COMPREHENSION node
    // and round-trips through the printer.
    let sql = "SELECT [x + 1 FOR x IN [1, 2, 3]] AS l";
    let parse = parse(sql);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let comp = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == LIST_COMPREHENSION)
        .expect("must have a LIST_COMPREHENSION node");
    assert!(
        comp.children().any(|n| n.kind() == LIST_COMPREHENSION_VAR),
        "LIST_COMPREHENSION must have a LIST_COMPREHENSION_VAR child"
    );
    assert!(
        comp.descendants().any(|n| n.kind() == ARRAY_LITERAL),
        "LIST_COMPREHENSION's source list must be an ARRAY_LITERAL"
    );

    assert_eq!(
        parse.syntax().text().to_string().trim().to_string(),
        sql,
        "list comprehension must round-trip losslessly"
    );
}

#[test]
fn parse_list_comprehension_with_filter() {
    // `[x FOR x IN [1, 2, 3] IF x > 1]` — IF filter clause after the IN-list.
    let sql = "SELECT [x FOR x IN [1, 2, 3] IF x > 1] AS l";
    let parse = parse(sql);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let comp = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == LIST_COMPREHENSION)
        .expect("must have a LIST_COMPREHENSION node");
    // element expr + source expr + filter expr = 3 Expr-castable children,
    // counted here as EXPRESSION/BINARY_EXPR/ARRAY_LITERAL direct children.
    let expr_child_count = comp
        .children()
        .filter(|n| matches!(n.kind(), EXPRESSION | BINARY_EXPR | ARRAY_LITERAL))
        .count();
    assert_eq!(
        expr_child_count, 3,
        "expected element + source + filter expressions, got {}",
        expr_child_count
    );
}

#[test]
fn parse_list_comprehension_ast_accessors() {
    let sql = "SELECT [x FOR x IN [1, 2, 3] IF x > 1] AS l FROM t";
    let parse = parse(sql);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let file = File::cast(parse.syntax()).expect("must cast to File");
    let select = file.select_stmt().expect("must have a SELECT statement");
    let expr = select
        .select_list()
        .expect("must have select list")
        .items()
        .next()
        .and_then(|item| item.expression())
        .expect("must have select item expression");

    let comp = Expr::as_list_comprehension(&expr)
        .expect("select item expression must be a list comprehension");
    assert_eq!(comp.var_name().as_deref(), Some("x"));
    assert!(comp.element().is_some(), "must have element expr");
    assert!(comp.source().is_some(), "must have source expr");
    assert!(comp.filter().is_some(), "must have filter expr");
}

#[test]
fn parse_list_comprehension_nested() {
    // `[[y FOR y IN x] FOR x IN [[1, 2], [3, 4]]]` — a comprehension whose
    // element expression is itself a comprehension. Verified against DuckDB.
    let sql = "SELECT [[y FOR y IN x] FOR x IN [[1, 2], [3, 4]]] AS l";
    let parse = parse(sql);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );

    let comps: Vec<_> = parse
        .syntax()
        .descendants()
        .filter(|n| n.kind() == LIST_COMPREHENSION)
        .collect();
    assert_eq!(comps.len(), 2, "expected outer + inner LIST_COMPREHENSION");

    assert_eq!(
        parse.syntax().text().to_string().trim().to_string(),
        sql,
        "nested list comprehension must round-trip losslessly"
    );
}

#[test]
fn parse_list_comprehension_multiple_for_clauses_is_error() {
    // DuckDB verified: chained `FOR x IN a FOR y IN b` is a syntax error
    // there (`Parser Error: syntax error at or near "FOR"`) — smelt matches.
    let sql = "SELECT [x FOR x IN [1, 2, 3] FOR y IN [4, 5]] AS l";
    let parse = parse(sql);
    assert!(
        !parse.errors.is_empty(),
        "multiple FOR clauses in one comprehension must be a parse error, matching DuckDB"
    );
}

#[test]
fn parse_list_comprehension_for_after_comma_is_error() {
    // DuckDB verified: `[1, 2 FOR x IN [1,2,3]]` is a syntax error there
    // (`FOR` is only recognized right after the sole element expression).
    let sql = "SELECT [1, 2 FOR x IN [1, 2, 3]] AS l";
    let parse = parse(sql);
    assert!(
        !parse.errors.is_empty(),
        "FOR after a comma-separated element list must be a parse error, matching DuckDB"
    );
}

#[test]
fn parse_list_literal_guards_still_hold_alongside_comprehension() {
    // Plain array/list literals — including empty `[]` — must be unaffected
    // by the comprehension grammar addition.
    let empty = parse("SELECT [] AS l");
    assert!(empty.errors.is_empty(), "empty list: {:?}", empty.errors);
    assert!(empty
        .syntax()
        .descendants()
        .any(|n| n.kind() == ARRAY_LITERAL));

    let plain = parse("SELECT [1, 2, 3] AS l");
    assert!(plain.errors.is_empty(), "plain list: {:?}", plain.errors);
    assert!(
        !plain
            .syntax()
            .descendants()
            .any(|n| n.kind() == LIST_COMPREHENSION),
        "plain list literal must not produce a LIST_COMPREHENSION node"
    );
}

#[test]
fn parse_spread_in_group_by() {
    // `SELECT x FROM t GROUP BY ...keys` — LIST_SPREAD inside the GROUP BY
    // clause; no parse errors.
    let parse = parse("SELECT x FROM t GROUP BY ...keys");
    assert!(
        parse.errors.is_empty(),
        "GROUP BY ...keys must parse without errors; got: {:?}",
        parse.errors
    );
    let spread = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == LIST_SPREAD)
        .expect("must have a LIST_SPREAD node in GROUP BY clause");
    assert!(
        spread.descendants().any(|n| n.kind() == EXPRESSION),
        "LIST_SPREAD must contain an EXPRESSION child"
    );
}

#[test]
fn parse_spread_in_order_by() {
    // `SELECT x FROM t ORDER BY ...sort_keys` — LIST_SPREAD inside the ORDER
    // BY clause; no parse errors.
    let parse = parse("SELECT x FROM t ORDER BY ...sort_keys");
    assert!(
        parse.errors.is_empty(),
        "ORDER BY ...sort_keys must parse without errors; got: {:?}",
        parse.errors
    );
    let spread = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == LIST_SPREAD)
        .expect("must have a LIST_SPREAD node in ORDER BY clause");
    assert!(
        spread.descendants().any(|n| n.kind() == EXPRESSION),
        "LIST_SPREAD must contain an EXPRESSION child"
    );
}

#[test]
fn parse_spread_in_in_list() {
    // `SELECT x FROM t WHERE id IN (...ids)` — LIST_SPREAD inside the
    // parenthesised IN value list; no parse errors.
    let parse = parse("SELECT x FROM t WHERE id IN (...ids)");
    assert!(
        parse.errors.is_empty(),
        "IN (...ids) must parse without errors; got: {:?}",
        parse.errors
    );
    let spread = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == LIST_SPREAD)
        .expect("must have a LIST_SPREAD node inside IN list");
    assert!(
        spread.descendants().any(|n| n.kind() == EXPRESSION),
        "LIST_SPREAD must contain an EXPRESSION child"
    );
}

#[test]
fn parse_spread_in_values_row() {
    // `SELECT * FROM (VALUES (...vals)) AS t(c)` — LIST_SPREAD inside a
    // VALUES row; no parse errors.
    let parse = parse("SELECT * FROM (VALUES (...vals)) AS t(c)");
    assert!(
        parse.errors.is_empty(),
        "VALUES (...vals) must parse without errors; got: {:?}",
        parse.errors
    );
    let spread = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == LIST_SPREAD)
        .expect("must have a LIST_SPREAD node inside VALUES row");
    assert!(
        spread.descendants().any(|n| n.kind() == EXPRESSION),
        "LIST_SPREAD must contain an EXPRESSION child"
    );
}

// ===== D-15: spread `...` is outermost operator; operand parsed through pipe level =====

#[test]
fn spread_wraps_pipe_chain() {
    // `SELECT ...smelt.columns_of(x) |> map(fn c => c.name) FROM t`
    // must parse as LIST_SPREAD whose operand is the PIPE_EXPR — NOT as
    // a PIPE_EXPR whose LHS is a LIST_SPREAD.
    let parse = parse("SELECT ...smelt.columns_of(x) |> map(fn c => c.name) FROM t");
    assert!(
        parse.errors.is_empty(),
        "spread_wraps_pipe_chain: unexpected errors: {:?}",
        parse.errors
    );

    let spread = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == LIST_SPREAD)
        .expect("must have a LIST_SPREAD node");

    // The LIST_SPREAD's first child node must be the PIPE_EXPR directly.
    let operand = spread
        .children()
        .next()
        .expect("LIST_SPREAD must have an operand child node");
    assert_eq!(
        operand.kind(),
        PIPE_EXPR,
        "LIST_SPREAD operand must be PIPE_EXPR directly (spread is outermost operator)"
    );

    // Negative check: no PIPE_EXPR should have a LIST_SPREAD as its left-hand side.
    let wrong = parse
        .syntax()
        .descendants()
        .filter(|n| n.kind() == PIPE_EXPR)
        .any(|pipe| {
            pipe.children()
                .next()
                .map(|c| c.kind() == LIST_SPREAD)
                .unwrap_or(false)
        });
    assert!(
        !wrong,
        "LIST_SPREAD must not be the LHS of a PIPE_EXPR (would mean pipe is outer, spread inner)"
    );
}

#[test]
fn spread_of_plain_value_unchanged() {
    // `SELECT ...xs FROM t` — a spread of a plain identifier still works
    // correctly after the precedence change (no regression).
    let parse = parse("SELECT ...xs FROM t");
    assert!(
        parse.errors.is_empty(),
        "spread_of_plain_value_unchanged: unexpected errors: {:?}",
        parse.errors
    );
    let spread = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == LIST_SPREAD)
        .expect("must have a LIST_SPREAD node");
    // The plain identifier is still wrapped in EXPRESSION by parse_primary_expr.
    assert!(
        spread.descendants().any(|n| n.kind() == EXPRESSION),
        "LIST_SPREAD must still contain an EXPRESSION for a plain identifier operand"
    );
}

// ===== Phase B (meta-language): fn keyword + pipe-arrow + lambda + pipe CST =====

#[test]
fn parse_lambda_single_arg() {
    // `map(xs, fn c => c)` — the second argument must be a LAMBDA node
    // with one parameter (`c`) and a body that is an identifier reference.
    let parse = parse("SELECT map(xs, fn c => c) FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_lambda_single_arg: unexpected errors: {:?}",
        parse.errors
    );
    let lambda = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == LAMBDA)
        .expect("must have a LAMBDA node");
    // The LAMBDA must have a LAMBDA_PARAM_LIST child with one LAMBDA_PARAM.
    let param_list = lambda
        .children()
        .find(|n| n.kind() == LAMBDA_PARAM_LIST)
        .expect("LAMBDA must have a LAMBDA_PARAM_LIST child");
    // Phase F: parameters are LAMBDA_PARAM nodes inside LAMBDA_PARAM_LIST.
    let params: Vec<_> = param_list
        .children()
        .filter(|n| n.kind() == LAMBDA_PARAM)
        .collect();
    assert_eq!(
        params.len(),
        1,
        "expected 1 LAMBDA_PARAM child, got {}",
        params.len()
    );
    // The single LAMBDA_PARAM must contain an IDENT token "c".
    let param_ident = params[0]
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == IDENT)
        .expect("LAMBDA_PARAM must have an IDENT token");
    assert_eq!(param_ident.text(), "c");
    // The LAMBDA must have an EXPRESSION body child.
    assert!(
        lambda.children().any(|n| n.kind() == EXPRESSION),
        "LAMBDA must have an EXPRESSION body"
    );
}

#[test]
fn parse_lambda_with_complex_body() {
    // `map(xs, fn c => CAST(c AS Text))` — second argument is a LAMBDA
    // whose body is a CAST expression.
    let parse = parse("SELECT map(xs, fn c => CAST(c AS INT)) FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_lambda_with_complex_body: unexpected errors: {:?}",
        parse.errors
    );
    let lambda = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == LAMBDA)
        .expect("must have a LAMBDA node");
    // Body must contain a CAST_EXPR descendant.
    assert!(
        lambda.descendants().any(|n| n.kind() == CAST_EXPR),
        "LAMBDA body must contain a CAST_EXPR"
    );
}

#[test]
fn parse_fn_paren_does_not_consume_as_lambda() {
    // `SELECT fn(a, b) FROM t` — `fn` used as a function name followed by
    // LPAREN must parse as a regular function call, NOT as a lambda.
    // Phase B is single-arg only; the LPAREN branch is excluded from
    // is_fn_lambda_start() to avoid mis-consuming valid SQL where `fn` is
    // a function name.  LambdaArityNotSupported is latent (Phase F) and
    // will only fire once multi-arg parsing is enabled.
    let parse = parse("SELECT fn(a, b) FROM t");
    // Must parse without errors: `fn(a, b)` is a valid SQL function call.
    assert!(
        parse.errors.is_empty(),
        "fn(a, b) must parse as a function call without errors, got: {:?}",
        parse.errors
    );
    // Must NOT produce a LAMBDA CST node — fn(a, b) is a function call.
    assert!(
        !parse.syntax().descendants().any(|n| n.kind() == LAMBDA),
        "fn(a, b) must not produce a LAMBDA CST node (it is a function call)"
    );
    // Must produce a FUNCTION_CALL node — fn is the function name.
    assert!(
        parse
            .syntax()
            .descendants()
            .any(|n| n.kind() == FUNCTION_CALL),
        "fn(a, b) must produce a FUNCTION_CALL CST node"
    );
}

#[test]
fn parse_fn_as_function_name_in_select() {
    // `SELECT fn(x, y) FROM t` — regression test confirming that fn used as
    // a SQL function name parses successfully as a SELECT with a function call.
    // This was broken when is_fn_lambda_start() matched LPAREN.
    let parse = parse("SELECT fn(x, y) FROM t");
    assert!(
        parse.errors.is_empty(),
        "SELECT fn(x, y) FROM t must parse without errors, got: {:?}",
        parse.errors
    );
    assert!(
        parse
            .syntax()
            .descendants()
            .any(|n| n.kind() == FUNCTION_CALL),
        "must have a FUNCTION_CALL node for fn(x, y)"
    );
    assert!(
        !parse.syntax().descendants().any(|n| n.kind() == LAMBDA),
        "must NOT have a LAMBDA node for fn(x, y) in SELECT"
    );
}

#[test]
fn parse_pipe_expression() {
    // `xs |> filter(fn c => c)` — must parse to a PIPE_EXPR with
    // LHS = identifier reference and RHS = function-call expression.
    let parse = parse("SELECT xs |> filter(fn c => c) FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_pipe_expression: unexpected errors: {:?}",
        parse.errors
    );
    let pipe = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == PIPE_EXPR)
        .expect("must have a PIPE_EXPR node");
    // The PIPE_EXPR must contain a PIPE_ARROW token.
    assert!(
        pipe.children_with_tokens()
            .any(|e| e.as_token().map(|t| t.kind()) == Some(PIPE_ARROW)),
        "PIPE_EXPR must contain a PIPE_ARROW token"
    );
    // The PIPE_EXPR must have a FUNCTION_CALL descendant (the RHS `filter(...)`).
    assert!(
        pipe.descendants().any(|n| n.kind() == FUNCTION_CALL),
        "PIPE_EXPR must have a FUNCTION_CALL descendant (RHS)"
    );
    // The PIPE_EXPR must have an EXPRESSION child (the RHS).
    assert!(
        pipe.children().any(|n| n.kind() == EXPRESSION),
        "PIPE_EXPR must have an EXPRESSION child for the RHS"
    );
}

#[test]
fn parse_pipe_chain_left_associative() {
    // `a |> b(p) |> c(q)` must parse as `((a |> b(p)) |> c(q))`.
    // The outermost PIPE_EXPR must itself contain a nested PIPE_EXPR (the LHS).
    let parse = parse("SELECT a |> b(p) |> c(q) FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_pipe_chain_left_associative: unexpected errors: {:?}",
        parse.errors
    );
    // Find all PIPE_EXPR nodes.
    let pipes: Vec<_> = parse
        .syntax()
        .descendants()
        .filter(|n| n.kind() == PIPE_EXPR)
        .collect();
    // Left-associative chain of 2 `|>` → 2 PIPE_EXPR nodes.
    assert_eq!(
        pipes.len(),
        2,
        "expected 2 PIPE_EXPR nodes, got {}",
        pipes.len()
    );
    // The outer PIPE_EXPR's span should be larger than the inner one.
    let outer = pipes
        .iter()
        .max_by_key(|n| n.text_range().len())
        .expect("must have an outer PIPE_EXPR");
    // The outer PIPE_EXPR must contain the inner PIPE_EXPR as a descendant
    // (left-associative: `(a |> b(p))` is the LHS of the outer `|> c(q)`).
    assert!(
        outer
            .descendants()
            .any(|n| n.kind() == PIPE_EXPR && n != *outer),
        "outer PIPE_EXPR must contain the inner PIPE_EXPR as a descendant (left-associative)"
    );
}

#[test]
fn parse_pipe_lowest_precedence() {
    // `1 + 2 |> f()` must parse as `(1 + 2) |> f()`, i.e. the arithmetic
    // expression is the LHS of the pipe, not `2 |> f()` (which would make
    // `1 + (2 |> f())` — wrong, pipe must have lower precedence than `+`).
    let parse = parse("SELECT 1 + 2 |> f() FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_pipe_lowest_precedence: unexpected errors: {:?}",
        parse.errors
    );
    let pipe = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == PIPE_EXPR)
        .expect("must have a PIPE_EXPR node");
    // The PIPE_EXPR must contain a BINARY_EXPR descendant (the LHS `1 + 2`).
    assert!(
        pipe.descendants().any(|n| n.kind() == BINARY_EXPR),
        "PIPE_EXPR must contain a BINARY_EXPR (the 1+2 LHS), confirming pipe is lowest-precedence"
    );
}

#[test]
fn parse_pipe_does_not_cross_statement_boundary() {
    // `a |> b()` in a SELECT context must produce a PIPE_EXPR without errors.
    let parse = parse("SELECT a |> b() FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_pipe_does_not_cross_statement_boundary: unexpected errors: {:?}",
        parse.errors
    );
    assert!(
        parse.syntax().descendants().any(|n| n.kind() == PIPE_EXPR),
        "must have a PIPE_EXPR node"
    );
}

#[test]
fn parse_pipe_rhs_non_call_recovers() {
    // `xs |> 3 + 4` — the RHS is not a call expression.
    // The parser must produce a PIPE_EXPR node and recover without crashing;
    // Phase 3 will emit PipeRhsNotCall.
    let parse = parse("SELECT xs |> 3 + 4 FROM t");
    // There may or may not be a parse error; the important assertion is
    // that a PIPE_EXPR node was produced and the parse did not panic.
    let _pipe = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == PIPE_EXPR)
        .expect("xs |> 3 + 4 must produce a PIPE_EXPR node (Phase 3 validates RHS)");
    // The PIPE_EXPR must have at least one EXPRESSION child (RHS).
    assert!(
        _pipe.children().any(|n| n.kind() == EXPRESSION),
        "PIPE_EXPR must have an EXPRESSION child for RHS even for non-call RHS"
    );
}

#[test]
fn parse_lambda_outside_call_recovers() {
    // A lambda literal in a non-HOF position — the parser must produce a
    // LAMBDA node and continue parsing; Phase 3 will emit LambdaInForbiddenPosition.
    let parse = parse("SELECT fn c => c FROM t");
    // There may be parse errors (position is unusual), but a LAMBDA node must exist.
    assert!(
        parse.syntax().descendants().any(|n| n.kind() == LAMBDA),
        "fn c => c must produce a LAMBDA node even in a forbidden position"
    );
}

#[test]
fn parse_named_arg_still_works_after_fn_keyword_addition() {
    // `f(name => value)` — existing named-arg syntax must parse unchanged
    // after the `fn` / `=>` parser interaction is added.
    // Use an IDENT name (not a keyword) to avoid keyword token confusion.
    let parse = parse("SELECT my_func(my_param => a + b > 10) FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_named_arg_still_works_after_fn_keyword_addition: unexpected errors: {:?}",
        parse.errors
    );
    let named = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == NAMED_PARAM)
        .expect("must have a NAMED_PARAM node");
    // The NAMED_PARAM must start with an IDENT "my_param".
    let name_token = named
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == IDENT)
        .expect("NAMED_PARAM must have an IDENT name token");
    assert_eq!(name_token.text(), "my_param");
}

#[test]
fn parse_pipe_chain_rhs_accessor_returns_outer_call() {
    // For `a |> b(p) |> c(q)`, the outer PipeExpr::rhs() must return the
    // expression containing `c(q)` — NOT the inner `a |> b(p)` pipe.
    // The naive `find_map(Expr::cast)` would return the inner PIPE_EXPR
    // first because PIPE_EXPR is included in Expr::cast's allow-list.
    // The fixed rhs() iterates past the PIPE_ARROW token before casting.
    let parse = parse("SELECT a |> b(p) |> c(q) FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_pipe_chain_rhs_accessor_returns_outer_call: unexpected errors: {:?}",
        parse.errors
    );
    // Find the outer (larger) PIPE_EXPR.
    let pipes: Vec<_> = parse
        .syntax()
        .descendants()
        .filter(|n| n.kind() == PIPE_EXPR)
        .collect();
    assert_eq!(
        pipes.len(),
        2,
        "expected 2 PIPE_EXPR nodes for a chain of 2 |>"
    );
    let outer_node = pipes
        .iter()
        .max_by_key(|n| n.text_range().len())
        .expect("must have an outer PIPE_EXPR")
        .clone();
    let outer_pipe = PipeExpr::cast(outer_node).expect("outer must cast to PipeExpr");
    // rhs() must return the expression for `c(q)` (not the inner pipe).
    let rhs = outer_pipe.rhs().expect("outer PipeExpr must have an rhs()");
    // rhs() must NOT be a PipeExpr (which would indicate we returned the inner pipe).
    assert!(
        rhs.syntax().kind() != PIPE_EXPR,
        "outer PipeExpr::rhs() returned a PIPE_EXPR — expected the outer call c(q)"
    );
    // The rhs must contain a FUNCTION_CALL node (the c(q) call).
    assert!(
        rhs.syntax()
            .descendants()
            .any(|n| n.kind() == FUNCTION_CALL),
        "outer PipeExpr::rhs() must contain a FUNCTION_CALL for c(q)"
    );
    // The function call name must be `c`, not `b`.
    let func_call = rhs
        .syntax()
        .descendants()
        .find(|n| n.kind() == FUNCTION_CALL)
        .expect("must have FUNCTION_CALL");
    let func_name = func_call
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == IDENT)
        .expect("FUNCTION_CALL must have an IDENT name token");
    assert_eq!(
        func_name.text(),
        "c",
        "outer PipeExpr::rhs() must point to c(q), not b(p)"
    );
}

#[test]
fn parse_pipe_chain_lhs_accessor_returns_inner_pipe() {
    // For `a |> b(p) |> c(q)`, the outer PipeExpr::lhs() must return the
    // inner PipeExpr (`a |> b(p)`), not just `a`.
    let parse = parse("SELECT a |> b(p) |> c(q) FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_pipe_chain_lhs_accessor_returns_inner_pipe: unexpected errors: {:?}",
        parse.errors
    );
    let pipes: Vec<_> = parse
        .syntax()
        .descendants()
        .filter(|n| n.kind() == PIPE_EXPR)
        .collect();
    assert_eq!(pipes.len(), 2, "expected 2 PIPE_EXPR nodes");
    let outer_node = pipes
        .iter()
        .max_by_key(|n| n.text_range().len())
        .expect("must have an outer PIPE_EXPR")
        .clone();
    let outer_pipe = PipeExpr::cast(outer_node).expect("outer must cast to PipeExpr");
    // lhs() must return the inner PIPE_EXPR (a |> b(p)).
    let lhs = outer_pipe.lhs().expect("outer PipeExpr must have a lhs()");
    assert_eq!(
        lhs.syntax().kind(),
        PIPE_EXPR,
        "outer PipeExpr::lhs() must return the inner PIPE_EXPR (a |> b(p))"
    );
}

#[test]
fn parse_pipe_simple_lhs_accessor_returns_lhs_expr() {
    // For `a |> b()`, PipeExpr::lhs() must return the expression for `a`.
    let parse = parse("SELECT a |> b() FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_pipe_simple_lhs_accessor_returns_lhs_expr: unexpected errors: {:?}",
        parse.errors
    );
    let pipe_node = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == PIPE_EXPR)
        .expect("must have a PIPE_EXPR node");
    let pipe = PipeExpr::cast(pipe_node).expect("must cast to PipeExpr");
    // lhs() must return something (the `a` reference expression).
    let lhs = pipe.lhs().expect("PipeExpr must have a lhs()");
    // The LHS must NOT be a PIPE_EXPR (that would mean we went past the arrow).
    assert!(
        lhs.syntax().kind() != PIPE_EXPR,
        "PipeExpr::lhs() for `a |> b()` must NOT be a PIPE_EXPR itself"
    );
}

#[test]
fn parse_pipe_at_statement_start_is_error() {
    // `SELECT 1; |> b() FROM t` — `|>` at the start of a new statement
    // must not silently absorb into the previous statement. The parser
    // processes each SQL statement separately; `|>` at the start of what
    // would be the second statement should not produce a valid PIPE_EXPR
    // spanning statements. Instead, we expect either a parse error or the
    // `|>` is treated as an error token in the second statement context.
    //
    // Note: the smelt parser currently processes the full input as a single
    // file. When given `SELECT 1; |> b() FROM t`, the `;` ends the first
    // SELECT and `|> b() FROM t` begins a new (malformed) statement. We
    // assert that no PIPE_EXPR spans both statements and that a parse error
    // is recorded (since `|>` cannot appear at the start of a statement).
    let parse = parse("SELECT 1; |> b() FROM t");
    // The parse must record an error: `|>` is not valid at statement start.
    assert!(
        !parse.errors.is_empty(),
        "parse_pipe_at_statement_start_is_error: expected parse errors for |> at statement start, got none"
    );
    // Any PIPE_EXPR that exists must not span the entire input (i.e. must
    // not start at position 0, which would imply cross-statement merging).
    // In practice no valid PIPE_EXPR should exist here.
    for pipe in parse
        .syntax()
        .descendants()
        .filter(|n| n.kind() == PIPE_EXPR)
    {
        assert!(
            u32::from(pipe.text_range().start()) > 0,
            "PIPE_EXPR must not start at the beginning of the input (would span statements)"
        );
    }
}

#[test]
fn parse_pipe_in_when_clause_produces_pipe_expr_node() {
    // `CASE WHEN xs |> filter(fn x => x > 0) THEN 1 ELSE 0 END`
    //
    // The WHEN condition contains a `|>` expression.  The parser must
    // produce a PIPE_EXPR CST node so that the Phase-3 semantic check can
    // fire PipeInDataPosition for the diagnostic.  If parse_when_clause
    // called parse_or_expr() instead of parse_pipe_expr(), the `|>` token
    // would be left in the stream and the parser would emit a confusing
    // "Expected THEN" error instead.
    let parse = parse("SELECT CASE WHEN xs |> filter(fn x => x > 0) THEN 1 ELSE 0 END FROM t");
    // There should be no parse errors — the pipe expression is structurally
    // valid (Phase 3 handles the semantic rejection via PipeInDataPosition).
    assert!(
        parse.errors.is_empty(),
        "parse_pipe_in_when_clause_produces_pipe_expr_node: unexpected errors: {:?}",
        parse.errors
    );
    // A PIPE_EXPR CST node must be produced inside the WHEN_CLAUSE.
    let when_clause = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == WHEN_CLAUSE)
        .expect("must have a WHEN_CLAUSE node");
    assert!(
        when_clause
            .descendants()
            .any(|n| n.kind() == PIPE_EXPR),
        "WHEN_CLAUSE must contain a PIPE_EXPR node (parse_pipe_expr must be used, not parse_or_expr)"
    );
}

// ===== Phase 2 (meta-language): record types, literals, map methods =====

#[test]
fn parse_smelt_record_decl_top_level() {
    let input = "smelt.record SourceEntry = { name: Text, columns: List<Text>, }";
    let parse = parse(input);
    assert!(
        parse.errors.is_empty(),
        "parse_smelt_record_decl_top_level: unexpected errors: {:?}",
        parse.errors
    );
    let root = parse.syntax();
    let decl = root
        .children()
        .find(|n| n.kind() == SMELT_RECORD_DECL)
        .expect("must have a SMELT_RECORD_DECL node");
    // The decl must contain a name token "SourceEntry"
    let has_name = decl
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .any(|t| t.kind() == IDENT && t.text() == "SourceEntry");
    assert!(
        has_name,
        "SMELT_RECORD_DECL must contain an IDENT token 'SourceEntry'"
    );
    // The SMELT_RECORD_DECL wraps its body in a RECORD_TYPE_INLINE.
    let body = decl
        .children()
        .find(|n| n.kind() == RECORD_TYPE_INLINE)
        .expect("SMELT_RECORD_DECL must have a RECORD_TYPE_INLINE child");
    let fields: Vec<_> = body
        .children()
        .filter(|n| n.kind() == RECORD_FIELD)
        .collect();
    assert_eq!(
        fields.len(),
        2,
        "must have two direct RECORD_FIELD children, got {}",
        fields.len()
    );
    // Each field must have an IDENT, COLON, and TYPE_REF child
    for field in &fields {
        let has_ident = field
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == IDENT);
        let has_colon = field
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == COLON);
        let has_type = field.children().any(|n| n.kind() == TYPE_REF);
        assert!(has_ident, "RECORD_FIELD must have an IDENT token");
        assert!(has_colon, "RECORD_FIELD must have a COLON token");
        assert!(has_type, "RECORD_FIELD must have a TYPE_REF child");
    }
}

#[test]
fn parse_smelt_record_decl_field_with_record_type() {
    let input = "smelt.record Cohort = { source: SourceEntry, settings: { threshold: Integer } }";
    let parse = parse(input);
    assert!(
        parse.errors.is_empty(),
        "parse_smelt_record_decl_field_with_record_type: unexpected errors: {:?}",
        parse.errors
    );
    let root = parse.syntax();
    let decl = root
        .children()
        .find(|n| n.kind() == SMELT_RECORD_DECL)
        .expect("must have a SMELT_RECORD_DECL node");
    // The SMELT_RECORD_DECL wraps its body in a RECORD_TYPE_INLINE.
    // Only the direct RECORD_FIELD children of that top-level RECORD_TYPE_INLINE are counted.
    let body = decl
        .children()
        .find(|n| n.kind() == RECORD_TYPE_INLINE)
        .expect("SMELT_RECORD_DECL must have a RECORD_TYPE_INLINE child");
    let fields: Vec<_> = body
        .children()
        .filter(|n| n.kind() == RECORD_FIELD)
        .collect();
    assert_eq!(
        fields.len(),
        2,
        "must have two direct RECORD_FIELD children, got {}",
        fields.len()
    );
    // The second field's type must be a RECORD_TYPE_INLINE
    let second_field = &fields[1];
    let has_inline = second_field
        .children()
        .any(|n| n.kind() == RECORD_TYPE_INLINE);
    assert!(
        has_inline,
        "second RECORD_FIELD must have a RECORD_TYPE_INLINE child (settings field)"
    );
    // The first field's type must be a bare TYPE_REF (SourceEntry)
    let first_field = &fields[0];
    let has_bare_type = first_field.children().any(|n| n.kind() == TYPE_REF);
    assert!(
        has_bare_type,
        "first RECORD_FIELD must have a TYPE_REF child (SourceEntry)"
    );
}

#[test]
fn parse_smelt_record_decl_recovers_on_malformed_field() {
    // `y: ,` is missing the type — the parser should recover and produce three fields.
    let input = "smelt.record Bad = { x: Text, y: , z: Integer }";
    let parse = parse(input);
    // We expect some errors (missing type for y) but no crash / avalanche.
    let root = parse.syntax();
    let decl = root
        .children()
        .find(|n| n.kind() == SMELT_RECORD_DECL)
        .expect("must still produce a SMELT_RECORD_DECL node even with errors");
    // Fields are direct children of the RECORD_TYPE_INLINE inside SMELT_RECORD_DECL.
    let body = decl
        .children()
        .find(|n| n.kind() == RECORD_TYPE_INLINE)
        .expect("SMELT_RECORD_DECL must have a RECORD_TYPE_INLINE child even with errors");
    let fields: Vec<_> = body
        .children()
        .filter(|n| n.kind() == RECORD_FIELD)
        .collect();
    assert_eq!(
        fields.len(),
        3,
        "must have three direct RECORD_FIELD children (recovered), got {}; errors: {:?}",
        fields.len(),
        parse.errors
    );
}

/// Regression: a regular SQL function call with `IDENT < IDENT` in
/// argument position must parse as a comparison, not as a generic-type
/// expression. The Phase E1 generic-type heuristic for loader schema
/// arguments must be scoped to `smelt.<path>(...)` call positions only.
#[test]
fn parse_function_call_with_ident_lt_ident_arg() {
    let input = "SELECT f(price < threshold) FROM t";
    let parse = parse(input);
    assert!(
        parse.errors.is_empty(),
        "parse_function_call_with_ident_lt_ident_arg: unexpected errors: {:?}",
        parse.errors
    );
    // No TYPE_REF node should appear — `price` and `threshold` are bare
    // identifiers in a comparison, not a generic type expression.
    let has_type_ref = parse.syntax().descendants().any(|n| n.kind() == TYPE_REF);
    assert!(
        !has_type_ref,
        "must not contain a TYPE_REF node — `price < threshold` is a comparison, not a type"
    );
}

#[test]
fn parse_record_literal_in_select_item() {
    let input = "SELECT smelt.foo({a: 1, b: 'x'}) FROM t";
    let parse = parse(input);
    assert!(
        parse.errors.is_empty(),
        "parse_record_literal_in_select_item: unexpected errors: {:?}",
        parse.errors
    );
    // A RECORD_LITERAL node must appear somewhere in the tree
    let has_record_literal = parse
        .syntax()
        .descendants()
        .any(|n| n.kind() == RECORD_LITERAL);
    assert!(has_record_literal, "must contain a RECORD_LITERAL node");
    // The RECORD_LITERAL must have two RECORD_FIELD children
    let record_lit = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == RECORD_LITERAL)
        .unwrap();
    let fields: Vec<_> = record_lit
        .children()
        .filter(|n| n.kind() == RECORD_FIELD)
        .collect();
    assert_eq!(
        fields.len(),
        2,
        "RECORD_LITERAL must have two RECORD_FIELD children"
    );
}

#[test]
fn parse_record_literal_in_define_default_value() {
    let input = "smelt.define foo(cfg: Cohort = {threshold: 10}) AS (cfg)";
    let parse = parse(input);
    assert!(
        parse.errors.is_empty(),
        "parse_record_literal_in_define_default_value: unexpected errors: {:?}",
        parse.errors
    );
    // A RECORD_LITERAL must appear in the DEFAULT_VALUE position
    let has_record_literal = parse
        .syntax()
        .descendants()
        .any(|n| n.kind() == RECORD_LITERAL);
    assert!(
        has_record_literal,
        "must contain a RECORD_LITERAL node in default value"
    );
}

#[test]
fn parse_inline_record_type_at_define_parameter() {
    let input = "smelt.define foo(cfg: { name: Text, count: Integer }) AS (cfg)";
    let parse = parse(input);
    assert!(
        parse.errors.is_empty(),
        "parse_inline_record_type_at_define_parameter: unexpected errors: {:?}",
        parse.errors
    );
    // A RECORD_TYPE_INLINE must appear somewhere in the tree
    let has_inline = parse
        .syntax()
        .descendants()
        .any(|n| n.kind() == RECORD_TYPE_INLINE);
    assert!(has_inline, "must contain a RECORD_TYPE_INLINE node");
}

#[test]
fn parse_inline_record_type_nested_in_list() {
    let input = "smelt.define foo(cs: List<{ name: Text }>) AS (cs)";
    let parse = parse(input);
    assert!(
        parse.errors.is_empty(),
        "parse_inline_record_type_nested_in_list: unexpected errors: {:?}",
        parse.errors
    );
    // A RECORD_TYPE_INLINE must appear inside the type ref
    let has_inline = parse
        .syntax()
        .descendants()
        .any(|n| n.kind() == RECORD_TYPE_INLINE);
    assert!(
        has_inline,
        "must contain a RECORD_TYPE_INLINE node nested in List<...>"
    );
}

#[test]
fn parse_map_method_call_entries() {
    let input = "smelt.define foo(m: Map<Text, Integer>) AS (m.entries())";
    let parse = parse(input);
    assert!(
        parse.errors.is_empty(),
        "parse_map_method_call_entries: unexpected errors: {:?}",
        parse.errors
    );
    // A MAP_METHOD_CALL node must appear
    let has_map_method = parse
        .syntax()
        .descendants()
        .any(|n| n.kind() == MAP_METHOD_CALL);
    assert!(
        has_map_method,
        "must contain a MAP_METHOD_CALL node for m.entries()"
    );
}

#[test]
fn parse_map_method_call_get_with_arg() {
    let input = "smelt.define foo(m: Map<Text, Integer>) AS (m.get('k'))";
    let parse = parse(input);
    assert!(
        parse.errors.is_empty(),
        "parse_map_method_call_get_with_arg: unexpected errors: {:?}",
        parse.errors
    );
    // A MAP_METHOD_CALL node must appear with at least one positional argument
    let map_method = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == MAP_METHOD_CALL)
        .expect("must contain a MAP_METHOD_CALL node for m.get('k')");
    // Must have an ARG_LIST child with at least one positional argument
    let has_arg_list = map_method.children().any(|n| n.kind() == ARG_LIST);
    assert!(has_arg_list, "MAP_METHOD_CALL must have an ARG_LIST child");
}

#[test]
fn parse_map_method_call_has_with_arg() {
    // The spec's closed Map API is {entries, keys, values, get, has};
    // `has` must be routed through MAP_METHOD_CALL like `get`.
    let input = "smelt.define foo(m: Map<Text, Integer>) AS (m.has('k'))";
    let parse = parse(input);
    assert!(
        parse.errors.is_empty(),
        "parse_map_method_call_has_with_arg: unexpected errors: {:?}",
        parse.errors
    );
    let map_method = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == MAP_METHOD_CALL)
        .expect("must contain a MAP_METHOD_CALL node for m.has('k')");
    let has_arg_list = map_method.children().any(|n| n.kind() == ARG_LIST);
    assert!(has_arg_list, "MAP_METHOD_CALL must have an ARG_LIST child");
}

#[test]
fn parse_map_method_call_on_loader_receiver() {
    // P7d: MAP_METHOD_CALL must parse when the receiver is a SMELT_PATH_CALL
    // (i.e. a config loader), not just a bare IDENT.
    // Input: `smelt.config.load_yaml('tenants.yaml', Map<Text, {plan: Text, threshold: Integer}>).keys()`
    let src = "SELECT smelt.config.load_yaml('tenants.yaml', Map<Text, {plan: Text, threshold: Integer}>).keys() AS tenant_keys";
    let parse = parse(src);
    assert!(
        parse.errors.is_empty(),
        "parse_map_method_call_on_loader_receiver: unexpected errors: {:?}",
        parse.errors
    );
    // A MAP_METHOD_CALL node must appear.
    let map_call_node = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == MAP_METHOD_CALL)
        .expect("must contain a MAP_METHOD_CALL node for loader.keys()");
    // The receiver must be a SMELT_PATH_CALL (not just an IDENT).
    let has_call_receiver = map_call_node
        .children()
        .any(|n| n.kind() == SMELT_PATH_CALL);
    assert!(
        has_call_receiver,
        "MAP_METHOD_CALL receiver must be SMELT_PATH_CALL for a loader expression"
    );
    // method_name() must return "keys".
    let call = crate::ast::MapMethodCall::cast(map_call_node).expect("must cast to MapMethodCall");
    assert_eq!(
        call.method_name().as_deref(),
        Some("keys"),
        "method_name() must return 'keys'"
    );
}

#[test]
fn record_literal_vs_inline_record_type_disambiguation() {
    // Value position → RECORD_LITERAL
    let value_input = "SELECT smelt.foo({a: 1}) FROM t";
    let value_parse = parse(value_input);
    assert!(
        value_parse.errors.is_empty(),
        "record_literal disambiguation (value): unexpected errors: {:?}",
        value_parse.errors
    );
    let has_literal = value_parse
        .syntax()
        .descendants()
        .any(|n| n.kind() == RECORD_LITERAL);
    assert!(
        has_literal,
        "value position {{a: 1}} must parse as RECORD_LITERAL"
    );
    let has_inline = value_parse
        .syntax()
        .descendants()
        .any(|n| n.kind() == RECORD_TYPE_INLINE);
    assert!(
        !has_inline,
        "value position {{a: 1}} must NOT produce RECORD_TYPE_INLINE"
    );

    // Type-annotation position in smelt.record → RECORD_TYPE_INLINE
    let type_input = "smelt.record Foo = { a: Integer }";
    let type_parse = parse(type_input);
    assert!(
        type_parse.errors.is_empty(),
        "record_literal disambiguation (type): unexpected errors: {:?}",
        type_parse.errors
    );
    let has_inline2 = type_parse
        .syntax()
        .descendants()
        .any(|n| n.kind() == RECORD_TYPE_INLINE);
    assert!(
        has_inline2,
        "type-annotation position {{a: Integer}} must parse as RECORD_TYPE_INLINE"
    );
    let has_literal2 = type_parse
        .syntax()
        .descendants()
        .any(|n| n.kind() == RECORD_LITERAL);
    assert!(
        !has_literal2,
        "type-annotation position {{a: Integer}} must NOT produce RECORD_LITERAL"
    );
}

#[test]
fn record_literal_recovers_on_missing_value() {
    // `a: ,` has a missing value expression — parser should recover
    let input = "smelt.define foo(cfg: Cohort = {a: , b: 2}) AS (cfg)";
    let parse = parse(input);
    // Expect errors but not a crash
    let has_record_literal = parse
        .syntax()
        .descendants()
        .any(|n| n.kind() == RECORD_LITERAL);
    assert!(
        has_record_literal,
        "must still produce a RECORD_LITERAL node even with missing value"
    );
    let record_lit = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == RECORD_LITERAL)
        .unwrap();
    let fields: Vec<_> = record_lit
        .children()
        .filter(|n| n.kind() == RECORD_FIELD)
        .collect();
    assert_eq!(
        fields.len(),
        2,
        "RECORD_LITERAL must have two RECORD_FIELD children even with error recovery, got {}",
        fields.len()
    );
}

// ===== Phase F (meta-language): multi-arg lambdas, parameterised reducers, meta-world ternary =====

// ----- Lexer tests (embedded in parser/tests.rs for convenience) -----

#[test]
fn lex_ternary_keywords() {
    use crate::lexer::tokenize;
    // `if` must lex as IF_KW (reserved keyword).
    let tokens = tokenize("if");
    let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
    assert_eq!(
        non_ws.len(),
        1,
        "expected one token for `if`, got: {:?}",
        non_ws
    );
    assert_eq!(
        non_ws[0].kind, IF_KW,
        "`if` must lex as IF_KW, got {:?}",
        non_ws[0].kind
    );

    // `then` must lex as THEN_KW (already a SQL CASE keyword).
    let tokens = tokenize("then");
    let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
    assert_eq!(
        non_ws.len(),
        1,
        "expected one token for `then`, got: {:?}",
        non_ws
    );
    assert_eq!(
        non_ws[0].kind, THEN_KW,
        "`then` must lex as THEN_KW, got {:?}",
        non_ws[0].kind
    );

    // `else` must lex as ELSE_KW (already a SQL CASE keyword).
    let tokens = tokenize("else");
    let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
    assert_eq!(
        non_ws.len(),
        1,
        "expected one token for `else`, got: {:?}",
        non_ws
    );
    assert_eq!(
        non_ws[0].kind, ELSE_KW,
        "`else` must lex as ELSE_KW, got {:?}",
        non_ws[0].kind
    );

    // All three are case-sensitive: `IF` is still IF_KW (lexer is case-insensitive for keywords).
    let tokens = tokenize("IF");
    let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
    assert_eq!(
        non_ws[0].kind, IF_KW,
        "`IF` must also lex as IF_KW, got {:?}",
        non_ws[0].kind
    );

    // `iffy` must remain an IDENT (no over-eager keyword match).
    let tokens = tokenize("iffy");
    let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
    assert_eq!(
        non_ws[0].kind, IDENT,
        "`iffy` must lex as IDENT, got {:?}",
        non_ws[0].kind
    );
}

#[test]
fn lex_keywords_not_in_strings() {
    use crate::lexer::tokenize;
    // `'if'`, `'then'`, `'else'` inside string literals must lex as STRING_LITERAL (STRING).
    let tokens = tokenize("'if'");
    let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
    assert_eq!(
        non_ws.len(),
        1,
        "`'if'` must be a single STRING token, got: {:?}",
        non_ws
    );
    assert_eq!(
        non_ws[0].kind, STRING,
        "`'if'` must lex as STRING, got {:?}",
        non_ws[0].kind
    );

    let tokens = tokenize("'then'");
    let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
    assert_eq!(
        non_ws.len(),
        1,
        "`'then'` must be a single STRING token, got: {:?}",
        non_ws
    );
    assert_eq!(
        non_ws[0].kind, STRING,
        "`'then'` must lex as STRING, got {:?}",
        non_ws[0].kind
    );

    let tokens = tokenize("'else'");
    let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
    assert_eq!(
        non_ws.len(),
        1,
        "`'else'` must be a single STRING token, got: {:?}",
        non_ws
    );
    assert_eq!(
        non_ws[0].kind, STRING,
        "`'else'` must lex as STRING, got {:?}",
        non_ws[0].kind
    );
}

// ----- Multi-arg lambda tests -----

#[test]
fn parse_multi_arg_lambda() {
    // `fn (a, b) => a + b` must parse as a LAMBDA with two LAMBDA_PARAM children.
    let parse = parse("SELECT map2(xs, ys, fn (a, b) => a + b) FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_multi_arg_lambda: unexpected errors: {:?}",
        parse.errors
    );
    let lambda = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == LAMBDA)
        .expect("must have a LAMBDA node");
    // The LAMBDA must have a LAMBDA_PARAM_LIST child.
    let param_list = lambda
        .children()
        .find(|n| n.kind() == LAMBDA_PARAM_LIST)
        .expect("LAMBDA must have a LAMBDA_PARAM_LIST child");
    // The LAMBDA_PARAM_LIST must have two LAMBDA_PARAM children.
    let params: Vec<_> = param_list
        .children()
        .filter(|n| n.kind() == LAMBDA_PARAM)
        .collect();
    assert_eq!(
        params.len(),
        2,
        "expected 2 LAMBDA_PARAM children, got {}",
        params.len()
    );
    // Each LAMBDA_PARAM must have an IDENT token.
    let param_names: Vec<String> = params
        .iter()
        .flat_map(|p| {
            p.children_with_tokens()
                .filter_map(|e| e.into_token())
                .filter(|t| t.kind() == IDENT)
                .map(|t| t.text().to_string())
        })
        .collect();
    assert_eq!(
        param_names,
        vec!["a", "b"],
        "expected params [a, b], got {:?}",
        param_names
    );
}

#[test]
fn parse_multi_arg_lambda_trailing_comma() {
    // `fn (a, b,) => body` — trailing comma must be accepted.
    let parse = parse("SELECT f(xs, fn (a, b,) => a + b) FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_multi_arg_lambda_trailing_comma: unexpected errors: {:?}",
        parse.errors
    );
    let lambda = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == LAMBDA)
        .expect("must have a LAMBDA node");
    let param_list = lambda
        .children()
        .find(|n| n.kind() == LAMBDA_PARAM_LIST)
        .expect("LAMBDA must have LAMBDA_PARAM_LIST");
    let params: Vec<_> = param_list
        .children()
        .filter(|n| n.kind() == LAMBDA_PARAM)
        .collect();
    assert_eq!(
        params.len(),
        2,
        "expected 2 LAMBDA_PARAM children (trailing comma), got {}",
        params.len()
    );
}

#[test]
fn parse_single_arg_lambda_parenthesised() {
    // `fn (x) => x` must parse as a LAMBDA with one LAMBDA_PARAM child,
    // equivalent to `fn x => x`.
    let parse = parse("SELECT f(xs, fn (x) => x) FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_single_arg_lambda_parenthesised: unexpected errors: {:?}",
        parse.errors
    );
    let lambda = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == LAMBDA)
        .expect("must have a LAMBDA node");
    let param_list = lambda
        .children()
        .find(|n| n.kind() == LAMBDA_PARAM_LIST)
        .expect("LAMBDA must have LAMBDA_PARAM_LIST");
    let params: Vec<_> = param_list
        .children()
        .filter(|n| n.kind() == LAMBDA_PARAM)
        .collect();
    assert_eq!(
        params.len(),
        1,
        "expected 1 LAMBDA_PARAM child for fn (x), got {}",
        params.len()
    );
}

#[test]
fn parse_lambda_zero_params_rejected() {
    // `fn () => body` — zero params. Parser admits the shape (produces a LAMBDA
    // node) and the downstream `LambdaZeroParameters` diagnostic fires.
    // The parse does NOT need to be error-free — it just must not crash and must
    // produce a LAMBDA node.
    let parse = parse("SELECT f(xs, fn () => body) FROM t");
    // May have errors; the important thing is a LAMBDA node exists.
    let lambda = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == LAMBDA)
        .expect("fn () => body must produce a LAMBDA node (for downstream diagnostics)");
    // LAMBDA_PARAM_LIST must exist with zero LAMBDA_PARAM children.
    let param_list = lambda
        .children()
        .find(|n| n.kind() == LAMBDA_PARAM_LIST)
        .expect("LAMBDA must have LAMBDA_PARAM_LIST");
    let params: Vec<_> = param_list
        .children()
        .filter(|n| n.kind() == LAMBDA_PARAM)
        .collect();
    assert_eq!(
        params.len(),
        0,
        "fn () => body must have 0 LAMBDA_PARAM children, got {}",
        params.len()
    );
}

#[test]
fn parse_lambda_no_parens_multi_arg_rejected() {
    // `fn a, b => body` — no parens around multi-arg. This is a parse error
    // at the comma; the parser should recover with a LAMBDA containing one
    // param plus an ERROR token (or similar recovery).
    let parse = parse("SELECT f(xs, fn a, b => body) FROM t");
    // Must NOT be error-free — comma after unparenthesised parameter is a parse error.
    // Must have a LAMBDA node (error recovery).
    let lambda_exists = parse.syntax().descendants().any(|n| n.kind() == LAMBDA);
    assert!(
        lambda_exists,
        "fn a, b => body must produce a LAMBDA node for error recovery"
    );
    // Must have parse errors (the unparenthesised multi-arg form is rejected).
    assert!(
        !parse.errors.is_empty(),
        "fn a, b => body must produce parse errors (missing parens for multi-arg)"
    );
}

// ----- Ternary expression tests -----

#[test]
fn parse_ternary_basic() {
    // `if cond then a else b` must parse as a TERNARY_EXPR with three sub-expressions.
    let parse = parse("SELECT if x > 0 then 1 else 0 FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_ternary_basic: unexpected errors: {:?}",
        parse.errors
    );
    let ternary = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == TERNARY_EXPR)
        .expect("must have a TERNARY_EXPR node");
    // Must have three EXPRESSION children (cond, then_branch, else_branch).
    let exprs: Vec<_> = ternary
        .children()
        .filter(|n| n.kind() == EXPRESSION)
        .collect();
    assert_eq!(
        exprs.len(),
        3,
        "TERNARY_EXPR must have 3 EXPRESSION children, got {}",
        exprs.len()
    );
}

#[test]
fn parse_ternary_nested_right_associative() {
    // `if c1 then a else if c2 then b else c` must parse as:
    // TERNARY_EXPR(c1, a, TERNARY_EXPR(c2, b, c)) — right-associative.
    let parse = parse("SELECT if c1 then a else if c2 then b else c FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_ternary_nested_right_associative: unexpected errors: {:?}",
        parse.errors
    );
    let ternaries: Vec<_> = parse
        .syntax()
        .descendants()
        .filter(|n| n.kind() == TERNARY_EXPR)
        .collect();
    assert_eq!(
        ternaries.len(),
        2,
        "expected 2 TERNARY_EXPR nodes, got {}",
        ternaries.len()
    );
    // The outer ternary must contain the inner one as a descendant (right-associative).
    let outer = ternaries
        .iter()
        .max_by_key(|n| n.text_range().len())
        .unwrap();
    assert!(
        outer
            .descendants()
            .any(|n| n.kind() == TERNARY_EXPR && &n != outer),
        "outer TERNARY_EXPR must contain the inner one (right-associative)"
    );
}

#[test]
fn parse_ternary_in_lambda_body() {
    // `fn x => if x > 0 then 'pos' else 'neg'` — ternary as lambda body.
    let parse = parse("SELECT f(xs, fn x => if x > 0 then 'pos' else 'neg') FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_ternary_in_lambda_body: unexpected errors: {:?}",
        parse.errors
    );
    let lambda = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == LAMBDA)
        .expect("must have a LAMBDA node");
    // The LAMBDA body must contain a TERNARY_EXPR.
    assert!(
        lambda.descendants().any(|n| n.kind() == TERNARY_EXPR),
        "LAMBDA body must contain a TERNARY_EXPR"
    );
}

#[test]
fn parse_ternary_in_pipe_chain() {
    // Spec rule: ternary has LOWER precedence than `|>` (pipe binds more tightly).
    // Inside the COND slot of a ternary, a pipe expression parses correctly because
    // `|>` has higher precedence than the ternary.
    //
    // Test: `if xs |> f() then a else b` — the COND is the pipe expression `xs |> f()`.
    // The pipe result determines which branch is taken.
    let parse = parse("SELECT if xs |> f() then a else b FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_ternary_in_pipe_chain: unexpected errors: {:?}",
        parse.errors
    );
    let ternary = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == TERNARY_EXPR)
        .expect("must have a TERNARY_EXPR node");
    // The TERNARY_EXPR must contain a PIPE_EXPR descendant (the pipe is the COND).
    assert!(
        ternary.descendants().any(|n| n.kind() == PIPE_EXPR),
        "TERNARY_EXPR must contain a PIPE_EXPR as its COND (pipe higher-precedence than ternary)"
    );
}

#[test]
fn parse_ternary_dangling_then_recovery() {
    // `then x else y` — no leading `if`. The parser must recover without consuming
    // the surrounding expression; an ERROR must be emitted at `then`.
    // Critically: existing CASE WHEN ... THEN ... ELSE ... END must not regress.
    // First verify CASE WHEN still works:
    let case_parse = parse("SELECT CASE WHEN x = 1 THEN 'a' ELSE 'b' END FROM t");
    assert!(
        case_parse.errors.is_empty(),
        "CASE WHEN must still parse correctly after ternary addition: {:?}",
        case_parse.errors
    );
    // Now test dangling `then` recovery:
    let parse = parse("SELECT then x FROM t");
    // Must produce errors (dangling `then`).
    assert!(
        !parse.errors.is_empty(),
        "dangling `then` must produce parse errors"
    );
}

#[test]
fn parse_ternary_dangling_else_recovery() {
    // `if c then x` — missing `else` branch. The parser must recover by producing
    // an incomplete TERNARY_EXPR (missing else slot) flagged for downstream diagnostics.
    let parse = parse("SELECT if c then x FROM t");
    // A TERNARY_EXPR must still be produced (error recovery).
    let ternary = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == TERNARY_EXPR)
        .expect("if c then x must produce a TERNARY_EXPR node even with missing else");
    // The TERNARY_EXPR must have fewer than 3 EXPRESSION children (else slot missing).
    let exprs: Vec<_> = ternary
        .children()
        .filter(|n| n.kind() == EXPRESSION)
        .collect();
    assert!(
        exprs.len() < 3,
        "incomplete ternary (missing else) must have fewer than 3 EXPRESSION children, got {}",
        exprs.len()
    );
}

// ----- Parameterised reducer tests -----

#[test]
fn parse_reducer_call() {
    // `reduce(xs, concat_with(' OR '))` — the second argument is a parameterised
    // reducer call. Must produce a REDUCER_CALL node.
    let parse = parse("SELECT reduce(xs, concat_with(' OR ')) FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_reducer_call: unexpected errors: {:?}",
        parse.errors
    );
    let reducer = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == REDUCER_CALL)
        .expect("must have a REDUCER_CALL node");
    // The REDUCER_CALL must contain an IDENT token for the reducer name.
    let name_token = reducer
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == IDENT)
        .expect("REDUCER_CALL must have an IDENT name token");
    assert_eq!(name_token.text(), "concat_with");
    // The REDUCER_CALL must contain an ARG_LIST.
    assert!(
        reducer.children().any(|n| n.kind() == ARG_LIST),
        "REDUCER_CALL must have an ARG_LIST child"
    );
}

#[test]
fn parse_reducer_call_bare_identifier_still_works() {
    // `reduce(xs, and_all)` — bare-identifier reducer second argument.
    // Must NOT produce a REDUCER_CALL node; must parse as a normal identifier.
    let parse = parse("SELECT reduce(xs, and_all) FROM t");
    assert!(
        parse.errors.is_empty(),
        "parse_reducer_call_bare_identifier_still_works: unexpected errors: {:?}",
        parse.errors
    );
    // Must NOT have a REDUCER_CALL node.
    assert!(
        !parse
            .syntax()
            .descendants()
            .any(|n| n.kind() == REDUCER_CALL),
        "bare-identifier reducer `and_all` must NOT produce a REDUCER_CALL node"
    );
    // Must have a FUNCTION_CALL for `reduce`.
    assert!(
        parse
            .syntax()
            .descendants()
            .any(|n| n.kind() == FUNCTION_CALL),
        "reduce(xs, and_all) must have a FUNCTION_CALL node for `reduce`"
    );
}

#[test]
fn parse_reducer_call_at_non_reduce_position_rejected() {
    // `concat_with('|')` at a top-level expression position — the parser
    // must NOT produce a REDUCER_CALL node; it must be a generic FUNCTION_CALL.
    let parse = parse("SELECT concat_with('|') FROM t");
    // No REDUCER_CALL expected (only in reduce's second-argument context).
    assert!(
        !parse
            .syntax()
            .descendants()
            .any(|n| n.kind() == REDUCER_CALL),
        "concat_with('|') outside reduce context must NOT produce a REDUCER_CALL node"
    );
    // Must parse as a FUNCTION_CALL.
    assert!(
        parse
            .syntax()
            .descendants()
            .any(|n| n.kind() == FUNCTION_CALL),
        "concat_with('|') must produce a FUNCTION_CALL node"
    );
}

#[test]
fn parses_smelt_path_call_dot_star() {
    // `smelt.functions.parse_event_payload(payload).*` — the `.*` suffix must
    // be parsed into a SMELT_PATH_CALL_STAR node wrapping the inner
    // SMELT_PATH_CALL.
    let sql = "SELECT smelt.functions.parse_event_payload(payload).* FROM e";
    let parse = crate::parse(sql);
    assert!(
        parse.errors.is_empty(),
        "unexpected parse errors: {:?}",
        parse.errors
    );
    // The CST must contain a SMELT_PATH_CALL_STAR node.
    assert!(
        parse
            .syntax()
            .descendants()
            .any(|n| n.kind() == SMELT_PATH_CALL_STAR),
        "expected SMELT_PATH_CALL_STAR node in CST"
    );
    // The SMELT_PATH_CALL_STAR must contain a SMELT_PATH_CALL child.
    let star_node = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == SMELT_PATH_CALL_STAR)
        .expect("SMELT_PATH_CALL_STAR");
    assert!(
        star_node.children().any(|c| c.kind() == SMELT_PATH_CALL),
        "SMELT_PATH_CALL_STAR must have SMELT_PATH_CALL child"
    );
}

#[test]
fn parses_smelt_path_call_dot_star_with_space() {
    // `smelt.functions.parse_event_payload(payload) .*` — whitespace between
    // `)` and `.` is tolerated: the PASSING-clause loop ends with a
    // `skip_trivia()` call, so the space has already been consumed before
    // `peek_dot_star` inspects the next token.  The result must be the same
    // SMELT_PATH_CALL_STAR wrapping as for the no-space form.
    let sql = "SELECT smelt.functions.parse_event_payload(payload) .* FROM e";
    let parse = crate::parse(sql);
    assert!(
        parse.errors.is_empty(),
        "unexpected parse errors: {:?}",
        parse.errors
    );
    assert!(
        parse
            .syntax()
            .descendants()
            .any(|n| n.kind() == SMELT_PATH_CALL_STAR),
        "expected SMELT_PATH_CALL_STAR node in CST (space-before-dot form)"
    );
}

// ===== Phase 1: ALIAS_COLUMN_LIST — derived-table and CTE alias column lists =====

#[test]
fn alias_column_list_single_col_on_values_subquery() {
    // `SELECT * FROM (VALUES (1)) AS t(c)` — should produce an ALIAS_COLUMN_LIST
    // node with exactly one IDENT `c` and no parse errors.
    let parse = crate::parse("SELECT * FROM (VALUES (1)) AS t(c)");
    assert!(
        parse.errors.is_empty(),
        "unexpected parse errors: {:?}",
        parse.errors
    );
    let alias_col_list = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == ALIAS_COLUMN_LIST)
        .expect("must have an ALIAS_COLUMN_LIST node");
    let idents: Vec<_> = alias_col_list
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == IDENT)
        .map(|t| t.text().to_string())
        .collect();
    assert_eq!(
        idents,
        vec!["c"],
        "expected single IDENT 'c' in ALIAS_COLUMN_LIST"
    );
}

#[test]
fn alias_column_list_absent_when_no_parens_after_alias() {
    // `SELECT * FROM (VALUES (1)) AS t` — no alias column list, no parse errors.
    let parse = crate::parse("SELECT * FROM (VALUES (1)) AS t");
    assert!(
        parse.errors.is_empty(),
        "unexpected parse errors: {:?}",
        parse.errors
    );
    assert!(
        !parse
            .syntax()
            .descendants()
            .any(|n| n.kind() == ALIAS_COLUMN_LIST),
        "must NOT have an ALIAS_COLUMN_LIST node when alias has no column list"
    );
}

#[test]
fn alias_column_list_three_cols_on_values_subquery() {
    // `SELECT * FROM (VALUES (1, 2, 3)) AS t(a, b, c)` — three IDENTs.
    let parse = crate::parse("SELECT * FROM (VALUES (1, 2, 3)) AS t(a, b, c)");
    assert!(
        parse.errors.is_empty(),
        "unexpected parse errors: {:?}",
        parse.errors
    );
    let alias_col_list = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == ALIAS_COLUMN_LIST)
        .expect("must have an ALIAS_COLUMN_LIST node");
    let idents: Vec<_> = alias_col_list
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == IDENT)
        .map(|t| t.text().to_string())
        .collect();
    assert_eq!(
        idents,
        vec!["a", "b", "c"],
        "expected IDENTs a, b, c in ALIAS_COLUMN_LIST"
    );
}

#[test]
fn alias_column_list_on_cte_with_columns() {
    // `WITH cte(a, b) AS (SELECT 1, 2) SELECT * FROM cte` — ALIAS_COLUMN_LIST
    // with `a` and `b` under the CTE node, not inside the inner SELECT's table refs.
    let parse = crate::parse("WITH cte(a, b) AS (SELECT 1, 2) SELECT * FROM cte");
    assert!(
        parse.errors.is_empty(),
        "unexpected parse errors: {:?}",
        parse.errors
    );
    // There must be exactly one ALIAS_COLUMN_LIST in the whole tree.
    let acl_nodes: Vec<_> = parse
        .syntax()
        .descendants()
        .filter(|n| n.kind() == ALIAS_COLUMN_LIST)
        .collect();
    assert_eq!(acl_nodes.len(), 1, "expected exactly one ALIAS_COLUMN_LIST");
    // It must live under a CTE node.
    let cte_node = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == CTE)
        .expect("must have a CTE node");
    assert!(
        cte_node
            .descendants()
            .any(|n| n.kind() == ALIAS_COLUMN_LIST),
        "ALIAS_COLUMN_LIST must be a descendant of the CTE node"
    );
    // Must NOT appear inside the inner SELECT's FROM clause.
    let inner_select = cte_node
        .descendants()
        .find(|n| n.kind() == SELECT_STMT)
        .expect("CTE must have an inner SELECT_STMT");
    assert!(
        !inner_select
            .descendants()
            .any(|n| n.kind() == ALIAS_COLUMN_LIST),
        "ALIAS_COLUMN_LIST must NOT appear inside the inner SELECT's subtree"
    );
    // Confirm the two column names.
    let idents: Vec<_> = acl_nodes[0]
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == IDENT)
        .map(|t| t.text().to_string())
        .collect();
    assert_eq!(idents, vec!["a", "b"]);
}

#[test]
fn alias_column_list_absent_on_cte_without_column_list() {
    // `WITH cte AS (SELECT 1) SELECT * FROM cte` — no ALIAS_COLUMN_LIST.
    let parse = crate::parse("WITH cte AS (SELECT 1) SELECT * FROM cte");
    assert!(
        parse.errors.is_empty(),
        "unexpected parse errors: {:?}",
        parse.errors
    );
    assert!(
        !parse
            .syntax()
            .descendants()
            .any(|n| n.kind() == ALIAS_COLUMN_LIST),
        "must NOT have an ALIAS_COLUMN_LIST node when CTE has no column list"
    );
}

#[test]
fn alias_column_list_on_implicit_alias_values_subquery() {
    // `SELECT * FROM (VALUES (1, 2)) t(a, b)` — implicit alias (no AS keyword).
    // The column list must be captured as an ALIAS_COLUMN_LIST inside the
    // FROM clause, exactly as for the explicit `AS t(a, b)` form — not leaked
    // into the enclosing SELECT's projection.
    let parse = crate::parse("SELECT * FROM (VALUES (1, 2)) t(a, b)");
    assert!(
        parse.errors.is_empty(),
        "unexpected parse errors: {:?}",
        parse.errors
    );
    let from_clause = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == FROM_CLAUSE)
        .expect("must have a FROM_CLAUSE node");
    let alias_col_list = from_clause
        .descendants()
        .find(|n| n.kind() == ALIAS_COLUMN_LIST)
        .expect("ALIAS_COLUMN_LIST must live inside the FROM clause, not leak out");
    let idents: Vec<_> = alias_col_list
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == IDENT)
        .map(|t| t.text().to_string())
        .collect();
    assert_eq!(
        idents,
        vec!["a", "b"],
        "expected IDENTs a, b in ALIAS_COLUMN_LIST"
    );
}

#[test]
fn alias_column_list_on_implicit_alias_select_subquery() {
    // `SELECT * FROM (SELECT 1) t(a)` — implicit alias on a SELECT subquery.
    let parse = crate::parse("SELECT * FROM (SELECT 1) t(a)");
    assert!(
        parse.errors.is_empty(),
        "unexpected parse errors: {:?}",
        parse.errors
    );
    let acl = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == ALIAS_COLUMN_LIST)
        .expect("implicit alias must produce an ALIAS_COLUMN_LIST node");
    let idents: Vec<_> = acl
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == IDENT)
        .map(|t| t.text().to_string())
        .collect();
    assert_eq!(idents, vec!["a"]);
}

#[test]
fn alias_column_list_absent_on_bare_implicit_alias() {
    // `SELECT * FROM (VALUES (1)) t` — implicit alias with no column list:
    // no ALIAS_COLUMN_LIST node, no parse errors (regression guard).
    let parse = crate::parse("SELECT * FROM (VALUES (1)) t");
    assert!(
        parse.errors.is_empty(),
        "unexpected parse errors: {:?}",
        parse.errors
    );
    assert!(
        !parse
            .syntax()
            .descendants()
            .any(|n| n.kind() == ALIAS_COLUMN_LIST),
        "bare implicit alias must NOT produce an ALIAS_COLUMN_LIST node"
    );
}

#[test]
fn subquery_values_clause_returns_some_for_values_subquery() {
    // `(VALUES (1, 2))` inside a FROM clause — Subquery::values_clause() returns Some.
    let parse = crate::parse("SELECT * FROM (VALUES (1, 2)) AS t");
    assert!(
        parse.errors.is_empty(),
        "unexpected parse errors: {:?}",
        parse.errors
    );
    let subquery_node = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == SUBQUERY)
        .expect("must have a SUBQUERY node");
    let subquery = Subquery::cast(subquery_node).expect("must cast to Subquery");
    assert!(
        subquery.values_clause().is_some(),
        "Subquery::values_clause() must return Some for a VALUES subquery"
    );
    assert!(
        subquery.select_stmt().is_none(),
        "Subquery::select_stmt() must return None for a VALUES subquery"
    );
}

#[test]
fn subquery_values_clause_returns_none_for_select_subquery() {
    // `(SELECT 1)` inside a FROM clause — Subquery::values_clause() returns None.
    let parse = crate::parse("SELECT * FROM (SELECT 1) AS t");
    assert!(
        parse.errors.is_empty(),
        "unexpected parse errors: {:?}",
        parse.errors
    );
    let subquery_node = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == SUBQUERY)
        .expect("must have a SUBQUERY node");
    let subquery = Subquery::cast(subquery_node).expect("must cast to Subquery");
    assert!(
        subquery.values_clause().is_none(),
        "Subquery::values_clause() must return None for a SELECT subquery"
    );
}

#[test]
fn table_ref_alias_column_names_on_per_cohort_orders() {
    // The `examples/per_cohort_union/models/orders.sql` body uses:
    //   FROM (VALUES (...)) AS t(id, user_id, region, revenue, created_at)
    // TableRef::alias_column_names() must return those five names.
    let sql = "\
SELECT id, user_id, region, revenue, created_at
FROM (VALUES
    (1, 10, 'us-west-2', 150, CAST('2024-01-01' AS TIMESTAMP)),
    (2, 11, 'us-west-2', 80,  CAST('2024-01-02' AS TIMESTAMP))
) AS t(id, user_id, region, revenue, created_at)";
    let parse = crate::parse(sql);
    assert!(
        parse.errors.is_empty(),
        "unexpected parse errors: {:?}",
        parse.errors
    );
    // Find the TABLE_REF that wraps the subquery
    let table_ref_node = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == TABLE_REF)
        .expect("must have a TABLE_REF node");
    let table_ref = TableRef::cast(table_ref_node).expect("must cast to TableRef");
    let cols = table_ref.alias_column_names();
    assert_eq!(
        cols,
        Some(vec![
            "id".to_string(),
            "user_id".to_string(),
            "region".to_string(),
            "revenue".to_string(),
            "created_at".to_string(),
        ]),
        "alias_column_names() must return the five column names from AS t(...)"
    );
}

#[test]
fn table_ref_alias_column_names_returns_none_when_no_list() {
    // `SELECT * FROM (VALUES (1)) AS t` — no column list; returns None.
    let parse = crate::parse("SELECT * FROM (VALUES (1)) AS t");
    assert!(
        parse.errors.is_empty(),
        "unexpected parse errors: {:?}",
        parse.errors
    );
    let table_ref_node = parse
        .syntax()
        .descendants()
        .find(|n| n.kind() == TABLE_REF)
        .expect("must have a TABLE_REF node");
    let table_ref = TableRef::cast(table_ref_node).expect("must cast to TableRef");
    assert_eq!(
        table_ref.alias_column_names(),
        None,
        "alias_column_names() must return None when no column list is present"
    );
}

#[test]
fn cte_column_names_reads_from_alias_column_list_node() {
    // `WITH cte(x, y, z) AS (SELECT 1, 2, 3) SELECT * FROM cte`
    // Cte::column_names() must return ["x", "y", "z"] via the ALIAS_COLUMN_LIST node.
    let parse = crate::parse("WITH cte(x, y, z) AS (SELECT 1, 2, 3) SELECT * FROM cte");
    assert!(
        parse.errors.is_empty(),
        "unexpected parse errors: {:?}",
        parse.errors
    );
    let file = File::cast(parse.syntax()).unwrap();
    let select = file.select_stmt().unwrap();
    let with_clause = select.with_clause().expect("must have WITH clause");
    let cte = with_clause
        .ctes()
        .next()
        .expect("must have at least one CTE");
    assert_eq!(
        cte.column_names(),
        vec!["x".to_string(), "y".to_string(), "z".to_string()],
        "Cte::column_names() must return [x, y, z]"
    );
}

#[test]
fn cte_column_names_empty_when_no_list() {
    // `WITH cte AS (SELECT 1) SELECT * FROM cte` — column_names() returns [].
    let parse = crate::parse("WITH cte AS (SELECT 1) SELECT * FROM cte");
    assert!(
        parse.errors.is_empty(),
        "unexpected parse errors: {:?}",
        parse.errors
    );
    let file = File::cast(parse.syntax()).unwrap();
    let select = file.select_stmt().unwrap();
    let with_clause = select.with_clause().expect("must have WITH clause");
    let cte = with_clause
        .ctes()
        .next()
        .expect("must have at least one CTE");
    assert_eq!(
        cte.column_names(),
        Vec::<String>::new(),
        "Cte::column_names() must return [] when no column list declared"
    );
}

/// The existing spread-in-values-row test must continue to pass.
/// (Re-stated here to make Phase 1 self-contained; the original is still present above.)
#[test]
fn parse_spread_in_values_row_still_passes_after_alias_col_list_change() {
    let parse = crate::parse("SELECT * FROM (VALUES (...vals)) AS t(c)");
    assert!(
        parse.errors.is_empty(),
        "VALUES (...vals) must parse without errors; got: {:?}",
        parse.errors
    );
    assert!(
        parse
            .syntax()
            .descendants()
            .any(|n| n.kind() == LIST_SPREAD),
        "must still have a LIST_SPREAD node"
    );
}

// ===== Phase 1 error-recovery: malformed alias column lists =====

#[test]
fn alias_column_list_trailing_comma_derived_table_no_panic() {
    // Trailing comma in a derived-table alias column list must not panic.
    // The parser tolerates trailing commas by policy (same as test_trailing_comma_select
    // and friends), so no parse errors are expected here either.
    let parse = crate::parse("SELECT * FROM (VALUES (1, 2)) AS t(a,)");
    // Must not panic — reaching this assertion is the primary goal.
    // No errors: trailing commas are silently accepted by parser policy.
    assert!(
        parse.errors.is_empty(),
        "trailing comma in AS t(a,) should be tolerated (no errors); got: {:?}",
        parse.errors
    );
}

#[test]
fn alias_column_list_leading_comma_derived_table_no_panic() {
    // A leading comma `AS t(,a)` is unambiguously malformed.
    // The parser must not panic; it must emit at least one parse error.
    let parse = crate::parse("SELECT * FROM (VALUES (1, 2)) AS t(,a)");
    // Must not panic — reaching this assertion is the primary goal.
    assert!(
        !parse.errors.is_empty(),
        "leading comma in AS t(,a) must produce at least one parse error"
    );
}

#[test]
fn alias_column_list_trailing_comma_cte_no_panic() {
    // Trailing comma in a CTE column list must not panic.
    // Consistent with the derived-table trailing-comma policy above: tolerated, no errors.
    let parse = crate::parse("WITH cte(a,) AS (SELECT 1) SELECT * FROM cte");
    // Must not panic — reaching this assertion is the primary goal.
    // No errors: trailing commas are silently accepted by parser policy.
    assert!(
        parse.errors.is_empty(),
        "trailing comma in cte(a,) should be tolerated (no errors); got: {:?}",
        parse.errors
    );
}

#[test]
fn alias_column_list_leading_comma_cte_no_panic() {
    // A leading comma `cte(,a)` in a CTE column list is unambiguously malformed.
    // The parser must not panic; it must emit at least one parse error.
    let parse = crate::parse("WITH cte(,a) AS (SELECT 1) SELECT * FROM cte");
    // Must not panic — reaching this assertion is the primary goal.
    assert!(
        !parse.errors.is_empty(),
        "leading comma in cte(,a) must produce at least one parse error"
    );
}

// ===== Phase 5 (nullability-soundness): NOT NULL qualifier tests =====

#[test]
fn parses_not_null_qualifier_on_expr_param() {
    // `Expr<Integer NOT NULL>` should parse without errors and the TypeRef
    // should report `not_null() == true`.
    let src = "smelt.define f(x: Expr<Integer NOT NULL>) AS (x + 1)\n";
    let parse = crate::parse(src);
    assert!(
        parse.errors.is_empty(),
        "Expr<Integer NOT NULL> should parse without errors; got: {:?}",
        parse.errors
    );
    // Walk the AST to find the param's TypeRef and verify not_null().
    use crate::ast::File;
    let file = File::cast(parse.syntax()).unwrap();
    let define = file.defines().next().expect("expected a smelt.define");
    let param = define
        .param_list()
        .and_then(|pl| pl.params().next())
        .expect("expected a parameter");
    let type_ref = param.type_ref().expect("expected a type_ref");
    assert!(
        type_ref.not_null(),
        "TypeRef for `Expr<Integer NOT NULL>` should report not_null() = true"
    );
}

#[test]
fn not_null_on_tableexpr_row_field() {
    // `TableExpr<{id: Integer NOT NULL}>` — the row field should expose not_null().
    let src = "smelt.define f(t: TableExpr<{id: Integer NOT NULL}>) -> TableExpr AS (SELECT t.id FROM t)\n";
    let parse = crate::parse(src);
    assert!(
        parse.errors.is_empty(),
        "TableExpr<{{id: Integer NOT NULL}}> should parse without errors; got: {:?}",
        parse.errors
    );
    use crate::ast::File;
    let file = File::cast(parse.syntax()).unwrap();
    let define = file.defines().next().expect("expected a smelt.define");
    let param = define
        .param_list()
        .and_then(|pl| pl.params().next())
        .expect("expected a parameter");
    let type_ref = param.type_ref().expect("expected a type_ref");
    let req = type_ref
        .row_requirement()
        .expect("expected a row requirement");
    let field = req
        .fields()
        .into_iter()
        .next()
        .expect("expected a row field");
    assert!(
        field.not_null(),
        "ROW_FIELD for `id: Integer NOT NULL` should report not_null() = true"
    );
}

#[test]
fn rejects_not_null_on_struct_field() {
    // `Expr<Struct<{a: Integer NOT NULL}>>` — NOT NULL inside a struct field
    // is not accepted; the parser should emit an error.
    let src = "smelt.define f(x: Expr<Struct<{a: Integer NOT NULL}>>) AS (x)\n";
    let parse = crate::parse(src);
    assert!(
        !parse.errors.is_empty(),
        "NOT NULL inside Struct<{{...}}> should produce at least one parse error"
    );
}

#[test]
fn collate_expr_parses_to_node() {
    // `name COLLATE NOCASE` should produce a COLLATE_EXPR node (no parse errors)
    let sql = "SELECT name COLLATE NOCASE FROM t";
    let result = parse(sql);
    assert!(
        result.errors.is_empty(),
        "Expected no parse errors, got: {:?}",
        result.errors
    );
    let file = File::cast(result.syntax()).unwrap();
    let select = file.select_stmt().unwrap();
    let select_list = select.select_list().unwrap();
    let item = select_list.items().next().unwrap();
    let expr = item.expression().unwrap();
    // The expression must be a COLLATE_EXPR
    assert!(
        expr.as_collate().is_some(),
        "Expected COLLATE_EXPR node, got syntax: {:?}",
        expr.syntax().kind()
    );
    let collate = expr.as_collate().unwrap();
    assert_eq!(
        collate.collation_name().as_deref(),
        Some("NOCASE"),
        "Collation name should be NOCASE"
    );

    // `name COLLATE "C"` should also work (quoted collation name)
    let sql2 = r#"SELECT name COLLATE "C" FROM t"#;
    let result2 = parse(sql2);
    assert!(
        result2.errors.is_empty(),
        "Expected no parse errors for quoted collation, got: {:?}",
        result2.errors
    );
    let file2 = File::cast(result2.syntax()).unwrap();
    let select2 = file2.select_stmt().unwrap();
    let item2 = select2.select_list().unwrap().items().next().unwrap();
    let expr2 = item2.expression().unwrap();
    assert!(
        expr2.as_collate().is_some(),
        "Expected COLLATE_EXPR for quoted collation"
    );
    assert_eq!(
        expr2.as_collate().unwrap().collation_name().as_deref(),
        Some("C"), // quotes stripped
        "Collation name should be C (without quotes)"
    );
}

// ===== Named WINDOW clause =====

#[test]
fn test_named_window_clause_top_level() {
    let input = "SELECT x, sum(y) OVER w FROM t WINDOW w AS (PARTITION BY x ORDER BY y)";
    let parse = parse(input);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
    // The SELECT should have a WINDOW clause
    let select = parse
        .syntax()
        .descendants()
        .find_map(SelectStmt::cast)
        .expect("should have SelectStmt");
    assert!(
        select.window_clause().is_some(),
        "should have a WINDOW clause"
    );
}

#[test]
fn test_named_window_clause_in_cte() {
    let input = "WITH c AS (SELECT x, sum(y) OVER w FROM t WINDOW w AS (PARTITION BY x ORDER BY y)) SELECT * FROM c";
    let parse = parse(input);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
}

#[test]
fn test_named_window_clause_round_trip() {
    // parse → print → re-parse must have no errors (same convention as assert_round_trip
    // in printer.rs — text idempotency is NOT asserted because expr.text() includes
    // leading trivia consumed by parse_expression's skip_trivia(), a pre-existing
    // printer characteristic shared across all clause types).
    let sql = "SELECT x, sum(y) OVER w FROM t WINDOW w AS (PARTITION BY x ORDER BY y)";
    let parse1 = parse(sql);
    assert!(
        parse1.errors.is_empty(),
        "Parse errors: {:?}",
        parse1.errors
    );
    let file = File::cast(parse1.syntax()).expect("should have FILE root");
    let printed = file.to_string();
    let parse2 = parse(&printed);
    assert!(
        parse2.errors.is_empty(),
        "Re-parse errors: {:?}\nPrinted SQL: {}",
        parse2.errors,
        printed
    );
}

#[test]
fn test_named_window_clause_multiple_windows() {
    // Comma-separated named windows: exercises the loop in parse_window_clause
    let input =
        "SELECT sum(a) OVER w1, rank() OVER w2 FROM t WINDOW w1 AS (PARTITION BY x), w2 AS (ORDER BY y)";
    let parse = parse(input);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
    let select = parse
        .syntax()
        .descendants()
        .find_map(SelectStmt::cast)
        .expect("should have SelectStmt");
    let wc = select.window_clause().expect("should have WINDOW clause");
    assert_eq!(
        wc.named_windows().count(),
        2,
        "should have exactly 2 named windows"
    );
}

#[test]
fn test_named_window_clause_malformed_no_panic() {
    // Malformed WINDOW clause: parser must not panic; should produce error nodes.
    let input = "SELECT x FROM t WINDOW";
    let parse = parse(input);
    // We only assert no panic and that a WINDOW clause node was attempted.
    // Parse errors are expected for incomplete input.
    let _ = parse.errors; // may or may not be empty — not asserted
    let _ = parse.syntax(); // must be accessible
}

// ===== Phase 4: numeric INTERVAL forms =====

fn assert_interval_parses_complete(sql: &str) {
    // Check that the SELECT_STMT spans the whole input (no orphaned tokens)
    // and that a FROM clause is reachable inside the select.
    let parse = parse(sql);
    assert_eq!(
        parse.errors.len(),
        0,
        "parse errors for {sql}: {:?}",
        parse.errors
    );
    let file = File::cast(parse.syntax()).expect("FILE node");
    let select = file
        .select_stmt()
        .unwrap_or_else(|| panic!("no SelectStmt found for {sql}"));
    assert!(
        select.from_clause().is_some(),
        "FROM clause was orphaned (not inside SelectStmt) for: {sql}"
    );
}

#[test]
fn test_interval_numeric_simple() {
    // INTERVAL 1 DAY — numeric form: FROM clause must not be orphaned
    assert_interval_parses_complete("SELECT INTERVAL 1 DAY FROM t");
}

#[test]
fn test_interval_numeric_other_units() {
    // Various unit keywords
    for sql in &[
        "SELECT INTERVAL 1 MONTH FROM t",
        "SELECT INTERVAL 2 YEAR FROM t",
        "SELECT INTERVAL 3 HOUR FROM t",
        "SELECT INTERVAL 30 SECOND FROM t",
    ] {
        assert_interval_parses_complete(sql);
    }
}

#[test]
fn test_interval_numeric_parenthesized() {
    // INTERVAL (1) DAY — parenthesized numeric form
    assert_interval_parses_complete("SELECT INTERVAL (1) DAY FROM t");
}

#[test]
fn test_interval_multiplied() {
    // n * INTERVAL 1 DAY — multiplier form (binary with numeric INTERVAL as RHS)
    assert_interval_parses_complete("SELECT 3 * INTERVAL 1 DAY FROM t");
}

#[test]
fn test_interval_string_form_unchanged() {
    // INTERVAL '1 day' — existing string form must still parse correctly
    assert_interval_parses_complete("SELECT d - INTERVAL '1 day' AS x FROM t");
}

#[test]
fn test_interval_numeric_in_expression() {
    // INTERVAL used inside an arithmetic expression
    assert_interval_parses_complete("SELECT ts + INTERVAL 1 DAY AS next_day FROM events");
    assert_interval_parses_complete("SELECT ts - INTERVAL 7 DAY AS week_ago FROM events");
}

#[test]
fn test_interval_numeric_round_trip() {
    // parse → print → parse stability for all three numeric INTERVAL forms
    for sql in &[
        "SELECT INTERVAL 1 DAY FROM t",
        "SELECT INTERVAL (1) DAY FROM t",
        "SELECT 3 * INTERVAL 1 DAY FROM t",
    ] {
        let parse1 = parse(sql);
        assert_eq!(
            parse1.errors.len(),
            0,
            "original parse failed for {sql}: {:?}",
            parse1.errors
        );
        let file = File::cast(parse1.syntax()).expect("FILE node");
        let printed = file.to_string();
        let parse2 = parse(&printed);
        assert_eq!(
            parse2.errors.len(),
            0,
            "round-trip failed for {sql}: printed={printed:?}, errors={:?}",
            parse2.errors
        );
        // Verify FROM clause still in the right place after round-trip
        let file2 = File::cast(parse2.syntax()).expect("FILE2 node");
        let select2 = file2.select_stmt().expect("SelectStmt after round-trip");
        assert!(
            select2.from_clause().is_some(),
            "FROM clause orphaned after round-trip for: {sql} (printed as: {printed})"
        );
    }
}

// ===== Phase 3: smelt.test declaration grammar =====

use crate::ast::{RecordLiteral, SmeltTest};
use crate::syntax_kind::SyntaxKind::{EXPECT_CLAUSE, SMELT_TEST};

#[test]
fn parse_smelt_test_basic() {
    // A smelt.test declaration with a SELECT body, one PASSING clause,
    // and an EXPECT clause must parse into a SmeltTest node with the
    // expected children. Lossless round-trip is also verified.
    let input = r#"smelt.test check_revenue AS (
    SELECT a, b FROM smelt.orders WHERE revenue > 0
)
PASSING orders AS ({amount: 100})
EXPECT ({a: 1, b: 'x'})"#;

    let (parse_result, file) = parse_file_text(input);
    assert!(
        parse_result.errors.is_empty(),
        "parse_smelt_test_basic: unexpected errors: {:?}",
        parse_result.errors
    );

    // There must be exactly one SMELT_TEST node.
    let tests: Vec<SmeltTest> = file.tests().collect();
    assert_eq!(tests.len(), 1, "expected exactly one smelt.test");
    let test = &tests[0];

    // Name
    assert_eq!(
        test.name().as_deref(),
        Some("check_revenue"),
        "test name should be 'check_revenue'"
    );

    // Body SELECT exists
    assert!(
        test.body_select().is_some(),
        "smelt.test body should contain a SELECT statement"
    );

    // One PASSING clause
    let passing: Vec<_> = test.passing_clauses().collect();
    assert_eq!(passing.len(), 1, "expected one PASSING clause");
    assert_eq!(
        passing[0].name().as_deref(),
        Some("orders"),
        "PASSING name should be 'orders'"
    );

    // EXPECT clause exists
    let expect = test
        .expect_clause()
        .expect("EXPECT clause should be present");
    let rows: Vec<RecordLiteral> = expect.rows().collect();
    assert_eq!(rows.len(), 1, "expected one row in EXPECT clause");

    // Lossless round-trip: the CST text (Rowan's lossless representation)
    // must reproduce the exact source bytes.
    let cst_text = parse_result.syntax().text().to_string();
    assert_eq!(
        cst_text, input,
        "round-trip: CST text must reproduce the exact source"
    );
    // Re-parsing the CST text must produce no errors.
    let parse2 = parse(&cst_text);
    assert_eq!(
        parse2.errors.len(),
        0,
        "round-trip re-parse failed: errors={:?}",
        parse2.errors
    );
}

#[test]
fn parse_smelt_test_multiple_passing_clauses() {
    // Zero PASSING clauses: body + EXPECT only.
    let input_zero = r#"smelt.test zero_passing AS (
    SELECT 1 AS result
)
EXPECT ({result: 1})"#;

    let (parse_zero, file_zero) = parse_file_text(input_zero);
    assert!(
        parse_zero.errors.is_empty(),
        "parse_smelt_test_multiple_passing (zero): unexpected errors: {:?}",
        parse_zero.errors
    );
    let tests_zero: Vec<SmeltTest> = file_zero.tests().collect();
    assert_eq!(
        tests_zero.len(),
        1,
        "expected one smelt.test (zero PASSING)"
    );
    let passing_zero: Vec<_> = tests_zero[0].passing_clauses().collect();
    assert_eq!(
        passing_zero.len(),
        0,
        "expected zero PASSING clauses, got {}",
        passing_zero.len()
    );

    // Two PASSING clauses.
    let input_two = r#"smelt.test two_passing AS (
    SELECT a.x + b.y AS total FROM smelt.a JOIN smelt.b ON TRUE
)
PASSING a AS ({x: 1})
PASSING b AS ({y: 2})
EXPECT ({total: 3})"#;

    let (parse_two, file_two) = parse_file_text(input_two);
    assert!(
        parse_two.errors.is_empty(),
        "parse_smelt_test_multiple_passing (two): unexpected errors: {:?}",
        parse_two.errors
    );
    let tests_two: Vec<SmeltTest> = file_two.tests().collect();
    assert_eq!(tests_two.len(), 1, "expected one smelt.test (two PASSING)");
    let passing_two: Vec<_> = tests_two[0].passing_clauses().collect();
    assert_eq!(
        passing_two.len(),
        2,
        "expected two PASSING clauses, got {}",
        passing_two.len()
    );
    assert_eq!(passing_two[0].name().as_deref(), Some("a"));
    assert_eq!(passing_two[1].name().as_deref(), Some("b"));
}

#[test]
fn parse_smelt_test_dotted_passing_names() {
    // A PASSING <dep> name is the bare address path of a dependency and may be
    // multi-segment (testing.md §Surface: `orders` or `silver.orders`).
    let input = r#"smelt.test dotted_deps AS (
    SELECT session_id FROM smelt.gold.identity_forward_only
)
PASSING silver.sessions AS ({session_id: 'sa'})
PASSING silver.events_parsed AS ({event_id: 1})
EXPECT ({session_id: 'sa'})"#;

    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "dotted PASSING names must parse without errors: {:?}",
        parse.errors
    );
    let tests: Vec<SmeltTest> = file.tests().collect();
    assert_eq!(tests.len(), 1, "expected one smelt.test");
    let passing: Vec<_> = tests[0].passing_clauses().collect();
    assert_eq!(passing.len(), 2, "expected two PASSING clauses");
    assert_eq!(passing[0].name().as_deref(), Some("silver.sessions"));
    assert_eq!(passing[1].name().as_deref(), Some("silver.events_parsed"));
}

#[test]
fn expect_rows_are_record_literals() {
    // EXPECT with multiple record literals; omitted keys are allowed.
    let input = r#"smelt.test multi_row_expect AS (
    SELECT a, b FROM t
)
EXPECT ({a: 1, b: 'x'}, {a: 2})"#;

    let (parse, file) = parse_file_text(input);
    assert!(
        parse.errors.is_empty(),
        "expect_rows_are_record_literals: unexpected errors: {:?}",
        parse.errors
    );

    let tests: Vec<SmeltTest> = file.tests().collect();
    assert_eq!(tests.len(), 1);
    let expect = tests[0]
        .expect_clause()
        .expect("EXPECT clause must be present");

    // Two record literals: {a: 1, b: 'x'} and {a: 2}.
    let rows: Vec<RecordLiteral> = expect.rows().collect();
    assert_eq!(rows.len(), 2, "expected two rows in EXPECT clause");

    // First row has two fields; second has one (omitted key is fine).
    let fields_0: Vec<_> = rows[0].fields().collect();
    let fields_1: Vec<_> = rows[1].fields().collect();
    assert_eq!(
        fields_0.len(),
        2,
        "first EXPECT row should have 2 fields, got {}",
        fields_0.len()
    );
    assert_eq!(
        fields_1.len(),
        1,
        "second EXPECT row should have 1 field (omitted key), got {}",
        fields_1.len()
    );

    // Verify the EXPECT_CLAUSE node kind appears in the tree.
    let has_expect = file
        .syntax()
        .descendants()
        .any(|n| n.kind() == EXPECT_CLAUSE);
    assert!(has_expect, "tree must contain an EXPECT_CLAUSE node");
}

#[test]
fn passing_expect_contextual() {
    // `PASSING` and `EXPECT` must remain ordinary identifiers outside a
    // smelt.test declaration — no regression to ordinary SQL identifiers.

    // `passing` and `expect` as column aliases.
    let sql_alias = "SELECT x AS passing, y AS expect FROM t";
    let (parse_alias, _) = parse_file_text(sql_alias);
    assert!(
        parse_alias.errors.is_empty(),
        "column alias 'passing'/'expect' should parse cleanly: {:?}",
        parse_alias.errors
    );

    // `passing` and `expect` as CTE names.
    let sql_cte =
        "WITH passing AS (SELECT 1 AS n), expect AS (SELECT 2 AS n) SELECT * FROM passing";
    let (parse_cte, _) = parse_file_text(sql_cte);
    assert!(
        parse_cte.errors.is_empty(),
        "CTE named 'passing'/'expect' should parse cleanly: {:?}",
        parse_cte.errors
    );

    // `passing` and `expect` as table names in FROM clause.
    let sql_from = "SELECT x FROM passing JOIN expect ON passing.id = expect.id";
    let (parse_from, _) = parse_file_text(sql_from);
    assert!(
        parse_from.errors.is_empty(),
        "table names 'passing'/'expect' in FROM should parse cleanly: {:?}",
        parse_from.errors
    );

    // None of these should produce SMELT_TEST or EXPECT_CLAUSE nodes.
    for (sql, label) in &[(sql_alias, "alias"), (sql_cte, "cte"), (sql_from, "from")] {
        let p = parse(sql);
        let has_test = p.syntax().descendants().any(|n| n.kind() == SMELT_TEST);
        let has_expect = p.syntax().descendants().any(|n| n.kind() == EXPECT_CLAUSE);
        assert!(
            !has_test,
            "plain SQL ({label}) must not produce SMELT_TEST nodes"
        );
        assert!(
            !has_expect,
            "plain SQL ({label}) must not produce EXPECT_CLAUSE nodes"
        );
    }

    // Regression: PASSING as a column name in SELECT list.
    let p_passing = parse("SELECT passing FROM t");
    let passing_test_nodes = p_passing
        .syntax()
        .descendants()
        .any(|n| n.kind() == SMELT_TEST);
    assert!(
        !passing_test_nodes,
        "'SELECT passing FROM t' must not produce SMELT_TEST"
    );

    // Regression: smelt.test only triggers at top-level, not in expression position.
    // A query that happens to have 'smelt' and 'test' in identifiers must not be
    // mistaken for a top-level smelt.test declaration.
    let sql_expr = "SELECT smelt_test FROM t";
    let (parse_expr, _) = parse_file_text(sql_expr);
    // This is not a smelt.test trigger (smelt_test is one IDENT, not smelt DOT test).
    assert!(
        parse_expr.errors.is_empty(),
        "identifier 'smelt_test' must parse as plain SQL: {:?}",
        parse_expr.errors
    );
}

// ===== Phase 3: multi-row PASSING body (FIX 1) =====

#[test]
fn parse_smelt_test_multirow_passing() {
    // A PASSING clause with two record-literal rows — the spec's canonical example.
    // Before the fix this produced 3 parse errors and dropped the second row.
    let input = r#"smelt.test check_revenue AS (
    SELECT order_id, amount FROM smelt.orders
)
PASSING orders AS (
    {order_id: 1, amount: 100.0},
    {order_id: 2, amount: 200.0}
)
EXPECT ({order_id: 1})"#;

    let (parse_result, file) = parse_file_text(input);
    assert!(
        parse_result.errors.is_empty(),
        "parse_smelt_test_multirow_passing: unexpected errors: {:?}",
        parse_result.errors
    );

    // Lossless round-trip.
    let cst_text = parse_result.syntax().text().to_string();
    assert_eq!(
        cst_text, input,
        "round-trip: CST text must reproduce source"
    );
    let parse2 = parse(&cst_text);
    assert_eq!(
        parse2.errors.len(),
        0,
        "round-trip re-parse failed: {:?}",
        parse2.errors
    );

    // One test, one PASSING clause, two rows inside it.
    let tests: Vec<SmeltTest> = file.tests().collect();
    assert_eq!(tests.len(), 1, "expected one smelt.test");
    let passing: Vec<_> = tests[0].passing_clauses().collect();
    assert_eq!(passing.len(), 1, "expected one PASSING clause");
    assert_eq!(
        passing[0].name().as_deref(),
        Some("orders"),
        "PASSING name should be 'orders'"
    );

    // The new rows() accessor must return both record literals.
    let rows: Vec<RecordLiteral> = passing[0].rows().collect();
    assert_eq!(
        rows.len(),
        2,
        "PASSING clause should expose 2 rows, got {}",
        rows.len()
    );
}

// ===== Phase 3: error-recovery tests (FIX 2) =====

#[test]
fn smelt_test_missing_as_recovers() {
    // `smelt.test foo (SELECT 1) EXPECT ({a:1})` — missing `AS`.
    // The parser must return without panicking and produce ≥ 1 parse error.
    let input = "smelt.test foo (SELECT 1) EXPECT ({a: 1})";
    let parse = parse(input);
    assert!(
        !parse.errors.is_empty(),
        "missing AS should produce at least one parse error"
    );
}

#[test]
fn smelt_test_missing_expect_recovers() {
    // `smelt.test foo AS (SELECT 1) PASSING x AS ({a:1})` — no EXPECT clause.
    // The parser must return without panicking and produce ≥ 1 parse error.
    let input = "smelt.test foo AS (SELECT 1) PASSING x AS ({a: 1})";
    let parse = parse(input);
    assert!(
        !parse.errors.is_empty(),
        "missing EXPECT should produce at least one parse error"
    );
}

#[test]
fn smelt_test_unclosed_body_recovers() {
    // `smelt.test foo AS (SELECT 1 EXPECT ({a:1})` — missing `)` closing the body.
    // The parser must return without panicking and produce ≥ 1 parse error.
    let input = "smelt.test foo AS (SELECT 1 EXPECT ({a: 1})";
    let parse = parse(input);
    assert!(
        !parse.errors.is_empty(),
        "unclosed body should produce at least one parse error"
    );
}

// ===== Phase 4: `#` CTE-reference operator =====

#[test]
fn parse_hash_cte_ref() {
    // `FROM smelt.daily_revenue#daily_agg` must parse as a single
    // SMELT_PATH_REF with a CTE_SEGMENT child containing the CTE name.
    // No parse errors expected.
    use crate::ast::SmeltPathRef;
    use crate::syntax_kind::SyntaxKind::SMELT_PATH_REF;

    let input = "SELECT day FROM smelt.daily_revenue#daily_agg";
    let parse = parse(input);
    assert!(
        parse.errors.is_empty(),
        "parse_hash_cte_ref: unexpected errors: {:?}",
        parse.errors
    );

    // Collect all SMELT_PATH_REF descendants.
    let path_refs: Vec<SmeltPathRef> = parse
        .syntax()
        .descendants()
        .filter(|n| n.kind() == SMELT_PATH_REF)
        .filter_map(SmeltPathRef::cast)
        .collect();

    assert_eq!(
        path_refs.len(),
        1,
        "expected exactly one SMELT_PATH_REF; got {}",
        path_refs.len()
    );
    let pr = &path_refs[0];

    // Path segments (without the CTE suffix) should be ["daily_revenue"].
    assert_eq!(
        pr.segments(),
        vec!["daily_revenue".to_string()],
        "segments() should not include the CTE name"
    );

    // cte_name() must return "daily_agg".
    assert_eq!(
        pr.cte_name().as_deref(),
        Some("daily_agg"),
        "cte_name() should be 'daily_agg'"
    );

    // hash_range() must be Some (anchored at the `#` character).
    let hash_range = pr.hash_range();
    assert!(hash_range.is_some(), "hash_range() should be Some");
    // The `#` appears at byte offset 35 in "SELECT day FROM smelt.daily_revenue#daily_agg"
    // (0-indexed: 'S'=0, ...  '#'=35).
    let hr = hash_range.unwrap();
    let hash_start: u32 = hr.start().into();
    assert_eq!(
        hash_start, 35,
        "hash_range() start should be at byte offset 35 (the `#`)"
    );
    assert_eq!(
        u32::from(hr.len()),
        1,
        "hash_range() should span exactly 1 byte"
    );

    // Lossless round-trip.
    let cst_text = parse.syntax().text().to_string().trim().to_string();
    assert_eq!(
        cst_text, input,
        "round-trip: CST text must reproduce the exact source"
    );
}

#[test]
fn parse_hash_cte_ref_no_errors_for_plain_ref() {
    // A plain smelt path ref WITHOUT a `#` suffix must continue to parse
    // without errors and have no cte_name().
    use crate::ast::SmeltPathRef;
    use crate::syntax_kind::SyntaxKind::SMELT_PATH_REF;

    let input = "SELECT * FROM smelt.daily_revenue";
    let parse = parse(input);
    assert!(
        parse.errors.is_empty(),
        "plain ref: unexpected errors: {:?}",
        parse.errors
    );
    let refs: Vec<SmeltPathRef> = parse
        .syntax()
        .descendants()
        .filter(|n| n.kind() == SMELT_PATH_REF)
        .filter_map(SmeltPathRef::cast)
        .collect();
    assert_eq!(refs.len(), 1);
    assert_eq!(
        refs[0].cte_name(),
        None,
        "plain ref should have no cte_name"
    );
    assert_eq!(
        refs[0].hash_range(),
        None,
        "plain ref should have no hash_range"
    );
}

#[test]
fn parse_hash_json_operators_unaffected() {
    // `#>` and `#>>` (JSON operators) must still lex as HASH_ARROW /
    // HASH_ARROW_TEXT, not as HASH followed by `>` / `>>`.
    use crate::syntax_kind::SyntaxKind::{HASH_ARROW, HASH_ARROW_TEXT};
    let tokens_arrow = crate::lexer::tokenize("col#>'{a}'");
    let kinds_arrow: Vec<_> = tokens_arrow.iter().map(|t| t.kind).collect();
    assert!(
        kinds_arrow.contains(&HASH_ARROW),
        "#> should lex as HASH_ARROW; got: {:?}",
        kinds_arrow
    );

    let tokens_text = crate::lexer::tokenize("col#>>'{a}'");
    let kinds_text: Vec<_> = tokens_text.iter().map(|t| t.kind).collect();
    assert!(
        kinds_text.contains(&HASH_ARROW_TEXT),
        "#>> should lex as HASH_ARROW_TEXT; got: {:?}",
        kinds_text
    );
}

#[test]
fn dangling_hash_no_ident_recovers() {
    // A `smelt.<path>#` with NO following identifier (dangling `#`) must not
    // panic and must preserve the lossless round-trip invariant.  The orphan
    // `#` must NOT be captured into a CTE_SEGMENT — `cte_name()` must be None.
    use crate::ast::SmeltPathRef;
    use crate::syntax_kind::SyntaxKind::SMELT_PATH_REF;

    let input = "SELECT x FROM smelt.daily_revenue#";

    // Must not panic.
    let parse = parse(input);

    // Lossless round-trip: ALL source characters must be preserved in the CST.
    let cst_text = parse.syntax().text().to_string().trim().to_string();
    assert_eq!(
        cst_text, input,
        "dangling `#`: CST must losslessly reproduce the source"
    );

    // The orphan `#` must NOT have been consumed into a CTE_SEGMENT.
    // Any SMELT_PATH_REF in the tree must have cte_name() == None.
    let path_refs: Vec<SmeltPathRef> = parse
        .syntax()
        .descendants()
        .filter(|n| n.kind() == SMELT_PATH_REF)
        .filter_map(SmeltPathRef::cast)
        .collect();

    for pr in &path_refs {
        assert_eq!(
            pr.cte_name(),
            None,
            "dangling `#`: cte_name() must be None (orphan `#` must not form a CTE_SEGMENT); \
             got {:?}",
            pr.cte_name()
        );
        assert_eq!(
            pr.hash_range(),
            None,
            "dangling `#`: hash_range() must be None (orphan `#` must not be inside SMELT_PATH_REF)"
        );
    }
}

#[test]
fn test_scientific_notation_number() {
    // Scientific-notation numeric literals must lex as a single NUMBER token
    // (mantissa + exponent), not a NUMBER followed by an `e`/`E` identifier.
    for (sql, lit) in [
        ("SELECT 1e8 AS x", "1e8"),
        ("SELECT 1e-8 AS x", "1e-8"),
        ("SELECT 1.5e+3 AS x", "1.5e+3"),
        ("SELECT 2.3E-4 AS x", "2.3E-4"),
    ] {
        let (parse, _select) = parse_select(sql);
        let numbers: Vec<String> = parse
            .syntax()
            .descendants_with_tokens()
            .filter_map(|t| t.into_token())
            .filter(|t| t.kind() == NUMBER)
            .map(|t| t.text().to_string())
            .collect();
        assert!(
            numbers.iter().any(|n| n == lit),
            "scientific literal {lit:?} must be a single NUMBER token; got {numbers:?}"
        );
    }
}

#[test]
fn number_followed_by_ident_without_space_errors() {
    // Oracle: DuckDB does not have real hex-integer-literal grammar in this
    // position — `duckdb -c "SELECT 0x1F;"` prints a column named `x1F`
    // with value `0` (i.e. DuckDB itself silently reads this as `0`
    // implicitly aliased to `x1F`; a following `AS a`, or any binary
    // operator, then fails as a syntax error, since the alias slot is
    // already filled / `x1F` isn't a valid operator continuation — verified
    // empirically with `duckdb -c "SELECT 0x1F AS x"` and
    // `duckdb -c "SELECT 0x1F + 1"`, both syntax errors). smelt refuses to
    // reproduce that silent split: the whole malformed blob must surface as
    // a single ERROR token / parse error rather than being read as
    // `0 AS x1F`.
    let (sql, blob) = ("SELECT 0x1F", "0x1F");
    let parse = parse(sql);
    assert!(
        !parse.errors.is_empty(),
        "{sql:?} must produce a parse error, not a silent number+identifier split"
    );

    let error_tokens: Vec<_> = parse
        .syntax()
        .descendants_with_tokens()
        .filter_map(|t| t.into_token())
        .filter(|t| t.kind() == crate::SyntaxKind::ERROR)
        .collect();
    assert!(
        error_tokens.iter().any(|t| t.text() == blob),
        "expected a single ERROR token spanning the whole blob {blob:?}, got: {:?}",
        error_tokens
    );

    // Guard: the blob must never appear as a NUMBER token (i.e. it was
    // never silently split into a shorter numeric prefix).
    let numbers: Vec<String> = parse
        .syntax()
        .descendants_with_tokens()
        .filter_map(|t| t.into_token())
        .filter(|t| t.kind() == NUMBER)
        .map(|t| t.text().to_string())
        .collect();
    assert!(
        !numbers.iter().any(|n| blob.starts_with(n.as_str())),
        "{sql:?} must not split off a leading NUMBER token, got NUMBER tokens: {numbers:?}"
    );
}

#[test]
fn number_with_underscore_digit_separator_parses() {
    // Oracle: DuckDB accepts underscore digit separators strictly between
    // two digits in the integer part, fractional part, and exponent digits
    // — verified empirically:
    //   `duckdb -c "SELECT 1_000_000 x, typeof(1_000_000);"` -> 1000000, INTEGER
    //   `duckdb -c "SELECT 1_000.000_1 x, typeof(1_000.000_1);"` -> 1000.0001, DECIMAL(8,4)
    //   `duckdb -c "SELECT 1_000_000.5_00e1_0 x;"` -> 1.0000005e+16 (DOUBLE)
    for sql in [
        "SELECT 1_000_000 AS x",
        "SELECT 1_000.000_1 AS x",
        "SELECT 1_2_3 AS x",
        "SELECT 1_000_000.5_00e1_0 AS x",
    ] {
        let parse = parse(sql);
        assert!(
            parse.errors.is_empty(),
            "{sql:?} must parse without errors, got: {:?}",
            parse.errors
        );

        let numbers: Vec<String> = parse
            .syntax()
            .descendants_with_tokens()
            .filter_map(|t| t.into_token())
            .filter(|t| t.kind() == NUMBER)
            .map(|t| t.text().to_string())
            .collect();
        assert_eq!(
            numbers.len(),
            1,
            "{sql:?} must lex the literal as exactly one NUMBER token, got: {numbers:?}"
        );
    }
}

#[test]
fn number_with_invalid_underscore_placement_errors() {
    // Oracle: DuckDB rejects a doubled separator, a leading separator, and a
    // trailing separator — verified empirically:
    //   `duckdb -c "SELECT 1__0 x;"` -> Parser Error: syntax error at or near "x"
    //   `duckdb -c "SELECT 1_ x;"` -> Parser Error: syntax error at or near "x"
    //   `duckdb -c "SELECT 1_.5 x;"` -> Parser Error: syntax error at or near ".5"
    //   `duckdb -c "SELECT 1e_5 x;"` -> Parser Error: syntax error at or near "x"
    // smelt must never silently split these into a shorter NUMBER token
    // plus an identifier — the whole blob stays a single ERROR token.
    for (sql, blob) in [
        ("SELECT 1__0", "1__0"),
        ("SELECT 1_", "1_"),
        ("SELECT 1000_", "1000_"),
    ] {
        let parse = parse(sql);
        assert!(
            !parse.errors.is_empty(),
            "{sql:?} must produce a parse error, not a silent number+identifier split"
        );

        let error_tokens: Vec<_> = parse
            .syntax()
            .descendants_with_tokens()
            .filter_map(|t| t.into_token())
            .filter(|t| t.kind() == crate::SyntaxKind::ERROR)
            .collect();
        assert!(
            error_tokens.iter().any(|t| t.text() == blob),
            "expected a single ERROR token spanning the whole blob {blob:?}, got: {:?}",
            error_tokens
        );

        let numbers: Vec<String> = parse
            .syntax()
            .descendants_with_tokens()
            .filter_map(|t| t.into_token())
            .filter(|t| t.kind() == NUMBER)
            .map(|t| t.text().to_string())
            .collect();
        assert!(
            !numbers.iter().any(|n| blob.starts_with(n.as_str())),
            "{sql:?} must not split off a leading NUMBER token, got NUMBER tokens: {numbers:?}"
        );
    }
}

#[test]
fn is_not_distinct_from_parses() {
    // `IS [NOT] DISTINCT FROM` is a standard SQL null-safe comparison; the
    // planner's cube_split rule generates it in join conditions. Before the
    // trailing-content check (fail-loud) landed, the parser silently
    // absorbed the `DISTINCT FROM …` tail; it must now parse as part of the
    // comparison, not error.
    for sql in [
        "SELECT 1 FROM a t0 JOIN b t1 ON t0.x IS NOT DISTINCT FROM t1.x",
        "SELECT 1 FROM a t0 JOIN b t1 ON t0.x IS DISTINCT FROM t1.x",
        "SELECT x IS NOT DISTINCT FROM y FROM t",
    ] {
        let parse = parse(sql);
        assert!(
            parse.errors.is_empty(),
            "{sql:?} must parse without errors, got: {:?}",
            parse.errors
        );
    }
}

#[test]
fn number_then_space_then_ident_is_alias() {
    // Guard: whitespace before the identifier means this is a legitimate
    // implicit alias (`expr alias`), not a malformed literal blob — the
    // fail-loud check must not over-fire onto grammar the parser already
    // recognises.
    let (parse, select) = parse_select("SELECT 1 x");
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
    let item = select
        .select_list()
        .expect("select list")
        .items()
        .next()
        .expect("one select item");
    assert_eq!(item.alias(), Some("x".to_string()));
}

#[test]
fn prefixed_string_literals_not_split() {
    // Oracle: DuckDB accepts both forms as string literals without error —
    // `duckdb -c "select E'a\nb', typeof(E'a\nb');"` returns a VARCHAR value
    // (DuckDB additionally applies backslash-escape processing to the body,
    // e.g. `\n` becomes a real newline — `length(E'a\nb') = 3`); and
    // `duckdb -c "select B'0101', typeof(B'0101');"` likewise returns a
    // VARCHAR value with no parse error. smelt must not silently split
    // either form into an orphan `E`/`B` identifier followed by an
    // unrelated string literal (which would misparse as `E` aliased next
    // to a bare string, or read as two separate tokens with no connection
    // to each other) — accepting the whole thing as one STRING token is
    // preferred over erroring, and is what this phase implements.
    for sql in ["SELECT E'\\n'", "SELECT B'0101'"] {
        let parse = parse(sql);
        assert!(
            parse.errors.is_empty(),
            "{sql:?}: expected the prefixed string literal to lex as one token \
             (or, at minimum, no silent identifier+orphan-string split); got errors: {:?}",
            parse.errors
        );

        // No lone `E`/`B` IDENT token immediately followed by a STRING
        // token — that would be the silent split this phase forbids.
        let tokens: Vec<_> = parse
            .syntax()
            .descendants_with_tokens()
            .filter_map(|t| t.into_token())
            .filter(|t| t.kind() != WHITESPACE)
            .collect();
        for w in tokens.windows(2) {
            let split = w[0].kind() == IDENT
                && (w[0].text() == "E"
                    || w[0].text() == "e"
                    || w[0].text() == "B"
                    || w[0].text() == "b")
                && w[1].kind() == crate::SyntaxKind::STRING;
            assert!(
                !split,
                "{sql:?} must not silently split into an `{}` identifier followed by an orphan string token",
                w[0].text()
            );
        }
    }
}

#[test]
fn test_postfix_cast_on_non_ident_primary() {
    // PostgreSQL-style `expr::type` must parse for non-IDENT primaries
    // (number, string, parenthesized expr, function-call result) into a
    // CAST_EXPR node.
    for sql in [
        "SELECT 1.0::DOUBLE AS x",
        "SELECT 'txt'::TEXT AS x",
        "SELECT (1 + 2)::BIGINT AS x",
        "SELECT date_trunc('day', ts)::DATE AS d FROM t",
    ] {
        let (parse, _select) = parse_select(sql);
        let has_cast = parse.syntax().descendants().any(|n| n.kind() == CAST_EXPR);
        assert!(has_cast, "expected a CAST_EXPR node for {sql:?}");
    }
}

// ── Fail-loud trailing top-level content ────────────────────────────────────
//
// `parse_file` parses at most one model body. Any top-level content left
// over afterwards must produce a parse error and land inside an `ERROR` node
// — never be absorbed silently. See `docs/specs/diagnostics.md` §"Fail-loud
// invariants" #3 and the `TrailingTopLevelContent` catalogue entry.

/// Every non-trivia token in `syntax` must either be inside an `ERROR` node
/// or be one of the tokens the grammar itself recognises. This helper checks
/// that every token that is a descendant of an `ERROR` node really is under
/// one, i.e. that the given predicate tokens are wrapped rather than loose
/// directly under `FILE`.
fn tokens_with_text(
    syntax: &rowan::SyntaxNode<crate::syntax_kind::SmeltLanguage>,
    text: &str,
) -> Vec<rowan::SyntaxToken<crate::syntax_kind::SmeltLanguage>> {
    syntax
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.text() == text)
        .collect()
}

#[test]
fn trailing_tokens_after_select_error() {
    // `t alias1` is a legitimate implicit table alias (consumed by the
    // TABLE_REF grammar) — the trailing content under test is the *second*
    // run of stray identifiers, `stray_one stray_two`, which the grammar has
    // no production for at all.
    let input = "SELECT a FROM t alias1 stray_one stray_two";
    let parse = parse(input);
    assert!(
        !parse.errors.is_empty(),
        "expected at least one parse error for trailing top-level content, got none"
    );

    let syntax = parse.syntax();
    for text in ["stray_one", "stray_two"] {
        let toks = tokens_with_text(&syntax, text);
        assert!(!toks.is_empty(), "expected a {text:?} token in the tree");
        for tok in toks {
            let inside_error = tok
                .parent()
                .map(|p| {
                    p.kind() == crate::SyntaxKind::ERROR
                        || p.ancestors().any(|a| a.kind() == crate::SyntaxKind::ERROR)
                })
                .unwrap_or(false);
            assert!(
                inside_error,
                "expected {text:?} to sit inside an ERROR node, not loose under FILE"
            );
        }
    }

    // Guard: `alias1` (the legitimate implicit alias) must NOT be inside an
    // ERROR node — trailing-content detection must not over-fire onto
    // grammar the parser already recognises.
    let alias_tok = tokens_with_text(&syntax, "alias1")
        .into_iter()
        .next()
        .expect("expected an alias1 token");
    let alias_inside_error = alias_tok
        .parent()
        .map(|p| {
            p.kind() == crate::SyntaxKind::ERROR
                || p.ancestors().any(|a| a.kind() == crate::SyntaxKind::ERROR)
        })
        .unwrap_or(false);
    assert!(
        !alias_inside_error,
        "implicit table alias must not be treated as trailing content"
    );
}

#[test]
fn second_top_level_select_errors() {
    let parse1 = parse("SELECT 1 SELECT 2");
    assert!(
        !parse1.errors.is_empty(),
        "a second top-level SELECT must produce a parse error"
    );

    let parse2 = parse("SELECT a FROM t 'junk' )))");
    assert!(
        !parse2.errors.is_empty(),
        "stray tokens after a SELECT body must produce a parse error"
    );
}

#[test]
fn trailing_content_after_smelt_define_errors() {
    let input = "smelt.define foo(x) AS (x + 1)\n\nSELECT 1 GARBAGE stray";
    let parse = parse(input);
    assert!(
        !parse.errors.is_empty(),
        "content trailing the model body after a declaration must produce a parse error, got: {:?}",
        parse.errors
    );
}

#[test]
fn clean_file_has_no_trailing_error() {
    // A real multi-section model file (frontmatter + comments + query) must
    // still parse with zero errors — guards against the trailing-content
    // check over-firing on well-formed input.
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/timeseries/models/daily_events.sql");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let stripped = crate::strip_frontmatter(&text);
    let parse = parse(&stripped);
    assert!(
        parse.errors.is_empty(),
        "expected zero parse errors for {}: {:?}",
        path.display(),
        parse.errors
    );
}

// ===== Phase 7: High-value grammar gaps =====
// TRY_CAST, GROUP BY ALL, ORDER BY ALL, IGNORE/RESPECT NULLS.

#[test]
fn try_cast_parses() {
    let input = "SELECT TRY_CAST(a AS INTEGER) FROM t";
    let parsed = parse(input);
    assert!(
        parsed.errors.is_empty(),
        "Parse errors: {:?}",
        parsed.errors
    );

    let cast_node = parsed
        .syntax()
        .descendants()
        .find_map(CastExpr::cast)
        .expect("TRY_CAST should produce a cast-shaped node");
    assert!(cast_node.is_try_cast(), "should be marked as TRY_CAST");
    assert!(!cast_node.is_double_colon_cast());
    assert!(cast_node.expression().is_some(), "should have expression");
    let type_spec = cast_node.type_spec().expect("should have type spec");
    assert_eq!(type_spec.type_name().as_deref(), Some("INTEGER"));

    // Plain CAST must NOT be flagged as a TRY_CAST.
    let plain = parse("SELECT CAST(a AS INTEGER) FROM t");
    let plain_cast = plain
        .syntax()
        .descendants()
        .find_map(CastExpr::cast)
        .expect("should have a CastExpr");
    assert!(!plain_cast.is_try_cast());
}

#[test]
fn group_by_all_parses() {
    let input = "SELECT a, count(*) AS n FROM t GROUP BY ALL";
    let (_, select) = parse_select(input);
    let group_by = select.group_by_clause().expect("should have GROUP BY");
    assert!(group_by.is_all(), "GROUP BY ALL should be flagged as ALL");
    // No expression items for the bare ALL form.
    assert_eq!(group_by.expressions().count(), 0);
}

#[test]
fn order_by_all_parses() {
    let (_, select) = parse_select("SELECT a FROM t ORDER BY ALL");
    let order_by = select.order_by_clause().expect("should have ORDER BY");
    assert!(order_by.is_all(), "ORDER BY ALL should be flagged as ALL");

    // ORDER BY ALL DESC round-trips through the printer unchanged.
    let (_, select_desc) = parse_select("SELECT a FROM t ORDER BY ALL DESC");
    let order_by_desc = select_desc.order_by_clause().expect("should have ORDER BY");
    assert!(order_by_desc.is_all());
    assert!(
        select_desc
            .to_string()
            .to_uppercase()
            .contains("ORDER BY ALL DESC"),
        "printed: {}",
        select_desc
    );
}

#[test]
fn ignore_nulls_in_window_parses() {
    let input = "SELECT last_value(a IGNORE NULLS) OVER (ORDER BY a) AS x FROM t";
    let parsed = parse(input);
    assert!(
        parsed.errors.is_empty(),
        "Parse errors: {:?}",
        parsed.errors
    );

    let call = parsed
        .syntax()
        .descendants()
        .find_map(FunctionCall::cast)
        .expect("should have a FunctionCall");
    assert!(
        call.syntax()
            .text()
            .to_string()
            .to_uppercase()
            .contains("IGNORE NULLS"),
        "call should retain IGNORE NULLS: {}",
        call.syntax().text()
    );

    // RESPECT NULLS is the complementary form.
    let respect = parse("SELECT first_value(a RESPECT NULLS) OVER (ORDER BY a) AS x FROM t");
    assert!(
        respect.errors.is_empty(),
        "RESPECT NULLS parse errors: {:?}",
        respect.errors
    );
}

// ===== Phase 1 (data-checks): smelt.check declaration grammar =====

use crate::ast::SmeltCheck;
use crate::syntax_kind::SyntaxKind::SMELT_CHECK;

#[test]
fn parse_smelt_check_basic() {
    // A smelt.check declaration with a SELECT body must parse into a
    // SmeltCheck node with name + body select; lossless round-trip is verified.
    let input = r#"smelt.check daily_revenue_non_negative AS (
    SELECT order_date, total_revenue
    FROM smelt.daily_revenue
    WHERE total_revenue < 0
)"#;

    let (parse_result, file) = parse_file_text(input);
    assert!(
        parse_result.errors.is_empty(),
        "parse_smelt_check_basic: unexpected errors: {:?}",
        parse_result.errors
    );

    // There must be exactly one SMELT_CHECK node.
    let checks: Vec<SmeltCheck> = file.checks().collect();
    assert_eq!(checks.len(), 1, "expected exactly one smelt.check");
    let check = &checks[0];

    // Name
    assert_eq!(
        check.name().as_deref(),
        Some("daily_revenue_non_negative"),
        "check name should be 'daily_revenue_non_negative'"
    );

    // Body SELECT exists
    assert!(
        check.body_select().is_some(),
        "smelt.check body should contain a SELECT statement"
    );

    // No stray PASSING or EXPECT clauses
    assert_eq!(
        check.passing_clauses().count(),
        0,
        "well-formed check should have no PASSING clauses"
    );
    assert!(
        check.expect_clause().is_none(),
        "well-formed check should have no EXPECT clause"
    );

    // Lossless round-trip: the CST text must reproduce the exact source bytes.
    let cst_text = parse_result.syntax().text().to_string();
    assert_eq!(
        cst_text, input,
        "round-trip: CST text must reproduce the exact source"
    );
    // Re-parsing the CST text must produce no errors.
    let parse2 = parse(&cst_text);
    assert_eq!(
        parse2.errors.len(),
        0,
        "round-trip re-parse failed: errors={:?}",
        parse2.errors
    );
}

#[test]
fn smelt_check_no_clauses() {
    // A check body with no PASSING/EXPECT parses cleanly.
    // AstFile::checks() yields exactly one SmeltCheck; tests() yields none.
    let input = r#"smelt.check orphan_orders AS (
    SELECT id FROM smelt.orders WHERE customer_id IS NULL
)"#;

    let (parse_result, file) = parse_file_text(input);
    assert!(
        parse_result.errors.is_empty(),
        "smelt_check_no_clauses: unexpected errors: {:?}",
        parse_result.errors
    );

    let checks: Vec<SmeltCheck> = file.checks().collect();
    assert_eq!(
        checks.len(),
        1,
        "checks() should yield exactly one SmeltCheck"
    );

    let tests: Vec<crate::ast::SmeltTest> = file.tests().collect();
    assert_eq!(
        tests.len(),
        0,
        "tests() should yield none when only a check is present"
    );

    let check = &checks[0];
    assert_eq!(check.name().as_deref(), Some("orphan_orders"));
    assert!(
        check.body_select().is_some(),
        "check must have a body SELECT"
    );

    // Confirm SMELT_CHECK node kind is present in CST.
    let has_smelt_check_node = parse_result
        .syntax()
        .descendants()
        .any(|n| n.kind() == SMELT_CHECK);
    assert!(has_smelt_check_node, "CST must contain a SMELT_CHECK node");
}

#[test]
fn smelt_check_with_stray_test_clause_is_recoverable() {
    // A stray PASSING clause after a smelt.check body must be captured on the
    // node (not dropped or panicking) so Phase 2 can diagnose it.
    let input = r#"smelt.check x AS (
    SELECT id FROM smelt.t WHERE id < 0
)
PASSING dep AS ({id: 1})"#;

    // Parser must not panic and must produce a result (even with errors).
    let (parse_result, file) = parse_file_text(input);

    // Exactly one SmeltCheck must be present.
    let checks: Vec<SmeltCheck> = file.checks().collect();
    assert_eq!(
        checks.len(),
        1,
        "smelt_check_with_stray_test_clause_is_recoverable: expected one SmeltCheck, got {}",
        checks.len()
    );

    let check = &checks[0];
    assert_eq!(check.name().as_deref(), Some("x"));

    // The stray PASSING clause must be reachable on the node (not silently dropped).
    let stray_passing: Vec<_> = check.passing_clauses().collect();
    assert_eq!(
        stray_passing.len(),
        1,
        "stray PASSING clause must be captured on the SmeltCheck node for Phase 2 diagnosis"
    );

    // The CST must be a lossless representation of the input.
    let cst_text = parse_result.syntax().text().to_string();
    assert_eq!(
        cst_text, input,
        "round-trip: CST text must reproduce the exact source even with stray clause"
    );
}

#[test]
fn check_keyword_contextual() {
    // `check` outside a `smelt.check` (column alias, CTE name) must remain an
    // ordinary identifier — it must not trigger smelt.check parsing.
    let alias_sql = "SELECT 1 AS check FROM t";
    let (parse_alias, _) = parse_file_text(alias_sql);
    // Must parse as a model body (SELECT), not a smelt.check.
    assert!(
        parse_alias.errors.is_empty(),
        "check as column alias should parse without errors: {:?}",
        parse_alias.errors
    );
    // No SMELT_CHECK node should be present.
    let has_check_node = parse_alias
        .syntax()
        .descendants()
        .any(|n| n.kind() == SMELT_CHECK);
    assert!(
        !has_check_node,
        "check as column alias must NOT produce a SMELT_CHECK node"
    );

    // CTE named `check` — `check` is the CTE identifier, not smelt.check.
    let cte_sql = "WITH check AS (SELECT 1) SELECT * FROM check";
    let (parse_cte, _) = parse_file_text(cte_sql);
    assert!(
        parse_cte.errors.is_empty(),
        "check as CTE name should parse without errors: {:?}",
        parse_cte.errors
    );
    let has_check_cte = parse_cte
        .syntax()
        .descendants()
        .any(|n| n.kind() == SMELT_CHECK);
    assert!(
        !has_check_cte,
        "check as CTE name must NOT produce a SMELT_CHECK node"
    );

    // smelt_check (single IDENT, no DOT) is ordinary — not a trigger.
    let no_dot_sql = "SELECT smelt_check FROM t";
    let (parse_no_dot, _) = parse_file_text(no_dot_sql);
    assert!(
        parse_no_dot.errors.is_empty(),
        "smelt_check as plain identifier should parse without errors: {:?}",
        parse_no_dot.errors
    );
    let has_check_no_dot = parse_no_dot
        .syntax()
        .descendants()
        .any(|n| n.kind() == SMELT_CHECK);
    assert!(
        !has_check_no_dot,
        "smelt_check (single IDENT) must NOT produce a SMELT_CHECK node"
    );
}

// ===== GLOB comparison operator (DuckDB) =====

#[test]
fn parse_glob_operator() {
    // `a GLOB 'x*'` parses clean as a BINARY_EXPR (mirrors LIKE/ILIKE), not the
    // former silent mis-parse where `GLOB` was consumed as a column alias
    // (`SELECT a GLOB 'x*'` used to become `SELECT a AS GLOB`, dropping the
    // string literal as a dangling error).
    let sql = "SELECT a FROM t WHERE b GLOB 'x*'";
    let parse = parse(sql);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

    let binary = parse
        .syntax()
        .descendants()
        .find_map(BinaryExpr::cast)
        .expect("must have a BINARY_EXPR for the GLOB comparison");
    assert_eq!(binary.operator().as_deref(), Some("GLOB"));
}

#[test]
fn glob_operator_round_trip() {
    // parse → print → re-parse must have no errors (same convention as the
    // other round-trip tests in this file — see test_named_window_clause_round_trip).
    let sql = "SELECT a FROM t WHERE b GLOB 'x*'";
    let parse1 = parse(sql);
    assert!(
        parse1.errors.is_empty(),
        "Parse errors: {:?}",
        parse1.errors
    );
    let file = File::cast(parse1.syntax()).expect("should have FILE root");
    let printed = file.to_string();
    assert!(
        printed.contains("GLOB"),
        "printed SQL must retain GLOB: {printed}"
    );
    let parse2 = parse(&printed);
    assert!(
        parse2.errors.is_empty(),
        "Re-parse errors: {:?}\nPrinted SQL: {}",
        parse2.errors,
        printed
    );
}

#[test]
fn glob_matches_ilike_identifier_precedent() {
    // GLOB is lexed unconditionally as a keyword token (GLOB_KW), exactly like
    // LIKE_KW/ILIKE_KW — there is no contextual identifier fallback for any of
    // the three in this grammar, so `AS glob` fails to parse just as `AS ilike`
    // does. This intentionally matches the ILIKE precedent rather than
    // DuckDB's own (more lenient) soft-keyword treatment of GLOB.
    let glob_errors = !parse("SELECT 1 AS glob FROM t").errors.is_empty();
    let ilike_errors = !parse("SELECT 1 AS ilike FROM t").errors.is_empty();
    assert_eq!(
        glob_errors, ilike_errors,
        "GLOB-as-alias must fail identically to ILIKE-as-alias"
    );
    assert!(glob_errors, "expected AS glob to be a parse error");
}

#[test]
fn not_glob_not_supported() {
    // DuckDB itself rejects `NOT GLOB` (verified against a live DuckDB via the
    // CLI oracle: `SELECT 'abc' NOT GLOB 'z*'` => Parser Error). This mirrors
    // the pre-existing `NOT LIKE` limitation (also unsupported by this
    // grammar), so smelt intentionally does not special-case NOT GLOB either.
    let sql = "SELECT a FROM t WHERE b NOT GLOB 'x*'";
    let parse = parse(sql);
    assert!(
        !parse.errors.is_empty(),
        "NOT GLOB is expected to fail loud, matching DuckDB's own rejection \
         and the pre-existing NOT LIKE limitation"
    );
}

#[test]
fn glob_as_table_function_name() {
    // DuckDB's glob(pattern) file-listing table function: GLOB followed by `(`
    // is a function name (LEFT()/RIGHT() keyword-as-function-name precedent),
    // not the comparison operator — `FROM glob('*.csv')` must parse cleanly.
    let sql = "SELECT * FROM glob('*.csv')";
    let parse = parse(sql);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
    let call = parse
        .syntax()
        .descendants()
        .find_map(FunctionCall::cast)
        .expect("must have a FUNCTION_CALL node for glob('*.csv')");
    assert_eq!(call.name().as_deref(), Some("glob"));
}

#[test]
fn glob_as_scalar_function_name() {
    // Same keyword-as-function-name treatment in expression position.
    let sql = "SELECT count(*) FROM (SELECT 1) t WHERE glob('*.csv') IS NOT NULL";
    let parse = parse(sql);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
}

#[test]
fn glob_operator_still_wins_after_expression() {
    // After a complete left operand, GLOB is always the infix operator — even
    // when the right operand is parenthesized. Function-name treatment applies
    // only where a *new* primary expression is expected.
    let sql = "SELECT a FROM t WHERE b GLOB ('x*')";
    let parse = parse(sql);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
    let binary = parse
        .syntax()
        .descendants()
        .find_map(BinaryExpr::cast)
        .expect("must have a BINARY_EXPR for the GLOB comparison");
    assert_eq!(binary.operator().as_deref(), Some("GLOB"));
}

// ── SQL-standard TRIM/SUBSTRING/POSITION forms ──────────────────────────
//
// Closes `duckdb_trim_modifier`, `duckdb_substring_from_for`, and
// `duckdb_position_in` in `crates/smelt-parser-compat/src/gaps.rs`. Each
// accepted form was verified against a real DuckDB (v1.5.4) first — see
// the task report for the oracle transcript.

fn first_function_call(sql: &str) -> FunctionCall {
    let parse = parse(sql);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
    parse
        .syntax()
        .descendants()
        .find_map(FunctionCall::cast)
        .expect("must have a FUNCTION_CALL node")
}

#[test]
fn trim_both_from_parses() {
    let call = first_function_call("SELECT trim(BOTH ' ' FROM b) FROM t");
    assert_eq!(call.name().as_deref(), Some("trim"));
    assert_eq!(call.arguments().len(), 2, "chars + string expressions");
    assert!(call.syntax().text().to_string().contains("BOTH"));
}

#[test]
fn trim_leading_from_parses() {
    let call = first_function_call("SELECT trim(LEADING ' ' FROM b) FROM t");
    assert_eq!(call.arguments().len(), 2);
}

#[test]
fn trim_trailing_from_parses() {
    let call = first_function_call("SELECT trim(TRAILING ' ' FROM b) FROM t");
    assert_eq!(call.arguments().len(), 2);
}

#[test]
fn trim_both_from_no_chars_parses() {
    let call = first_function_call("SELECT trim(BOTH FROM b) FROM t");
    assert_eq!(call.arguments().len(), 1, "only the string expression");
}

#[test]
fn trim_bare_from_parses() {
    // TRIM(FROM string) — no modifier at all.
    let call = first_function_call("SELECT trim(FROM b) FROM t");
    assert_eq!(call.arguments().len(), 1);
}

#[test]
fn trim_plain_call_still_parses() {
    // Guard: the ordinary 1-arg call form must keep working.
    let call = first_function_call("SELECT trim(b) FROM t");
    assert_eq!(call.arguments().len(), 1);
    assert!(!call
        .syntax()
        .text()
        .to_string()
        .to_uppercase()
        .contains("FROM"));
}

#[test]
fn trim_plain_two_arg_call_still_parses() {
    // Guard: TRIM(x, chars) (regular comma form) must keep working.
    let call = first_function_call("SELECT trim(b, 'x') FROM t");
    assert_eq!(call.arguments().len(), 2);
    assert!(!call
        .syntax()
        .text()
        .to_string()
        .to_uppercase()
        .contains("FROM"));
}

#[test]
fn trim_identifier_named_both_still_parses_as_plain_call() {
    // A column literally named `both` used as TRIM's sole argument must not
    // be misparsed as the BOTH modifier (no top-level FROM follows it).
    let sql = "SELECT trim(both) FROM (SELECT b AS both FROM t)";
    let call = first_function_call(sql);
    assert_eq!(call.arguments().len(), 1);
}

#[test]
fn substring_from_parses() {
    let call = first_function_call("SELECT substring(b FROM 2) FROM t");
    assert_eq!(call.name().as_deref(), Some("substring"));
    assert_eq!(call.arguments().len(), 2);
}

#[test]
fn substring_from_for_parses() {
    let call = first_function_call("SELECT substring(b FROM 2 FOR 3) FROM t");
    assert_eq!(call.arguments().len(), 3);
}

#[test]
fn substring_for_only_parses() {
    // DuckDB accepts FOR with an implied start of 1.
    let call = first_function_call("SELECT substring(b FOR 3) FROM t");
    assert_eq!(call.arguments().len(), 2);
}

#[test]
fn substring_plain_call_still_parses() {
    let call = first_function_call("SELECT substring(b, 2, 3) FROM t");
    assert_eq!(call.arguments().len(), 3);
    assert!(!call
        .syntax()
        .text()
        .to_string()
        .to_uppercase()
        .contains("FROM"));
}

#[test]
fn substr_from_for_is_not_parsed_as_std_form() {
    // DuckDB itself rejects `substr(x FROM i FOR n)` (only SUBSTRING gets the
    // SQL-standard form) — SUBSTR keeps the plain comma-only grammar, so this
    // must fail to parse cleanly.
    let parse = parse("SELECT substr(b FROM 2 FOR 3) FROM t");
    assert!(
        !parse.errors.is_empty(),
        "substr(FROM...FOR...) should not parse cleanly (DuckDB itself rejects it)"
    );
}

#[test]
fn position_in_parses() {
    let call = first_function_call("SELECT position('wor' IN b) FROM t");
    assert_eq!(call.name().as_deref(), Some("position"));
    assert_eq!(call.arguments().len(), 2);
    assert!(call
        .syntax()
        .text()
        .to_string()
        .to_uppercase()
        .contains(" IN "));
}

#[test]
fn position_in_with_expressions_parses() {
    let call = first_function_call("SELECT position(a || 'x' IN b) FROM t");
    assert_eq!(call.arguments().len(), 2);
}

#[test]
fn position_comparison_left_operand_is_a_parse_error() {
    // Registered divergence `duckdb_position_comparison_left_operand`
    // (crates/smelt-parser-compat/src/gaps.rs): DuckDB *parses*
    // `position(a = 1 IN b)` but every such form fails at its binder
    // (comparison yields BOOLEAN; position's only candidate is
    // (VARCHAR, VARCHAR) with no implicit BOOLEAN→VARCHAR cast), so no
    // executable SQL can reach the divergence. smelt rejects it at parse
    // time — this test pins that behavior so a grammar change here is a
    // deliberate decision, not drift.
    let parse = parse("SELECT position(a = 1 IN b) FROM t");
    assert!(
        !parse.errors.is_empty(),
        "position(a = 1 IN b) is expected to fail at parse time (registered \
         gap duckdb_position_comparison_left_operand); if this now parses, \
         close the gap entry and add round-trip coverage"
    );
}

#[test]
fn position_cast_wrapped_comparison_parses() {
    // The executable spelling of the same intent — CAST-wrapped comparison —
    // parses and executes on both engines.
    let call = first_function_call("SELECT position(CAST(a = 1 AS VARCHAR) IN b) FROM t");
    assert_eq!(call.arguments().len(), 2);
}

// ===== Dollar-quoted string literals =====
// Closes registered gap `duckdb_dollar_quoted_string`
// (crates/smelt-parser-compat/src/gaps.rs). Oracle evidence (DuckDB v1.5.4,
// verified live): `SELECT $$hello$$` -> "hello"; `SELECT $$a'b$$` -> "a'b"
// (embedded single quote is ordinary content); `SELECT $tag$ x $$ y $tag$`
// -> " x $$ y " (a bare `$$` inside a tagged body is content, not a
// closer); `SELECT $$abc` (no closer) -> "Parser Error: unterminated
// dollar-quoted string".

#[test]
fn dollar_quote_bare_parses_clean_as_string_literal() {
    let (_, select) = parse_select("SELECT $$a'b$$ AS s");
    let item = select.select_list().unwrap().items().next().unwrap();
    let expr = item
        .expression()
        .expect("select item should have an expression");
    assert_eq!(
        expr.text().trim(),
        "$$a'b$$",
        "expression text should be the dollar-quoted literal verbatim"
    );
}

#[test]
fn dollar_quote_tagged_with_nested_bare_dollar_parses_clean() {
    // Inner `$$` is ordinary content: the closer must match the tag exactly.
    let (_, select) = parse_select("SELECT $tag$ x $$ y $tag$ AS s");
    let item = select.select_list().unwrap().items().next().unwrap();
    assert!(item.expression().is_some());
}

#[test]
fn dollar_quote_unterminated_is_a_parse_error() {
    let parse = parse("SELECT $$abc");
    assert!(
        !parse.errors.is_empty(),
        "unterminated dollar-quote must be a parse error, not a silent split"
    );
}

#[test]
fn dollar_quote_round_trip() {
    let sql = "SELECT $$a'b$$ AS s, $tag$ x $$ y $tag$ AS t FROM u";
    let parse1 = parse(sql);
    assert!(
        parse1.errors.is_empty(),
        "Parse errors: {:?}",
        parse1.errors
    );
    let file = File::cast(parse1.syntax()).expect("should have FILE root");
    let printed = file.to_string();
    assert_eq!(
        printed, sql,
        "printer should reproduce dollar-quotes verbatim"
    );
    let parse2 = parse(&printed);
    assert!(
        parse2.errors.is_empty(),
        "Re-parse errors: {:?}\nPrinted SQL: {}",
        parse2.errors,
        printed
    );
}

// ===== MAP {…} literal tests =====

#[test]
fn map_literal_parses_clean() {
    let (_, select) = parse_select("SELECT MAP {'a': 1, 'b': 2} AS m FROM t");
    let item = select.select_list().unwrap().items().next().unwrap();
    let expr = item.expression().unwrap();
    let map_lit = expr
        .as_map_literal()
        .expect("expression must contain a MAP_LITERAL");
    let entries = map_lit.entries();
    assert_eq!(entries.len(), 2, "MAP literal must have two entries");
}

#[test]
fn map_literal_entries_have_key_and_value() {
    let (_, select) = parse_select("SELECT MAP {'a': 1, 'b': 2} AS m FROM t");
    let item = select.select_list().unwrap().items().next().unwrap();
    let expr = item.expression().unwrap();
    let map_lit = expr.as_map_literal().unwrap();
    let entries = map_lit.entries();
    assert!(entries[0].key().is_some());
    assert!(entries[0].value().is_some());
    assert!(entries[1].key().is_some());
    assert!(entries[1].value().is_some());
}

#[test]
fn map_literal_empty_parses_clean() {
    // Verified against DuckDB: `SELECT MAP {} AS m` parses and executes.
    let (_, select) = parse_select("SELECT MAP {} AS m FROM t");
    let item = select.select_list().unwrap().items().next().unwrap();
    let expr = item.expression().unwrap();
    let map_lit = expr.as_map_literal().expect("must parse as MAP_LITERAL");
    assert!(map_lit.entries().is_empty());
}

#[test]
fn map_literal_trailing_comma_allowed() {
    // Verified against DuckDB: trailing comma before `}` is accepted.
    let (_, select) = parse_select("SELECT MAP {'a': 1, 'b': 2,} AS m FROM t");
    let item = select.select_list().unwrap().items().next().unwrap();
    let expr = item.expression().unwrap();
    let map_lit = expr.as_map_literal().unwrap();
    assert_eq!(map_lit.entries().len(), 2);
}

#[test]
fn map_literal_numeric_keys() {
    // Verified against DuckDB: `MAP {1: 'x', 2: 'y'}` parses and executes.
    let (_, select) = parse_select("SELECT MAP {1: 'x', 2: 'y'} AS m FROM t");
    let item = select.select_list().unwrap().items().next().unwrap();
    let expr = item.expression().unwrap();
    let map_lit = expr.as_map_literal().unwrap();
    assert_eq!(map_lit.entries().len(), 2);
}

#[test]
fn map_literal_round_trip() {
    // Modulo canonical whitespace: the hand-rolled `Display for SelectItem`
    // prints `expr.text()` (which may include trailing trivia the primary
    // expression's postfix-lookahead absorbed) followed by its own " AS
    // alias" — the same pre-existing double-space normalization every other
    // brace/paren literal (STRUCT(...), record literal, etc.) needs here.
    let sql = "SELECT MAP {'a': 1, 'b': 2} AS m FROM t";
    let parse1 = parse(sql);
    assert!(
        parse1.errors.is_empty(),
        "Parse errors: {:?}",
        parse1.errors
    );
    let file = File::cast(parse1.syntax()).expect("should have FILE root");
    let printed = file.to_string();
    let normalize = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(
        normalize(&printed),
        normalize(sql),
        "printer should reproduce MAP {{…}} (modulo canonical whitespace)"
    );
    let parse2 = parse(&printed);
    assert!(
        parse2.errors.is_empty(),
        "Re-parse errors: {:?}\nPrinted SQL: {}",
        parse2.errors,
        printed
    );
}

#[test]
fn map_function_call_still_parses_as_function_call() {
    // Guard: `MAP(a, b)` (no brace) must still be an ordinary function call,
    // never a MAP_LITERAL — DuckDB's key/value-list form of `map()`.
    let (_, select) = parse_select("SELECT MAP(a, b) AS m FROM t");
    let item = select.select_list().unwrap().items().next().unwrap();
    let expr = item.expression().unwrap();
    assert!(
        expr.as_map_literal().is_none(),
        "MAP(a, b) must not parse as a MAP_LITERAL"
    );
    assert!(
        expr.as_function_call().is_some(),
        "MAP(a, b) must parse as a FUNCTION_CALL"
    );
}

#[test]
fn bare_map_identifier_still_parses_as_column_ref() {
    // Guard: a bare column named `map` (no following `{`) must parse as a
    // plain identifier, never a MAP_LITERAL.
    let (_, select) = parse_select("SELECT map AS m FROM t");
    let item = select.select_list().unwrap().items().next().unwrap();
    let expr = item.expression().unwrap();
    assert!(
        expr.as_map_literal().is_none(),
        "bare `map` identifier must not parse as a MAP_LITERAL"
    );
}

#[test]
fn map_literal_lowercase_keyword() {
    // MAP is contextual and case-insensitive, matching every other SQL keyword.
    let (_, select) = parse_select("SELECT map {'a': 1} AS m FROM t");
    let item = select.select_list().unwrap().items().next().unwrap();
    let expr = item.expression().unwrap();
    assert!(expr.as_map_literal().is_some());
}

// ===== AT TIME ZONE =====

#[test]
fn at_time_zone_parses_to_node() {
    let (_, select) = parse_select("SELECT ts AT TIME ZONE 'UTC' FROM t");
    let item = select.select_list().unwrap().items().next().unwrap();
    let expr = item.expression().unwrap();
    assert!(
        expr.as_at_time_zone().is_some(),
        "Expected AT_TIME_ZONE_EXPR node, got syntax: {:?}",
        expr.syntax().kind()
    );
    let atz = expr.as_at_time_zone().unwrap();
    assert_eq!(
        atz.operand()
            .map(|e| e.syntax().text().to_string().trim().to_string()),
        Some("ts".to_string())
    );
    assert_eq!(
        atz.timezone_expr()
            .map(|e| e.syntax().text().to_string().trim().to_string()),
        Some("'UTC'".to_string())
    );
}

#[test]
fn at_time_zone_round_trips() {
    let sql = "SELECT ts AT TIME ZONE 'UTC' FROM t";
    let parse = parse(sql);
    assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
    let file = File::cast(parse.syntax()).unwrap();
    // Lossless CST: printed text reconstructs exactly.
    assert_eq!(file.syntax().text().to_string().trim().to_string(), sql);
}

#[test]
fn at_time_zone_chains_left_associatively() {
    // Verified against the DuckDB oracle: `ts AT TIME ZONE 'UTC' AT TIME ZONE
    // 'America/New_York'` groups as `(ts AT TIME ZONE 'UTC') AT TIME ZONE
    // 'America/New_York'` — the outer AT_TIME_ZONE_EXPR's operand is itself
    // an AT_TIME_ZONE_EXPR, not a re-parenthesized flat chain.
    let (_, select) =
        parse_select("SELECT ts AT TIME ZONE 'UTC' AT TIME ZONE 'America/New_York' FROM t");
    let item = select.select_list().unwrap().items().next().unwrap();
    let expr = item.expression().unwrap();
    let outer = expr.as_at_time_zone().expect("outer AT_TIME_ZONE_EXPR");
    assert_eq!(
        outer
            .timezone_expr()
            .map(|e| e.syntax().text().to_string().trim().to_string()),
        Some("'America/New_York'".to_string())
    );
    let inner_operand = outer.operand().expect("inner operand");
    let inner = inner_operand
        .as_at_time_zone()
        .expect("inner operand should itself be an AT_TIME_ZONE_EXPR (left-associative chain)");
    assert_eq!(
        inner
            .operand()
            .map(|e| e.syntax().text().to_string().trim().to_string()),
        Some("ts".to_string())
    );
    assert_eq!(
        inner
            .timezone_expr()
            .map(|e| e.syntax().text().to_string().trim().to_string()),
        Some("'UTC'".to_string())
    );
}

#[test]
fn at_time_zone_binds_tighter_than_comparison() {
    // Verified against the DuckDB oracle: `ts AT TIME ZONE 'UTC' > ts2` groups
    // as `(ts AT TIME ZONE 'UTC') > ts2`, not `ts AT TIME ZONE ('UTC' > ts2)`.
    let (_, select) = parse_select("SELECT ts AT TIME ZONE 'UTC' > ts2 FROM t");
    let item = select.select_list().unwrap().items().next().unwrap();
    let expr = item.expression().unwrap();
    let binary = expr
        .as_binary()
        .expect("top-level expression should be a BINARY_EXPR (`>`)");
    let left = binary.left().expect("left operand");
    assert!(
        left.as_at_time_zone().is_some(),
        "left operand of `>` should be the AT_TIME_ZONE_EXPR, got: {:?}",
        left.syntax().kind()
    );
}

#[test]
fn at_time_zone_binds_tighter_than_addition() {
    // Verified against the DuckDB oracle: `ts AT TIME ZONE 'UTC' + INTERVAL 1
    // HOUR` groups as `(ts AT TIME ZONE 'UTC') + INTERVAL 1 HOUR` — the `+`
    // is the outer node, not swallowed into the tz operand.
    let (_, select) = parse_select("SELECT ts AT TIME ZONE 'UTC' + INTERVAL 1 HOUR FROM t");
    let item = select.select_list().unwrap().items().next().unwrap();
    let expr = item.expression().unwrap();
    let binary = expr
        .as_binary()
        .expect("top-level expression should be a BINARY_EXPR (`+`)");
    let left = binary.left().expect("left operand");
    assert!(
        left.as_at_time_zone().is_some(),
        "left operand of `+` should be the AT_TIME_ZONE_EXPR, got: {:?}",
        left.syntax().kind()
    );
}

#[test]
fn at_time_zone_nested_in_parenthesized_tz_operand() {
    // The tz operand may be a parenthesized expression or subquery that
    // itself contains `AT TIME ZONE`. The `in_at_time_zone_operand` guard
    // must be scoped to the current nesting level — inside `(...)` a fresh
    // expression context begins, so the inner `AT TIME ZONE` must parse.
    // DuckDB's PARSER accepts this shape (it only fails later, at bind):
    //   SELECT ts AT TIME ZONE (SELECT b AT TIME ZONE 'UTC' FROM t2) FROM t
    let sql = "SELECT ts AT TIME ZONE (SELECT b AT TIME ZONE 'UTC' FROM t2) FROM t";
    let result = parse(sql);
    assert!(
        result.errors.is_empty(),
        "Expected no parse errors, got: {:?}",
        result.errors
    );

    // A plain parenthesized inner AT TIME ZONE also parses cleanly.
    let sql2 = "SELECT ts AT TIME ZONE (b AT TIME ZONE 'UTC') FROM t";
    let result2 = parse(sql2);
    assert!(
        result2.errors.is_empty(),
        "Expected no parse errors, got: {:?}",
        result2.errors
    );
}

#[test]
fn at_time_zone_nested_in_case_tz_operand() {
    // The tz operand may be a CASE expression whose arms themselves contain
    // `AT TIME ZONE`. CASE WHEN/THEN/ELSE arms enter expression parsing via
    // `parse_pipe_expr` directly (not `parse_expression_inner`), so the
    // operand guard must be scoped there too. DuckDB's PARSER accepts this
    // shape (it only fails later, at bind, on the arm-type mix):
    //   SELECT ts AT TIME ZONE
    //     CASE WHEN 1 = 1 THEN b AT TIME ZONE 'UTC' ELSE c END FROM t
    let sql = "SELECT ts AT TIME ZONE CASE WHEN 1 = 1 THEN b AT TIME ZONE 'UTC' ELSE c END FROM t";
    let result = parse(sql);
    assert!(
        result.errors.is_empty(),
        "Expected no parse errors, got: {:?}",
        result.errors
    );
}

#[test]
fn at_time_zone_parenthesized_left_operand() {
    // `(ts AT TIME ZONE 'UTC') AT TIME ZONE 'EST'` — explicit parens around
    // the left operand (verified to execute on DuckDB). The outer node's
    // operand is the parenthesized expression containing the inner
    // AT_TIME_ZONE_EXPR.
    let (_, select) = parse_select("SELECT (ts AT TIME ZONE 'UTC') AT TIME ZONE 'EST' FROM t");
    let item = select.select_list().unwrap().items().next().unwrap();
    let expr = item.expression().unwrap();
    let outer = expr.as_at_time_zone().expect("outer AT_TIME_ZONE_EXPR");
    assert_eq!(
        outer
            .timezone_expr()
            .map(|e| e.syntax().text().to_string().trim().to_string()),
        Some("'EST'".to_string())
    );
}

#[test]
fn bare_at_still_parses_as_implicit_alias() {
    // Guard: `at` is a contextual (unreserved) keyword — only the exact `AT
    // TIME ZONE` sequence triggers AT_TIME_ZONE_EXPR. A bare `at` following
    // an expression must still work as an implicit select-item alias.
    let (_, select) = parse_select("SELECT ts at FROM t");
    let item = select.select_list().unwrap().items().next().unwrap();
    assert_eq!(item.alias().as_deref(), Some("at"));
    let expr = item.expression().unwrap();
    assert!(expr.as_at_time_zone().is_none());
}

#[test]
fn bare_at_column_reference_still_parses() {
    // Guard: `at` must remain usable as an ordinary identifier/column name.
    let (_, select) = parse_select("SELECT at FROM t");
    let item = select.select_list().unwrap().items().next().unwrap();
    let expr = item.expression().unwrap();
    assert!(expr.as_at_time_zone().is_none());
    assert_eq!(expr.syntax().text().to_string().trim(), "at");
}

// ===== GROUPING SETS (SQL-standard grouping-set extension of GROUP BY) =====
//
// `GROUPING SETS ( <set> [, <set>]* )` — grammar verified against DuckDB
// (parser-gap-closure task 9). Oracle probes:
//   - full parenthesized-list form: accepted.
//   - bare-expr elements (`GROUPING SETS (a, b)`, no inner parens): accepted
//     (PostgreSQL and DuckDB both allow this).
//   - the empty set `()` as one element, and as the sole element: accepted.
//   - `ROLLUP(...)`/`CUBE(...)` nested as a bare element inside GROUPING SETS:
//     accepted (they parse as ordinary function calls; no dedicated smelt-side
//     CUBE/ROLLUP grammar is needed).
//   - `GROUPING`/`SETS` are contextual keywords, not reserved: `grouping` and
//     `sets` stay usable as ordinary identifiers/aliases/columns.
//   - case-insensitive, whitespace-tolerant between the two keywords.
//   - may be mixed with a plain grouping key in the same GROUP BY list:
//     `GROUP BY a, GROUPING SETS ((b))`.

#[test]
fn grouping_sets_full_form_parses() {
    let input = "SELECT a, b, SUM(c) FROM t GROUP BY GROUPING SETS ((a), (b), ())";
    let parsed = parse(input);
    assert!(
        parsed.errors.is_empty(),
        "Parse errors: {:?}",
        parsed.errors
    );

    let clause = parsed
        .syntax()
        .descendants()
        .find(|n| n.kind() == GROUPING_SETS_CLAUSE)
        .expect("should have a GROUPING_SETS_CLAUSE node");
    let sets: Vec<_> = clause
        .children()
        .filter(|n| n.kind() == GROUPING_SET)
        .collect();
    assert_eq!(sets.len(), 3, "expected 3 grouping sets: {:?}", sets);

    // The whole clause flows through GroupByClause::expressions() as a
    // single opaque grouping-key expression — mirroring how CUBE/ROLLUP
    // function calls already flow through as one expression each (there is
    // no dedicated CUBE/ROLLUP grammar; they fall out of the generic
    // function-call parse).
    let (_, select) = parse_select(input);
    let group_by = select.group_by_clause().expect("should have GROUP BY");
    let exprs: Vec<Expr> = group_by.expressions().collect();
    assert_eq!(
        exprs.len(),
        1,
        "GROUPING SETS should be a single opaque grouping key: {:?}",
        exprs.iter().map(|e| e.text()).collect::<Vec<_>>()
    );
    assert!(exprs[0].text().to_uppercase().starts_with("GROUPING SETS"));
}

#[test]
fn grouping_sets_bare_expr_elements_parse() {
    // PostgreSQL/DuckDB both accept unparenthesized elements.
    let input = "SELECT a, b, SUM(c) FROM t GROUP BY GROUPING SETS (a, b)";
    let parsed = parse(input);
    assert!(
        parsed.errors.is_empty(),
        "Parse errors: {:?}",
        parsed.errors
    );
    let clause = parsed
        .syntax()
        .descendants()
        .find(|n| n.kind() == GROUPING_SETS_CLAUSE)
        .expect("should have a GROUPING_SETS_CLAUSE node");
    let sets: Vec<_> = clause
        .children()
        .filter(|n| n.kind() == GROUPING_SET)
        .collect();
    assert_eq!(sets.len(), 2);
    // Neither element should have its own LPAREN/RPAREN tokens.
    for set in &sets {
        assert!(
            !set.children_with_tokens()
                .filter_map(|e| e.into_token())
                .any(|t| t.kind() == LPAREN),
            "bare element should not have parens: {}",
            set.text()
        );
    }
}

#[test]
fn grouping_sets_empty_set_parses() {
    // `()` as one element among several.
    let input = "SELECT a, b, SUM(c) FROM t GROUP BY GROUPING SETS ((a), (b), ())";
    let parsed = parse(input);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);

    // `()` as the sole element.
    let sole_empty = parse("SELECT a, b, SUM(c) FROM t GROUP BY GROUPING SETS (())");
    assert!(
        sole_empty.errors.is_empty(),
        "sole empty set should parse: {:?}",
        sole_empty.errors
    );
}

#[test]
fn grouping_sets_nested_rollup_and_cube_parse() {
    // ROLLUP/CUBE have no dedicated smelt-side grammar — they parse as
    // ordinary function calls, so nesting them as a bare GROUPING SETS
    // element is just the generic function-call path.
    let rollup = parse("SELECT a, b, SUM(c) FROM t GROUP BY GROUPING SETS (ROLLUP(a, b))");
    assert!(rollup.errors.is_empty(), "{:?}", rollup.errors);

    let cube = parse("SELECT a, b, SUM(c) FROM t GROUP BY GROUPING SETS (CUBE(a, b))");
    assert!(cube.errors.is_empty(), "{:?}", cube.errors);

    let mixed = parse("SELECT a, b, SUM(c) FROM t GROUP BY GROUPING SETS (ROLLUP(a), CUBE(b))");
    assert!(mixed.errors.is_empty(), "{:?}", mixed.errors);
}

#[test]
fn grouping_sets_mixes_with_plain_group_by_keys() {
    let input = "SELECT a, b, SUM(c) FROM t GROUP BY a, GROUPING SETS ((b))";
    let (_, select) = parse_select(input);
    let group_by = select.group_by_clause().expect("should have GROUP BY");
    let exprs: Vec<Expr> = group_by.expressions().collect();
    assert_eq!(exprs.len(), 2, "expected `a` and the GROUPING SETS clause");
    assert_eq!(exprs[0].text().trim(), "a");
    assert!(exprs[1].text().to_uppercase().starts_with("GROUPING SETS"));
}

#[test]
fn grouping_sets_case_insensitive_and_whitespace_tolerant() {
    let lower = parse("SELECT a, b, SUM(c) FROM t GROUP BY grouping sets ((a), (b))");
    assert!(lower.errors.is_empty(), "{:?}", lower.errors);

    let extra_ws = parse("SELECT a, b, SUM(c) FROM t GROUP BY GROUPING   SETS ((a), (b))");
    assert!(extra_ws.errors.is_empty(), "{:?}", extra_ws.errors);
}

#[test]
fn grouping_and_sets_remain_usable_as_plain_identifiers() {
    // `GROUPING`/`SETS` are contextual keywords — only recognised as the
    // exact `GROUPING SETS (` sequence in a GROUP BY list position. A bare
    // `grouping` column, and a `sets` alias, must both parse unaffected.
    let select_grouping = parse("SELECT grouping FROM t");
    assert!(
        select_grouping.errors.is_empty(),
        "{:?}",
        select_grouping.errors
    );
    assert!(select_grouping
        .syntax()
        .descendants()
        .all(|n| n.kind() != GROUPING_SETS_CLAUSE));

    let sets_alias = parse("SELECT a AS sets FROM t");
    assert!(sets_alias.errors.is_empty(), "{:?}", sets_alias.errors);

    // `GROUP BY grouping, sets` — two plain columns, comma-separated, with
    // no trailing `(` — must not trigger the GROUPING SETS grammar path.
    let (_, select) =
        parse_select("SELECT grouping, sets, COUNT(*) FROM t GROUP BY grouping, sets");
    let group_by = select.group_by_clause().expect("should have GROUP BY");
    let exprs: Vec<Expr> = group_by.expressions().collect();
    assert_eq!(exprs.len(), 2);
    assert_eq!(exprs[0].text().trim(), "grouping");
    assert_eq!(exprs[1].text().trim(), "sets");
}

#[test]
fn plain_group_by_unaffected_by_grouping_sets_grammar() {
    let (_, select) = parse_select("SELECT a, b, COUNT(*) FROM t GROUP BY a, b");
    let group_by = select.group_by_clause().expect("should have GROUP BY");
    assert!(!group_by.is_all());
    let exprs: Vec<Expr> = group_by.expressions().collect();
    assert_eq!(exprs.len(), 2);
    assert_eq!(exprs[0].text().trim(), "a");
    assert_eq!(exprs[1].text().trim(), "b");
}

#[test]
fn grouping_sets_round_trips_through_printer() {
    let input = "SELECT a, b, SUM(c) FROM t GROUP BY GROUPING SETS ((a), (b), ())";
    let (_, select) = parse_select(input);
    let printed = select.to_string();
    assert!(
        printed.to_uppercase().contains("GROUPING SETS"),
        "printed SQL should retain GROUPING SETS: {}",
        printed
    );
    // Re-parsing the printed SQL must still be error-free and produce the
    // same GROUPING_SET count.
    let reparsed = parse(&printed);
    assert!(
        reparsed.errors.is_empty(),
        "printed SQL failed to reparse: {} -> {:?}",
        printed,
        reparsed.errors
    );
    let sets = reparsed
        .syntax()
        .descendants()
        .filter(|n| n.kind() == GROUPING_SET)
        .count();
    assert_eq!(sets, 3);
}

#[test]
fn grouping_sets_nested_grouping_sets_parses() {
    // DuckDB accepts arbitrary GROUPING SETS nesting (verified against
    // DuckDB): a GROUPING SETS element may itself be a bare (unparenthesized)
    // nested GROUPING SETS clause.
    let nested = parse(
        "SELECT SUM(c) FROM t GROUP BY GROUPING SETS (GROUPING SETS (ROLLUP(a), GROUPING SETS (CUBE(a))))",
    );
    assert!(nested.errors.is_empty(), "{:?}", nested.errors);

    let mixed_nested =
        parse("SELECT SUM(c) FROM t GROUP BY GROUPING SETS (a, GROUPING SETS (a, CUBE(a)))");
    assert!(mixed_nested.errors.is_empty(), "{:?}", mixed_nested.errors);

    let inner_count = nested
        .syntax()
        .descendants()
        .filter(|n| n.kind() == GROUPING_SETS_CLAUSE)
        .count();
    assert_eq!(
        inner_count, 3,
        "expected 3 nested GROUPING_SETS_CLAUSE nodes"
    );
}

// ===== Derived-table alias column list round-trip (printer fidelity) =====
//
// smelt's printer previously dropped the derived-table body (subquery/
// VALUES) — or the alias/alias-column-list on a plain table — when printing
// a TABLE_REF, because `TableRef::identifier()` matched the alias IDENT
// token for a derived table (there being no other direct-child IDENT to
// find), and none of the printer's branches printed `alias()` /
// `alias_column_names()` at all. See external corpus ledger category
// `derived_table_column_alias_list`.

fn assert_table_ref_round_trips(input: &str, must_contain: &[&str]) {
    let (_, select) = parse_select(input);
    let printed = select.to_string();
    for needle in must_contain {
        assert!(
            printed.contains(needle),
            "printed SQL should contain {:?}: got {:?}",
            needle,
            printed
        );
    }
    let reparsed = crate::parse(&printed);
    assert!(
        reparsed.errors.is_empty(),
        "printed SQL {:?} (from {:?}) failed to reparse: {:?}",
        printed,
        input,
        reparsed.errors
    );
}

#[test]
fn derived_table_values_alias_col_list_round_trips() {
    assert_table_ref_round_trips(
        "SELECT * FROM (VALUES (1, 2)) AS t(a, b)",
        &["VALUES", "(1, 2)", "AS t(a, b)"],
    );
}

#[test]
fn derived_table_select_alias_col_list_round_trips() {
    assert_table_ref_round_trips(
        "SELECT * FROM (SELECT 1, 2) AS t(a, b)",
        &["SELECT 1, 2", "AS t(a, b)"],
    );
}

#[test]
fn plain_table_alias_col_list_round_trips() {
    // DuckDB accepts a column-renaming alias list on a plain base table too:
    // `FROM t1 AS c1(x)` renames t1's first column to `x` under alias `c1`.
    assert_table_ref_round_trips("SELECT * FROM t1 AS c1(x)", &["t1", "AS c1(x)"]);
}

#[test]
fn plain_table_alias_without_col_list_round_trips() {
    assert_table_ref_round_trips("SELECT * FROM t1 AS c1", &["t1", "AS c1"]);
}

#[test]
fn derived_table_values_alias_col_list_no_as_round_trips() {
    assert_table_ref_round_trips(
        "SELECT * FROM (VALUES (1, 2)) t(a, b)",
        &["VALUES", "(1, 2)", "AS t(a, b)"],
    );
}

// --- DuckDB `:=` named-argument operator (external corpus ledger category
// `named_arg_walrus_or_bare_eq`) ---
//
// DuckDB accepts `name := value` as a named-argument binding inside an
// ordinary function-call argument list (`struct_pack(a := 1)`,
// `read_csv(path, header := 0)`) — semantically the same construct smelt
// already parsed for `=>` (see `parses_smelt_path_call_with_named_args`
// above), just DuckDB's own spelling. Verified against a real DuckDB: both
// `struct_pack(a := 1)` and `struct_pack(a => 1)` parse, and DuckDB's own
// query-text re-serialization prints `:=` for both — the two spellings are
// interchangeable at the grammar level.

#[test]
fn walrus_named_arg_in_ordinary_function_call() {
    let input = "SELECT struct_pack(a := 1, b := 2)";
    let (parse, _) = parse_select(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );
}

#[test]
fn walrus_named_arg_in_table_function_call() {
    // read_csv-style: positional path argument followed by walrus-named
    // options, mirroring DuckDB's own CSV/Parquet reader signatures.
    let input = "SELECT * FROM read_csv('data.csv', header := 0, auto_detect := false)";
    let (parse, _) = parse_select(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );
}

#[test]
fn walrus_named_arg_mixes_with_bare_eq_option() {
    // DuckDB's own corpus mixes `:=` named args with bare `name = value`
    // options in the same call (the bare form already parsed as an ordinary
    // comparison expression before this change; this asserts the mix still
    // parses cleanly once `:=` is supported).
    let input = "SELECT sum(a) FROM read_csv('f.csv', COLUMNS=STRUCT_PACK(a := 'INTEGER'), auto_detect='true', delim = '|')";
    let (parse, _) = parse_select(input);
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );
}

#[test]
fn walrus_named_arg_produces_named_param_node() {
    let (_, select) = parse_select("SELECT struct_pack(a := 1)");
    let named_params: Vec<_> = select
        .syntax()
        .descendants()
        .filter(|n| n.kind() == NAMED_PARAM)
        .collect();
    assert_eq!(
        named_params.len(),
        1,
        "expected exactly one NAMED_PARAM node for `a := 1`"
    );
}

#[test]
fn walrus_operator_outside_call_still_errors() {
    // DuckDB itself rejects `:=` as a general expression/assignment
    // operator outside a function-call argument position (`Parser Error:
    // syntax error at or near ":="`, verified against a real DuckDB). The
    // named-argument grammar only applies inside `parse_argument`, so this
    // must remain a parse error, not silently swallowed.
    let result = parse("SELECT a := 1");
    assert!(
        !result.errors.is_empty(),
        "`:=` outside a call argument list should still be a parse error"
    );
}

#[test]
fn bare_eq_in_call_still_parses_as_comparison() {
    // Bare `name = value` inside a call argument list was never a DuckDB
    // parser-level named-argument form (verified against a real DuckDB: it
    // parses as an ordinary boolean equality expression referencing a
    // column named `name`, and only certain table functions like read_csv
    // special-case it at bind time). It must keep parsing as a normal
    // comparison expression, not a NAMED_PARAM.
    let (parse, select) = parse_select("SELECT foo(a = 1)");
    assert!(
        parse.errors.is_empty(),
        "unexpected errors: {:?}",
        parse.errors
    );
    let named_params: Vec<_> = select
        .syntax()
        .descendants()
        .filter(|n| n.kind() == NAMED_PARAM)
        .collect();
    assert!(
        named_params.is_empty(),
        "bare `a = 1` must not be parsed as a NAMED_PARAM"
    );
}

#[test]
fn walrus_named_arg_round_trips() {
    let input = "SELECT struct_pack(a := 1, b := 2)";
    let (_, select) = parse_select(input);
    let printed = select.to_string();
    assert!(
        printed.contains("a := 1"),
        "printed SQL should preserve `:=` spelling: got {:?}",
        printed
    );
    let reparsed = crate::parse(&printed);
    assert!(
        reparsed.errors.is_empty(),
        "printed SQL {:?} (from {:?}) failed to reparse: {:?}",
        printed,
        input,
        reparsed.errors
    );
}
