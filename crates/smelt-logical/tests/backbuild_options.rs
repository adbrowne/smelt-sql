//! Pure (no DuckDB) tests for the backbuild option-set data model:
//! `smelt_logical::backbuild::{BackbuildOptions, AtomAnalysis, assemble}`.
//! See `docs/research/20260802-backbuild-synthesis.md` §2 ("Refusal
//! posture": "partial application is never offered; an atom with an empty
//! option set leaves FullRefresh as the model's only option") and
//! `.superpowers/sdd/20260802-backbuild-synthesis/task-2-brief.md`.

use std::collections::BTreeMap;

use smelt_logical::backbuild::{
    assemble, definition_diff, derive_backbuild_options, AtomAnalysis, AtomicChange,
    BackbuildInputs, BackbuildOption, BackbuildOptions, HSlot, Selection, SourceRef, Technique,
    WriteScope,
};

fn parse(sql: &str) -> smelt_parser::File {
    let parse = smelt_parser::parse(sql);
    smelt_parser::File::cast(parse.syntax()).expect("file")
}

fn inputs(added_column_types: &[(&str, &str)]) -> BackbuildInputs {
    BackbuildInputs {
        table: "t".to_string(),
        after_sql: "SELECT 1".to_string(),
        row_identity: None,
        added_column_types: added_column_types
            .iter()
            .map(|(name, ty)| (name.to_string(), ty.to_string()))
            .collect(),
        sources: BTreeMap::new(),
    }
}

/// Hand-construct a `BackbuildOptions` value with two atoms — one carrying
/// an admissible option, one with an empty option set — bypassing
/// `derive_backbuild_options` entirely so this test pins `assemble`'s
/// composition rule (partial application never offered) independently of
/// any classifier logic, per the brief: "Hand-construction keeps this
/// phase off Phase 3's admission logic; classification-driven coverage of
/// the same rule arrives with Phase 3's first admitted case."
#[test]
fn atom_without_options_leaves_only_full_refresh() {
    let admissible_atom = AtomAnalysis {
        change: AtomicChange::Unclassified,
        options: vec![BackbuildOption {
            technique: Technique::FullRefresh,
            slot: Some(HSlot::UpdateMerge),
            statements: vec!["UPDATE t SET c = 1".to_string()],
            write_scope: WriteScope::ColumnScoped,
            reads_upstream: false,
            rerun_safe: true,
        }],
        inadmissible: Vec::new(),
    };
    let refused_atom = AtomAnalysis {
        change: AtomicChange::Skeleton {
            reason: "GROUP BY changed".to_string(),
        },
        options: Vec::new(),
        inadmissible: vec![smelt_logical::backbuild::BackbuildRefusal {
            atom: "skeleton".to_string(),
            reason: "G1 (grain change) — GROUP BY changed".to_string(),
        }],
    };

    let full_refresh = BackbuildOption {
        technique: Technique::FullRefresh,
        slot: None,
        statements: vec!["CREATE OR REPLACE TABLE t AS SELECT 1".to_string()],
        write_scope: WriteScope::FullWrite,
        reads_upstream: true,
        rerun_safe: true,
    };

    let options = BackbuildOptions {
        atoms: vec![admissible_atom, refused_atom],
        full_refresh: full_refresh.clone(),
    };

    // Partial application is never offered: even though the first atom has
    // an admissible option, the second atom's empty option set blocks any
    // composed targeted script.
    let targeted = assemble(
        &options,
        &Selection::Targeted {
            atom_choices: vec![0, 0],
        },
    );
    assert!(
        targeted.is_empty(),
        "an atom with no admissible option must block the composed targeted script, got \
         {targeted:?}"
    );

    // FullRefresh remains the model's only option, unaffected by the mix of
    // atom outcomes above.
    let full_refresh_script = assemble(&options, &Selection::FullRefresh);
    assert_eq!(full_refresh_script, full_refresh.statements);
}

