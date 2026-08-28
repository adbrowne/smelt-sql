//! Pure (no DuckDB) tests for `smelt_logical::backbuild::plan`:
//! `derive_migration_plan`, `plan_hash`, `MigrationVerdict`.
//! See `docs/outcomes/20260815-definition-delta-migrate/phases/02-plan.md`.

use std::collections::{BTreeMap, BTreeSet};

use smelt_logical::backbuild::{
    definition_diff, derive_migration_plan, plan_hash, BackbuildInputs, MigrationVerdict, SourceRef,
};

fn parse(sql: &str) -> smelt_parser::File {
    let parse = smelt_parser::parse(sql);
    smelt_parser::File::cast(parse.syntax()).expect("file")
}

fn inputs(after_sql: &str, added_column_types: &[(&str, &str)]) -> BackbuildInputs {
    BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.to_string(),
        row_identity: None,
        not_null_columns: BTreeSet::new(),
        added_column_types: added_column_types
            .iter()
            .map(|(name, ty)| (name.to_string(), ty.to_string()))
            .collect(),
        sources: BTreeMap::new(),
    }
}

#[test]
fn formatting_only_change_is_eclipsed() {
    let before_sql = "SELECT id, amount FROM orders WHERE amount > 0";
    let after_sql = "SELECT\n  id,\n  amount\nFROM orders\nWHERE amount > 0";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(diff.is_noop(), "expected a formatting-only no-op diff");

    let plan = derive_migration_plan("test_model", &diff, &inputs(after_sql, &[]));
    assert!(
        plan.groups.is_empty(),
        "expected no groups for a no-op diff, got {:?}",
        plan.groups
    );
    assert_eq!(plan.verdict(), MigrationVerdict::Eclipsed);
}

#[test]
fn self_derived_column_add_is_backfill_in_place() {
    let before_sql = "SELECT id, amount, discount FROM orders";
    let after_sql = "SELECT id, amount, discount, amount - discount AS net_amount FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let plan = derive_migration_plan(
        "test_model",
        &diff,
        &inputs(after_sql, &[("net_amount", "DOUBLE")]),
    );
    assert_eq!(plan.groups.len(), 1, "groups: {:?}", plan.groups);
    let group = &plan.groups[0];
    assert_eq!(group.columns, vec!["net_amount".to_string()]);
    assert_eq!(group.verdict, MigrationVerdict::BackfillInPlace);
    assert!(
        group
            .options
            .iter()
            .any(|o| o.technique == smelt_logical::backbuild::Technique::SelfDerivedColumnAdd),
        "expected a SelfDerivedColumnAdd option, got {:?}",
        group.options
    );
    assert_eq!(plan.verdict(), MigrationVerdict::BackfillInPlace);
}

#[test]
fn upstream_pull_through_is_rederive() {
    let before_sql = "SELECT o.order_id AS order_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.discount AS discount FROM orders o";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let mut built = inputs(after_sql, &[("discount", "INTEGER")]);
    built.sources.insert(
        "o".to_string(),
        SourceRef {
            physical_name: "orders".to_string(),
            unique_key: Some(vec!["order_id".to_string()]),
            not_null_columns: ["order_id"].into_iter().map(str::to_string).collect(),
        },
    );

    let plan = derive_migration_plan("test_model", &diff, &built);
    assert_eq!(plan.groups.len(), 1, "groups: {:?}", plan.groups);
    let group = &plan.groups[0];
    assert_eq!(group.columns, vec!["discount".to_string()]);
    assert_eq!(group.verdict, MigrationVerdict::Rederive);
    assert!(
        group
            .options
            .iter()
            .any(|o| o.technique == smelt_logical::backbuild::Technique::UpstreamPullthrough),
        "expected an UpstreamPullthrough option, got {:?}",
        group.options
    );
    assert_eq!(plan.verdict(), MigrationVerdict::Rederive);
}

#[test]
fn group_by_change_is_skeleton_change() {
    let before_sql = "SELECT status, count(*) AS n FROM orders GROUP BY status";
    let after_sql = "SELECT status, amount, count(*) AS n FROM orders GROUP BY status, amount";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let plan = derive_migration_plan("test_model", &diff, &inputs(after_sql, &[]));
    assert_eq!(plan.groups.len(), 1, "groups: {:?}", plan.groups);
    let group = &plan.groups[0];
    assert_eq!(group.verdict, MigrationVerdict::SkeletonChange);
    assert!(
        group.options.is_empty(),
        "a grain change must admit no in-place technique, got {:?}",
        group.options
    );
    assert!(
        !group.refusals.is_empty(),
        "expected a named refusal for the grain change"
    );
    assert_eq!(plan.verdict(), MigrationVerdict::SkeletonChange);
    // Full-refresh baseline is always the only route.
    assert_eq!(
        plan.full_refresh.technique,
        smelt_logical::backbuild::Technique::FullRefresh
    );
}

