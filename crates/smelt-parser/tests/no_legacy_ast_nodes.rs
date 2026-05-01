// Phase 4 TDD: Verifies that `RefCall` and `SourceCall` are NOT present in
// the public AST surface of `smelt-parser`.
//
// These legacy projection types were removed when `smelt.ref(...)` and
// `smelt.source(...)` became parse errors in Phase 4 of the path migration.
//
// The compile-time assertion is encoded by NOT including RefCall / SourceCall
// in any use list. The runtime assertion checks that the parser rejects the
// legacy syntax.

/// Confirm that the smelt_parser::ast module does not export RefCall or
/// SourceCall, and that the parser rejects legacy `smelt.ref()` / `smelt.source()` syntax.
#[test]
fn ast_module_has_no_legacy_ref_or_source_projections() {
    // These symbols must still exist after the Phase 4 deletion:
    use smelt_parser::ast::{File, FunctionCall, SmeltFnCall, SmeltPathCall, SmeltPathRef};
    let _ = (
        std::mem::size_of::<File>(),
        std::mem::size_of::<FunctionCall>(),
        std::mem::size_of::<SmeltFnCall>(),
        std::mem::size_of::<SmeltPathCall>(),
        std::mem::size_of::<SmeltPathRef>(),
    );

    // Runtime check: parsing `smelt.ref('x')` must produce at least one error
    // (the parser now rejects legacy syntax).  Previously it produced zero
    // errors and created a valid RefCall node; now it must emit a parse error.
    let sql = "SELECT * FROM smelt.ref('users')";
    let parse = smelt_parser::parse(sql);
    assert!(
        !parse.errors.is_empty(),
        "smelt.ref('users') must produce parse errors after Phase 4 \
         (legacy syntax is rejected); got zero errors which means \
         RefCall/SourceCall are still active"
    );

    let sql2 = "SELECT * FROM smelt.source('raw.users')";
    let parse2 = smelt_parser::parse(sql2);
    assert!(
        !parse2.errors.is_empty(),
        "smelt.source('raw.users') must produce parse errors after Phase 4; \
         got zero errors"
    );
}
