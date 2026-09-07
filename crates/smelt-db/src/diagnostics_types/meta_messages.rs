use super::DiagnosticCode;

/// Render the diagnostic message for Phase A (meta-language) list and spread
/// diagnostic codes.
///
/// Parameters:
/// - `code`: one of the four Phase A `DiagnosticCode` variants.
/// - `first_type`: the first element's rendered type (for `MetaListHeterogeneous`).
/// - `other_type`: the incompatible/actual type (for `MetaListHeterogeneous` and
///   `MetaSpreadOnNonList`).
/// - `position_name`: the human-readable position name (for
///   `MetaSpreadInForbiddenPosition`), e.g. `"WHERE clause"`.
///
/// Returns the exact message string specified in `meta_language.md` §"Diagnostic
/// codes".
pub fn meta_list_diagnostic_message(
    code: DiagnosticCode,
    first_type: Option<&str>,
    other_type: Option<&str>,
    position_name: Option<&str>,
) -> String {
    match code {
        DiagnosticCode::MetaListEmptyTypeUnknown => {
            "cannot infer element type for empty list literal".to_string()
        }
        DiagnosticCode::MetaListHeterogeneous => {
            let t0 = first_type.unwrap_or("?");
            let tk = other_type.unwrap_or("?");
            format!("list elements have incompatible types: {}, {}", t0, tk)
        }
        DiagnosticCode::MetaSpreadInForbiddenPosition => {
            let pos = position_name.unwrap_or("unknown position");
            format!("spread is not allowed in {}", pos)
        }
        DiagnosticCode::MetaSpreadOnNonList => {
            let actual = other_type.unwrap_or("?");
            format!("spread expects List<T>; found {}", actual)
        }
        // invariant: unreachable from user input — caller is dispatched only for meta-list diagnostic codes
        _ => panic!("meta_list_diagnostic_message called with non-Phase-A code"),
    }
}

