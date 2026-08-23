//! Accepted `(entry, dialect)` divergences.
//!
//! A pair absent from this table must pass both legs. A pair present in it is a
//! recorded, reviewer-visible exception — never a silent skip.
//!
//! The table is **two-sided**, like `.claude/hardening-baseline.txt`: an
//! unregistered mismatch fails loudly, and so does an unreachable row. A row
//! naming an entry the registry no longer has, or a pair the harness never
//! probes, is an error telling you to delete it.

use smelt_types::DialectId;

use crate::probe::Position;

/// Why a pair is exempt from passing both legs.
///
/// One variant today. Two others were designed and deliberately not written
/// yet, because neither has a real instance to record:
///
/// - **`Divergent`** — an accepted, permanent semantic difference (Spark's
///   integer-division semantics and the like). DuckDB is the reference engine,
///   so it cannot diverge from itself; the variant lands with the first
///   cross-engine sweep that finds one.
/// - **`SchemaOnly`** — nondeterministic entries. That verdict already lives in
///   `overrides.rs` as `Override::schema_only`, attached to the probe rather
///   than to a `(name, dialect)` pair, which is the right home: `NOW()` is
///   nondeterministic on every engine, not on one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// A lowering smelt owes, with a tracking issue. Does not fail, but the
    /// count ratchets down only (`.claude/dialect-gaps-baseline.txt`).
    Gap {
        issue: &'static str,
        detail: &'static str,
    },
    /// An accepted, permanent semantic difference between the engines — one a
    /// rename or a rewrite cannot close, so users must know about it. Does not
    /// fail and does not ratchet.
    Divergent { reason: &'static str },
}

/// Which leg a row exempts.
///
/// The distinction is load-bearing. A `Schema` row says the engine *refuses*
/// the probe; a `Value` row says it runs and computes something different. Two
/// very different findings, and conflating them would make the schema leg
/// report a value-only row as stale the moment the engine parsed the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leg {
    /// The engine refuses the probe outright.
    Schema,
    /// The engine accepts the probe and returns a different answer.
    Value,
}

#[derive(Debug, Clone, Copy)]
pub struct LedgerRow {
    pub name: &'static str,
    pub dialect: DialectId,
    /// The position this row covers. `None` means every position.
    ///
    /// Scoping matters: Spark has `MEDIAN` as an aggregate but not as a window
    /// function, and a whole-entry exemption would have stopped covering the
    /// position that works.
    pub position: Option<Position>,
    /// Which leg this row exempts.
    pub leg: Leg,
    pub verdict: Verdict,
}

/// A gap the engine surfaces by refusing the probe.
const fn gap(name: &'static str, dialect: DialectId, detail: &'static str) -> LedgerRow {
    LedgerRow {
        name,
        dialect,
        position: None,
        leg: Leg::Schema,
        verdict: Verdict::Gap {
            issue: "#171",
            detail,
        },
    }
}

/// A gap the engine hides: it accepts the probe and computes a different
/// number. The dangerous class — the schema leg cannot see it.
const fn value_gap(name: &'static str, dialect: DialectId, detail: &'static str) -> LedgerRow {
    LedgerRow {
        name,
        dialect,
        position: None,
        leg: Leg::Value,
        verdict: Verdict::Gap {
            issue: "#171",
            detail,
        },
    }
}

/// A gap that applies to one position only.
const fn gap_at(
    name: &'static str,
    dialect: DialectId,
    position: Position,
    detail: &'static str,
) -> LedgerRow {
    LedgerRow {
        name,
        dialect,
        position: Some(position),
        leg: Leg::Schema,
        verdict: Verdict::Gap {
            issue: "#171",
            detail,
        },
    }
}

const fn divergent(name: &'static str, dialect: DialectId, reason: &'static str) -> LedgerRow {
    LedgerRow {
        name,
        dialect,
        position: None,
        leg: Leg::Value,
        verdict: Verdict::Divergent { reason },
    }
}

/// Every accepted `(entry, dialect)` divergence.
pub fn dialect_divergences() -> &'static [LedgerRow] {
    ROWS
}

