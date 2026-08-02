//! Pure (no DuckDB) tests for the backbuild option-set data model:
//! `smelt_logical::backbuild::{BackbuildOptions, AtomAnalysis, assemble}`.
//! See `docs/research/20260802-backbuild-synthesis.md` §2 ("Refusal
//! posture": "partial application is never offered; an atom with an empty
//! option set leaves FullRefresh as the model's only option") and
//! `.superpowers/sdd/20260802-backbuild-synthesis/task-2-brief.md`.

use std::collections::{BTreeMap, BTreeSet};

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
        not_null_columns: BTreeSet::new(),
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
        not_null_columns: BTreeSet::new(),
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
    // B3 is attempted independently and refuses (a literal constant has no
    // column dependency to bind an upstream alias to) but coexists with
    // the admitted B1 option (`b_dual_derivable_refusals_still_recorded`).
    assert_eq!(
        status_atom.inadmissible.len(),
        1,
        "{:?}",
        status_atom.inadmissible
    );
    assert!(status_atom.inadmissible[0].reason.contains("B3"));
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
fn b_dual_derivable_refusals_still_recorded() {
    // `amount_copy` is a bare copy of `amount`, which is already a stored,
    // unchanged bare pull-through of alias `o` — B1 is admissible
    // regardless of `o`'s declared keys. B3 is attempted independently and
    // refuses (no declared `unique_key`, same shape as
    // `b3_undeclared_unique_key_refuses`). Options and refusals are not
    // mutually exclusive (research §2 "Options, not choices"): the atom
    // must carry the admitted B1 option *and* the B3 refusal.
    let before_sql = "SELECT o.order_id AS order_id, o.amount AS amount FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.amount AS amount, o.amount AS amount_copy \
                      FROM orders o";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = inputs_with_sources(
        &[("amount_copy", "INTEGER")],
        &[("o", source_ref("orders", None, &[]))],
    );
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = single_atom(&options);

    assert_eq!(
        atom.options.len(),
        1,
        "expected only the B1 option to be admitted, got {:?}",
        atom.options
    );
    assert_eq!(atom.options[0].technique, Technique::SelfDerivedColumnAdd);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.contains("B3") && r.reason.to_lowercase().contains("unique_key")),
        "expected the B3 refusal to still be recorded alongside the admitted B1 option, got: \
         {:?}",
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

// ===== B5 (research §4 B5: new aggregate column at unchanged GROUP BY
// grain) =====

/// `BackbuildInputs` for a B5 refusal test: no declared `sources` (mirrors
/// `backbuild_conformance.rs`'s `group_by_inputs` — B5's re-aggregation
/// reads the model's own FROM/WHERE text directly, and leaving `sources`
/// empty keeps B3 from ever admitting the same aggregate expression
/// instead, since `admit_upstream_pullthrough` always refuses without a
/// declared source).
fn group_by_inputs(
    added_column_types: &[(&str, &str)],
    not_null_group_keys: &[&str],
) -> BackbuildInputs {
    let mut built = inputs(added_column_types);
    built.not_null_columns = not_null_group_keys.iter().map(|s| s.to_string()).collect();
    built
}

#[test]
fn b5_nullable_group_key_refuses() {
    let before_sql = "SELECT o.customer_id AS customer_id, count(*) AS n FROM orders o GROUP BY \
         o.customer_id";
    let after_sql = "SELECT o.customer_id AS customer_id, count(*) AS n, SUM(o.qty) AS \
                      total_qty FROM orders o GROUP BY o.customer_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    // `customer_id` is pulled through and unchanged, but not declared NOT
    // NULL.
    let options =
        derive_backbuild_options(&diff, &group_by_inputs(&[("total_qty", "HUGEINT")], &[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.contains("B5") && r.reason.to_lowercase().contains("not null")),
        "expected a B5 NOT NULL refusal for the GROUP BY key, got: {:?}",
        atom.inadmissible
    );
}

#[test]
fn b5_group_key_not_pulled_through_refuses() {
    // The model groups by `o.region`, but never selects it — there is no
    // stored bare pull-through to address a backfill by.
    let before_sql = "SELECT count(*) AS n FROM orders o GROUP BY o.region";
    let after_sql = "SELECT count(*) AS n, SUM(o.qty) AS total_qty FROM orders o GROUP BY \
                      o.region";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options =
        derive_backbuild_options(&diff, &group_by_inputs(&[("total_qty", "HUGEINT")], &[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.contains("B5") && r.reason.contains("no addressable identity")),
        "expected a B5 'no addressable identity' refusal for the ungrouped-key, got: {:?}",
        atom.inadmissible
    );
}

#[test]
fn b5_volatile_aggregate_input_refuses() {
    let before_sql = "SELECT o.customer_id AS customer_id, count(*) AS n FROM orders o GROUP BY \
         o.customer_id";
    let after_sql = "SELECT o.customer_id AS customer_id, count(*) AS n, SUM(random()) AS \
                      total_qty FROM orders o GROUP BY o.customer_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(
        &diff,
        &group_by_inputs(&[("total_qty", "HUGEINT")], &["customer_id"]),
    );
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.contains("B5")
                && r.reason.to_lowercase().contains("non-deterministic")),
        "expected a B5 refusal naming the non-deterministic aggregate input, got: {:?}",
        atom.inadmissible
    );
}

