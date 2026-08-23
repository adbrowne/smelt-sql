//! The only hand-written per-function data in the audit.
//!
//! Most probes are derived: a parameter's `TypeConstraint` picks a fixture
//! column and `SyntaxForm` decides the spelling. This table covers the minority
//! where a *type-correct* argument is not a *meaningful* one — regex patterns,
//! date-part strings, JSON paths, format strings — plus the
//! `SyntaxForm::Special` entries, which by definition have no uniform shape to
//! derive.
//!
//! It supersedes `core_functions()` (`prop_helpers/generators.rs`, 85
//! hand-maintained registry-blind `FuncDesc` rows) **as the source of probe
//! shapes for the dialect audit**. `core_functions()` still drives the *type*
//! property sweep — a different suite with a different purpose, whose three
//! reachability tests in `type_property_tests.rs` depend on it — so unifying
//! the two generators is deliberately out of scope here.

/// A hand-written probe shape for one registry entry.
#[derive(Debug, Clone)]
pub struct Override {
    /// Canonical registry name.
    pub name: &'static str,
    /// Argument expressions, replacing the derived ones entirely.
    pub args: Option<&'static [&'static str]>,
    /// Full spelling template; `{0}`, `{1}`, … are the arguments.
    /// Required for every `SyntaxForm::Special` entry.
    pub spelling: Option<&'static str>,
    /// Probe the schema leg only, never the value leg, with a reason.
    ///
    /// Nondeterministic entries execute at different instants (`NOW`,
    /// `CURRENT_DATE`) or produce no stable value at all (`RANDOM`, `UUID`), so
    /// a cross-engine value comparison would report a divergence that says
    /// nothing about emission. Recorded here rather than silently skipped.
    pub schema_only: Option<&'static str>,
}

/// Shorthand for a row that only replaces arguments.
const fn args(name: &'static str, args: &'static [&'static str]) -> Override {
    Override {
        name,
        args: Some(args),
        spelling: None,
        schema_only: None,
    }
}

/// Shorthand for a spelling plus fixed arguments.
const fn spell_args(
    name: &'static str,
    spelling: &'static str,
    a: &'static [&'static str],
) -> Override {
    Override {
        name,
        args: Some(a),
        spelling: Some(spelling),
        schema_only: None,
    }
}

/// Shorthand for a schema-only (nondeterministic) entry.
const fn nondeterministic(name: &'static str, reason: &'static str) -> Override {
    Override {
        name,
        args: None,
        spelling: None,
        schema_only: Some(reason),
    }
}

pub fn overrides() -> &'static [Override] {
    OVERRIDES
}

