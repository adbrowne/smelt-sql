//! Gap 4 (`docs/outcomes/20260913-trino-incremental/phases/03f-plan.md`):
//! the windowed-keyed driver's per-step driving-source pushdown filter
//! renders its partition literal through the single [`partition_literal`]
//! owner, typed against the driving source's own declared column, instead
//! of the pre-fix hardcoded [`PartitionColumnType::Undeclared`].
//!
//! `inject_source_filters` is the mechanism `cumulative.rs`'s per-step
//! `compile_step` closure calls with the driving source's resolved
//! [`SourceBound::column_type`] (`crate::execute::sources::
//! source_partition_column_type`, wired in this same phase) — these tests
//! exercise that mechanism directly against the three column-type shapes,
//! matching `docs/specs/incremental_shapes.md` §"The partition grain" rule
//! 8a's own byte-for-byte-unchanged guarantee for `Text`/`Undeclared`.

use smelt_logical::maintenance::emit::PartitionColumnType;
use smelt_logical::PartitionAxis;
use smelt_runtime::{inject_source_filters, SourceBound, TimeRange};
use std::collections::HashMap;

fn bounds(column_type: PartitionColumnType) -> HashMap<String, SourceBound> {
    let mut map = HashMap::new();
    map.insert(
        "smelt.silver.events_parsed".to_string(),
        SourceBound {
            partition_col: "event_date".to_string(),
            before_secs: 0,
            after_secs: 0,
            column_type,
        },
    );
    map
}

fn calendar_range(start: &str, end: &str) -> TimeRange {
    TimeRange {
        start: start.to_string(),
        end: end.to_string(),
        axis: PartitionAxis::Calendar,
        column_type: PartitionColumnType::Undeclared,
    }
}

const SQL: &str = "SELECT * FROM smelt.silver.events_parsed";

#[test]
fn driving_source_pushdown_renders_typed_date_literal() {
    let result = inject_source_filters(
        SQL,
        &bounds(PartitionColumnType::Date),
        &calendar_range("2026-01-01", "2026-01-02"),
    );
    assert!(
        result.contains("event_date >= DATE '2026-01-01'")
            && result.contains("event_date < DATE '2026-01-02'"),
        "expected typed DATE literals in: {result}"
    );
}

#[test]
fn driving_source_pushdown_renders_bare_quoted_for_declared_varchar() {
    let result = inject_source_filters(
        SQL,
        &bounds(PartitionColumnType::Text),
        &calendar_range("2026-01-01", "2026-01-02"),
    );
    assert!(
        result.contains("event_date >= '2026-01-01'")
            && result.contains("event_date < '2026-01-02'")
            && !result.contains("DATE '"),
        "expected bare-quoted (non-DATE) literals in: {result}"
    );
}

#[test]
fn driving_source_pushdown_renders_bare_integer_on_integer_axis() {
    let mut map = HashMap::new();
    map.insert(
        "smelt.silver.events_parsed".to_string(),
        SourceBound {
            partition_col: "batch_id".to_string(),
            before_secs: 0,
            after_secs: 0,
            column_type: PartitionColumnType::Undeclared,
        },
    );
    let range = TimeRange {
        start: "7".to_string(),
        end: "9".to_string(),
        axis: PartitionAxis::Integer,
        column_type: PartitionColumnType::Undeclared,
    };
    let result = inject_source_filters(SQL, &map, &range);
    assert!(
        result.contains("batch_id >= 7") && result.contains("batch_id < 9"),
        "expected bare (never quoted) integer literals in: {result}"
    );
    assert!(
        !result.contains("'7'") && !result.contains("'9'"),
        "integer-axis literals must never be quoted: {result}"
    );
}