/// `derive_backbuild_options` itself must never silently report zero atoms
/// for a diff that actually changed something — a changed expression
/// (D1-shaped: same output column name, different expression, skeleton
/// unchanged) is not yet classified into an admissible technique this
/// phase, but it must still surface as a named, fail-closed refusal rather
/// than an empty atom list that would make the (empty) targeted script look
/// composable.
#[test]
fn unclassified_diff_still_refuses_rather_than_reporting_no_atoms() {
    let before_sql = "SELECT id, amount FROM orders";
    let after_sql = "SELECT id, amount * 2 AS amount FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.to_string(),
        row_identity: None,
        added_column_types: BTreeMap::new(),
        sources: BTreeMap::new(),
    };
    let options = derive_backbuild_options(&diff, &inputs);

    assert_eq!(
        options.atoms.len(),
        1,
        "a changed-but-unclassified diff must still yield a named atom, got {:?}",
        options.atoms
    );
    let atom = &options.atoms[0];
    assert!(atom.options.is_empty());
    assert_eq!(
        atom.inadmissible.len(),
        1,
        "inadmissible: {:?}",
        atom.inadmissible
    );
    assert!(
        !atom.inadmissible[0].reason.is_empty(),
        "the refusal must carry a named reason"
    );

    let targeted = assemble(
        &options,
        &Selection::Targeted {
            atom_choices: vec![0],
        },
    );
    assert!(targeted.is_empty());
}

// ===== B1/B2 (task-3-brief.md) =====

fn single_atom(options: &BackbuildOptions) -> &AtomAnalysis {
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    &options.atoms[0]
}

/// At least one named, non-empty refusal, and no admissible option. Phase 5
/// (B3/D2) can attempt a sibling technique after a same-shaped D1/B1
/// failure (`d1_upstream_dependency_refuses_until_d2`,
/// `b1_dependency_on_upstream_only_column_is_not_b1`): when that sibling
/// attempt *also* refuses, the atom carries both named reasons rather than
/// exactly one, so this checks "at least one, all non-empty" rather than
/// pinning the count.
fn assert_refused(atom: &AtomAnalysis) {
    assert!(
        atom.options.is_empty(),
        "expected no admissible option, got {:?}",
        atom.options
    );
    assert!(
        !atom.inadmissible.is_empty(),
        "expected at least one refusal, got none"
    );
    for refusal in &atom.inadmissible {
        assert!(
            !refusal.reason.is_empty(),
            "every refusal must carry a named reason"
        );
    }
}

#[test]
fn b1_opaque_function_refuses() {
    let before_sql = "SELECT id FROM orders";
    let after_sql = "SELECT id, mystery_function() AS computed FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert!(matches!(
        &atom.change,
        AtomicChange::AddedColumn { name } if name == "computed"
    ));
    assert_refused(atom);
    assert!(
        atom.inadmissible[0].reason.contains("mystery_function"),
        "reason: {}",
        atom.inadmissible[0].reason
    );
}

#[test]
fn b1_volatile_function_refuses() {
    for volatile_expr in ["RANDOM()", "NOW()"] {
        let before_sql = "SELECT id FROM orders";
        let after_sql = format!("SELECT id, {volatile_expr} AS computed FROM orders");

        let diff = definition_diff(&parse(before_sql), &parse(&after_sql));
        assert!(!diff.is_noop());

        let options = derive_backbuild_options(&diff, &inputs(&[]));
        let atom = single_atom(&options);
        assert_refused(atom);
        assert!(
            atom.inadmissible[0]
                .reason
                .to_lowercase()
                .contains("non-deterministic"),
            "reason for {volatile_expr}: {}",
            atom.inadmissible[0].reason
        );
    }
}

#[test]
fn b1_subquery_refuses() {
    let before_sql = "SELECT id FROM orders";
    let after_sql = "SELECT id, (SELECT MAX(amount) FROM orders) AS max_amount FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible[0]
            .reason
            .to_lowercase()
            .contains("subquery"),
        "reason: {}",
        atom.inadmissible[0].reason
    );
}

