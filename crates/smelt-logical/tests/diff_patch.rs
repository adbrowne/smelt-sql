//! Correctness oracle for the `diff_patch` write pattern
//! (`docs/specs/incremental_models.md` §"The write-pattern set is open" →
//! "`diff_patch` — compute, diff, write only the difference"): registry
//! membership, admission ([`admit_diff_patch`]), and the pure emitter
//! ([`emit_diff_patch`]). Mirrors `crates/smelt-logical/tests/repair_cell.rs`'s
//! doc-comment-header + helper-fn style for admission, and
//! `crates/smelt-logical/tests/emit_statements.rs`'s style for the emit-side
//! assertions.

use smelt_logical::analysis::walk::{ColumnComparability, Comparability};
use smelt_logical::maintenance::diff_patch::{admit_diff_patch, DeleteLeg, DiffPatchRefusal};
use smelt_logical::maintenance::emit::{emit_diff_patch, MaintenanceDialect, Region};
use smelt_logical::maintenance::{
    admissible_write_patterns, lookup_write_pattern, BackendWriteCapabilities, ContractFact,
    OutputContractFacts, RowIdentity, RowIdentityVerdict, WriteCapability,
};

fn key_identity(cols: &[&str]) -> RowIdentityVerdict {
    RowIdentityVerdict {
        identity: RowIdentity::Key(cols.iter().map(|c| c.to_string()).collect()),
        proven_mismatch: None,
    }
}

fn whole_row_identity() -> RowIdentityVerdict {
    RowIdentityVerdict {
        identity: RowIdentity::WholeRow,
        proven_mismatch: None,
    }
}

fn comparable(cols: &[&str]) -> Vec<ColumnComparability> {
    cols.iter()
        .map(|c| ColumnComparability {
            output: c.to_string(),
            comparability: Comparability::Comparable,
        })
        .collect()
}

#[test]
fn registry_exposes_diff_patch() {
    let pattern = lookup_write_pattern("diff_patch").expect("diff_patch must be registered");
    assert_eq!(pattern.required_facts, &[ContractFact::Identity]);
    assert_eq!(pattern.capability, WriteCapability::Always);
}

#[test]
fn diff_patch_absent_without_identity() {
    let no_identity = OutputContractFacts {
        has_identity: false,
        has_partition_axis: true,
    };
    let names = admissible_write_patterns(no_identity, BackendWriteCapabilities::all());
    assert!(
        !names.contains(&"diff_patch"),
        "diff_patch must not be admissible without a declared identity; got {names:?}"
    );

    let with_identity = OutputContractFacts {
        has_identity: true,
        has_partition_axis: false,
    };
    let names = admissible_write_patterns(with_identity, BackendWriteCapabilities::all());
    assert!(
        names.contains(&"diff_patch"),
        "diff_patch must be admissible for an identity-bearing output; got {names:?}"
    );
}

#[test]
fn admit_requires_identity() {
    let err = admit_diff_patch(
        &["amount".to_string()],
        &comparable(&["amount"]),
        &whole_row_identity(),
        Ok(()),
    )
    .expect_err("expected refusal for a WholeRow identity");
    assert!(matches!(err, DiffPatchRefusal::IdentityRequired { .. }));
}

#[test]
fn admit_requires_comparability_for_update_leg() {
    let identity = key_identity(&["customer_id"]);
    // No comparability proof at all for "amount" — fail-closed absence.
    let err = admit_diff_patch(&["amount".to_string()], &[], &identity, Ok(()))
        .expect_err("expected refusal for an unproven compared column");
    match err {
        DiffPatchRefusal::ComparabilityRequired { .. } => {}
        other => panic!("expected ComparabilityRequired, got {other:?}"),
    }

    // Explicitly marked Incomparable also refuses.
    let incomparable = vec![ColumnComparability {
        output: "amount".to_string(),
        comparability: Comparability::Incomparable,
    }];
    let err = admit_diff_patch(&["amount".to_string()], &incomparable, &identity, Ok(()))
        .expect_err("expected refusal for an incomparable column");
    assert!(matches!(
        err,
        DiffPatchRefusal::ComparabilityRequired { .. }
    ));
}

#[test]
fn admit_without_slice_completeness_degrades() {
    let identity = key_identity(&["customer_id"]);
    let admitted = admit_diff_patch(
        &["amount".to_string()],
        &comparable(&["amount"]),
        &identity,
        Err("windowed read is not proven complete".to_string()),
    )
    .expect("admission must succeed even without slice completeness");
    match admitted.delete_leg {
        DeleteLeg::Omitted { why } => assert!(why.contains("windowed read")),
        DeleteLeg::Complete => panic!("expected Omitted, got Complete"),
    }
    assert_eq!(admitted.key, vec!["customer_id".to_string()]);
    assert_eq!(admitted.compared_columns, vec!["amount".to_string()]);
}

#[test]
fn admit_with_slice_completeness_keeps_delete_leg() {
    let identity = key_identity(&["customer_id"]);
    let admitted = admit_diff_patch(
        &["amount".to_string()],
        &comparable(&["amount"]),
        &identity,
        Ok(()),
    )
    .expect("admission must succeed");
    assert_eq!(admitted.delete_leg, DeleteLeg::Complete);
}

fn test_region() -> Region {
    Region {
        start: "'2026-01-01'".to_string(),
        end: "'2026-01-02'".to_string(),
    }
}

fn test_slice_predicate() -> String {
    test_region().predicate(Some("main.customers"), "event_date")
}

