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
    /// The `Emission::Conditional` arm this row scopes to. `None` means
    /// every arm — the right default for a non-conditional entry, and for a
    /// conditional one whose gap really does affect every arm alike.
    /// Scoping matters the same way `position` does: an arm-specific
    /// `Unsupported` must not silently exempt a sibling arm the engine
    /// actually gets right.
    pub arm: Option<usize>,
    /// Which leg this row exempts.
    pub leg: Leg,
    pub verdict: Verdict,
}

/// A gap the engine surfaces by refusing the probe.
const fn gap(
    name: &'static str,
    dialect: DialectId,
    issue: &'static str,
    detail: &'static str,
) -> LedgerRow {
    LedgerRow {
        name,
        dialect,
        position: None,
        arm: None,
        leg: Leg::Schema,
        verdict: Verdict::Gap { issue, detail },
    }
}

/// A gap the engine surfaces in the *type* of the result: it accepts the probe
/// but reports a different output type than smelt inferred.
const fn type_gap(
    name: &'static str,
    dialect: DialectId,
    issue: &'static str,
    detail: &'static str,
) -> LedgerRow {
    LedgerRow {
        name,
        dialect,
        position: None,
        arm: None,
        leg: Leg::Type,
        verdict: Verdict::Gap { issue, detail },
    }
}

/// A gap the engine hides: it accepts the probe and computes a different
/// number. The dangerous class — neither the schema nor the type leg can see
/// it.
const fn value_gap(
    name: &'static str,
    dialect: DialectId,
    issue: &'static str,
    detail: &'static str,
) -> LedgerRow {
    LedgerRow {
        name,
        dialect,
        position: None,
        arm: None,
        leg: Leg::Value,
        verdict: Verdict::Gap { issue, detail },
    }
}

/// A gap that applies to one position only.
const fn gap_at(
    name: &'static str,
    dialect: DialectId,
    position: Position,
    issue: &'static str,
    detail: &'static str,
) -> LedgerRow {
    LedgerRow {
        name,
        dialect,
        position: Some(position),
        arm: None,
        leg: Leg::Schema,
        verdict: Verdict::Gap { issue, detail },
    }
}

/// A gap that applies to one position only, reachable only by the value leg:
/// the engine accepts the probe (including a dry run) and only execution
/// refuses it.
const fn value_gap_at(
    name: &'static str,
    dialect: DialectId,
    position: Position,
    issue: &'static str,
    detail: &'static str,
) -> LedgerRow {
    LedgerRow {
        name,
        dialect,
        position: Some(position),
        arm: None,
        leg: Leg::Value,
        verdict: Verdict::Gap { issue, detail },
    }
}

const fn divergent(name: &'static str, dialect: DialectId, reason: &'static str) -> LedgerRow {
    LedgerRow {
        name,
        dialect,
        position: None,
        arm: None,
        leg: Leg::Value,
        verdict: Verdict::Divergent { reason },
    }
}

/// Scope an existing row to one `Emission::Conditional` arm — every other
/// field is unaffected; only the arm narrows.
const fn arm_at(row: LedgerRow, arm: usize) -> LedgerRow {
    LedgerRow {
        arm: Some(arm),
        ..row
    }
}

/// Every accepted `(entry, dialect)` divergence.
pub fn dialect_divergences() -> &'static [LedgerRow] {
    ROWS
}

/// Whether `r` covers `(name, dialect, position, arm)` on `leg` — the pure
/// predicate [`find`] applies to the real [`ROWS`], pulled out so it can be
/// exercised against a synthetic row directly rather than requiring a real
/// entry to carry an arm before the matching logic can be tested at all (no
/// production entry is `Conditional` yet).
///
/// A `Schema` row also exempts the type and value legs: a probe the engine
/// refuses has neither a type nor a value to compare. The reverse is not true.
///
/// `arm` mirrors `position`'s own None-means-every convention: a row with no
/// arm of its own (`r.arm == None`) exempts every arm, including a probe
/// that names one; a row scoped to arm `k` exempts only a probe naming that
/// same arm.
fn row_matches(
    r: &LedgerRow,
    name: &str,
    dialect: DialectId,
    position: Position,
    arm: Option<usize>,
    leg: Leg,
) -> bool {
    r.name == name
        && r.dialect == dialect
        && r.position.map(|p| p == position).unwrap_or(true)
        && r.arm.map(|a| Some(a) == arm).unwrap_or(true)
        // A `Schema` row exempts every downstream leg: a probe the engine
        // refuses has neither a type nor a value to compare. The reverse
        // does not hold.
        && (r.leg == leg || r.leg == Leg::Schema)
}