#[test]
fn b1_window_refuses() {
    let before_sql = "SELECT id FROM orders";
    let after_sql = "SELECT id, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible[0]
            .reason
            .to_lowercase()
            .contains("window"),
        "reason: {}",
        atom.inadmissible[0].reason
    );
}

#[test]
fn b1_dependency_on_upstream_only_column_is_not_b1() {
    let before_sql = "SELECT o.id AS id FROM orders o JOIN regions r ON o.region_id = r.region_id";
    let after_sql = "SELECT o.id AS id, r.region_name AS region_name FROM orders o JOIN regions r \
                      ON o.region_id = r.region_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert!(matches!(
        &atom.change,
        AtomicChange::AddedColumn { name } if name == "region_name"
    ));
    assert_refused(atom);
    assert!(
        atom.inadmissible[0].reason.contains("representative"),
        "expected the refusal to name the missing stored representative, got: {}",
        atom.inadmissible[0].reason
    );
}

#[test]
fn b1_constant_and_arithmetic_columns_admit_alter_plus_update() {
    let before_sql = "SELECT id, price, qty FROM orders";
    let after_sql = "SELECT id, price, qty, 'active' AS status, price * qty AS total FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options =
        derive_backbuild_options(&diff, &inputs(&[("status", "TEXT"), ("total", "INTEGER")]));
    assert_eq!(options.atoms.len(), 2, "atoms: {:?}", options.atoms);

    let status_atom = options
        .atoms
        .iter()
        .find(|a| matches!(&a.change, AtomicChange::AddedColumn { name } if name == "status"))
        .expect("status atom");
    assert_eq!(status_atom.options.len(), 1, "options: {status_atom:?}");
    assert!(status_atom.inadmissible.is_empty());
    let status_option = &status_atom.options[0];
    assert_eq!(status_option.technique, Technique::SelfDerivedColumnAdd);
    assert_eq!(status_option.slot, Some(HSlot::Alter));
    assert_eq!(status_option.statements.len(), 2, "{status_option:?}");
    assert!(status_option.statements[0].starts_with("ALTER TABLE t ADD COLUMN status"));
    assert!(status_option.statements[1].starts_with("UPDATE t SET status ="));

    let total_atom = options
        .atoms
        .iter()
        .find(|a| matches!(&a.change, AtomicChange::AddedColumn { name } if name == "total"))
        .expect("total atom");
    assert_eq!(total_atom.options.len(), 1, "options: {total_atom:?}");
    let total_option = &total_atom.options[0];
    assert!(total_option.statements[1].contains("price * qty"));
}

#[test]
fn b2_clean_rename_pairs_and_no_b1_atom_remains() {
    let before_sql = "SELECT id, amount FROM orders";
    let after_sql = "SELECT id, amount AS total FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    match &atom.change {
        AtomicChange::RenamedColumn { from, to } => {
            assert_eq!(from, "amount");
            assert_eq!(to, "total");
        }
        other => panic!("expected a RenamedColumn atom, got {other:?}"),
    }
    assert_eq!(atom.options.len(), 1, "{atom:?}");
    let option = &atom.options[0];
    assert_eq!(option.technique, Technique::Rename);
    assert_eq!(option.slot, Some(HSlot::Rename));
    assert_eq!(
        option.statements,
        vec!["ALTER TABLE t RENAME COLUMN amount TO total"]
    );
    assert!(atom.inadmissible.is_empty());
}

#[test]
fn b2_ambiguous_rename_refuses() {
    // Two dropped columns share an identical expression; one added column
    // matches — ambiguous which dropped column it came from (research §7.2).
    let before_sql = "SELECT id, amount AS old1, amount AS old2 FROM orders";
    let after_sql = "SELECT id, amount AS new1 FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));

    let ambiguous: Vec<_> = options
        .atoms
        .iter()
        .filter(|a| {
            a.inadmissible
                .iter()
                .any(|r| r.reason.to_lowercase().contains("ambiguous"))
        })
        .collect();
    assert_eq!(
        ambiguous.len(),
        2,
        "expected both dropped columns to refuse ambiguously, got {:?}",
        options.atoms
    );
    for atom in &ambiguous {
        assert!(atom.options.is_empty());
    }
    // Neither dropped column is ever offered as a rename anywhere in the
    // option set.
    assert!(!options
        .atoms
        .iter()
        .any(|a| matches!(&a.change, AtomicChange::RenamedColumn { .. })));
}