/// Render the diagnostic message for Phase B / Phase F (meta-language) HOF, lambda,
/// pipe, reducer, ternary, and `smelt.config.var` diagnostic codes.
///
/// Parameters:
/// - `code`: one of the Phase B/F `DiagnosticCode` variants.
/// - `hof`: HOF name for `LambdaResultTypeMismatch`, `HofExpectsLambda`,
///   `LambdaArityMismatch` (e.g. `"map"`).
/// - `name`: function/reducer/variable/keyword name for `HofNameShadowed`,
///   `ReducerNameShadowed`, `ConfigVarNotFound`, `ConfigVarNullCoercion`,
///   `TernaryKeywordShadowed`, `LambdaDuplicateParameter`.
/// - `expected`: expected type or arity string.
/// - `actual`: actual type or arity string.
/// - `reducer`: reducer name for `ReducerInputTypeMismatch`, `ReducerEmptyNoIdentity`,
///   `ReducerArityMismatch`, `ReducerArgTypeMismatch`, `ReducerArgNotCompileTime`,
///   `ReducerNamedArgument`.
/// - `t_in`: expected input element type string for `ReducerInputTypeMismatch`; or
///   parameter name for `ReducerArgTypeMismatch`, `ReducerArgNotCompileTime`.
/// - `t_actual`: actual input element type string for `ReducerInputTypeMismatch`,
///   or actual type for `ReducerArgTypeMismatch`, `ReducerArgNotCompileTime`.
///
/// Returns the exact message string specified in `meta_language.md` §"Diagnostic codes".
#[allow(clippy::too_many_arguments)]
pub fn meta_hof_diagnostic_message(
    code: DiagnosticCode,
    hof: Option<&str>,
    name: Option<&str>,
    expected: Option<&str>,
    actual: Option<&str>,
    reducer: Option<&str>,
    t_in: Option<&str>,
    t_actual: Option<&str>,
) -> String {
    match code {
        DiagnosticCode::LambdaInForbiddenPosition => {
            "lambda is only valid as an argument to a higher-order function".to_string()
        }
        DiagnosticCode::LambdaArityMismatch => {
            let h = hof.unwrap_or("HOF");
            let exp = expected.unwrap_or("?");
            let act = actual.unwrap_or("?");
            format!(
                "{} expects a lambda of arity {}; found arity {}",
                h, exp, act
            )
        }
        DiagnosticCode::LambdaZeroParameters => {
            "lambda must declare at least one parameter".to_string()
        }
        DiagnosticCode::LambdaDuplicateParameter => {
            let n = name.unwrap_or("?");
            format!(
                "parameter `{}` already appears in this lambda's parameter list",
                n
            )
        }
        DiagnosticCode::LambdaResultTypeMismatch => {
            let h = hof.unwrap_or("HOF");
            let exp = expected.unwrap_or("?");
            let act = actual.unwrap_or("?");
            format!("{} requires lambda result {}; found {}", h, exp, act)
        }
        DiagnosticCode::HofExpectsLambda => {
            let h = hof.unwrap_or("HOF");
            let act = actual.unwrap_or("?");
            format!("{} expects a lambda; found {}", h, act)
        }
        DiagnosticCode::HofExpectsReducer => {
            let act = actual.unwrap_or("?");
            format!("reduce expects a reducer; found {}", act)
        }
        DiagnosticCode::HofNameShadowed => {
            let n = name.unwrap_or("?");
            format!("{} is a reserved higher-order function name", n)
        }
        DiagnosticCode::ReducerNameShadowed => {
            let n = name.unwrap_or("?");
            format!("{} is a reserved reducer name", n)
        }
        DiagnosticCode::PipeRhsNotCall => {
            "pipe right-hand side must be a function call".to_string()
        }
        DiagnosticCode::PipeInDataPosition => {
            "|> is meta-only; use SQL composition in this position".to_string()
        }
        DiagnosticCode::ReducerInputTypeMismatch => {
            let r = reducer.unwrap_or("?");
            let ti = t_in.unwrap_or("?");
            let ta = t_actual.unwrap_or("?");
            format!("reducer {} expects List<{}>; found List<{}>", r, ti, ta)
        }
        DiagnosticCode::ReducerEmptyNoIdentity => {
            let r = reducer.unwrap_or("?");
            format!("reducer {} has no identity for an empty list", r)
        }
        DiagnosticCode::ReducerArityMismatch => {
            let r = reducer.unwrap_or("?");
            let exp = expected.unwrap_or("?");
            let act = actual.unwrap_or("?");
            format!("reducer {} expects {} argument(s); found {}", r, exp, act)
        }
        DiagnosticCode::ReducerArgTypeMismatch => {
            let r = reducer.unwrap_or("?");
            let param = t_in.unwrap_or("?");
            let exp = expected.unwrap_or("?");
            let act = t_actual.unwrap_or("?");
            format!(
                "reducer {}'s argument `{}` expects {}; found {}",
                r, param, exp, act
            )
        }
        DiagnosticCode::ReducerArgNotCompileTime => {
            let r = reducer.unwrap_or("?");
            let param = t_in.unwrap_or("?");
            let act = t_actual.unwrap_or("?");
            format!(
                "reducer {}'s argument `{}` must be a compile-time value; found {}",
                r, param, act
            )
        }
        DiagnosticCode::ReducerNamedArgument => {
            let r = reducer.unwrap_or("?");
            format!("reducer {} takes positional arguments only", r)
        }
        DiagnosticCode::HofNamedArgument => {
            let h = hof.unwrap_or("HOF");
            format!(
                "{} takes positional arguments only; named arguments are not supported",
                h
            )
        }
        DiagnosticCode::TernaryConditionNotBoolean => {
            let act = actual.unwrap_or("?");
            format!("ternary condition expects Boolean; found {}", act)
        }
        DiagnosticCode::TernaryBranchTypeMismatch => {
            // t_in = then_type, t_actual = else_type
            let then_ty = t_in.unwrap_or("?");
            let else_ty = t_actual.unwrap_or("?");
            format!(
                "ternary branches have incompatible types: {} vs {}",
                then_ty, else_ty
            )
        }
        DiagnosticCode::TernaryKeywordShadowed => {
            let n = name.unwrap_or("?");
            format!("{} is a reserved meta-language keyword", n)
        }
        DiagnosticCode::TernaryInDataPosition => {
            "if-then-else is meta-only; use SQL CASE WHEN in this position".to_string()
        }
        DiagnosticCode::TernaryDanglingThen => {
            "unexpected `then` keyword outside of `if ... then ...` form".to_string()
        }
        DiagnosticCode::TernaryDanglingElse => {
            "unexpected `else` keyword outside of `... then ... else` form".to_string()
        }
        DiagnosticCode::ConfigVarNotFound => {
            let n = name.unwrap_or("?");
            format!("compile-time variable {} not declared in smelt.yml vars", n)
        }
        DiagnosticCode::ConfigVarNameNotLiteral => {
            "smelt.config.var name must be a string literal".to_string()
        }
        DiagnosticCode::ConfigVarNullCoercion => {
            let n = name.unwrap_or("?");
            format!(
                "null variable {} coerced to empty string; declare a default in smelt.yml",
                n
            )
        }
        // invariant: unreachable from user input — caller is dispatched only for meta-HOF diagnostic codes
        _ => panic!("meta_hof_diagnostic_message called with non-Phase-B/F code"),
    }
}