#[test]
fn b5_changed_skeleton_refuses() {
    // The aggregate add is bundled with a FROM-tree change (a newly added
    // INNER JOIN, a join-multiplicity change) — the shared skeleton guard
    // refuses the whole diff before B5 (or any other targeted technique)
    // ever gets a chance: the row-set-unchanged proof B5 needs simply does
    // not hold. The refused atom is the coarse skeleton atom, not a
    // per-column B5 atom — "the atom lands wherever the skeleton diff
    // sends it" (research §4 B5).
    let before_sql = "SELECT o.customer_id AS customer_id, count(*) AS n FROM orders o GROUP BY \
         o.customer_id";
    let after_sql = "SELECT o.customer_id AS customer_id, count(*) AS n, SUM(o.qty) AS \
                      total_qty FROM orders o JOIN regions r ON o.customer_id = r.customer_id \
                      GROUP BY o.customer_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(
        &diff,
        &group_by_inputs(&[("total_qty", "HUGEINT")], &["customer_id"]),
    );
    let atom = single_atom(&options);
    assert!(matches!(&atom.change, AtomicChange::Skeleton { .. }));
    assert_refused(atom);
}

// ===== B6 (research §4 B6) =====

/// `BackbuildInputs` for a B6 window-column test: `identity` sets both
/// `row_identity` and, when `not_null` is `true`, the matching
/// `not_null_columns` entries (a declared identity that is not provably NOT
/// NULL must be treated as unaddressable — `b6_nullable_identity_refuses`
/// pins this by passing `not_null: false`).
fn window_inputs(
    added_column_types: &[(&str, &str)],
    identity: Option<&[&str]>,
    not_null: bool,
) -> BackbuildInputs {
    let mut built = inputs(added_column_types);
    built.row_identity = identity.map(|cols| cols.iter().map(|s| s.to_string()).collect());
    if not_null {
        if let Some(cols) = identity {
            built.not_null_columns = cols.iter().map(|s| s.to_string()).collect();
        }
    }
    built
}

#[test]
fn b6_no_row_identity_refuses() {
    let before_sql = "SELECT o.order_id AS order_id, o.status AS status FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.status AS status, ROW_NUMBER() OVER \
                      (PARTITION BY status ORDER BY order_id) AS rn FROM orders o";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &window_inputs(&[("rn", "BIGINT")], None, false));
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.contains("B6") && r.reason.contains("row_identity")),
        "expected a B6 refusal naming the missing row_identity, got: {:?}",
        atom.inadmissible
    );
}

#[test]
fn b6_nullable_identity_refuses() {
    let before_sql = "SELECT o.order_id AS order_id, o.status AS status FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.status AS status, ROW_NUMBER() OVER \
                      (PARTITION BY status ORDER BY order_id) AS rn FROM orders o";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    // `row_identity` is declared but not proven NOT NULL.
    let options = derive_backbuild_options(
        &diff,
        &window_inputs(&[("rn", "BIGINT")], Some(&["order_id"]), false),
    );
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.contains("B6") && r.reason.to_lowercase().contains("not null")),
        "expected a B6 NOT NULL refusal for the identity column, got: {:?}",
        atom.inadmissible
    );
}

#[test]
fn b6_window_reading_unstored_column_refuses() {
    // `region` is never pulled through into the model's own output — the
    // window's PARTITION BY has no addressable stored representative.
    let before_sql = "SELECT o.order_id AS order_id, o.status AS status FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.status AS status, ROW_NUMBER() OVER \
                      (PARTITION BY o.region ORDER BY order_id) AS rn FROM orders o";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(
        &diff,
        &window_inputs(&[("rn", "BIGINT")], Some(&["order_id"]), true),
    );
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible.iter().any(|r| r.reason.contains("B6")
            && r.reason.contains("region")
            && r.reason.contains("no 1:1 stored representative")),
        "expected a B6 refusal naming the unstored 'region' dependency, got: {:?}",
        atom.inadmissible
    );
}