#[test]
fn emit_diff_patch_stages_then_patches() {
    let slice_predicate = test_slice_predicate();
    let candidate_select = "SELECT customer_id, tier FROM main.stg_customers WHERE event_date \
                             >= '2026-01-01' AND event_date < '2026-01-02'";
    let group = emit_diff_patch(
        "main.customers",
        "__smelt_staged",
        &["customer_id".to_string()],
        candidate_select,
        &["tier".to_string()],
        &slice_predicate,
        &DeleteLeg::Complete,
        MaintenanceDialect::DuckDb,
    );

    assert!(group.transactional, "diff_patch must be one transaction");
    assert_eq!(group.statements.len(), 6);

    assert!(group.statements[0].sql.starts_with("CREATE TEMP TABLE"));
    assert!(group.statements[1]
        .sql
        .starts_with("INSERT INTO __smelt_staged"));
    assert!(
        group.statements[2]
            .sql
            .starts_with("DELETE FROM main.customers USING __smelt_staged"),
        "update-leg delete: {}",
        group.statements[2].sql
    );
    assert!(
        group.statements[3]
            .sql
            .starts_with("DELETE FROM main.customers WHERE")
            && group.statements[3].sql.contains("NOT EXISTS"),
        "delete-leg delete: {}",
        group.statements[3].sql
    );
    assert!(group.statements[4]
        .sql
        .starts_with("INSERT INTO main.customers"));
    assert!(group.statements[5].sql.starts_with("DROP TABLE"));

    for stmt in &group.statements {
        if stmt.sql.starts_with("DELETE") {
            assert!(
                stmt.sql.contains("'2026-01-01'") && stmt.sql.contains("'2026-01-02'"),
                "every DELETE must carry the region's rendered bound literals: {}",
                stmt.sql
            );
        }
    }
}

#[test]
fn emit_diff_patch_restricts_both_delete_legs_to_the_caller_slice_predicate() {
    let slice_predicate = "EXISTS (SELECT 1 FROM (SELECT DISTINCT customer_id FROM \
                            main.stg_orders) AS __smelt_repair_keys WHERE \
                            main.customers.customer_id = __smelt_repair_keys.customer_id)"
        .to_string();
    let candidate_select = "SELECT customer_id, tier FROM main.stg_customers";
    let group = emit_diff_patch(
        "main.customers",
        "__smelt_staged",
        &["customer_id".to_string()],
        candidate_select,
        &["tier".to_string()],
        &slice_predicate,
        &DeleteLeg::Complete,
        MaintenanceDialect::DuckDb,
    );

    let update_leg_delete = &group.statements[2].sql;
    let delete_leg_delete = &group.statements[3].sql;
    assert!(
        update_leg_delete.contains(&slice_predicate),
        "update-leg delete must carry the caller's slice predicate verbatim: {update_leg_delete}"
    );
    assert!(
        delete_leg_delete.contains(&slice_predicate),
        "delete-leg delete must carry the caller's slice predicate verbatim: {delete_leg_delete}"
    );
}

#[test]
fn emit_diff_patch_comparison_is_null_safe() {
    let slice_predicate = test_slice_predicate();
    let candidate_select = "SELECT customer_id, tier FROM main.stg_customers";
    let group = emit_diff_patch(
        "main.customers",
        "__smelt_staged",
        &["customer_id".to_string()],
        candidate_select,
        &["tier".to_string()],
        &slice_predicate,
        &DeleteLeg::Complete,
        MaintenanceDialect::DuckDb,
    );
    let update_delete = &group.statements[2].sql;
    assert!(
        update_delete.contains("IS DISTINCT FROM"),
        "update-leg delete must use IS DISTINCT FROM: {update_delete}"
    );
    assert!(
        !update_delete.contains(" <> "),
        "update-leg delete must never use bare <>: {update_delete}"
    );
}

#[test]
fn emit_diff_patch_omits_delete_leg_when_incomplete() {
    let slice_predicate = test_slice_predicate();
    let candidate_select = "SELECT customer_id, tier FROM main.stg_customers";

    let complete_group = emit_diff_patch(
        "main.customers",
        "__smelt_staged",
        &["customer_id".to_string()],
        candidate_select,
        &["tier".to_string()],
        &slice_predicate,
        &DeleteLeg::Complete,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(complete_group.statements.len(), 6);
    let delete_count = complete_group
        .statements
        .iter()
        .filter(|s| s.sql.starts_with("DELETE"))
        .count();
    assert_eq!(delete_count, 2, "Complete must emit both delete legs");

    let omitted_group = emit_diff_patch(
        "main.customers",
        "__smelt_staged",
        &["customer_id".to_string()],
        candidate_select,
        &["tier".to_string()],
        &slice_predicate,
        &DeleteLeg::Omitted {
            why: "not proven complete".to_string(),
        },
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(omitted_group.statements.len(), 5);
    let delete_count = omitted_group
        .statements
        .iter()
        .filter(|s| s.sql.starts_with("DELETE"))
        .count();
    assert_eq!(
        delete_count, 1,
        "Omitted must emit only the update-leg delete"
    );
    assert!(
        !omitted_group
            .statements
            .iter()
            .any(|s| s.sql.starts_with("DELETE") && s.sql.contains("NOT EXISTS")),
        "Omitted must not contain an anti-join DELETE"
    );
}

#[test]
fn emit_diff_patch_rejects_empty_key() {
    let slice_predicate = test_slice_predicate();
    let result = std::panic::catch_unwind(|| {
        emit_diff_patch(
            "main.customers",
            "__smelt_staged",
            &[],
            "SELECT customer_id, tier FROM main.stg_customers",
            &["tier".to_string()],
            &slice_predicate,
            &DeleteLeg::Complete,
            MaintenanceDialect::DuckDb,
        )
    });
    let err = result.expect_err("emit_diff_patch must panic on an empty key");
    let msg = err
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(msg.contains("emit_diff_patch"), "{msg}");
    assert!(msg.contains("main.customers"), "{msg}");
}