/// Render the diagnostic message for Phase C (meta-language) reflection diagnostic codes.
///
/// Parameters:
/// - `code`: one of the four Phase C `DiagnosticCode` variants.
/// - `actual`: the actual synthesised type (for `ColumnsOfRequiresTableExpr`).
/// - `field_name`: the unknown field name (for `ColumnRefFieldUnknown`).
/// - `table_expr`: the text of the table expression (for `ColumnsOfUnresolvableSchema`).
///
/// Returns the exact message string specified in `meta_language.md` §"Diagnostic
/// codes (new in Phase C)".
pub fn meta_reflection_diagnostic_message(
    code: DiagnosticCode,
    actual: Option<&str>,
    field_name: Option<&str>,
) -> String {
    meta_reflection_diagnostic_message_with_table_expr(code, actual, field_name, None)
}

/// Extended form of [`meta_reflection_diagnostic_message`] that also accepts a
/// `table_expr` string for the `ColumnsOfUnresolvableSchema` variant.
pub fn meta_reflection_diagnostic_message_with_table_expr(
    code: DiagnosticCode,
    actual: Option<&str>,
    field_name: Option<&str>,
    table_expr: Option<&str>,
) -> String {
    match code {
        DiagnosticCode::ColumnsOfRequiresTableExpr => {
            let act = actual.unwrap_or("?");
            format!("smelt.columns_of expects TableExpr; found {act}")
        }
        DiagnosticCode::ColumnsOfNamedArgument => {
            "smelt.columns_of takes one positional argument; named arguments are not supported"
                .to_string()
        }
        DiagnosticCode::ColumnRefFieldUnknown => {
            let name = field_name.unwrap_or("?");
            format!(
                "ColumnRef has no field {name}; expected one of: \
                 name, type, is_numeric, is_decimal, is_string, is_temporal, is_integer, is_boolean"
            )
        }
        DiagnosticCode::ColumnsOfUnresolvableSchema => {
            let t = table_expr.unwrap_or("t");
            format!("cannot resolve column list for {t}; upstream schema is unknown")
        }
        // Phase D diagnostic messages
        DiagnosticCode::WithTagRequiresText => {
            let act = actual.unwrap_or("?");
            format!("with_tag expects a compile-time Text; found {act}")
        }
        DiagnosticCode::WithTagNamedArgument => {
            "with_tag takes one positional argument; named arguments are not supported".to_string()
        }
        DiagnosticCode::WideReflectionUnknownAccessor => {
            // `actual` carries the namespace ("models" or "sources"),
            // `field_name` carries the unknown accessor name.
            let ns = actual.unwrap_or("models");
            let name = field_name.unwrap_or("?");
            format!("smelt.{ns} has no accessor `{name}`; expected one of: with_tag, all")
        }
        DiagnosticCode::WideReflectionUnexpectedArgument => {
            // `actual` carries the full accessor name ("smelt.models.all", etc.).
            let accessor = actual.unwrap_or("all");
            format!("{accessor} takes no arguments")
        }
        DiagnosticCode::ModelRefFieldUnknown => {
            let name = field_name.unwrap_or("?");
            format!("ModelRef has no field `{name}`; expected one of: path, name, tags, columns")
        }
        DiagnosticCode::SourceRefFieldUnknown => {
            let name = field_name.unwrap_or("?");
            format!("SourceRef has no field `{name}`; expected one of: path, name, tags, columns")
        }
        // invariant: unreachable from user input — caller is dispatched only for meta-reflection diagnostic codes
        _ => panic!(
            "meta_reflection_diagnostic_message called with non-Phase-C/D code: {:?}",
            code
        ),
    }
}

