//! Statement, group, dialect and region value types shared by every
//! emitter in this module, plus the partition-literal and widened-scan
//! predicate renderers built on them.

use crate::maintenance::ScanClamp;
use crate::PartitionAxis;
use smelt_types::DataType;

/// The referenced partition column's own SQL type, as declared or inferred —
/// the second axis `partition_literal` renders against, alongside the
/// [`PartitionAxis`] (`docs/specs/incremental_shapes.md` §"The partition
/// grain" rule 8a). On the calendar `PartitionAxis` a column can be genuinely
/// `Date`/`Timestamp`-typed (an Iceberg/Delta table's declared column) or
/// merely `Text`-typed but calendar-*shaped* (a legacy `VARCHAR` column
/// holding `YYYY-MM-DD` strings, e.g. `examples/web_analytics`'s
/// `event_date`) — the two need different literal spellings on a strict
/// engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PartitionColumnType {
    /// Declared or inferred `DATE`.
    Date,
    /// Declared or inferred `TIMESTAMP` (any precision/timezone variant).
    Timestamp,
    /// Declared or inferred a string type (`VARCHAR`/`TEXT`/...).
    Text,
    /// No declaration and no resolvable inference reached this call site.
    /// This is the *compatibility* arm, not an `Unknown`-style silent
    /// default: it renders exactly as today's calendar-axis behaviour
    /// (quoted, escaped string) and is only reachable where no declared or
    /// inferred type exists — it never changes a currently-working
    /// spelling.
    #[default]
    Undeclared,
}

/// Classify a resolved [`DataType`] into the [`PartitionColumnType`]
/// [`partition_literal`] renders against. `Text`/`Varchar`/`Char` classify as
/// `Text`; every other type (including numeric types on the integer axis,
/// where `column_type` is unconsulted) classifies as `Undeclared` — this is
/// not a positive claim about those types, only that no calendar-literal
/// spelling decision depends on them.
pub fn partition_column_type_for_type(data_type: &DataType) -> PartitionColumnType {
    match data_type {
        DataType::Date => PartitionColumnType::Date,
        DataType::Timestamp { .. } => PartitionColumnType::Timestamp,
        DataType::Text | DataType::Varchar { .. } | DataType::Char { .. } => {
            PartitionColumnType::Text
        }
        _ => PartitionColumnType::Undeclared,
    }
}

/// Render a bare partition-column value as a SQL literal **in its axis's own
/// domain and its referenced column's own type** — the single owner of
/// partition-literal quoting (`docs/specs/incremental_shapes.md` §"The
/// partition grain" rule 8a). On the integer axis: bare (`7`), `column_type`
/// unconsulted. On the calendar axis: `DATE '…'`/`TIMESTAMP '…'` when
/// `column_type` is `Date`/`Timestamp`; a quoted, escaped string
/// (`'2026-01-01'`) when it is `Text` or `Undeclared`. `Err` when `value`
/// doesn't parse as a bare integer on the integer axis, or doesn't parse as a
/// date/timestamp shape against a `Date`/`Timestamp` column — fail-closed
/// rather than silently emitting a malformed or mistyped literal.
pub fn partition_literal(
    axis: PartitionAxis,
    column_type: PartitionColumnType,
    value: &str,
) -> Result<String, String> {
    match axis {
        PartitionAxis::Calendar => match column_type {
            PartitionColumnType::Date => {
                if is_date_shaped(value) {
                    Ok(format!("DATE '{value}'"))
                } else {
                    Err(format!(
                        "expected a DATE-shaped ('YYYY-MM-DD') value for a DATE partition \
                         column, got '{value}'"
                    ))
                }
            }
            PartitionColumnType::Timestamp => {
                if is_date_shaped(value) || is_timestamp_shaped(value) {
                    Ok(format!("TIMESTAMP '{value}'"))
                } else {
                    Err(format!(
                        "expected a TIMESTAMP-shaped ('YYYY-MM-DD HH:MM:SS[.fff]') value for a \
                         TIMESTAMP partition column, got '{value}'"
                    ))
                }
            }
            PartitionColumnType::Text | PartitionColumnType::Undeclared => {
                Ok(format!("'{}'", value.replace('\'', "''")))
            }
        },
        PartitionAxis::Integer => {
            value
                .trim()
                .parse::<i64>()
                .map(|v| v.to_string())
                .map_err(|_| {
                    format!(
                "expected a bare integer literal for an integer partition axis, got '{value}'"
            )
                })
        }
    }
}

