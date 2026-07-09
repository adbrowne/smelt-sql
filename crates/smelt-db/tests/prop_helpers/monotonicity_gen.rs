//! smelt-sql generator for the event-time monotonicity soundness oracle
//! (`crates/smelt-db/tests/monotonicity_soundness_tests.rs`).
//!
//! This module only produces *smelt-sql fragments* (the projected
//! `event_time` expression text) and the proptest strategies that drive
//! window/data generation. It deliberately knows nothing about
//! `smelt_logical::trace_event_time` or DuckDB execution — those live in the
//! test file, which compiles the generated smelt-sql through smelt's own
//! compiler (`smelt-runtime`) rather than hand-assembling backend SQL here
//! (see plan `docs/plans/20260702-monotonicity-primitive-tested.md` Phase 2,
//! "owner correction 2026-07-02").
//!
//! Reuses the DuckDB/Spark *execution* plumbing in `duckdb_oracle.rs` /
//! `spark_oracle.rs` and NULL-bearing-data ideas from `null_data.rs`; does
//! NOT reuse `generators.rs`'s SQL generation (that module targets raw
//! backend SQL for type-divergence testing — a different purpose).

use proptest::prelude::*;

/// Duration constants, in seconds.
pub const HOUR: i64 = 3_600;
pub const DAY: i64 = 86_400;

/// Base epoch every generated timestamp is anchored to. Expressed as a SQL
/// fragment so all timestamp arithmetic happens inside DuckDB itself (no
/// chrono/date-formatting needed on the Rust side).
pub const BASE_TS_SQL: &str = "TIMESTAMP '2024-01-01 00:00:00'";

/// The (single-segment) `smelt.<path>` source ref used by every generated
/// model. Single segment resolves to `main.mono_src` per
/// `SqlCompiler::make_path_ref_resolver` (`{schema}.{segs.join("_")}`).
pub const SOURCE_REF: &str = "smelt.mono_src";
/// The physical table name the compiler resolves `SOURCE_REF` to under
/// schema `main` (matches the `test_config()`/`duckdb_target()` recipe used
/// by the test file, mirroring `compile_parity.rs`).
pub const SOURCE_TABLE: &str = "main.mono_src";
/// Name used for the source in `BoundContext::with_source` — descriptive
/// only; `trace_event_time` never compares it against `SOURCE_REF`.
pub const SOURCE_NAME: &str = "mono_src";
/// The timestamp column on the source table; also the declared partition
/// column in `BoundContext`.
pub const SOURCE_COLUMN: &str = "event_ts";

/// A generated shape for the projected `event_time` expression. The
/// whitelist variants (`is_whitelist() == true`) are expected to trace as
/// `Traceable`; the rest are blacklist / random-head shapes expected to
/// never trace as `Traceable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    // ---- Whitelist (must trace Traceable) ----
    BareColumn,
    QualifiedColumn,
    IntervalShiftSeconds(u32),
    IntervalShiftMinutes(u32),
    IntervalShiftHours(u32),
    IntervalShiftDays(u32),
    CastTimestamp,
    /// Weak (day-grid): `CAST(col AS DATE)`.
    CastDate,
    /// Weak (day-grid): `DATE_TRUNC('day', col)`.
    DateTruncDay,
    /// Weak (hour-grid): `DATE_TRUNC('hour', col)`.
    DateTruncHour,
    /// Strict composition: `CAST(col AS TIMESTAMP) + INTERVAL 'N hours'`.
    ComposeCastShiftHours(u32),
    /// Weak (day-grid) composition: `DATE_TRUNC('day', CAST(col AS TIMESTAMP))`.
    ComposeTruncDayOfCast,

    // ---- Blacklist / random / unknown heads (must never trace Traceable) ----
    TwoColumnArithmetic,
    ModFn,
    ExtractFn,
    CaseExpr,
    /// `COALESCE(col, const)` — StaticSeed, not NotTraceable, but still
    /// "never Traceable".
    CoalesceConst,
    GreatestConst,
    UnknownUdf,
    /// Bare constant literal — StaticSeed.
    ConstLiteral,
    /// Bare `NULL` literal — StaticSeed.
    NullLiteral,
    NowFn,
    CastVarchar,
}

