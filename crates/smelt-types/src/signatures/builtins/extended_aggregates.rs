//! Built-in registry rows: extended aggregates.
//!
//! Data only — every row is handed to the single `BuiltinRegistry` table
//! constructed in [`super`].

use super::super::{
    Emission, ExprKind, Position, RestructureId, RewriteId, SigParam, Signature, TypeConstraint,
    TypeExpr,
};
use super::{concrete, tp, var};
use crate::{DataType, DialectId};

pub(super) fn register(insert: &mut dyn FnMut(Signature)) {
    // ─── Phase 50: Extended aggregates ──────────────────────────────────────

    insert(
        Signature::new(
            "STRING_AGG",
            vec![],
            vec![concrete(DataType::Text), concrete(DataType::Text)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "LISTAGG",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T"), concrete(DataType::Text)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .with_kind(ExprKind::Agg)
        .with_emission(&[
            // GoogleSQL spells the separator-taking string aggregate `STRING_AGG`.
            // Verified live 2026-08-24: `STRING_AGG(x, ',')` -> STRING.
            (
                DialectId::BigQuery,
                Position::Any,
                Emission::Rename("STRING_AGG"),
            ),
        ]),
    );
    insert(
        Signature::new(
            "ARRAY_AGG",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            // Array<T> cannot be expressed as a TypeExpr::Var directly; use Unknown for v1.
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Unknown(
                crate::UnknownReason::Dynamic,
            ))),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "MEDIAN",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Agg)
        .with_emission(&[
            // GoogleSQL has no `MEDIAN`; the printer lowers it to an exact
            // form per position (`printer/registry_emit.rs::print_bigquery_median`). The
            // aggregate and whole-partition-window forms are both exact
            // lowerings. A running `MEDIAN(x) OVER (PARTITION BY g ORDER BY
            // t)` has no exact GoogleSQL form — `PERCENTILE_CONT` forbids a
            // window `ORDER BY` — and is refused
            // (`docs/specs/multi_backend.md` §"Exact-median lowering").
            (
                DialectId::BigQuery,
                Position::Aggregate,
                Emission::Rewrite(RewriteId::BigQueryMedian),
            ),
            (
                DialectId::BigQuery,
                Position::WholePartitionWindow,
                Emission::Rewrite(RewriteId::BigQueryMedian),
            ),
            (
                DialectId::BigQuery,
                Position::Window,
                Emission::Unsupported {
                    reason: "GoogleSQL's PERCENTILE_CONT lowering of MEDIAN forbids a \
                             window ORDER BY; only a window covering the whole partition \
                             has an exact GoogleSQL form",
                },
            ),
            // DuckDB has `MEDIAN` natively in every position (no entry
            // needed — a pair with no entry is `Native`). Spark's `MEDIAN`
            // is an ordered-set aggregate with no window form, so it follows
            // `PERCENTILE_CONT`/`PERCENTILE_DISC`: a whole-partition window
            // restructures around a grouped CTE, and a running window is
            // refused.
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
        ]),
    );
    insert(
        Signature::new(
            "STDDEV",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "STDDEV_POP",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "STDDEV_SAMP",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "VARIANCE",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "VAR_POP",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "VAR_SAMP",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "BOOL_AND",
            vec![],
            vec![concrete(DataType::Boolean)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
        )
        .with_kind(ExprKind::Agg)
        .with_emission(&[
            (
                DialectId::SparkSql,
                Position::Any,
                Emission::Rename("EVERY"),
            ),
            (
                DialectId::BigQuery,
                Position::Any,
                Emission::Rename("LOGICAL_AND"),
            ),
        ]),
    );
    insert(
        Signature::new(
            "BOOL_OR",
            vec![],
            vec![concrete(DataType::Boolean)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
        )
        .with_kind(ExprKind::Agg)
        .with_emission(&[
            (DialectId::SparkSql, Position::Any, Emission::Rename("SOME")),
            (
                DialectId::BigQuery,
                Position::Any,
                Emission::Rename("LOGICAL_OR"),
            ),
        ]),
    );
    insert(
        Signature::new(
            "BIT_AND",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "BIT_OR",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "BIT_XOR",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "ANY_VALUE",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Agg),
    );
    // arg_max(value, key) → value: return the value from the row with the maximum key.
    // Accepts any value type T and any key type K (must be orderable at runtime).
    // `MAX_BY` is DuckDB/Postgres's alias for the same order-monotone-overwrite
    // combiner (`incremental_shapes.md` §"The column-family catalogue").
    insert(
        Signature::new(
            "ARG_MAX",
            vec![tp("T", TypeConstraint::Any), tp("K", TypeConstraint::Any)],
            vec![var("T"), var("K")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Agg)
        .with_aliases(&["MAX_BY"])
        .with_emission(&[
            // Both spell it `MAX_BY`. Verified live 2026-08-24 on Spark 4.0.0 and
            // BigQuery: `MAX_BY(x, y)` resolves on each. Spark accepts `MAX_BY`
            // in every position, so it stays a single `Any` entry.
            (
                DialectId::SparkSql,
                Position::Any,
                Emission::Rename("MAX_BY"),
            ),
            // BigQuery's `MAX_BY` has no analytic form at all — refused even
            // with a partition-only `OVER` clause (measured live 2026-08-27).
            // Aggregate position falls through the `Any` entry below to
            // `Rename("MAX_BY")`; a whole-partition window restructures
            // around a grouped CTE, and a running window is refused, because
            // the lowering computes one value per partition
            // (`docs/specs/multi_backend.md` §"Statement-level lowering").
            (
                DialectId::BigQuery,
                Position::Any,
                Emission::Rename("MAX_BY"),
            ),
            (
                DialectId::BigQuery,
                Position::WholePartitionWindow,
                Emission::Restructure(RestructureId::WindowToCte),
            ),
            (
                DialectId::BigQuery,
                Position::Window,
                Emission::Unsupported {
                    reason: "BigQuery's MAX_BY has no analytic form, even over a \
                             partition-only window; only a window covering the whole \
                             partition can be restructured around a grouped CTE",
                },
            ),
        ]),
    );
    // arg_min(value, key) → value: the order-monotone-overwrite family's
    // minimum-ordering counterpart to `ARG_MAX`, aliased `MIN_BY`.
    insert(
        Signature::new(
            "ARG_MIN",
            vec![tp("T", TypeConstraint::Any), tp("K", TypeConstraint::Any)],
            vec![var("T"), var("K")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Agg)
        .with_aliases(&["MIN_BY"])
        .with_emission(&[
            // Both spell it `MIN_BY`. Verified live 2026-08-24. Spark accepts
            // `MIN_BY` in every position, so it stays a single `Any` entry.
            (
                DialectId::SparkSql,
                Position::Any,
                Emission::Rename("MIN_BY"),
            ),
            // Same asymmetry as `ARG_MAX`/`MAX_BY`: BigQuery's `MIN_BY` has no
            // analytic form at all.
            (
                DialectId::BigQuery,
                Position::Any,
                Emission::Rename("MIN_BY"),
            ),
            (
                DialectId::BigQuery,
                Position::WholePartitionWindow,
                Emission::Restructure(RestructureId::WindowToCte),
            ),
            (
                DialectId::BigQuery,
                Position::Window,
                Emission::Unsupported {
                    reason: "BigQuery's MIN_BY has no analytic form, even over a \
                             partition-only window; only a window covering the whole \
                             partition can be restructured around a grouped CTE",
                },
            ),
        ]),
    );
    insert(
        Signature::new(
            "APPROX_COUNT_DISTINCT",
            vec![],
            vec![SigParam::Concrete(TypeConstraint::Any)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        .with_kind(ExprKind::Agg)
        .with_emission(&[
            // BigQuery's `APPROX_COUNT_DISTINCT` is a plain aggregate with no
            // analytic form. GoogleSQL's own dry run *accepts* the analytic
            // spelling — `APPROX_COUNT_DISTINCT(x) OVER (PARTITION BY g)`
            // parses and dry-runs cleanly — and only execution refuses it
            // (measured live 2026-08-27); a schema/dry-run probe alone
            // cannot see this gap, only a leg that actually executes.
            // Aggregate position is unaffected — it's already `Native` — a
            // whole-partition window restructures around a grouped CTE, and
            // a running window is refused.
            (
                DialectId::BigQuery,
                Position::WholePartitionWindow,
                Emission::Restructure(RestructureId::WindowToCte),
            ),
            (
                DialectId::BigQuery,
                Position::Window,
                Emission::Unsupported {
                    reason: "BigQuery's APPROX_COUNT_DISTINCT has no analytic form, even \
                             over a partition-only window; only a window covering the \
                             whole partition can be restructured around a grouped CTE",
                },
            ),
        ]),
    );
}
