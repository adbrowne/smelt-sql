//! Known type divergences between smelt inference and actual databases.
//!
//! Each divergence records what smelt infers vs what DuckDB and Spark actually
//! return, giving a unified view across backends.  When proptest finds a mismatch
//! that is already registered here, the test passes instead of failing.
//! Unknown mismatches still fail and print the full SQL for debugging.

use smelt_types::DataType;

/// Explicit wildcard sentinel: matches any `Decimal` regardless of precision/scale,
/// wherever it appears as a divergence's type pattern (smelt/duckdb/spark/bigquery
/// side alike). Deliberately spelled with out-of-range field values
/// (`u8::MAX`/`u8::MAX`) so it can never collide with a real reported decimal
/// width. `Decimal { precision: 0, scale: 0 }` used to serve as this wildcard, but
/// that spelling collides with a real BigQuery value: a BigQuery query output
/// schema reports NUMERIC/BIGNUMERIC precision/scale as absent, and the BigQuery
/// oracle (`bigquery_oracle.rs`, `bigquery_type_to_smelt`) maps that absence to
/// `Decimal { precision: 0, scale: 0 }` as a "width not reported" sentinel — an
/// exact value that must compare normally, not wildcard-match. `Decimal { 0, 0 }`
/// is therefore an ordinary type from here on; only `ANY_DECIMAL` wildcards.
pub const ANY_DECIMAL: DataType = DataType::Decimal {
    precision: u8::MAX,
    scale: u8::MAX,
};

/// Why this divergence exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivergenceStatus {
    /// A bug in smelt's inference that we plan to fix.
    KnownBug,
    /// Intentional design choice in smelt.
    ByDesign,
    /// Database-specific behavior we can't fully model.
    BackendSpecific,
}

/// A registered divergence between smelt and backends.
///
/// Each record shows what smelt infers and what each backend actually returns.
/// `None` means no divergence for that backend (smelt matches, or untested).
/// `ANY_DECIMAL` acts as a wildcard matching any Decimal; `Decimal { precision: 0,
/// scale: 0 }` is an ordinary exact value (see its doc comment for why it is no
/// longer the wildcard).
#[derive(Debug)]
pub struct TypeDivergence {
    pub id: &'static str,
    pub description: &'static str,
    pub smelt_type: DataType,
    pub duckdb_type: Option<DataType>,
    pub spark_type: Option<DataType>,
    /// What BigQuery's query output schema reports for this pattern. `None`
    /// means no divergence recorded for BigQuery (smelt matches, or untested —
    /// entries are filled in only from a verified live probe, never guessed).
    pub bigquery_type: Option<DataType>,
    pub status: DivergenceStatus,
}