/// The row covering `(name, dialect, position, arm)` on `leg`, if one exists.
pub fn find(
    name: &str,
    dialect: DialectId,
    position: Position,
    arm: Option<usize>,
    leg: Leg,
) -> Option<&'static LedgerRow> {
    ROWS.iter()
        .find(|r| row_matches(r, name, dialect, position, arm, leg))
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
    // Type-leg gaps: the engine accepts the probe, but smelt's inferred output
    // type disagrees with what the engine reports. These are inference holes,
    // not emission ones — and none of them is reachable by
    // `type_property_tests`, which generates from `core_functions()`, a
    // hand-maintained registry-blind table.
    //
    // `DATE_ADD`/`DATE_SUB` specifically: the registry's `Concrete(Timestamp)`
    // return type (corrected to match `binary.rs` and the live engine —
    // `docs/outcomes/20260904-dialect-emission-vocabulary` phase 4) is never
    // consulted, because neither name has a `SqlFunction` enum variant, so
    // `infer_function_type` returns `None` before `try_registry_inference`
    // ever runs. Not the contingency's assumed unquoted-`INTERVAL`-literal
    // gap — that literal already infers `Interval` correctly on its own
    // (measured directly) — a distinct, deeper registry-wiring gap. See
    // `phases/04-summary.md`.
    type_gap(
        "DATE_ADD",
        DialectId::DuckDb,
        "#176", "`DATE_ADD` has no `SqlFunction` enum variant, so `infer_function_type` never \
                 reaches the registry; infers Unknown(Dynamic) while DuckDB returns TIMESTAMP",
    ),
    type_gap(
        "DATE_SUB",
        DialectId::DuckDb,
        "#176", "`DATE_SUB` has no `SqlFunction` enum variant, so `infer_function_type` never \
                 reaches the registry; infers Unknown(Dynamic) while DuckDB returns TIMESTAMP",
    ),
    type_gap(
        "EXPLODE",
        DialectId::DuckDb,
        "#176", "unnesting an ARRAY<T> infers Unknown(Dynamic) rather than the element type T",
    ),
    type_gap(
        "UNNEST",
        DialectId::DuckDb,
        "#176", "unnesting an ARRAY<T> infers Unknown(Dynamic) rather than the element type T",
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
        "#175", "lexed as the `NULLS FIRST` keyword, so `FIRST(x)` never parses as a call",
    ),
    type_gap(
        "LAST",
        DialectId::DuckDb,
        "#175", "lexed as the `NULLS LAST` keyword, so `LAST(x)` never parses as a call",
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
    gap("DATE_ADD", DialectId::SparkSql, "#176", "Spark's `date_add(date, days)` takes an integer, not an INTERVAL"),
    // Same registry-wiring gap as `DATE_SUB` on DuckDB (phase 4): no
    // `SqlFunction` enum variant, so `infer_function_type` never reaches the
    // registry and infers Unknown(Dynamic). Surfaced for the first time here
    // because this phase gave `DATE_SUB` its first Spark emission verdict
    // (`Emission::Template("{0} - {1}")`), which made its type leg run at
    // all — same bail-out the plan named in advance. Spark's own `{0} -
    // {1}` on a DATE/INTERVAL pair returns DATE, not TIMESTAMP (measured
    // live 2026-09-06: `DATE '2026-01-02' - INTERVAL '5' DAY` = DATE
    // `2025-12-28`), unlike DuckDB's TIMESTAMP-widening subtraction.
    type_gap(
        "DATE_SUB",
        DialectId::SparkSql,
        "#176", "`DATE_SUB` has no `SqlFunction` enum variant, so `infer_function_type` never \
                 reaches the registry; infers Unknown(Dynamic) while Spark returns DATE",
    ),
    //
    // Value-leg divergence: same name, same shape, permanent semantic
    // difference for a non-array JSON argument. Not a schema gap — see the
    // `JSON_ARRAY_LENGTH` `Emission::Native` registry comment
    // (docs/outcomes/20260904-dialect-emission-vocabulary phase 8, verified
    // live 2026-09-06): DuckDB's `json_array_length('{"k": 1}')` = `0`;
    // Spark's = `NULL`.
    divergent(
        "JSON_ARRAY_LENGTH",
        DialectId::SparkSql,
        "Spark returns NULL for `json_array_length` on a non-array JSON value where DuckDB returns 0.",
    ),
    //
    // Type-leg gaps. The same two inference families DuckDB surfaces, confirmed
    // independently on Spark — so neither is an engine quirk.
    type_gap(
        "EXPLODE",
        DialectId::SparkSql,
        "#176", "unnesting an ARRAY<T> infers Unknown(Dynamic) rather than the element type T",
    ),
    type_gap(
        "UNNEST",
        DialectId::SparkSql,
        "#176", "unnesting an ARRAY<T> infers Unknown(Dynamic) rather than the element type T",
    ),
    type_gap(
        "FIRST",
        DialectId::SparkSql,
        "#175", "lexed as the `NULLS FIRST` keyword, so `FIRST(x)` never parses as a call",
    ),
    type_gap(
        "LAST",
        DialectId::SparkSql,
        "#175", "lexed as the `NULLS LAST` keyword, so `LAST(x)` never parses as a call",
    ),
    //
    // ── BigQuery ─────────────────────────────────────────────────────────
    // Established by `bash scripts/bigquery-dialect-audit.sh` against the live
    // warehouse on 2026-08-24 — measured, not read from documentation. This is
    // the manual tier: the value leg executes rather than dry-runs, so it bills.
    gap("QUARTER", DialectId::BigQuery, "#179", "no `quarter`; GoogleSQL spells it `EXTRACT(QUARTER FROM d)`"),
    gap("QUOTE_IDENT", DialectId::BigQuery, "#179", "PostgreSQL-only builtin"),
    gap("QUOTE_LITERAL", DialectId::BigQuery, "#179", "PostgreSQL-only builtin"),
    gap("SPLIT_PART", DialectId::BigQuery, "#179", "no `split_part`; GoogleSQL has `SPLIT` returning an array"),
    gap("TO_CHAR", DialectId::BigQuery, "#179", "no `to_char`; GoogleSQL has `FORMAT_TIMESTAMP` / `FORMAT`"),
    gap("TO_SECONDS", DialectId::BigQuery, "#179", "no `to_seconds` in GoogleSQL"),
    gap("YEAR", DialectId::BigQuery, "#179", "no `year`; GoogleSQL spells it `EXTRACT(YEAR FROM d)`"),
    gap("POSITION", DialectId::BigQuery, "#179", "no `POSITION(x IN y)` form; GoogleSQL has `STRPOS(y, x)`"),
    gap("UNNEST", DialectId::BigQuery, "#179", "GoogleSQL allows `UNNEST` only in a FROM clause, never in a select list"),
    gap("MODE", DialectId::BigQuery, "#179", "no `mode`; GoogleSQL has `APPROX_TOP_COUNT`"),
    gap("REGR_SLOPE", DialectId::BigQuery, "#179", "no regression aggregates in GoogleSQL"),
    gap("FIRST", DialectId::BigQuery, "#179", "GoogleSQL's `FIRST` exists only inside a MATCH_RECOGNIZE MEASURES clause"),
    gap("LAST", DialectId::BigQuery, "#179", "GoogleSQL's `LAST` exists only inside a MATCH_RECOGNIZE MEASURES clause"),
    // `MEDIAN` lowers correctly in aggregate position and over a whole-partition
    // window; a running window has no exact GoogleSQL form at all, because the
    // `PERCENTILE_CONT` lowering forbids a window `ORDER BY`, and the registry
    // refuses it rather than emitting a rewrite that would fail at the
    // warehouse. Scoped to the failing position.
    gap_at(
        "MEDIAN",
        DialectId::BigQuery,
        Position::Window,
        "#179",
        "the window rewrite would emit PERCENTILE_CONT with a window ORDER BY, which \
         GoogleSQL forbids; only a window covering the whole partition has an exact \
         GoogleSQL form",
    ),
    //
    // Type-leg gaps.
    type_gap(
        "SIGN",
        DialectId::BigQuery,
        "#179", "GoogleSQL's SIGN(FLOAT64) returns FLOAT64; smelt infers SmallInt, matching DuckDB's TINYINT",
    ),
    type_gap(
        "TRUNC",
        DialectId::BigQuery,
        "#179", "GoogleSQL's TRUNC always returns FLOAT64; smelt infers the argument's integer type",
    ),
    //
    // A second BigQuery family: names GoogleSQL has no function for at all.
    gap("AGE", DialectId::BigQuery, "#179", "no `age`; GoogleSQL expresses interval difference with `TIMESTAMP_DIFF`"),
    gap("DATE_PART", DialectId::BigQuery, "#179", "no `date_part`; GoogleSQL spells it `EXTRACT(part FROM d)`"),
    gap("DATE_TRUNC", DialectId::BigQuery, "#179", "GoogleSQL's argument order is `DATE_TRUNC(date, part)`, the reverse of DuckDB's"),
    gap("DAY", DialectId::BigQuery, "#179", "no `day`; GoogleSQL spells it `EXTRACT(DAY FROM d)`"),
    gap("DAYOFWEEK", DialectId::BigQuery, "#179", "no `dayofweek`; GoogleSQL spells it `EXTRACT(DAYOFWEEK FROM d)`"),
    gap("EXPLODE", DialectId::BigQuery, "#179", "GoogleSQL allows `UNNEST` only in a FROM clause, never in a select list"),
    gap("GLOB", DialectId::BigQuery, "#179", "no `GLOB` operator in GoogleSQL"),
    gap("ILIKE", DialectId::BigQuery, "#179", "no `ILIKE` operator; GoogleSQL case-folds with `LOWER(x) LIKE LOWER(p)`"),
    gap("JSON_ARRAY_LENGTH", DialectId::BigQuery, "#179", "no `json_array_length`; GoogleSQL has `ARRAY_LENGTH(JSON_QUERY_ARRAY(...))`"),
    gap("JSON_CONTAINS", DialectId::BigQuery, "#179", "no `json_contains` in GoogleSQL"),
    gap("JSON_OBJECT_KEYS", DialectId::BigQuery, "#179", "no `json_object_keys` in GoogleSQL"),
    gap("LOG2", DialectId::BigQuery, "#179", "no `log2`; GoogleSQL spells it `LOG(x, 2)`"),
    gap("MAKE_TIMESTAMPTZ", DialectId::BigQuery, "#179", "no `make_timestamptz` in GoogleSQL"),
    gap("MONTH", DialectId::BigQuery, "#179", "no `month`; GoogleSQL spells it `EXTRACT(MONTH FROM d)`"),
    gap("PI", DialectId::BigQuery, "#179", "no `pi()` in GoogleSQL"),
    //
    // The `%` finding is sharper than a missing name: smelt *does* lower
    // `a % b` to `MOD(a, b)` on GoogleSQL, but GoogleSQL's MOD accepts only
    // INT64 and NUMERIC. The lowering is therefore correct for integer
    // operands and a hard failure for floating-point ones — the same
    // operand-type-dependence that made `//` unlowerable.
    gap(
        "%",
        DialectId::BigQuery,
        "#173", "the MOD lowering only type-checks for INT64/NUMERIC operands; a float `%` is refused",
    ),
    //
    // Type-leg gaps.
    type_gap("DATE_ADD", DialectId::BigQuery, "#176", "`DATE_ADD(date, INTERVAL …)` infers Unknown(Dynamic); GoogleSQL returns DATE"),
    type_gap("DATE_SUB", DialectId::BigQuery, "#176", "`DATE_SUB(date, INTERVAL …)` infers Unknown(Dynamic); GoogleSQL returns DATE"),
    type_gap("MD5", DialectId::BigQuery, "#179", "GoogleSQL's MD5 returns BYTES; smelt infers Text, matching DuckDB's hex string"),
    //
    // Value-leg gaps: accepted, and computing something different.
    value_gap(
        "LOG",
        DialectId::BigQuery,
        "#174", "GoogleSQL's LOG(x) is the natural logarithm; DuckDB's is base 10 - the same silently wrong number Spark has",
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
    // GoogleSQL's dry run *accepts*
    // `APPROX_COUNT_DISTINCT(x) OVER (PARTITION BY g)` — the schema leg alone
    // cannot see this gap — but execution refuses it outright, even over a
    // partition-only window (measured live 2026-08-27). A whole-partition
    // window restructures around a grouped CTE; only the running case remains
    // a gap.
    value_gap_at(
        "APPROX_COUNT_DISTINCT",
        DialectId::BigQuery,
        Position::Window,
        "#179",
        "GoogleSQL has APPROX_COUNT_DISTINCT as an aggregate but no running-window form \
         of it; only a window covering the whole partition can be restructured around a \
         grouped CTE. GoogleSQL's dry run accepts the analytic form; only execution refuses \
         it.",
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
    // Renamed correctly, but the *shape* still differs — each found only after
    // the rename landed, because until then the name itself was missing.
    // GoogleSQL's `MAX_BY`/`MIN_BY` have no analytic form at all — refused
    // even over a partition-only window (measured live 2026-08-27). A
    // whole-partition window now restructures around a grouped CTE; only the
    // running case remains a gap.
    gap_at(
        "ARG_MAX",
        DialectId::BigQuery,
        Position::Window,
        "#179",
        "GoogleSQL's MAX_BY has no analytic form, even over a partition-only window; only \
         a window covering the whole partition can be restructured around a grouped CTE",
    ),
    gap_at(
        "ARG_MIN",
        DialectId::BigQuery,
        Position::Window,
        "#179",
        "GoogleSQL's MIN_BY has no analytic form, even over a partition-only window; only \
         a window covering the whole partition can be restructured around a grouped CTE",
    ),
    // GoogleSQL requires an `OVER` clause and rejects `WITHIN GROUP` outright,
    // so the aggregate position now restructures the other way — the
    // `FROM`/`WHERE` move into a CTE that computes the value as an analytic
    // column over the grouping keys (`RestructureId::AnalyticToCte`) — and a
    // whole-partition window is rewritten in place to the two-argument
    // analytic spelling (`RewriteId::WithinGroupToAnalytic`). Only the
    // running case, which GoogleSQL forbids a window `ORDER BY` on, remains
    // a gap.
    gap_at(
        "PERCENTILE_CONT",
        DialectId::BigQuery,
        Position::Window,
        "#179",
        "GoogleSQL has it as an analytic function only, and forbids a window ORDER BY on it",
    ),
    gap_at(
        "PERCENTILE_DISC",
        DialectId::BigQuery,
        Position::Window,
        "#179",
        "GoogleSQL has it as an analytic function only, and forbids a window ORDER BY on it",
    ),
    type_gap(
        "TRUNCATE",
        DialectId::BigQuery,
        "#179", "renames to TRUNC, which returns FLOAT64; smelt infers the argument's integer type",
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
    divergent(
        "AGE",
        DialectId::SparkSql,
        "Spark renders a zero day-time interval as `0 00:00:00.000000000`; DuckDB renders it `0 secs`. Same interval, different textual representation.",
    ),
    divergent(
        "TO_SECONDS",
        DialectId::SparkSql,
        "Spark renders `make_interval`'s output as `1.5 seconds`; DuckDB renders the equivalent interval `1.500000000 secs`. Same interval, different textual representation.",
    ),
    divergent(
        "DATE_SUB",
        DialectId::SparkSql,
        "Spark's `{0} - {1}` on a DATE/INTERVAL pair stays a DATE; DuckDB widens to TIMESTAMP. Both name the same day — the same type_gap this phase registered for the schema leg.",
    ),
    divergent(
        "JSON_OBJECT_KEYS",
        DialectId::SparkSql,
        "Spark renders a TEXT[] as JSON array syntax (`[\"k\"]`); DuckDB renders it its own array literal syntax (`[k]`). Same elements, different textual representation.",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Test 8: a row scoped to one arm does not exempt a sibling arm of the
    /// same entry.
    #[test]
    fn a_ledger_row_scoped_to_an_arm_does_not_exempt_another_arm() {
        let row = arm_at(
            gap("TEST_ARM_LEDGER", DialectId::SparkSql, "#0", "test row"),
            0,
        );
        assert!(
            row_matches(
                &row,
                "TEST_ARM_LEDGER",
                DialectId::SparkSql,
                Position::Scalar,
                Some(0),
                Leg::Schema,
            ),
            "sanity: the row must match its own arm"
        );
        assert!(
            !row_matches(
                &row,
                "TEST_ARM_LEDGER",
                DialectId::SparkSql,
                Position::Scalar,
                Some(1),
                Leg::Schema,
            ),
            "a row scoped to arm 0 must not exempt arm 1"
        );
        // An unscoped query (`arm: None`) does not implicitly widen a
        // scoped row into matching every arm either — only a row with no
        // arm of its own does that.
        assert!(!row_matches(
            &row,
            "TEST_ARM_LEDGER",
            DialectId::SparkSql,
            Position::Scalar,
            None,
            Leg::Schema,
        ));
    }
}