#[test]
fn b6_nondeterministic_order_refuses() {
    // No ORDER BY in the OVER clause — the window's draw within each
    // partition is underdetermined and can never be proven equal to a
    // rebuild's own draw (research §4 B6, §2 "Determinism caveat").
    let before_sql = "SELECT o.order_id AS order_id, o.status AS status FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.status AS status, ROW_NUMBER() OVER \
                      (PARTITION BY status) AS rn FROM orders o";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(
        &diff,
        &window_inputs(&[("rn", "BIGINT")], Some(&["order_id"]), true),
    );
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.contains("B6") && r.reason.contains("ORDER BY")),
        "expected a B6 refusal naming the missing ORDER BY, got: {:?}",
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
fn g2_bare_join_defaults_to_inner_refuses() {
    // Renamed from `b4_inner_join_refuses` (final-review finding I2): this
    // was misnamed and reason-blind — despite the name, it refuses at the
    // *skeleton* level (G2, join-multiplicity change), never reaching B4's
    // own admission code at all, and the original assertion never pinned
    // the reason substring. A bare `JOIN` (no LEFT keyword) defaults to
    // INNER — `diff.rs`'s `diff_joins` only ever recognises an *added* LEFT
    // JOIN as `SkeletonDiff::AddedLeftJoins`; anything else (including a
    // bare/INNER added join) routes to `SkeletonDiff::Changed`, refused by
    // `classify_skeleton_reason` with a "G2" label before B4's
    // `admit_added_left_join` is ever called.
    //
    // Whether a *true* B4-level INNER-join refusal is reachable at all: it
    // is not. `admit_added_left_join`'s own `join.join_type() !=
    // Some(JoinType::Left)` check is explicitly documented as "Defensive
    // only ... never reachable in practice", because `diff_joins` never
    // constructs `SkeletonDiff::AddedLeftJoins` for a non-LEFT added join in
    // the first place — every diff shape that could reach B4's admission
    // code already has skeleton = `AddedLeftJoins`, which by construction
    // only ever contains LEFT joins. So there is no diff shape that gets
    // past G2 with an added INNER join and an otherwise-unchanged skeleton;
    // this skeleton-level refusal is the only one reachable, hence it is
    // what this test (correctly, now) pins.
    let before_sql = "SELECT o.order_id AS order_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, d.name AS dim_name FROM orders o JOIN dims \
                      d ON o.order_id = d.order_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = dims_inputs(&[("dim_name", "TEXT")], Some(&["order_id"]), &["order_id"]);
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible.iter().any(|r| r.reason.contains("G2")),
        "expected a G2-labelled refusal (join-multiplicity change), got: {:?}",
        atom.inadmissible
    );
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

// ===== B7 (task-9-brief.md) =====

fn two_dims_inputs(
    added_column_types: &[(&str, &str)],
    d1_unique_key: Option<&[&str]>,
    d1_not_null: &[&str],
    d2_unique_key: Option<&[&str]>,
    d2_not_null: &[&str],
) -> BackbuildInputs {
    inputs_with_sources(
        added_column_types,
        &[
            ("d1", source_ref("dim1", d1_unique_key, d1_not_null)),
            ("d2", source_ref("dim2", d2_unique_key, d2_not_null)),
        ],
    )
}

#[test]
fn b7_unstored_intermediate_refuses() {
    // dim2 keys on `d1.key_col`, but the model never pulls `d1.key_col`
    // through to the output at all (bare or otherwise) — there is nothing
    // to bind dim2's fact-side key to. dim1's own admission (keyed on the
    // pre-existing, stored `order_id`) is otherwise unimpeachable, so this
    // pins the refusal to dim2 specifically.
    let before_sql = "SELECT o.order_id AS order_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, d2.name AS dim_name FROM orders o LEFT JOIN \
                      dim1 d1 ON o.order_id = d1.order_id LEFT JOIN dim2 d2 ON d1.key_col = \
                      d2.key_col";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = two_dims_inputs(
        &[("dim_name", "TEXT")],
        Some(&["order_id"]),
        &["order_id"],
        Some(&["key_col"]),
        &["key_col"],
    );
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible.iter().any(|r| r.reason.contains("d2")),
        "expected the refusal to name the failing join 'd2', got: {:?}",
        atom.inadmissible
    );
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.contains("key_col") && r.reason.contains("stored")),
        "expected a named, actionable refusal naming the missing column 'key_col', got: {:?}",
        atom.inadmissible
    );
}

#[test]
fn b7_nonbare_intermediate_refuses() {
    // `key_col` IS pulled through, but wrapped (`COALESCE(d1.key_col, 0)`)
    // rather than bare — the carrier stores `0` where a rebuild would leave
    // NULL, so dim2's join on it could hit a dim row the rebuild's NULL key
    // never would (research §4 B7 bareness). Bareness is enforced purely by
    // construction: a wrapped carrier is never turned into a representative
    // at all, so dim2's admission fails the same "no stored bare
    // representative" check an entirely-unstored column would.
    let before_sql = "SELECT o.order_id AS order_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, COALESCE(d1.key_col, 0) AS key_col, \
                      d2.name AS dim_name FROM orders o LEFT JOIN dim1 d1 ON o.order_id = \
                      d1.order_id LEFT JOIN dim2 d2 ON d1.key_col = d2.key_col";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = two_dims_inputs(
        &[("key_col", "INTEGER"), ("dim_name", "TEXT")],
        Some(&["order_id"]),
        &["order_id"],
        Some(&["key_col"]),
        &["key_col"],
    );
    let options = derive_backbuild_options(&diff, &inputs);
    // The wrapped `key_col` column would otherwise be independently
    // admissible on its own merits (a valid B4 scalar-subquery backfill) —
    // but dim2's join can never be licensed, and partial application is
    // never offered (research §2 "Refusal posture"), so the whole diff
    // still refuses as a single atom.
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.contains("key_col")),
        "expected a refusal naming the unaddressable column 'key_col', got: {:?}",
        atom.inadmissible
    );
}