/// All known divergences.  Add new entries here when proptest surfaces expected mismatches.
pub fn known_divergences() -> Vec<TypeDivergence> {
    vec![
        // Names the single blanket compatibility rule in `type_comparison.rs`.
        // smelt models Text/Varchar(n)/Char(n) as one logical string type, so a
        // length/name difference from the backend is a designed leniency, not an
        // inference bug. `compare_types` returns Compatible for the string family
        // and cites this entry; it never reaches `find_divergence`, but the entry
        // exists so the leniency is named and greppable per the strictness rule.
        TypeDivergence {
            id: "text_varchar_compat",
            description: "Text/Varchar/Char — smelt has one logical string type; backends \
                distinguish VARCHAR(n)/CHAR(n)/TEXT. Authorises the string-family \
                Compatible verdict in type_comparison.rs.",
            smelt_type: DataType::Text,
            duckdb_type: Some(DataType::Varchar { max_length: None }),
            spark_type: Some(DataType::Varchar { max_length: None }),
            // BigQuery's STRING is the same unbounded-length family member.
            bigquery_type: Some(DataType::Varchar { max_length: None }),
            status: DivergenceStatus::ByDesign,
        },
        // Names the struct-field-naming blanket leniency in `type_comparison.rs`
        // (`compare_struct_fields`). DuckDB's own `ROW(...)` constructor leaves
        // fields anonymous (empty name); smelt names positional struct fields
        // v1, v2, ... for ergonomic dot-access (`infer_row_constructor_type`).
        // `compare_types` returns Compatible for this pattern and cites this
        // entry; it never reaches `find_divergence` (struct fields are compared
        // structurally, not via this registry's exact-type matching), but the
        // entry exists so the leniency is named and greppable.
        TypeDivergence {
            id: "row_constructor_field_naming",
            description: "ROW(...) — smelt names positional struct fields v1, v2, ...; \
                DuckDB leaves them anonymous (empty name). Authorises the struct \
                field-naming Compatible verdict in type_comparison.rs.",
            smelt_type: DataType::Struct(vec![("v1".to_string(), DataType::Integer)]),
            duckdb_type: Some(DataType::Struct(vec![(String::new(), DataType::Integer)])),
            spark_type: None,
            bigquery_type: None, // untested against BigQuery
            status: DivergenceStatus::ByDesign,
        },
        // verified: 2026-07-20 `SELECT SUM(x) FROM (SELECT CAST(1 AS INT) x)` and
        // the BIGINT variant — Spark's DESCRIBE QUERY reports `bigint` for both,
        // matching smelt.
        TypeDivergence {
            id: "sum_integer",
            description: "SUM(INTEGER/BIGINT) — smelt infers BigInt, DuckDB returns Decimal(38,0) (HUGEINT via Arrow)",
            smelt_type: DataType::BigInt,
            duckdb_type: Some(DataType::Decimal {
                precision: 38,
                scale: 0,
            }),
            spark_type: None, // Spark also returns BigInt, matches smelt
            bigquery_type: None,
            status: DivergenceStatus::BackendSpecific,
        },
        // verified: 2026-07-20 `SELECT CAST('a' AS STRING) || CAST('b' AS STRING)`
        // — Spark's DESCRIBE QUERY reports `string`.
        TypeDivergence {
            id: "string_concat",
            description: "|| operator — smelt infers Text, backends return Varchar/String",
            smelt_type: DataType::Text,
            duckdb_type: Some(DataType::Varchar { max_length: None }),
            spark_type: Some(DataType::Varchar { max_length: None }),
            bigquery_type: None,
            status: DivergenceStatus::ByDesign,
        },
        // verified: 2026-07-20 `SELECT UPPER('a')` — Spark's DESCRIBE QUERY
        // reports `string`.
        TypeDivergence {
            id: "string_functions",
            description: "UPPER/LOWER/etc — smelt infers Text, backends return Varchar/String",
            smelt_type: DataType::Text,
            duckdb_type: Some(DataType::Varchar { max_length: None }),
            spark_type: Some(DataType::Varchar { max_length: None }),
            bigquery_type: None,
            status: DivergenceStatus::ByDesign,
        },
        // verified: 2026-07-20 `SELECT CEIL(CAST(1.5 AS DOUBLE))` and the FLOOR
        // variant — Spark's DESCRIBE QUERY reports `bigint` for both.
        TypeDivergence {
            id: "ceil_floor_double",
            description: "CEIL/FLOOR(DOUBLE) — smelt returns Double (matches DuckDB), Spark returns BigInt",
            smelt_type: DataType::Double,
            duckdb_type: None,
            spark_type: Some(DataType::BigInt),
            bigquery_type: None,
            status: DivergenceStatus::BackendSpecific,
        },
        // verified: 2026-07-20 `SELECT AVG(x) FROM (SELECT CAST(1.5 AS
        // DECIMAL(10,2)) x)` — Spark's DESCRIBE QUERY reports `decimal(14,6)`,
        // matched by the wildcard.
        TypeDivergence {
            id: "avg_decimal",
            description:
                "AVG(DECIMAL) — smelt infers Double (matches DuckDB), Spark returns Decimal (varying precision)",
            smelt_type: DataType::Double,
            duckdb_type: None,
            // Wildcard: matches any Decimal precision/scale.
            spark_type: Some(ANY_DECIMAL),
            bigquery_type: None,
            status: DivergenceStatus::BackendSpecific,
        },
        // verified: 2026-07-20 `SELECT MEDIAN(x) FROM (SELECT CAST(1.5 AS
        // DECIMAL(10,2)) x)` — Spark's DESCRIBE QUERY reports `double`.
        TypeDivergence {
            id: "median_decimal",
            description:
                "MEDIAN(DECIMAL) — smelt infers Decimal, unchanged (matches DuckDB, which \
                preserves the input Decimal type). Spark's MEDIAN is implemented via \
                percentile_cont, which always returns DOUBLE regardless of input type.",
            // Wildcard: matches any smelt Decimal (any precision/scale MEDIAN was
            // called on).
            smelt_type: ANY_DECIMAL,
            duckdb_type: None,
            spark_type: Some(DataType::Double),
            bigquery_type: None,
            status: DivergenceStatus::BackendSpecific,
        },
        // PERCENTILE_CONT/PERCENTILE_DISC ordered-set aggregates: smelt's
        // BuiltinRegistry signature returns a fixed `Double`, because the
        // WITHIN GROUP clause's `ORDER BY` expression (where the real,
        // arg-dependent type lives) isn't yet exposed to type inference — there
        // is no AST accessor for it (`FunctionCall::arguments()` only sees the
        // fraction literal, e.g. `0.5`). DuckDB itself (probed directly)
        // preserves the sort column's type: `percentile_cont` interpolates like
        // `MEDIAN` (integer widens to Double, Decimal/Double preserved) and
        // `percentile_disc` always preserves the exact input type (it just
        // picks an existing row's value, no interpolation). These are smelt
        // bugs (`KnownBug`, same shape as `round_integer` below) rather than
        // legitimate backend differences; fixing requires threading the
        // WITHIN GROUP ORDER BY expression's type through
        // `try_registry_inference`, which is out of scope for the property-test
        // generator work that surfaced them.
        //
        // verified: 2026-07-20 `SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER
        // BY x) FROM (SELECT CAST(1.5 AS DECIMAL(10,2)) x)` (and the DISC
        // variant) — Spark's DESCRIBE QUERY reports `double` for both, which
        // matches smelt's registry-fixed Double, so there is no Spark
        // divergence even though the DuckDB one (and the underlying KnownBug)
        // stands.
        TypeDivergence {
            id: "percentile_ordered_set_decimal",
            description: "PERCENTILE_CONT/PERCENTILE_DISC(DECIMAL) WITHIN GROUP — smelt infers \
                Double (registry-fixed, doesn't see the WITHIN GROUP ORDER BY column type), \
                DuckDB preserves the input Decimal type for both functions.",
            smelt_type: DataType::Double,
            // Wildcard: matches any DuckDB Decimal.
            duckdb_type: Some(ANY_DECIMAL),
            spark_type: None,
            bigquery_type: None,
            status: DivergenceStatus::KnownBug,
        },
        // verified: 2026-07-20 `SELECT PERCENTILE_DISC(0.5) WITHIN GROUP (ORDER
        // BY x) FROM (SELECT CAST(1 AS INT) x)` — Spark's DESCRIBE QUERY
        // reports `double`, matching smelt's registry-fixed Double (unlike
        // DuckDB, which preserves Integer).
        TypeDivergence {
            id: "percentile_disc_integer",
            description: "PERCENTILE_DISC(INTEGER) WITHIN GROUP — smelt infers Double \
                (registry-fixed), DuckDB preserves Integer (percentile_disc never \
                interpolates; it returns an actual input value).",
            smelt_type: DataType::Double,
            duckdb_type: Some(DataType::Integer),
            spark_type: None,
            bigquery_type: None,
            status: DivergenceStatus::KnownBug,
        },
        // verified: 2026-07-20 `SELECT PERCENTILE_DISC(0.5) WITHIN GROUP (ORDER
        // BY x) FROM (SELECT CAST(1 AS BIGINT) x)` — Spark's DESCRIBE QUERY
        // reports `double`, matching smelt's registry-fixed Double (unlike
        // DuckDB, which preserves BigInt).
        TypeDivergence {
            id: "percentile_disc_bigint",
            description: "PERCENTILE_DISC(BIGINT) WITHIN GROUP — smelt infers Double \
                (registry-fixed), DuckDB preserves BigInt (percentile_disc never \
                interpolates; it returns an actual input value).",
            smelt_type: DataType::Double,
            duckdb_type: Some(DataType::BigInt),
            spark_type: None,
            bigquery_type: None,
            status: DivergenceStatus::KnownBug,
        },
        // verified: 2026-07-20 `SELECT SIGN(CAST(1.5 AS DOUBLE))` — Spark's
        // DESCRIBE QUERY reports `double`. Spark's SIGN is always DOUBLE
        // regardless of argument type (see sign_integer/sign_bigint/
        // sign_decimal below — all four confirmed to report `double`).
        TypeDivergence {
            id: "sign_double",
            description:
                "SIGN(DOUBLE) — smelt infers SmallInt (matches DuckDB TINYINT), Spark returns Double",
            smelt_type: DataType::SmallInt,
            duckdb_type: None,
            spark_type: Some(DataType::Double),
            bigquery_type: None,
            status: DivergenceStatus::BackendSpecific,
        },
        // verified: 2026-07-20 `SELECT SIGN(CAST(1 AS INT))` — Spark's DESCRIBE
        // QUERY reports `double`, not `int`. Corrected from a stale `Integer`
        // recording: Spark's SIGN always returns DOUBLE, it doesn't preserve
        // the argument type the way DuckDB's TINYINT-returning `sign` does.
        TypeDivergence {
            id: "sign_integer",
            description:
                "SIGN(INTEGER) — smelt infers SmallInt (matches DuckDB TINYINT), Spark always \
                returns Double regardless of argument type",
            smelt_type: DataType::SmallInt,
            duckdb_type: None,
            spark_type: Some(DataType::Double),
            bigquery_type: None,
            status: DivergenceStatus::BackendSpecific,
        },
        // verified: 2026-07-20 `SELECT SIGN(CAST(1 AS BIGINT))` — Spark's
        // DESCRIBE QUERY reports `double`, not `bigint`. Corrected from a stale
        // `BigInt` recording; see sign_integer above.
        TypeDivergence {
            id: "sign_bigint",
            description:
                "SIGN(BIGINT) — smelt infers SmallInt (matches DuckDB TINYINT), Spark always \
                returns Double regardless of argument type",
            smelt_type: DataType::SmallInt,
            duckdb_type: None,
            spark_type: Some(DataType::Double),
            bigquery_type: None,
            status: DivergenceStatus::BackendSpecific,
        },
        // verified: 2026-07-20 `SELECT SIGN(CAST(1.5 AS DECIMAL(10,2)))` —
        // Spark's DESCRIBE QUERY reports `double`, not a Decimal type.
        // Corrected from a stale Decimal-wildcard recording; see sign_integer
        // above.
        TypeDivergence {
            id: "sign_decimal",
            description:
                "SIGN(DECIMAL) — smelt infers SmallInt (matches DuckDB TINYINT), Spark always \
                returns Double regardless of argument type",
            smelt_type: DataType::SmallInt,
            duckdb_type: None,
            spark_type: Some(DataType::Double),
            bigquery_type: None,
            status: DivergenceStatus::BackendSpecific,
        },
        // verified: 2026-07-20 `SELECT CAST(1.5 AS FLOAT)` — Spark's DESCRIBE
        // QUERY reports `float`.
        TypeDivergence {
            id: "cast_float_as_double",
            description:
                "CAST(x AS FLOAT) — smelt normalizes FLOAT to DOUBLE, DuckDB and Spark both return FLOAT (4-byte)",
            smelt_type: DataType::Double,
            duckdb_type: Some(DataType::Float),
            spark_type: Some(DataType::Float),
            bigquery_type: None,
            status: DivergenceStatus::ByDesign,
        },
        // verified: 2026-07-20 `SELECT CAST('2024-01-02' AS DATE) -
        // CAST('2024-01-01' AS DATE)` — Spark's DESCRIBE QUERY reports
        // `interval day`, which maps to smelt's Interval.
        TypeDivergence {
            id: "date_minus_date",
            description: "DATE - DATE — smelt infers Interval (Spark-aligned, the portable \
                temporal difference); DuckDB returns BIGINT (a plain day count). Spark also \
                returns an interval, matching smelt.",
            smelt_type: DataType::Interval,
            duckdb_type: Some(DataType::BigInt),
            spark_type: None, // Spark returns Interval, matches smelt
            bigquery_type: None,
            status: DivergenceStatus::BackendSpecific,
        },
        // Decimal arithmetic model: smelt applies the portable, Spark-aligned
        // decimal growth formulas (spec §15) — multiplication p'=p1+p2+1,
        // s'=s1+s2; addition/subtraction/modulo p'=max(p1-s1,p2-s2)+max(s1,s2)+1;
        // ROUND keeps the input scale. DuckDB uses its native, physically-clamped
        // decimal arithmetic — multiplication clamps the result precision to the
        // storage-type boundary (BIGINT-backed DECIMAL, max 18 digits), ROUP(x)
        // reduces scale to 0, IFNULL widens precision to hold an integer literal,
        // and these differences propagate through nested arithmetic. On *raw*
        // (un-cast) SQL the two decimal models therefore disagree on precision
        // and/or scale across an open-ended family of expressions, so this is a
        // single named class divergence rather than one entry per (p,s) pair.
        //
        // Both operands are wildcards (`ANY_DECIMAL`): the
        // entry matches any Decimal-vs-Decimal pair. This is admissible — and not
        // a return to the removed blanket `is_decimal_compat` rule — because the
        // *exact* decimal correctness of smelt's inference is verified separately
        // and strictly by `tests/proptests/type_conformance_tests.rs`, which
        // cast-wraps smelt's inferred types and asserts DuckDB AND Spark reproduce
        // them with zero divergence. A smelt-vs-backend decimal difference on raw
        // SQL is expected; a cast-wrapped difference is a bug the conformance test
        // fails on. Verified behaviours (confirmed independently against each
        // engine): `CAST(99.99 AS DECIMAL(10,2)) * CAST(99.99 AS DECIMAL(10,2))` →
        // DuckDB DECIMAL(18,4) (smelt (21,4), diverges), Spark DECIMAL(21,4)
        // (matches smelt — multiplication genuinely is Spark-aligned);
        // `ROUND(CAST(99.99 AS DECIMAL(10,2)))` → DuckDB DECIMAL(10,0), Spark
        // DECIMAL(9,0) (smelt (10,2) — smelt's ROUND keeps input scale, neither
        // backend does, and the two backends don't even agree with each other);
        // `IFNULL(CAST(99.99 AS DECIMAL(10,2)), 0)` → DuckDB DECIMAL(12,2), Spark
        // DECIMAL(12,2) (smelt (10,2) — both backends widen precision to hold the
        // integer literal, contrary to the "Spark-aligned" assumption this entry
        // used to make about Spark).
        //
        // verified: 2026-07-20 re-ran all three probes above against live
        // Spark — `... * ...` → decimal(21,4) (matches smelt), `ROUND(...)` →
        // decimal(9,0) (diverges), `IFNULL(..., 0)` → decimal(12,2) (diverges).
        // No change from the prior verification; wildcard still covers both
        // divergent cases.
        TypeDivergence {
            id: "decimal_arithmetic_model",
            description: "smelt uses Spark-aligned decimal growth (spec §15) for multiplication, \
                but neither DuckDB nor Spark match smelt's raw-SQL precision/scale for ROUND or \
                IFNULL/COALESCE decimal widening. On raw SQL the three disagree on Decimal \
                precision/scale across ROUND, IFNULL, and nested arithmetic. Exact decimal \
                correctness is verified by the cast-wrap conformance oracle \
                (type_conformance_tests.rs); this entry tolerates the raw-SQL Decimal-vs-Decimal \
                difference only, against either backend.",
            // Wildcard: matches any smelt Decimal.
            smelt_type: ANY_DECIMAL,
            // Wildcard: matches any DuckDB Decimal.
            duckdb_type: Some(ANY_DECIMAL),
            // Wildcard: matches any Spark Decimal. Previously `None` on the
            // (disproven) assumption that Spark's decimal growth always matches
            // smelt's; ROUND and IFNULL both diverge on raw SQL against Spark too.
            spark_type: Some(ANY_DECIMAL),
            bigquery_type: None,
            status: DivergenceStatus::BackendSpecific,
        },
        // verified: 2026-07-20 `SELECT ROUND(CAST(1 AS INT))` — Spark's
        // DESCRIBE QUERY reports `int`, not `double`. Corrected from a stale
        // `None` recording (which assumed Spark matched smelt's Double): Spark
        // preserves the integer type here, the same as DuckDB, so this is the
        // same KnownBug surfacing against both backends rather than a
        // Spark-only match.
        TypeDivergence {
            id: "round_integer",
            description: "ROUND(INTEGER) — smelt's ROUND signature is Double→Double only; \
                integer inputs are upcast to Double before rounding, so smelt infers Double \
                while DuckDB and Spark both preserve the integer type. Propagates to \
                downstream arithmetic on ROUND outputs (Double+Double in smelt vs \
                Integer+Integer in DuckDB/Spark). Fixing requires a polymorphic ROUND \
                signature.",
            smelt_type: DataType::Double,
            duckdb_type: Some(DataType::Integer),
            spark_type: Some(DataType::Integer),
            bigquery_type: None,
            status: DivergenceStatus::KnownBug,
        },
        // verified: 2026-07-20 `SELECT CAST('2024-01-01' AS DATE) + INTERVAL
        // '1' DAY` — Spark's DESCRIBE QUERY reports `date`.
        TypeDivergence {
            id: "date_plus_interval",
            description: "DATE + INTERVAL / DATE - INTERVAL — smelt infers Timestamp (matches \
                DuckDB, which always promotes to TIMESTAMP). Spark keeps the result as DATE when \
                the interval is day/year-month granularity (only promotes to TIMESTAMP once the \
                interval carries a time-of-day component). smelt's Interval type doesn't \
                distinguish granularity, so it can't conditionally match Spark here.",
            smelt_type: DataType::Timestamp {
                with_timezone: false,
            },
            duckdb_type: None,
            spark_type: Some(DataType::Date),
            bigquery_type: None,
            status: DivergenceStatus::BackendSpecific,
        },
        // verified: 2026-07-20 `SELECT COALESCE(CAST(1 AS FLOAT), CAST(2 AS
        // DOUBLE))` and the IFNULL variant — Spark's DESCRIBE QUERY reports
        // `double` for both.
        TypeDivergence {
            id: "float_promotes_to_double_spark",
            description: "FLOAT combined with another numeric type in COALESCE/IFNULL/GREATEST/ \
                LEAST/MOD — smelt keeps FLOAT (matches DuckDB's promotion, which also keeps \
                FLOAT). Spark instead widens to DOUBLE whenever FLOAT is promoted against any \
                other numeric type in these functions.",
            smelt_type: DataType::Float,
            duckdb_type: None,
            spark_type: Some(DataType::Double),
            bigquery_type: None,
            status: DivergenceStatus::BackendSpecific,
        },
        // verified: 2026-08-24 (dialect_audit sweep, live Spark 4.0.0)
        // `SELECT CURRENT_TIMESTAMP` and `SELECT NOW()` — Spark's DESCRIBE
        // QUERY reports `timestamp` for both, which is Spark's session-local
        // type and carries no zone. smelt infers a zone-aware timestamp,
        // matching DuckDB's TIMESTAMPTZ.
        TypeDivergence {
            id: "spark_timestamp_is_zone_naive",
            description: "Spark's TIMESTAMP is session-local and reports no zone, so any \
                expression smelt types as zone-aware (CURRENT_TIMESTAMP, NOW) reports as a \
                plain timestamp there.",
            smelt_type: DataType::Timestamp {
                with_timezone: true,
            },
            duckdb_type: None,
            spark_type: Some(DataType::Timestamp {
                with_timezone: false,
            }),
            bigquery_type: None,
            status: DivergenceStatus::BackendSpecific,
        },
        // verified: 2026-07-20 `SELECT ROW_NUMBER() OVER (ORDER BY x) FROM
        // (SELECT 1 x)` and the RANK/DENSE_RANK variants — Spark's DESCRIBE
        // QUERY reports `int` for all three.
        TypeDivergence {
            id: "row_number_rank_family",
            description: "ROW_NUMBER/RANK/DENSE_RANK — smelt infers BigInt (matches DuckDB \
                BIGINT), Spark returns INT (Integer) for these ranking window functions.",
            smelt_type: DataType::BigInt,
            duckdb_type: None,
            spark_type: Some(DataType::Integer),
            bigquery_type: None,
            status: DivergenceStatus::BackendSpecific,
        },
        // BigQuery's query output schema (the only surface the BigQuery oracle
        // can probe — see `bigquery_oracle.rs`) reports precision/scale as
        // absent for both NUMERIC and BIGNUMERIC columns; there is no field to
        // read a width from. `bigquery_type_to_smelt` surfaces that absence as
        // the sentinel `Decimal { precision: 0, scale: 0 }` — "a Decimal was
        // reported, but BigQuery didn't say how wide". smelt legitimately
        // infers many different Decimal widths depending on the expression, so
        // the smelt side is the `ANY_DECIMAL` wildcard; the BigQuery side is
        // the exact sentinel, not a wildcard, so a leg that reports a real
        // width (see the next paragraph) is NOT absorbed here. What this entry
        // does still check strictly: smelt infers a Decimal at all — Double or
        // BigInt vs Decimal remain real mismatches on the BigQuery leg. If
        // BigQuery ever begins reporting precision/scale on query output
        // schemas, this entry stops matching (the actual type would carry a
        // real width, not the `0,0` sentinel) and the leg fails loudly, which
        // is the intended outcome — the oracle mapping and this entry would
        // both need to be revisited.
        TypeDivergence {
            id: "bigquery_decimal_width_unreported",
            description: "Decimal width — smelt infers a specific precision/scale, but \
                BigQuery's query output schema reports NUMERIC/BIGNUMERIC precision/scale as \
                absent, which the oracle surfaces as the sentinel Decimal{0,0} (\"width not \
                reported\"), not a real value to compare against.",
            smelt_type: ANY_DECIMAL,
            duckdb_type: None,
            spark_type: None,
            bigquery_type: Some(DataType::Decimal {
                precision: 0,
                scale: 0,
            }),
            status: DivergenceStatus::BackendSpecific,
        },
        // verified: 2026-08-17 `SELECT [CAST('2024-01-01 12:00:00' AS TIMESTAMP)]`
        // — BigQuery's dry-run schema reports TIMESTAMP, which is its *absolute
        // instant* type, while smelt infers a naive timestamp.
        //
        // This is a dialect collision over one keyword, not an arithmetic bug.
        // In SQL-standard/DuckDB/PostgreSQL spelling, bare `TIMESTAMP` is the
        // naive wall-clock type and the zone-aware one is `TIMESTAMPTZ`.
        // BigQuery inverts the pair: its zone-aware type is spelled `TIMESTAMP`
        // and its naive one is spelled `DATETIME`. smelt's `CAST(x AS
        // TIMESTAMP)` inference reads the keyword with the standard meaning,
        // because type inference has no notion of which dialect the model will
        // be lowered to, so on BigQuery it lands on the wrong side of the pair.
        //
        // Registered rather than fixed: making CAST target-type resolution
        // dialect-aware is a real change to type inference's inputs (it would
        // have to be threaded a target dialect), not a local correction. Until
        // then this entry keeps the leg honest about a difference that is
        // genuinely there. Note it is *not* symmetric — smelt inferring
        // zone-aware where BigQuery reports naive is NOT registered and still
        // fails, because that direction has no dialect explanation.
        TypeDivergence {
            id: "bigquery_timestamp_keyword_is_zone_aware",
            description: "CAST(x AS TIMESTAMP) — smelt reads TIMESTAMP with its \
                SQL-standard/DuckDB meaning (naive wall clock); BigQuery spells its \
                zone-aware absolute-instant type TIMESTAMP and its naive type DATETIME, \
                so the same keyword denotes the other member of the pair.",
            smelt_type: DataType::Timestamp {
                with_timezone: false,
            },
            duckdb_type: None,
            spark_type: None,
            bigquery_type: Some(DataType::Timestamp {
                with_timezone: true,
            }),
            status: DivergenceStatus::BackendSpecific,
        },
        // verified: 2026-08-17 — the single divergence class a 512-case sweep
        // against the live warehouse produced (18 occurrences out of 285
        // columns compared; nothing else was unregistered).
        //
        // BigQuery has exactly one integer type, INT64, which the query output
        // schema reports under its legacy name INTEGER and the oracle maps to
        // `BigInt` (see `bigquery_oracle.rs` — reporting it as `Integer` would
        // assert a 32-bit width the warehouse does not have). So every smelt
        // integer inference, whatever its width, meets `BigInt` on this leg:
        // integer *width* is simply not observable against BigQuery, in the
        // same way decimal width is not (see
        // `bigquery_decimal_width_unreported`). Width conformance for integers
        // is carried by the DuckDB and Spark legs, which do distinguish.
        //
        // Deliberately narrow: only `Integer` is registered, because only
        // `Integer` was observed. `SmallInt` or `TinyInt` meeting `BigInt` would
        // still fail the leg — that would mean the generators had started
        // producing a shape this sweep never covered, which is worth being told
        // about rather than absorbing in advance.
        TypeDivergence {
            id: "bigquery_single_integer_width",
            description: "Integer width — BigQuery has exactly one integer type (INT64, \
                reported under the legacy name INTEGER), so a smelt Integer inference meets \
                BigInt on every BigQuery column. Integer width is unobservable on this leg; \
                the DuckDB and Spark legs carry that conformance.",
            smelt_type: DataType::Integer,
            duckdb_type: None,
            spark_type: None,
            bigquery_type: Some(DataType::BigInt),
            status: DivergenceStatus::BackendSpecific,
        },
    ]
}

