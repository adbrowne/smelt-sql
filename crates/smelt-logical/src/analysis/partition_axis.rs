//! Partition-axis domain classification (`docs/specs/timeseries.md` §Semantics
//! "Partition axis domain").
//!
//! A partition-grain model's `partition_column` resolves to one of two axis
//! domains: a **calendar** axis (date/timestamp — the existing behavior) or a
//! **unit-step integer grid** (one partition is one integer value). This is a
//! pure leaf classifier over the column's resolved [`smelt_types::DataType`] —
//! it does not scan SQL and is not part of the property-composition walk.

use smelt_types::DataType;

/// The domain a partition axis operates in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PartitionAxis {
    /// Calendar dates/timestamps — bounds are `YYYY-MM-DD`, chunking steps by
    /// `timeseries.granularity`.
    Calendar,
    /// A unit-step integer grid — one partition is one integer value; bounds
    /// are bare integers, chunking steps by 1 unit (or `--batch-size N`
    /// units).
    Integer,
}

/// Classify `partition_column`'s resolved type into a [`PartitionAxis`].
///
/// Returns `None` for a type that is neither a calendar nor an integer type
/// (e.g. `Text`, `Unknown`) — undecidable/inadmissible, not a positive
/// disproof of either domain; callers must handle `None` explicitly rather
/// than defaulting.
pub fn partition_axis_for_type(data_type: &DataType) -> Option<PartitionAxis> {
    match data_type {
        DataType::Date | DataType::Timestamp { .. } => Some(PartitionAxis::Calendar),
        DataType::SmallInt | DataType::Integer | DataType::BigInt => Some(PartitionAxis::Integer),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_axis_for_type_classifies_date_integer_and_other() {
        assert_eq!(
            partition_axis_for_type(&DataType::Date),
            Some(PartitionAxis::Calendar)
        );
        assert_eq!(
            partition_axis_for_type(&DataType::Timestamp {
                with_timezone: false
            }),
            Some(PartitionAxis::Calendar)
        );
        assert_eq!(
            partition_axis_for_type(&DataType::Timestamp {
                with_timezone: true
            }),
            Some(PartitionAxis::Calendar)
        );

        assert_eq!(
            partition_axis_for_type(&DataType::SmallInt),
            Some(PartitionAxis::Integer)
        );
        assert_eq!(
            partition_axis_for_type(&DataType::Integer),
            Some(PartitionAxis::Integer)
        );
        assert_eq!(
            partition_axis_for_type(&DataType::BigInt),
            Some(PartitionAxis::Integer)
        );

        assert_eq!(partition_axis_for_type(&DataType::Text), None);
        assert_eq!(partition_axis_for_type(&DataType::unknown_dynamic()), None);
    }
}
