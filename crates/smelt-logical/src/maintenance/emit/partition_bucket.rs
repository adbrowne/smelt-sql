//! How a source's declared partition column maps onto its declared
//! partition grid — the bucket expression every per-partition maintenance
//! statement groups by.
//!
//! A source declares `timeseries: { partition_column, granularity }`
//! (`docs/specs/sources.md`). The two are independent: a TIMESTAMP
//! `partition_column` under `granularity: day` holds one value per
//! *second*, not per day. Grouping such a column raw does not produce the
//! source's partitions — it produces one group per distinct instant, which
//! makes every per-partition verdict (the append-only posture baseline's
//! "closed partition" reasoning above all) wrong on every backend, and on
//! GoogleSQL additionally unplannable once the group count reaches the
//! thousands.
//!
//! So the grid is applied explicitly, here, once: [`classify_partition_bucket`]
//! decides from the column's *declared* type and the declared granularity
//! whether the raw column already holds exactly one value per partition, and
//! [`partition_bucket_expr`] renders the truncation where it does not.
//!
//! The week grid is Monday-based, matching the only week boundary smelt
//! commits to anywhere (`smelt-runtime`'s `align_output_start`); the
//! per-dialect spellings below are pinned to that rather than to each
//! engine's default week start, which differs (GoogleSQL's bare `WEEK` is
//! Sunday-based).

use smelt_core::config::Granularity;
use smelt_types::DataType;

use super::types::MaintenanceDialect;

/// Whether a partition column's raw values already are its partitions, or
/// have to be truncated onto the declared grid first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartitionBucket {
    /// The raw column holds exactly one value per declared partition: an
    /// integer partition key (`20260805`), a DATE column at `day`
    /// granularity, or a column whose declared type does not let us prove
    /// otherwise (the conservative status quo — grouping raw is then what
    /// smelt has always done, and narrowing it would need a diagnostic
    /// rather than a silent change of meaning).
    Exact,
    /// A temporal column finer than the declared grid: truncate to
    /// `Granularity` before grouping. The [`DataType`] is carried because
    /// GoogleSQL's truncation function name is type-dependent
    /// (`TIMESTAMP_TRUNC` vs `DATE_TRUNC`).
    Truncate(Granularity, DataType),
}

/// Decide `partition_column`'s bucket from its **declared** type and the
/// source's declared `granularity`. Pure, and deliberately typed off the
/// declaration rather than off a catalog probe: the emitters this feeds are
/// pure statement authors with no warehouse access.
///
/// `column_type` is `None` when the source declares no type for the
/// partition column, which yields [`PartitionBucket::Exact`] — see that
/// variant's doc comment for why that stays the answer.
pub fn classify_partition_bucket(
    column_type: Option<&DataType>,
    granularity: Granularity,
) -> PartitionBucket {
    match column_type {
        // A DATE column is already a day grid; truncating it to `day` is a
        // no-op, so the raw column is the bucket. Coarser grids still need
        // the truncation.
        Some(DataType::Date) if granularity == Granularity::Day => PartitionBucket::Exact,
        Some(ty @ (DataType::Date | DataType::Timestamp { .. })) => {
            PartitionBucket::Truncate(granularity, ty.clone())
        }
        // Everything else — integer partition keys, and any type we cannot
        // prove is finer than the grid.
        _ => PartitionBucket::Exact,
    }
}

/// The expression to `GROUP BY` (and to project as the partition value) for
/// `partition_column` under `bucket`, in `dialect`'s spelling.
pub fn partition_bucket_expr(
    partition_column: &str,
    bucket: &PartitionBucket,
    dialect: MaintenanceDialect,
) -> String {
    let (granularity, column_type) = match bucket {
        PartitionBucket::Exact => return partition_column.to_string(),
        PartitionBucket::Truncate(g, ty) => (g, ty),
    };
    match dialect {
        // DuckDB and Spark both take the unit as a quoted first argument and
        // are Monday-based on `week`.
        MaintenanceDialect::DuckDb | MaintenanceDialect::Spark => {
            let unit = quoted_unit(*granularity);
            format!("DATE_TRUNC('{unit}', {partition_column})")
        }
        // GoogleSQL takes the unit as a bare keyword *after* the value, and
        // spells the function per argument type. Its bare `WEEK` starts on
        // Sunday, so the Monday grid is requested explicitly.
        MaintenanceDialect::BigQuery => {
            let func = match column_type {
                DataType::Date => "DATE_TRUNC",
                _ => "TIMESTAMP_TRUNC",
            };
            let unit = googlesql_unit(*granularity);
            format!("{func}({partition_column}, {unit})")
        }
    }
}

