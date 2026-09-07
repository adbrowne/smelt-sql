use crate::support::*;
use crate::support_ext::*;

/// Phase B TDD: `examples/meta_hofs/` produces zero diagnostics (excluding intentional
/// `ConfigVarNullCoercion` warnings if a fixture exercises null coercion).
#[test]
fn meta_hofs_clean_workspace() {
    check_workspace_no_diagnostics("examples/meta_hofs");
}

/// Phase B TDD: `examples/meta_hofs_broken_lambda_in_forbidden_position/` produces exactly
/// one `LambdaInForbiddenPosition` diagnostic.
#[test]
fn meta_hofs_broken_lambda_in_forbidden_position() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_lambda_in_forbidden_position",
        "models/lambda_in_forbidden_position.sql",
        smelt_db::DiagnosticCode::LambdaInForbiddenPosition,
    );
}

/// Phase F TDD: `examples/meta_hofs_broken_lambda_arity_not_supported/` produces exactly
/// one `LambdaArityMismatch` diagnostic (Phase F replaces the old LambdaArityNotSupported
/// code with the arity-aware code now that multi-arg lambdas parse as proper CST nodes).
#[test]
fn meta_hofs_broken_lambda_arity_not_supported() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_lambda_arity_not_supported",
        "models/lambda_arity_not_supported.sql",
        smelt_db::DiagnosticCode::LambdaArityMismatch,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_lambda_result_type_mismatch/` produces exactly
/// one `LambdaResultTypeMismatch` diagnostic.
#[test]
fn meta_hofs_broken_lambda_result_type_mismatch() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_lambda_result_type_mismatch",
        "models/lambda_result_type_mismatch.sql",
        smelt_db::DiagnosticCode::LambdaResultTypeMismatch,
    );
}

/// D-19 TDD: `examples/meta_hofs_broken_hof_named_argument/` produces exactly
/// one `HofNamedArgument` diagnostic.
#[test]
fn meta_hofs_broken_hof_named_argument() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_hof_named_argument",
        "models/hof_named_argument.sql",
        smelt_db::DiagnosticCode::HofNamedArgument,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_hof_expects_lambda/` produces exactly
/// one `HofExpectsLambda` diagnostic.
#[test]
fn meta_hofs_broken_hof_expects_lambda() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_hof_expects_lambda",
        "models/hof_expects_lambda.sql",
        smelt_db::DiagnosticCode::HofExpectsLambda,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_hof_expects_reducer/` produces exactly
/// one `HofExpectsReducer` diagnostic.
#[test]
fn meta_hofs_broken_hof_expects_reducer() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_hof_expects_reducer",
        "models/hof_expects_reducer.sql",
        smelt_db::DiagnosticCode::HofExpectsReducer,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_hof_name_shadowed/` produces exactly
/// one `HofNameShadowed` diagnostic.
#[test]
fn meta_hofs_broken_hof_name_shadowed() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_hof_name_shadowed",
        "functions/shadowed_hof.sql",
        smelt_db::DiagnosticCode::HofNameShadowed,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_pipe_rhs_not_call/` produces exactly
/// one `PipeRhsNotCall` diagnostic.
#[test]
fn meta_hofs_broken_pipe_rhs_not_call() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_pipe_rhs_not_call",
        "models/pipe_rhs_not_call.sql",
        smelt_db::DiagnosticCode::PipeRhsNotCall,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_reducer_input_type_mismatch/` produces exactly
/// one `ReducerInputTypeMismatch` diagnostic.
#[test]
fn meta_hofs_broken_reducer_input_type_mismatch() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_reducer_input_type_mismatch",
        "models/reducer_input_type_mismatch.sql",
        smelt_db::DiagnosticCode::ReducerInputTypeMismatch,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_reducer_empty_no_identity/` produces exactly
/// one `ReducerEmptyNoIdentity` diagnostic.
#[test]
fn meta_hofs_broken_reducer_empty_no_identity() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_reducer_empty_no_identity",
        "models/reducer_empty_no_identity.sql",
        smelt_db::DiagnosticCode::ReducerEmptyNoIdentity,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_config_var_not_found/` produces exactly
/// one `ConfigVarNotFound` diagnostic.
#[test]
fn meta_hofs_broken_config_var_not_found() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_config_var_not_found",
        "models/config_var_not_found.sql",
        smelt_db::DiagnosticCode::ConfigVarNotFound,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_reducer_name_shadowed/` produces exactly
/// one `ReducerNameShadowed` diagnostic.
#[test]
fn meta_hofs_broken_reducer_name_shadowed() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_reducer_name_shadowed",
        "functions/shadowed_reducer.sql",
        smelt_db::DiagnosticCode::ReducerNameShadowed,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_pipe_in_data_position/` produces exactly
/// one `PipeInDataPosition` diagnostic.
#[test]
fn meta_hofs_broken_pipe_in_data_position() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_pipe_in_data_position",
        "models/pipe_in_data_position.sql",
        smelt_db::DiagnosticCode::PipeInDataPosition,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_config_var_name_not_literal/` produces exactly
/// one `ConfigVarNameNotLiteral` diagnostic.
#[test]
fn meta_hofs_broken_config_var_name_not_literal() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_config_var_name_not_literal",
        "models/config_var_name_not_literal.sql",
        smelt_db::DiagnosticCode::ConfigVarNameNotLiteral,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_config_var_null_coercion/` produces exactly
/// one `ConfigVarNullCoercion` diagnostic (a warning).
#[test]
fn meta_hofs_broken_config_var_null_coercion() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_config_var_null_coercion",
        "models/config_var_null_coercion.sql",
        smelt_db::DiagnosticCode::ConfigVarNullCoercion,
    );
}