/// The row covering `(name, dialect, position)` on `leg`, if one exists.
///
/// A `Schema` row also exempts the value leg: a probe the engine refuses can
/// never be value-compared. The reverse is not true.
pub fn find(
    name: &str,
    dialect: DialectId,
    position: Position,
    leg: Leg,
) -> Option<&'static LedgerRow> {
    ROWS.iter().find(|r| {
        r.name == name
            && r.dialect == dialect
            && r.position.map(|p| p == position).unwrap_or(true)
            && (r.leg == leg || (r.leg == Leg::Schema && leg == Leg::Value))
    })
}

/// Every accepted `(entry, dialect)` divergence, found by sweeping the derived
/// probes against live engines.
///
/// A `Gap` row is a name smelt's `BuiltinRegistry` recognises for which the
/// engine has no such function — or has one with different semantics — and the
/// registry records no emission verdict, so the printer emits it verbatim.
/// Closing one means adding an `Emission::Rename`, an `Emission::Rewrite`, or
/// an `Emission::Unsupported` row to the entry, and deleting its row here.
static ROWS: &[LedgerRow] = &[
    // ── Spark ────────────────────────────────────────────────────────────
    // Established by running the derived probes against a live Spark 4.0.0
    // (`SPARK_CONTAINER_ID=$(docker ps -qf name=smelt-spark) cargo test -p
    // smelt-db --test dialect_audit`) on 2026-08-24 — measured, not read from
    // documentation.
    //
    // Gaps: a name Spark SQL has no function for, or has with different
    // semantics. Each is a lowering smelt owes — a `Rename` where Spark spells
    // it differently, a `Rewrite` where the shape differs, or an
    // `Unsupported` verdict where neither is possible.
    gap("AGE", DialectId::SparkSql, "no `age`; Spark expresses interval difference as `ts1 - ts2`"),
    gap("DATE_ADD", DialectId::SparkSql, "Spark's `date_add(date, days)` takes an integer, not an INTERVAL"),
    gap("DATE_SUB", DialectId::SparkSql, "Spark's `date_sub(date, days)` takes an integer, not an INTERVAL"),
    gap("GLOB", DialectId::SparkSql, "no `GLOB` operator; Spark has `LIKE` and `RLIKE`"),
    gap("JSON_ARRAY", DialectId::SparkSql, "no `json_array`; Spark builds JSON with `to_json(array(...))`"),
    gap("JSON_ARRAY_LENGTH", DialectId::SparkSql, "Spark's `json_array_length` wants a JSON string, not a number"),
    gap("JSON_CONTAINS", DialectId::SparkSql, "no `json_contains` in Spark"),
    gap("JSON_EXTRACT", DialectId::SparkSql, "Spark spells it `get_json_object`"),
    gap("JSON_EXTRACT_TEXT", DialectId::SparkSql, "Spark spells it `get_json_object`"),
    gap("JSON_OBJECT", DialectId::SparkSql, "no `json_object`; Spark builds JSON with `to_json(named_struct(...))`"),
    gap("JSON_OBJECT_KEYS", DialectId::SparkSql, "Spark reaches object keys through `from_json`, not a scalar function"),
    gap("MAKE_TIME", DialectId::SparkSql, "no `make_time` in Spark"),
    gap("MAKE_TIMESTAMPTZ", DialectId::SparkSql, "no `make_timestamptz`; Spark has `make_timestamp`"),
    gap("QUOTE_IDENT", DialectId::SparkSql, "PostgreSQL-only builtin"),
    gap("QUOTE_LITERAL", DialectId::SparkSql, "PostgreSQL-only builtin"),
    gap("STRPOS", DialectId::SparkSql, "Spark spells it `instr` / `position`"),
    gap("TO_JSON", DialectId::SparkSql, "Spark's `to_json` takes a struct or array, not a scalar"),
    gap("TO_SECONDS", DialectId::SparkSql, "no `to_seconds` in Spark"),
    gap("TRUNC", DialectId::SparkSql, "Spark's `trunc(date, fmt)` is temporal; there is no numeric `trunc`"),
    gap("TRUNCATE", DialectId::SparkSql, "no `truncate` scalar in Spark"),
    gap("ARG_MAX", DialectId::SparkSql, "Spark spells it `max_by`"),
    gap("ARG_MIN", DialectId::SparkSql, "Spark spells it `min_by`"),
    gap("GROUP_CONCAT", DialectId::SparkSql, "Spark spells it `concat_ws(sep, collect_list(x))`"),
    gap("PERCENTILE_CONT", DialectId::SparkSql, "Spark's ordered-set form requires `WITHIN GROUP (ORDER BY ...)`"),
    gap("PERCENTILE_DISC", DialectId::SparkSql, "Spark's ordered-set form requires `WITHIN GROUP (ORDER BY ...)`"),
    value_gap("LOG", DialectId::SparkSql, "Spark's `log(x)` is the natural logarithm; DuckDB's is base 10 - a silently wrong number, closable by a rename to `log10`"),
    value_gap("DAYOFWEEK", DialectId::SparkSql, "Spark numbers the week from Sunday=1, DuckDB from Sunday=0 - a silently wrong number, closable by a rewrite"),
    // `MEDIAN` works as an aggregate on Spark but not as a window function, so
    // the row is scoped to the position that fails rather than the whole entry.
    gap_at(
        "MEDIAN",
        DialectId::SparkSql,
        Position::Window,
        "Spark has `median` as an aggregate but not as a window function",
    ),
    //
    // Divergences: accepted, permanent semantic differences. No rename or
    // rewrite closes these, so users have to know about them.
    divergent(
        "CONCAT",
        DialectId::SparkSql,
        "Spark's `concat` propagates NULL where DuckDB's treats it as the empty string. A NULL-propagation model difference, not a spelling one.",
    ),
    divergent(
        "ARRAY_AGG",
        DialectId::SparkSql,
        "Spark's `collect_list` drops NULL elements; DuckDB's `array_agg` keeps them. Not closable by a rename.",
    ),
    divergent(
        "CORR",
        DialectId::SparkSql,
        "Spark returns NULL for a degenerate correlation where DuckDB returns NaN.",
    ),
    divergent(
        "REGR_SLOPE",
        DialectId::SparkSql,
        "Spark returns NULL for a degenerate regression where DuckDB returns NaN.",
    ),
    // ── DuckDB ───────────────────────────────────────────────────────────
    // Measured against DuckDB 1.5.4 in-process.
    gap(
        "INITCAP",
        DialectId::DuckDb,
        "no `initcap` in DuckDB 1.5.4; the closest is a manual UPPER/LOWER split",
    ),
    gap(
        "TO_CHAR",
        DialectId::DuckDb,
        "no `to_char` in DuckDB; `strftime` is the temporal half of it",
    ),
    gap(
        "TRUNCATE",
        DialectId::DuckDb,
        "no `truncate` scalar in DuckDB; `trunc` is the numeric one",
    ),
    gap("QUOTE_IDENT", DialectId::DuckDb, "PostgreSQL-only builtin"),
    gap(
        "QUOTE_LITERAL",
        DialectId::DuckDb,
        "PostgreSQL-only builtin",
    ),
    gap(
        "JSON_EXTRACT_TEXT",
        DialectId::DuckDb,
        "DuckDB spells it `json_extract_string`",
    ),
    gap(
        "JSON_OBJECT_KEYS",
        DialectId::DuckDb,
        "DuckDB spells it `json_keys`",
    ),
    gap(
        "PERCENTILE_CONT",
        DialectId::DuckDb,
        "DuckDB spells the ordered-set aggregate `quantile_cont`",
    ),
    gap(
        "PERCENTILE_DISC",
        DialectId::DuckDb,
        "DuckDB spells the ordered-set aggregate `quantile_disc`",
    ),
    gap(
        "DATE_SUB",
        DialectId::DuckDb,
        "no `date_sub` in DuckDB; interval subtraction is infix `-`",
    ),
];
