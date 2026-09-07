use crate::support::*;
use crate::support_ext::*;

/// Phase A TDD: `examples/meta_lists/` produces zero diagnostics.
/// This is the acceptance gate for the four happy-path models.
#[test]
fn meta_lists_clean_workspace() {
    check_workspace_no_diagnostics("examples/meta_lists");
}

/// Phase A TDD: `examples/meta_lists_broken_heterogeneous/` produces exactly
/// one `MetaListHeterogeneous` diagnostic anchored at `models/heterogeneous.sql`.
#[test]
fn meta_lists_broken_heterogeneous() {
    check_workspace_emits_exactly_one_diagnostic(
        "examples/meta_lists_broken_heterogeneous",
        "models/heterogeneous.sql",
        smelt_db::DiagnosticCode::MetaListHeterogeneous,
    );
}

/// Phase A TDD: `examples/meta_lists_broken_empty_unknown/` produces exactly
/// one `MetaListEmptyTypeUnknown` diagnostic anchored at `models/empty_unknown.sql`.
#[test]
fn meta_lists_broken_empty_unknown() {
    check_workspace_emits_exactly_one_diagnostic(
        "examples/meta_lists_broken_empty_unknown",
        "models/empty_unknown.sql",
        smelt_db::DiagnosticCode::MetaListEmptyTypeUnknown,
    );
}

/// P6 TDD: `examples/meta_lists_broken_list_in_scalar_position/` produces
/// exactly one `MetaListInScalarPosition` diagnostic anchored at
/// `models/list_in_scalar.sql` — a bare meta `List<T>` in a Data-World scalar
/// position, in a FROM-less model (the select-shape check must still run).
#[test]
fn meta_lists_broken_list_in_scalar_position() {
    check_workspace_emits_exactly_one_diagnostic(
        "examples/meta_lists_broken_list_in_scalar_position",
        "models/list_in_scalar.sql",
        smelt_db::DiagnosticCode::MetaListInScalarPosition,
    );
}

/// Phase A TDD: `examples/meta_lists_broken_spread_forbidden/` produces exactly
/// one `MetaSpreadInForbiddenPosition` diagnostic anchored at
/// `models/spread_forbidden.sql`.  A `ParseError` from the parser's error
/// recovery on the `...` token is allowed alongside the Phase A code (the helper
/// only counts Phase A codes).
#[test]
fn meta_lists_broken_spread_forbidden() {
    check_workspace_emits_exactly_one_diagnostic(
        "examples/meta_lists_broken_spread_forbidden",
        "models/spread_forbidden.sql",
        smelt_db::DiagnosticCode::MetaSpreadInForbiddenPosition,
    );
}