/// Render the diagnostic message for Phase E1 (meta-language) record diagnostic codes.
///
/// Parameters vary by code — see each variant's doc comment for the placeholders.
/// All `Option<&str>` parameters default to `"?"` when `None`.
///
/// Returns the exact message string specified in `meta_language.md`
/// §"Record diagnostic codes".
#[allow(clippy::too_many_arguments)]
pub fn meta_record_diagnostic_message(
    code: DiagnosticCode,
    type_name: Option<&str>,
    field_name: Option<&str>,
    path: Option<&str>,
    expected: Option<&str>,
    actual: Option<&str>,
    fields: Option<&str>,
) -> String {
    let ty = type_name.unwrap_or("?");
    let name = field_name.unwrap_or("?");
    match code {
        DiagnosticCode::SmeltRecordRedefinition => {
            let p = path.unwrap_or("?");
            format!(
                "record `{ty}` is already declared in {p}; record names must be unique workspace-wide"
            )
        }
        DiagnosticCode::RecordFieldUnknown => {
            let fs = fields.unwrap_or("?");
            format!("record `{ty}` has no field `{name}`; expected one of: {fs}")
        }
        DiagnosticCode::RecordFieldMissing => {
            format!("record literal for `{ty}` is missing required field `{name}`")
        }
        DiagnosticCode::RecordFieldDuplicate => {
            format!("field `{name}` already appears in this record literal")
        }
        DiagnosticCode::RecordFieldTypeMismatch => {
            let exp = expected.unwrap_or("?");
            let act = actual.unwrap_or("?");
            format!("record field `{name}` expects {exp}; found {act}")
        }
        DiagnosticCode::RecordLiteralUnknownTarget => {
            "cannot infer record type from context; annotate the target type".to_string()
        }
        DiagnosticCode::RecordFieldNotProjectable => {
            format!("value of type {ty} has no fields; projection `{name}` is not valid")
        }
        DiagnosticCode::RecordFieldTypeForbidden => {
            format!(
                "record field types may not reference {ty}; reflection witnesses are not user-writable"
            )
        }
        DiagnosticCode::RecordCyclicDeclaration => {
            format!(
                "record `{ty}` forms a cycle; recursive record declarations are not supported in v1"
            )
        }
        DiagnosticCode::RecordInDataWorld => {
            "record-typed value may not appear in a Data-World (SQL) position; use field projection to produce a spliced value".to_string()
        }
        // invariant: unreachable from user input — caller is dispatched only for meta-record diagnostic codes
        _ => panic!(
            "meta_record_diagnostic_message called with non-record code: {:?}",
            code
        ),
    }
}

/// Render the diagnostic message for Phase E1 (meta-language) map diagnostic codes.
///
/// Parameters vary by code:
/// - `method`: the Map API method name (for arity/named-arg/unexpected-arg codes).
/// - `name`: the unknown method name (for `MapApiUnknown`).
/// - `key`: the missing key (for `MapGetMissingKey`).
/// - `n`: the actual argument count as a string (for `MapApiArityMismatch`).
/// - `expected`: expected key type (for `MapApiArgTypeMismatch`).
/// - `actual`: actual type/found value (various).
///
/// Returns the exact message string specified in `meta_language.md`
/// §"Map diagnostic codes".
#[allow(clippy::too_many_arguments)]
pub fn meta_map_diagnostic_message(
    code: DiagnosticCode,
    method: Option<&str>,
    name: Option<&str>,
    key: Option<&str>,
    n: Option<&str>,
    type_name: Option<&str>,
    expected: Option<&str>,
    actual: Option<&str>,
) -> String {
    let m = method.unwrap_or("?");
    match code {
        DiagnosticCode::MapKeyTypeNotText => {
            let ty = type_name.unwrap_or("?");
            format!("Map key type must be Text in v1; found {ty}")
        }
        DiagnosticCode::MapApiUnknown => {
            let n = name.unwrap_or("?");
            format!("Map has no method `{n}`; expected one of: entries, keys, values, get, has")
        }
        DiagnosticCode::MapApiArityMismatch => {
            let count = n.unwrap_or("?");
            format!("Map.{m} expects one positional argument; found {count}")
        }
        DiagnosticCode::MapApiNamedArgument => {
            format!("Map.{m} does not support named arguments")
        }
        DiagnosticCode::MapApiUnexpectedArgument => {
            format!("Map.{m} takes no arguments")
        }
        DiagnosticCode::MapGetMissingKey => {
            let k = key.unwrap_or("?");
            format!("Map has no binding for key `{k}`")
        }
        DiagnosticCode::MapApiArgTypeMismatch => {
            let exp = expected.unwrap_or("?");
            let act = actual.unwrap_or("?");
            format!("Map.{m} expects key of type {exp}; found {act}")
        }
        // invariant: unreachable from user input — caller is dispatched only for meta-map diagnostic codes
        _ => panic!(
            "meta_map_diagnostic_message called with non-map code: {:?}",
            code
        ),
    }
}

