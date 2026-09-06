
use super::*;

/// Every multi-model diagnostic code exists in `DiagnosticCode` and its
/// rendered message matches the spec table verbatim.
#[test]
fn diagnostic_codes_multi_model_set_complete() {
    // GeneratesUnknownValue
    let msg = meta_multi_model_diagnostic_message(
        DiagnosticCode::GeneratesUnknownValue,
        Some("views"),
        None,
        None,
        None,
        None,
    );
    assert_eq!(msg, "generates must be `models`; found views");

    // GeneratesMixedWithBareModel
    let msg = meta_multi_model_diagnostic_message(
        DiagnosticCode::GeneratesMixedWithBareModel,
        None,
        None,
        None,
        None,
        None,
    );
    assert_eq!(
            msg,
            "generates: models cannot coexist with bare-model identity (name field or section delimiter)"
        );

    // GenerateFileBareSelectForbidden
    let msg = meta_multi_model_diagnostic_message(
        DiagnosticCode::GenerateFileBareSelectForbidden,
        None,
        None,
        None,
        None,
        None,
    );
    assert_eq!(
            msg,
            "generator file body must produce List<ModelDef>; bare SELECT is the hand-authored model shape"
        );

    // GenerateFileBodyTypeError
    let msg = meta_multi_model_diagnostic_message(
        DiagnosticCode::GenerateFileBodyTypeError,
        None,
        Some("List<Text>"),
        None,
        None,
        None,
    );
    assert_eq!(
        msg,
        "generator file body must evaluate to List<ModelDef>; found List<Text>"
    );

    // ModelDefOutsideGeneratorFile
    let msg = meta_multi_model_diagnostic_message(
        DiagnosticCode::ModelDefOutsideGeneratorFile,
        None,
        None,
        None,
        None,
        None,
    );
    assert_eq!(
        msg,
        "ModelDef literals are only valid inside a `generates: models` file body"
    );

    // ModelDefInvalidName
    let msg = meta_multi_model_diagnostic_message(
        DiagnosticCode::ModelDefInvalidName,
        Some("bad-name!"),
        None,
        None,
        None,
        None,
    );
    assert_eq!(
        msg,
        "ModelDef.name must be a non-empty Text of [A-Za-z0-9_]+; found bad-name!"
    );

    // ModelDefInvalidMaterialization
    let msg = meta_multi_model_diagnostic_message(
        DiagnosticCode::ModelDefInvalidMaterialization,
        Some("external"),
        None,
        None,
        None,
        None,
    );
    assert_eq!(
        msg,
        "ModelDef.materialization must be one of view, table, incremental; found external"
    );

    // ModelDefDuplicateName
    let msg = meta_multi_model_diagnostic_message(
        DiagnosticCode::ModelDefDuplicateName,
        None,
        None,
        Some("my_model"),
        None,
        None,
    );
    assert_eq!(
        msg,
        "duplicate ModelDef.name `my_model` in this generator file"
    );

    // ModelDefHandAuthoredCollision
    let msg = meta_multi_model_diagnostic_message(
        DiagnosticCode::ModelDefHandAuthoredCollision,
        None,
        None,
        None,
        Some("models/revenue.sql"),
        Some("models/revenue.sql"),
    );
    assert_eq!(
        msg,
        "ModelDef emits `models/revenue.sql` which collides with models/revenue.sql"
    );

    // GeneratorBodyForbidsModelReflection
    let msg = meta_multi_model_diagnostic_message(
        DiagnosticCode::GeneratorBodyForbidsModelReflection,
        None,
        None,
        None,
        None,
        None,
    );
    assert_eq!(
            msg,
            "smelt.models.* is not available inside a generator body; use smelt.sources.* or literal smelt.<path> references"
        );
}
