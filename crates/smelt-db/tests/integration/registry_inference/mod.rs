//! Phase 9 TDD tests: registry-first inference of SQL built-ins.
//!
//! Each test here pins a specific contract of the registry-backed inference
//! path in `smelt_db::type_inference::infer_function_type`, irrespective of
//! whether the entry has been migrated yet or still lives in the legacy
//! hand-written match. The assertion is on the resulting [`TypedColumn`] —
//! property that must hold under either path.

use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_parser::ast::File;
use smelt_types::{DataType, TypedColumn};

/// Parse a SELECT statement and infer column types using the given context.
fn infer(sql: &str, ctx: &TypeContext) -> Vec<TypedColumn> {
    let parse = smelt_parser::parse(sql);
    let file = File::cast(parse.syntax()).expect("parse File");
    let select = file.select_stmt().expect("parse SELECT");
    infer_select_column_types(&select, ctx)
}

/// Build a TypeContext with one upstream model that has the given columns.
fn ctx_with_model(model: &str, cols: &[(&str, DataType)]) -> TypeContext {
    let mut ctx = TypeContext::new();
    for (name, dt) in cols {
        ctx.add_model_column(
            model,
            name,
            TypedColumn {
                data_type: dt.clone(),
                nullable: true,
            },
        );
    }
    ctx
}

#[test]
fn unrecognized_function_uses_existing_code() {
    // Phase 9 TDD test #3. `MY_UNKNOWN_FUNC(x)` is not in `SqlFunction::from_name`
    // nor in the `BuiltinRegistry`. `infer_function_type` must return `None`
    // (the existing hand-written fallback — the lib-level
    // `check_expression_types` turns that `None` into an
    // `UnrecognizedFunction` warning).
    let ctx = ctx_with_model("upstream", &[("x", DataType::Integer)]);
    let sql = "SELECT MY_UNKNOWN_FUNC(x) AS r FROM upstream";
    let types = infer(sql, &ctx);
    assert_eq!(types.len(), 1);
    // Unknown function → Unknown result type, preserving legacy behaviour.
    assert_eq!(types[0].data_type, DataType::unknown_dynamic());
}

#[test]
fn sum_of_decimal_returns_decimal() {
    // Phase 9 TDD test #4. Per §16 #9: `SUM(DECIMAL(p, s))` widens to
    // `DECIMAL(38, s)`. This was the canonical SUM-widening case called out
    // in the phase brief. Since the legacy hand-written SUM implements this
    // rule and the registry's vanilla `<T: Numeric>(T) → T` signature does
    // NOT, we must either (a) keep SUM in the legacy fallback or (b) extend
    // the registry with widening semantics. Either way the assertion on the
    // final `TypedColumn` must hold.
    let ctx = ctx_with_model(
        "upstream",
        &[(
            "x",
            DataType::Decimal {
                precision: 10,
                scale: 2,
            },
        )],
    );
    let sql = "SELECT SUM(x) AS total FROM upstream";
    let types = infer(sql, &ctx);
    assert_eq!(types.len(), 1);
    assert_eq!(
        types[0].data_type,
        DataType::Decimal {
            precision: 38,
            scale: 2,
        },
        "SUM(DECIMAL(10,2)) must widen to DECIMAL(38,2), got {:?}",
        types[0].data_type,
    );
}

#[test]
fn min_of_timestamp_returns_timestamp() {
    // Phase 9 TDD test #5. `MIN<T: Ordered>(T) → T` with Timestamp input
    // must preserve Timestamp in the return type — exercises the registry's
    // generic type-preservation path AND the legacy path's `first_arg_type_or`
    // branch. Either way the resulting `TypedColumn` must carry Timestamp.
    let ctx = ctx_with_model(
        "upstream",
        &[(
            "ts",
            DataType::Timestamp {
                with_timezone: false,
            },
        )],
    );
    let sql = "SELECT MIN(ts) AS first_ts FROM upstream";
    let types = infer(sql, &ctx);
    assert_eq!(types.len(), 1);
    assert_eq!(
        types[0].data_type,
        DataType::Timestamp {
            with_timezone: false,
        },
        "MIN(TIMESTAMP) must preserve TIMESTAMP, got {:?}",
        types[0].data_type,
    );
    assert!(types[0].nullable, "MIN(...) result is always nullable");
}

#[test]
fn date_add_infers_timestamp() {
    // Phase 9: `DATE_ADD` is an ordinary callable (not `SyntaxForm::Special`)
    // typed `(Date, Interval) -> Timestamp` by the registry.
    let ctx = ctx_with_model("upstream", &[("d", DataType::Date)]);
    let sql = "SELECT DATE_ADD(d, INTERVAL 1 DAY) AS r FROM upstream";
    let types = infer(sql, &ctx);
    assert_eq!(types.len(), 1);
    assert_eq!(
        types[0].data_type,
        DataType::Timestamp {
            with_timezone: false,
        },
        "DATE_ADD(DATE, INTERVAL) must infer TIMESTAMP, got {:?}",
        types[0].data_type,
    );
}

#[test]
fn date_sub_infers_timestamp() {
    // Phase 9: same contract as `date_add_infers_timestamp` for `DATE_SUB`.
    let ctx = ctx_with_model("upstream", &[("d", DataType::Date)]);
    let sql = "SELECT DATE_SUB(d, INTERVAL 1 DAY) AS r FROM upstream";
    let types = infer(sql, &ctx);
    assert_eq!(types.len(), 1);
    assert_eq!(
        types[0].data_type,
        DataType::Timestamp {
            with_timezone: false,
        },
        "DATE_SUB(DATE, INTERVAL) must infer TIMESTAMP, got {:?}",
        types[0].data_type,
    );
}