#[test]
fn b2_one_dropped_two_added_pins_lexicographic() {
    // One dropped column, two added columns with an identical expression:
    // the lexicographically-first added name ("a" < "b") takes the rename;
    // "b" classifies as B1 reading the renamed column "a".
    let before_sql = "SELECT id, amount AS old FROM orders";
    let after_sql = "SELECT id, amount AS b, amount AS a FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[("b", "INTEGER")]));

    let rename_atom = options
        .atoms
        .iter()
        .find(|a| matches!(&a.change, AtomicChange::RenamedColumn { .. }))
        .expect("a rename atom");
    match &rename_atom.change {
        AtomicChange::RenamedColumn { from, to } => {
            assert_eq!(from, "old");
            assert_eq!(to, "a");
        }
        _ => unreachable!(),
    }
    assert_eq!(rename_atom.options.len(), 1);

    let b_atom = options
        .atoms
        .iter()
        .find(|a| matches!(&a.change, AtomicChange::AddedColumn { name } if name == "b"))
        .expect("b atom");
    assert_eq!(b_atom.options.len(), 1, "b_atom: {b_atom:?}");
    assert!(b_atom.inadmissible.is_empty());
    let b_option = &b_atom.options[0];
    assert_eq!(b_option.technique, Technique::SelfDerivedColumnAdd);
    assert_eq!(b_option.statements.len(), 2);
    assert!(
        b_option.statements[1].contains("SET b = a"),
        "expected b's update to read the renamed column 'a' directly, got: {:?}",
        b_option.statements
    );

    // Exactly two atoms: the rename and b's B1-via-rename atom.
    assert_eq!(options.atoms.len(), 2, "atoms: {:?}", options.atoms);
}

// ===== D1 (task-4-brief.md) =====

#[test]
fn d1_new_expr_reading_own_old_value_refuses() {
    // The new expression's only dependency is the column's own (changed)
    // name — never a legitimate representative, so this must refuse rather
    // than blindly substituting the old value.
    let before_sql = "SELECT id, amount FROM orders";
    let after_sql = "SELECT id, amount * 1.1 AS amount FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert!(matches!(
        &atom.change,
        AtomicChange::ChangedColumn { name } if name == "amount"
    ));
    assert_refused(atom);
    assert!(
        atom.inadmissible[0].reason.contains("amount"),
        "reason: {}",
        atom.inadmissible[0].reason
    );
}

#[test]
fn d1_swapped_columns_refuse() {
    // `x AS a, y AS b` -> `y AS a, x AS b`: a weaker "not its own value"
    // rule would admit both (neither new expression reads its own old
    // name), but the uniform representative rule refuses both — `a` and
    // `b` are both `changed`, so neither `x` nor `y` (referenced only via
    // the other's old name) has a stored representative.
    let before_sql = "SELECT id, x AS a, y AS b FROM orders";
    let after_sql = "SELECT id, y AS a, x AS b FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    assert_eq!(options.atoms.len(), 2, "atoms: {:?}", options.atoms);
    for atom in &options.atoms {
        assert!(matches!(&atom.change, AtomicChange::ChangedColumn { .. }));
        assert_refused(atom);
    }
}

