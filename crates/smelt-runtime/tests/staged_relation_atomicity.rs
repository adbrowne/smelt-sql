//! The fail-loud leg of the staged relation group's atomicity contract
//! (`docs/outcomes/20260913-trino-ledger/phases/07-plan.md` criterion 8): a
//! group built for a `staged_relation_group_is_atomic == false` capability
//! must never carry `StatementGroup::transactional == true` into
//! `Backend::execute_statement_group`'s default sequential implementation
//! (`crates/smelt-backend/src/lib.rs`), which runs each statement one at a
//! time with no transaction and no warning — a claim silently not honoured
//! is exactly the fail-loud discipline's own failure shape.
//!
//! Every staged-relation emitter derives `transactional` from the
//! [`StagedRelation`] it is handed, never from a hardcoded `true` — this is
//! a standing gate over all four re-keyed emitters, not a fix for one.

use smelt_logical::maintenance::diff_patch::DeleteLeg;
use smelt_logical::maintenance::emit::{
    emit_diff_patch, emit_per_group_recompute, emit_staged_candidate_conditional,
    emit_staged_candidate_conditional_recompute, MaintenanceDialect, StagedRelation,
    StagedRelationResidence,
};

fn non_atomic_relation(name: &str) -> StagedRelation {
    StagedRelation::derive(
        "__smelt_staged_",
        name,
        StagedRelationResidence::TargetSchema,
        false,
    )
}

#[test]
fn no_non_atomic_backend_is_handed_a_transactional_group() {
    let key = vec!["id".to_string()];
    let compared = vec!["v".to_string()];

    let conditional = emit_staged_candidate_conditional(
        "main.t",
        &non_atomic_relation("t"),
        &key,
        "SELECT id, v FROM src",
        &compared,
        MaintenanceDialect::DuckDb,
    );
    assert!(!conditional.transactional);

    let recompute = emit_staged_candidate_conditional_recompute(
        "main.t",
        &non_atomic_relation("t"),
        &key,
        "SELECT id, v FROM src",
        &compared,
        MaintenanceDialect::DuckDb,
    );
    assert!(!recompute.transactional);

    let per_group = emit_per_group_recompute(
        "main.t",
        &non_atomic_relation("t"),
        &key,
        "SELECT DISTINCT id AS delta_key FROM delta",
        "SELECT id, v FROM src",
        MaintenanceDialect::DuckDb,
    );
    assert!(!per_group.transactional);

    let diff_patch = emit_diff_patch(
        "main.t",
        &non_atomic_relation("t"),
        &key,
        "SELECT id, v FROM src",
        &compared,
        "TRUE",
        &DeleteLeg::Complete,
        MaintenanceDialect::DuckDb,
    );
    assert!(!diff_patch.transactional);

    // The atomic (session-temporary) shape stays `true` — this is the
    // non-regression half of the same gate.
    let atomic = emit_staged_candidate_conditional(
        "main.t",
        &StagedRelation::session_temporary("__smelt_staged_t"),
        &key,
        "SELECT id, v FROM src",
        &compared,
        MaintenanceDialect::DuckDb,
    );
    assert!(atomic.transactional);
}