/// Render the diagnostic message for Phase E1 (meta-language) loader diagnostic codes.
///
/// Parameters vary by code:
/// - `expr`: the expression text (for `ConfigLoaderPathNotLiteral`).
/// - `path`: the path text (for path-related codes).
/// - `format`: the file format name (for `ConfigLoaderParseError`).
/// - `parser_error`: the parser error string (for `ConfigLoaderParseError`).
/// - `name`: field name (for field-related codes).
/// - `fields`: comma-separated list of valid fields (for `ConfigLoaderUnknownField`).
/// - `expected_type`, `actual_type`: type strings (for `ConfigLoaderTypeMismatch`,
///   `ConfigLoaderRootShapeMismatch`).
/// - `expected_shape`, `actual_shape`: shape strings (for `ConfigLoaderRootShapeMismatch`).
/// - `key`: the duplicate key (for `ConfigLoaderDuplicateMapKey`).
/// - `row`, `first_row`: row references (for `ConfigLoaderDuplicateMapKey`,
///   `ConfigLoaderNullCoercion`).
///
/// Returns the exact message string specified in `meta_config_loading.md`
/// §"Validation diagnostics".
#[allow(clippy::too_many_arguments)]
pub fn meta_loader_diagnostic_message(
    code: DiagnosticCode,
    expr: Option<&str>,
    path: Option<&str>,
    format: Option<&str>,
    parser_error: Option<&str>,
    name: Option<&str>,
    fields: Option<&str>,
    expected_type: Option<&str>,
    actual_type: Option<&str>,
    expected_shape: Option<&str>,
    actual_shape: Option<&str>,
    key: Option<&str>,
    row: Option<&str>,
    first_row: Option<&str>,
) -> String {
    match code {
        DiagnosticCode::ConfigLoaderPathNotLiteral => {
            let e = expr.unwrap_or("?");
            format!("loader path must be a string literal; found {e}")
        }
        DiagnosticCode::ConfigLoaderPathEscapesWorkspace => {
            let p = path.unwrap_or("?");
            format!("loader path must be a workspace-relative path; found {p}")
        }
        DiagnosticCode::ConfigLoaderPathBackslash => {
            let p = path.unwrap_or("?");
            format!(r"loader paths use `/` as the path separator; found `\` in {p}")
        }
        DiagnosticCode::ConfigLoaderFileNotFound => {
            let p = path.unwrap_or("?");
            format!("loader file `{p}` not found in workspace")
        }
        DiagnosticCode::ConfigLoaderSchemaForbidden => {
            let act = actual_type.unwrap_or("?");
            format!("loader schema must be a record type, `List<record>`, or `Map<Text, record>`; found {act}")
        }
        DiagnosticCode::ConfigLoaderTomlNotYetSupported => {
            "smelt.config.load_toml is reserved; only YAML and JSON loaders are supported in v1"
                .to_string()
        }
        DiagnosticCode::ConfigLoaderParseError => {
            let fmt = format.unwrap_or("?");
            let p = path.unwrap_or("?");
            let err = parser_error.unwrap_or("?");
            format!("failed to parse {fmt} file `{p}`: {err}")
        }
        DiagnosticCode::ConfigLoaderRequiredFieldMissing => {
            let n = name.unwrap_or("?");
            format!("field `{n}` required by schema is missing")
        }
        DiagnosticCode::ConfigLoaderUnknownField => {
            let n = name.unwrap_or("?");
            let fs = fields.unwrap_or("?");
            format!("field `{n}` is not declared in the schema; expected one of: {fs}")
        }
        DiagnosticCode::ConfigLoaderTypeMismatch => {
            let n = name.unwrap_or("?");
            let exp = expected_type.unwrap_or("?");
            let act = actual_type.unwrap_or("?");
            format!("field `{n}` expects {exp}; got {act}")
        }
        DiagnosticCode::ConfigLoaderRootShapeMismatch => {
            let ty = expected_type.unwrap_or("?");
            let exp = expected_shape.unwrap_or("?");
            let act = actual_shape.unwrap_or("?");
            format!("schema `{ty}` expects {exp}; file's top level is {act}")
        }
        DiagnosticCode::ConfigLoaderDuplicateMapKey => {
            let k = key.unwrap_or("?");
            let r = row.unwrap_or("?");
            let fr = first_row.unwrap_or("?");
            format!("duplicate map key `{k}` at {r}; earlier appearance at {fr}")
        }
        DiagnosticCode::ConfigLoaderNullCoercion => {
            let r = row.unwrap_or("?");
            format!(
                "null value at {r} coerced to empty string; declare a default in the source file"
            )
        }
        // invariant: unreachable from user input — caller is dispatched only for meta-loader diagnostic codes
        _ => panic!(
            "meta_loader_diagnostic_message called with non-loader code: {:?}",
            code
        ),
    }
}