#[test]
fn b7_per_join_proof_still_enforced() {
    // dim1 admits cleanly (keyed on the pre-existing, stored `order_id`);
    // dim2's own row-set-preservation proof otherwise holds too, except its
    // alias leaks into the WHERE clause — B4's three legs (LEFT + unique +
    // NOT NULL + bare key equality + nothing else references the alias)
    // hold *per join*, not just for the first one in the chain.
    let before_sql = "SELECT o.order_id AS order_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, d1.name AS dim1_name, d2.name AS dim2_name \
                      FROM orders o LEFT JOIN dim1 d1 ON o.order_id = d1.order_id LEFT JOIN dim2 \
                      d2 ON o.order_id = d2.order_id WHERE d2.active = TRUE";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let inputs = two_dims_inputs(
        &[("dim1_name", "TEXT"), ("dim2_name", "TEXT")],
        Some(&["order_id"]),
        &["order_id"],
        Some(&["order_id"]),
        &["order_id"],
    );
    let options = derive_backbuild_options(&diff, &inputs);
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.contains("d2") && r.reason.to_lowercase().contains("where")),
        "expected a refusal naming join 'd2' and the WHERE reference, got: {:?}",
        atom.inadmissible
    );

    // The second leg of "B4's three legs hold per join": an added join that
    // is not a LEFT JOIN at all is never even classified as
    // `SkeletonDiff::AddedLeftJoins` (`diff.rs` requires *every* added join
    // to be LEFT) — it refuses one level up, as a skeleton (G2) change, but
    // is still a named, fail-closed refusal rather than a silent
    // misclassification as an admissible B7 chain.
    let inner_after_sql = "SELECT o.order_id AS order_id, d1.name AS dim1_name, d2.name AS \
                            dim2_name FROM orders o LEFT JOIN dim1 d1 ON o.order_id = d1.order_id \
                            JOIN dim2 d2 ON o.order_id = d2.order_id";
    let inner_diff = definition_diff(&parse(before_sql), &parse(inner_after_sql));
    assert!(!inner_diff.is_noop());
    let inner_options = derive_backbuild_options(&inner_diff, &inputs);
    let inner_atom = single_atom(&inner_options);
    assert_refused(inner_atom);
    assert!(
        inner_atom
            .inadmissible
            .iter()
            .any(|r| r.reason.to_lowercase().contains("left join")),
        "expected a refusal naming the non-LEFT added join, got: {:?}",
        inner_atom.inadmissible
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
fn e4_nonpadded_date_literal_refuses() {
    // The counter-example: `'2025-9-1'` (Sept 1) -> `'2025-10-1'` (Oct 1) is
    // chronologically a NARROWING for a `>=` lower bound (the threshold
    // moved later), but `'2025-10-1' < '2025-9-1'` lexicographically (`'1'`
    // < `'9'` at the first differing byte) — a naive lexicographic compare
    // would wrongly admit this as "widened". Neither literal is a
    // fixed-width `YYYY-MM-DD` (single-digit month/day), so this must
    // refuse rather than trust the comparison.
    let before_sql = "SELECT ts FROM events WHERE ts >= '2025-9-1'";
    let after_sql = "SELECT ts FROM events WHERE ts >= '2025-10-1'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    let reason = atom.inadmissible[0].reason.to_lowercase();
    assert!(
        reason.contains("iso-8601") || reason.contains("fixed-width"),
        "reason: {reason}"
    );
}

#[test]
fn e4_quoted_numeric_literal_refuses() {
    // `'9'` -> `'100'`: chronologically/numerically a NARROWING for a `>=`
    // lower bound (100 > 9), but `'100' < '9'` lexicographically (`'1'` <
    // `'9'`) — a naive lexicographic compare would wrongly admit this as
    // "widened". Quoted plain numbers are not a fixed-width ISO-8601
    // date/timestamp, so this must refuse.
    let before_sql = "SELECT id FROM events WHERE id >= '9'";
    let after_sql = "SELECT id FROM events WHERE id >= '100'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    let reason = atom.inadmissible[0].reason.to_lowercase();
    assert!(
        reason.contains("iso-8601") || reason.contains("fixed-width"),
        "reason: {reason}"
    );
}

#[test]
fn e4_group_key_carve_out_refuses_across_multiple_aliases() {
    // Two aliases (`o`, `d`) expose a same-named `report_date` column; the
    // GROUP BY key is `o.report_date` but the range predicate is on
    // `d.report_date` — a genuinely different column that a bare-name-only
    // match would wrongly treat as the same group key. `SkeletonDiff`
    // proves the FROM tree *unchanged* between versions, not single-alias,
    // so the carve-out must refuse whenever more than one alias is joined
    // in, regardless of whether the names happen to coincide.
    let before_sql = "SELECT o.report_date AS report_date, SUM(o.amount) AS total FROM orders o \
                       JOIN dim_calendar d ON o.report_date = d.report_date \
                       WHERE d.report_date >= '2025-01-01' GROUP BY o.report_date";
    let after_sql = "SELECT o.report_date AS report_date, SUM(o.amount) AS total FROM orders o \
                      JOIN dim_calendar d ON o.report_date = d.report_date \
                      WHERE d.report_date >= '2024-01-01' GROUP BY o.report_date";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    let reason = atom.inadmissible[0].reason.to_lowercase();
    assert!(
        reason.contains("single") || reason.contains("alias"),
        "reason: {reason}"
    );
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

// ===== E3 (task-8 review fix) =====

#[test]
fn e3_same_column_pair_still_refuses() {
    // `status = 'a'` -> `status = 'b'`: added and removed conjuncts on the
    // SAME column, but not a recognised range-widening shape at all
    // (equality, not `>`/`>=`/`<`/`<=`) — the disjoint-column E3 middle path
    // must not over-reach here: same-column pairs stay exclusively E4's
    // territory (a proof like range-widening), refusing when no such proof
    // holds, same posture as `e4_mixed_operator_refuses` et al.
    let before_sql = "SELECT id, status FROM orders WHERE status = 'a'";
    let after_sql = "SELECT id, status FROM orders WHERE status = 'b'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    let reason = atom.inadmissible[0].reason.to_lowercase();
    assert!(
        reason.contains("e4") && reason.contains("column"),
        "expected a refusal naming the shared-column/E4 territory, got: {reason}"
    );

    let targeted = assemble(
        &options,
        &Selection::Targeted {
            atom_choices: vec![0],
        },
    );
    assert!(targeted.is_empty());
}

/// Regression: the disjointness gate must never be a *second* admission
/// check on top of E1's own — it only decides whether the composition is
/// attempted at all, by column name, never by whether an added conjunct is
/// independently well-formed (that's `classify_e1_conjunct`/`try_e1`'s own
/// job, via `requalify::requalify`, exercised on this same conjunct here).
/// The added conjunct (`created_at < NOW()`) calls a non-deterministic
/// function on a column disjoint from the removed conjunct's (`region`) —
/// with the pre-fix `analysis::model_diff::collect_dependencies`-based gate
/// this would have wrongly refused the whole composition (that walk rejects
/// `NOW()`), even though `requalify::requalify` (E1's actual validator)
/// never checks determinism and would admit it on its own.
#[test]
fn e3_disjoint_composition_tolerates_nondeterministic_added_conjunct() {
    // `NOW() - INTERVAL '1 day'` (not bare `NOW()`) deliberately: a bare
    // zero-arg function call as a *direct* comparison operand hits an
    // unrelated, pre-existing `Expr::as_column_ref`/`as_function_call`
    // AST-cast ambiguity (both return `Some` for the same `FUNCTION_CALL`
    // node in that position) that is orthogonal to this fix — the
    // `NOW() - INTERVAL` shape is the same one
    // `e2_nondeterministic_removed_conjunct_still_admits`
    // (`backbuild_conformance.rs`) already uses, confirmed clear of it.
    let before_sql = "SELECT id, region, created_at FROM orders WHERE region = 'EU'";
    let after_sql =
        "SELECT id, region, created_at FROM orders WHERE created_at < NOW() - INTERVAL '1 day'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    assert_eq!(options.atoms.len(), 2, "atoms: {:?}", options.atoms);
    for atom in &options.atoms {
        assert_eq!(
            atom.options.len(),
            1,
            "expected both the added (E1) and removed (E2) atoms admissible: {atom:?}"
        );
    }
    assert!(
        matches!(
            &options.atoms[0].change,
            AtomicChange::AddedConjunct { index: 0 }
        ),
        "expected the non-deterministic added conjunct to still classify as E1, got {:?}",
        options.atoms[0].change
    );
    assert!(
        options.atoms[0].options[0].statements[0].contains("NOW()"),
        "expected NOW() preserved verbatim: {:?}",
        options.atoms[0].options[0].statements
    );
}

// ===== F1 (task-8-brief.md) =====

#[test]
fn f1_plain_union_refuses() {
    // Plain `UNION` dedups across branches — the pure diff module does not
    // attempt to diff dedup semantics, so this refuses; only `UNION ALL`
    // (additive, branch-diffable) admits F1 (research §4 F1).
    let before_sql = "SELECT id FROM events_a";
    let after_sql = "SELECT id FROM events_a UNION SELECT id FROM events_b";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    let reason = atom.inadmissible[0].reason.to_lowercase();
    assert!(reason.contains("f1"), "reason: {reason}");
    assert!(reason.contains("union all"), "reason: {reason}");

    let targeted = assemble(
        &options,
        &Selection::Targeted {
            atom_choices: vec![0],
        },
    );
    assert!(targeted.is_empty());
}

// ===== F2 (task-11-brief.md) =====

#[test]
fn f2_no_discriminator_refuses() {
    // Neither branch carries any constant column at all — there is no
    // candidate discriminator, so F2 refuses with an actionable nudge.
    let before_sql = "SELECT id FROM events_a UNION ALL SELECT id FROM events_b";
    let after_sql = "SELECT id FROM events_a";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    let reason = atom.inadmissible[0].reason.to_lowercase();
    assert!(reason.contains("provenance predicate"), "reason: {reason}");
    assert!(reason.contains("discriminator column"), "reason: {reason}");
}

#[test]
fn f2_shared_discriminator_refuses() {
    // The removed branch (`events_b`) shares its 'src' constant ('a') with
    // a surviving branch (`events_a`) — an equality predicate on it would
    // also delete the surviving branch's rows, so F2 must refuse.
    let before_sql = "SELECT id, 'a' AS src FROM events_a UNION ALL SELECT id, 'a' AS src FROM \
                       events_b UNION ALL SELECT id, 'c' AS src FROM events_c";
    let after_sql =
        "SELECT id, 'a' AS src FROM events_a UNION ALL SELECT id, 'c' AS src FROM events_c";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    let reason = atom.inadmissible[0].reason.to_lowercase();
    assert!(
        reason.contains("not distinct per branch"),
        "reason: {reason}"
    );
    assert!(reason.contains("surviving branch"), "reason: {reason}");
}

#[test]
fn f2_mixed_literal_kind_discriminator_refuses() {
    // The removed branch's 'src' discriminator is a NUMBER literal (`1`)
    // while the surviving branches' 'src' discriminators are TEXT literals
    // (`'1'`, `'2'`) — UNION ALL coerces the column to a common supertype
    // (VARCHAR), so the emitted `DELETE ... WHERE src = 1` implicit-casts
    // and also matches the surviving `'1'` rows even though `(Number, "1")`
    // and `(Text, "1")` compare unequal as a Rust tuple. F2 must refuse by
    // name rather than admit a same-text-but-different-kind "distinct"
    // discriminator.
    let before_sql = "SELECT id, '1' AS src FROM events_a UNION ALL SELECT id, 1 AS src FROM \
                       events_b UNION ALL SELECT id, '2' AS src FROM events_c";
    let after_sql =
        "SELECT id, '1' AS src FROM events_a UNION ALL SELECT id, '2' AS src FROM events_c";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    let reason = atom.inadmissible[0].reason.to_lowercase();
    assert!(
        reason.contains("mixes literal kinds across branches"),
        "reason: {reason}"
    );
    assert!(reason.contains("union-coerced"), "reason: {reason}");

    let targeted = assemble(
        &options,
        &Selection::Targeted {
            atom_choices: vec![0],
        },
    );
    assert!(targeted.is_empty());
}

#[test]
fn f2_nonconstant_discriminator_refuses() {
    // The removed branch's own 'src' column is not a constant (it's a bare
    // column reference), even though the surviving branches' own 'src'
    // columns are constants — fail-closed, since only a constant-per-branch
    // discriminator is provable from the definitions alone.
    let before_sql = "SELECT id, 'a' AS src FROM events_a UNION ALL SELECT id, tag AS src FROM \
                       events_b UNION ALL SELECT id, 'c' AS src FROM events_c";
    let after_sql =
        "SELECT id, 'a' AS src FROM events_a UNION ALL SELECT id, 'c' AS src FROM events_c";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    let reason = atom.inadmissible[0].reason.to_lowercase();
    assert!(
        reason.contains("not a constant literal in every branch"),
        "reason: {reason}"
    );
}

#[test]
fn f2_plain_union_refuses() {
    // Plain `UNION` dedups across branches — the pure diff module does not
    // attempt to diff dedup semantics, so removal via F2 refuses just like
    // F1's addition does (research §4 F2's posture extends F1's).
    let before_sql = "SELECT id, 'a' AS src FROM events_a UNION SELECT id, 'b' AS src FROM \
                       events_b";
    let after_sql = "SELECT id, 'a' AS src FROM events_a";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    let reason = atom.inadmissible[0].reason.to_lowercase();
    assert!(reason.contains("union all"), "reason: {reason}");
}

// ===== C2 regression: representative matching is qualifier-aware, not
// bare-name keyed (final-review finding C2) — a qualified dependency must
// only resolve against a representative bound to that same qualifier and
// raw column name, never against an unrelated representative that merely
// shares the same *output* name. I1 (unqualified-reference alias sweep,
// fixed alongside C2) is covered by
// `i1_unqualified_dim_reference_in_where_refuses_b4` below. =====

#[test]
fn c2_qualified_dependency_does_not_collide_with_differently_sourced_representative() {
    // The exact reproduction from the final review: a pre-existing,
    // unchanged join (`orders o LEFT JOIN dim d`, present in both
    // definitions — never routed through B4's added-join admission at all)
    // already has a stored representative named 'region' (`o.rc AS
    // region`). Adding `d.region AS region_u` must NOT be admitted by
    // matching the *unrelated* 'region' representative just because the
    // bare output names collide — 'region_u' depends on `d.region`, a
    // different qualifier and a different raw column, which has no
    // representative of its own (no source is declared for alias 'd' at
    // all here) — both B1 and B3 must refuse.
    let before_sql = "SELECT o.rc AS region FROM orders o LEFT JOIN dim d ON o.order_id = \
                       d.order_id";
    let after_sql = "SELECT o.rc AS region, d.region AS region_u FROM orders o LEFT JOIN dim d \
                      ON o.order_id = d.order_id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[("region_u", "TEXT")]));
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.contains("d.region")),
        "expected a refusal naming the qualified dependency 'd.region', got: {:?}",
        atom.inadmissible
    );
    // The unsound pre-fix behaviour emitted `UPDATE t SET region_u = region`
    // — reading the fact's own stored 'region' column. No option at all
    // (let alone one shaped like that) may be admitted.
    for option in &atom.options {
        for stmt in &option.statements {
            assert!(
                !stmt.contains("region_u = region"),
                "must never emit the unsound self-read collision, got: {stmt}"
            );
        }
    }
}