/// `value` parses as a bare `YYYY-MM-DD` date.
fn is_date_shaped(value: &str) -> bool {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
}

/// `value` parses as `YYYY-MM-DD[ T]HH:MM:SS[.fff][Z]` — a space or `T`
/// separator, optional fractional seconds, optional trailing `Z`.
fn is_timestamp_shaped(value: &str) -> bool {
    let trimmed = value.strip_suffix('Z').unwrap_or(value);
    let normalized = trimmed.replacen('T', " ", 1);
    ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S"]
        .iter()
        .any(|fmt| chrono::NaiveDateTime::parse_from_str(&normalized, fmt).is_ok())
}

/// One SQL statement a maintenance run executes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceStatement {
    pub sql: String,
}

impl MaintenanceStatement {
    pub(crate) fn new(sql: String) -> Self {
        Self { sql }
    }
}

/// An ordered group of [`MaintenanceStatement`]s produced by one emitter
/// call, plus whether they must run inside a single backend transaction. A
/// paired region `DELETE`+`INSERT` is transactional: a failed `INSERT` must
/// roll back its `DELETE` (`docs/specs/incremental_models.md` §"Statement
/// emission (single owner)").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementGroup {
    pub statements: Vec<MaintenanceStatement>,
    pub transactional: bool,
}

/// The backend SQL dialect a [`StatementGroup`] is rendered for. Dialect
/// differences (e.g. a `MERGE … UPDATE SET *` requiring a full-row source
/// projection versus an explicit column-list `SET`) live in the emitters as
/// dialect-keyed variants, not in backend string construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintenanceDialect {
    DuckDb,
    Spark,
    BigQuery,
    Trino,
}

/// A half-open region `[start, end)` on the output partition column; values
/// are SQL literals (already quoted where needed).
#[derive(Debug, Clone)]
pub struct Region {
    pub start: String,
    pub end: String,
}

/// The widened scan predicate a derived [`ScanClamp`] implies for
/// maintaining output region `[start, end)`: the source's partition column
/// over `[start − before, end + after)`. This is the *derived* number turned
/// into SQL — the caller injects it into the read (the body's source scan),
/// so a wrongly-derived window fails the equivalence oracle rather than
/// silently over- or under-reading.
pub fn widened_scan_predicate(clamp: &ScanClamp, region: &Region) -> String {
    let lower = if clamp.before.0 == 0 {
        region.start.clone()
    } else {
        format!("{} - INTERVAL '{} seconds'", region.start, clamp.before.0)
    };
    let upper = if clamp.after.0 == 0 {
        region.end.clone()
    } else {
        format!("{} + INTERVAL '{} seconds'", region.end, clamp.after.0)
    };
    format!("{col} >= {lower} AND {col} < {upper}", col = clamp.column)
}

impl Region {
    /// Build a [`Region`] from bare (unquoted) partition-column values,
    /// rendering each through [`partition_literal`] for `axis` and
    /// `column_type` — the single owner of partition-literal quoting. `Err`
    /// propagates a malformed integer-axis value or a value that doesn't
    /// parse against a `Date`/`Timestamp` `column_type`.
    pub fn for_axis(
        axis: PartitionAxis,
        column_type: PartitionColumnType,
        start: &str,
        end: &str,
    ) -> Result<Region, String> {
        Ok(Region {
            start: partition_literal(axis, column_type, start)?,
            end: partition_literal(axis, column_type, end)?,
        })
    }

    pub fn predicate(&self, qualifier: Option<&str>, column: &str) -> String {
        let col = match qualifier {
            Some(q) => format!("{q}.{column}"),
            None => column.to_string(),
        };
        format!(
            "{col} >= {start} AND {col} < {end}",
            start = self.start,
            end = self.end
        )
    }
}