/// Render the diagnostic message for multi-model production diagnostic codes.
///
/// Parameters vary by code — see the spec table in `meta_language.md`
/// §"Multi-model production diagnostic codes" for the full message formats.
///
/// All `Option<&str>` parameters default to `"?"` when `None`.
pub fn meta_multi_model_diagnostic_message(
    code: DiagnosticCode,
    value: Option<&str>,
    actual: Option<&str>,
    name: Option<&str>,
    smelt_path: Option<&str>,
    other_path: Option<&str>,
) -> String {
    match code {
        DiagnosticCode::GeneratesUnknownValue => {
            let v = value.unwrap_or("?");
            format!("generates must be `models`; found {v}")
        }
        DiagnosticCode::GeneratesMixedWithBareModel => {
            "generates: models cannot coexist with bare-model identity (name field or section delimiter)".to_string()
        }
        DiagnosticCode::GenerateFileBareSelectForbidden => {
            "generator file body must produce List<ModelDef>; bare SELECT is the hand-authored model shape".to_string()
        }
        DiagnosticCode::GenerateFileBodyTypeError => {
            let act = actual.unwrap_or("?");
            format!("generator file body must evaluate to List<ModelDef>; found {act}")
        }
        DiagnosticCode::ModelDefOutsideGeneratorFile => {
            "ModelDef literals are only valid inside a `generates: models` file body".to_string()
        }
        DiagnosticCode::ModelDefInvalidName => {
            let v = value.unwrap_or("?");
            format!("ModelDef.name must be a non-empty Text of [A-Za-z0-9_]+; found {v}")
        }
        DiagnosticCode::ModelDefInvalidMaterialization => {
            let v = value.unwrap_or("?");
            format!("ModelDef.materialization must be one of view, table, incremental; found {v}")
        }
        DiagnosticCode::ModelDefDuplicateName => {
            let n = name.unwrap_or("?");
            format!("duplicate ModelDef.name `{n}` in this generator file")
        }
        DiagnosticCode::ModelDefHandAuthoredCollision => {
            let sp = smelt_path.unwrap_or("?");
            let op = other_path.unwrap_or("?");
            format!("ModelDef emits `{sp}` which collides with {op}")
        }
        DiagnosticCode::GeneratorBodyForbidsModelReflection => {
            "smelt.models.* is not available inside a generator body; use smelt.sources.* or literal smelt.<path> references".to_string()
        }
        DiagnosticCode::ModelDefOverrideRequiresIncremental => {
            let n = name.unwrap_or("?");
            format!("ModelDef.{n} is only valid when materialization is 'incremental'")
        }
        // invariant: unreachable from user input — caller is dispatched only for meta-multi_model diagnostic codes
        _ => panic!(
            "meta_multi_model_diagnostic_message called with non-E2 code: {:?}",
            code
        ),
    }
}