#[test]
fn unclassifiable_change_surfaces_its_refusal() {
    let before_sql = "SELECT id FROM orders";
    let after_sql = "SELECT id, mystery_function() AS computed FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let plan = derive_migration_plan(
        "test_model",
        &diff,
        &inputs(after_sql, &[("computed", "TEXT")]),
    );
    assert_eq!(plan.groups.len(), 1, "groups: {:?}", plan.groups);
    let group = &plan.groups[0];
    assert!(
        group.options.is_empty(),
        "expected no admissible option, got {:?}",
        group.options
    );
    assert!(
        !group.refusals.is_empty(),
        "an unclassifiable change must surface a named refusal, never an empty plan"
    );
    for refusal in &group.refusals {
        assert!(!refusal.reason.is_empty());
    }
    // Fail-loud: this is never silently reported as eclipsed.
    assert_ne!(plan.verdict(), MigrationVerdict::Eclipsed);
}

#[test]
fn plan_hash_is_stable_across_derivations() {
    let before_sql = "SELECT id, amount, discount FROM orders";
    let after_sql = "SELECT id, amount, discount, amount - discount AS net_amount FROM orders";
    let built = inputs(after_sql, &[("net_amount", "DOUBLE")]);

    let diff1 = definition_diff(&parse(before_sql), &parse(after_sql));
    let plan1 = derive_migration_plan("test_model", &diff1, &built);
    let hash1 = plan_hash(&plan1, &built);

    let diff2 = definition_diff(&parse(before_sql), &parse(after_sql));
    let plan2 = derive_migration_plan("test_model", &diff2, &built);
    let hash2 = plan_hash(&plan2, &built);

    assert_eq!(hash1, hash2);
}

#[test]
fn plan_hash_changes_when_an_input_fact_changes() {
    let before_sql = "SELECT o.order_id AS order_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.discount AS discount FROM orders o";
    let diff = definition_diff(&parse(before_sql), &parse(after_sql));

    let mut without_key = inputs(after_sql, &[("discount", "INTEGER")]);
    without_key.sources.insert(
        "o".to_string(),
        SourceRef {
            physical_name: "orders".to_string(),
            unique_key: None,
            not_null_columns: BTreeSet::new(),
        },
    );
    let plan_without_key = derive_migration_plan("test_model", &diff, &without_key);
    let hash_without_key = plan_hash(&plan_without_key, &without_key);

    let mut with_key = inputs(after_sql, &[("discount", "INTEGER")]);
    with_key.sources.insert(
        "o".to_string(),
        SourceRef {
            physical_name: "orders".to_string(),
            unique_key: Some(vec!["order_id".to_string()]),
            not_null_columns: ["order_id"].into_iter().map(str::to_string).collect(),
        },
    );
    let plan_with_key = derive_migration_plan("test_model", &diff, &with_key);
    let hash_with_key = plan_hash(&plan_with_key, &with_key);

    assert_ne!(
        hash_without_key, hash_with_key,
        "flipping SourceRef::unique_key must change the plan hash"
    );
}

#[test]
fn plan_hash_ignores_region_enumeration() {
    // There is no region field anywhere in `MigrationPlan`/`BackbuildInputs`
    // — deriving twice from identical inputs (which, in a real caller, would
    // still separately enumerate the affected regions at apply time) hashes
    // equal, documenting that region enumeration plays no part in the hash.
    let before_sql = "SELECT id, amount, discount FROM orders";
    let after_sql = "SELECT id, amount, discount, amount - discount AS net_amount FROM orders";
    let built = inputs(after_sql, &[("net_amount", "DOUBLE")]);

    let diff_a = definition_diff(&parse(before_sql), &parse(after_sql));
    let plan_a = derive_migration_plan("test_model", &diff_a, &built);
    let hash_a = plan_hash(&plan_a, &built);

    let diff_b = definition_diff(&parse(before_sql), &parse(after_sql));
    let plan_b = derive_migration_plan("test_model", &diff_b, &built);
    let hash_b = plan_hash(&plan_b, &built);

    assert_eq!(hash_a, hash_b);
}