static OVERRIDES: &[Override] = &[
    // ── Dedicated syntax (SyntaxForm::Special) ───────────────────────────
    spell_args("CAST", "CAST({0} AS BIGINT)", &["n_int"]),
    spell_args("BETWEEN", "{0} BETWEEN 1 AND 10", &["n_bigint"]),
    spell_args("IN", "{0} IN (1, 2, 3)", &["n_bigint"]),
    spell_args("EXISTS", "EXISTS (SELECT 1)", &[]),
    spell_args("IS_NULL", "{0} IS NULL", &["n_int"]),
    spell_args("IS_NOT_NULL", "{0} IS NOT NULL", &["n_int"]),
    spell_args("LIKE", "{0} LIKE 'a%'", &["s_text"]),
    spell_args("ILIKE", "{0} ILIKE 'A%'", &["s_text"]),
    spell_args("GLOB", "{0} GLOB 'a*'", &["s_text"]),
    spell_args("DATE_ADD", "DATE_ADD({0}, INTERVAL 1 DAY)", &["d_date"]),
    spell_args("DATE_SUB", "DATE_SUB({0}, INTERVAL 1 DAY)", &["d_date"]),
    // ── Type-correct is not meaningful ───────────────────────────────────
    args("DATE_TRUNC", &["'month'", "ts_ts"]),
    args("DATE_PART", &["'year'", "ts_ts"]),
    args("LPAD", &["s_text", "10", "'.'"]),
    args("RPAD", &["s_text", "10", "'.'"]),
    args("PERCENTILE_CONT", &["0.5", "n_double"]),
    args("PERCENTILE_DISC", &["0.5", "n_double"]),
    args("NTILE", &["4"]),
    args("TO_CHAR", &["ts_ts", "'%Y-%m-%d'"]),
    args("JSON_EXTRACT", &["j_json", "'$.k'"]),
    args("JSON_EXTRACT_STRING", &["j_json", "'$.k'"]),
    args("JSON_VALUE", &["j_json", "'$.k'"]),
    args("GET_JSON_OBJECT", &["j_json", "'$.k'"]),
    // ── Arity and argument-type corrections ──────────────────────────────
    // Much of the registry spells "arity not yet modelled" as
    // `Variadic(Any)`, and `TypeConstraint::Numeric` picks the widest numeric
    // column. Both are right for the type system and wrong for a probe, so
    // these rows name the arguments the function actually takes. Every one was
    // added because DuckDB — the reference engine — refused the derived form.
    args("BIT_AND", &["n_bigint"]),
    args("BIT_OR", &["n_bigint"]),
    args("BIT_XOR", &["n_bigint"]),
    args("CORR", &["n_double", "n_dec"]),
    args("COVAR_POP", &["n_double", "n_dec"]),
    args("COVAR_SAMP", &["n_double", "n_dec"]),
    args("REGR_SLOPE", &["n_double", "n_dec"]),
    args("DAY", &["d_date"]),
    args("DAYOFWEEK", &["d_date"]),
    args("MONTH", &["d_date"]),
    args("QUARTER", &["d_date"]),
    args("YEAR", &["d_date"]),
    args("EVERY", &["b_bool"]),
    args("POW", &["n_double", "2"]),
    args("REVERSE", &["s_text"]),
    args("TRANSLATE", &["s_text", "'a'", "'z'"]),
    args("JSON_CONTAINS", &["j_json", "'1'"]),
    args("JSON_OBJECT", &["'k'", "n_bigint"]),
    args("LISTAGG", &["s_text", "','"]),
    args("STRING_AGG", &["s_text", "','"]),
    args("MAKE_DATE", &["2026", "1", "2"]),
    args("MAKE_TIME", &["1", "2", "3.0"]),
    args("MAKE_TIMESTAMP", &["2026", "1", "2", "3", "4", "5.0"]),
    args("MAKE_TIMESTAMPTZ", &["2026", "1", "2", "3", "4", "5.0"]),
    // Domain-restricted maths. `n_double` carries a negative row on purpose
    // (a probe over only-positive numbers would not exercise sign handling),
    // so the entries whose domain excludes it take the all-positive column.
    args("LN", &["n_bigint"]),
    args("LOG", &["n_bigint"]),
    args("LOG10", &["n_bigint"]),
    args("LOG2", &["n_bigint"]),
    args("SQRT", &["n_bigint"]),
    args("ACOS", &["n_double / 100"]),
    args("ASIN", &["n_double / 100"]),
    // Keyword-shaped: no argument list at all — and nondeterministic, so the
    // value leg is skipped with a recorded reason rather than reporting a
    // divergence that says nothing about emission.
    Override {
        name: "CURRENT_TIMESTAMP",
        args: Some(&[]),
        spelling: Some("CURRENT_TIMESTAMP"),
        schema_only: Some("engines execute at different instants"),
    },
    Override {
        name: "CURRENT_DATE",
        args: Some(&[]),
        spelling: Some("CURRENT_DATE"),
        schema_only: Some("engines execute at different instants"),
    },
    Override {
        name: "RANDOM",
        args: Some(&[]),
        spelling: Some("RANDOM()"),
        schema_only: Some("no stable value: a different draw per engine and per call"),
    },
    // Dedicated argument syntax the registry models as a plain call.
    spell_args("EXTRACT", "EXTRACT(YEAR FROM {0})", &["ts_ts"]),
    spell_args("POSITION", "POSITION('a' IN {0})", &["s_text"]),
    // ── Nondeterministic: schema leg only ────────────────────────────────
    nondeterministic("NOW", "engines execute at different instants"),
];

/// The override row for `name`, if one exists.
pub fn find(name: &str) -> Option<&'static Override> {
    OVERRIDES.iter().find(|o| o.name == name)
}