/// The unit spelling DuckDB and Spark's `DATE_TRUNC(unit, value)` take.
fn quoted_unit(granularity: Granularity) -> &'static str {
    match granularity {
        Granularity::Hour => "hour",
        Granularity::Day => "day",
        Granularity::Week => "week",
        Granularity::Month => "month",
        Granularity::Quarter => "quarter",
        Granularity::Year => "year",
    }
}

/// The unit keyword GoogleSQL's `*_TRUNC(value, unit)` takes. `WEEK(MONDAY)`
/// rather than `WEEK`, because the bare form is Sunday-based and smelt's
/// week grid is Monday-based.
fn googlesql_unit(granularity: Granularity) -> &'static str {
    match granularity {
        Granularity::Hour => "HOUR",
        Granularity::Day => "DAY",
        Granularity::Week => "WEEK(MONDAY)",
        Granularity::Month => "MONTH",
        Granularity::Quarter => "QUARTER",
        Granularity::Year => "YEAR",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts() -> DataType {
        DataType::Timestamp {
            with_timezone: false,
        }
    }

    #[test]
    fn a_timestamp_column_under_a_day_grid_is_truncated_not_grouped_raw() {
        let bucket = classify_partition_bucket(Some(&ts()), Granularity::Day);
        assert_eq!(bucket, PartitionBucket::Truncate(Granularity::Day, ts()));
        assert_eq!(
            partition_bucket_expr("created_at", &bucket, MaintenanceDialect::DuckDb),
            "DATE_TRUNC('day', created_at)"
        );
        assert_eq!(
            partition_bucket_expr("created_at", &bucket, MaintenanceDialect::BigQuery),
            "TIMESTAMP_TRUNC(created_at, DAY)"
        );
        assert_eq!(
            partition_bucket_expr("created_at", &bucket, MaintenanceDialect::Spark),
            "DATE_TRUNC('day', created_at)"
        );
    }

    #[test]
    fn a_date_column_under_a_day_grid_is_already_its_own_bucket() {
        let bucket = classify_partition_bucket(Some(&DataType::Date), Granularity::Day);
        assert_eq!(bucket, PartitionBucket::Exact);
        for dialect in [
            MaintenanceDialect::DuckDb,
            MaintenanceDialect::Spark,
            MaintenanceDialect::BigQuery,
        ] {
            assert_eq!(
                partition_bucket_expr("ingested_date", &bucket, dialect),
                "ingested_date"
            );
        }
    }

    #[test]
    fn a_date_column_under_a_coarser_grid_is_still_truncated() {
        let bucket = classify_partition_bucket(Some(&DataType::Date), Granularity::Month);
        assert_eq!(
            bucket,
            PartitionBucket::Truncate(Granularity::Month, DataType::Date)
        );
        assert_eq!(
            partition_bucket_expr("ingested_date", &bucket, MaintenanceDialect::BigQuery),
            "DATE_TRUNC(ingested_date, MONTH)"
        );
    }

    #[test]
    fn an_integer_partition_key_is_never_truncated() {
        for ty in [DataType::Integer, DataType::BigInt] {
            let bucket = classify_partition_bucket(Some(&ty), Granularity::Day);
            assert_eq!(bucket, PartitionBucket::Exact);
        }
    }

    #[test]
    fn an_undeclared_column_type_keeps_the_status_quo() {
        assert_eq!(
            classify_partition_bucket(None, Granularity::Day),
            PartitionBucket::Exact
        );
    }

    /// The week grid is Monday-based everywhere, including on GoogleSQL
    /// whose bare `WEEK` is Sunday-based.
    #[test]
    fn the_week_grid_is_monday_based_on_every_dialect() {
        let bucket = classify_partition_bucket(Some(&ts()), Granularity::Week);
        assert_eq!(
            partition_bucket_expr("created_at", &bucket, MaintenanceDialect::BigQuery),
            "TIMESTAMP_TRUNC(created_at, WEEK(MONDAY))"
        );
        assert_eq!(
            partition_bucket_expr("created_at", &bucket, MaintenanceDialect::DuckDb),
            "DATE_TRUNC('week', created_at)"
        );
    }
}