/// Check if a (smelt_type, actual_type) pair matches a known divergence for the given backend.
/// Returns the divergence if found.
pub fn find_divergence<'a>(
    smelt: &DataType,
    actual: &DataType,
    backend: &str,
    divergences: &'a [TypeDivergence],
) -> Option<&'a TypeDivergence> {
    // Unwrap one level of Array (e.g. ARRAY_AGG(expr) results) so element-level
    // divergences (like decimal growth) are still recognized under wrapping —
    // the registry has no separate Array-of-Decimal entries to maintain.
    if let (DataType::Array(smelt_elem), DataType::Array(actual_elem)) = (smelt, actual) {
        return find_divergence(smelt_elem, actual_elem, backend, divergences);
    }
    divergences.iter().find(|d| {
        types_match(&d.smelt_type, smelt) && {
            let expected = match backend {
                "duckdb" => d.duckdb_type.as_ref(),
                "spark" => d.spark_type.as_ref(),
                "bigquery" => d.bigquery_type.as_ref(),
                _ => None,
            };
            expected.is_some_and(|t| types_match(t, actual))
        }
    })
}

/// Check if a divergence's type pattern matches an actual type.
/// `ANY_DECIMAL` acts as a wildcard for any Decimal; every other pattern
/// (including `Decimal { precision: 0, scale: 0 }`) compares exactly.
fn types_match(pattern: &DataType, actual: &DataType) -> bool {
    if pattern == actual {
        return true;
    }
    *pattern == ANY_DECIMAL && matches!(actual, DataType::Decimal { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_sum_integer_divergence_duckdb() {
        let divs = known_divergences();
        let found = find_divergence(
            &DataType::BigInt,
            &DataType::Decimal {
                precision: 38,
                scale: 0,
            },
            "duckdb",
            &divs,
        );
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "sum_integer");
    }

    #[test]
    fn finds_ceil_floor_double_divergence_spark() {
        let divs = known_divergences();
        let found = find_divergence(&DataType::Double, &DataType::BigInt, "spark", &divs);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "ceil_floor_double");
    }

    #[test]
    fn finds_decimal_arithmetic_model_divergence_spark() {
        // Regression: a local soak run (PROPTEST_CASES beyond CI's 256) caught
        // `IFNULL(DECIMAL(10,2), 0)` diverging against Spark (widens to
        // DECIMAL(12,2), same as DuckDB) even though decimal_arithmetic_model
        // previously assumed Spark always matches smelt's raw-SQL decimal type.
        let divs = known_divergences();
        let found = find_divergence(
            &DataType::Decimal {
                precision: 10,
                scale: 2,
            },
            &DataType::Decimal {
                precision: 12,
                scale: 2,
            },
            "spark",
            &divs,
        );
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "decimal_arithmetic_model");
    }

    #[test]
    fn finds_decimal_arithmetic_model_divergence_under_array_wrapping_duckdb() {
        // Regression: a soak run caught `ARRAY_AGG(dec_col * dec_col)` diverging
        // against DuckDB — the existing decimal_arithmetic_model wildcard covers
        // bare Decimal-vs-Decimal, but ARRAY_AGG wraps the result in Array, and
        // the matcher didn't unwrap it before comparing.
        let divs = known_divergences();
        let found = find_divergence(
            &DataType::Array(Box::new(DataType::Decimal {
                precision: 21,
                scale: 4,
            })),
            &DataType::Array(Box::new(DataType::Decimal {
                precision: 18,
                scale: 4,
            })),
            "duckdb",
            &divs,
        );
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "decimal_arithmetic_model");
    }

    #[test]
    fn finds_median_decimal_divergence_spark() {
        // Regression: a soak run at 10000 cases caught `MEDIAN(DECIMAL(10,2))`
        // diverging against Spark — smelt/DuckDB preserve the Decimal type,
        // Spark's percentile_cont-backed MEDIAN always returns DOUBLE.
        let divs = known_divergences();
        let found = find_divergence(
            &DataType::Decimal {
                precision: 10,
                scale: 2,
            },
            &DataType::Double,
            "spark",
            &divs,
        );
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "median_decimal");
    }

    #[test]
    fn finds_date_plus_interval_divergence_spark() {
        // Regression: a soak run at 3000 cases (CI runs 256) caught
        // `CAST('2024-01-01' AS DATE) + CAST('1 day' AS INTERVAL)` diverging
        // against Spark — smelt/DuckDB return Timestamp, Spark returns Date for
        // a day-granularity interval.
        let divs = known_divergences();
        let found = find_divergence(
            &DataType::Timestamp {
                with_timezone: false,
            },
            &DataType::Date,
            "spark",
            &divs,
        );
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "date_plus_interval");
    }

    #[test]
    fn finds_float_promotes_to_double_divergence_spark() {
        // Regression: the dedicated numeric-function property test caught
        // COALESCE/IFNULL/GREATEST/LEAST(SMALLINT, FLOAT) diverging against
        // Spark after promote_types-based promotion was fixed to correctly
        // keep FLOAT (matching DuckDB) — Spark widens to DOUBLE instead.
        let divs = known_divergences();
        let found = find_divergence(&DataType::Float, &DataType::Double, "spark", &divs);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "float_promotes_to_double_spark");
    }

    #[test]
    fn finds_cast_float_as_double_divergence_spark() {
        // Regression: CI caught `CAST(DECIMAL AS FLOAT)` failing against the
        // Spark oracle because cast_float_as_double only listed a duckdb_type,
        // even though Spark also returns FLOAT (not DOUBLE) here.
        let divs = known_divergences();
        let found = find_divergence(&DataType::Double, &DataType::Float, "spark", &divs);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "cast_float_as_double");
    }

    #[test]
    fn finds_row_number_rank_family_divergence_spark() {
        let divs = known_divergences();
        let found = find_divergence(&DataType::BigInt, &DataType::Integer, "spark", &divs);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "row_number_rank_family");
    }

    #[test]
    fn backend_none_prevents_match() {
        let divs = known_divergences();
        // sum_integer has spark_type: None — should not match spark
        let found = find_divergence(
            &DataType::BigInt,
            &DataType::Decimal {
                precision: 38,
                scale: 0,
            },
            "spark",
            &divs,
        );
        assert!(found.is_none());
    }

    #[test]
    fn decimal_arithmetic_model_matches_any_decimal_pair_duckdb() {
        let divs = known_divergences();
        // Multiplication precision growth.
        let found = find_divergence(
            &DataType::Decimal {
                precision: 21,
                scale: 4,
            },
            &DataType::Decimal {
                precision: 18,
                scale: 4,
            },
            "duckdb",
            &divs,
        );
        assert_eq!(found.unwrap().id, "decimal_arithmetic_model");
        // ROUND scale reduction (different scale) is also covered.
        let found2 = find_divergence(
            &DataType::Decimal {
                precision: 10,
                scale: 2,
            },
            &DataType::Decimal {
                precision: 10,
                scale: 0,
            },
            "duckdb",
            &divs,
        );
        assert_eq!(found2.unwrap().id, "decimal_arithmetic_model");
    }

    #[test]
    fn decimal_wildcard_does_not_match_non_decimal_smelt() {
        let divs = known_divergences();
        // smelt Boolean vs DuckDB Decimal must NOT be absorbed by the decimal
        // wildcard (smelt side is not a Decimal, and no registered divergence
        // pairs Boolean with Decimal).
        let found = find_divergence(
            &DataType::Boolean,
            &DataType::Decimal {
                precision: 10,
                scale: 2,
            },
            "duckdb",
            &divs,
        );
        assert!(found.is_none());
    }

    #[test]
    fn finds_percentile_ordered_set_decimal_divergence_duckdb() {
        // smelt Double vs DuckDB Decimal IS a registered divergence now
        // (`percentile_ordered_set_decimal`, added for PERCENTILE_CONT/
        // PERCENTILE_DISC's registry-fixed Double signature vs DuckDB's
        // sort-column-type-preserving WITHIN GROUP behavior) — this
        // supersedes the old blanket assumption in
        // `decimal_wildcard_does_not_match_non_decimal_smelt` above that no
        // such pairing existed.
        let divs = known_divergences();
        let found = find_divergence(
            &DataType::Double,
            &DataType::Decimal {
                precision: 10,
                scale: 2,
            },
            "duckdb",
            &divs,
        );
        assert_eq!(found.unwrap().id, "percentile_ordered_set_decimal");
    }

    #[test]
    fn finds_percentile_disc_integer_divergence_duckdb() {
        let divs = known_divergences();
        let found = find_divergence(&DataType::Double, &DataType::Integer, "duckdb", &divs);
        assert_eq!(found.unwrap().id, "percentile_disc_integer");
    }

    #[test]
    fn finds_percentile_disc_bigint_divergence_duckdb() {
        let divs = known_divergences();
        let found = find_divergence(&DataType::Double, &DataType::BigInt, "duckdb", &divs);
        assert_eq!(found.unwrap().id, "percentile_disc_bigint");
    }

    #[test]
    fn returns_none_for_unknown() {
        let divs = known_divergences();
        let found = find_divergence(&DataType::Boolean, &DataType::Date, "duckdb", &divs);
        assert!(found.is_none());
    }

    #[test]
    fn wildcard_decimal_matches_any_precision() {
        let divs = known_divergences();
        let found = find_divergence(
            &DataType::Double,
            &DataType::Decimal {
                precision: 14,
                scale: 6,
            },
            "spark",
            &divs,
        );
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "avg_decimal");
    }

    #[test]
    fn any_decimal_matches_any_precision_and_scale() {
        assert!(types_match(
            &ANY_DECIMAL,
            &DataType::Decimal {
                precision: 10,
                scale: 2,
            }
        ));
        assert!(types_match(
            &ANY_DECIMAL,
            &DataType::Decimal {
                precision: 0,
                scale: 0,
            }
        ));
    }

    #[test]
    fn decimal_zero_zero_no_longer_wildcards_as_a_pattern() {
        // Decimal{0,0} used to be the wildcard sentinel; now it's an ordinary
        // exact value (BigQuery's real "width not reported" signal), so it
        // must not absorb a differently-shaped Decimal.
        assert!(!types_match(
            &DataType::Decimal {
                precision: 0,
                scale: 0
            },
            &DataType::Decimal {
                precision: 10,
                scale: 2
            }
        ));
    }

    #[test]
    fn finds_bigquery_decimal_width_unreported_divergence() {
        let divs = known_divergences();
        let found = find_divergence(
            &DataType::Decimal {
                precision: 10,
                scale: 2,
            },
            &DataType::Decimal {
                precision: 0,
                scale: 0,
            },
            "bigquery",
            &divs,
        );
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "bigquery_decimal_width_unreported");
    }

    #[test]
    fn bigquery_reported_width_is_not_absorbed() {
        // A BigQuery leg that ever reports an actual width must NOT be
        // swallowed by the "width unreported" entry — that entry's
        // bigquery_type is the exact sentinel Decimal{0,0}, not a wildcard.
        let divs = known_divergences();
        let found = find_divergence(
            &DataType::Decimal {
                precision: 10,
                scale: 2,
            },
            &DataType::Decimal {
                precision: 38,
                scale: 2,
            },
            "bigquery",
            &divs,
        );
        assert!(found.is_none());
    }

    #[test]
    fn bigquery_double_vs_decimal_sentinel_is_still_a_mismatch() {
        let divs = known_divergences();
        let found = find_divergence(
            &DataType::Double,
            &DataType::Decimal {
                precision: 0,
                scale: 0,
            },
            "bigquery",
            &divs,
        );
        assert!(found.is_none());
    }

    #[test]
    fn bigquery_arm_is_reachable_and_none_when_unset() {
        let divs = known_divergences();
        // sum_integer has bigquery_type: None — should not match bigquery.
        let found = find_divergence(
            &DataType::BigInt,
            &DataType::Decimal {
                precision: 38,
                scale: 0,
            },
            "bigquery",
            &divs,
        );
        assert!(found.is_none());
    }

    #[test]
    fn finds_bigquery_decimal_width_unreported_under_array_wrapping() {
        let divs = known_divergences();
        let found = find_divergence(
            &DataType::Array(Box::new(DataType::Decimal {
                precision: 10,
                scale: 2,
            })),
            &DataType::Array(Box::new(DataType::Decimal {
                precision: 0,
                scale: 0,
            })),
            "bigquery",
            &divs,
        );
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "bigquery_decimal_width_unreported");
    }
}
