//! Integration test: real workspace fixture exercises the unified
//! `smelt.<path>` value-form grammar (Phase 1 of the smelt.<path> migration).
//!
//! `examples/test_workspace/models/path_demo.sql` is a minimal model whose
//! FROM clause uses the unified value form. The parser must accept it with
//! zero diagnostics.

use std::path::PathBuf;

use smelt_parser::ast::{File, SmeltPathRef};
use smelt_parser::parse;

#[test]
fn test_workspace_path_form_parses() {
    let path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/test_workspace/models/path_demo.sql");

    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

    let parsed = parse(&text);
    assert!(
        parsed.errors.is_empty(),
        "parse errors in {}: {:?}",
        path.display(),
        parsed.errors,
    );

    let file = File::cast(parsed.syntax()).expect("expected FILE node");
    let refs: Vec<SmeltPathRef> = file
        .syntax()
        .descendants()
        .filter_map(SmeltPathRef::cast)
        .collect();
    assert_eq!(
        refs.len(),
        1,
        "expected exactly one SMELT_PATH_REF in path_demo.sql, got {}",
        refs.len()
    );
    assert_eq!(refs[0].segments(), vec!["users"]);
}