#[test]
fn d1_lateral_alias_to_changed_sibling_refuses() {
    // `net` is derivable straight from stored `amount` and admits D1; `gross`
    // reads `net`'s *new* value via a bare reference to its output alias —
    // but `net` itself changed, so it is not a representative, and `gross`
    // must refuse rather than treat a same-select-list alias as stored.
    let before_sql = "SELECT id, amount, amount AS net, amount AS gross FROM orders";
    let after_sql = "SELECT id, amount, amount * 0.9 AS net, net * 1.05 AS gross FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    assert_eq!(options.atoms.len(), 2, "atoms: {:?}", options.atoms);

    let net_atom = options
        .atoms
        .iter()
        .find(|a| matches!(&a.change, AtomicChange::ChangedColumn { name } if name == "net"))
        .expect("net atom");
    assert_eq!(net_atom.options.len(), 1, "net_atom: {net_atom:?}");
    assert!(net_atom.inadmissible.is_empty());
    assert_eq!(
        net_atom.options[0].technique,
        Technique::SelfDerivedColumnRewrite
    );

    let gross_atom = options
        .atoms
        .iter()
        .find(|a| matches!(&a.change, AtomicChange::ChangedColumn { name } if name == "gross"))
        .expect("gross atom");
    assert_refused(gross_atom);
    assert!(
        gross_atom.inadmissible[0].reason.contains("net"),
        "expected the refusal to name the changed sibling 'net', got: {}",
        gross_atom.inadmissible[0].reason
    );
    // A changed-sibling dependency has nothing to do with an upstream read
    // (research §4 D2 is specifically "needs an upstream read"; `net` is a
    // same-model column, not an upstream one) — the refusal must not point
    // at D2, unlike the genuinely-upstream case
    // (`d1_upstream_dependency_refuses_until_d2`).
    assert!(
        !gross_atom.inadmissible[0].reason.contains("D2"),
        "a changed-sibling refusal must not be conflated with the upstream-read (D2) case, got: {}",
        gross_atom.inadmissible[0].reason
    );
}

#[test]
fn d1_distinct_model_refuses() {
    let before_sql = "SELECT DISTINCT id, amount, amount AS amount_usd FROM orders";
    let after_sql = "SELECT DISTINCT id, amount, amount * 1.1 AS amount_usd FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible[0]
            .reason
            .to_lowercase()
            .contains("distinct"),
        "reason: {}",
        atom.inadmissible[0].reason
    );
}

#[test]
fn d1_limit_model_refuses() {
    let before_sql = "SELECT id, amount, amount AS amount_usd FROM orders LIMIT 10";
    let after_sql = "SELECT id, amount, amount * 1.1 AS amount_usd FROM orders LIMIT 10";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible[0].reason.to_lowercase().contains("limit"),
        "reason: {}",
        atom.inadmissible[0].reason
    );
}

#[test]
fn d1_upstream_dependency_refuses_until_d2() {
    let before_sql = "SELECT o.id AS id, o.amount AS amount FROM orders o \
                       JOIN regions r ON o.region_id = r.region_id";
    let after_sql = "SELECT o.id AS id, r.region_name AS amount FROM orders o \
                      JOIN regions r ON o.region_id = r.region_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert!(matches!(
        &atom.change,
        AtomicChange::ChangedColumn { name } if name == "amount"
    ));
    assert_refused(atom);
    assert!(
        atom.inadmissible[0].reason.contains("D2"),
        "expected the refusal to point at D2 (upstream read), got: {}",
        atom.inadmissible[0].reason
    );
    // Distinguishable from the changed-sibling case
    // (`d1_lateral_alias_to_changed_sibling_refuses`): a genuinely
    // upstream-only dependency was never a changed column in this edit.
    assert!(
        !atom.inadmissible[0]
            .reason
            .contains("also changed in this same edit"),
        "an upstream-only refusal must not be conflated with the changed-sibling case, got: {}",
        atom.inadmissible[0].reason
    );
}

// ===== B3 (task-5-brief.md) =====

fn inputs_with_sources(
    added_column_types: &[(&str, &str)],
    sources: &[(&str, SourceRef)],
) -> BackbuildInputs {
    let mut built = inputs(added_column_types);
    built.sources = sources
        .iter()
        .map(|(alias, source)| (alias.to_string(), source.clone()))
        .collect();
    built
}

