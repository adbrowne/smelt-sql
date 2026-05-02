// Phase 5b TDD: Verifies that `smelt.fn.*` call syntax is rejected by the
// parser after the SMELT_FN_CALL parser arm is removed.
//
// The runtime assertion checks that the parser rejects `smelt.fn.foo(x)`
// and produces parse errors (rather than a valid SMELT_FN_CALL node).

/// Confirm that `smelt.fn.foo(x)` form produces parse errors after Phase 5b.
#[test]
fn smelt_fn_call_not_in_public_ast() {
    // Runtime check: SELECT smelt.fn.foo(x) AS r must produce parse errors
    // after Phase 5b. Before Phase 5b, this parses without errors.
    let sql = "SELECT smelt.fn.foo(x) AS r";
    let parse = smelt_parser::parse(sql);
    assert!(
        !parse.errors.is_empty(),
        "smelt.fn.foo(x) must produce parse errors after Phase 5b (smelt.fn.* is removed); \
         got zero errors which means SMELT_FN_CALL parser arm is still active"
    );
}
