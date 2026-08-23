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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Leg {
    /// The engine refuses the probe outright.
    Schema,
    /// The engine accepts the probe but reports a different output *type* than
    /// smelt inferred for it.
    Type,
    /// The engine accepts the probe and returns a different *answer*.
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

/// A one-position gap the engine surfaces only at execution: the dry run
/// accepts the query and running it does not.
const fn value_gap_at(
    name: &'static str,
    dialect: DialectId,
    position: Position,
    detail: &'static str,
) -> LedgerRow {
    LedgerRow {
        name,
        dialect,
        position: Some(position),
        leg: Leg::Value,
        verdict: Verdict::Gap {
            issue: "#171",
            detail,
        },
    }
}

/// A gap the engine surfaces in the *type* of the result: it accepts the probe
/// but reports a different output type than smelt inferred.
const fn type_gap(name: &'static str, dialect: DialectId, detail: &'static str) -> LedgerRow {
    LedgerRow {
        name,
        dialect,
        position: None,
        leg: Leg::Type,
        verdict: Verdict::Gap {
            issue: "#171",
            detail,
        },
    }
}

/// A gap the engine hides: it accepts the probe and computes a different
/// number. The dangerous class — neither the schema nor the type leg can see
/// it.
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
/// A `Schema` row also exempts the type and value legs: a probe the engine
/// refuses has neither a type nor a value to compare. The reverse is not true.
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
            // A `Schema` row exempts every downstream leg: a probe the engine
            // refuses has neither a type nor a value to compare. The reverse
            // does not hold.
            && (r.leg == leg || r.leg == Leg::Schema)
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
    //
    // Type-leg gaps: the engine accepts the probe, but smelt's inferred output
    // type disagrees with what the engine reports. These are inference holes,
    // not emission ones — and none of them is reachable by
    // `type_property_tests`, which generates from `core_functions()`, a
    // hand-maintained registry-blind table.
    type_gap(
        "DATE_ADD",
        DialectId::DuckDb,
        "`DATE_ADD(date, INTERVAL …)` infers Unknown(Dynamic); DuckDB returns TIMESTAMP",
    ),
    type_gap(
        "EXPLODE",
        DialectId::DuckDb,
        "unnesting an ARRAY<T> infers Unknown(Dynamic) rather than the element type T",
    ),
    type_gap(
        "UNNEST",
        DialectId::DuckDb,
        "unnesting an ARRAY<T> infers Unknown(Dynamic) rather than the element type T",
    ),
    // `FIRST` and `LAST` are lexed as keywords (`FIRST_KW`/`LAST_KW`, for
    // `NULLS FIRST` / `NULLS LAST`), so a call to either never parses as a
    // FUNCTION_CALL at all. In aggregate position that surfaces as an
    // Unknown type; in window position the select item does not even yield an
    // alias. The registry claims both are aggregates, so this is a
    // registry-versus-parser gap, and closing it is a contextual-keyword
    // change in `smelt-parser`, not a registry edit.
    type_gap(
        "FIRST",
        DialectId::DuckDb,
        "lexed as the `NULLS FIRST` keyword, so `FIRST(x)` never parses as a call",
    ),
    type_gap(
        "LAST",
        DialectId::DuckDb,
        "lexed as the `NULLS LAST` keyword, so `LAST(x)` never parses as a call",
    ),
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
    // Type-leg gaps. The same two inference families DuckDB surfaces, confirmed
    // independently on Spark — so neither is an engine quirk.
    type_gap(
        "EXPLODE",
        DialectId::SparkSql,
        "unnesting an ARRAY<T> infers Unknown(Dynamic) rather than the element type T",
    ),
    type_gap(
        "UNNEST",
        DialectId::SparkSql,
        "unnesting an ARRAY<T> infers Unknown(Dynamic) rather than the element type T",
    ),
    type_gap(
        "FIRST",
        DialectId::SparkSql,
        "lexed as the `NULLS FIRST` keyword, so `FIRST(x)` never parses as a call",
    ),
    type_gap(
        "LAST",
        DialectId::SparkSql,
        "lexed as the `NULLS LAST` keyword, so `LAST(x)` never parses as a call",
    ),
    //
    // ── BigQuery ─────────────────────────────────────────────────────────
    // Established by `bash scripts/bigquery-dialect-audit.sh` against the live
    // warehouse on 2026-08-24 — measured, not read from documentation. This is
    // the manual tier: the value leg executes rather than dry-runs, so it bills.
    gap("QUARTER", DialectId::BigQuery, "no `quarter`; GoogleSQL spells it `EXTRACT(QUARTER FROM d)`"),
    gap("QUOTE_IDENT", DialectId::BigQuery, "PostgreSQL-only builtin"),
    gap("QUOTE_LITERAL", DialectId::BigQuery, "PostgreSQL-only builtin"),
    gap("RANDOM", DialectId::BigQuery, "GoogleSQL spells it `RAND`"),
    gap("SPLIT_PART", DialectId::BigQuery, "no `split_part`; GoogleSQL has `SPLIT` returning an array"),
    gap("TO_CHAR", DialectId::BigQuery, "no `to_char`; GoogleSQL has `FORMAT_TIMESTAMP` / `FORMAT`"),
    gap("TO_SECONDS", DialectId::BigQuery, "no `to_seconds` in GoogleSQL"),
    gap("TRUNCATE", DialectId::BigQuery, "no `truncate` scalar in GoogleSQL"),
    gap("YEAR", DialectId::BigQuery, "no `year`; GoogleSQL spells it `EXTRACT(YEAR FROM d)`"),
    gap("POSITION", DialectId::BigQuery, "no `POSITION(x IN y)` form; GoogleSQL has `STRPOS(y, x)`"),
    gap("UNNEST", DialectId::BigQuery, "GoogleSQL allows `UNNEST` only in a FROM clause, never in a select list"),
    gap("ARG_MAX", DialectId::BigQuery, "GoogleSQL spells it `MAX_BY`"),
    gap("ARG_MIN", DialectId::BigQuery, "GoogleSQL spells it `MIN_BY`"),
    gap("GROUP_CONCAT", DialectId::BigQuery, "GoogleSQL spells it `STRING_AGG`"),
    gap("LISTAGG", DialectId::BigQuery, "GoogleSQL spells it `STRING_AGG`"),
    gap("MODE", DialectId::BigQuery, "no `mode`; GoogleSQL has `APPROX_TOP_COUNT`"),
    gap("REGR_SLOPE", DialectId::BigQuery, "no regression aggregates in GoogleSQL"),
    gap("FIRST", DialectId::BigQuery, "GoogleSQL's `FIRST` exists only inside a MATCH_RECOGNIZE MEASURES clause"),
    gap("LAST", DialectId::BigQuery, "GoogleSQL's `LAST` exists only inside a MATCH_RECOGNIZE MEASURES clause"),
    gap("PERCENTILE_CONT", DialectId::BigQuery, "GoogleSQL requires argument 2 to be a literal, so a column percentile is not expressible"),
    gap("PERCENTILE_DISC", DialectId::BigQuery, "GoogleSQL requires argument 2 to be a literal, so a column percentile is not expressible"),
    // `MEDIAN` lowers correctly in aggregate position; the window rewrite emits
    // `PERCENTILE_CONT ... OVER (... ORDER BY ...)`, and GoogleSQL forbids a
    // window ORDER BY on that analytic function. Scoped to the failing position.
    gap_at(
        "MEDIAN",
        DialectId::BigQuery,
        Position::Window,
        "the window rewrite emits PERCENTILE_CONT with a window ORDER BY, which GoogleSQL forbids",
    ),
    //
    // Type-leg gaps.
    type_gap(
        "SIGN",
        DialectId::BigQuery,
        "GoogleSQL's SIGN(FLOAT64) returns FLOAT64; smelt infers SmallInt, matching DuckDB's TINYINT",
    ),
    type_gap(
        "TRUNC",
        DialectId::BigQuery,
        "GoogleSQL's TRUNC always returns FLOAT64; smelt infers the argument's integer type",
    ),
    //
    // A second BigQuery family: names GoogleSQL has no function for at all.
    gap("AGE", DialectId::BigQuery, "no `age`; GoogleSQL expresses interval difference with `TIMESTAMP_DIFF`"),
    gap("DATE_PART", DialectId::BigQuery, "no `date_part`; GoogleSQL spells it `EXTRACT(part FROM d)`"),
    gap("DATE_TRUNC", DialectId::BigQuery, "GoogleSQL's argument order is `DATE_TRUNC(date, part)`, the reverse of DuckDB's"),
    gap("DAY", DialectId::BigQuery, "no `day`; GoogleSQL spells it `EXTRACT(DAY FROM d)`"),
    gap("DAYOFWEEK", DialectId::BigQuery, "no `dayofweek`; GoogleSQL spells it `EXTRACT(DAYOFWEEK FROM d)`"),
    gap("EXPLODE", DialectId::BigQuery, "GoogleSQL allows `UNNEST` only in a FROM clause, never in a select list"),
    gap("GLOB", DialectId::BigQuery, "no `GLOB` operator in GoogleSQL"),
    gap("ILIKE", DialectId::BigQuery, "no `ILIKE` operator; GoogleSQL case-folds with `LOWER(x) LIKE LOWER(p)`"),
    gap("JSON_ARRAY_LENGTH", DialectId::BigQuery, "no `json_array_length`; GoogleSQL has `ARRAY_LENGTH(JSON_QUERY_ARRAY(...))`"),
    gap("JSON_CONTAINS", DialectId::BigQuery, "no `json_contains` in GoogleSQL"),
    gap("JSON_EXTRACT_TEXT", DialectId::BigQuery, "GoogleSQL spells it `JSON_VALUE`"),
    gap("JSON_OBJECT_KEYS", DialectId::BigQuery, "no `json_object_keys` in GoogleSQL"),
    gap("LOG2", DialectId::BigQuery, "no `log2`; GoogleSQL spells it `LOG(x, 2)`"),
    gap("MAKE_DATE", DialectId::BigQuery, "no `make_date`; GoogleSQL spells it `DATE(y, m, d)`"),
    gap("MAKE_TIME", DialectId::BigQuery, "no `make_time`; GoogleSQL spells it `TIME(h, m, s)`"),
    gap("MAKE_TIMESTAMP", DialectId::BigQuery, "no `make_timestamp`; GoogleSQL spells it `DATETIME(...)`"),
    gap("MAKE_TIMESTAMPTZ", DialectId::BigQuery, "no `make_timestamptz` in GoogleSQL"),
    gap("MONTH", DialectId::BigQuery, "no `month`; GoogleSQL spells it `EXTRACT(MONTH FROM d)`"),
    gap("NOW", DialectId::BigQuery, "no `now()`; GoogleSQL spells it `CURRENT_TIMESTAMP()`"),
    gap("PI", DialectId::BigQuery, "no `pi()` in GoogleSQL"),
    //
    // The `%` finding is sharper than a missing name: smelt *does* lower
    // `a % b` to `MOD(a, b)` on GoogleSQL, but GoogleSQL's MOD accepts only
    // INT64 and NUMERIC. The lowering is therefore correct for integer
    // operands and a hard failure for floating-point ones — the same
    // operand-type-dependence that made `//` unlowerable.
    gap(
        "%",
        DialectId::BigQuery,
        "the MOD lowering only type-checks for INT64/NUMERIC operands; a float `%` is refused",
    ),
    //
    // Type-leg gaps.
    type_gap("DATE_ADD", DialectId::BigQuery, "`DATE_ADD(date, INTERVAL …)` infers Unknown(Dynamic); GoogleSQL returns DATE"),
    type_gap("DATE_SUB", DialectId::BigQuery, "`DATE_SUB(date, INTERVAL …)` infers Unknown(Dynamic); GoogleSQL returns DATE"),
    type_gap("MD5", DialectId::BigQuery, "GoogleSQL's MD5 returns BYTES; smelt infers Text, matching DuckDB's hex string"),
    //
    // Value-leg gaps: accepted, and computing something different.
    value_gap(
        "LOG",
        DialectId::BigQuery,
        "GoogleSQL's LOG(x) is the natural logarithm; DuckDB's is base 10 - the same silently wrong number Spark has",
    ),
    divergent(
        "CONCAT",
        DialectId::BigQuery,
        "GoogleSQL's CONCAT propagates NULL where DuckDB's treats it as the empty string. A NULL-propagation model difference, not a spelling one.",
    ),
    divergent(
        "CORR",
        DialectId::BigQuery,
        "GoogleSQL returns NULL for a degenerate correlation where DuckDB returns NaN.",
    ),
    divergent(
        "GREATEST",
        DialectId::BigQuery,
        "GoogleSQL returns NULL if any argument is NULL; DuckDB ignores NULL arguments. A NULL-propagation model difference.",
    ),
    divergent(
        "LEAST",
        DialectId::BigQuery,
        "GoogleSQL returns NULL if any argument is NULL; DuckDB ignores NULL arguments. A NULL-propagation model difference.",
    ),
    divergent(
        "MD5",
        DialectId::BigQuery,
        "GoogleSQL returns raw BYTES where DuckDB returns a hex string; the digests agree byte for byte.",
    ),
    divergent(
        "TO_JSON",
        DialectId::BigQuery,
        "GoogleSQL renders a SQL NULL as the JSON text `null`; DuckDB returns SQL NULL.",
    ),
    divergent(
        "DATE_ADD",
        DialectId::BigQuery,
        "GoogleSQL's DATE_ADD on a DATE stays a DATE; DuckDB widens to TIMESTAMP. Both name the same day.",
    ),
    //
    // Execution-time findings, reachable only by the value leg: the query is
    // accepted, then the warehouse refuses the data.
    // Dry-run-invisible: BigQuery plans the analytic form happily and only
    // refuses it on execution, so this is a value-leg row, not a schema one.
    value_gap_at(
        "APPROX_COUNT_DISTINCT",
        DialectId::BigQuery,
        Position::Window,
        "GoogleSQL has APPROX_COUNT_DISTINCT as an aggregate but not as an analytic function",
    ),
    divergent(
        "POWER",
        DialectId::BigQuery,
        "GoogleSQL raises on a negative base with a fractional exponent (POW(-2.5, -2.5)); DuckDB returns NaN. A loud failure, not a wrong number.",
    ),
    divergent(
        "**",
        DialectId::BigQuery,
        "lowers to POWER, and inherits its domain: GoogleSQL raises on a negative base with a fractional exponent where DuckDB returns NaN.",
    ),
    divergent(
        "^",
        DialectId::BigQuery,
        "lowers to POWER, and inherits its domain: GoogleSQL raises on a negative base with a fractional exponent where DuckDB returns NaN.",
    ),
    divergent(
        "ARRAY_AGG",
        DialectId::BigQuery,
        "a GoogleSQL ARRAY cannot hold a NULL element, so aggregating a NULL-bearing column raises; DuckDB keeps the NULL.",
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