#[test]
fn c2_qualified_dependency_via_where_conjunct_does_not_collide_e1() {
    // The same qualifier-collision hole, reached through E1's WHERE-conjunct
    // requalification rather than B1's SELECT-list requalification: a
    // pre-existing, unchanged join already has a stored representative named
    // 'region' (`o.region AS region`); adding a WHERE conjunct on the
    // *dimension's* 'region' column (`d.region = 'east'`) must not resolve
    // against the fact-side representative just because the bare names
    // collide.
    let before_sql =
        "SELECT o.region AS region FROM orders o LEFT JOIN dim d ON o.order_id = d.order_id";
    let after_sql = "SELECT o.region AS region FROM orders o LEFT JOIN dim d ON o.order_id = \
                      d.order_id WHERE d.region = 'east'";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.contains("region") && r.reason.contains("no stored representative")),
        "expected a refusal naming 'region' with no stored representative (the qualified \
         `d.region` dependency must not collide with the fact-side `o.region` representative), \
         got: {:?}",
        atom.inadmissible
    );
}

// ===== I1 regression: the B4 "nothing else references it" alias sweep must
// fail closed on an unqualified reference it cannot prove fact-side (fixed
// alongside C2 — final-review finding I1). =====

#[test]
fn i1_unqualified_dim_reference_in_where_refuses_b4() {
    // 'active' is never stored anywhere in the fact's own output — an
    // unqualified reference to it inside the WHERE clause cannot be proven
    // fact-side, so it must be treated exactly like a qualified reference to
    // the added join's alias would be: the join is not a pure enrichment,
    // and B4 refuses.
    let before_sql = "SELECT o.order_id AS order_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, d.name AS dim_name FROM orders o LEFT JOIN \
                      dims d ON o.order_id = d.order_id WHERE active = TRUE";

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

// ===== C4 regression: F1 must validate the added branch's own output
// column names/order against positional UNION ALL semantics (final-review
// finding C4) — an added branch with swapped column names/order would bind
// differently under a real rebuild's positional UNION ALL than this
// emitter's name-based INSERT would. =====

#[test]
fn c4_added_branch_with_swapped_column_order_refuses() {
    // The added branch declares its columns in the opposite order/name
    // pairing from the first branch: the first branch declares (id, kind)
    // but the added branch declares (kind, id). A real rebuild binds
    // `UNION ALL` positionally under the first branch's own column names,
    // so the added branch's `kind` value would actually land in the
    // deployed table's `id` column — a name-based `INSERT` (matching by
    // declared name instead) would do the opposite, silently diverging.
    let before_sql = "SELECT id, kind FROM events_a";
    let after_sql = "SELECT id, kind FROM events_a UNION ALL SELECT kind, id FROM events_z";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.to_lowercase().contains("positionally")),
        "expected a refusal naming UNION ALL's positional binding, got: {:?}",
        atom.inadmissible
    );
}

