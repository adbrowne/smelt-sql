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
//! a standing gate over all five emitters (the four keyed/recompute shapes
//! plus the keyless whole-row shape), not a fix for one. A further standing
//! fact closes criterion 3 (a claim implies a builder): no source file in
//! `crates/smelt-logical/src/maintenance/emit/` spells `CREATE TEMP TABLE`
//! outside `staged_relation.rs`'s own `create_prefix`.
//!
//! `smelt_backend::maintenance_dialect(SqlDialect::Trino)` now resolves
//! (`20260913-trino-incremental` phase 3), so it no longer stands between a
//! Trino caller and the keyless executor — which families actually reach it
//! is `20260913-trino-incremental` phase 5's subject (the merge-less
//! conditional write over T3's staged relation), not this file's.

use smelt_logical::maintenance::diff_patch::DeleteLeg;
use smelt_logical::maintenance::emit::{
    emit_diff_patch, emit_per_group_recompute, emit_staged_candidate_conditional,
    emit_staged_candidate_conditional_keyless, emit_staged_candidate_conditional_recompute,
    MaintenanceDialect, StagedRelation, StagedRelationResidence,
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

    let keyless = emit_staged_candidate_conditional_keyless(
        "main.t",
        &non_atomic_relation("t"),
        &non_atomic_relation("t_sentinel"),
        None,
        "SELECT id, v FROM src",
        MaintenanceDialect::DuckDb,
    );
    assert!(!keyless.transactional);

    let keyless_atomic = emit_staged_candidate_conditional_keyless(
        "main.t",
        &StagedRelation::session_temporary("__smelt_staged_t"),
        &StagedRelation::session_temporary("__smelt_sentinel_t"),
        None,
        "SELECT id, v FROM src",
        MaintenanceDialect::DuckDb,
    );
    assert!(keyless_atomic.transactional);
}

/// Regression gate: the string literal `"CREATE TEMP TABLE` must appear in
/// exactly one source file under `crates/smelt-logical/src/maintenance/
/// emit/` — `staged_relation.rs`'s own `create_prefix()` — outside test
/// modules. Doc comments that *describe* emitted SQL (numbered statement
/// lists) are not scanned; only an actual string-literal spelling counts. A
/// staged emitter that hardcodes its own `CREATE TEMP TABLE` spelling
/// instead of deriving it from a [`StagedRelation`] would defeat the whole
/// residence-as-data migration for whichever backend it is (a temp-table
/// spelling on a backend with no temp tables is exactly the
/// claim-without-builder failure criterion 3 excludes).
#[test]
fn no_staged_emitter_hardcodes_a_temp_table_spelling() {
    let emit_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .join("smelt-logical/src/maintenance/emit");
    assert!(emit_dir.is_dir(), "emit dir not found: {emit_dir:?}");

    let mut offending = Vec::new();
    for entry in std::fs::read_dir(&emit_dir).expect("read emit dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let contents = std::fs::read_to_string(&path).expect("read source file");
        // Only the non-#[cfg(test)] prefix of each file counts — test
        // modules are allowed to spell out expected literal statement text.
        let production_prefix = match contents.find("#[cfg(test)]") {
            Some(idx) => &contents[..idx],
            None => contents.as_str(),
        };
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if file_name == "staged_relation.rs" {
            // `create_prefix()` itself is the single owner of this literal.
            continue;
        }
        // Only a string-literal spelling counts — doc comments that
        // *describe* the emitted SQL (e.g. a numbered statement list) are
        // not a hardcoded emission and must not trip this gate.
        if production_prefix.contains("\"CREATE TEMP TABLE") {
            offending.push(path.display().to_string());
        }
    }
    assert!(
        offending.is_empty(),
        "these emit/ source files hardcode `CREATE TEMP TABLE` outside \
         staged_relation.rs's create_prefix(): {offending:?}"
    );
}

/// `smelt_backend::maintenance_dialect(SqlDialect::Trino)` now resolves to
/// `MaintenanceDialect::Trino` (`20260913-trino-incremental` phase 3) — the
/// non-regression counterpart of this file's other assertions, pinning that
/// the mapping succeeds rather than the pre-phase-3 refusal.
#[test]
fn trino_now_resolves_a_maintenance_dialect() {
    let result = smelt_backend::maintenance_dialect(smelt_dialect::SqlDialect::Trino);
    assert_eq!(result, Ok(MaintenanceDialect::Trino));
}
