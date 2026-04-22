//! Regression tests for SQL standard `''` escape inside single-quoted string literals.
//!
//! `'Can''t Lose Them'` must lex as a single STRING token whose text is
//! `'Can''t Lose Them'` and whose decoded value is `Can't Lose Them`.
//! Surfaced by iter-1 of the smelt-shop 0.3 follow-up validation loop.

use smelt_parser::lexer::tokenize;
use smelt_parser::parse;
use smelt_parser::syntax_kind::SyntaxKind;

#[test]
fn lex_doubled_quote_does_not_split_string() {
    // Token-level: 'a''b' must be a single STRING token, not three.
    let tokens = tokenize("'a''b'");
    let string_tokens: Vec<_> = tokens
        .iter()
        .filter(|t| t.kind == SyntaxKind::STRING)
        .collect();
    assert_eq!(
        string_tokens.len(),
        1,
        "expected one STRING token, got tokens: {:?}",
        tokens
    );
    assert_eq!(
        string_tokens[0].len,
        "'a''b'".len(),
        "STRING token should span the whole literal including the doubled quote"
    );
}

#[test]
fn parse_string_with_escaped_quote() {
    let input = "SELECT 'Can''t Lose Them' AS label";
    let parse = parse(input);
    assert!(
        parse.errors.is_empty(),
        "expected no parse errors, got: {:?}",
        parse.errors
    );
}

#[test]
fn parse_string_with_multiple_escaped_quotes() {
    // 'a''b''c' — three logical chars separated by two escapes.
    let input = "SELECT 'a''b''c' AS label";
    let parse = parse(input);
    assert!(
        parse.errors.is_empty(),
        "expected no parse errors, got: {:?}",
        parse.errors
    );

    // And as a token: still a single STRING.
    let tokens = tokenize("'a''b''c'");
    let string_tokens: Vec<_> = tokens
        .iter()
        .filter(|t| t.kind == SyntaxKind::STRING)
        .collect();
    assert_eq!(string_tokens.len(), 1);
    assert_eq!(string_tokens[0].len, "'a''b''c'".len());
}

#[test]
fn parse_case_with_escaped_quote_literal() {
    // Mirrors the failing snippet from iter-1's mart_customer_rfm.sql.
    let input = "SELECT CASE WHEN x > 0 THEN 'Can''t Lose Them' ELSE 'Other' END AS label FROM t";
    let parse = parse(input);
    assert!(
        parse.errors.is_empty(),
        "expected no parse errors, got: {:?}",
        parse.errors
    );
}

#[test]
fn lex_doubled_quote_only_string_is_two_chars() {
    // '''' is an empty-then-escaped form: it's the 4-char source for a one-char
    // value containing a single quote. Must lex as a single STRING.
    let tokens = tokenize("''''");
    let string_tokens: Vec<_> = tokens
        .iter()
        .filter(|t| t.kind == SyntaxKind::STRING)
        .collect();
    assert_eq!(
        string_tokens.len(),
        1,
        "'''' should be a single STRING token, got tokens: {:?}",
        tokens
    );
    assert_eq!(string_tokens[0].len, 4);
}

#[test]
fn lex_empty_string_still_one_token() {
    // Sanity check that the new logic doesn't break the empty string '' case.
    // This is just the empty-string literal — it terminates immediately.
    let tokens = tokenize("''");
    let string_tokens: Vec<_> = tokens
        .iter()
        .filter(|t| t.kind == SyntaxKind::STRING)
        .collect();
    assert_eq!(string_tokens.len(), 1);
    assert_eq!(string_tokens[0].len, 2);
}