// ===== F1 edited-survivor branch matching + the C4 reorder guard (research
// §4 F1's edited-survivor generalization) =====

#[test]
fn edited_nonfirst_branch_refuses_named() {
    // An edit sits in branch 1 (not branch 0) — the top-level
    // (whole-definition) SELECT-list/WHERE/skeleton diff only ever
    // describes branch 0's own content (`definition_diff` computes it from
    // each side's outermost `SelectStmt`, which is branch 0 of its own
    // chain), so a non-first branch's edit can never be classified from it.
    // Must refuse by name rather than silently treating the (accidentally
    // clean-looking) top-level diff as proof that nothing changed.
    let before_sql = "SELECT id FROM t1 UNION ALL SELECT id, a FROM t2";
    let after_sql = "SELECT id FROM t1 UNION ALL SELECT id, a, a * 2 AS doubled FROM t2";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[("doubled", "INTEGER")]));
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible
            .iter()
            .any(|r| r.reason.contains("branch 0")),
        "expected a refusal naming that only branch 0's own edit is describable, got: {:?}",
        atom.inadmissible
    );

    let targeted = assemble(
        &options,
        &Selection::Targeted {
            atom_choices: vec![0],
        },
    );
    assert!(targeted.is_empty());
}

#[test]
fn c4_swapped_order_reorder_append_refuses() {
    // A contrived corner: the before-definition's two branches already
    // share the same name-set/expressions/FROM (no WHERE), but declare
    // their columns in mutually swapped orders. A single edit that reorders
    // the branches and appends a third makes the *name-keyed* top-level
    // SELECT-list diff report a no-op (same names, same expressions on both
    // sides — `id`/`kind` are plain column references either way) even
    // though the real first branch's own *declared* column order changed
    // from `(id, kind)` to `(kind, id)`. `UNION ALL` binds columns
    // positionally, so this divergence must refuse rather than slip past a
    // bare name-set no-op check (predecessor residual "C4 contrived
    // corner").
    let before_sql = "SELECT id, kind FROM t1 UNION ALL SELECT kind, id FROM t1";
    let after_sql = "SELECT kind, id FROM t1 UNION ALL SELECT id, kind FROM t1 UNION ALL SELECT \
                      id, kind FROM t3";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        atom.inadmissible.iter().any(|r| {
            let lower = r.reason.to_lowercase();
            lower.contains("declared") && lower.contains("order")
        }),
        "expected a refusal naming the declared-order divergence, got: {:?}",
        atom.inadmissible
    );

    let targeted = assemble(
        &options,
        &Selection::Targeted {
            atom_choices: vec![0],
        },
    );
    assert!(targeted.is_empty());
}

