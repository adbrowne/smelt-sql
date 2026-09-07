//! Pinned printer output for `%`/`^`/`**`, and direct interpreter exercises
//! (parenthesisation of compound arguments and non-call-shaped templates).

use super::{bigquery, duckdb, first_arg_node, print_template_str, spark};

/// (sql, dialect-printer, expected).
type PinnedCase = (&'static str, fn(&str) -> String, &'static str);

#[test]
fn modulo_and_power_output_is_pinned() {
    // Expected strings captured from the pre-migration
    // `print_modulo_call`/`print_power_call` implementation.
    let cases: &[PinnedCase] = &[
        // Bare columns.
        ("SELECT a % b FROM t", bigquery, "SELECT MOD(a, b) FROM t"),
        ("SELECT a % b FROM t", duckdb, "SELECT a % b FROM t"),
        ("SELECT a % b FROM t", spark, "SELECT a % b FROM t"),
        ("SELECT a ^ b FROM t", bigquery, "SELECT POWER(a, b) FROM t"),
        ("SELECT a ^ b FROM t", spark, "SELECT POWER(a, b) FROM t"),
        ("SELECT a ^ b FROM t", duckdb, "SELECT a ^ b FROM t"),
        (
            "SELECT a ** b FROM t",
            bigquery,
            "SELECT POWER(a, b) FROM t",
        ),
        ("SELECT a ** b FROM t", spark, "SELECT POWER(a, b) FROM t"),
        ("SELECT a ** b FROM t", duckdb, "SELECT a ** b FROM t"),
        // Literals.
        ("SELECT 10 % 3 FROM t", bigquery, "SELECT MOD(10, 3) FROM t"),
        // Nested calls as operands.
        (
            "SELECT f(a) % g(b) FROM t",
            bigquery,
            "SELECT MOD(f(a), g(b)) FROM t",
        ),
        // Parenthesised compound operands — parens are dropped, not
        // reproduced, exactly like the pre-migration rewrite
        // (`modulo_lowering.rs::bigquery_lowers_modulo_over_expression_operands`).
        (
            "SELECT (a + 1) % (b - 1) FROM t",
            bigquery,
            "SELECT MOD(a + 1, b - 1) FROM t",
        ),
        (
            "SELECT (a + 1) ^ (b - 1) FROM t",
            bigquery,
            "SELECT POWER(a + 1, b - 1) FROM t",
        ),
        // Nested same-operator chain — left-associative, so the left operand
        // is itself a lowered call, recursively dispatched through
        // `print_node`.
        (
            "SELECT a % b % c FROM t",
            bigquery,
            "SELECT MOD(MOD(a, b), c) FROM t",
        ),
        (
            "SELECT a ^ b ^ c FROM t",
            bigquery,
            "SELECT POWER(POWER(a, b), c) FROM t",
        ),
    ];
    for (sql, printer, expected) in cases {
        let out = printer(sql);
        assert_eq!(&out, expected, "input: {sql}");
    }
}

#[test]
fn compound_argument_is_parenthesised() {
    // `{0} - 1` is itself non-call-shaped, so its whole output is wrapped
    // once regardless of the argument (`non_call_template_is_wrapped_in_parens`
    // covers that rule in isolation); the assertions below are about the
    // *inner* pair — present only when the argument itself is compound.

    // `f(a + b)` — the argument is a bare (unparenthesised in source)
    // BINARY_EXPR: compound, so it picks up its own inner wrap on top of the
    // template's outer one.
    let compound = first_arg_node("SELECT f(a + b) FROM t");
    let out = print_template_str("{0} - 1", &[compound]);
    assert_eq!(out, "((a + b) - 1)");

    // `f((a + b))` — a parenthesised compound. The grammar drops the literal
    // source parens once already reaching this accessor (the parenthesised
    // group's own `EXPRESSION` wrapper is what `arguments()` returns, and it
    // spans just the inner content), so this prints identically to the
    // unparenthesised case above — nothing here to double-wrap.
    let paren_compound = first_arg_node("SELECT f((a + b)) FROM t");
    let out = print_template_str("{0} - 1", &[paren_compound]);
    assert_eq!(out, "((a + b) - 1)");

    // `f(x)` — a bare identifier is an atom: no inner wrap, only the
    // template's own outer one.
    let ident = first_arg_node("SELECT f(x) FROM t");
    let out = print_template_str("{0} - 1", &[ident]);
    assert_eq!(out, "(x - 1)");
}

#[test]
fn non_call_template_is_wrapped_in_parens() {
    use smelt_parser::syntax_kind::SyntaxKind;
    use smelt_parser::{parse, FunctionCall};

    // A template such as `{0} - {1}` is not call-shaped — its output must be
    // wrapped so it composes correctly inside a larger expression.
    let left = first_arg_node("SELECT f(a, b) FROM t");
    let call = parse("SELECT f(a, b) FROM t").syntax();
    let fc = call
        .descendants()
        .find(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
        .unwrap();
    let args = FunctionCall::cast(fc)
        .unwrap()
        .arguments()
        .into_iter()
        .map(|e| e.syntax().clone())
        .collect::<Vec<_>>();
    let out = print_template_str("{0} - {1}", &args);
    assert_eq!(out, "(a - b)");
    // Sanity: the call-shaped counterpart never wraps.
    let out_call_shaped = print_template_str("MOD({0}, {1})", &args);
    assert_eq!(out_call_shaped, "MOD(a, b)");
    let _ = left;
}