impl Shape {
    /// Render the SQL text for this shape's `event_time` projection, given
    /// the source column name (unqualified) to reference.
    pub fn sql_expr(&self, col: &str) -> String {
        match self {
            Shape::BareColumn => col.to_string(),
            Shape::QualifiedColumn => format!("t.{col}"),
            Shape::IntervalShiftSeconds(n) => format!("{col} + INTERVAL '{n} seconds'"),
            Shape::IntervalShiftMinutes(n) => format!("{col} + INTERVAL '{n} minutes'"),
            Shape::IntervalShiftHours(n) => format!("{col} + INTERVAL '{n} hours'"),
            Shape::IntervalShiftDays(n) => format!("{col} + INTERVAL '{n} days'"),
            Shape::CastTimestamp => format!("CAST({col} AS TIMESTAMP)"),
            Shape::CastDate => format!("CAST({col} AS DATE)"),
            Shape::DateTruncDay => format!("DATE_TRUNC('day', {col})"),
            Shape::DateTruncHour => format!("DATE_TRUNC('hour', {col})"),
            Shape::ComposeCastShiftHours(n) => {
                format!("CAST({col} AS TIMESTAMP) + INTERVAL '{n} hours'")
            }
            Shape::ComposeTruncDayOfCast => format!("DATE_TRUNC('day', CAST({col} AS TIMESTAMP))"),

            Shape::TwoColumnArithmetic => format!("{col} - other_ts"),
            Shape::ModFn => format!("MOD({col}, 100)"),
            Shape::ExtractFn => format!("EXTRACT(HOUR FROM {col})"),
            Shape::CaseExpr => {
                format!("CASE WHEN {col} IS NULL THEN {col} ELSE {col} END")
            }
            Shape::CoalesceConst => format!("COALESCE({col}, {BASE_TS_SQL})"),
            Shape::GreatestConst => format!("GREATEST({col}, {BASE_TS_SQL})"),
            Shape::UnknownUdf => format!("my_custom_fn({col})"),
            Shape::ConstLiteral => "42".to_string(),
            Shape::NullLiteral => "NULL".to_string(),
            Shape::NowFn => "NOW()".to_string(),
            Shape::CastVarchar => format!("CAST({col} AS VARCHAR)"),
        }
    }

    /// Whether the real primitive is expected to classify this shape's
    /// expression as `Traceable`.
    pub fn is_whitelist(&self) -> bool {
        matches!(
            self,
            Shape::BareColumn
                | Shape::QualifiedColumn
                | Shape::IntervalShiftSeconds(_)
                | Shape::IntervalShiftMinutes(_)
                | Shape::IntervalShiftHours(_)
                | Shape::IntervalShiftDays(_)
                | Shape::CastTimestamp
                | Shape::CastDate
                | Shape::DateTruncDay
                | Shape::DateTruncHour
                | Shape::ComposeCastShiftHours(_)
                | Shape::ComposeTruncDayOfCast
        )
    }

    /// Grid granularity in seconds for weakly-monotonic (many-to-one)
    /// shapes; `0` for strict (bijective-shift) shapes. Window bounds for
    /// weak shapes MUST be generated as multiples of this grid — see the
    /// module doc on `gen_case_strategy` for why (the naive
    /// output-clamp-equals-source-filter identity only holds when the
    /// window boundaries are grid-aligned; unaligned bounds would require a
    /// widening consumer, deferred to W2-W5).
    pub fn grid_seconds(&self) -> i64 {
        match self {
            Shape::CastDate | Shape::DateTruncDay | Shape::ComposeTruncDayOfCast => DAY,
            Shape::DateTruncHour => HOUR,
            _ => 0,
        }
    }

    /// The offset (in seconds) this shape's chain is expected to fold to,
    /// for whitelist shapes only (cross-checked against the trace's actual
    /// `Offset` in the test).
    pub fn expected_offset_seconds(&self) -> i64 {
        match self {
            Shape::IntervalShiftSeconds(n) => i64::from(*n),
            Shape::IntervalShiftMinutes(n) => i64::from(*n) * 60,
            Shape::IntervalShiftHours(n) => i64::from(*n) * HOUR,
            Shape::IntervalShiftDays(n) => i64::from(*n) * DAY,
            Shape::ComposeCastShiftHours(n) => i64::from(*n) * HOUR,
            _ => 0,
        }
    }
}

/// One generated test case: a whitelist shape, a window `[lo, hi)` (seconds
/// relative to `BASE_TS_SQL`), and a data set of source-column offsets
/// (`None` = NULL row).
#[derive(Debug, Clone)]
pub struct GenCase {
    pub shape: Shape,
    pub lo: i64,
    pub hi: i64,
    pub data: Vec<Option<i64>>,
}

/// Strategy over whitelist-only shapes (used by the commutation property —
/// blacklist shapes are covered by the separate deterministic hazard corpus
/// in the test file, since they need no DB execution).
pub fn whitelist_shape_strategy() -> impl Strategy<Value = Shape> {
    prop_oneof![
        Just(Shape::BareColumn),
        Just(Shape::QualifiedColumn),
        (1u32..3000).prop_map(Shape::IntervalShiftSeconds),
        (1u32..90).prop_map(Shape::IntervalShiftMinutes),
        (1u32..36).prop_map(Shape::IntervalShiftHours),
        (1u32..3).prop_map(Shape::IntervalShiftDays),
        Just(Shape::CastTimestamp),
        Just(Shape::CastDate),
        Just(Shape::DateTruncDay),
        Just(Shape::DateTruncHour),
        (1u32..36).prop_map(Shape::ComposeCastShiftHours),
        Just(Shape::ComposeTruncDayOfCast),
    ]
}

