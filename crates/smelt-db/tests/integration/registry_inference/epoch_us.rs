//! Phase 6e TDD test: `EPOCH_US` registry-first inference.

use smelt_types::DataType;

use super::{ctx_with_model, infer};

#[test]
fn epoch_us_infers_bigint() {
    // `EPOCH_US` is registered `(Timestamp) -> BigInt` in the BuiltinRegistry
    // (docs/outcomes/20260912-databricks-dogfood-spine phase 6e); previously
    // it had no registry entry at all and inferred Unknown.
    let ctx = ctx_with_model(
        "upstream",
        &[(
            "event_ts",
            DataType::Timestamp {
                with_timezone: false,
            },
        )],
    );
    let sql = "SELECT EPOCH_US(event_ts) AS r FROM upstream";
    let types = infer(sql, &ctx);
    assert_eq!(types.len(), 1);
    assert_eq!(
        types[0].data_type,
        DataType::BigInt,
        "EPOCH_US(TIMESTAMP) must infer BIGINT, got {:?}",
        types[0].data_type,
    );
}
