//! Type inference for the SQL-standard TRIM/SUBSTRING/POSITION
//! keyword-argument forms.
//!
//! Closes the `duckdb_trim_modifier`, `duckdb_substring_from_for`, and
//! `duckdb_position_in` gaps in `crates/smelt-parser-compat/src/gaps.rs`.
//! The parser dispatches these forms to dedicated `ARG_LIST` parsers
//! (`crates/smelt-parser/src/parser/expr.rs`) that still produce a plain
//! `FUNCTION_CALL` node, so typing flows through the existing
//! registry-backed TRIM/SUBSTRING/POSITION function-call inference
//! unchanged — these tests assert that end-to-end result.

use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_parser::ast::File;
use smelt_types::{DataType, TypedColumn};

/// Parse a SELECT statement and infer column types using an empty context.
fn infer(sql: &str) -> Vec<TypedColumn> {
    let parse = smelt_parser::parse(sql);
    assert!(parse.errors.is_empty(), "parse errors: {:?}", parse.errors);
    let file = File::cast(parse.syntax()).expect("parse File");
    let select = file.select_stmt().expect("parse SELECT");
    infer_select_column_types(&select, &TypeContext::new())
}

#[test]
fn trim_both_from_infers_text() {
    let types = infer("SELECT trim(BOTH ' ' FROM CAST('  x  ' AS VARCHAR)) AS result");
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].data_type, DataType::Text);
}

#[test]
fn trim_bare_from_infers_text() {
    let types = infer("SELECT trim(FROM CAST('  x  ' AS VARCHAR)) AS result");
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].data_type, DataType::Text);
}

#[test]
fn substring_from_for_infers_text() {
    let types = infer("SELECT substring(CAST('hello' AS VARCHAR) FROM 2 FOR 3) AS result");
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].data_type, DataType::Text);
}

#[test]
fn substring_for_only_infers_text() {
    let types = infer("SELECT substring(CAST('hello' AS VARCHAR) FOR 3) AS result");
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].data_type, DataType::Text);
}

#[test]
fn position_in_infers_bigint() {
    // Registry.rs: POSITION -> BigInt (rg POSITION crates/smelt-types/src/signatures.rs).
    let types = infer("SELECT position('wor' IN CAST('hello world' AS VARCHAR)) AS result");
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].data_type, DataType::BigInt);
}
