use super::*;

// ─── Emission

#[test]
fn the_rename_matrix_matches_the_printer_it_replaces() {
    // These rows were transcribed from the printer's hand-written rename chain
    // before that chain was deleted, so the registry is provably a faithful
    // replacement for it rather than a re-derivation.
    let expected: &[(&str, DialectId, &str)] = &[
        ("EXPLODE", DialectId::DuckDb, "UNNEST"),
        ("EXPLODE", DialectId::BigQuery, "UNNEST"),
        ("UNNEST", DialectId::SparkSql, "EXPLODE"),
        ("EVERY", DialectId::DuckDb, "BOOL_AND"),
        ("EVERY", DialectId::BigQuery, "LOGICAL_AND"),
        ("BOOL_AND", DialectId::SparkSql, "EVERY"),
        ("BOOL_AND", DialectId::BigQuery, "LOGICAL_AND"),
        ("BOOL_OR", DialectId::SparkSql, "SOME"),
        ("BOOL_OR", DialectId::BigQuery, "LOGICAL_OR"),
    ];
    for (name, dialect, renamed) in expected {
        let sig = BuiltinRegistry::resolve(name).expect(name);
        assert_eq!(
            sig.emission_at(*dialect, Position::Any),
            Emission::Rename(renamed),
            "{name} on {}",
            dialect.slug()
        );
    }
}

#[test]
fn caret_is_rewritten_wherever_infix_caret_means_xor() {
    // GoogleSQL and Spark SQL both define infix `^` as bitwise XOR while smelt's
    // grammar reads it as power. Emitting it verbatim returns a different number
    // rather than failing — the silent-divergence class this work exists to close.
    for dialect in [DialectId::SparkSql, DialectId::BigQuery] {
        for op in ["^", "**"] {
            let sig = BuiltinRegistry::resolve(op).expect(op);
            assert_eq!(
                sig.emission_at(dialect, Position::Any),
                Emission::Template("POWER({0}, {1})"),
                "{op} on {}",
                dialect.slug()
            );
        }
    }
    for op in ["^", "**"] {
        let sig = BuiltinRegistry::resolve(op).expect(op);
        assert_eq!(
            sig.emission_at(DialectId::DuckDb, Position::Any),
            Emission::Native
        );
    }
}

#[test]
fn floor_divide_is_unsupported_everywhere_it_has_no_safe_lowering() {
    let sig = BuiltinRegistry::resolve("//").expect("//");
    assert_eq!(
        sig.emission_at(DialectId::DuckDb, Position::Any),
        Emission::Native
    );
    // BigQuery has no per-class lowering at all — flatly unsupported.
    assert!(
        matches!(
            sig.emission_at(DialectId::BigQuery, Position::Any),
            Emission::Unsupported { .. }
        ),
        "// on bigquery must be a declared refusal, not a pass-through"
    );
    // Spark settles per operand class (phase 7); an operand whose class
    // cannot be resolved must still land on a declared refusal, never a
    // pass-through.
    assert_eq!(
        sig.settle_at(
            DialectId::SparkSql,
            Position::Any,
            &CallFacts::unresolved(2)
        ),
        SettledEmission::Unsupported {
            reason: "Spark SQL has no infix `//`; use a typed FLOOR(a / b) or DIV(a, b)"
        }
    );
}

#[test]
fn an_unlisted_dialect_defaults_to_native() {
    let sig = BuiltinRegistry::resolve("LOWER").expect("LOWER");
    for d in DialectId::ALL {
        assert_eq!(sig.emission_at(*d, Position::Any), Emission::Native);
    }
}

#[test]
fn every_declared_rewrite_id_is_reachable_from_some_entry() {
    // A RewriteId with no registry row is printer code nothing can call.
    let mut seen: Vec<RewriteId> = BuiltinRegistry::names()
        .filter_map(BuiltinRegistry::resolve)
        .flat_map(|sig| sig.emission.iter())
        .filter_map(|(_, _, e)| match e {
            Emission::Rewrite(id) => Some(*id),
            _ => None,
        })
        .collect();
    seen.sort();
    seen.dedup();
    assert_eq!(
        seen,
        vec![RewriteId::BigQueryMedian, RewriteId::WithinGroupToAnalytic],
    );
}

#[test]
fn every_declared_template_is_reachable_from_some_entry() {
    // The `%`/`^`/`**` templates migrated off `RewriteId` in this phase; this
    // is their equivalent reachability check for `Emission::Template`.
    let templates: Vec<&'static str> = BuiltinRegistry::names()
        .filter_map(BuiltinRegistry::resolve)
        .flat_map(|sig| sig.emission.iter())
        .filter_map(|(_, _, e)| match e {
            Emission::Template(t) => Some(*t),
            _ => None,
        })
        .collect();
    assert!(
        templates.contains(&"MOD({0}, {1})"),
        "expected the `%` modulo template to be registered: {templates:?}"
    );
    assert!(
        templates.contains(&"POWER({0}, {1})"),
        "expected the `^`/`**` power template to be registered: {templates:?}"
    );
}
