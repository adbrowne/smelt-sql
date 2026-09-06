//! Built-in registry rows: remaining.
//!
//! Data only — every row is handed to the single `BuiltinRegistry` table
//! constructed in [`super`].

use super::super::{
    ConditionalArm, Emission, ExprKind, OperandClass, Position, RestructureId, RewriteId,
    SettledEmission, SigParam, Signature, TypeConstraint, TypeExpr,
};
use super::{tp, var};
use crate::{DataType, DialectId};

pub(super) fn register(insert: &mut dyn FnMut(Signature)) {
    // ─── Function-registry consolidation: remaining recognised built-ins ─────
    //
    // Every name recognised by `SqlFunction::from_name` must resolve here so
    // the registry is the single authoritative home for recognition,
    // classification (`kind`), and — for migrated functions — typing. The
    // consistency gate `every_recognized_function_is_registry_backed`
    // (smelt-db integration tests) enforces this. Argument shapes here are
    // deliberately permissive (`Any`-variadic) for functions whose typing
    // still lives in the hand-written match; migrating a function tightens
    // both its parameter constraints and its return type to match the legacy
    // arm exactly.
    let any_args = || {
        vec![SigParam::Variadic(Box::new(SigParam::Concrete(
            TypeConstraint::Any,
        )))]
    };

    // Extended statistical / distribution aggregates → Double.
    for name in ["CORR", "COVAR_POP", "COVAR_SAMP", "REGR_SLOPE"] {
        insert(
            Signature::new(
                name,
                vec![],
                any_args(),
                TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
            )
            .with_kind(ExprKind::Agg),
        );
    }

    // Deliberately NOT renamed to DuckDB's `quantile_cont`/`quantile_disc`.
    // DuckDB has both spellings with *different shapes*: `percentile_disc(f)
    // WITHIN GROUP (ORDER BY x)` is the ordered-set aggregate, and
    // `quantile_disc(x, f)` is a plain two-argument aggregate. A blanket rename
    // turns the first into a parser error ("Unknown ordered aggregate
    // QUANTILE_DISC") — caught by `proptests::type_conformance_tests`, which
    // generates the WITHIN GROUP form. Closing the BigQuery gap here needs a
    // shape-aware rewrite, not a rename.
    for name in ["PERCENTILE_CONT", "PERCENTILE_DISC"] {
        insert(
            Signature::new(
                name,
                vec![],
                any_args(),
                TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
            )
            .with_kind(ExprKind::Agg)
            .with_emission(&[
                // DuckDB and Spark have the ordered-set form only as an
                // aggregate — no window form — so a whole-partition window
                // call restructures around a synthesised CTE
                // (`RestructureId::WindowToCte`); a running window has no
                // correct CTE form and is refused
                // (`docs/specs/multi_backend.md` §"Statement-level
                // lowering"). The two window positions are stated together —
                // lookup never falls between them.
                (
                    DialectId::DuckDb,
                    Position::WholePartitionWindow,
                    Emission::Restructure(RestructureId::WindowToCte),
                ),
                (
                    DialectId::DuckDb,
                    Position::Window,
                    Emission::Unsupported {
                        reason: "DuckDB has the ordered-set aggregate but no running-window \
                                 form of it; only a window covering the whole partition can be \
                                 restructured around a grouped CTE",
                    },
                ),
                (
                    DialectId::SparkSql,
                    Position::WholePartitionWindow,
                    Emission::Restructure(RestructureId::WindowToCte),
                ),
                (
                    DialectId::SparkSql,
                    Position::Window,
                    Emission::Unsupported {
                        reason: "Spark has the ordered-set aggregate but no running-window \
                                 form of it; only a window covering the whole partition can be \
                                 restructured around a grouped CTE",
                    },
                ),
                // GoogleSQL requires an `OVER` clause and rejects `WITHIN
                // GROUP` outright, so a call under `GROUP BY` restructures
                // the other way: the `FROM`/`WHERE` move into a CTE that
                // computes the value as an analytic column over the grouping
                // keys (`RestructureId::AnalyticToCte`).
                (
                    DialectId::BigQuery,
                    Position::Aggregate,
                    Emission::Restructure(RestructureId::AnalyticToCte),
                ),
                // GoogleSQL accepts a partition-only `OVER` clause natively,
                // but only in its two-argument analytic spelling — `WITHIN
                // GROUP` under an `OVER` clause is a syntax error there
                // (measured live 2026-08-27). The whole-partition window is
                // rewritten to that spelling in place; a running window
                // still forbids a window `ORDER BY` and is refused rather
                // than lowered.
                (
                    DialectId::BigQuery,
                    Position::WholePartitionWindow,
                    Emission::Rewrite(RewriteId::WithinGroupToAnalytic),
                ),
                (
                    DialectId::BigQuery,
                    Position::Window,
                    Emission::Unsupported {
                        reason: "GoogleSQL's PERCENTILE_CONT/PERCENTILE_DISC forbid a \
                                 window ORDER BY; only a window covering the whole \
                                 partition is accepted",
                    },
                ),
            ]),
        );
    }
    // Boolean aggregate.
    insert(
        Signature::new(
            "EVERY",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
        )
        .with_kind(ExprKind::Agg)
        .with_emission(&[
            (
                DialectId::DuckDb,
                Position::Any,
                Emission::Rename("BOOL_AND"),
            ),
            (
                DialectId::BigQuery,
                Position::Any,
                Emission::Rename("LOGICAL_AND"),
            ),
        ]),
    );
    // Text-returning aggregate.
    insert(
        Signature::new(
            "GROUP_CONCAT",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .with_kind(ExprKind::Agg)
        .with_emission(&[
            // GoogleSQL spells it `STRING_AGG`. Verified live 2026-08-24.
            (
                DialectId::BigQuery,
                Position::Any,
                Emission::Rename("STRING_AGG"),
            ),
            // Spark has no `group_concat`/`string_agg`; the equivalent is
            // `concat_ws(sep, collect_list(x))`, which reorders and wraps
            // the argument rather than renaming the call, and whose arity
            // (bare value vs. value+separator) a fixed-arity template can't
            // express while this signature's typing is still the permissive
            // variadic `any_args()` shape. Verified live 2026-09-06 (phase 8):
            // no `group_concat` routine resolves.
            (
                DialectId::SparkSql,
                Position::Any,
                Emission::Unsupported {
                    reason: "no `group_concat`/`string_agg`; Spark spells it \
                             `concat_ws(sep, collect_list(x))`, a shape change no rename or \
                             fixed-arity template over this variadic signature can express",
                },
            ),
        ]),
    );
    // First-argument identity aggregates (typing stays in the exception list).
    for name in ["FIRST", "LAST", "MODE"] {
        insert(
            Signature::new(
                name,
                vec![tp("T", TypeConstraint::Any)],
                vec![var("T")],
                TypeExpr::Var("T".into()),
            )
            .with_kind(ExprKind::Agg),
        );
    }

    // Lifted out of the loop below so each can carry its own emission verdict,
    // both verified live on 2026-08-24.
    insert(
        Signature::new(
            "RANDOM",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        // GoogleSQL spells it `RAND`.
        .with_emission(&[(DialectId::BigQuery, Position::Any, Emission::Rename("RAND"))]),
    );
    insert(
        Signature::new(
            "TRUNCATE",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        // Neither DuckDB nor GoogleSQL has `truncate`; both spell the numeric
        // truncation `TRUNC`.
        .with_emission(&[
            (DialectId::DuckDb, Position::Any, Emission::Rename("TRUNC")),
            (
                DialectId::BigQuery,
                Position::Any,
                Emission::Rename("TRUNC"),
            ),
            // Spark's own `TRUNC` is date-only (`Position::Any` here, per phase
            // 7, is `Conditional` and already refuses a numeric first argument
            // with `DATATYPE_MISMATCH`); no numeric truncation routine exists
            // under any name. Verified live 2026-09-06 (phase 8):
            // `trunc(3.14159, 2)` fails, requiring a DATE first argument.
            (
                DialectId::SparkSql,
                Position::Any,
                Emission::Unsupported {
                    reason: "no numeric truncation routine in Spark under any name; its own \
                             `trunc` is date-only",
                },
            ),
        ]),
    );

    // Extended math / trig scalars → Double.
    for name in ["ACOS", "ASIN", "POW", "CEILING"] {
        insert(Signature::new(
            name,
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        ));
    }
    // Lifted out of the loop above so it can carry its own emission verdict.
    // Spark's `TRUNC` is temporal-only (`trunc(date, fmt)`); there is no
    // numeric `TRUNC` on Spark. Verified live 2026-09-06:
    // `trunc(DATE'2026-09-06', 'MM')` = 2026-09-01, while `trunc(3.14159, 2)`
    // fails analysis (`the first parameter requires the "DATE" type`).
    insert(
        Signature::new(
            "TRUNC",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_emission(&[
            (
                DialectId::SparkSql,
                Position::Any,
                Emission::Conditional(&[
                    // Spark's temporal `TRUNC` is the two-argument `trunc(date,
                    // fmt)` form; the arity guard keeps a probe for this arm a
                    // real, engine-accepted call rather than the single-argument
                    // shape the (numeric) `otherwise` arm below actually uses.
                    ConditionalArm {
                        arity: Some(2),
                        classes: &[(0, OperandClass::Temporal), (1, OperandClass::String)],
                        verdict: SettledEmission::Native,
                    },
                    ConditionalArm {
                        arity: None,
                        classes: &[],
                        verdict: SettledEmission::Unsupported {
                            reason: "Spark's TRUNC is temporal-only; there is no numeric TRUNC",
                        },
                    },
                ]),
            ),
            // DuckDB's `TRUNC` is numeric-only (verified live 2026-09-06:
            // `trunc(DATE '2026-09-06', 'MM')` and `trunc(<timestamp>)` both
            // fail analysis — no candidate overload takes a temporal
            // argument at any arity). Declared here, rather than left to
            // reach the engine as a raw binder error, and so the audit's own
            // DuckDB-as-reference run has a real verdict to settle the
            // Spark arm's probe against instead of a harness gap.
            (
                DialectId::DuckDb,
                Position::Any,
                Emission::Conditional(&[
                    ConditionalArm {
                        arity: Some(2),
                        classes: &[(0, OperandClass::Temporal), (1, OperandClass::String)],
                        verdict: SettledEmission::Unsupported {
                            reason: "DuckDB's TRUNC is numeric-only; there is no temporal TRUNC",
                        },
                    },
                    ConditionalArm {
                        arity: None,
                        classes: &[],
                        verdict: SettledEmission::Native,
                    },
                ]),
            ),
        ]),
    );
    // Extended text scalars → Text.
    for name in ["REVERSE", "TRANSLATE"] {
        insert(Signature::new(
            name,
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        ));
    }
    // Lifted out of the loop above so each can carry its own emission
    // verdict: DuckDB 1.5.x has none of these four scalars (`Catalog Error:
    // … does not exist`, measured live 2026-09-06), and none has a
    // placeholder-expressible equivalent — `TO_CHAR`'s format string is not
    // `strftime`'s. `docs/outcomes/20260904-dialect-emission-vocabulary`
    // phase 4.
    insert(
        Signature::new(
            "INITCAP",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .with_emission(&[(
            DialectId::DuckDb,
            Position::Any,
            Emission::Unsupported {
                reason: "DuckDB has no `initcap`; the closest is a manual UPPER/LOWER split",
            },
        )]),
    );
    insert(
        Signature::new(
            "TO_CHAR",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .with_emission(&[(
            DialectId::DuckDb,
            Position::Any,
            Emission::Unsupported {
                reason: "DuckDB has no `to_char`; `strftime` is the temporal half of it, not \
                         a placeholder-expressible general formatter",
            },
        )]),
    );
    insert(
        Signature::new(
            "QUOTE_IDENT",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .with_emission(&[
            (
                DialectId::DuckDb,
                Position::Any,
                Emission::Unsupported {
                    reason:
                        "a builtin from another SQL dialect entirely, with no DuckDB equivalent",
                },
            ),
            // Verified live 2026-09-06 (phase 8): no `quote_ident` routine on Spark.
            (
                DialectId::SparkSql,
                Position::Any,
                Emission::Unsupported {
                    reason: "a builtin from another SQL dialect entirely, with no Spark equivalent",
                },
            ),
        ]),
    );
    insert(
        Signature::new(
            "QUOTE_LITERAL",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .with_emission(&[
            (
                DialectId::DuckDb,
                Position::Any,
                Emission::Unsupported {
                    reason:
                        "a builtin from another SQL dialect entirely, with no DuckDB equivalent",
                },
            ),
            // Verified live 2026-09-06 (phase 8): no `quote_literal` routine on Spark.
            (
                DialectId::SparkSql,
                Position::Any,
                Emission::Unsupported {
                    reason: "a builtin from another SQL dialect entirely, with no Spark equivalent",
                },
            ),
        ]),
    );
    // 1-based string search position → BigInt.
    insert(Signature::new(
        "POSITION",
        vec![],
        any_args(),
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
    ));
    // Date-part extraction scalars → BigInt.
    for name in ["DAY", "MONTH", "QUARTER", "YEAR"] {
        insert(Signature::new(
            name,
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        ));
    }
    // Lifted out of the loop above so it can carry its own emission verdict.
    // Spark numbers the week from Sunday=1; DuckDB from Sunday=0. Verified
    // live 2026-09-06: Spark's `dayofweek(DATE'2026-09-06')` (a Sunday) = 1,
    // DuckDB's `dayofweek` for the same date = 0.
    insert(
        Signature::new(
            "DAYOFWEEK",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        .with_emission(&[(
            DialectId::SparkSql,
            Position::Any,
            Emission::Template("DAYOFWEEK({0}) - 1"),
        )]),
    );
    // Temporal constructors.
    insert(
        Signature::new(
            "MAKE_TIME",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Time)),
        )
        .with_emission(&[
            // GoogleSQL's `TIME(h, m, s)`. Verified live 2026-08-24.
            (DialectId::BigQuery, Position::Any, Emission::Rename("TIME")),
            // Verified live 2026-09-06 (phase 8): no `make_time` routine on
            // Spark, and no TIME-typed constructor under any name.
            (
                DialectId::SparkSql,
                Position::Any,
                Emission::Unsupported {
                    reason: "no `make_time`, and no TIME-typed constructor under any name",
                },
            ),
        ]),
    );
    insert(
        Signature::new(
            "MAKE_TIMESTAMPTZ",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Timestamp {
                with_timezone: true,
            })),
        )
        // Spark's nearest equivalent is `make_timestamp_ltz(y, m, d, h, mi,
        // s, tz)` — a different name taking an extra fixed timezone argument,
        // a shape change a fixed-arity template can't express while this
        // signature's typing is still the permissive variadic `any_args()`
        // shape. Verified live 2026-09-06 (phase 8): no `make_timestamptz`
        // routine resolves; `make_timestamp_ltz(2024,1,1,1,1,1,'UTC')`
        // succeeds but is a different call shape.
        .with_emission(&[(
            DialectId::SparkSql,
            Position::Any,
            Emission::Unsupported {
                reason: "no `make_timestamptz`; Spark's `make_timestamp_ltz` takes a \
                         differently-shaped call (an extra fixed timezone argument), which a \
                         rename or fixed-arity template over this variadic signature can't \
                         express",
            },
        )]),
    );
    // JSON built-ins. Aliases per canonical name (dialect side-channel
    // consolidated into the registry per architecture.md §Constraints #14):
    // JSON_BUILD_OBJECT (Postgres) → JSON_OBJECT, JSON_BUILD_ARRAY (Postgres)
    // → JSON_ARRAY, TO_JSONB/ROW_TO_JSON (Postgres) → TO_JSON,
    // JSON_EXTRACT_PATH (Postgres) → JSON_EXTRACT, JSON_EXTRACT_STRING
    // (DuckDB) / JSON_EXTRACT_PATH_TEXT (Postgres) / GET_JSON_OBJECT (Spark
    // Hive) / JSON_VALUE (SQL-standard/Snowflake) → JSON_EXTRACT_TEXT.
    //
    // JSON_OBJECT and JSON_ARRAY each carry their own emission verdict (see
    // below), so neither is in a shared alias loop.
    insert(
        Signature::new(
            "JSON_OBJECT",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .with_aliases(&["JSON_BUILD_OBJECT"])
        // Spark builds this via `to_json(named_struct(...))` — a variadic
        // restructuring into a nested call with reordered/paired arguments,
        // not a rename or a fixed-arity template over this variadic
        // signature. Verified live 2026-09-06 (phase 8): no `json_object`
        // routine resolves.
        .with_emission(&[(
            DialectId::SparkSql,
            Position::Any,
            Emission::Unsupported {
                reason: "no `json_object`; Spark builds JSON via \
                         `to_json(named_struct(...))`, a variadic restructuring no rename or \
                         fixed-arity template can express",
            },
        )]),
    );
    insert(
        Signature::new(
            "JSON_ARRAY",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .with_aliases(&["JSON_BUILD_ARRAY"])
        // Same shape-change problem as `JSON_OBJECT`: Spark builds this via
        // `to_json(array(...))`. Verified live 2026-09-06 (phase 8): no
        // `json_array` routine resolves.
        .with_emission(&[(
            DialectId::SparkSql,
            Position::Any,
            Emission::Unsupported {
                reason: "no `json_array`; Spark builds JSON via `to_json(array(...))`, a \
                         variadic restructuring no rename or fixed-arity template can express",
            },
        )]),
    );
    // Lifted out of the alias loop above so it can carry its own emission
    // verdict. Spark's `TO_JSON` takes a struct, array, map or variant, not a
    // scalar. Verified live 2026-09-06: `to_json(struct(1 as a, 2 as b))` =
    // `{"a":1,"b":2}`, while `to_json(5)` fails analysis (`Input schema
    // "INT" must be a struct, an array, a map or a variant`).
    insert(
        Signature::new(
            "TO_JSON",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .with_aliases(&["TO_JSONB", "ROW_TO_JSON"])
        .with_emission(&[(
            DialectId::SparkSql,
            Position::Any,
            Emission::Conditional(&[
                ConditionalArm {
                    arity: None,
                    classes: &[(0, OperandClass::Composite)],
                    verdict: SettledEmission::Native,
                },
                ConditionalArm {
                    arity: None,
                    classes: &[],
                    verdict: SettledEmission::Unsupported {
                        reason: "Spark's TO_JSON requires a struct, array or map argument; \
                                 there is no scalar TO_JSON",
                    },
                },
            ]),
        )]),
    );
    // Lifted out of the alias loop above so each can carry its own emission
    // verdict. The alias lists already record what the other engines *call*
    // these; the emission rows are what smelt now *emits*, so resolution and
    // emission finally agree. All verified live on 2026-08-24.
    insert(
        Signature::new(
            "JSON_EXTRACT",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .with_aliases(&["JSON_EXTRACT_PATH"])
        .with_emission(&[(
            DialectId::SparkSql,
            Position::Any,
            Emission::Rename("GET_JSON_OBJECT"),
        )]),
    );
    insert(
        Signature::new(
            "JSON_EXTRACT_TEXT",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .with_aliases(&[
            "JSON_EXTRACT_STRING",
            "JSON_EXTRACT_PATH_TEXT",
            "GET_JSON_OBJECT",
            "JSON_VALUE",
        ])
        .with_emission(&[
            (
                DialectId::DuckDb,
                Position::Any,
                Emission::Rename("JSON_EXTRACT_STRING"),
            ),
            (
                DialectId::SparkSql,
                Position::Any,
                Emission::Rename("GET_JSON_OBJECT"),
            ),
            (
                DialectId::BigQuery,
                Position::Any,
                Emission::Rename("JSON_VALUE"),
            ),
        ]),
    );

    insert(
        Signature::new(
            "JSON_ARRAY_LENGTH",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        // Spark has `json_array_length` under the identical name and shape —
        // an explicit `Native` verdict, not the implicit default, records
        // that this was measured rather than assumed. Verified live
        // 2026-09-06 (phase 8): `json_array_length('{"k": 1}')` resolves and
        // returns a BIGINT. Its value diverges from DuckDB's for a
        // non-array JSON argument (0 vs. NULL) — recorded as a `divergent`
        // ledger row, not a schema gap.
        .with_emission(&[(DialectId::SparkSql, Position::Any, Emission::Native)]),
    );
    insert(
        Signature::new(
            "JSON_OBJECT_KEYS",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Array(Box::new(
                DataType::Text,
            )))),
        )
        // DuckDB alias.
        .with_aliases(&["JSON_KEYS"])
        .with_emission(&[
            // DuckDB spells it `json_keys` — already carried as an alias on this entry,
            // so resolution and emission now agree. Verified live 2026-08-24.
            (
                DialectId::DuckDb,
                Position::Any,
                Emission::Rename("JSON_KEYS"),
            ),
            // Spark has `json_object_keys` under the identical name and
            // shape. Verified live 2026-09-06 (phase 8):
            // `json_object_keys('{"k": 1}')` = `["k"]`, matching DuckDB's
            // `json_keys('{"k": 1}')` = `[k]`.
            (DialectId::SparkSql, Position::Any, Emission::Native),
        ]),
    );
    insert(
        Signature::new(
            "JSON_CONTAINS",
            vec![],
            any_args(),
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
        )
        // Verified live 2026-09-06 (phase 8): no `json_contains` routine on
        // Spark, and no simple placeholder-expressible equivalent for
        // arbitrary JSON containment.
        .with_emission(&[(
            DialectId::SparkSql,
            Position::Any,
            Emission::Unsupported {
                reason: "no `json_contains` in Spark, and no template-expressible equivalent \
                         for arbitrary JSON containment",
            },
        )]),
    );
}