fn source_ref(
    physical_name: &str,
    unique_key: Option<&[&str]>,
    not_null_columns: &[&str],
) -> SourceRef {
    SourceRef {
        physical_name: physical_name.to_string(),
        unique_key: unique_key.map(|k| k.iter().map(|s| s.to_string()).collect()),
        not_null_columns: not_null_columns.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn b3_missing_key_pullthrough_refuses() {
    // The output never pulls `order_id` through at all — there is nothing
    // to bind the equality backfill's key to.
    let before_sql = "SELECT o.customer AS customer FROM orders o";
    let after_sql = "SELECT o.customer AS customer, o.discount AS discount FROM orders o";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = inputs_with_sources(
        &[("discount", "INTEGER")],
        &[(
            "o",
            source_ref("orders", Some(&["order_id"]), &["order_id"]),
        )],
    );
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.contains("no addressable identity")),
        "expected a 'no addressable identity' refusal, got: {:?}",
        atom.inadmissible
    );
}

#[test]
fn b3_undeclared_unique_key_refuses() {
    let before_sql = "SELECT o.order_id AS order_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.discount AS discount FROM orders o";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = inputs_with_sources(
        &[("discount", "INTEGER")],
        &[("o", source_ref("orders", None, &[]))],
    );
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.to_lowercase().contains("unique_key")),
        "expected a refusal naming the missing unique_key, got: {:?}",
        atom.inadmissible
    );
}

#[test]
fn b3_nullable_key_refuses() {
    // `unique_key` is declared and pulled through, but not declared NOT
    // NULL — SQL UNIQUE admits NULLs, and an equality backfill never
    // addresses a NULL-keyed row (research §4 "Key addressability").
    let before_sql = "SELECT o.order_id AS order_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.discount AS discount FROM orders o";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = inputs_with_sources(
        &[("discount", "INTEGER")],
        &[("o", source_ref("orders", Some(&["order_id"]), &[]))],
    );
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.to_lowercase().contains("not null")),
        "expected a NOT NULL refusal, got: {:?}",
        atom.inadmissible
    );
}

#[test]
fn b3_self_join_binds_per_alias() {
    // Only `o1.order_id` is pulled through; adding `o2.discount` must not
    // be licensed by `o1`'s key pull-through even though both aliases
    // share the same physical table and both have a declared, NOT-NULL
    // unique_key — the proof binds per FROM-tree alias, not per table (a
    // table-level match would backfill o1's discount where the rebuild
    // wants o2's).
    let before_sql =
        "SELECT o1.order_id AS order_id FROM orders o1 JOIN orders o2 ON o1.order_id = o2.order_id";
    let after_sql = "SELECT o1.order_id AS order_id, o2.discount AS discount FROM orders o1 \
                      JOIN orders o2 ON o1.order_id = o2.order_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = inputs_with_sources(
        &[("discount", "INTEGER")],
        &[
            (
                "o1",
                source_ref("orders", Some(&["order_id"]), &["order_id"]),
            ),
            (
                "o2",
                source_ref("orders", Some(&["order_id"]), &["order_id"]),
            ),
        ],
    );
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.contains("no addressable identity")),
        "expected a 'no addressable identity' refusal (o1's pull-through must not license o2), \
         got: {:?}",
        atom.inadmissible
    );
}

// ===== B4 (task-6-brief.md) =====

fn dims_inputs(
    added_column_types: &[(&str, &str)],
    unique_key: Option<&[&str]>,
    not_null_columns: &[&str],
) -> BackbuildInputs {
    inputs_with_sources(
        added_column_types,
        &[("d", source_ref("dims", unique_key, not_null_columns))],
    )
}

#[test]
fn b4_join_key_not_stored_refuses() {
    // `order_id` is never pulled through at all — nothing to bind the
    // added join's fact-side key to.
    let before_sql = "SELECT o.customer AS customer FROM orders o";
    let after_sql = "SELECT o.customer AS customer, d.name AS dim_name FROM orders o LEFT JOIN \
                      dims d ON o.order_id = d.order_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = dims_inputs(&[("dim_name", "TEXT")], Some(&["order_id"]), &["order_id"]);
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.contains("order_id") && r.reason.contains("stored")),
        "expected a named, actionable refusal naming 'order_id', got: {:?}",
        atom.inadmissible
    );
}