/// Data-offset strategy: seconds relative to `BASE_TS_SQL`, wide enough to
/// straddle any window/offset combination the whitelist strategies above can
/// produce, with ~20% NULL rows (`None`) and forced boundary rows are added
/// separately by the caller once `lo`/`hi` are known.
fn data_strategy() -> impl Strategy<Value = Vec<Option<i64>>> {
    let value = prop_oneof![
        1 => Just(None),
        4 => (-7 * DAY..7 * DAY).prop_map(Some),
    ];
    prop::collection::vec(value, 16..24)
}

/// Window strategy for strict (grid = 0) shapes: arbitrary integer-second
/// bounds.
fn strict_window_strategy() -> impl Strategy<Value = (i64, i64)> {
    (-3 * DAY..3 * DAY, 1..2 * DAY).prop_map(|(lo, delta)| (lo, lo + delta))
}

/// Window strategy for a grid-aligned shape: both bounds are multiples of
/// `grid` seconds relative to `BASE_TS_SQL`.
fn grid_window_strategy(grid: i64, max_k: i64, max_m: i64) -> impl Strategy<Value = (i64, i64)> {
    (-max_k..max_k, 1..max_m).prop_map(move |(k, m)| (k * grid, (k + m) * grid))
}

/// Build the full `GenCase` strategy: pick a whitelist shape, then a
/// window matching its grid, then a data set (plus forced boundary rows at
/// `lo` and `hi - 1` for deterministic edge coverage).
pub fn gen_case_strategy() -> impl Strategy<Value = GenCase> {
    whitelist_shape_strategy().prop_flat_map(|shape| {
        let grid = shape.grid_seconds();
        let window_strategy: BoxedStrategy<(i64, i64)> = if grid == 0 {
            strict_window_strategy().boxed()
        } else if grid == DAY {
            grid_window_strategy(DAY, 3, 4).boxed()
        } else {
            grid_window_strategy(HOUR, 72, 24).boxed()
        };
        (window_strategy, data_strategy()).prop_map(move |((lo, hi), mut data)| {
            // Force a couple of boundary-straddling rows so every case
            // exercises the window edges, not just randomly-placed data.
            data.push(Some(lo));
            data.push(Some(hi - 1));
            GenCase {
                shape,
                lo,
                hi,
                data,
            }
        })
    })
}

/// Render a SQL timestamp literal for `secs` seconds relative to
/// `BASE_TS_SQL`, via DuckDB's own interval arithmetic (works for negative
/// `secs` too — DuckDB accepts negative `INTERVAL` literals).
pub fn ts_literal(secs: i64) -> String {
    format!("({BASE_TS_SQL} + INTERVAL '{secs} seconds')")
}

/// Deterministic hazard corpus: hand-picked hazardous smelt-sql
/// `event_time` expressions that must NEVER classify as `Traceable`,
/// covering every blacklist entry plus the named research hazards folded in
/// as regression seeds:
/// - P3: `COALESCE`-injected NULL-seed constant (§2.5 static-seed hazard).
/// - Q5: a `CASE`-shaped non-commuting body (piecewise, not monotone).
/// - J3/J4: multi-clock two-column arithmetic (join driving-fact ambiguity).
pub fn hazard_corpus() -> Vec<(&'static str, String)> {
    let col = SOURCE_COLUMN;
    vec![
        (
            "two_column_arithmetic_j3j4",
            Shape::TwoColumnArithmetic.sql_expr(col),
        ),
        ("mod_fn", Shape::ModFn.sql_expr(col)),
        ("extract_fn", Shape::ExtractFn.sql_expr(col)),
        ("case_expr_q5", Shape::CaseExpr.sql_expr(col)),
        ("coalesce_const_p3", Shape::CoalesceConst.sql_expr(col)),
        ("greatest_const", Shape::GreatestConst.sql_expr(col)),
        ("least_const", format!("LEAST({col}, {BASE_TS_SQL})")),
        ("unknown_udf", Shape::UnknownUdf.sql_expr(col)),
        ("const_literal", Shape::ConstLiteral.sql_expr(col)),
        ("null_literal_p3", Shape::NullLiteral.sql_expr(col)),
        ("now_fn", Shape::NowFn.sql_expr(col)),
        ("cast_varchar", Shape::CastVarchar.sql_expr(col)),
        (
            "case_q5_branching_columns",
            format!("CASE WHEN {col} > {BASE_TS_SQL} THEN {col} ELSE other_ts END"),
        ),
        ("coalesce_two_columns", format!("COALESCE({col}, other_ts)")),
        (
            "greatest_two_columns_j3",
            format!("GREATEST({col}, other_ts)"),
        ),
        (
            "nested_extract_in_case_q5",
            format!("CASE WHEN EXTRACT(HOUR FROM {col}) > 12 THEN {col} ELSE {col} END"),
        ),
        ("current_date_fn", "CURRENT_DATE".to_string()),
        ("current_timestamp_fn", "CURRENT_TIMESTAMP".to_string()),
    ]
}
