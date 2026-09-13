//! Built-in registry rows: window.
//!
//! Data only — every row is handed to the single `BuiltinRegistry` table
//! constructed in [`super`].

use super::super::{Emission, ExprKind, Position, RewriteId, Signature, TypeConstraint, TypeExpr};
use super::{tp, var};
use crate::{DataType, DialectId};

pub(super) fn register(insert: &mut dyn FnMut(Signature)) {
    // ─── Window-only built-ins (Phase 14, §16 #24).
    //
    // These are dispatched only at call sites that carry an `OVER (…)`
    // clause; calling them without `OVER` is a runtime error in every
    // backend. Phase 14 records the kind only — argument-list checks for
    // these signatures land in a later phase. The placeholder `Any` arg
    // lists keep the existing `unify_call` happy without imposing a
    // false constraint.
    insert(
        Signature::new(
            "ROW_NUMBER",
            vec![],
            vec![],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "RANK",
            vec![],
            vec![],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "DENSE_RANK",
            vec![],
            vec![],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        .with_kind(ExprKind::Window),
    );
    // `LAG`/`LEAD` are offset functions the SQL standard defines to ignore
    // their window frame; Spark refuses any frame on them outright,
    // regardless of whether the frame happens to cover the whole partition
    // or is a running one — so both window positions carry the same verdict
    // (the coverage-totality gate requires either both or neither). The
    // frame is dropped only for SparkSQL — see `docs/specs/multi_backend.md`
    // §"Frame elision on offset functions".
    const LAG_LEAD_EMISSION: &[(DialectId, Position, Emission)] = &[
        (
            DialectId::SparkSql,
            Position::Window,
            Emission::Rewrite(RewriteId::ElideWindowFrame),
        ),
        (
            DialectId::SparkSql,
            Position::WholePartitionWindow,
            Emission::Rewrite(RewriteId::ElideWindowFrame),
        ),
    ];
    insert(
        Signature::new(
            "LAG",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Window)
        .with_emission(LAG_LEAD_EMISSION),
    );
    insert(
        Signature::new(
            "LEAD",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Window)
        .with_emission(LAG_LEAD_EMISSION),
    );
}