#[test]
fn b4_on_beyond_bare_key_equality_refuses() {
    let before_sql = "SELECT o.order_id AS order_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, d.name AS dim_name FROM orders o LEFT JOIN \
                      dims d ON o.order_id = d.order_id AND d.active";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = dims_inputs(&[("dim_name", "TEXT")], Some(&["order_id"]), &["order_id"]);
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.to_lowercase().contains("bare key equality")),
        "expected a refusal naming the ON condition, got: {:?}",
        atom.inadmissible
    );
}

#[test]
fn b4_nullable_join_key_refuses() {
    let before_sql = "SELECT o.order_id AS order_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, d.name AS dim_name FROM orders o LEFT JOIN \
                      dims d ON o.order_id = d.order_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    // `unique_key` is declared, but the key column is not declared NOT NULL
    // (research §4 "Key addressability").
    let inputs = dims_inputs(&[("dim_name", "TEXT")], Some(&["order_id"]), &[]);
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.to_lowercase().contains("not null")),
        "expected a NOT NULL refusal, got: {:?}",
        atom.inadmissible
    );
}

#[test]
fn b4_inner_join_refuses() {
    // A bare `JOIN` (no LEFT keyword) defaults to INNER — `diff.rs` never
    // even produces `SkeletonDiff::AddedLeftJoins` for it (it lands in
    // `SkeletonDiff::Changed`'s G2 refusal), so this never reaches B4's own
    // admission code at all.
    let before_sql = "SELECT o.order_id AS order_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, d.name AS dim_name FROM orders o JOIN dims \
                      d ON o.order_id = d.order_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = dims_inputs(&[("dim_name", "TEXT")], Some(&["order_id"]), &["order_id"]);
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = single_atom(&options);
    assert_refused(atom);
}

#[test]
fn b4_nonunique_dim_key_refuses() {
    let before_sql = "SELECT o.order_id AS order_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, d.name AS dim_name FROM orders o LEFT JOIN \
                      dims d ON o.order_id = d.order_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = dims_inputs(&[("dim_name", "TEXT")], None, &[]);
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.to_lowercase().contains("unique_key")),
        "expected a refusal naming the missing unique_key, got: {:?}",
        atom.inadmissible
    );
}

#[test]
fn b4_alias_referenced_in_where_refuses() {
    // The new alias 'd' is referenced in the WHERE clause — the join is no
    // longer a pure enrichment (the row set is no longer preserved by the
    // join alone), even though the join's own row-set-preservation proof
    // (LEFT + unique + NOT NULL + bare key equality) otherwise holds.
    let before_sql = "SELECT o.order_id AS order_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, d.name AS dim_name FROM orders o LEFT JOIN \
                      dims d ON o.order_id = d.order_id WHERE d.active = TRUE";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = dims_inputs(&[("dim_name", "TEXT")], Some(&["order_id"]), &["order_id"]);
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.to_lowercase().contains("where")),
        "expected a refusal naming the WHERE reference, got: {:?}",
        atom.inadmissible
    );
}

// ===== E1/E4 (task-7-brief.md) =====

#[test]
fn e1_conjunct_over_unstored_column_refuses() {
    // `status` is never selected at all — no stored representative to
    // requalify the added conjunct against.
    let before_sql = "SELECT id FROM orders";
    let after_sql = "SELECT id FROM orders WHERE status = 'active'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert!(
        matches!(&atom.change, AtomicChange::AddedConjunct { index: 0 }),
        "expected an AddedConjunct atom, got {:?}",
        atom.change
    );
    assert_refused(atom);
    assert!(
        atom.inadmissible[0].reason.contains("representative"),
        "expected the refusal to name the missing stored representative, got: {}",
        atom.inadmissible[0].reason
    );
}

