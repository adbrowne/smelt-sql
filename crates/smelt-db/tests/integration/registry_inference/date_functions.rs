//! Phase 9 TDD tests: `DATE_ADD`/`DATE_SUB` registry-first inference.

use smelt_types::DataType;

use super::{ctx_with_model, infer};

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