#[test]
fn branch_swap_with_edit_refuses_phantom_top_level_diff() {
    // Reviewer-confirmed soundness bug (execution against real DuckDB): the
    // trusted edited-branch-0 admission (`edited.len() == 1 &&
    // edited[0].index == 0 && removed.is_empty()`) checked only the
    // before-chain position of the leftover edit, not which after-position
    // it was actually *paired* against. Here after's literal branch 0
    // (`SELECT id, a FROM t1 WHERE a > 100`) exact-text-matches before's
    // branch 1, so it is consumed into `unchanged`; before's branch 0's
    // true edit (`a` -> `a * 2 AS a`) is left paired against after-position
    // 1, not after's own declared branch 0. The top-level (whole-definition)
    // diff nonetheless compares before's literal branch 0
    // (`SELECT id, a FROM t1`) against after's literal branch 0
    // (`SELECT id, a FROM t1 WHERE a > 100`), reporting a phantom added
    // WHERE conjunct `a > 100` that describes no single branch's real
    // change. Trusting it previously admitted a wrong E1
    // `DELETE ... WHERE (a > 100) IS NOT TRUE`, which — against
    // `t1 = {(1,10),(2,20),(3,200)}` — diverges from a full rebuild (a full
    // rebuild keeps `(1,10)` doubled to `(1,20)` from the reordered-and-
    // edited former branch 0, and keeps `(3,200)` from the now-filtered
    // former branch 1; the wrong DELETE instead drops the stored `(1,10)`
    // row outright). This must now be a named refusal (FullRefresh-only —
    // no E1 atom admitted).
    let before_sql = "SELECT id, a FROM t1 UNION ALL SELECT id, a FROM t1 WHERE a > 100";
    let after_sql = "SELECT id, a FROM t1 WHERE a > 100 UNION ALL SELECT id, a * 2 AS a FROM t1";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    assert_refused(atom);
    assert!(
        !atom
            .options
            .iter()
            .any(|o| o.technique == Technique::PredicateTightenDelete),
        "must not admit a wrong E1 DELETE from a phantom top-level diff, got options: {:?}",
        atom.options
    );

    let targeted = assemble(
        &options,
        &Selection::Targeted {
            atom_choices: vec![0],
        },
    );
    assert!(
        targeted.is_empty(),
        "expected FullRefresh-only (no targeted atoms), got: {targeted:?}"
    );
}