#[test]
fn e_opaque_predicate_rewrite_refuses() {
    // `a OR b` -> `a`: a top-level OR is not a conjunctive predicate — a
    // conjunct-set add/remove framing is unsound for a non-conjunctive
    // rewrite (research §4 E-class intro).
    let before_sql = "SELECT id, status FROM orders WHERE status = 'a' OR status = 'b'";
    let after_sql = "SELECT id, status FROM orders WHERE status = 'a'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible[0]
            .reason
            .to_lowercase()
            .contains("conjunctive"),
        "reason: {}",
        atom.inadmissible[0].reason
    );
}

#[test]
fn e_group_by_model_refuses() {
    // A predicate change under GROUP BY re-partitions inputs to aggregates,
    // not stored rows — refused even though this shape (an equality swap,
    // not a range widening) isn't the E4 group-key carve-out either way.
    let before_sql = "SELECT status, COUNT(*) AS n FROM orders WHERE status = 'a' GROUP BY status";
    let after_sql = "SELECT status, COUNT(*) AS n FROM orders WHERE status = 'b' GROUP BY status";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible[0]
            .reason
            .to_lowercase()
            .contains("group by"),
        "reason: {}",
        atom.inadmissible[0].reason
    );
}

#[test]
fn e_distinct_model_refuses() {
    let before_sql = "SELECT DISTINCT id, status FROM orders";
    let after_sql = "SELECT DISTINCT id, status FROM orders WHERE status = 'active'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible[0]
            .reason
            .to_lowercase()
            .contains("distinct"),
        "reason: {}",
        atom.inadmissible[0].reason
    );
}

#[test]
fn e_limit_model_refuses() {
    // The LIMIT is identical on both sides, so the skeleton diff stays
    // `Unchanged` (LIMIT presence is otherwise invisible to it) — the
    // E-class guard must detect LIMIT from the CST directly.
    let before_sql = "SELECT id, status FROM orders LIMIT 10";
    let after_sql = "SELECT id, status FROM orders WHERE status = 'active' LIMIT 10";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible[0].reason.to_lowercase().contains("limit"),
        "reason: {}",
        atom.inadmissible[0].reason
    );
}

#[test]
fn e4_mixed_operator_refuses() {
    // `ts > X` -> `ts >= Y`: mixed comparison operators — boundary
    // semantics are not literal arithmetic, so this refuses rather than
    // trusting the literals alone (research §4 E4).
    let before_sql = "SELECT ts FROM events WHERE ts > '2025-01-01'";
    let after_sql = "SELECT ts FROM events WHERE ts >= '2024-01-01'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    let reason = atom.inadmissible[0].reason.to_lowercase();
    assert!(reason.contains("mixed"), "reason: {reason}");
    assert!(reason.contains("operator"), "reason: {reason}");
}

#[test]
fn b4_alias_in_opaque_where_refuses() {
    // A top-level OR makes the WHERE conjunct-set diff `Opaque` — no `added`
    // conjuncts to inspect at all, independently of whether the alias is
    // actually referenced. Without sweeping the retained raw before/after
    // WHERE expressions, this would pass the alias check vacuously and
    // admit B4 even though the join's row-set-preservation precondition
    // (nothing outside the added columns references the new alias) does
    // not hold: `o.order_id = 1 OR d.active` means some fact rows are kept
    // or dropped based on the dimension side, which a plain LEFT JOIN
    // enrichment backfill does not reproduce.
    let before_sql = "SELECT o.order_id AS order_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, d.name AS dim_name FROM orders o LEFT JOIN \
                      dims d ON o.order_id = d.order_id WHERE o.order_id = 1 OR d.active";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = dims_inputs(&[("dim_name", "TEXT")], Some(&["order_id"]), &["order_id"]);
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.to_lowercase().contains("where")),
        "expected a refusal naming the WHERE reference even though the WHERE diff is opaque \
         (a top-level OR), got: {:?}",
        atom.inadmissible
    );
}
