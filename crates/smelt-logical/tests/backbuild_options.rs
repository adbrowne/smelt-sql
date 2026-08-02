//! Pure (no DuckDB) tests for the backbuild option-set data model:
//! `smelt_logical::backbuild::{BackbuildOptions, AtomAnalysis, assemble}`.
//! See `docs/research/20260802-backbuild-synthesis.md` §2 ("Refusal
//! posture": "partial application is never offered; an atom with an empty
//! option set leaves FullRefresh as the model's only option") and
//! `.superpowers/sdd/20260802-backbuild-synthesis/task-2-brief.md`.

use std::collections::BTreeMap;

use smelt_logical::backbuild::{
    assemble, definition_diff, derive_backbuild_options, AtomAnalysis, AtomicChange,
    BackbuildInputs, BackbuildOption, BackbuildOptions, HSlot, Selection, Technique, WriteScope,
};

fn parse(sql: &str) -> smelt_parser::File {
    let parse = smelt_parser::parse(sql);
    smelt_parser::File::cast(parse.syntax()).expect("file")
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
