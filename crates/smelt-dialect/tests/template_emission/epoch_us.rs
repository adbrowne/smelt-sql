//! `EPOCH_US` template row, exercised through the real `print` entry point,
//! registry row and all (docs/outcomes/20260912-databricks-dogfood-spine
//! phase 6e).

use super::{bigquery, duckdb, spark};

/// DuckDB keeps its native `epoch_us` spelling (the registry's default);
/// Spark/Databricks has no `epoch_us`, so it lowers to `unix_micros`, its own
/// microsecond-epoch equivalent.
#[test]
fn epoch_us_emits_unix_micros_on_spark() {
    assert_eq!(
        duckdb("SELECT EPOCH_US(ts) FROM t"),
        "SELECT EPOCH_US(ts) FROM t"
    );
    assert_eq!(
        spark("SELECT EPOCH_US(ts) FROM t"),
        "SELECT unix_micros(ts) FROM t"
    );
    assert_eq!(
        bigquery("SELECT EPOCH_US(ts) FROM t"),
        "SELECT UNIX_MICROS(ts) FROM t"
    );
}
