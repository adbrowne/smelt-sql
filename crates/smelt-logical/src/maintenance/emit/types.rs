//! Statement, group, dialect and region value types shared by every
//! emitter in this module, plus the partition-literal and widened-scan
//! predicate renderers built on them.

use crate::maintenance::ScanClamp;
use crate::PartitionAxis;

/// Render a bare partition-column value as a SQL literal **in its axis's own
/// domain** — the single owner of partition-literal quoting
/// (`docs/specs/incremental_shapes.md` §"The partition grain" rule 8a): quoted
/// and escaped (`'2026-01-01'`) on the calendar axis, bare (`7`) on the
/// integer axis. `Err` when `value` doesn't parse as a bare integer on the
/// integer axis — fail-closed rather than silently emitting a malformed
/// literal.
pub fn partition_literal(axis: PartitionAxis, value: &str) -> Result<String, String> {
    match axis {
        PartitionAxis::Calendar => Ok(format!("'{}'", value.replace('\'', "''"))),
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
    /// rendering each through [`partition_literal`] for `axis` — the single
    /// owner of partition-literal quoting. `Err` propagates a malformed
    /// integer-axis value.
    pub fn for_axis(axis: PartitionAxis, start: &str, end: &str) -> Result<Region, String> {
        Ok(Region {
            start: partition_literal(axis, start)?,
            end: partition_literal(axis, end)?,
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
