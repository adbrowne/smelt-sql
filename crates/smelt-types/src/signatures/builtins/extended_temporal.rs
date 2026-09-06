//! Built-in registry rows: extended temporal.
//!
//! Data only — every row is handed to the single `BuiltinRegistry` table
//! constructed in [`super`].

use super::super::{Emission, Position, Signature, SyntaxForm, TypeConstraint, TypeExpr};
use super::concrete;
use crate::{DataType, DialectId};

pub(super) fn register(insert: &mut dyn FnMut(Signature)) {
    // ─── Phase 50: Extended temporal scalars ──────────────────────────────────

    insert(Signature::new(
        "DATE_PART",
        vec![],
        vec![
            concrete(DataType::Text),
            concrete(DataType::Timestamp {
                with_timezone: false,
            }),
        ],
        // BigInt to match the hand-written arm (the date-part extraction
        // family — YEAR/MONTH/DAY/… — all return BigInt).
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
    ));
    insert(
        Signature::new(
            "DATE_ADD",
            vec![],
            vec![concrete(DataType::Date), concrete(DataType::Interval)],
            // `Date + Interval → Timestamp`, matching `binary.rs`'s handling
            // of the equivalent infix form and DuckDB's own reported type
            // (measured live 2026-09-06; phase 4 of
            // docs/outcomes/20260904-dialect-emission-vocabulary).
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Timestamp {
                with_timezone: false,
            })),
        )
        .with_syntax_form(SyntaxForm::Special),
    );
    insert(
        Signature::new(
            "DATE_SUB",
            vec![],
            vec![concrete(DataType::Date), concrete(DataType::Interval)],
            // See `DATE_ADD` above — `Date - Interval → Timestamp` as well.
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Timestamp {
                with_timezone: false,
            })),
        )
        .with_syntax_form(SyntaxForm::Special)
        // DuckDB spells interval subtraction infix; its own
        // `date_sub(VARCHAR, ts, ts)` is a different function entirely.
        // Verified live 2026-09-06.
        .with_emission(&[
            (
                DialectId::DuckDb,
                Position::Any,
                Emission::Template("{0} - {1}"),
            ),
            // Spark's own `date_sub(date, days: INT)` is a different function
            // (integer days, not an INTERVAL); infix `DATE - INTERVAL '5' DAY`
            // is what actually accepts this signature's second argument.
            // Verified live 2026-09-06 (phase 8):
            // `DATE '2026-01-02' - INTERVAL '5' DAY` = `2025-12-28`.
            (
                DialectId::SparkSql,
                Position::Any,
                Emission::Template("{0} - {1}"),
            ),
        ]),
    );
    insert(
        Signature::new(
            "MAKE_DATE",
            vec![],
            vec![
                concrete(DataType::BigInt),
                concrete(DataType::BigInt),
                concrete(DataType::BigInt),
            ],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Date)),
        )
        .with_emission(&[
            // GoogleSQL's three-argument `DATE(y, m, d)` is the same constructor.
            // Verified live 2026-08-24.
            (DialectId::BigQuery, Position::Any, Emission::Rename("DATE")),
        ]),
    );
    insert(
        Signature::new(
            "MAKE_TIMESTAMP",
            vec![],
            vec![
                concrete(DataType::BigInt),
                concrete(DataType::BigInt),
                concrete(DataType::BigInt),
                concrete(DataType::BigInt),
                concrete(DataType::BigInt),
                concrete(DataType::Double),
            ],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Timestamp {
                with_timezone: false,
            })),
        )
        .with_emission(&[
            // GoogleSQL's `DATETIME(y, m, d, h, mi, s)`. Verified live 2026-08-24.
            (
                DialectId::BigQuery,
                Position::Any,
                Emission::Rename("DATETIME"),
            ),
        ]),
    );
    insert(
        Signature::new(
            "AGE",
            vec![],
            vec![
                concrete(DataType::Timestamp {
                    with_timezone: false,
                }),
                concrete(DataType::Timestamp {
                    with_timezone: false,
                }),
            ],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Interval)),
        )
        // Spark has no `age` routine; plain timestamp subtraction is its
        // interval-difference form. Verified live 2026-09-06 (phase 8):
        // `TIMESTAMP '2026-01-02 01:00:00' - TIMESTAMP '2026-01-01 00:00:00'`
        // = `1 01:00:00.000000000` (a day-time interval).
        .with_emission(&[(
            DialectId::SparkSql,
            Position::Any,
            Emission::Template("{0} - {1}"),
        )]),
    );
    insert(
        Signature::new(
            "TO_SECONDS",
            vec![],
            vec![concrete(DataType::Double)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Interval)),
        )
        // Spark has no `to_seconds`; `make_interval` with every field but
        // seconds zeroed is the fixed-shape equivalent. Verified live
        // 2026-09-06 (phase 8): `make_interval(0, 0, 0, 0, 0, 0, 60.0)` =
        // `1 minutes`.
        .with_emission(&[(
            DialectId::SparkSql,
            Position::Any,
            Emission::Template("make_interval(0, 0, 0, 0, 0, 0, {0})"),
        )]),
    );
}