#[test]
fn c1_option_marked_destructive() {
    let before_sql = "SELECT id, extra FROM orders";
    let after_sql = "SELECT id FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    let atom = single_atom(&options);
    match &atom.change {
        AtomicChange::DroppedColumn { name } => assert_eq!(name, "extra"),
        other => panic!("expected a DroppedColumn atom, got {other:?}"),
    }
    assert_eq!(atom.options.len(), 1, "{atom:?}");
    let option = &atom.options[0];
    assert_eq!(option.technique, Technique::ColumnDrop);
    assert_eq!(option.slot, Some(HSlot::Drop));
    assert_eq!(option.statements, vec!["ALTER TABLE t DROP COLUMN extra"]);
    // The metadata records a write scope distinct from a plain column-value
    // update (`WriteScope::ColumnScoped`) — so a wiring-time chooser can
    // gate this technique behind the `--allow-column-removal` opt-in
    // doctrine without re-classifying (research §4 C1's "Classification and
    // the opt-in flag doctrine ... stay owned by schema_evolution.md").
    assert_eq!(option.write_scope, WriteScope::Destructive);
    assert_ne!(option.write_scope, WriteScope::ColumnScoped);
    assert!(atom.inadmissible.is_empty());
}

#[test]
fn c1_rename_pairing_still_wins() {
    // A dropped column whose expression matches an added one must still
    // classify as B2 (rename), never as a C1 drop paired with an unrelated
    // B1 add — a regression guard on pairing order (rename pairing runs
    // *before* C1/B1 classification).
    let before_sql = "SELECT id, amount FROM orders";
    let after_sql = "SELECT id, amount AS total FROM orders";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs(&[]));
    assert!(
        !options
            .atoms
            .iter()
            .any(|a| matches!(&a.change, AtomicChange::DroppedColumn { .. })),
        "expected no DroppedColumn atom — 'amount' must pair into the rename, got {:?}",
        options.atoms
    );
    let atom = single_atom(&options);
    match &atom.change {
        AtomicChange::RenamedColumn { from, to } => {
            assert_eq!(from, "amount");
            assert_eq!(to, "total");
        }
        other => panic!("expected a RenamedColumn atom, got {other:?}"),
    }
    assert_eq!(atom.options[0].technique, Technique::Rename);
}
